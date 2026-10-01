//! Role-gated facade functions -- what a generated screen's capability
//! gate does in the codegen'd path (`cargo-nirdosha`'s
//! `render_capability_gate`: check the screen's required role, then mint,
//! then call the gateway), written by hand here since this slice does not
//! wire CTMS screens/codegen yet (see crate module doc). `CapabilityIssuer`
//! itself is role-agnostic (`mint` never checks `Auth`'s roles -- see its
//! own doc comment), so the role check has to happen here, before mint, or
//! not at all.
//!
//! **Which role each step requires is policy data, not a Rust literal**:
//! every function here takes a [`CaseWorkflowPolicy`] and asks it for the
//! required role, rather than hardcoding one -- see `crate::policy`'s
//! module doc for why (the severity->role disposition routing in
//! particular is exactly the kind of thing this workspace's other
//! policy-driven surfaces, like `policy-services.toml`, keep out of code).

use nirdosha_guard_rfc0029::{CapabilityIssuer, EffectGateway, GatewayError};
use nirdosha_rt::Auth;

use crate::case::{CaseStore, CtmsCaseAssignGatewayV1, CtmsCaseDispositionGatewayV1, CtmsCaseEscalateGatewayV1, CtmsCaseSeniorReviewGatewayV1, MonitoringCase};
use crate::policy::{CaseWorkflowPolicy, RuleWorkflowPolicy};
use crate::rule_gateway::{CtmsRuleApproveGatewayV1, CtmsRuleCreateGatewayV1, CtmsRuleDisableGatewayV1, CtmsRuleEnableGatewayV1, CtmsRuleRetireGatewayV1, CtmsRuleRollbackGatewayV1, CtmsRuleSubmitGatewayV1};
use crate::rule_model::{RuleDefinition, RuleStore};

#[derive(Debug, PartialEq)]
pub enum CtmsServiceError {
    Forbidden { required_role: String },
    Gateway(GatewayError),
}

impl std::fmt::Display for CtmsServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CtmsServiceError::Forbidden { required_role } => write!(f, "requires role \"{required_role}\""),
            CtmsServiceError::Gateway(e) => write!(f, "{e}"),
        }
    }
}

fn require_role(auth: &Auth, required_role: &str) -> Result<(), CtmsServiceError> {
    if auth.has_role(required_role) {
        Ok(())
    } else {
        Err(CtmsServiceError::Forbidden { required_role: required_role.to_string() })
    }
}

/// `[service.case_assign]`: `access = 'requires role "{policy.assign_role}"'`.
pub fn assign_case(
    issuer: &CapabilityIssuer,
    gateway: &CtmsCaseAssignGatewayV1,
    store: &dyn CaseStore,
    policy: &CaseWorkflowPolicy,
    auth: &Auth,
    alert_id: &str,
    severity: &str,
    now_ms: u64,
) -> Result<MonitoringCase, CtmsServiceError> {
    require_role(auth, &policy.assign_role)?;
    let capability = issuer
        .mint(auth, <CtmsCaseAssignGatewayV1 as EffectGateway>::RESOURCE, <CtmsCaseAssignGatewayV1 as EffectGateway>::EFFECT, now_ms)
        .map_err(CtmsServiceError::Gateway)?;
    gateway.assign(&capability, store, alert_id, auth.user(), severity, now_ms).map_err(CtmsServiceError::Gateway)
}

/// `[service.case_escalate]`: `access = 'requires role
/// "{policy.escalate_role}"'`. The design note's acceptance scenario has
/// the investigating analyst escalate their own case;
/// `InMemoryCaseStore::escalate`'s own `NotAssignedActor` check is the
/// independent, data-level control that a *different* holder of the same
/// role (one this role check alone would also admit) cannot escalate a
/// case that isn't theirs.
pub fn escalate_case(
    issuer: &CapabilityIssuer,
    gateway: &CtmsCaseEscalateGatewayV1,
    store: &dyn CaseStore,
    policy: &CaseWorkflowPolicy,
    auth: &Auth,
    case_id: &str,
    reason: &str,
    now_ms: u64,
) -> Result<MonitoringCase, CtmsServiceError> {
    require_role(auth, &policy.escalate_role)?;
    let capability = issuer
        .mint(auth, <CtmsCaseEscalateGatewayV1 as EffectGateway>::RESOURCE, <CtmsCaseEscalateGatewayV1 as EffectGateway>::EFFECT, now_ms)
        .map_err(CtmsServiceError::Gateway)?;
    gateway.escalate(&capability, store, case_id, auth.user(), reason, now_ms).map_err(CtmsServiceError::Gateway)
}

/// `[service.case_senior_review]`: `access = 'requires role
/// "{policy.senior_review_role}"'`.
pub fn senior_review_case(
    issuer: &CapabilityIssuer,
    gateway: &CtmsCaseSeniorReviewGatewayV1,
    store: &dyn CaseStore,
    policy: &CaseWorkflowPolicy,
    auth: &Auth,
    case_id: &str,
    now_ms: u64,
) -> Result<MonitoringCase, CtmsServiceError> {
    require_role(auth, &policy.senior_review_role)?;
    let capability = issuer
        .mint(auth, <CtmsCaseSeniorReviewGatewayV1 as EffectGateway>::RESOURCE, <CtmsCaseSeniorReviewGatewayV1 as EffectGateway>::EFFECT, now_ms)
        .map_err(CtmsServiceError::Gateway)?;
    gateway.senior_review(&capability, store, case_id, auth.user(), now_ms).map_err(CtmsServiceError::Gateway)
}

/// `[service.case_disposition]`: required role comes from
/// `policy.disposition_role_for_severity(case_severity)` -- e.g. the
/// design note's own table routes a `"high"`-severity case to
/// `ComplianceOfficer` and everything else to `SeniorInvestigator|`. See
/// `crate::policy` for how that routing table itself is data.
/// `crate::case::InMemoryCaseStore::disposition`'s own same-actor check is
/// a second, independent control -- this role gate alone does not prove
/// the disposing analyst differs from the one assigned; see that module's
/// doc comment.
pub fn disposition_case(
    issuer: &CapabilityIssuer,
    gateway: &CtmsCaseDispositionGatewayV1,
    store: &dyn CaseStore,
    policy: &CaseWorkflowPolicy,
    auth: &Auth,
    case_id: &str,
    case_severity: &str,
    verdict: &str,
    now_ms: u64,
) -> Result<MonitoringCase, CtmsServiceError> {
    require_role(auth, policy.disposition_role_for_severity(case_severity))?;
    let capability = issuer
        .mint(auth, <CtmsCaseDispositionGatewayV1 as EffectGateway>::RESOURCE, <CtmsCaseDispositionGatewayV1 as EffectGateway>::EFFECT, now_ms)
        .map_err(CtmsServiceError::Gateway)?;
    gateway.disposition(&capability, store, case_id, auth.user(), verdict, now_ms).map_err(CtmsServiceError::Gateway)
}

/// `[service.rule_create]`: `access = 'requires role "{policy.create_role}"'`.
pub fn create_rule(issuer: &CapabilityIssuer, gateway: &CtmsRuleCreateGatewayV1, store: &dyn RuleStore, policy: &RuleWorkflowPolicy, auth: &Auth, rule: RuleDefinition, now_ms: u64) -> Result<RuleDefinition, CtmsServiceError> {
    require_role(auth, &policy.create_role)?;
    let capability = issuer.mint(auth, CtmsRuleCreateGatewayV1::RESOURCE, CtmsRuleCreateGatewayV1::EFFECT, now_ms).map_err(CtmsServiceError::Gateway)?;
    gateway.create(&capability, store, rule, now_ms).map_err(CtmsServiceError::Gateway)
}

/// `[service.rule_submit_for_approval]`: same role as drafting -- the
/// author moves their own draft into the approval queue.
pub fn submit_rule_for_approval(issuer: &CapabilityIssuer, gateway: &CtmsRuleSubmitGatewayV1, store: &dyn RuleStore, policy: &RuleWorkflowPolicy, auth: &Auth, rule_id: &str, version: u32, now_ms: u64) -> Result<RuleDefinition, CtmsServiceError> {
    require_role(auth, &policy.create_role)?;
    let capability = issuer.mint(auth, CtmsRuleSubmitGatewayV1::RESOURCE, CtmsRuleSubmitGatewayV1::EFFECT, now_ms).map_err(CtmsServiceError::Gateway)?;
    gateway.submit(&capability, store, rule_id, version, now_ms).map_err(CtmsServiceError::Gateway)
}

/// `[service.rule_approve]`: `access = 'requires role "{policy.approve_role}"'`.
/// `RuleStore::approve`'s own same-actor check is the independent,
/// data-level control that a *different* `RuleApprover` (one this role
/// check alone would also admit) still cannot approve their own rule --
/// see `crate::rule_model`'s doc.
pub fn approve_rule(issuer: &CapabilityIssuer, gateway: &CtmsRuleApproveGatewayV1, store: &dyn RuleStore, policy: &RuleWorkflowPolicy, auth: &Auth, rule_id: &str, version: u32, now_ms: u64) -> Result<RuleDefinition, CtmsServiceError> {
    require_role(auth, &policy.approve_role)?;
    let capability = issuer.mint(auth, CtmsRuleApproveGatewayV1::RESOURCE, CtmsRuleApproveGatewayV1::EFFECT, now_ms).map_err(CtmsServiceError::Gateway)?;
    gateway.approve(&capability, store, rule_id, version, auth.user(), now_ms).map_err(CtmsServiceError::Gateway)
}

/// `[service.rule_enable]`: `access = 'requires role "{policy.enable_role}"'`.
/// `RuleStore::enable` itself refuses a rule that isn't yet `Approved` --
/// "a RiskAnalyst cannot activate a rule without approval" holds even for
/// a caller with the right role, because the *rule* was never approved.
pub fn enable_rule(issuer: &CapabilityIssuer, gateway: &CtmsRuleEnableGatewayV1, store: &dyn RuleStore, policy: &RuleWorkflowPolicy, auth: &Auth, rule_id: &str, version: u32, now_ms: u64) -> Result<RuleDefinition, CtmsServiceError> {
    require_role(auth, &policy.enable_role)?;
    let capability = issuer.mint(auth, CtmsRuleEnableGatewayV1::RESOURCE, CtmsRuleEnableGatewayV1::EFFECT, now_ms).map_err(CtmsServiceError::Gateway)?;
    gateway.enable(&capability, store, rule_id, version, now_ms).map_err(CtmsServiceError::Gateway)
}

/// `[service.rule_disable]`: `access = 'requires role "{policy.disable_role}"'`.
pub fn disable_rule(issuer: &CapabilityIssuer, gateway: &CtmsRuleDisableGatewayV1, store: &dyn RuleStore, policy: &RuleWorkflowPolicy, auth: &Auth, rule_id: &str, version: u32, now_ms: u64) -> Result<RuleDefinition, CtmsServiceError> {
    require_role(auth, &policy.disable_role)?;
    let capability = issuer.mint(auth, CtmsRuleDisableGatewayV1::RESOURCE, CtmsRuleDisableGatewayV1::EFFECT, now_ms).map_err(CtmsServiceError::Gateway)?;
    gateway.disable(&capability, store, rule_id, version, now_ms).map_err(CtmsServiceError::Gateway)
}

/// `[service.rule_retire]`: `access = 'requires role "{policy.retire_role}"'`.
pub fn retire_rule(issuer: &CapabilityIssuer, gateway: &CtmsRuleRetireGatewayV1, store: &dyn RuleStore, policy: &RuleWorkflowPolicy, auth: &Auth, rule_id: &str, version: u32, now_ms: u64) -> Result<RuleDefinition, CtmsServiceError> {
    require_role(auth, &policy.retire_role)?;
    let capability = issuer.mint(auth, CtmsRuleRetireGatewayV1::RESOURCE, CtmsRuleRetireGatewayV1::EFFECT, now_ms).map_err(CtmsServiceError::Gateway)?;
    gateway.retire(&capability, store, rule_id, version, now_ms).map_err(CtmsServiceError::Gateway)
}

/// `[service.rule_rollback]`: `access = 'requires role "{policy.rollback_role}"'`.
pub fn rollback_rule(issuer: &CapabilityIssuer, gateway: &CtmsRuleRollbackGatewayV1, store: &dyn RuleStore, policy: &RuleWorkflowPolicy, auth: &Auth, rule_id: &str, to_version: u32, now_ms: u64) -> Result<RuleDefinition, CtmsServiceError> {
    require_role(auth, &policy.rollback_role)?;
    let capability = issuer.mint(auth, CtmsRuleRollbackGatewayV1::RESOURCE, CtmsRuleRollbackGatewayV1::EFFECT, now_ms).map_err(CtmsServiceError::Gateway)?;
    gateway.rollback(&capability, store, rule_id, to_version, now_ms).map_err(CtmsServiceError::Gateway)
}
