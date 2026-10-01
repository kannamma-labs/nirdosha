use rfc0029_conformance::pilot::funds_reserve::{AccountFact, Ledger, ReservationRequest, ReservationStatus};
use rfc0029_conformance::pilot::report::{self, OverallVerdict, RequirementOutcome};

const NOW: &str = "2026-09-27T00:00:00Z";

fn fresh_ledger() -> Ledger {
    let mut ledger = Ledger::new();
    ledger.put_fact(AccountFact::signed("acct:1", "subject:alice", 10_000, 1, "2026-09-28T00:00:00Z"));
    ledger
}

fn request(request_id: &str, amount_minor: u64) -> ReservationRequest {
    ReservationRequest {
        request_id: request_id.into(),
        subject_id: "subject:alice".into(),
        account_id: "acct:1".into(),
        amount_minor,
        account_version: 1,
        now: NOW.into(),
    }
}

#[test]
fn accepted_reservation_mutates_atomically_and_reports_accepted() {
    let mut ledger = fresh_ledger();
    let before = ledger.accounts.get("acct:1").cloned();
    let req = request("req:1", 2_500);
    let receipt = ledger.reserve(&req);
    let after = ledger.accounts.get("acct:1").cloned();

    assert_eq!(receipt.status, ReservationStatus::Reserved);
    assert_eq!(after.as_ref().unwrap().balance_minor, 7_500);
    assert_eq!(after.as_ref().unwrap().version, 2);

    let rep = report::compile(before.as_ref(), after.as_ref(), &receipt);
    assert_eq!(rep.overall, OverallVerdict::Accepted);
    assert!(rep
        .requirements
        .iter()
        .all(|r| r.outcome == RequirementOutcome::Satisfied || r.outcome == RequirementOutcome::NotApplicable));
}

#[test]
fn identical_retry_replays_without_double_reservation() {
    let mut ledger = fresh_ledger();
    let req = request("req:2", 1_000);
    let first = ledger.reserve(&req);
    let balance_after_first = ledger.accounts["acct:1"].balance_minor;

    let before = ledger.accounts.get("acct:1").cloned();
    let second = ledger.reserve(&req);
    let after = ledger.accounts.get("acct:1").cloned();

    assert_eq!(first.status, ReservationStatus::Reserved);
    assert_eq!(second.status, ReservationStatus::Replayed);
    assert_eq!(second.resulting_version, first.resulting_version);
    assert_eq!(ledger.accounts["acct:1"].balance_minor, balance_after_first);

    let rep = report::compile(before.as_ref(), after.as_ref(), &second);
    assert_eq!(rep.overall, OverallVerdict::Accepted);
}

#[test]
fn changed_input_with_reused_request_id_is_rejected_deterministically() {
    let mut ledger = fresh_ledger();
    let req = request("req:3", 1_000);
    let first = ledger.reserve(&req);
    assert_eq!(first.status, ReservationStatus::Reserved);

    let mut mutated = req.clone();
    mutated.amount_minor = 2_000;
    let second = ledger.reserve(&mutated);
    assert_eq!(second.status, ReservationStatus::Denied);
    assert_eq!(second.diagnostic.as_deref(), Some("IdempotencyKeyReuseMismatch"));
}

#[test]
fn unknown_account_is_denied_and_report_marks_dependent_checks_not_applicable() {
    let mut ledger = Ledger::new();
    let req = request("req:4", 1_000);
    let receipt = ledger.reserve(&req);
    assert_eq!(receipt.status, ReservationStatus::Denied);
    assert_eq!(receipt.diagnostic.as_deref(), Some("UnknownAccount"));

    let rep = report::compile(None, None, &receipt);
    assert_eq!(rep.overall, OverallVerdict::Rejected);
    let presence = rep.requirements.iter().find(|r| r.id == "fact-presence").unwrap();
    assert_eq!(presence.outcome, RequirementOutcome::Failed);
    let funds = rep.requirements.iter().find(|r| r.id == "sufficient-funds").unwrap();
    assert_eq!(funds.outcome, RequirementOutcome::NotApplicable);
}

#[test]
fn revoked_fact_is_denied() {
    let mut ledger = fresh_ledger();
    // Revoke through the authority's own re-issuance path so the
    // authenticity check still passes; only revocation should fail here,
    // isolating this negative path from fact-tampering detection.
    ledger.accounts.get_mut("acct:1").unwrap().revoke();

    let before = ledger.accounts.get("acct:1").cloned();
    let receipt = ledger.reserve(&request("req:5", 1_000));
    assert_eq!(receipt.diagnostic.as_deref(), Some("RevokedFact"));
    let after = ledger.accounts.get("acct:1").cloned();
    let rep = report::compile(before.as_ref(), after.as_ref(), &receipt);
    assert_eq!(rep.overall, OverallVerdict::Rejected);
    assert_eq!(
        rep.requirements.iter().find(|r| r.id == "fact-not-revoked").unwrap().outcome,
        RequirementOutcome::Failed
    );
}

#[test]
fn stale_fact_is_denied() {
    let mut ledger = Ledger::new();
    ledger.put_fact(AccountFact::signed("acct:1", "subject:alice", 10_000, 1, "2026-09-01T00:00:00Z"));
    let receipt = ledger.reserve(&request("req:6", 1_000));
    assert_eq!(receipt.diagnostic.as_deref(), Some("StaleFact"));
}

#[test]
fn subject_mismatch_is_denied() {
    let mut ledger = fresh_ledger();
    let mut req = request("req:7", 1_000);
    req.subject_id = "subject:mallory".into();
    let receipt = ledger.reserve(&req);
    assert_eq!(receipt.diagnostic.as_deref(), Some("SubjectMismatch"));
}

#[test]
fn resource_version_mismatch_is_denied() {
    let mut ledger = fresh_ledger();
    let mut req = request("req:8", 1_000);
    req.account_version = 99;
    let receipt = ledger.reserve(&req);
    assert_eq!(receipt.diagnostic.as_deref(), Some("ResourceVersionMismatch"));
}

#[test]
fn insufficient_funds_is_denied() {
    let mut ledger = fresh_ledger();
    let receipt = ledger.reserve(&request("req:9", 999_999));
    assert_eq!(receipt.diagnostic.as_deref(), Some("InsufficientFunds"));
}

#[test]
fn simulated_commit_failure_leaves_state_untouched_and_report_proves_it() {
    let mut ledger = fresh_ledger();
    ledger.commit_should_fail = true;
    let before = ledger.accounts.get("acct:1").cloned();
    let receipt = ledger.reserve(&request("req:10", 1_000));
    let after = ledger.accounts.get("acct:1").cloned();

    assert_eq!(receipt.diagnostic.as_deref(), Some("CommitFailed"));
    assert_eq!(before, after, "a failed commit must not mutate account state");

    let rep = report::compile(before.as_ref(), after.as_ref(), &receipt);
    assert_eq!(rep.overall, OverallVerdict::Rejected);
    assert_eq!(
        rep.requirements
            .iter()
            .find(|r| r.id == "atomic-commit-integrity")
            .unwrap()
            .outcome,
        RequirementOutcome::Satisfied,
        "denial correctly left state untouched, so atomicity itself holds even though the request was denied"
    );
}

#[test]
fn a_tampered_fact_fails_authenticity_and_the_report_catches_it() {
    let mut ledger = fresh_ledger();
    // Simulate tampering: mutate a field without updating the signature.
    ledger.accounts.get_mut("acct:1").unwrap().balance_minor = 999_999;
    let before = ledger.accounts.get("acct:1").cloned();
    let receipt = ledger.reserve(&request("req:11", 1_000));
    let after = ledger.accounts.get("acct:1").cloned();
    assert_eq!(receipt.diagnostic.as_deref(), Some("FactSignatureInvalid"));

    let rep = report::compile(before.as_ref(), after.as_ref(), &receipt);
    assert_eq!(
        rep.requirements
            .iter()
            .find(|r| r.id == "fact-authenticity")
            .unwrap()
            .outcome,
        RequirementOutcome::Failed
    );
}

#[test]
fn admission_report_is_byte_deterministic_across_independent_compiles() {
    let mut ledger_a = fresh_ledger();
    let before_a = ledger_a.accounts.get("acct:1").cloned();
    let receipt_a = ledger_a.reserve(&request("req:det", 2_500));
    let after_a = ledger_a.accounts.get("acct:1").cloned();
    let report_a = report::compile(before_a.as_ref(), after_a.as_ref(), &receipt_a);

    let mut ledger_b = fresh_ledger();
    let before_b = ledger_b.accounts.get("acct:1").cloned();
    let receipt_b = ledger_b.reserve(&request("req:det", 2_500));
    let after_b = ledger_b.accounts.get("acct:1").cloned();
    let report_b = report::compile(before_b.as_ref(), after_b.as_ref(), &receipt_b);

    let bytes_a = serde_json::to_vec(&report_a).unwrap();
    let bytes_b = serde_json::to_vec(&report_b).unwrap();
    assert_eq!(bytes_a, bytes_b, "two independent compiles of an equivalent run must be byte-identical");
    assert_eq!(report_a, report_b);
}
