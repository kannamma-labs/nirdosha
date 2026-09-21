//! What a `GuardedTable` operation can fail with, and how a screen route
//! turns that into an HTTP response.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardScreenError {
    /// `Decision::Deny` -- no matching `allow` policy, or an explicit
    /// `deny` record matched (deny-overrides).
    Denied(String),
    /// `Decision::Escalate` -- the matching policy routes through an
    /// `approval_chain!`; this bridge doesn't implement the approval UI
    /// in Phase 0/A, so an escalating request is reported, not silently
    /// treated as a denial.
    Escalated(String),
    /// G1: a submitted write's fields violate the matching policy's
    /// `field_policy { required/allowed/forbidden }`.
    FieldPolicyViolation(Vec<String>),
    /// G2: a `requires field(x) == y` (or unresolvable
    /// `invariant(...)`/`.transition_allowed()`) condition failed.
    ConditionFailed(String),
    /// A `filter <scope-fn>()` this bridge can't resolve yet (G3), or a
    /// real driver-level failure (`PlanError`, JSON decode, ...).
    Store(String),
}

impl std::fmt::Display for GuardScreenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GuardScreenError::Denied(reason) => write!(f, "denied: {reason}"),
            GuardScreenError::Escalated(target) => write!(f, "escalated to {target}"),
            GuardScreenError::FieldPolicyViolation(violations) => write!(f, "field policy violation: {}", violations.join("; ")),
            GuardScreenError::ConditionFailed(reason) => write!(f, "condition failed: {reason}"),
            GuardScreenError::Store(reason) => write!(f, "store error: {reason}"),
        }
    }
}

impl std::error::Error for GuardScreenError {}

/// The one place a `GuardScreenError` becomes an HTTP response --
/// deny/escalate map to 403 (the requester is who they say they are,
/// just not entitled), field-policy/condition failures to 400 (the
/// request itself was malformed against the policy's own contract), and
/// a store-layer failure to 500.
pub fn guard_error_response(error: GuardScreenError) -> nirdosha_rt::Response {
    match &error {
        GuardScreenError::Denied(_) | GuardScreenError::Escalated(_) => nirdosha_rt::Response::html(403, format!("<p>{error}</p>")),
        GuardScreenError::FieldPolicyViolation(_) | GuardScreenError::ConditionFailed(_) => nirdosha_rt::Response::html(400, format!("<p>{error}</p>")),
        GuardScreenError::Store(_) => nirdosha_rt::Response::html(500, format!("<p>{error}</p>")),
    }
}
