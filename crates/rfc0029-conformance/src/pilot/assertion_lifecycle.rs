//! Temporal-separation pilot: `L8`–`L14` (the fact-provenance fixtures)
//! each check a producer/consumer *contract* on one static graph evaluated
//! at one instant — none of them can show a real elapsed-time gap between
//! an authority's assertion and a *later*, possibly-revoked or -expired
//! consumption, because `compute_influence`/`evaluate_fact_provenance` have
//! no state that persists across separate calls. This module is that
//! missing stateful harness, in the same spirit as `funds_reserve`: a
//! falsification experiment against the versioned candidate IR, not a
//! production gateway. It has no store, network, or effect-gateway
//! dependency, and — unlike a real deployment — no cross-process or
//! restart persistence claim at all: state lives only in one `Registry`
//! value for the lifetime of one process. If persistence ever becomes part
//! of a real claim, that needs its own, separately falsified pilot; nothing
//! here should be read as covering it.
//!
//! The property under test, stated once: **a consumer may not treat "once
//! valid" as "valid forever."** Every `consume` call re-checks the
//! assertion's *current* stored state — authenticity, revocation, and
//! expiry against the time of that specific call — never a cached answer
//! from an earlier call, even one that consumed the very same assertion.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// A logical clock tick, not wall time — deterministic and directly
/// testable, matching this pilot's falsification purpose rather than a
/// real deployment's actual clock.
pub type Tick = u64;

/// A signed authority assertion, ratifying some upstream classification as
/// an admissible fact for a bounded validity window (mirrors L8-L14's
/// `AuthorityAssertion -AssertFact-> Fact` shape, but as mutable runtime
/// state rather than one static graph node).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityAssertion {
    pub assertion_id: String,
    pub asserted_by: String,
    pub subject: String,
    pub valid_from: Tick,
    pub valid_until: Tick,
    pub revoked: bool,
    pub signature: String,
}

impl AuthorityAssertion {
    pub fn signed(
        assertion_id: &str,
        asserted_by: &str,
        subject: &str,
        valid_from: Tick,
        valid_until: Tick,
    ) -> Self {
        let mut a = Self {
            assertion_id: assertion_id.into(),
            asserted_by: asserted_by.into(),
            subject: subject.into(),
            valid_from,
            valid_until,
            revoked: false,
            signature: String::new(),
        };
        a.signature = a.commitment();
        a
    }

    fn commitment(&self) -> String {
        let payload = format!(
            "{}:{}:{}:{}:{}:{}",
            self.assertion_id, self.asserted_by, self.subject, self.valid_from, self.valid_until, self.revoked
        );
        format!("sha256:{:x}", Sha256::digest(payload.as_bytes()))
    }

    pub fn is_authentic(&self) -> bool {
        self.signature == self.commitment()
    }

    /// Revocation is re-issuance with a new signature, not a field flip on
    /// an already-signed value — same principle as `AccountFact::revoke`
    /// in `funds_reserve`, for the same reason: a bare field flip would be
    /// indistinguishable from tampering, which `is_authentic` must catch
    /// either way.
    pub fn revoke(&mut self) {
        self.revoked = true;
        self.signature = self.commitment();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsumptionStatus {
    Consumed,
    Denied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsumptionRecord {
    pub assertion_id: String,
    pub decision_id: String,
    pub at: Tick,
    pub status: ConsumptionStatus,
    pub diagnostic: Option<String>,
}

#[derive(Debug, Default)]
pub struct Registry {
    assertions: BTreeMap<String, AuthorityAssertion>,
    /// Every consumption attempt, permitted or denied — the evidence trail
    /// this pilot is actually falsifying: does it show two different
    /// outcomes for the same assertion at two different times when the
    /// assertion's own state changed in between?
    pub consumption_log: Vec<ConsumptionRecord>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// t0: an authority asserts (ratifies) a fact.
    pub fn assert(&mut self, assertion: AuthorityAssertion) {
        self.assertions.insert(assertion.assertion_id.clone(), assertion);
    }

    /// t2: an authority revokes a previously asserted fact. Re-signs in
    /// place, per `AuthorityAssertion::revoke`'s own contract.
    pub fn revoke(&mut self, assertion_id: &str) -> Result<(), &'static str> {
        let assertion = self
            .assertions
            .get_mut(assertion_id)
            .ok_or("UnknownAssertion")?;
        assertion.revoke();
        Ok(())
    }

    /// t1 or t3: a decision consumes the assertion *now*. Always re-reads
    /// the assertion's current stored state and re-evaluates authenticity/
    /// revocation/expiry against `at` — never a cached result from an
    /// earlier call, including an earlier call against the very same
    /// `assertion_id`. This is the one property the whole module exists to
    /// prove: identical inputs at two different `at` values, with the
    /// assertion's own state having changed in between, must not produce
    /// the same `Consumed` answer both times.
    pub fn consume(&mut self, assertion_id: &str, decision_id: &str, at: Tick) -> ConsumptionRecord {
        let record = |status, diagnostic: Option<&str>| ConsumptionRecord {
            assertion_id: assertion_id.to_string(),
            decision_id: decision_id.to_string(),
            at,
            status,
            diagnostic: diagnostic.map(str::to_owned),
        };
        let outcome = match self.assertions.get(assertion_id) {
            None => record(ConsumptionStatus::Denied, Some("UnknownAssertion")),
            Some(assertion) if !assertion.is_authentic() => {
                record(ConsumptionStatus::Denied, Some("AssertionSignatureInvalid"))
            }
            Some(assertion) if assertion.revoked => {
                record(ConsumptionStatus::Denied, Some("AssertionRevoked"))
            }
            Some(assertion) if at < assertion.valid_from || at > assertion.valid_until => {
                record(ConsumptionStatus::Denied, Some("AssertionNotCurrentlyValid"))
            }
            Some(_) => record(ConsumptionStatus::Consumed, None),
        };
        self.consumption_log.push(outcome.clone());
        outcome
    }
}
