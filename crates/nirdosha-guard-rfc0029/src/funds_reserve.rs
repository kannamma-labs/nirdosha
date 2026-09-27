//! `FundsReserveGatewayV1`: the `funds.reserve` gateway declared (but,
//! until now, unwired) in `policy-services.toml` as
//! `gateway = "funds_reserve_gateway_v1"`. Built on the same
//! [`crate::gateway::EffectGateway`] contract [`crate::TransferRequestGatewayV1`]
//! implements -- capability/bundle/idempotency-key verification and
//! evidence-writing are shared, not reimplemented here.
//!
//! What's new for this gateway is the *domain* check: RFC 0029 §7's
//! `NumericInvariant` policy kind requires the balance check and the
//! reservation mutation to share one atomic boundary (§18: "For `Atomic`,
//! the gateway must exclude relevant interleavings inside the transaction
//! boundary"). `fresh_balance`, `daily_limit`, `transfer_approval`, and
//! `account_status` are therefore not four independent trait methods (each
//! would need its own lock, and a second caller could interleave between
//! them -- a classic time-of-check/time-of-use gap) but one
//! [`FundsReserveBackend::reserve_atomic`] call that fetches all four
//! facts and performs the mutation under a single lock acquisition. Each
//! fact still carries its own [`FactEnvelope`] (RFC 0029 §17: fact type,
//! issuer, freshness window, resource version, revocation) for the
//! evidence trail; only the *locking*, not the *provenance modeling*, is
//! collapsed.
//!
//! This closely follows `rfc0029-conformance`'s `pilot::funds_reserve`
//! policy logic (idempotency → account lookup → status/freshness/subject/
//! version/balance/daily-limit/approval checks → atomic mutation), but
//! wired through a real capability, a real governed bundle, and the real
//! hash-chained evidence log that pilot deliberately has none of.
//!
//! Disclosed scope boundaries for this pass (see the crate's own top-level
//! module doc for the analogous capability-layer disclosures):
//! - [`InMemoryFundsReserveBackend`] is in-process and non-durable --
//!   unlike the capability-nonce replay guard (`GatewayCore`'s
//!   `replay_store`, now pluggable via `crate::ReplayStore` with a real
//!   `crate::SqliteReplayStore`), the account ledger and its business-level
//!   `request_id` idempotency table have no durable option yet. A durable,
//!   transaction-backed reservation store for multi-process deployment is
//!   explicitly later work, not attempted here.
//! - A negative `reconcile` (external settlement ultimately failed) marks
//!   the receipt `SettlementStatus::Failed` but does not compensate the
//!   already-committed local balance mutation -- same disclosed gap as
//!   `rfc0029-conformance`'s pilot (`funds_reserve.rs`'s
//!   `decisions_for_account` doc comment: "Discovery does not itself
//!   retract or compensate a prior effect").
//! - `GatewayCore::record`'s evidence appends are not mutually exclusive
//!   across concurrent callers sharing one gateway (`nirdosha_audit`'s
//!   `append_entry` has no cross-call locking) -- a real concern for a
//!   busy production gateway, but orthogonal to this pass's atomicity
//!   requirement, which is about the *balance mutation*, not the evidence
//!   log's own internal hash-chain ordering.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use nirdosha_audit::envelope::ModuleAuditChain;
use serde::{Deserialize, Serialize};

use crate::capability::{CapabilityIssuer, DecisionCapability, GatewayError};
use crate::gateway::{EffectGateway, GatewayCore};
use crate::replay_store::ReplayStore;

// ---------------------------------------------------------------------------
// Facts (RFC 0029 §17: fact type, value, issuer, freshness, resource
// version, revocation -- failure behavior is fail-closed by construction,
// see `FactError`).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FactEnvelope<T> {
    pub fact_type: &'static str,
    pub value: T,
    pub issuer: String,
    pub observed_at_ms: u64,
    pub valid_until_ms: u64,
    pub resource_version: Option<u64>,
    pub revoked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountStatus {
    Active,
    Frozen,
    Closed,
}

// ---------------------------------------------------------------------------
// Request / outcome
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FundsReserveRequest {
    /// Business idempotency key -- distinct from the capability's own
    /// single-use nonce (`GatewayCore` already consumes that). This one
    /// makes a *retried* `funds.reserve` call idempotent even though it
    /// necessarily carries a freshly minted capability.
    pub request_id: String,
    pub subject: String,
    pub account_id: String,
    pub amount_minor: u64,
    pub expected_account_version: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FundsReserveDenial {
    UnknownAccount,
    SubjectMismatch,
    AccountNotActive,
    StaleBalanceFact,
    ResourceVersionMismatch,
    InsufficientBalance,
    DailyLimitExceeded,
    ApprovalMissing,
    ApprovalStale,
    /// Same `request_id`, different request fields -- distinct from a
    /// genuine retry, which is `FundsReserveOutcome::IdempotentReplay`.
    IdempotencyKeyReuseMismatch,
    CommitFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettlementStatus {
    Reserved,
    /// Readiness matrix analogue: local commit succeeded, the paired
    /// external settlement submission's outcome is not yet known (a
    /// timeout, not a negative acknowledgement).
    AwaitingReconciliation,
    /// The external leg was ultimately negative. See this module's doc
    /// comment: the local mutation is not compensated here.
    Failed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FundsReserveReceipt {
    pub resulting_version: u64,
    pub facts: Vec<FactEnvelope<serde_json::Value>>,
    pub status: SettlementStatus,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FundsReserveOutcome {
    Settled(FundsReserveReceipt),
    IdempotentReplay(Box<FundsReserveReceipt>),
    Denied { reason: FundsReserveDenial, facts: Vec<FactEnvelope<serde_json::Value>> },
}

// ---------------------------------------------------------------------------
// Backend contract
// ---------------------------------------------------------------------------

pub trait FundsReserveBackend: Send + Sync {
    /// Check `fresh_balance`, `daily_limit`, `transfer_approval`, and
    /// `account_status`, and -- only if every check passes -- mutate the
    /// balance, entirely under one lock. See this module's doc comment
    /// for why this can't be four separate fact-provider calls.
    fn reserve_atomic(&self, request: &FundsReserveRequest, now_ms: u64) -> FundsReserveOutcome;

    fn confirm_external_submission(&self, request_id: &str, confirmed: Option<bool>) -> Result<(), &'static str>;

    fn reconcile(&self, request_id: &str, externally_confirmed: bool) -> Result<(), &'static str>;
}

#[derive(Debug, Clone)]
struct AccountRecord {
    owner_subject_id: String,
    balance_minor: u64,
    version: u64,
    status: AccountStatus,
    daily_limit_minor: Option<u64>,
    balance_issuer: String,
    balance_observed_at_ms: u64,
    balance_valid_until_ms: u64,
}

#[derive(Debug, Clone)]
struct ApprovalRecord {
    approver: String,
    valid_until_ms: u64,
}

#[derive(Default)]
struct Inner {
    accounts: BTreeMap<String, AccountRecord>,
    approvals: BTreeMap<String, ApprovalRecord>,
    /// Cumulative reserved amount per (account_id, calendar day), checked
    /// and updated inside `reserve_atomic`'s own lock -- same
    /// concurrency-under-partition property `rfc0029-conformance`'s pilot
    /// enforces.
    daily_reserved: BTreeMap<(String, String), u64>,
    consumed: BTreeMap<String, (FundsReserveRequest, FundsReserveReceipt)>,
    commit_should_fail: bool,
}

/// The one real (if in-process/non-durable, see module doc) fact provider
/// this pass wires up: an account ledger a caller populates directly,
/// checked and mutated atomically by `reserve_atomic`.
#[derive(Default)]
pub struct InMemoryFundsReserveBackend {
    inner: Mutex<Inner>,
}

const BALANCE_AUTHORITY: &str = "account-ledger-authority";

impl InMemoryFundsReserveBackend {
    pub fn new() -> Self {
        Self::default()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn put_account(
        &self,
        account_id: &str,
        owner_subject_id: &str,
        balance_minor: u64,
        version: u64,
        status: AccountStatus,
        daily_limit_minor: Option<u64>,
        balance_observed_at_ms: u64,
        balance_valid_until_ms: u64,
    ) {
        let mut inner = self.inner.lock().expect("funds-reserve backend lock poisoned");
        inner.accounts.insert(
            account_id.to_string(),
            AccountRecord {
                owner_subject_id: owner_subject_id.to_string(),
                balance_minor,
                version,
                status,
                daily_limit_minor,
                balance_issuer: BALANCE_AUTHORITY.to_string(),
                balance_observed_at_ms,
                balance_valid_until_ms,
            },
        );
    }

    pub fn put_approval(&self, account_id: &str, approver: &str, valid_until_ms: u64) {
        let mut inner = self.inner.lock().expect("funds-reserve backend lock poisoned");
        inner
            .approvals
            .insert(account_id.to_string(), ApprovalRecord { approver: approver.to_string(), valid_until_ms });
    }

    pub fn set_commit_should_fail(&self, should_fail: bool) {
        self.inner.lock().expect("funds-reserve backend lock poisoned").commit_should_fail = should_fail;
    }

    pub fn balance_of(&self, account_id: &str) -> Option<u64> {
        self.inner
            .lock()
            .expect("funds-reserve backend lock poisoned")
            .accounts
            .get(account_id)
            .map(|a| a.balance_minor)
    }
}

fn deny(reason: FundsReserveDenial, facts: Vec<FactEnvelope<serde_json::Value>>) -> FundsReserveOutcome {
    FundsReserveOutcome::Denied { reason, facts }
}

impl FundsReserveBackend for InMemoryFundsReserveBackend {
    fn reserve_atomic(&self, request: &FundsReserveRequest, now_ms: u64) -> FundsReserveOutcome {
        let mut inner = self.inner.lock().expect("funds-reserve backend lock poisoned");

        if let Some((prior_request, prior_receipt)) = inner.consumed.get(&request.request_id) {
            if prior_request == request {
                return FundsReserveOutcome::IdempotentReplay(Box::new(prior_receipt.clone()));
            }
            return deny(FundsReserveDenial::IdempotencyKeyReuseMismatch, vec![]);
        }

        let mut facts = Vec::new();

        let Some(account) = inner.accounts.get(&request.account_id).cloned() else {
            return deny(FundsReserveDenial::UnknownAccount, facts);
        };

        facts.push(FactEnvelope {
            fact_type: "account_status",
            value: serde_json::json!(account.status),
            issuer: BALANCE_AUTHORITY.to_string(),
            observed_at_ms: now_ms,
            valid_until_ms: now_ms,
            resource_version: Some(account.version),
            revoked: false,
        });
        if account.status != AccountStatus::Active {
            return deny(FundsReserveDenial::AccountNotActive, facts);
        }

        facts.push(FactEnvelope {
            fact_type: "fresh_balance",
            value: serde_json::json!(account.balance_minor),
            issuer: account.balance_issuer.clone(),
            observed_at_ms: account.balance_observed_at_ms,
            valid_until_ms: account.balance_valid_until_ms,
            resource_version: Some(account.version),
            revoked: false,
        });
        if account.balance_valid_until_ms <= now_ms {
            return deny(FundsReserveDenial::StaleBalanceFact, facts);
        }
        if account.owner_subject_id != request.subject {
            return deny(FundsReserveDenial::SubjectMismatch, facts);
        }
        if account.version != request.expected_account_version {
            return deny(FundsReserveDenial::ResourceVersionMismatch, facts);
        }
        if account.balance_minor < request.amount_minor {
            return deny(FundsReserveDenial::InsufficientBalance, facts);
        }

        let day = day_key(now_ms);
        let daily_key = (request.account_id.clone(), day.clone());
        if let Some(limit) = account.daily_limit_minor {
            facts.push(FactEnvelope {
                fact_type: "daily_limit",
                value: serde_json::json!(limit),
                issuer: BALANCE_AUTHORITY.to_string(),
                observed_at_ms: now_ms,
                valid_until_ms: now_ms,
                resource_version: Some(account.version),
                revoked: false,
            });
            let already_today = inner.daily_reserved.get(&daily_key).copied().unwrap_or(0);
            if already_today + request.amount_minor > limit {
                return deny(FundsReserveDenial::DailyLimitExceeded, facts);
            }
        }

        match inner.approvals.get(&request.account_id).cloned() {
            None => return deny(FundsReserveDenial::ApprovalMissing, facts),
            Some(approval) => {
                facts.push(FactEnvelope {
                    fact_type: "transfer_approval",
                    value: serde_json::json!(true),
                    issuer: approval.approver.clone(),
                    observed_at_ms: now_ms,
                    valid_until_ms: approval.valid_until_ms,
                    resource_version: None,
                    revoked: false,
                });
                if approval.valid_until_ms <= now_ms {
                    return deny(FundsReserveDenial::ApprovalStale, facts);
                }
            }
        }

        if inner.commit_should_fail {
            return deny(FundsReserveDenial::CommitFailed, facts);
        }

        // Atomic boundary: every fact above was read, and the balance
        // mutation below happens, without releasing `inner`'s lock --
        // no other call on this backend can observe or act on a state
        // between the check and the mutation.
        let mut updated = account;
        updated.balance_minor -= request.amount_minor;
        updated.version += 1;
        let resulting_version = updated.version;
        inner.accounts.insert(request.account_id.clone(), updated);
        *inner.daily_reserved.entry(daily_key).or_insert(0) += request.amount_minor;

        let receipt = FundsReserveReceipt { resulting_version, facts, status: SettlementStatus::Reserved };
        inner.consumed.insert(request.request_id.clone(), (request.clone(), receipt.clone()));
        FundsReserveOutcome::Settled(receipt)
    }

    fn confirm_external_submission(&self, request_id: &str, confirmed: Option<bool>) -> Result<(), &'static str> {
        let mut inner = self.inner.lock().expect("funds-reserve backend lock poisoned");
        let Some((_, receipt)) = inner.consumed.get_mut(request_id) else { return Err("UnknownRequest") };
        if receipt.status != SettlementStatus::Reserved {
            return Err("NotAwaitingExternalSubmission");
        }
        receipt.status = match confirmed {
            Some(true) => SettlementStatus::Reserved,
            Some(false) | None => SettlementStatus::AwaitingReconciliation,
        };
        Ok(())
    }

    fn reconcile(&self, request_id: &str, externally_confirmed: bool) -> Result<(), &'static str> {
        let mut inner = self.inner.lock().expect("funds-reserve backend lock poisoned");
        let Some((_, receipt)) = inner.consumed.get_mut(request_id) else { return Err("UnknownRequest") };
        if receipt.status != SettlementStatus::AwaitingReconciliation {
            return Err("NotAwaitingReconciliation");
        }
        receipt.status = if externally_confirmed { SettlementStatus::Reserved } else { SettlementStatus::Failed };
        Ok(())
    }
}

fn day_key(now_ms: u64) -> String {
    // Whole-day bucketing off the same epoch-ms clock the rest of this
    // crate uses; a real deployment would want a real calendar/timezone
    // rule here, out of scope for this narrow slice (matches
    // `rfc0029-conformance`'s pilot, which buckets off a caller-supplied
    // `YYYY-MM-DD` string instead).
    (now_ms / 86_400_000).to_string()
}

// ---------------------------------------------------------------------------
// Gateway
// ---------------------------------------------------------------------------

/// The one thing allowed to turn an admitted `funds_reserve` capability
/// into a real balance mutation.
pub struct FundsReserveGatewayV1 {
    core: GatewayCore,
    backend: Arc<dyn FundsReserveBackend>,
}

impl FundsReserveGatewayV1 {
    pub fn new(evidence_path: impl AsRef<Path>, issuer: &CapabilityIssuer, backend: Arc<dyn FundsReserveBackend>) -> Self {
        Self { core: GatewayCore::new(Self::MODULE, evidence_path, issuer), backend }
    }

    /// Like `new`, but with an explicit [`ReplayStore`] -- use with
    /// [`crate::SqliteReplayStore`] before a multi-process deployment. The
    /// account ledger itself ([`InMemoryFundsReserveBackend`]) is a
    /// separate, still-in-process concern; see this module's doc comment.
    pub fn with_replay_store(
        evidence_path: impl AsRef<Path>,
        issuer: &CapabilityIssuer,
        backend: Arc<dyn FundsReserveBackend>,
        replay_store: Arc<dyn ReplayStore>,
    ) -> Self {
        Self { core: GatewayCore::with_replay_store(Self::MODULE, evidence_path, issuer, replay_store), backend }
    }

    pub fn evidence(&self) -> &ModuleAuditChain {
        self.core.evidence()
    }

    pub fn reserve(
        &self,
        capability: &DecisionCapability,
        request: &FundsReserveRequest,
        now_ms: u64,
    ) -> Result<FundsReserveOutcome, GatewayError> {
        self.consume_and_execute(capability, now_ms, || match self.backend.reserve_atomic(request, now_ms) {
            FundsReserveOutcome::Denied { reason, .. } => Err(format!("{reason:?}")),
            outcome => Ok(outcome),
        })
    }

    pub fn confirm_external_submission(&self, request_id: &str, confirmed: Option<bool>) -> Result<(), &'static str> {
        self.backend.confirm_external_submission(request_id, confirmed)
    }

    pub fn reconcile(&self, request_id: &str, externally_confirmed: bool) -> Result<(), &'static str> {
        self.backend.reconcile(request_id, externally_confirmed)
    }
}

impl EffectGateway for FundsReserveGatewayV1 {
    const RESOURCE: &'static str = "Account";
    const EFFECT: &'static str = "funds.reserve";
    const MODULE: &'static str = "rfc0029:funds_reserve_gateway_v1";

    fn core(&self) -> &GatewayCore {
        &self.core
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle::PolicyBundle;
    use nirdosha_rt::Auth;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;

    const VALID_BUNDLE: &str = r#"
[bundle]
authority_id = "banking-domain-authority"
policy_owner = "payments-platform-team"
jurisdiction = "IN"
approved_by = "banking-domain-authority"
signed_by = "banking-domain-authority"
effective_from = "2026-01-01T00:00:00Z"
expires_at = "2027-01-01T00:00:00Z"
"#;

    fn ms_2026_06_01() -> u64 {
        crate::bundle::parse_rfc3339_utc_seconds("2026-06-01T00:00:00Z").unwrap() * 1000
    }

    fn issuer() -> CapabilityIssuer {
        let bundle = PolicyBundle::from_toml_str(VALID_BUNDLE).unwrap();
        CapabilityIssuer::new(bundle, 5 * 60 * 1000)
    }

    fn scratch_evidence_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("rfc0029_funds_reserve_test_{name}_{}", std::process::id()))
    }

    fn mint(issuer: &CapabilityIssuer, subject: &str, now: u64) -> DecisionCapability {
        let auth = Auth::login(subject, &["Customer"]);
        issuer.mint(&auth, FundsReserveGatewayV1::RESOURCE, FundsReserveGatewayV1::EFFECT, now).unwrap()
    }

    fn gateway_with_backend(name: &str, issuer: &CapabilityIssuer) -> (FundsReserveGatewayV1, Arc<InMemoryFundsReserveBackend>) {
        let backend = Arc::new(InMemoryFundsReserveBackend::new());
        let gateway = FundsReserveGatewayV1::new(scratch_evidence_path(name), issuer, backend.clone());
        (gateway, backend)
    }

    #[test]
    fn insufficient_balance_is_denied() {
        let issuer = issuer();
        let (gateway, backend) = gateway_with_backend("insufficient_balance", &issuer);
        let now = ms_2026_06_01();
        backend.put_account("acct-1", "alice", 50, 0, AccountStatus::Active, None, now - 1_000, now + 1_000);
        backend.put_approval("acct-1", "supervisor", now + 1_000);
        let cap = mint(&issuer, "alice", now);
        let request = FundsReserveRequest {
            request_id: "req-1".into(),
            subject: "alice".into(),
            account_id: "acct-1".into(),
            amount_minor: 100,
            expected_account_version: 0,
        };
        let err = gateway.reserve(&cap, &request, now).unwrap_err();
        assert_eq!(err, GatewayError::Execution(format!("{:?}", FundsReserveDenial::InsufficientBalance)));
        assert_eq!(backend.balance_of("acct-1"), Some(50));
    }

    #[test]
    fn exceeded_daily_limit_is_denied() {
        let issuer = issuer();
        let (gateway, backend) = gateway_with_backend("daily_limit", &issuer);
        let now = ms_2026_06_01();
        backend.put_account("acct-1", "alice", 1_000, 0, AccountStatus::Active, Some(100), now - 1_000, now + 1_000);
        backend.put_approval("acct-1", "supervisor", now + 1_000);
        let cap = mint(&issuer, "alice", now);
        let request = FundsReserveRequest {
            request_id: "req-1".into(),
            subject: "alice".into(),
            account_id: "acct-1".into(),
            amount_minor: 150,
            expected_account_version: 0,
        };
        let err = gateway.reserve(&cap, &request, now).unwrap_err();
        assert_eq!(err, GatewayError::Execution(format!("{:?}", FundsReserveDenial::DailyLimitExceeded)));
    }

    #[test]
    fn stale_balance_fact_is_denied() {
        let issuer = issuer();
        let (gateway, backend) = gateway_with_backend("stale_balance", &issuer);
        let now = ms_2026_06_01();
        backend.put_account("acct-1", "alice", 1_000, 0, AccountStatus::Active, None, now - 10_000, now - 1_000);
        backend.put_approval("acct-1", "supervisor", now + 1_000);
        let cap = mint(&issuer, "alice", now);
        let request = FundsReserveRequest {
            request_id: "req-1".into(),
            subject: "alice".into(),
            account_id: "acct-1".into(),
            amount_minor: 100,
            expected_account_version: 0,
        };
        let err = gateway.reserve(&cap, &request, now).unwrap_err();
        assert_eq!(err, GatewayError::Execution(format!("{:?}", FundsReserveDenial::StaleBalanceFact)));
    }

    #[test]
    fn missing_approval_is_denied() {
        let issuer = issuer();
        let (gateway, backend) = gateway_with_backend("missing_approval", &issuer);
        let now = ms_2026_06_01();
        backend.put_account("acct-1", "alice", 1_000, 0, AccountStatus::Active, None, now - 1_000, now + 1_000);
        let cap = mint(&issuer, "alice", now);
        let request = FundsReserveRequest {
            request_id: "req-1".into(),
            subject: "alice".into(),
            account_id: "acct-1".into(),
            amount_minor: 100,
            expected_account_version: 0,
        };
        let err = gateway.reserve(&cap, &request, now).unwrap_err();
        assert_eq!(err, GatewayError::Execution(format!("{:?}", FundsReserveDenial::ApprovalMissing)));
    }

    #[test]
    fn replayed_request_id_returns_the_original_outcome() {
        let issuer = issuer();
        let (gateway, backend) = gateway_with_backend("replay_request_id", &issuer);
        let now = ms_2026_06_01();
        backend.put_account("acct-1", "alice", 1_000, 0, AccountStatus::Active, None, now - 1_000, now + 1_000);
        backend.put_approval("acct-1", "supervisor", now + 1_000);
        let request = FundsReserveRequest {
            request_id: "req-1".into(),
            subject: "alice".into(),
            account_id: "acct-1".into(),
            amount_minor: 100,
            expected_account_version: 0,
        };

        let cap1 = mint(&issuer, "alice", now);
        let first = gateway.reserve(&cap1, &request, now).unwrap();
        let FundsReserveOutcome::Settled(first_receipt) = first else { panic!("expected Settled, got {first:?}") };

        // A retried business request necessarily carries a fresh capability
        // (the first one is now single-use consumed), but the same
        // request_id must not re-execute the mutation.
        let cap2 = mint(&issuer, "alice", now);
        let second = gateway.reserve(&cap2, &request, now).unwrap();
        assert_eq!(second, FundsReserveOutcome::IdempotentReplay(Box::new(first_receipt)));
        assert_eq!(backend.balance_of("acct-1"), Some(900), "the mutation must not run twice");
    }

    #[test]
    fn reused_capability_is_rejected_by_the_generalized_gateway_core() {
        let issuer = issuer();
        let (gateway, backend) = gateway_with_backend("replay_capability", &issuer);
        let now = ms_2026_06_01();
        backend.put_account("acct-1", "alice", 1_000, 0, AccountStatus::Active, None, now - 1_000, now + 1_000);
        backend.put_approval("acct-1", "supervisor", now + 1_000);
        let cap = mint(&issuer, "alice", now);
        let request = FundsReserveRequest {
            request_id: "req-1".into(),
            subject: "alice".into(),
            account_id: "acct-1".into(),
            amount_minor: 100,
            expected_account_version: 0,
        };
        assert!(gateway.reserve(&cap, &request, now).is_ok());
        let request2 = FundsReserveRequest { request_id: "req-2".into(), ..request };
        let err = gateway.reserve(&cap, &request2, now).unwrap_err();
        assert_eq!(err, GatewayError::CapabilityReplayed);
    }

    #[test]
    fn concurrent_reservations_never_let_two_requests_win_the_same_stale_version() {
        // All threads race with the account's *initial* version -- exactly
        // one may observe a matching `expected_account_version` and commit;
        // every other thread must see `ResourceVersionMismatch`, never a
        // partial or double-applied mutation. This is the sharpest test of
        // "the balance check and the reservation share one atomic
        // boundary": a check-then-mutate race here would let more than one
        // thread through the version check before either had committed.
        let issuer = Arc::new(issuer());
        let backend = Arc::new(InMemoryFundsReserveBackend::new());
        let now = ms_2026_06_01();
        backend.put_account("acct-1", "alice", 10_000, 0, AccountStatus::Active, None, now - 1_000, now + 1_000);
        backend.put_approval("acct-1", "supervisor", now + 1_000);
        let gateway = Arc::new(FundsReserveGatewayV1::new(
            scratch_evidence_path("concurrent"),
            &issuer,
            backend.clone(),
        ));

        let successes = Arc::new(AtomicU64::new(0));
        let handles: Vec<_> = (0..16)
            .map(|i| {
                let gateway = gateway.clone();
                let issuer = issuer.clone();
                let successes = successes.clone();
                thread::spawn(move || {
                    let cap = mint(&issuer, "alice", now);
                    let request = FundsReserveRequest {
                        request_id: format!("req-{i}"),
                        subject: "alice".into(),
                        account_id: "acct-1".into(),
                        amount_minor: 100,
                        expected_account_version: 0,
                    };
                    match gateway.reserve(&cap, &request, now) {
                        Ok(FundsReserveOutcome::Settled(_)) => {
                            successes.fetch_add(1, Ordering::SeqCst);
                        }
                        Ok(other) => panic!("unexpected non-denied outcome: {other:?}"),
                        Err(GatewayError::Execution(msg)) => {
                            assert_eq!(msg, format!("{:?}", FundsReserveDenial::ResourceVersionMismatch));
                        }
                        Err(e) => panic!("unexpected gateway error: {e}"),
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(successes.load(Ordering::SeqCst), 1, "exactly one racer may win a stale version");
        assert_eq!(backend.balance_of("acct-1"), Some(9_900), "the mutation must have applied exactly once");
    }

    #[test]
    fn gateway_failure_leaves_balance_unchanged() {
        let issuer = issuer();
        let (gateway, backend) = gateway_with_backend("commit_failure", &issuer);
        let now = ms_2026_06_01();
        backend.put_account("acct-1", "alice", 1_000, 0, AccountStatus::Active, None, now - 1_000, now + 1_000);
        backend.put_approval("acct-1", "supervisor", now + 1_000);
        backend.set_commit_should_fail(true);
        let cap = mint(&issuer, "alice", now);
        let request = FundsReserveRequest {
            request_id: "req-1".into(),
            subject: "alice".into(),
            account_id: "acct-1".into(),
            amount_minor: 100,
            expected_account_version: 0,
        };
        let err = gateway.reserve(&cap, &request, now).unwrap_err();
        assert_eq!(err, GatewayError::Execution(format!("{:?}", FundsReserveDenial::CommitFailed)));
        assert_eq!(backend.balance_of("acct-1"), Some(1_000), "no partial mutation on gateway failure");
    }

    #[test]
    fn unknown_transaction_outcome_is_awaiting_reconciliation_until_resolved() {
        let issuer = issuer();
        let (gateway, backend) = gateway_with_backend("reconciliation", &issuer);
        let now = ms_2026_06_01();
        backend.put_account("acct-1", "alice", 1_000, 0, AccountStatus::Active, None, now - 1_000, now + 1_000);
        backend.put_approval("acct-1", "supervisor", now + 1_000);
        let cap = mint(&issuer, "alice", now);
        let request = FundsReserveRequest {
            request_id: "req-1".into(),
            subject: "alice".into(),
            account_id: "acct-1".into(),
            amount_minor: 100,
            expected_account_version: 0,
        };
        let outcome = gateway.reserve(&cap, &request, now).unwrap();
        assert!(matches!(outcome, FundsReserveOutcome::Settled(_)));

        // External settlement submission times out: a genuinely unknown
        // outcome, not a negative acknowledgement.
        gateway.confirm_external_submission("req-1", None).unwrap();
        let cap2 = mint(&issuer, "alice", now);
        let replay = gateway.reserve(&cap2, &request, now).unwrap();
        match replay {
            FundsReserveOutcome::IdempotentReplay(receipt) => {
                assert_eq!(receipt.status, SettlementStatus::AwaitingReconciliation);
            }
            other => panic!("expected IdempotentReplay, got {other:?}"),
        }

        gateway.reconcile("req-1", true).unwrap();
        let cap3 = mint(&issuer, "alice", now);
        let resolved = gateway.reserve(&cap3, &request, now).unwrap();
        match resolved {
            FundsReserveOutcome::IdempotentReplay(receipt) => {
                assert_eq!(receipt.status, SettlementStatus::Reserved);
            }
            other => panic!("expected IdempotentReplay, got {other:?}"),
        }
    }

    #[test]
    fn bundle_rotation_is_rejected_by_the_generalized_gateway_core() {
        let old_issuer = issuer();
        let backend = Arc::new(InMemoryFundsReserveBackend::new());
        let now = ms_2026_06_01();
        backend.put_account("acct-1", "alice", 1_000, 0, AccountStatus::Active, None, now - 1_000, now + 1_000);
        backend.put_approval("acct-1", "supervisor", now + 1_000);
        let cap = mint(&old_issuer, "alice", now);

        let rotated_bundle = PolicyBundle::from_toml_str(
            &VALID_BUNDLE.replace("policy_owner = \"payments-platform-team\"", "policy_owner = \"payments-platform-team-v2\""),
        )
        .unwrap();
        let new_issuer = CapabilityIssuer::new(rotated_bundle, 5 * 60 * 1000);
        let gateway = FundsReserveGatewayV1::new(scratch_evidence_path("bundle_rotation"), &new_issuer, backend);
        let request = FundsReserveRequest {
            request_id: "req-1".into(),
            subject: "alice".into(),
            account_id: "acct-1".into(),
            amount_minor: 100,
            expected_account_version: 0,
        };
        let err = gateway.reserve(&cap, &request, now).unwrap_err();
        assert_eq!(err, GatewayError::BundleMismatch);
    }
}
