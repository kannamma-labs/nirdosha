# frozen_string_literal: true

# Independent fact-provenance evaluator, derived from:
#  - RFC 0029 §17 (fact provenance: issuer, freshness/validity, revocation
#    status, authorized consumers);
#  - RFC 0029.a §15 ("A decision capability is invalid if ... the model was
#    not permitted for that effect scope, required assurance evidence expired
#    before effect commit, the receipt does not match ...");
#  - the symbolic source's own lossless_expansion note: "checked against a
#    reachable Fact's fact_provenance by a dedicated evaluator, never by
#    compute_influence."
#
# This is deliberately a separate module from Influence: a Decision's
# `fact_requirement` (required_model, required_deployment) is checked against
# a Fact node's `fact_provenance` (produced_by_model, produced_by_deployment,
# valid_until, revoked, authorized_consumers), independent of the influence
# fold.
module FactProvenance
  EVAL_NOW = "2026-09-27T00:00:00Z"

  # Returns nil (no defect / not applicable) or one of
  # FactProvenanceMissing | FactWrongModel | FactWrongDeployment |
  # FactRevoked | FactStale | FactUnauthorizedConsumer.
  #
  # Found by the 2026-09-27 differential re-run against fixture L14 (which
  # declares a fact_requirement but no fact_provenance at all, on purpose):
  # this previously returned nil ("not applicable") whenever fact_provenance
  # was absent, silently treating a *declared but unmet* requirement the
  # same as "no requirement exists." A Decision that declares
  # required_model/required_deployment is asserting that some fact must
  # satisfy it; the absence of any such fact is a failure of that
  # requirement, not a reason to skip checking it.
  def self.evaluate(fixture)
    fact_requirement = fixture["fact_requirement"]
    return nil unless fact_requirement

    decision_id, req = fact_requirement.to_a.first
    return nil unless req

    fact_provenance = fixture["fact_provenance"]
    entry = fact_provenance && fact_provenance.to_a.first
    fp = entry && entry[1]
    return "FactProvenanceMissing" unless fp

    required_model = req["required_model"]
    required_deployment = req["required_deployment"]
    produced_model = fp["produced_by_model"]
    produced_deployment = fp["produced_by_deployment"]

    return "FactWrongModel" if produced_model != required_model
    return "FactWrongDeployment" if produced_deployment != required_deployment
    return "FactRevoked" if fp["revoked"]
    return "FactStale" if fp["valid_until"] && fp["valid_until"] <= EVAL_NOW

    authorized_consumers = fp["authorized_consumers"] || []
    return "FactUnauthorizedConsumer" unless authorized_consumers.include?(decision_id)

    nil
  end
end
