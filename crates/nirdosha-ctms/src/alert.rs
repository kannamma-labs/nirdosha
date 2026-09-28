//! `TransactionAlert` domain model and the `alert.create` effect gateway.
//! An [`AlertCandidate`] (a rule firing) only ever becomes a real
//! [`Alert`] through [`CtmsAlertGatewayV1`] -- there is no other insert
//! path, same discipline `crud_screens!`'s capability-gated create route
//! uses for `transfer_create`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use nirdosha_guard_rfc0029::{CapabilityIssuer, DecisionCapability, EffectGateway, GatewayCore, GatewayError};
use serde::{Deserialize, Serialize};

use crate::rules::AlertCandidate;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlertStatus {
    New,
    Assigned,
    FalsePositive,
    SuspiciousActivity,
}

/// The advisory-only model output attached to an alert -- see
/// `crate::model`'s module doc for why this can never influence
/// authorization. Distinct, in the type system, from `Alert::severity`
/// (the rule's own, authoritative field): a caller can read
/// `model_signal.score` to prioritize a worklist, but nothing here lets it
/// overwrite `severity` or `status`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelSignal {
    pub model_id: String,
    pub model_version: String,
    pub score: f64,
    pub input_digest: String,
    pub output_digest: String,
    pub explanation: Vec<String>,
    pub abstained: bool,
    pub limitations: Vec<String>,
}

impl From<crate::model::ModelReceipt> for ModelSignal {
    fn from(receipt: crate::model::ModelReceipt) -> Self {
        ModelSignal {
            model_id: receipt.model_id,
            model_version: receipt.model_version,
            score: receipt.score,
            input_digest: receipt.input_digest,
            output_digest: receipt.output_digest,
            explanation: receipt.explanation,
            abstained: receipt.abstained,
            limitations: receipt.limitations,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Alert {
    pub alert_id: String,
    pub rule_id: String,
    pub rule_version: u32,
    pub customer_key: String,
    pub severity: String,
    pub status: AlertStatus,
    /// `"realtime"` when created via [`CtmsAlertGatewayV1::create`],
    /// `"batch"` when created via `crate::batch::BatchRunner::run` --
    /// both paths share the same [`crate::rules::VelocityWindowStore`]
    /// evaluator and the same idempotency key, so a batch replay of
    /// events that already alerted in realtime returns the existing
    /// alert rather than creating a second one with `origin = "batch"`.
    pub origin: String,
    pub matched_event_ids: Vec<String>,
    pub created_at_ms: u64,
    /// `None` until a model has scored this alert -- attaching one never
    /// changes `severity`/`status` (see [`ModelSignal`]'s own doc).
    pub model_signal: Option<ModelSignal>,
}

pub trait AlertStore: Send + Sync {
    /// Idempotent by `(rule_id, rule_version, key, window_start_ms)`,
    /// regardless of `origin`: a repeated candidate for the same rule
    /// firing over the same window returns the existing alert rather than
    /// creating a second one -- the design note's "a batch rerun must not
    /// duplicate alerts", and, since the key doesn't include `origin`,
    /// this also covers a batch replay of events that already alerted in
    /// realtime (first writer's `origin` wins).
    fn create_or_get_with_origin(&self, candidate: &AlertCandidate, origin: &str, now_ms: u64) -> Alert;
    /// `crate::batch::BatchRunner` is the only caller that should ever
    /// pass a non-`"realtime"` origin to [`Self::create_or_get_with_origin`]
    /// directly; every other caller (namely [`CtmsAlertGatewayV1::create`])
    /// uses this realtime-origin convenience instead.
    fn create_or_get(&self, candidate: &AlertCandidate, now_ms: u64) -> Alert {
        self.create_or_get_with_origin(candidate, "realtime", now_ms)
    }
    /// Enrichment only -- deliberately not part of the `alert.create`
    /// effect gateway (see `crate::model`'s module doc): a model signal is
    /// advisory metadata, not a governed mutation, so it is attached
    /// directly rather than requiring its own capability.
    fn attach_model_signal(&self, alert_id: &str, signal: ModelSignal) -> Result<Alert, String>;
    fn get(&self, alert_id: &str) -> Option<Alert>;
    fn list(&self) -> Vec<Alert>;
}

type IdempotencyKey = (String, u32, String, u64);

#[derive(Default)]
struct Inner {
    by_idempotency_key: HashMap<IdempotencyKey, String>,
    alerts: HashMap<String, Alert>,
    next_id: u64,
}

#[derive(Default)]
pub struct InMemoryAlertStore {
    inner: Mutex<Inner>,
}

impl InMemoryAlertStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl AlertStore for InMemoryAlertStore {
    fn create_or_get_with_origin(&self, candidate: &AlertCandidate, origin: &str, now_ms: u64) -> Alert {
        let mut inner = self.inner.lock().expect("alert store lock poisoned");
        let key: IdempotencyKey = (candidate.rule_id.clone(), candidate.rule_version, candidate.key.clone(), candidate.window_start_ms);
        if let Some(existing_id) = inner.by_idempotency_key.get(&key) {
            return inner.alerts[existing_id].clone();
        }
        let alert_id = format!("alert-{}", inner.next_id);
        inner.next_id += 1;
        let alert = Alert {
            alert_id: alert_id.clone(),
            rule_id: candidate.rule_id.clone(),
            rule_version: candidate.rule_version,
            customer_key: candidate.key.clone(),
            severity: candidate.severity.clone(),
            status: AlertStatus::New,
            origin: origin.to_string(),
            matched_event_ids: candidate.matched_event_ids.clone(),
            created_at_ms: now_ms,
            model_signal: None,
        };
        inner.by_idempotency_key.insert(key, alert_id.clone());
        inner.alerts.insert(alert_id, alert.clone());
        alert
    }

    fn attach_model_signal(&self, alert_id: &str, signal: ModelSignal) -> Result<Alert, String> {
        let mut inner = self.inner.lock().expect("alert store lock poisoned");
        let alert = inner.alerts.get_mut(alert_id).ok_or_else(|| format!("unknown alert {alert_id}"))?;
        alert.model_signal = Some(signal);
        Ok(alert.clone())
    }

    fn get(&self, alert_id: &str) -> Option<Alert> {
        self.inner.lock().expect("alert store lock poisoned").alerts.get(alert_id).cloned()
    }

    fn list(&self) -> Vec<Alert> {
        self.inner.lock().expect("alert store lock poisoned").alerts.values().cloned().collect()
    }
}

/// The only thing allowed to turn an [`AlertCandidate`] into a real
/// [`Alert`] row. Alert creation is system-triggered (the rule engine, not
/// a human role), so unlike `case.assign`/`case.disposition` there is no
/// role gate here -- the capability alone (minted under the governing
/// bundle, for the evaluator's own subject identity) is the authorization.
pub struct CtmsAlertGatewayV1 {
    core: GatewayCore,
}

impl CtmsAlertGatewayV1 {
    pub fn new(evidence_path: impl AsRef<Path>, issuer: &CapabilityIssuer) -> Self {
        Self { core: GatewayCore::new(<Self as EffectGateway>::MODULE, evidence_path, issuer) }
    }

    pub fn evidence(&self) -> &nirdosha_audit::envelope::ModuleAuditChain {
        self.core.evidence()
    }

    /// Inherent forwarder to `EffectGateway::consume_and_execute` --
    /// `cargo-nirdosha`'s `render_capability_gate` emits a plain
    /// `gateway.consume_and_execute(...)` call into generated app code
    /// that never imports this crate's `EffectGateway` trait, so method
    /// resolution needs an inherent method here (same reasoning
    /// `nirdosha_guard_rfc0029::TransferRequestGatewayV1`'s own identical
    /// forwarder gives).
    pub fn consume_and_execute<T>(&self, capability: &DecisionCapability, now_ms: u64, execute: impl FnOnce() -> Result<T, String>) -> Result<T, GatewayError> {
        <Self as EffectGateway>::consume_and_execute(self, capability, now_ms, execute)
    }

    pub fn create(&self, capability: &DecisionCapability, store: &dyn AlertStore, candidate: &AlertCandidate, now_ms: u64) -> Result<Alert, GatewayError> {
        <Self as EffectGateway>::consume_and_execute(self, capability, now_ms, || Ok(store.create_or_get(candidate, now_ms)))
    }

    /// Like [`Self::create`], but for `crate::batch::run_batch` -- the
    /// only caller that should ever pass a non-`"realtime"` origin.
    pub fn create_with_origin(&self, capability: &DecisionCapability, store: &dyn AlertStore, candidate: &AlertCandidate, origin: &str, now_ms: u64) -> Result<Alert, GatewayError> {
        <Self as EffectGateway>::consume_and_execute(self, capability, now_ms, || Ok(store.create_or_get_with_origin(candidate, origin, now_ms)))
    }
}

impl EffectGateway for CtmsAlertGatewayV1 {
    const RESOURCE: &'static str = "TransactionAlert";
    const EFFECT: &'static str = "alert.create";
    const MODULE: &'static str = "rfc0029:ctms_alert_gateway_v1";

    fn core(&self) -> &GatewayCore {
        &self.core
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nirdosha_guard_rfc0029::PolicyBundle;
    use nirdosha_rt::Auth;

    const VALID_BUNDLE: &str = r#"
[bundle]
authority_id = "ctms-domain-authority"
policy_owner = "risk-platform-team"
jurisdiction = "IN"
approved_by = "ctms-domain-authority"
signed_by = "ctms-domain-authority"
effective_from = "2026-01-01T00:00:00Z"
expires_at = "2027-01-01T00:00:00Z"
"#;

    fn ms_2026_06_01() -> u64 {
        1_780_000_000 * 1000 // any timestamp inside the bundle window; exact value irrelevant to the test
    }

    fn issuer() -> CapabilityIssuer {
        let bundle = PolicyBundle::from_toml_str(VALID_BUNDLE).unwrap();
        CapabilityIssuer::new(bundle, 5 * 60 * 1000)
    }

    fn candidate() -> AlertCandidate {
        AlertCandidate {
            rule_id: "velocity_24h".to_string(),
            rule_version: 1,
            key: "cust-1".to_string(),
            window_start_ms: 0,
            window_end_ms: 1_000,
            severity: "high".to_string(),
            matched_event_ids: vec!["evt-1".to_string()],
            debit_count: 5,
            debit_total_minor: 1_000_000,
        }
    }

    #[test]
    fn a_valid_capability_creates_an_alert_and_writes_one_evidence_entry() {
        let dir = std::env::temp_dir().join(format!("ctms_alert_gw_{}", std::process::id()));
        let issuer = issuer();
        let gateway = CtmsAlertGatewayV1::new(&dir, &issuer);
        let store = InMemoryAlertStore::new();
        let auth = Auth::login("ctms-evaluator", &["System"]);
        let now = ms_2026_06_01();
        let capability = issuer.mint(&auth, CtmsAlertGatewayV1::RESOURCE, CtmsAlertGatewayV1::EFFECT, now).unwrap();

        let alert = gateway.create(&capability, &store, &candidate(), now).unwrap();
        assert_eq!(alert.status, AlertStatus::New);
        assert_eq!(alert.rule_id, "velocity_24h");
        assert_eq!(gateway.evidence().entries().len(), 1);
    }

    #[test]
    fn a_repeated_candidate_for_the_same_window_does_not_create_a_second_alert() {
        let dir = std::env::temp_dir().join(format!("ctms_alert_gw_idem_{}", std::process::id()));
        let issuer = issuer();
        let gateway = CtmsAlertGatewayV1::new(&dir, &issuer);
        let store = InMemoryAlertStore::new();
        let auth = Auth::login("ctms-evaluator", &["System"]);
        let now = ms_2026_06_01();

        let cap1 = issuer.mint(&auth, CtmsAlertGatewayV1::RESOURCE, CtmsAlertGatewayV1::EFFECT, now).unwrap();
        let alert1 = gateway.create(&cap1, &store, &candidate(), now).unwrap();

        let cap2 = issuer.mint(&auth, CtmsAlertGatewayV1::RESOURCE, CtmsAlertGatewayV1::EFFECT, now).unwrap();
        let alert2 = gateway.create(&cap2, &store, &candidate(), now).unwrap();

        assert_eq!(alert1.alert_id, alert2.alert_id, "same idempotency key must yield the same alert");
        assert_eq!(store.list().len(), 1);
    }
}
