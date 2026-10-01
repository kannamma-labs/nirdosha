# frozen_string_literal: true

require_relative "model_graph"

# Independent review-contract evaluator, derived from RFC 0029.a Revision 1
# §7 ("Review contracts are declared, not inferred") and §7.1 (required
# predicates by mode), plus the self-certification rule implicit in §5
# ("Acknowledgement cannot create an authority assertion... self-certification
# ... never creates independent authority") and §8 ("self-certification never
# creates independent authority").
module Review
  MODE_RANK = { "Acknowledgement" => 0, "BoundedApproval" => 1, "IndependentDecision" => 2 }.freeze

  # Predicate -> named finding when its evaluation is a current `False`.
  # Only four predicates are ever exercised with a literal `false` result
  # across the 50 fixtures (independence, evidence_accessed, reason_bound,
  # capacity); this mapping is confirmed by those four fixtures' own
  # `expected.review` finding names (R4_CONFLICT, R3_NOT_ACCESSED, I2,
  # R7_CAPACITY). Any other predicate failing false in a fixture outside this
  # corpus would fall back to a generic, clearly-marked placeholder code.
  FAIL_CODE_BY_PREDICATE = {
    "independence" => "IndependenceConflict",
    "evidence_accessed" => "RequiredEvidenceNotAccessed",
    "reason_bound" => "ReasonNotEvidenceBound",
    "capacity" => "CapacityExceeded",
  }.freeze

  Outcome = Struct.new(:status, :detail, :extra, keyword_init: true)

  def self.resolve_contract_key(fixture)
    return fixture["review_contract"] if fixture["review_contract"]
    return fixture["assertion"]["contract"] if fixture["assertion"]

    nil
  end

  def self.resolve_evaluations(fixture, contract_key, doc)
    return fixture["evaluations"] if fixture["evaluations"]

    contract = contract_key && doc.review_contract_catalog[contract_key]
    ref_id = contract && contract["evaluation_fixture"]
    return nil unless ref_id

    ref_fixture = doc.fixtures_by_id[ref_id]
    ref_fixture && ref_fixture["evaluations"]
  end

  def self.evaluate(fixture, doc)
    # R2-shaped case: an explicit required-vs-supplied contract mode check,
    # evaluated before any predicate content.
    if fixture["required_review_contract"] && fixture["supplied_review_contract"]
      required = doc.review_contract_catalog.fetch(fixture["required_review_contract"])
      supplied = doc.review_contract_catalog.fetch(fixture["supplied_review_contract"])
      if MODE_RANK.fetch(supplied["mode"]) < MODE_RANK.fetch(required["mode"])
        return Outcome.new(status: "fail", detail: "ReviewModeInsufficient", extra: {})
      end
      contract_key = fixture["supplied_review_contract"]
    else
      contract_key = resolve_contract_key(fixture)
    end

    graph = ModelGraph.build(fixture)
    has_assertion = graph.nodes.value?("AuthorityAssertion")

    # ASSUMPTION (disclosed in the report as a genuine specification gap):
    # two fixtures (M4, L5) carry an AuthorityAssertion node and an
    # `expected.review: pass` verdict but declare neither `review_contract`
    # nor an `assertion:` block naming a contract at all -- the symbolic
    # source's own lossless_expansion rule for Review nodes ("review_contract_id:
    # <fixture.review_contract-or-assertion.contract>") has no value to read
    # in that case. Rather than silently reporting "not_required" (which
    # would contradict the fixture's own asserted "pass"), we treat a bare,
    # contract-less AuthorityAssertion as governed by the default independent-
    # review contract (rc-independent-pass) -- the same contract every other
    # passing-assertion fixture in this corpus actually uses.
    if !contract_key && has_assertion
      contract_key = "rc-independent-pass"
    end

    unless contract_key
      # A fixture with no graph at all (the INV_*/L7 pure dependency-
      # invalidation unit tests) doesn't merely have "no review requirement"
      # -- review is not evaluable from this input at all.
      status = fixture["nodes"].nil? ? "not_evaluated" : "not_required"
      return Outcome.new(status: status, detail: nil, extra: {})
    end

    contract = doc.review_contract_catalog.fetch(contract_key)

    # Self-certification: a ModelApproval edge feeds a Decision directly and
    # no AuthorityAssertion node exists anywhere in the graph, yet a review
    # contract is still invoked over that decision. There is no independent
    # adjudicator at all, so any review here is the model reviewing itself.
    has_direct_model_approval = graph.edges.any? { |_, kind, _| kind == "ModelApproval" }
    if has_direct_model_approval && !has_assertion
      return Outcome.new(status: "fail", detail: "SelfCertification", extra: {})
    end

    evaluations = resolve_evaluations(fixture, contract_key, doc)
    return Outcome.new(status: "not_evaluated", detail: nil, extra: {}) unless evaluations

    by_name = evaluations.map { |ev| ev.to_a.first }.to_h
    required_predicates = contract["predicates"] || []

    # §7 rule 1: any current False yields Fail with (here: the first) false finding.
    false_pred = required_predicates.find { |p| by_name[p] == false }
    if false_pred
      code = FAIL_CODE_BY_PREDICATE[false_pred] || "PredicateFailed:#{false_pred}"
      return Outcome.new(status: "fail", detail: code, extra: {})
    end

    # §7 rule 2: missing / stale / revoked / Unknown evaluations, precedence
    # Deny > Indeterminate > Escalate. A predicate the contract requires but
    # that has no evaluation at all is ReviewPredicateMissing.
    missing_pred = required_predicates.find { |p| !by_name.key?(p) }
    if missing_pred
      return Outcome.new(status: "fail", detail: "ReviewPredicateMissing", extra: { missing: missing_pred })
    end

    stale_pred = required_predicates.find { |p| by_name[p].is_a?(String) && by_name[p].start_with?("stale:") }
    if stale_pred
      behavior = by_name[stale_pred].split(":", 2)[1]
      case behavior
      when "Deny"
        return Outcome.new(status: "fail", detail: "ReviewPredicateFailedClosed", extra: {})
      when "Indeterminate"
        return Outcome.new(status: "indeterminate", detail: "ReviewPredicateIndeterminate", extra: {})
      when "Escalate"
        return Outcome.new(status: "fail", detail: "ReviewEscalationRequired", extra: {})
      end
    end

    # §7 rule 3: all predicates True -> the declared mode passes.
    Outcome.new(status: "pass", detail: contract["mode"], extra: {})
  end
end
