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
use crate::policy::CaseWorkflowPolicy;

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
