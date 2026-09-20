//! Minimal exact-match policy evaluator.
//!
//! This is deliberately smaller than a policy frontend. It provides the
//! deterministic deny-overrides kernel that registry and Cedar adapters can
//! feed without making the IR depend on either frontend.

use crate::{Action, Cap, Condition, Decision, EscalateTarget, EvaluationContext, FieldMask, FilterExpr, Obligation};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyCandidate {
    pub effect: PolicyEffect,
    pub subjects: Vec<String>,
    pub action: Action,
    pub resource: String,
    pub purpose: Option<String>,
    pub conditions: Vec<Condition>,
    pub filter: Option<FilterExpr>,
    pub obligations: Vec<Obligation>,
    pub escalation: Option<EscalateTarget>,
    /// Cost/correctness caps this policy grants (`RowCap`, `MaxScanRows`,
    /// ...) — an execution engine (read-path plan building) needs these;
    /// the deny-overrides-allow decision itself does not.
    pub caps: Vec<Cap>,
    /// Field masks this policy grants — same reasoning as `caps`.
    pub masks: Vec<FieldMask>,
    /// `cap(affected_rows = N)` — the write-side counterpart to `caps`
    /// (see `Cap`'s own doc comment on why it's a separate field, not a
    /// `Cap` variant).
    pub affected_row_cap: Option<u64>,
    pub id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyEffect { Allow, Deny }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluationResult {
    pub decision: Decision,
    pub obligations: Vec<Obligation>,
    pub residual_filter: Option<FilterExpr>,
    pub caps: Vec<Cap>,
    pub masks: Vec<FieldMask>,
    pub affected_row_cap: Option<u64>,
}

pub fn evaluate(context: &EvaluationContext, policies: &[PolicyCandidate]) -> EvaluationResult {
    let mut matching = policies.iter().filter(|policy| matches_context(context, policy));
    let mut obligations = Vec::new();
    let mut residual_filter = None;
    let mut caps = Vec::new();
    let mut masks = Vec::new();
    let mut affected_row_cap = None;
    let mut allow = false;
    let mut escalation = None;

    while let Some(policy) = matching.next() {
        if matches!(policy.effect, PolicyEffect::Deny) {
            return EvaluationResult { decision: Decision::Deny { reason: format!("policy denied: {}", policy.id) }, obligations: Vec::new(), residual_filter: None, caps: Vec::new(), masks: Vec::new(), affected_row_cap: None };
        }
        allow = true;
        obligations.extend(policy.obligations.clone());
        residual_filter = residual_filter.or_else(|| policy.filter.clone());
        escalation = escalation.or_else(|| policy.escalation.clone());
        caps.extend(policy.caps.clone());
        masks.extend(policy.masks.clone());
        affected_row_cap = affected_row_cap.or(policy.affected_row_cap);
    }

    if let Some(target) = escalation {
        return EvaluationResult { decision: Decision::Escalate { to: target }, obligations, residual_filter, caps, masks, affected_row_cap };
    }
    if allow {
        EvaluationResult { decision: Decision::Allow, obligations, residual_filter, caps, masks, affected_row_cap }
    } else {
        EvaluationResult { decision: Decision::Deny { reason: "deny by default".into() }, obligations: Vec::new(), residual_filter: None, caps: Vec::new(), masks: Vec::new(), affected_row_cap: None }
    }
}

fn matches_context(context: &EvaluationContext, policy: &PolicyCandidate) -> bool {
    let role_match = policy.subjects.is_empty() || policy.subjects.iter().any(|subject| context.subject.roles.iter().any(|role| role == subject));
    role_match
        && context.action == policy.action
        && context.entity == policy.resource
        && policy.purpose.as_ref().is_none_or(|purpose| purpose == &context.purpose.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Destination, Environment, Purpose, QueryShape, Subject, Tenant, Classification};

    fn context() -> EvaluationContext {
        EvaluationContext { subject: Subject { id: "u".into(), roles: vec!["analyst".into()], claims: vec![], clearance: Classification::Internal }, tenant: Tenant("t".into()), entity: "orders".into(), dataset: "db".into(), action: Action::Read, destination: Destination::Browser, environment: Environment { env: "test".into(), ip: None, geo: None, device_posture: None, session_freshness: None }, time_bucket: "now".into(), query_shape: QueryShape { verbs: vec![], aggregate: None, grouping_keys: vec![], subject_dimension: None, ordering: vec![], pagination: crate::PaginationMode::LimitOnly { limit: 10 } }, purpose: Purpose("support".into()), policy_version: "v1".into() }
    }

    fn candidate(effect: PolicyEffect) -> PolicyCandidate {
        PolicyCandidate { effect, subjects: vec!["analyst".into()], action: Action::Read, resource: "orders".into(), purpose: Some("support".into()), conditions: vec![], filter: None, obligations: vec![], escalation: None, caps: vec![], masks: vec![], affected_row_cap: None, id: "orders-read".into() }
    }

    #[test]
    fn deny_by_default() { assert!(matches!(evaluate(&context(), &[]).decision, Decision::Deny { .. })); }

    #[test]
    fn deny_overrides_allow() { assert!(matches!(evaluate(&context(), &[candidate(PolicyEffect::Allow), candidate(PolicyEffect::Deny)]).decision, Decision::Deny { .. })); }

    #[test]
    fn exact_match_allows() { assert_eq!(evaluate(&context(), &[candidate(PolicyEffect::Allow)]).decision, Decision::Allow); }
}