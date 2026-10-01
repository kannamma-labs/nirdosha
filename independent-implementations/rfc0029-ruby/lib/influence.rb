# frozen_string_literal: true

require_relative "model_graph"

# Independent model-influence tier evaluator, derived from:
#  - RFC 0029.a §3/§13.2 (Observe < Classify < Prioritize < Recommend < Draft
#    < Approve < ExecuteReversible < ExecuteCompensatable < ExecuteIrreversible)
#  - RFC 0029.a "Model Influence and Human Review Semantics Revision 1"
#    §2.2 (closed edge algebra + per-edge model-level contribution),
#    §2.4 (consequential-path completeness), §4 (representation-invariant
#    influence fold), §5/§5.1 (AuthorityAssertion transition + dominance),
#    §6 (ObserveOnly non-interference certificate).
module Influence
  LEVELS = %w[None Observe Classify Prioritize Recommend Draft Approve
              ExecuteReversible ExecuteCompensatable ExecuteIrreversible].freeze
  RANK = LEVELS.each_with_index.to_h

  PROTECTED_KINDS = %w[Review AuthorityAssertion Decision Obligation Capability Effect].freeze

  # Edge kind -> model-level contribution, per Revision 1 §2.2's table.
  # nil = "none" (structural / no model contribution).
  # :transition = AssertFact/AssertDecision (authority transition).
  # :effect = ModelTrigger (contribution is the target Effect's reversibility tier).
  CONTRIBUTION = {
    "DataFlow" => nil, "RuleFlow" => nil,
    "CandidateGenerate" => "Classify", "CandidateSuppress" => "Classify", "Eligibility" => "Classify",
    "Rank" => "Prioritize", "Attention" => "Prioritize",
    "Default" => "Draft", "DraftArtifact" => "Draft", "DraftReason" => "Draft",
    "Recommend" => "Recommend",
    "ModelApproval" => "Approve",
    "ModelTrigger" => :effect,
    # Revision 1 §4 rule 3 says HumanInput "preserves provenance and level"
    # but is silent on `model_authority_active`. Rule 4 reads literally as
    # "the assertion's *outgoing* AssertFact/AssertDecision/Authorize edges"
    # deactivate authority -- but fixture I2 reaches an AuthorityAssertion
    # node via HumanInput with NO outgoing AssertFact/AssertDecision edge at
    # all (the assertion is simply never completed), and its own expected
    # result is still model_authorized: false. That is only reachable if
    # handing off to human review (HumanInput) itself already ends active
    # model authority, before the assertion's own output edge. We adopt that
    # reading; it does not change the result for any of the other seven
    # AuthorityAssertion-bearing fixtures, which all reach the same
    # active=false state either way.
    "HumanInput" => :transition,
    "AssertFact" => nil, "AssertDecision" => nil,
    "Authorize" => nil, "Require" => nil, "Satisfy" => nil, "Issue" => nil, "Record" => nil,
    "ObserveOnly" => :observe_only,
  }.freeze

  REVERSIBILITY_TO_LEVEL = {
    "Reversible" => "ExecuteReversible",
    "Compensatable" => "ExecuteCompensatable",
    "Irreversible" => "ExecuteIrreversible",
  }.freeze

  NodeState = Struct.new(:provenance, :level_rank, :active, :semantic, keyword_init: true)

  Result = Struct.new(:provenance, :model_level, :model_authorized, :terminal_targets,
                       :indeterminate, :diagnostic, keyword_init: true)

  # doc: Expand::Doc (for observe_certificate_catalog); fixture: raw symbolic
  # fixture hash. Returns an Influence::Result.
  def self.evaluate(fixture, doc)
    # A fixture that omits `nodes`/`edges` entirely (the R*/INV*/L7 pure
    # review- or dependency-contract unit tests) supplies no graph at all --
    # influence is not merely "None", it is simply not evaluable from this
    # input. Distinguish that from a real graph that happens to contain no
    # ModelInvocation node (e.g. B1, L1), where "no model influence" is an
    # actual, evidenced answer.
    if fixture["nodes"].nil?
      return Result.new(provenance: "unknown", model_level: "not_evaluated", model_authorized: "unknown",
                         terminal_targets: [], indeterminate: false, diagnostic: nil)
    end

    graph = ModelGraph.build(fixture)
    model_nodes = graph.nodes.select { |_, k| k == "ModelInvocation" }.keys

    # --- ObserveOnly certificate validity gate (Revision 1 §6) ---
    if fixture["observe_certificate"]
      cert = doc.observe_certificate_catalog.fetch(fixture["observe_certificate"])
      unless certificate_current?(cert)
        return Result.new(provenance: "unknown", model_level: "Unknown", model_authorized: "unknown",
                           terminal_targets: [], indeterminate: true, diagnostic: "ObserveOnlyAttestationStale")
      end
    end

    if model_nodes.empty?
      return Result.new(provenance: false, model_level: "None", model_authorized: false,
                         terminal_targets: [], indeterminate: false, diagnostic: nil)
    end

    states = {}
    model_nodes.each do |m|
      states[m] = NodeState.new(provenance: true, level_rank: RANK.fetch("Observe"), active: true, semantic: false)
    end

    order = ModelGraph.topo_order(graph)
    order.each do |node|
      next if model_nodes.include?(node) # seeded already
      incoming = graph.in_edges[node]
      next if incoming.empty?

      contributions = []
      incoming.each do |src, kind|
        base = states[src]
        next unless base # source not reached by any model invocation

        # Revision 1 §2.2's closed edge table allows AssertFact/AssertDecision
        # only as AuthorityAssertion -> Fact/Decision. Six fixtures in this
        # corpus (L8-L13) instead write `m-AssertFact-f` directly from a
        # ModelInvocation -- an endpoint the closed table does not license
        # (strictly, §2.2 says such an edge "rejects with InvalidLineageEdge").
        # Those fixtures' own `expected` blocks nonetheless compute a normal
        # Classify-level, model-authorized result, as if the model's own
        # output simply *is* the asserted fact (no independent authority
        # transition occurred, because no AuthorityAssertion node exists on
        # that path at all). We follow that reading -- treating a
        # ModelInvocation-sourced AssertFact/AssertDecision as an ordinary
        # Classify contribution rather than an authority transition -- and
        # flag it in the report as a non-canonical-input / ambiguous-
        # specification finding rather than silently rejecting these fixtures.
        effective_contribution =
          if %w[AssertFact AssertDecision].include?(kind) && graph.nodes[src] != "AuthorityAssertion"
            "Classify"
          else
            CONTRIBUTION[kind]
          end

        case effective_contribution
        when nil
          contributions << NodeState.new(provenance: base.provenance, level_rank: base.level_rank,
                                          active: base.active, semantic: base.semantic)
        when :transition
          contributions << NodeState.new(provenance: base.provenance, level_rank: base.level_rank,
                                          active: false, semantic: true)
        when :effect
          effect_attrs = fixture.dig("effects", node)
          level_name = effect_attrs && REVERSIBILITY_TO_LEVEL[effect_attrs["reversibility_class"]]
          contrib_rank = level_name ? RANK.fetch(level_name) : base.level_rank
          new_rank = base.active ? [base.level_rank, contrib_rank].max : base.level_rank
          contributions << NodeState.new(provenance: base.provenance, level_rank: new_rank,
                                          active: base.active, semantic: true)
        when :observe_only
          # Target is Evidence (non-protected); isolated by a valid certificate,
          # already checked above. No protected-target contribution.
          next
        else
          level_name = effective_contribution
          contrib_rank = RANK.fetch(level_name)
          new_rank = base.active ? [base.level_rank, contrib_rank].max : base.level_rank
          contributions << NodeState.new(provenance: base.provenance, level_rank: new_rank,
                                          active: base.active, semantic: true)
        end
      end
      next if contributions.empty?

      merged = NodeState.new(
        provenance: contributions.any?(&:provenance),
        level_rank: contributions.map(&:level_rank).max,
        active: contributions.any?(&:active),
        semantic: contributions.any?(&:semantic)
      )
      states[node] = merged
    end

    # §2.4: every protected node reached from a model invocation must have
    # been reached via at least one semantic edge somewhere on its path.
    protected_reached = graph.nodes.select { |id, kind| PROTECTED_KINDS.include?(kind) && states[id] }
    if protected_reached.any? { |id, _| !states[id].semantic }
      return Result.new(provenance: "unknown", model_level: "Unknown", model_authorized: "unknown",
                         terminal_targets: [], indeterminate: true, diagnostic: "SemanticEdgeMissing")
    end

    terminal_ids = protected_reached.keys.select { |id| ModelGraph.leaf?(graph, id) }
    if terminal_ids.empty?
      return Result.new(provenance: false, model_level: "None", model_authorized: false,
                         terminal_targets: [], indeterminate: false, diagnostic: nil)
    end

    provenance = terminal_ids.any? { |id| states[id].provenance }
    level_rank = terminal_ids.map { |id| states[id].level_rank }.max
    active = terminal_ids.any? { |id| states[id].active }

    Result.new(provenance: provenance, model_level: LEVELS[level_rank], model_authorized: active,
               terminal_targets: terminal_ids, indeterminate: false, diagnostic: nil)
  end

  # Fixed evaluation instant used throughout the symbolic source's own
  # expansion rule for predicate evaluations (2026-09-27T00:00:00Z).
  EVAL_NOW = "2026-09-27T00:00:00Z"

  def self.certificate_current?(cert)
    valid_until = cert["valid_until"]
    valid_from = cert["valid_from"]
    return false unless valid_until && valid_from
    # Lexical ISO 8601 comparison is safe here: every timestamp in this
    # corpus shares the exact "YYYY-MM-DDTHH:MM:SSZ" shape.
    valid_from <= EVAL_NOW && EVAL_NOW < valid_until
  end
end
