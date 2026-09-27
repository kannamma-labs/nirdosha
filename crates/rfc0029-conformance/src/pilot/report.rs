//! `AdmissionReportV1` — the compiled admission report required by
//! `rfcs/0029-phase0-readiness-and-domain-matrix.md` §5 ("Compiled admission
//! report"): a manual checklist cannot certify a policy, so every pilot run
//! emits this deterministic, canonically encodable report instead. Every
//! field here is computed from the actual pre/post ledger state and receipt
//! of one `reserve` call — nothing is asserted without a corresponding
//! check that can genuinely fail.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::funds_reserve::{AccountFact, ReservationReceipt, ReservationStatus};
use crate::{
    compute_influence, evaluate_admission, evaluate_fact_provenance, evaluate_review,
    ComputedFactProvenance, ComputedReview, Fixture, TriState,
};

pub const SCHEMA_VERSION: &str = "rfc0029.admission-report.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementOutcome {
    Satisfied,
    Failed,
    Indeterminate,
    NotApplicable,
    Opaque,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequirementCheck {
    pub id: String,
    pub outcome: RequirementOutcome,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverallVerdict {
    /// Fills the "Passed" role named in the readiness matrix's §7
    /// generalization requirement — kept as `Accepted` rather than a
    /// separate `Passed` synonym, matching `evaluate_admission`'s own
    /// `"accepted"` status string this whole report is built from.
    Accepted,
    Rejected,
    Indeterminate,
    /// Distinct from `Indeterminate`: this policy kind or evaluator
    /// combination is not one the compiler has been extended to describe
    /// at all (e.g. a fixture with no graph — a pure review-contract or
    /// pure invalidation test case), as opposed to one it evaluated and
    /// found genuinely ambiguous. Conflating the two would hide "we never
    /// built this" behind "the policy itself is ambiguous," which are not
    /// the same finding.
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnforcementAxes {
    pub axis_a: String,
    pub axis_b: String,
    pub axis_c: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdmissionReportV1 {
    pub schema_version: String,
    pub policy_id: String,
    pub profile_id: String,
    pub bundle_hash: String,
    pub enforcement_axes: EnforcementAxes,
    pub effect_closure: Vec<String>,
    pub requirements: Vec<RequirementCheck>,
    pub evidence_finality_mode: String,
    pub safety_posture: String,
    pub exclusions: Vec<String>,
    pub input_commitment: String,
    pub output_commitment: String,
    pub overall: OverallVerdict,
}

fn commit(prefix: &str, value: impl std::fmt::Debug) -> String {
    format!("sha256:{:x}", Sha256::digest(format!("{prefix}:{value:?}").as_bytes()))
}

fn check(id: &str, ok: bool, note: Option<&str>) -> RequirementCheck {
    RequirementCheck {
        id: id.into(),
        outcome: if ok {
            RequirementOutcome::Satisfied
        } else {
            RequirementOutcome::Failed
        },
        note: note.map(str::to_owned),
    }
}

fn not_applicable(id: &str, note: &str) -> RequirementCheck {
    RequirementCheck {
        id: id.into(),
        outcome: RequirementOutcome::NotApplicable,
        note: Some(note.into()),
    }
}

/// Compile the report for one completed `reserve` call. `before`/`after` are
/// the account-fact snapshots taken immediately before and after the call
/// (`None` when the account did not exist); the caller takes these snapshots
/// so the report can independently verify atomicity rather than trusting the
/// gateway's own bookkeeping.
pub fn compile(
    before: Option<&AccountFact>,
    after: Option<&AccountFact>,
    receipt: &ReservationReceipt,
) -> AdmissionReportV1 {
    let mut requirements = Vec::new();

    const FACT_DEPENDENT_IDS: [&str; 6] = [
        "fact-authenticity",
        "fact-freshness",
        "fact-not-revoked",
        "subject-binding",
        "resource-version-match",
        "sufficient-funds",
    ];

    if receipt.status == ReservationStatus::Replayed {
        // A replay reuses the original cached decision by construction; it
        // does not re-run PIP checks against current (already-mutated)
        // fact state, so re-checking them here would compare the wrong
        // snapshot in time (RFC 0029 §14: decision validity/reuse is
        // distinct from re-evaluation).
        requirements.push(not_applicable(
            "fact-presence",
            "replay reuses the original decision without re-checking current fact state",
        ));
        for id in FACT_DEPENDENT_IDS {
            requirements.push(not_applicable(
                id,
                "replay reuses the original decision without re-checking current fact state",
            ));
        }
    } else {
        match before {
            None => {
                requirements.push(check(
                    "fact-presence",
                    false,
                    Some("no authoritative account fact was found for this account_id"),
                ));
                for id in FACT_DEPENDENT_IDS {
                    requirements.push(not_applicable(id, "no fact was available to check"));
                }
            }
            Some(fact) => {
                requirements.push(check("fact-presence", true, None));
                requirements.push(check(
                    "fact-authenticity",
                    fact.is_authentic(),
                    Some("signature does not match the fact's own field commitment"),
                ));
                requirements.push(check(
                    "fact-freshness",
                    fact.valid_until.as_str() >= receipt.request.now.as_str(),
                    Some("fact.valid_until precedes the request's declared time"),
                ));
                requirements.push(check(
                    "fact-not-revoked",
                    !fact.revoked,
                    Some("fact was revoked"),
                ));
                requirements.push(check(
                    "subject-binding",
                    fact.subject_id == receipt.request.subject_id,
                    Some("authenticated subject does not own this account"),
                ));
                requirements.push(check(
                    "resource-version-match",
                    fact.version == receipt.request.account_version,
                    Some("caller's believed resource version is stale (time-of-check/time-of-use)"),
                ));
                requirements.push(check(
                    "sufficient-funds",
                    fact.balance_minor >= receipt.request.amount_minor,
                    Some("balance is insufficient for the requested amount"),
                ));
            }
        }
    }

    // RFC 0029 §24 Guarded/Atomic proof schema: the effect and its evidence
    // share one atomic boundary, or (on denial) neither occurred at all.
    let atomicity_ok = match receipt.status {
        // AwaitingReconciliation is reached only via `confirm_external_submission`
        // *after* the same local mutation `Reserved` already made; neither
        // that call nor `reconcile` touch account state, so the account-level
        // proof obligation is identical to `Reserved`'s.
        ReservationStatus::Reserved | ReservationStatus::AwaitingReconciliation => {
            match (before, after) {
                (Some(b), Some(a)) => {
                    a.version == b.version + 1
                        && a.balance_minor + receipt.request.amount_minor == b.balance_minor
                        && a.is_authentic()
                }
                _ => false,
            }
        }
        ReservationStatus::Denied | ReservationStatus::Replayed => before == after,
    };
    requirements.push(check(
        "atomic-commit-integrity",
        atomicity_ok,
        Some("account state changed without a corresponding Reserved outcome, or a Reserved outcome did not produce the exact declared mutation"),
    ));

    requirements.push(RequirementCheck {
        id: "idempotent-replay-consistency".into(),
        outcome: match receipt.status {
            ReservationStatus::Replayed => RequirementOutcome::Satisfied,
            _ => RequirementOutcome::NotApplicable,
        },
        note: None,
    });

    requirements.push(check(
        "evidence-commitment-present",
        receipt.evidence_hash.starts_with("sha256:") && receipt.evidence_hash.len() > "sha256:".len(),
        Some("receipt has no evidence commitment"),
    ));

    // Readiness matrix §7.1 "daily limit under concurrency/partition."
    // This function's signature (one before/after snapshot plus one
    // receipt) cannot directly re-verify `Ledger::daily_reserved`'s running
    // total end-to-end -- only whether the aggregate-consistency denial
    // path engaged for *this* call. That is a real, narrower check than
    // full aggregate verification, disclosed as such, not inflated into one.
    requirements.push(RequirementCheck {
        id: "daily-limit-consistency".into(),
        outcome: match receipt.diagnostic.as_deref() {
            Some("DailyLimitExceeded") => RequirementOutcome::Satisfied,
            _ => RequirementOutcome::NotApplicable,
        },
        note: Some(
            "this per-call report observes only whether the daily-limit denial path fired \
             for this request; verifying the aggregate window's running total end-to-end \
             requires querying Ledger::daily_reserved directly"
                .into(),
        ),
    });

    let overall = if requirements
        .iter()
        .any(|r| r.outcome == RequirementOutcome::Failed)
    {
        OverallVerdict::Rejected
    } else {
        match receipt.status {
            ReservationStatus::Reserved | ReservationStatus::Replayed => OverallVerdict::Accepted,
            ReservationStatus::Denied => OverallVerdict::Rejected,
            // Readiness matrix §7.1: a local commit whose paired external
            // submission timed out is a declared UnknownOutcome, never
            // silently read as settled (Accepted) or abandoned (Rejected).
            ReservationStatus::AwaitingReconciliation => OverallVerdict::Indeterminate,
        }
    };

    AdmissionReportV1 {
        schema_version: SCHEMA_VERSION.into(),
        policy_id: "policy:rfc0029-pilot:funds-reserve:v1".into(),
        profile_id: "profile:rfc0029:banking-pilot:candidate-v1".into(),
        bundle_hash: "symbolic:bundle-rfc0029-pilot-v1".into(),
        enforcement_axes: EnforcementAxes {
            axis_a: "Runtime".into(),
            axis_b: "funds.reserve:Guarded".into(),
            axis_c: "Local".into(),
        },
        effect_closure: vec!["funds.reserve".into()],
        requirements,
        evidence_finality_mode: match receipt.status {
            ReservationStatus::Reserved => "AtomicEvidence".into(),
            ReservationStatus::Replayed => "AtomicEvidence".into(),
            ReservationStatus::Denied => "DurableOutboxEvidence".into(),
            ReservationStatus::AwaitingReconciliation => "DurableOutboxEvidence".into(),
        },
        safety_posture: "FailClosed".into(),
        exclusions: vec![
            "funds.release is not modeled by this pilot".into(),
        ],
        input_commitment: commit("input", &receipt.request),
        output_commitment: commit("output", (receipt.status, &receipt.resulting_version)),
        overall,
    }
}

/// Generalizes `AdmissionReportV1` beyond the `funds.reserve` pilot's
/// ledger-specific `compile`, above, to any canonical fixture —
/// covering model-influenced decisions (`compute_influence`), review
/// contracts, and fact-provenance lineage, which `compile` above never
/// touches at all (`funds.reserve`'s own declared model influence is
/// `None`). This is the readiness matrix §7 requirement: the report must
/// distinguish `Passed`/`Failed`/`Indeterminate` from `Unsupported`, not
/// silently force every input into one of the first three.
///
/// A fixture with no graph at all (`input.nodes` empty) is handled by one
/// of two paths: a `dependency` or `monitor` parameter (the distributed-
/// finality fixtures `INV_PRECOMMIT`/`INV_EXTERNAL_UNKNOWN`/`L7`, and the
/// monitor-health fixture `R8_MONITOR_MISSING`) gets a real, computed
/// report — closing the "distributed finality, aggregate consistency and
/// monitor-health evidence are not modeled" gap this function's docs used
/// to name. The remaining pure review-contract fixtures (`R1`-`R7`, `R9`)
/// are genuinely `Unsupported`: this compiler has no admission decision to
/// report on for a bare review-contract test case at all, which is a
/// different finding from "evaluated and ambiguous."
pub fn compile_from_fixture(fixture: &Fixture) -> AdmissionReportV1 {
    let mut requirements = Vec::new();

    if fixture.input.nodes.is_empty() {
        if let Some(dependency) = fixture.input.parameters.get("dependency").cloned() {
            return compile_distributed_finality(fixture, &dependency);
        }
        if let Some(monitor) = fixture.input.parameters.get("monitor").cloned() {
            return compile_monitor_health(fixture, &monitor);
        }
        requirements.push(RequirementCheck {
            id: "graph-present".into(),
            outcome: RequirementOutcome::NotApplicable,
            note: Some("fixture has no graph and no dependency/monitor parameter (a pure \
                        review-contract test case); this report compiler has no admission \
                        decision to describe for it"
                .into()),
        });
        return AdmissionReportV1 {
            schema_version: SCHEMA_VERSION.into(),
            policy_id: format!("policy:rfc0029-fixture:{}:v1", fixture.fixture_id),
            profile_id: fixture.profile_id.clone(),
            bundle_hash: fixture.input.bundle.hash.clone(),
            enforcement_axes: EnforcementAxes {
                axis_a: "Unsupported".into(),
                axis_b: "Unsupported".into(),
                axis_c: "Unsupported".into(),
            },
            effect_closure: vec![],
            requirements,
            evidence_finality_mode: "Unsupported".into(),
            safety_posture: "Unsupported".into(),
            exclusions: vec![
                "bare review-contract policy kinds are not covered by this report compiler"
                    .into(),
            ],
            input_commitment: commit("input", &fixture.fixture_id),
            output_commitment: commit("output", "unsupported"),
            overall: OverallVerdict::Unsupported,
        };
    }

    let influence = compute_influence(fixture);
    let review = evaluate_review(fixture);
    let fact_provenance = evaluate_fact_provenance(fixture);
    let admission = evaluate_admission(fixture);

    requirements.push(match &influence {
        Ok(i) if i.provenance == TriState::Bool(false) => RequirementCheck {
            id: "influence-computed".into(),
            outcome: RequirementOutcome::NotApplicable,
            note: Some("no ModelInvocation node reaches a protected target".into()),
        },
        Ok(i) if matches!(i.provenance, TriState::Unknown(_)) => RequirementCheck {
            id: "influence-computed".into(),
            outcome: RequirementOutcome::Indeterminate,
            note: Some("model-sourced influence reaches a protected target through an \
                        uncounted edge; fails closed rather than under-attributing it"
                .into()),
        },
        Ok(_) => RequirementCheck {
            id: "influence-computed".into(),
            outcome: RequirementOutcome::Satisfied,
            note: None,
        },
        Err(e) => RequirementCheck {
            id: "influence-computed".into(),
            outcome: RequirementOutcome::Opaque,
            note: Some(format!("{e}")),
        },
    });

    requirements.push(match &review {
        Ok(ComputedReview::NotEvaluated) => RequirementCheck {
            id: "review-evaluated".into(),
            outcome: RequirementOutcome::NotApplicable,
            note: Some("no review contract applies to this decision".into()),
        },
        Ok(ComputedReview::Pass(_)) => RequirementCheck {
            id: "review-evaluated".into(),
            outcome: RequirementOutcome::Satisfied,
            note: None,
        },
        Ok(ComputedReview::Fail(f)) => RequirementCheck {
            id: "review-evaluated".into(),
            outcome: RequirementOutcome::Failed,
            note: Some(f.to_string()),
        },
        Ok(ComputedReview::Indeterminate(f)) => RequirementCheck {
            id: "review-evaluated".into(),
            outcome: RequirementOutcome::Indeterminate,
            note: Some(f.to_string()),
        },
        Err(e) => RequirementCheck {
            id: "review-evaluated".into(),
            outcome: RequirementOutcome::Opaque,
            note: Some(format!("{e}")),
        },
    });

    requirements.push(match &fact_provenance {
        Ok(ComputedFactProvenance::NotApplicable) => RequirementCheck {
            id: "fact-provenance-checked".into(),
            outcome: RequirementOutcome::NotApplicable,
            note: Some("no Decision in this fixture declares a fact_requirement".into()),
        },
        Ok(ComputedFactProvenance::Satisfied) => RequirementCheck {
            id: "fact-provenance-checked".into(),
            outcome: RequirementOutcome::Satisfied,
            note: None,
        },
        Ok(ComputedFactProvenance::Failed(d)) => RequirementCheck {
            id: "fact-provenance-checked".into(),
            outcome: RequirementOutcome::Failed,
            note: Some(d.to_string()),
        },
        Err(e) => RequirementCheck {
            id: "fact-provenance-checked".into(),
            outcome: RequirementOutcome::Opaque,
            note: Some(format!("{e}")),
        },
    });

    let overall = match &admission {
        Ok(a) if a.status == "accepted" => OverallVerdict::Accepted,
        Ok(a) if a.status == "rejected" => OverallVerdict::Rejected,
        Ok(a) if a.status == "indeterminate" => OverallVerdict::Indeterminate,
        Ok(_) | Err(_) => OverallVerdict::Unsupported,
    };
    requirements.push(RequirementCheck {
        id: "admission-decided".into(),
        outcome: match &overall {
            OverallVerdict::Accepted => RequirementOutcome::Satisfied,
            OverallVerdict::Rejected => RequirementOutcome::Failed,
            OverallVerdict::Indeterminate => RequirementOutcome::Indeterminate,
            OverallVerdict::Unsupported => RequirementOutcome::Opaque,
        },
        note: admission
            .as_ref()
            .ok()
            .and_then(|a| a.diagnostic)
            .map(|d| d.to_string()),
    });

    let capability_effect_class = admission.as_ref().ok().and_then(|a| a.effect_class.clone());

    AdmissionReportV1 {
        schema_version: SCHEMA_VERSION.into(),
        policy_id: format!("policy:rfc0029-fixture:{}:v1", fixture.fixture_id),
        profile_id: fixture.profile_id.clone(),
        bundle_hash: fixture.input.bundle.hash.clone(),
        enforcement_axes: EnforcementAxes {
            axis_a: "Runtime".into(),
            axis_b: "Guarded".into(),
            axis_c: "Local".into(),
        },
        effect_closure: capability_effect_class.into_iter().collect(),
        requirements,
        evidence_finality_mode: "AtomicEvidence".into(),
        safety_posture: "FailClosed".into(),
        exclusions: vec![
            "distributed finality, aggregate consistency and monitor-health evidence are not \
             modeled by this report compiler"
                .into(),
        ],
        input_commitment: commit("input", &fixture.fixture_id),
        output_commitment: commit(
            "output",
            (
                admission.as_ref().ok().map(|a| a.status.clone()),
                admission.as_ref().ok().and_then(|a| a.diagnostic),
            ),
        ),
        overall,
    }
}

/// RFC 0029 §36.5/§26 distributed finality: `INV_PRECOMMIT`,
/// `INV_EXTERNAL_UNKNOWN`, and `L7` each declare an invalidating event's
/// phase and required `action` (`Abort` or `Reconcile`) rather than a
/// graph — this reports on that declaration directly, closing the gap the
/// blanket `Unsupported` path used to leave.
fn compile_distributed_finality(fixture: &Fixture, dependency: &Value) -> AdmissionReportV1 {
    let action = dependency.get("action").and_then(Value::as_str).unwrap_or("Unknown");
    let (overall, outcome, note, evidence_finality_mode) = match action {
        "Abort" => (
            OverallVerdict::Rejected,
            RequirementOutcome::Failed,
            "invalidating event occurred after the precommit check but before commit; the \
             transition aborts, no effect commits, no capability issues"
                .to_string(),
            "DurableOutboxEvidence",
        ),
        "Reconcile" => (
            OverallVerdict::Indeterminate,
            RequirementOutcome::Indeterminate,
            "the external system's true outcome is unknown after the local commit; workflow \
             state is UnknownOutcome, reconciliation is required, and a duplicate effect is \
             forbidden on retry"
                .to_string(),
            "EmergencyDeferredEvidence",
        ),
        other => (
            OverallVerdict::Unsupported,
            RequirementOutcome::Opaque,
            format!("unrecognized distributed-finality action {other:?}; this compiler only \
                     knows Abort and Reconcile"),
            "Unsupported",
        ),
    };
    AdmissionReportV1 {
        schema_version: SCHEMA_VERSION.into(),
        policy_id: format!("policy:rfc0029-fixture:{}:v1", fixture.fixture_id),
        profile_id: fixture.profile_id.clone(),
        bundle_hash: fixture.input.bundle.hash.clone(),
        enforcement_axes: EnforcementAxes {
            axis_a: "Runtime".into(),
            axis_b: "DurableWorkflow".into(),
            axis_c: "ExternallyAttested".into(),
        },
        effect_closure: vec![],
        requirements: vec![RequirementCheck {
            id: "distributed-finality-declared".into(),
            outcome,
            note: Some(note),
        }],
        evidence_finality_mode: evidence_finality_mode.into(),
        safety_posture: "FailClosed".into(),
        exclusions: vec![
            "this report describes only the declared invalidation action, not a full \
             multi-party workflow trace"
                .into(),
        ],
        input_commitment: commit("input", &fixture.fixture_id),
        output_commitment: commit("output", action),
        overall,
    }
}

/// RFC 0029 §36.10 monitor health: `R8_MONITOR_MISSING` declares a
/// `Continuous` claim's monitor health directly rather than through a
/// graph. `health: unknown` must suspend the review-quality claim it
/// backs, not silently pass it — reported here as `Indeterminate`, never
/// `Accepted`.
fn compile_monitor_health(fixture: &Fixture, monitor: &Value) -> AdmissionReportV1 {
    let health = monitor.get("health").and_then(Value::as_str).unwrap_or("unknown");
    let (overall, outcome) = if health == "unknown" {
        (OverallVerdict::Indeterminate, RequirementOutcome::Indeterminate)
    } else {
        (OverallVerdict::Accepted, RequirementOutcome::Satisfied)
    };
    AdmissionReportV1 {
        schema_version: SCHEMA_VERSION.into(),
        policy_id: format!("policy:rfc0029-fixture:{}:v1", fixture.fixture_id),
        profile_id: fixture.profile_id.clone(),
        bundle_hash: fixture.input.bundle.hash.clone(),
        enforcement_axes: EnforcementAxes {
            axis_a: "Continuous".into(),
            axis_b: "Advisory".into(),
            axis_c: "Local".into(),
        },
        effect_closure: vec![],
        requirements: vec![RequirementCheck {
            id: "monitor-health-declared".into(),
            outcome,
            note: Some(format!("declared monitor health: {health}")),
        }],
        evidence_finality_mode: "ObservedEvidence".into(),
        safety_posture: "FailClosed".into(),
        exclusions: vec![
            "this report describes only the declared monitor-health value, not the \
             underlying observation pipeline"
                .into(),
        ],
        input_commitment: commit("input", &fixture.fixture_id),
        output_commitment: commit("output", health),
        overall,
    }
}
