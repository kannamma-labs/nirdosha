//! The t0-t3 temporal-separation scenario the readiness matrix names as
//! still open after `L8`-`L14`: those fixtures each check a producer/
//! consumer *contract* on one static graph, never two genuinely separated
//! decision episodes. This exercises `pilot::assertion_lifecycle::Registry`
//! against exactly that gap:
//!
//!   t0: authority assertion accepted
//!   t1: a decision consumes the assertion
//!   t2: the assertion is revoked (or its validity window expires)
//!   t3: a second, later decision's repeated consumption must be rejected
//!
//! The property under test is not "the assertion is checked once" (L8-L14
//! already prove that) but "the assertion is checked *again*, against
//! *current* state, every single time it is consumed" — a consumer must
//! never treat an earlier `Consumed` outcome as evidence the assertion is
//! still good later.

use rfc0029_conformance::pilot::assertion_lifecycle::{AuthorityAssertion, ConsumptionStatus, Registry};

#[test]
fn t0_t1_t2_t3_revocation_between_two_decisions_is_caught() {
    let mut registry = Registry::new();

    // t0: authority assertion accepted.
    registry.assert(AuthorityAssertion::signed(
        "assertion-1",
        "authority:customs-officer",
        "fact:shipment-classification-42",
        0,
        100,
    ));

    // t1: decision-1 consumes the assertion. Nothing has invalidated it yet.
    let t1 = registry.consume("assertion-1", "decision-1", 10);
    assert_eq!(t1.status, ConsumptionStatus::Consumed);
    assert_eq!(t1.diagnostic, None);

    // t2: the authority revokes the assertion.
    registry.revoke("assertion-1").unwrap();

    // t3: decision-2 consumes the *same* assertion, later. It must not
    // reuse decision-1's `Consumed` answer -- the assertion's own state
    // has changed since t1, and t3 must see that.
    let t3 = registry.consume("assertion-1", "decision-2", 30);
    assert_eq!(t3.status, ConsumptionStatus::Denied);
    assert_eq!(t3.diagnostic.as_deref(), Some("AssertionRevoked"));

    // The evidence trail records both outcomes distinctly, not one
    // memoized answer reused twice.
    assert_eq!(registry.consumption_log.len(), 2);
    assert_ne!(
        registry.consumption_log[0].status,
        registry.consumption_log[1].status,
        "t1 and t3 must disagree once the assertion was revoked in between"
    );
}

#[test]
fn expiry_alone_without_explicit_revocation_is_also_caught_at_t3() {
    // The gap can be purely time-based, with no revocation event at all --
    // a decision must not assume "no one revoked it" means "still valid."
    let mut registry = Registry::new();
    registry.assert(AuthorityAssertion::signed(
        "assertion-2",
        "authority:customs-officer",
        "fact:shipment-classification-77",
        0,
        15,
    ));

    let t1 = registry.consume("assertion-2", "decision-1", 10);
    assert_eq!(t1.status, ConsumptionStatus::Consumed);

    // t3, no revoke() call at all -- only time has passed the assertion's
    // own valid_until.
    let t3 = registry.consume("assertion-2", "decision-2", 30);
    assert_eq!(t3.status, ConsumptionStatus::Denied);
    assert_eq!(t3.diagnostic.as_deref(), Some("AssertionNotCurrentlyValid"));
}

#[test]
fn a_decision_before_the_validity_window_opens_is_denied_too() {
    // Symmetric case: `valid_from` is meaningful, not just `valid_until`.
    let mut registry = Registry::new();
    registry.assert(AuthorityAssertion::signed(
        "assertion-3",
        "authority:customs-officer",
        "fact:shipment-classification-5",
        20,
        100,
    ));
    let early = registry.consume("assertion-3", "decision-1", 5);
    assert_eq!(early.status, ConsumptionStatus::Denied);
    assert_eq!(early.diagnostic.as_deref(), Some("AssertionNotCurrentlyValid"));
}

#[test]
fn repeated_consumption_before_any_invalidating_event_stays_consistent() {
    // The property is "re-check every time," not "deny the second read" --
    // two consumptions of a still-valid assertion by two different
    // decisions, with nothing having changed in between, must both
    // succeed. This is what distinguishes real revalidation from a
    // single-use/one-shot token model, which is not the claim here.
    let mut registry = Registry::new();
    registry.assert(AuthorityAssertion::signed(
        "assertion-4",
        "authority:customs-officer",
        "fact:shipment-classification-9",
        0,
        100,
    ));
    let first = registry.consume("assertion-4", "decision-1", 10);
    let second = registry.consume("assertion-4", "decision-2", 20);
    assert_eq!(first.status, ConsumptionStatus::Consumed);
    assert_eq!(second.status, ConsumptionStatus::Consumed);
}

#[test]
fn a_tampered_assertion_fails_authenticity_at_consumption_time() {
    // Mirrors funds_reserve's tamper-detection test: flipping a field
    // without going through `revoke()` (which re-signs) must be caught as
    // inauthentic, not silently accepted as if it were a legitimately
    // re-issued value.
    let mut registry = Registry::new();
    let mut assertion = AuthorityAssertion::signed(
        "assertion-5",
        "authority:customs-officer",
        "fact:shipment-classification-1",
        0,
        100,
    );
    assertion.revoked = true; // bypasses revoke(); no re-sign.
    registry.assert(assertion);

    let outcome = registry.consume("assertion-5", "decision-1", 10);
    assert_eq!(outcome.status, ConsumptionStatus::Denied);
    assert_eq!(outcome.diagnostic.as_deref(), Some("AssertionSignatureInvalid"));
}

#[test]
fn consuming_an_assertion_that_was_never_asserted_is_denied() {
    let mut registry = Registry::new();
    let outcome = registry.consume("never-asserted", "decision-1", 10);
    assert_eq!(outcome.status, ConsumptionStatus::Denied);
    assert_eq!(outcome.diagnostic.as_deref(), Some("UnknownAssertion"));
}

#[test]
fn revoking_an_unknown_assertion_is_an_explicit_error_not_a_silent_no_op() {
    let mut registry = Registry::new();
    assert_eq!(registry.revoke("never-asserted"), Err("UnknownAssertion"));
}
