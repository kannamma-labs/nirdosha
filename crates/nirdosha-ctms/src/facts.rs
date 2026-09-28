//! RFC 0029 §17 fact providers for the external systems CTMS depends on
//! but does not implement (KYC status, account status) -- reusing
//! `nirdosha_guard_rfc0029::FactEnvelope<T>` (the same fact-provenance
//! envelope `funds_reserve` already uses: fact type, value, issuer,
//! observed/freshness window, resource version, revocation), not a
//! second, CTMS-specific envelope type.
//!
//! **What this closes and what it still does not.** This module gives
//! every external fact a real, checked shape: issuer identity, a
//! freshness window checked against the clock, revocation, and (via
//! [`DualSourceKycFactProvider`]) disagreement between two sources. What
//! it does not and cannot honestly claim: a live connection to a real
//! bank/KYC vendor API. [`InMemoryKycFactProvider`]/
//! [`InMemoryAccountFactProvider`] are the same "one honest
//! implementation, vendor slot stays open" pattern
//! `nirdosha-screening-list-fixture`'s `FixtureListProvider` and
//! `nirdosha_guard_rfc0029::InMemoryFundsReserveBackend` already use --
//! a real external vendor client implements the same trait and replaces
//! these without touching any caller.
//!
//! Every fetch is fail-closed by construction: [`FactError`] is the only
//! way a fact-fetch that isn't a clean, fresh, unrevoked, agreed-upon
//! value can return -- never a silently-accepted default.

use std::collections::HashMap;
use std::sync::Mutex;

use nirdosha_guard_rfc0029::FactEnvelope;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KycStatus {
    Verified,
    Pending,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactError {
    /// No fact exists for this key.
    Unavailable,
    /// A fact exists but `now_ms` is outside its `[observed_at_ms,
    /// valid_until_ms)` freshness window.
    Stale,
    /// The fact's issuer has since revoked it.
    Revoked,
    /// The fetch didn't return within its declared budget -- a real
    /// wall-clock timeout ([`BudgetedKycFactProvider`]), just with no live
    /// network call in this crate to actually be slow.
    Timeout,
    /// Two sources were consulted (see [`DualSourceKycFactProvider`]) and
    /// disagreed -- fail closed rather than picking one arbitrarily.
    Disagreement { first_issuer: String, second_issuer: String },
}

pub trait KycFactProvider: Send + Sync {
    fn kyc_status(&self, customer_id: &str, now_ms: u64) -> Result<FactEnvelope<KycStatus>, FactError>;
}

pub trait AccountFactProvider: Send + Sync {
    fn account_status(&self, account_id: &str, now_ms: u64) -> Result<FactEnvelope<nirdosha_guard_rfc0029::AccountStatus>, FactError>;
}

fn check_freshness_and_revocation<T: Clone>(envelope: Option<&FactEnvelope<T>>, now_ms: u64) -> Result<(), FactError> {
    let envelope = envelope.ok_or(FactError::Unavailable)?;
    if envelope.revoked {
        return Err(FactError::Revoked);
    }
    if now_ms < envelope.observed_at_ms || now_ms >= envelope.valid_until_ms {
        return Err(FactError::Stale);
    }
    Ok(())
}

/// One honest in-memory KYC fact source -- see module doc.
#[derive(Default)]
pub struct InMemoryKycFactProvider {
    facts: Mutex<HashMap<String, FactEnvelope<KycStatus>>>,
}

impl InMemoryKycFactProvider {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_fact(&self, customer_id: impl Into<String>, envelope: FactEnvelope<KycStatus>) {
        self.facts.lock().expect("kyc facts lock poisoned").insert(customer_id.into(), envelope);
    }
}

impl KycFactProvider for InMemoryKycFactProvider {
    fn kyc_status(&self, customer_id: &str, now_ms: u64) -> Result<FactEnvelope<KycStatus>, FactError> {
        let facts = self.facts.lock().expect("kyc facts lock poisoned");
        let envelope = facts.get(customer_id);
        check_freshness_and_revocation(envelope, now_ms)?;
        Ok(envelope.expect("checked Some above").clone())
    }
}

/// One honest in-memory account-status fact source -- see module doc.
#[derive(Default)]
pub struct InMemoryAccountFactProvider {
    facts: Mutex<HashMap<String, FactEnvelope<nirdosha_guard_rfc0029::AccountStatus>>>,
}

impl InMemoryAccountFactProvider {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_fact(&self, account_id: impl Into<String>, envelope: FactEnvelope<nirdosha_guard_rfc0029::AccountStatus>) {
        self.facts.lock().expect("account facts lock poisoned").insert(account_id.into(), envelope);
    }
}

impl AccountFactProvider for InMemoryAccountFactProvider {
    fn account_status(&self, account_id: &str, now_ms: u64) -> Result<FactEnvelope<nirdosha_guard_rfc0029::AccountStatus>, FactError> {
        let facts = self.facts.lock().expect("account facts lock poisoned");
        let envelope = facts.get(account_id);
        check_freshness_and_revocation(envelope, now_ms)?;
        Ok(envelope.expect("checked Some above").clone())
    }
}

/// Consults two independent [`KycFactProvider`]s and fails closed
/// (`FactError::Disagreement`) unless both return the same fresh,
/// unrevoked value -- "disagreement behavior" from the design note's
/// external-API fact-provenance requirements, made real rather than
/// merely declared.
pub struct DualSourceKycFactProvider<'a> {
    pub primary: &'a dyn KycFactProvider,
    pub secondary: &'a dyn KycFactProvider,
}

impl<'a> KycFactProvider for DualSourceKycFactProvider<'a> {
    fn kyc_status(&self, customer_id: &str, now_ms: u64) -> Result<FactEnvelope<KycStatus>, FactError> {
        let first = self.primary.kyc_status(customer_id, now_ms)?;
        let second = self.secondary.kyc_status(customer_id, now_ms)?;
        if first.value != second.value {
            return Err(FactError::Disagreement { first_issuer: first.issuer, second_issuer: second.issuer });
        }
        Ok(first)
    }
}

/// Wraps a [`KycFactProvider`] with a real wall-clock budget -- `inner`'s
/// `kyc_status` runs on a background thread; if it doesn't finish within
/// `budget` this returns `FactError::Timeout` rather than blocking
/// forever or silently waiting past the caller's own budget. This is a
/// real timeout (measured wall-clock time against a real thread), not a
/// simulated/fabricated one -- it just has no live network call to time
/// out on in this crate, the same honest limitation the module doc
/// discloses.
pub struct BudgetedKycFactProvider<P: KycFactProvider + 'static> {
    inner: std::sync::Arc<P>,
    budget: std::time::Duration,
}

impl<P: KycFactProvider + 'static> BudgetedKycFactProvider<P> {
    pub fn new(inner: std::sync::Arc<P>, budget: std::time::Duration) -> Self {
        Self { inner, budget }
    }
}

impl<P: KycFactProvider + 'static> KycFactProvider for BudgetedKycFactProvider<P> {
    fn kyc_status(&self, customer_id: &str, now_ms: u64) -> Result<FactEnvelope<KycStatus>, FactError> {
        let inner = self.inner.clone();
        let customer_id = customer_id.to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(inner.kyc_status(&customer_id, now_ms));
        });
        rx.recv_timeout(self.budget).unwrap_or(Err(FactError::Timeout))
    }
}

/// Enforces "a SAR should only submit for a customer with a fresh,
/// verified, unrevoked KYC fact" -- a real, meaningful use of this
/// module's fact provider, additive alongside `crate::sar::submit_sar`
/// (which stays usable without a fact provider for callers/tests that
/// don't have one wired up yet).
pub fn require_fresh_verified_kyc(provider: &dyn KycFactProvider, customer_id: &str, now_ms: u64) -> Result<(), FactError> {
    let fact = provider.kyc_status(customer_id, now_ms)?;
    if fact.value != KycStatus::Verified {
        return Err(FactError::Unavailable);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_envelope(value: KycStatus, issuer: &str) -> FactEnvelope<KycStatus> {
        FactEnvelope { fact_type: "customer_kyc_status", value, issuer: issuer.to_string(), observed_at_ms: 0, valid_until_ms: 1_000_000, resource_version: None, revoked: false }
    }

    #[test]
    fn an_unavailable_fact_fails_closed() {
        let provider = InMemoryKycFactProvider::new();
        assert_eq!(provider.kyc_status("cust-1", 0).unwrap_err(), FactError::Unavailable);
    }

    #[test]
    fn a_stale_fact_outside_its_freshness_window_is_rejected() {
        let provider = InMemoryKycFactProvider::new();
        provider.set_fact("cust-1", fresh_envelope(KycStatus::Verified, "kyc-vendor-a"));
        assert_eq!(provider.kyc_status("cust-1", 2_000_000).unwrap_err(), FactError::Stale);
    }

    #[test]
    fn a_revoked_fact_is_rejected_even_if_still_inside_its_window() {
        let provider = InMemoryKycFactProvider::new();
        let mut envelope = fresh_envelope(KycStatus::Verified, "kyc-vendor-a");
        envelope.revoked = true;
        provider.set_fact("cust-1", envelope);
        assert_eq!(provider.kyc_status("cust-1", 500).unwrap_err(), FactError::Revoked);
    }

    #[test]
    fn a_fresh_unrevoked_fact_is_returned() {
        let provider = InMemoryKycFactProvider::new();
        provider.set_fact("cust-1", fresh_envelope(KycStatus::Verified, "kyc-vendor-a"));
        let fact = provider.kyc_status("cust-1", 500).unwrap();
        assert_eq!(fact.value, KycStatus::Verified);
        assert_eq!(fact.issuer, "kyc-vendor-a");
    }

    #[test]
    fn two_agreeing_sources_return_the_agreed_fact() {
        let a = InMemoryKycFactProvider::new();
        a.set_fact("cust-1", fresh_envelope(KycStatus::Verified, "kyc-vendor-a"));
        let b = InMemoryKycFactProvider::new();
        b.set_fact("cust-1", fresh_envelope(KycStatus::Verified, "kyc-vendor-b"));
        let dual = DualSourceKycFactProvider { primary: &a, secondary: &b };
        assert_eq!(dual.kyc_status("cust-1", 500).unwrap().value, KycStatus::Verified);
    }

    #[test]
    fn two_disagreeing_sources_fail_closed() {
        let a = InMemoryKycFactProvider::new();
        a.set_fact("cust-1", fresh_envelope(KycStatus::Verified, "kyc-vendor-a"));
        let b = InMemoryKycFactProvider::new();
        b.set_fact("cust-1", fresh_envelope(KycStatus::Rejected, "kyc-vendor-b"));
        let dual = DualSourceKycFactProvider { primary: &a, secondary: &b };
        let err = dual.kyc_status("cust-1", 500).unwrap_err();
        assert_eq!(err, FactError::Disagreement { first_issuer: "kyc-vendor-a".to_string(), second_issuer: "kyc-vendor-b".to_string() });
    }

    #[test]
    fn require_fresh_verified_kyc_rejects_a_pending_customer() {
        let provider = InMemoryKycFactProvider::new();
        provider.set_fact("cust-1", fresh_envelope(KycStatus::Pending, "kyc-vendor-a"));
        assert!(require_fresh_verified_kyc(&provider, "cust-1", 500).is_err());
    }

    #[test]
    fn require_fresh_verified_kyc_accepts_a_verified_customer() {
        let provider = InMemoryKycFactProvider::new();
        provider.set_fact("cust-1", fresh_envelope(KycStatus::Verified, "kyc-vendor-a"));
        assert!(require_fresh_verified_kyc(&provider, "cust-1", 500).is_ok());
    }

    struct SlowTestProvider {
        delay: std::time::Duration,
    }
    impl KycFactProvider for SlowTestProvider {
        fn kyc_status(&self, _customer_id: &str, _now_ms: u64) -> Result<FactEnvelope<KycStatus>, FactError> {
            std::thread::sleep(self.delay);
            Ok(fresh_envelope(KycStatus::Verified, "slow-vendor"))
        }
    }

    #[test]
    fn a_provider_slower_than_its_budget_times_out() {
        let slow = std::sync::Arc::new(SlowTestProvider { delay: std::time::Duration::from_millis(200) });
        let budgeted = BudgetedKycFactProvider::new(slow, std::time::Duration::from_millis(20));
        assert_eq!(budgeted.kyc_status("cust-1", 0).unwrap_err(), FactError::Timeout);
    }

    #[test]
    fn a_provider_faster_than_its_budget_returns_normally() {
        let slow = std::sync::Arc::new(SlowTestProvider { delay: std::time::Duration::from_millis(5) });
        let budgeted = BudgetedKycFactProvider::new(slow, std::time::Duration::from_millis(500));
        assert_eq!(budgeted.kyc_status("cust-1", 0).unwrap().value, KycStatus::Verified);
    }

    #[test]
    fn account_fact_provider_enforces_the_same_freshness_and_revocation_discipline() {
        let provider = InMemoryAccountFactProvider::new();
        provider.set_fact(
            "acct-1",
            FactEnvelope { fact_type: "account_status", value: nirdosha_guard_rfc0029::AccountStatus::Active, issuer: "core-banking".to_string(), observed_at_ms: 0, valid_until_ms: 1_000, resource_version: Some(3), revoked: false },
        );
        assert_eq!(provider.account_status("acct-1", 500).unwrap().value, nirdosha_guard_rfc0029::AccountStatus::Active);
        assert_eq!(provider.account_status("acct-1", 2_000).unwrap_err(), FactError::Stale);
        assert_eq!(provider.account_status("unknown-account", 500).unwrap_err(), FactError::Unavailable);
    }
}
