# frozen_string_literal: true

require_relative "influence"
require_relative "review"
require_relative "fact_provenance"
require_relative "model_graph"

# Independent admission / capability evaluator. Combines the influence,
# review and fact-provenance evaluators per:
#  - RFC 0029 §12 (admission checks, rejection reasons) and §19 ("mandatory
#    denies dominate permits");
#  - RFC 0029.a §13.2 (effect-authority tier ceiling; ExecuteIrreversible is
#    "unsupported unless an explicit high-risk profile admits it" -- no
#    fixture here declares such a profile);
#  - RFC 0029.a §5/§5.1 (an AuthorityAssertion establishes a new authority
#    for its output only when the assertion's own review/adjudication
#    contract passes -- a failed assertion deactivates the model's own
#    authority per §4 rule 4 but does not itself become "the" final
#    authority, matching every assertion-bearing fixture whose review fails
#    (I2, H3): they carry no final_authority despite the assertion existing).
module Admission
  # Cross-fixture pattern (profile -> the human authority a *passing*
  # AuthorityAssertion resolves to). Derived by reading which authority_catalog
  # entry is consistent with every EQ3/H2/K4/M1/I1/L3-style fixture's own
  # profile, per the task's explicit license to use fixture inputs+expected
  # blocks together to derive underdetermined rules.
  REVIEWER_AUTHORITY_BY_PROFILE = {
    "healthcare" => "clinician",
    "insurance" => "adjuster",
    "manufacturing" => "maintenance-supervisor",
    "kyc" => "compliance-reviewer",
    "logistics" => "customs-officer",
    # Added 2026-09-27 when fixture B3 (banking + a passing human review)
    # first exercised this: banking's authority_catalog defines no distinct
    # human-reviewer role separate from the institutional "bank" entry that
    # INSTITUTIONAL_AUTHORITY_BY_PROFILE below already uses for B1's
    # no-human case. Every other profile has two separate authorities
    # (an institution and a named human role); banking has only one. Both
    # tables point at the same catalog entry for banking, not because the
    # two concepts collapse in general, but because this corpus's banking
    # profile has never needed to distinguish them.
    "banking" => "bank",
  }.freeze

  # Profile -> the institutional authority behind a Decision/Capability
  # reached with no ModelInvocation and no AuthorityAssertion at all (a pure
  # fact/rule-driven decision), per B1 (banking) and L1 (logistics).
  INSTITUTIONAL_AUTHORITY_BY_PROFILE = {
    "banking" => "bank",
    "logistics" => "customs-authority",
  }.freeze

  Result = Struct.new(:provenance, :model_level, :model_authorized, :final_authority,
                       :admission_status, :admission_diagnostic,
                       :review_status, :review_detail,
                       :capability_status, :capability_effect_class,
                       :invalidation, keyword_init: true)

  def self.evaluate(fixture, doc)
    influence = Influence.evaluate(fixture, doc)
    review = Review.evaluate(fixture, doc)
    fact_issue = FactProvenance.evaluate(fixture)
    graph = ModelGraph.build(fixture)

    if influence.indeterminate
      return Result.new(
        provenance: influence.provenance, model_level: influence.model_level,
        model_authorized: influence.model_authorized, final_authority: nil,
        admission_status: "indeterminate", admission_diagnostic: influence.diagnostic,
        review_status: review.status, review_detail: review.detail,
        capability_status: "not_issued", capability_effect_class: nil,
        invalidation: invalidation_for(fixture)
      )
    end

    mandatory_deny = fixture["mandatory_fact"] && fixture["mandatory_fact"]["value"] == true

    admission_status, admission_diagnostic =
      if mandatory_deny
        ["rejected", "MandatoryDeny"]
      elsif fact_issue
        ["rejected", fact_issue]
      elsif review.status == "fail" && review.detail == "SelfCertification"
        ["rejected", "ReviewSelfCertification"]
      elsif review.status == "fail"
        ["rejected", "ReviewProtocolFailed"]
      elsif review.status == "indeterminate"
        ["indeterminate", review.detail]
      elsif effect_tier_exceeded?(fixture, graph, influence)
        ["rejected", "InfluenceLevelExceedsProfile"]
      else
        ["accepted", nil]
      end

    cap_local_id, cap_decl = (fixture["capabilities"] || {}).to_a.first
    capability_status, capability_effect_class =
      if cap_local_id && admission_status == "accepted"
        ["issued", cap_decl["effect_class"]]
      else
        ["not_issued", nil]
      end

    has_assertion = graph.nodes.value?("AuthorityAssertion")
    has_model = graph.nodes.value?("ModelInvocation")
    final_authority =
      if has_assertion
        review.status == "pass" ? REVIEWER_AUTHORITY_BY_PROFILE[fixture["profile"]] : nil
      elsif !has_model && graph.edges.any? { |_, k, _| k == "Authorize" }
        INSTITUTIONAL_AUTHORITY_BY_PROFILE[fixture["profile"]]
      end

    # MandatoryDeny fixtures (M4, L5) still carry an assertion+passing review
    # (handled by the `has_assertion` branch above) even though admission is
    # rejected; a pure institutional path (B1/L1) has no assertion, so it
    # falls to the elsif branch above regardless of mandatory_deny.

    Result.new(
      provenance: influence.provenance, model_level: influence.model_level,
      model_authorized: influence.model_authorized, final_authority: final_authority,
      admission_status: admission_status, admission_diagnostic: admission_diagnostic,
      review_status: review.status, review_detail: review.detail,
      capability_status: capability_status, capability_effect_class: capability_effect_class,
      invalidation: invalidation_for(fixture)
    )
  end

  def self.invalidation_for(fixture)
    dep = fixture["dependency"]
    dep ? dep["action"] : "none"
  end

  # RFC 0029.a §13.2: ExecuteIrreversible is unsupported absent an explicit
  # high-risk profile (none exists in this corpus), so any Effect reached via
  # a direct ModelTrigger edge whose reversibility_class is Irreversible
  # always exceeds the model's permitted authority. Separately, an Effect
  # reached only via Issue (i.e. a capability consumed downstream, not model-
  # triggered) must not require a tier stronger than the tier actually
  # established for that capability's Decision/Capability chain.
  def self.effect_tier_exceeded?(fixture, graph, influence)
    effect_ids = graph.nodes.select { |_, k| k == "Effect" }.keys
    return false if effect_ids.empty?

    effect_ids.any? do |eid|
      eff = fixture.dig("effects", eid)
      next false unless eff

      required_level = Influence::REVERSIBILITY_TO_LEVEL[eff["reversibility_class"]]
      required_rank = Influence::RANK.fetch(required_level)
      via_model_trigger = graph.in_edges[eid].any? { |_, kind| kind == "ModelTrigger" }

      if via_model_trigger
        required_rank == Influence::RANK.fetch("ExecuteIrreversible")
      else
        influence.model_authorized == true && Influence::RANK.fetch(influence.model_level) < required_rank
      end
    end
  end
end
