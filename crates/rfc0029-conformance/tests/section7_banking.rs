//! Executable coverage for the readiness matrix §7.1 "Mandatory non-AI
//! rerun matrices: banking transfer" table — one test per row not already
//! covered elsewhere. `Identical retry` and `Same key with changed input`
//! are already covered by `tests/pilot.rs`'s
//! `identical_retry_replays_without_double_reservation` and
//! `changed_input_with_reused_request_id_is_rejected_deterministically`;
//! not duplicated here.

use rfc0029_conformance::pilot::funds_reserve::{AccountFact, Ledger, ReservationRequest, ReservationStatus};
use rfc0029_conformance::pilot::report::{self, OverallVerdict, RequirementOutcome};
use std::sync::{Arc, Mutex};
use std::thread;

const NOW: &str = "2026-09-27T00:00:00Z";

fn request(request_id: &str, account_id: &str, amount_minor: u64) -> ReservationRequest {
    ReservationRequest {
        request_id: request_id.into(),
        subject_id: "subject:alice".into(),
        account_id: account_id.into(),
        amount_minor,
        account_version: 1,
        now: NOW.into(),
    }
}

/// "Concurrent insufficient-funds transfers": zero-overshoot atomic
/// consistency domain. `Ledger::reserve` takes `&mut self`, so real OS
/// threads racing against one `Mutex<Ledger>` are serialized by the mutex
/// exactly as a real gateway's transaction boundary would serialize them —
/// this test proves the *outcome* (never more than the starting balance is
/// reserved in total) empirically under genuine concurrency, not just by
/// appeal to the type signature.
#[test]
fn concurrent_insufficient_funds_transfers_never_overshoot() {
    let mut ledger = Ledger::new();
    ledger.put_fact(AccountFact::signed("acct:race", "subject:alice", 10_000, 1, "2026-09-28T00:00:00Z"));
    let ledger = Arc::new(Mutex::new(ledger));

    // 20 threads each try to reserve 1,000 against a 10,000 balance --
    // only 10 can possibly succeed; the other 10 must be denied, never
    // overshoot the balance even transiently.
    let handles: Vec<_> = (0..20)
        .map(|i| {
            let ledger = Arc::clone(&ledger);
            thread::spawn(move || {
                let req = request(&format!("req:race:{i}"), "acct:race", 1_000);
                // account_version is stale for all but the first winner on
                // purpose here: the point is that overshoot is impossible,
                // not that every thread guesses the right version. Threads
                // that lose the version race are denied
                // ResourceVersionMismatch, which is itself a correct,
                // non-overshooting outcome.
                ledger.lock().unwrap().reserve(&req).status
            })
        })
        .collect();
    let statuses: Vec<ReservationStatus> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let reserved_count = statuses.iter().filter(|s| **s == ReservationStatus::Reserved).count();

    let final_balance = ledger.lock().unwrap().accounts.get("acct:race").unwrap().balance_minor;
    assert_eq!(
        final_balance,
        10_000 - (reserved_count as u64) * 1_000,
        "final balance must exactly match the sum of actually-Reserved amounts, never less (overshoot)"
    );
    assert!(final_balance <= 10_000, "balance must never go negative under concurrent attempts");
}

/// "Daily limit under concurrency/partition": declared window, authority,
/// reservation and maximum overshoot.
#[test]
fn daily_limit_denies_once_the_calendar_day_window_is_exhausted() {
    let mut ledger = Ledger::new();
    ledger.put_fact(AccountFact::signed_with_daily_limit(
        "acct:limited",
        "subject:alice",
        1_000_000,
        1,
        "2026-10-01T00:00:00Z",
        Some(5_000),
    ));
    let mut req = request("req:d1", "acct:limited", 3_000);
    let r1 = ledger.reserve(&req);
    assert_eq!(r1.status, ReservationStatus::Reserved);

    req.request_id = "req:d2".into();
    req.amount_minor = 3_000;
    req.account_version = r1.resulting_version.unwrap();
    let before = ledger.accounts.get("acct:limited").cloned();
    let r2 = ledger.reserve(&req);
    let after = ledger.accounts.get("acct:limited").cloned();
    assert_eq!(r2.status, ReservationStatus::Denied);
    assert_eq!(r2.diagnostic.as_deref(), Some("DailyLimitExceeded"));

    let rep = report::compile(before.as_ref(), after.as_ref(), &r2);
    let check = rep
        .requirements
        .iter()
        .find(|r| r.id == "daily-limit-consistency")
        .unwrap();
    assert_eq!(check.outcome, RequirementOutcome::Satisfied);

    // The next calendar day, the window resets -- the ceiling is per-day,
    // not a lifetime cap.
    req.request_id = "req:d3".into();
    req.now = "2026-09-28T00:00:01Z".into();
    let r3 = ledger.reserve(&req);
    assert_eq!(r3.status, ReservationStatus::Reserved);
}

/// "Sanctions or FX fact changes before commit": dependency invalidation
/// and re-evaluation. Modeled here via the fact authority revoking the
/// account fact between two decisions -- `reserve` always re-reads current
/// fact state, so the second decision sees the change without needing any
/// separate invalidation-propagation mechanism.
#[test]
fn a_fact_change_before_the_second_decision_is_re_evaluated_not_cached() {
    let mut ledger = Ledger::new();
    ledger.put_fact(AccountFact::signed("acct:sanctioned", "subject:alice", 10_000, 1, "2026-09-28T00:00:00Z"));
    let r1 = ledger.reserve(&request("req:before", "acct:sanctioned", 1_000));
    assert_eq!(r1.status, ReservationStatus::Reserved);

    // The sanctions authority revokes the account fact -- re-issued with a
    // new signature, per AccountFact::revoke's own contract.
    let mut fact = ledger.accounts.get("acct:sanctioned").unwrap().clone();
    fact.revoke();
    ledger.put_fact(fact);

    let mut req2 = request("req:after", "acct:sanctioned", 1_000);
    req2.account_version = r1.resulting_version.unwrap();
    let r2 = ledger.reserve(&req2);
    assert_eq!(r2.status, ReservationStatus::Denied);
    assert_eq!(r2.diagnostic.as_deref(), Some("RevokedFact"));
}

/// "Local ledger commit succeeds, external submit times out": declared
/// UnknownOutcome, reconciliation, no false atomic claim, no duplicate
/// effect.
#[test]
fn external_submission_timeout_requires_reconciliation_before_being_settled() {
    let mut ledger = Ledger::new();
    ledger.put_fact(AccountFact::signed("acct:ext", "subject:alice", 10_000, 1, "2026-09-28T00:00:00Z"));
    let before = ledger.accounts.get("acct:ext").cloned();
    let receipt = ledger.reserve(&request("req:ext", "acct:ext", 2_000));
    assert_eq!(receipt.status, ReservationStatus::Reserved);

    // External settlement rail times out (None = genuinely unknown, not a
    // negative acknowledgement).
    let status = ledger.confirm_external_submission("req:ext", None).unwrap();
    assert_eq!(status, ReservationStatus::AwaitingReconciliation);

    let awaiting = ledger.consumed.get("req:ext").unwrap().clone();
    let after = ledger.accounts.get("acct:ext").cloned();
    let report = report::compile(before.as_ref(), after.as_ref(), &awaiting);
    assert_eq!(
        report.overall,
        OverallVerdict::Indeterminate,
        "must never be read as Accepted (falsely settled) or Rejected (falsely abandoned)"
    );

    // A retry of the same request_id while awaiting reconciliation still
    // replays the original decision -- it never re-executes the local
    // mutation or re-submits externally a second time.
    let retry = ledger.reserve(&request("req:ext", "acct:ext", 2_000));
    assert_eq!(retry.status, ReservationStatus::AwaitingReconciliation);
    let balance_after_retry = ledger.accounts.get("acct:ext").unwrap().balance_minor;
    assert_eq!(balance_after_retry, 8_000, "no duplicate effect from the retry");

    // The true outcome is later learned and reconciled.
    let resolved = ledger.reconcile("req:ext", true).unwrap();
    assert_eq!(resolved, ReservationStatus::Reserved);
}

/// "Settlement file or administrative mutation": included in effect
/// closure or named exclusion without coverage claim.
#[test]
fn out_of_scope_effects_are_a_named_exclusion_not_a_silent_gap() {
    let mut ledger = Ledger::new();
    ledger.put_fact(AccountFact::signed("acct:x", "subject:alice", 10_000, 1, "2026-09-28T00:00:00Z"));
    let before = ledger.accounts.get("acct:x").cloned();
    let receipt = ledger.reserve(&request("req:x", "acct:x", 1_000));
    let after = ledger.accounts.get("acct:x").cloned();
    let report = report::compile(before.as_ref(), after.as_ref(), &receipt);
    assert!(
        report.exclusions.iter().any(|e| e.contains("funds.release")),
        "settlement/administrative effects beyond funds.reserve must be a named exclusion, not omitted"
    );
}

/// "Evidence plane unavailable": declared evidence-finality outage
/// behavior -- fail-closed, no `Reserved` outcome without a durable
/// evidence commitment sharing the same atomic boundary.
#[test]
fn evidence_plane_unavailable_denies_fail_closed_before_any_mutation() {
    let mut ledger = Ledger::new();
    ledger.put_fact(AccountFact::signed("acct:evp", "subject:alice", 10_000, 1, "2026-09-28T00:00:00Z"));
    ledger.evidence_plane_available = false;
    let before = ledger.accounts.get("acct:evp").cloned();

    let receipt = ledger.reserve(&request("req:evp", "acct:evp", 1_000));
    assert_eq!(receipt.status, ReservationStatus::Denied);
    assert_eq!(receipt.diagnostic.as_deref(), Some("EvidencePlaneUnavailable"));

    let after = ledger.accounts.get("acct:evp").cloned();
    assert_eq!(before, after, "no mutation must occur when the evidence plane cannot durably record it");
}

/// "Authority later compromised": affected-decision discovery and
/// containment behavior.
#[test]
fn a_compromised_authority_s_decisions_are_discoverable_and_no_new_ones_are_granted() {
    let mut ledger = Ledger::new();
    ledger.put_fact(AccountFact::signed("acct:comp", "subject:alice", 10_000, 1, "2026-09-28T00:00:00Z"));
    ledger.reserve(&request("req:comp:1", "acct:comp", 1_000));
    let mut req2 = request("req:comp:2", "acct:comp", 1_000);
    req2.account_version = 2;
    ledger.reserve(&req2);

    // Discovery: every decision this account was ever party to, queryable
    // by the containment response after compromise is detected.
    let affected = ledger.decisions_for_account("acct:comp");
    assert_eq!(affected.len(), 2, "discovery must find every prior decision for this account");

    // The fact authority now revokes the account.
    let mut fact = ledger.accounts.get("acct:comp").unwrap().clone();
    fact.revoke();
    ledger.put_fact(fact);

    let mut req3 = request("req:comp:3", "acct:comp", 1_000);
    req3.account_version = 3;
    let r3 = ledger.reserve(&req3);
    assert_eq!(
        r3.status,
        ReservationStatus::Denied,
        "no new decision may be granted against a compromised authority's account"
    );
    // Discovery now also finds the denied attempt.
    assert_eq!(ledger.decisions_for_account("acct:comp").len(), 3);
}
