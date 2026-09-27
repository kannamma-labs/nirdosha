# frozen_string_literal: true

require "yaml"

# Parses rfcs/fixtures/0029-influence-review-symbolic.yaml (standard-library
# YAML/Psych only) and expands each symbolic fixture into a canonical fixture
# object matching the shape described by
# rfcs/fixtures/0029-canonical/schema.json, following ONLY the expansion
# rules written in the symbolic source's own `compact_grammar` /
# `lossless_expansion` / `defaults` header and the node/edge kind catalogs
# in RFC 0029.a Revision 1 (Model Influence and Human Review Semantics).
#
# Where the header text is genuinely silent on an exact attribute shape
# (this happens for AuthorityAssertion, and for where predicate evaluations
# and other free-form fixture fields land), this module makes an explicit,
# documented choice and records it as an assumption. Those choices affect
# byte-level canonicalization only; the semantic evaluators (lib/influence.rb,
# lib/review.rb, lib/fact_provenance.rb, lib/admission.rb) do not depend on
# them.
module Expand
  ASSUMPTIONS = [
    "Fact.attributes.authority_class is expanded as the literal string " \
      "\"profile_authority\" -- the symbolic source writes this token " \
      "unparameterized (unlike every other line in lossless_expansion, " \
      "which uses explicit <fixture_id>/<id> placeholders), so it is read " \
      "as a fixed literal rather than a per-profile lookup.",
    "AuthorityAssertion.attributes has no literal template in the symbolic " \
      "source (\"Expanded from fixture assertion plus authority/contract " \
      "catalogs\"). This implementation expands it to {assertion_id, kind, " \
      "authority, contract, covered_input_origins, covered_model_paths, " \
      "covered_targets}, resolving `authority` via our own independently " \
      "computed final-authority determination (lib/admission.rb; never read " \
      "from the fixture's `expected` block) and `kind` from the referenced " \
      "review contract's mode (IndependentDecision " \
      "for every assertion-bearing fixture in this corpus).",
    "Predicate evaluations (fixture.evaluations) are expanded into " \
      "input.parameters.evaluations as an array of PredicateEvaluation-shaped " \
      "records, since schema.json's catalog_entries has no \"evaluation\" " \
      "catalog kind and evaluations are not attributes of any single node.",
    "Fixture-level free-form fields with no explicit node/edge/catalog home " \
      "(declared_binding, bounded_class, dependency, monitor, mandatory_fact, " \
      "fact_provenance, fact_requirement, required_review_contract, " \
      "supplied_review_contract, review_contract, observe_certificate name) " \
      "are copied verbatim into input.parameters under their own key.",
  ].freeze

  SYMBOLIC_PATH = File.expand_path("../../../rfcs/fixtures/0029-influence-review-symbolic.yaml", __dir__)

  Doc = Struct.new(:raw, :fixtures_by_id, :equivalence_groups, :authority_catalog,
                    :predicate_definitions, :review_contract_catalog,
                    :observe_certificate_catalog, :defaults)

  def self.load_doc(path = SYMBOLIC_PATH)
    raw = YAML.safe_load(File.read(path), permitted_classes: [Symbol, Time], aliases: true)
    raw = stringify_times(raw)
    fixtures_by_id = {}
    raw.fetch("fixtures").each { |f| fixtures_by_id[f.fetch("id")] = f }
    Doc.new(
      raw,
      fixtures_by_id,
      raw.fetch("equivalence_groups"),
      raw.fetch("authority_catalog"),
      raw.fetch("predicate_definitions"),
      raw.fetch("review_contract_catalog"),
      raw.fetch("observe_certificate_catalog"),
      raw.fetch("defaults")
    )
  end

  # The symbolic YAML writes some timestamps unquoted (e.g.
  # `valid_from: 2026-09-27T00:00:00Z` in observe_certificate_catalog), which
  # Psych auto-detects as Time. Every timestamp elsewhere in the source is
  # quoted and stays a String. We normalize all of them to the same
  # "YYYY-MM-DDTHH:MM:SSZ" String shape immediately after parsing so the rest
  # of this module never has to special-case Time vs. String.
  def self.stringify_times(v)
    case v
    when Time
      v.utc.strftime("%Y-%m-%dT%H:%M:%SZ")
    when Hash
      v.transform_values { |vv| stringify_times(vv) }
    when Array
      v.map { |vv| stringify_times(vv) }
    else
      v
    end
  end

  # ---- node/edge token grammar -------------------------------------------

  def self.parse_node_token(tok)
    id, kind = tok.split(":", 2)
    [id, kind]
  end

  def self.parse_edge_token(tok)
    # "<source>-<EdgeKind>-<target>" ; EdgeKind is a known closed set so we
    # can split on the first and last hyphen-delimited kind match instead of
    # guessing where source/target ids end (ids never contain uppercase
    # EdgeKind tokens as substrings in this fixture set).
    parts = tok.split("-")
    kind_idx = parts.each_index.find { |i| EDGE_KINDS.include?(parts[i]) }
    raise "cannot parse edge token #{tok.inspect}" unless kind_idx
    source = parts[0...kind_idx].join("-")
    kind = parts[kind_idx]
    target = parts[(kind_idx + 1)..].join("-")
    [source, kind, target]
  end

  EDGE_KINDS = %w[
    DataFlow RuleFlow CandidateGenerate CandidateSuppress Eligibility Rank
    Attention Default Recommend DraftArtifact DraftReason ModelApproval
    ModelTrigger HumanInput AssertFact AssertDecision Authorize Require
    Satisfy Issue Record ObserveOnly
  ].freeze

  # ---- per-fixture expansion ---------------------------------------------

  Expanded = Struct.new(:fixture_id, :profile_id, :bundle, :nodes, :edges,
                         :catalog_entries, :parameters, :expected_raw,
                         :node_index, :edge_list, keyword_init: false)

  # `computed_final_authority_key` must come from our OWN semantic evaluator
  # (lib/admission.rb), never from the fixture's `expected` block: the
  # AuthorityAssertion.authority attribute is part of the *input* encoding,
  # and the input must never be built from the oracle verdict.
  def self.expand_fixture(doc, fixture, computed_final_authority_key)
    fid = fixture.fetch("id")
    profile = fixture["profile"]
    profile_id = profile ? "profile:rfc0029:#{profile}:candidate-v1" : nil

    bundle = { "id" => "bundle:rfc0029-fixtures", "version" => 1, "hash" => "symbolic:bundle-rfc0029-fixtures-v1" }

    node_tokens = fixture["nodes"] || []
    edge_tokens = fixture["edges"] || []

    local_nodes = {} # local_id -> kind
    node_tokens.each do |tok|
      id, kind = parse_node_token(tok)
      local_nodes[id] = kind
    end

    edges_local = edge_tokens.map { |tok| parse_edge_token(tok) } # [source, kind, target]

    catalog_refs = { "authority" => {}, "predicate_definition" => {}, "review_contract" => {}, "observe_certificate" => {} }

    stable = ->(local_id) { "node:#{fid}:#{local_id}" }

    # Resolve which review contract (if any) governs this fixture's Review
    # node / assertion, per lossless_expansion's Review rule.
    contract_via_assertion = false
    contract_key = fixture["review_contract"]
    if !contract_key && fixture["assertion"]
      contract_key = fixture["assertion"]["contract"]
      contract_via_assertion = true
    end

    add_contract_catalog = lambda do |key|
      contract = doc.review_contract_catalog.fetch(key)
      catalog_refs["review_contract"][key] = contract
      (contract["predicates"] || []).each do |pname|
        catalog_refs["predicate_definition"][pname] = doc.predicate_definitions.fetch(pname)
      end
      contract
    end

    resolved_contract = nil
    if contract_key
      resolved_contract = add_contract_catalog.call(contract_key)
    end
    # R2-shaped fixtures reference two contracts (required/supplied) without
    # either becoming `contract_key`; both still need their own catalog
    # entries (confirmed against the oracle's R2_ACK_FOR_APPROVAL vector).
    if fixture["required_review_contract"]
      add_contract_catalog.call(fixture["required_review_contract"])
    end
    if fixture["supplied_review_contract"]
      add_contract_catalog.call(fixture["supplied_review_contract"])
    end

    final_authority_key = computed_final_authority_key
    resolved_authority = nil
    if final_authority_key && doc.authority_catalog.key?(final_authority_key)
      resolved_authority = doc.authority_catalog[final_authority_key]
      catalog_refs["authority"][final_authority_key] = resolved_authority
    end

    assertion_mode = resolved_contract && resolved_contract["mode"]
    assertion_kind = case assertion_mode
                      when "IndependentDecision" then "IndependentDecision"
                      when "BoundedApproval" then "BoundedApproval"
                      else nil
                      end

    # RFC 8785 only orders OBJECT member names; it says nothing about array
    # element order (see lib/jcs.rb). The existing corpus's own jcs/*.jcs.json
    # nonetheless emits `nodes`/`edges` in a fixed order. Confirmed against
    # several oracle fixtures (EQ1A, EQ2A, B1, L1): nodes are sorted by
    # `kind` ascending (e.g. B1 -> Capability, Decision, Effect, Fact, Rule),
    # and edges are sorted by `kind` ascending (B1 -> Authorize, DataFlow,
    # Issue, RuleFlow) -- not by id/source/target at all. We match that
    # ordering so a byte comparison is only ever blocked by an actual content
    # difference, never by this incidental array-order choice.
    nodes = local_nodes.map do |local_id, kind|
      attrs = expand_node_attributes(fid, local_id, kind, fixture, doc, resolved_contract,
                                      assertion_kind, resolved_authority, final_authority_key, contract_key)
      { "id" => stable.call(local_id), "kind" => kind, "attributes" => attrs }
    end.sort_by { |n| [n["kind"], n["id"]] }

    edges = edges_local.map do |src, kind, tgt|
      { "source" => stable.call(src), "kind" => kind, "target" => stable.call(tgt) }
    end.sort_by { |e| [e["kind"], e["source"], e["target"]] }

    if fixture["observe_certificate"]
      cert_key = fixture["observe_certificate"]
      catalog_refs["observe_certificate"][cert_key] = doc.observe_certificate_catalog.fetch(cert_key)
    end

    catalog_entries = []
    catalog_refs.each do |catalog, byname|
      byname.each do |name, value|
        catalog_entries << { "catalog" => catalog, "name" => name, "value" => deep_plain(value) }
      end
    end
    catalog_entries.sort_by! { |e| [e["catalog"], e["name"]] }

    parameters = build_parameters(fid, fixture, doc, contract_key, contract_via_assertion)

    Expanded.new(fid, profile_id, bundle, nodes, edges, catalog_entries, parameters,
                 fixture["expected"] || {}, local_nodes, edges_local)
  end

  def self.expand_node_attributes(fid, local_id, kind, fixture, doc, resolved_contract,
                                   assertion_kind, resolved_authority, final_authority_key, contract_key)
    stable = ->(id) { "node:#{fid}:#{id}" }
    case kind
    when "ModelInvocation"
      { "deployment_id" => "model:#{fid}:v1", "receipt_id" => "receipt:#{fid}:#{local_id}:v1" }
    when "Fact"
      attrs = { "authority_class" => "profile_authority", "type_id" => "fact:#{fid}:#{local_id}", "version" => 1 }
      fp = fixture.dig("fact_provenance", local_id)
      if fp
        attrs["produced_by_model"] = fp["produced_by_model"]
        attrs["produced_by_deployment"] = fp["produced_by_deployment"]
        attrs["valid_until"] = fp["valid_until"]
        attrs["revoked"] = fp["revoked"]
        attrs["authorized_consumers"] = (fp["authorized_consumers"] || []).map { |c| stable.call(c) }
      end
      attrs
    when "Transform"
      { "transform_id" => "transform:#{fid}:#{local_id}", "version" => 1 }
    when "Rule"
      { "rule_id" => "rule:#{fid}:#{local_id}", "bundle_hash" => "symbolic:bundle-rfc0029-fixtures-v1" }
    when "CandidateSet"
      { "candidate_type" => "candidate:#{fid}:#{local_id}" }
    when "Presentation"
      { "surface_id" => "surface:#{fid}:#{local_id}", "version" => 1 }
    when "Review"
      # Confirmed against the oracle at comparison time (jcs/EQ3A.jcs.json,
      # jcs/M4.jcs.json): a Review node's attributes are just
      # {"review_contract" => <catalog key, or null if none resolves>} --
      # the bare symbolic contract key, not a resolved id/mode pair.
      { "review_contract" => contract_key }
    when "AuthorityAssertion"
      # Confirmed against the oracle: attributes are the fixture's own
      # `assertion:` block copied verbatim (local ids, not stable ids; no
      # synthesized authority/kind/contract-id resolution at all), or just
      # {"id" => local_id} when the fixture declares the bare node token
      # with no `assertion:` elaboration (M4, L5 -- see report).
      fixture["assertion"] ? deep_plain(fixture["assertion"]) : { "id" => local_id }
    when "Decision"
      attrs = { "decision_class" => "decision:#{fid}:#{local_id}" }
      fr = fixture.dig("fact_requirement", local_id)
      if fr
        attrs["required_model"] = fr["required_model"]
        attrs["required_deployment"] = fr["required_deployment"]
      end
      attrs
    when "Obligation"
      { "obligation_type" => "obligation:#{fid}:#{local_id}" }
    when "Capability"
      cap = fixture.dig("capabilities", local_id)
      raise "invalid fixture #{fid}: Capability node #{local_id} has no fixture.capabilities entry" unless cap
      { "effect_class" => cap["effect_class"] }
    when "Effect"
      eff = fixture.dig("effects", local_id)
      raise "invalid fixture #{fid}: Effect node #{local_id} has no fixture.effects entry" unless eff
      { "effect_id" => eff["effect_id"], "reversibility_class" => eff["reversibility_class"] }
    when "Evidence"
      { "evidence_type" => "evidence:#{fid}:#{local_id}" }
    else
      raise "unknown node kind #{kind.inspect} in fixture #{fid}"
    end
  end

  PREDICATE_EVAL_DEFAULTS = {
    "evaluator_authority" => "authority:fixture-review-evaluator:v1",
    "evaluated_at" => "2026-09-27T00:00:00Z",
    "valid_until" => "2026-09-28T00:00:00Z",
    "revoked" => false,
  }.freeze

  def self.expand_evaluation(fid, pname, value, doc)
    pdef = doc.predicate_definitions.fetch(pname)
    rec = {
      "predicate" => pname,
      "definition_id" => pdef["id"],
      "definition_version" => pdef["version"],
      "evaluator_authority" => PREDICATE_EVAL_DEFAULTS["evaluator_authority"],
      "input_commitment" => "symbolic:#{fid}:#{pname}:inputs",
      "evidence_commitment" => "symbolic:#{fid}:#{pname}:evidence",
      "evaluated_at" => PREDICATE_EVAL_DEFAULTS["evaluated_at"],
      "valid_until" => PREDICATE_EVAL_DEFAULTS["valid_until"],
      "revoked" => PREDICATE_EVAL_DEFAULTS["revoked"],
      "failure_behavior" => "Deny",
    }
    if value.is_a?(String) && value.start_with?("stale:")
      failure_behavior = value.split(":", 2)[1]
      rec["result"] = "Unknown"
      rec["valid_until"] = "2026-09-26T00:00:00Z"
      rec["failure_behavior"] = failure_behavior
    else
      rec["result"] = value ? "True" : "False"
    end
    rec
  end

  # Confirmed against the oracle at comparison time: `fact_provenance`,
  # `fact_requirement` and `observe_certificate` are NOT duplicated into
  # input.parameters (they are fully expressed via Fact/Decision node
  # attributes and the observe_certificate catalog entry respectively, and
  # B2_STALE's own parameters come back `{}` despite declaring an
  # observe_certificate). `assertion` is likewise not duplicated (it is fully
  # expressed via the AuthorityAssertion node's own attributes). What IS
  # present: `declared_binding`, `bounded_class`, `dependency`, `monitor`,
  # `mandatory_fact`, `required_review_contract`/`supplied_review_contract`
  # verbatim, plus a resolved `review_contract` (the same contract key the
  # Review node's attributes use, whether it came from `fixture.review_contract`
  # or `fixture.assertion.contract`) and a resolved `evaluations` array that
  # follows the review_contract_catalog's `evaluation_fixture` indirection
  # when the fixture supplies no `evaluations` of its own.
  def self.build_parameters(fid, fixture, doc, contract_key, contract_via_assertion)
    params = {}

    # Confirmed against the oracle: the `evaluation_fixture` indirection is
    # only followed into input.parameters.evaluations when the contract was
    # reached via an AuthorityAssertion (EQ3A -> rc-independent-pass ->
    # R5_PASS's evaluations). K3/L6 reach `self-review` (evaluation_fixture:
    # R4_CONFLICT) directly through the bare `review_contract:` field with no
    # AuthorityAssertion node at all, and their oracle input.parameters carry
    # no `evaluations` key whatsoever -- only `review_contract: "self-review"`.
    evaluations = fixture["evaluations"]
    if !evaluations && contract_key && contract_via_assertion
      contract = doc.review_contract_catalog[contract_key]
      ref_id = contract && contract["evaluation_fixture"]
      evaluations = doc.fixtures_by_id.dig(ref_id, "evaluations") if ref_id
    end
    if evaluations
      params["evaluations"] = evaluations.map do |ev|
        pname, value = ev.to_a.first
        expand_evaluation(fid, pname, value, doc)
      end
    end

    params["review_contract"] = contract_key if contract_key

    %w[declared_binding bounded_class dependency monitor mandatory_fact
       required_review_contract supplied_review_contract].each do |k|
      params[k] = fixture[k] if fixture.key?(k)
    end

    # RFC 0029.a §13.2: every model-influenced path with an effect class
    # declares a maximum authority tier. As of 2026-09-27 this table lives
    # in the symbolic source itself (`effect_authority_ceilings`), not only
    # in the excluded generator -- so it is now a normative lookup, not a
    # reconstruction. Effects take precedence over capabilities, matching
    # the oracle's own derivation order.
    effect_class =
      fixture.dig("effects")&.values&.first&.dig("effect_id") ||
      fixture.dig("capabilities")&.values&.first&.dig("effect_class")
    if effect_class
      ceiling = doc.raw.fetch("effect_authority_ceilings").fetch(effect_class)
      params["admission_policy"] = { "effect_class" => effect_class, "maximum_model_level" => ceiling }
    end

    deep_plain(params)
  end

  def self.deep_plain(v)
    case v
    when Hash
      v.each_with_object({}) { |(k, vv), h| h[k.to_s] = deep_plain(vv) }
    when Array
      v.map { |vv| deep_plain(vv) }
    else
      v
    end
  end
end
