//! `MonitoringCase` domain model and the case-workflow effect gateways:
//! `case.assign`, `case.escalate`, `case.senior_review`,
//! `case.disposition`. Four gateway types (`EffectGateway`'s `RESOURCE`/
//! `EFFECT` are compile-time consts, one effect per type -- see
//! `nirdosha-guard-rfc0029/src/gateway.rs`), sharing one `MODULE` label
//! and evidence file, since the design note treats `ctms_case_gateway_v1`
//! as one logical gateway boundary with several effects.
//!
//! **Escalation chain, bounded at disposition** (SAR drafting/approval is
//! explicitly a separate, not-yet-built slice): `New -> Assigned ->
//! Escalated -> SeniorReviewed -> {FalsePositive | SuspiciousActivity}`.
//! `InMemoryCaseStore::disposition` refuses to run before a case has
//! passed `SeniorReviewed` -- the workflow order is a real state
//! invariant, not just a suggested UI flow.
//!
//! **Role routing follows the design note's own table**: `RiskAnalyst`
//! investigates and escalates their own assigned case;
//! `SeniorInvestigator` reviews that escalation and (for anything other
//! than a `"high"`-severity case) also gives the final disposition;
//! `ComplianceOfficer` is required instead for a `"high"`-severity
//! case's disposition ("SeniorInvestigator: approve medium-risk
//! dispositions" / "ComplianceOfficer: approve high-risk dispositions").
//! See `crate::service` for where severity picks the required role.
//!
//! Three independent controls jointly enforce "an analyst cannot approve
//! their own disposition" and its escalation-time analogue: the *role*
//! gates in `crate::service` (mint-time), and two same-actor checks here
//! in the store (`escalate` refuses a non-assigned actor;
//! `disposition` refuses the case's own assigned analyst) -- defense in
//! depth, matching this crate's `policy_kind = "Authorization"` (role)
//! vs. `policy_kind = "StateInvariant"` (data-level) split from the
//! design note's own service-catalog sketch.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use nirdosha_guard_rfc0029::{CapabilityIssuer, DecisionCapability, EffectGateway, GatewayCore, GatewayError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaseStatus {
    New,
    Assigned,
    Escalated,
    SeniorReviewed,
    FalsePositive,
    SuspiciousActivity,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitoringCase {
    pub case_id: String,
    pub alert_id: String,
    /// Copied from the alert at assignment time -- drives which role
    /// `crate::service::disposition_case` requires (see module doc).
    pub severity: String,
    pub assigned_analyst: Option<String>,
    pub status: CaseStatus,
    pub escalated_by: Option<String>,
    pub escalation_reason: Option<String>,
    pub senior_reviewed_by: Option<String>,
    pub disposition: Option<String>,
    pub disposition_by: Option<String>,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaseStoreError {
    UnknownCase(String),
    UnknownVerdict(String),
    /// The workflow step was attempted out of order (e.g. dispositioning
    /// before a senior review, or escalating a case that isn't
    /// `Assigned`).
    WrongState { expected: &'static str, actual: &'static str },
    /// Only the analyst a case is actually assigned to may escalate it.
    NotAssignedActor,
    /// "an analyst cannot approve their own disposition."
    SameActorDisposition,
}

impl std::fmt::Display for CaseStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CaseStoreError::UnknownCase(id) => write!(f, "no such case: {id}"),
            CaseStoreError::UnknownVerdict(v) => write!(f, "unknown disposition verdict: {v}"),
            CaseStoreError::WrongState { expected, actual } => write!(f, "case must be {expected} for this action, was {actual}"),
            CaseStoreError::NotAssignedActor => write!(f, "only the analyst this case is assigned to may escalate it"),
            CaseStoreError::SameActorDisposition => write!(f, "an analyst cannot approve their own disposition"),
        }
    }
}

pub trait CaseStore: Send + Sync {
    /// Idempotently creates a case for `alert_id` if none exists yet, then
    /// assigns `analyst` to it -- one atomic effect for this slice (case
    /// *creation* is not separately billed; see module doc).
    fn assign_for_alert(&self, alert_id: &str, analyst: &str, severity: &str, now_ms: u64) -> Result<MonitoringCase, CaseStoreError>;
    /// Only the assigned analyst may escalate; the case must currently be
    /// `Assigned`.
    fn escalate(&self, case_id: &str, actor: &str, reason: &str) -> Result<MonitoringCase, CaseStoreError>;
    /// The case must currently be `Escalated`.
    fn senior_review(&self, case_id: &str, actor: &str) -> Result<MonitoringCase, CaseStoreError>;
    /// The case must currently be `SeniorReviewed`; the disposing actor
    /// must not be the case's own assigned analyst.
    fn disposition(&self, case_id: &str, actor: &str, verdict: &str) -> Result<MonitoringCase, CaseStoreError>;
    fn get(&self, case_id: &str) -> Option<MonitoringCase>;
    fn list(&self) -> Vec<MonitoringCase>;
}

#[derive(Default)]
struct Inner {
    by_alert_id: HashMap<String, String>,
    cases: HashMap<String, MonitoringCase>,
    next_id: u64,
}

#[derive(Default)]
pub struct InMemoryCaseStore {
    inner: Mutex<Inner>,
}

impl InMemoryCaseStore {
    pub fn new() -> Self {
        Self::default()
    }
}

pub(crate) fn status_name(status: CaseStatus) -> &'static str {
    match status {
        CaseStatus::New => "New",
        CaseStatus::Assigned => "Assigned",
        CaseStatus::Escalated => "Escalated",
        CaseStatus::SeniorReviewed => "SeniorReviewed",
        CaseStatus::FalsePositive => "FalsePositive",
        CaseStatus::SuspiciousActivity => "SuspiciousActivity",
    }
}

impl CaseStore for InMemoryCaseStore {
    fn assign_for_alert(&self, alert_id: &str, analyst: &str, severity: &str, now_ms: u64) -> Result<MonitoringCase, CaseStoreError> {
        let mut inner = self.inner.lock().expect("case store lock poisoned");
        let case_id = if let Some(existing) = inner.by_alert_id.get(alert_id) {
            existing.clone()
        } else {
            let id = format!("case-{}", inner.next_id);
            inner.next_id += 1;
            inner.by_alert_id.insert(alert_id.to_string(), id.clone());
            inner.cases.insert(
                id.clone(),
                MonitoringCase {
                    case_id: id.clone(),
                    alert_id: alert_id.to_string(),
                    severity: severity.to_string(),
                    assigned_analyst: None,
                    status: CaseStatus::New,
                    escalated_by: None,
                    escalation_reason: None,
                    senior_reviewed_by: None,
                    disposition: None,
                    disposition_by: None,
                    created_at_ms: now_ms,
                },
            );
            id
        };
        let case = inner.cases.get_mut(&case_id).expect("case just created or looked up");
        case.assigned_analyst = Some(analyst.to_string());
        case.status = CaseStatus::Assigned;
        Ok(case.clone())
    }

    fn escalate(&self, case_id: &str, actor: &str, reason: &str) -> Result<MonitoringCase, CaseStoreError> {
        let mut inner = self.inner.lock().expect("case store lock poisoned");
        let case = inner.cases.get_mut(case_id).ok_or_else(|| CaseStoreError::UnknownCase(case_id.to_string()))?;
        if case.status != CaseStatus::Assigned {
            return Err(CaseStoreError::WrongState { expected: "Assigned", actual: status_name(case.status) });
        }
        if case.assigned_analyst.as_deref() != Some(actor) {
            return Err(CaseStoreError::NotAssignedActor);
        }
        case.status = CaseStatus::Escalated;
        case.escalated_by = Some(actor.to_string());
        case.escalation_reason = Some(reason.to_string());
        Ok(case.clone())
    }

    fn senior_review(&self, case_id: &str, actor: &str) -> Result<MonitoringCase, CaseStoreError> {
        let mut inner = self.inner.lock().expect("case store lock poisoned");
        let case = inner.cases.get_mut(case_id).ok_or_else(|| CaseStoreError::UnknownCase(case_id.to_string()))?;
        if case.status != CaseStatus::Escalated {
            return Err(CaseStoreError::WrongState { expected: "Escalated", actual: status_name(case.status) });
        }
        case.status = CaseStatus::SeniorReviewed;
        case.senior_reviewed_by = Some(actor.to_string());
        Ok(case.clone())
    }

    fn disposition(&self, case_id: &str, actor: &str, verdict: &str) -> Result<MonitoringCase, CaseStoreError> {
        let mut inner = self.inner.lock().expect("case store lock poisoned");
        let case = inner.cases.get_mut(case_id).ok_or_else(|| CaseStoreError::UnknownCase(case_id.to_string()))?;
        if case.status != CaseStatus::SeniorReviewed {
            return Err(CaseStoreError::WrongState { expected: "SeniorReviewed", actual: status_name(case.status) });
        }
        if case.assigned_analyst.as_deref() == Some(actor) {
            return Err(CaseStoreError::SameActorDisposition);
        }
        case.status = match verdict {
            "SuspiciousActivity" => CaseStatus::SuspiciousActivity,
            "FalsePositive" => CaseStatus::FalsePositive,
            other => return Err(CaseStoreError::UnknownVerdict(other.to_string())),
        };
        case.disposition = Some(verdict.to_string());
        case.disposition_by = Some(actor.to_string());
        Ok(case.clone())
    }

    fn get(&self, case_id: &str) -> Option<MonitoringCase> {
        self.inner.lock().expect("case store lock poisoned").cases.get(case_id).cloned()
    }

    fn list(&self) -> Vec<MonitoringCase> {
        self.inner.lock().expect("case store lock poisoned").cases.values().cloned().collect()
    }
}

pub struct CtmsCaseAssignGatewayV1 {
    core: GatewayCore,
}

impl CtmsCaseAssignGatewayV1 {
    pub fn new(evidence_path: impl AsRef<Path>, issuer: &CapabilityIssuer) -> Self {
        Self { core: GatewayCore::new(<Self as EffectGateway>::MODULE, evidence_path, issuer) }
    }

    pub fn evidence(&self) -> &nirdosha_audit::envelope::ModuleAuditChain {
        self.core.evidence()
    }

    /// Inherent forwarder to `EffectGateway::consume_and_execute` -- see
    /// `crate::alert::CtmsAlertGatewayV1`'s identical forwarder for why
    /// generated code needs this instead of the trait's default method.
    pub fn consume_and_execute<T>(&self, capability: &DecisionCapability, now_ms: u64, execute: impl FnOnce() -> Result<T, String>) -> Result<T, GatewayError> {
        <Self as EffectGateway>::consume_and_execute(self, capability, now_ms, execute)
    }

    pub fn assign(&self, capability: &DecisionCapability, store: &dyn CaseStore, alert_id: &str, analyst: &str, severity: &str, now_ms: u64) -> Result<MonitoringCase, GatewayError> {
        <Self as EffectGateway>::consume_and_execute(self, capability, now_ms, || {
            store.assign_for_alert(alert_id, analyst, severity, now_ms).map_err(|e| e.to_string())
        })
    }
}

impl EffectGateway for CtmsCaseAssignGatewayV1 {
    const RESOURCE: &'static str = "MonitoringCase";
    const EFFECT: &'static str = "case.assign";
    const MODULE: &'static str = "rfc0029:ctms_case_gateway_v1";

    fn core(&self) -> &GatewayCore {
        &self.core
    }
}

pub struct CtmsCaseEscalateGatewayV1 {
    core: GatewayCore,
}

impl CtmsCaseEscalateGatewayV1 {
    pub fn new(evidence_path: impl AsRef<Path>, issuer: &CapabilityIssuer) -> Self {
        Self { core: GatewayCore::new(<Self as EffectGateway>::MODULE, evidence_path, issuer) }
    }

    pub fn evidence(&self) -> &nirdosha_audit::envelope::ModuleAuditChain {
        self.core.evidence()
    }

    /// Inherent forwarder to `EffectGateway::consume_and_execute` -- see
    /// `crate::alert::CtmsAlertGatewayV1`'s identical forwarder for why
    /// generated code needs this instead of the trait's default method.
    pub fn consume_and_execute<T>(&self, capability: &DecisionCapability, now_ms: u64, execute: impl FnOnce() -> Result<T, String>) -> Result<T, GatewayError> {
        <Self as EffectGateway>::consume_and_execute(self, capability, now_ms, execute)
    }

    pub fn escalate(&self, capability: &DecisionCapability, store: &dyn CaseStore, case_id: &str, actor: &str, reason: &str, now_ms: u64) -> Result<MonitoringCase, GatewayError> {
        <Self as EffectGateway>::consume_and_execute(self, capability, now_ms, || store.escalate(case_id, actor, reason).map_err(|e| e.to_string()))
    }
}

impl EffectGateway for CtmsCaseEscalateGatewayV1 {
    const RESOURCE: &'static str = "MonitoringCase";
    const EFFECT: &'static str = "case.escalate";
    const MODULE: &'static str = "rfc0029:ctms_case_gateway_v1";

    fn core(&self) -> &GatewayCore {
        &self.core
    }
}

pub struct CtmsCaseSeniorReviewGatewayV1 {
    core: GatewayCore,
}

impl CtmsCaseSeniorReviewGatewayV1 {
    pub fn new(evidence_path: impl AsRef<Path>, issuer: &CapabilityIssuer) -> Self {
        Self { core: GatewayCore::new(<Self as EffectGateway>::MODULE, evidence_path, issuer) }
    }

    pub fn evidence(&self) -> &nirdosha_audit::envelope::ModuleAuditChain {
        self.core.evidence()
    }

    /// Inherent forwarder to `EffectGateway::consume_and_execute` -- see
    /// `crate::alert::CtmsAlertGatewayV1`'s identical forwarder for why
    /// generated code needs this instead of the trait's default method.
    pub fn consume_and_execute<T>(&self, capability: &DecisionCapability, now_ms: u64, execute: impl FnOnce() -> Result<T, String>) -> Result<T, GatewayError> {
        <Self as EffectGateway>::consume_and_execute(self, capability, now_ms, execute)
    }

    pub fn senior_review(&self, capability: &DecisionCapability, store: &dyn CaseStore, case_id: &str, actor: &str, now_ms: u64) -> Result<MonitoringCase, GatewayError> {
        <Self as EffectGateway>::consume_and_execute(self, capability, now_ms, || store.senior_review(case_id, actor).map_err(|e| e.to_string()))
    }
}

impl EffectGateway for CtmsCaseSeniorReviewGatewayV1 {
    const RESOURCE: &'static str = "MonitoringCase";
    const EFFECT: &'static str = "case.senior_review";
    const MODULE: &'static str = "rfc0029:ctms_case_gateway_v1";

    fn core(&self) -> &GatewayCore {
        &self.core
    }
}

pub struct CtmsCaseDispositionGatewayV1 {
    core: GatewayCore,
}

impl CtmsCaseDispositionGatewayV1 {
    pub fn new(evidence_path: impl AsRef<Path>, issuer: &CapabilityIssuer) -> Self {
        Self { core: GatewayCore::new(<Self as EffectGateway>::MODULE, evidence_path, issuer) }
    }

    pub fn evidence(&self) -> &nirdosha_audit::envelope::ModuleAuditChain {
        self.core.evidence()
    }

    /// Inherent forwarder to `EffectGateway::consume_and_execute` -- see
    /// `crate::alert::CtmsAlertGatewayV1`'s identical forwarder for why
    /// generated code needs this instead of the trait's default method.
    pub fn consume_and_execute<T>(&self, capability: &DecisionCapability, now_ms: u64, execute: impl FnOnce() -> Result<T, String>) -> Result<T, GatewayError> {
        <Self as EffectGateway>::consume_and_execute(self, capability, now_ms, execute)
    }

    pub fn disposition(&self, capability: &DecisionCapability, store: &dyn CaseStore, case_id: &str, actor: &str, verdict: &str, now_ms: u64) -> Result<MonitoringCase, GatewayError> {
        <Self as EffectGateway>::consume_and_execute(self, capability, now_ms, || store.disposition(case_id, actor, verdict).map_err(|e| e.to_string()))
    }
}

impl EffectGateway for CtmsCaseDispositionGatewayV1 {
    const RESOURCE: &'static str = "MonitoringCase";
    const EFFECT: &'static str = "case.disposition";
    const MODULE: &'static str = "rfc0029:ctms_case_gateway_v1";

    fn core(&self) -> &GatewayCore {
        &self.core
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn escalated_case(store: &InMemoryCaseStore) -> MonitoringCase {
        store.assign_for_alert("alert-1", "priya", "high", 0).unwrap();
        store.escalate("case-0", "priya", "velocity pattern looks structured").unwrap()
    }

    #[test]
    fn only_the_assigned_analyst_may_escalate() {
        let store = InMemoryCaseStore::new();
        store.assign_for_alert("alert-1", "priya", "high", 0).unwrap();
        let err = store.escalate("case-0", "someone-else", "reason").unwrap_err();
        assert_eq!(err, CaseStoreError::NotAssignedActor);
    }

    #[test]
    fn disposition_before_senior_review_is_rejected() {
        let store = InMemoryCaseStore::new();
        let case = escalated_case(&store);
        let err = store.disposition(&case.case_id, "arjun", "SuspiciousActivity").unwrap_err();
        assert_eq!(err, CaseStoreError::WrongState { expected: "SeniorReviewed", actual: "Escalated" });
    }

    #[test]
    fn escalate_review_then_disposition_by_a_different_actor_succeeds() {
        let store = InMemoryCaseStore::new();
        let case = escalated_case(&store);
        let reviewed = store.senior_review(&case.case_id, "arjun").unwrap();
        assert_eq!(reviewed.status, CaseStatus::SeniorReviewed);
        assert_eq!(reviewed.senior_reviewed_by.as_deref(), Some("arjun"));

        let disposed = store.disposition(&case.case_id, "meera", "SuspiciousActivity").unwrap();
        assert_eq!(disposed.status, CaseStatus::SuspiciousActivity);
        assert_eq!(disposed.disposition_by.as_deref(), Some("meera"));
    }

    #[test]
    fn same_actor_cannot_disposition_a_case_assigned_to_themselves() {
        let store = InMemoryCaseStore::new();
        let case = escalated_case(&store);
        store.senior_review(&case.case_id, "arjun").unwrap();
        let err = store.disposition(&case.case_id, "priya", "SuspiciousActivity").unwrap_err();
        assert_eq!(err, CaseStoreError::SameActorDisposition);
    }

    #[test]
    fn assigning_the_same_alert_twice_reuses_the_same_case() {
        let store = InMemoryCaseStore::new();
        let first = store.assign_for_alert("alert-1", "priya", "high", 0).unwrap();
        let second = store.assign_for_alert("alert-1", "priya", "high", 0).unwrap();
        assert_eq!(first.case_id, second.case_id);
        assert_eq!(store.list().len(), 1);
    }
}
