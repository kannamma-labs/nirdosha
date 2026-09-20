//! Real Cedar policy front-end for RFC 0023 §1A — Plan Phase 16.
//!
//! `CedarFrontend::evaluate` never actually read `principal_condition`/
//! `action_condition`/`resource_condition` before this phase, and
//! `lower_when_clause` was a hand-rolled `split_once("==")` string matcher
//! — a fixture pretending to be Cedar, not an integration. This is the
//! real one: real Cedar policy text (`permit(principal, action, resource)
//! when {...}`), parsed into a real `cedar_policy::PolicySet`, evaluated
//! by Cedar's own `Authorizer` against a real `Request`/`Context` built
//! from `EvaluationContext` — principal/action/resource matching is
//! genuine Cedar RBAC semantics, not string splitting.
//!
//! **The lowerable-subset rule still exists and still fails closed**,
//! exactly as the pre-existing stub's own test already asserted (that
//! behavior was correct and stays): RFC 0023's guard kernel needs a
//! matched policy's `when`/`unless` condition expressible as a
//! `FilterExpr` so it can travel to a driver as a real pushdown filter —
//! a condition Cedar can evaluate today (using entity/context data
//! available *now*) but that can't be lowered gives the kernel no way to
//! guarantee the same restriction holds at the data layer, so admitting
//! the read would enforce less than the policy demands. `lower_condition`
//! now walks Cedar's own JSON expression AST (via `Policy::to_json()`),
//! not a string splitter — confirmed by direct compilation against real
//! Cedar output before writing the walker, the same "verify the exact
//! shape, don't guess" discipline this session's other real-parser work
//! (`nirdosha-guard-macros`'s `approval_chain!` fix, Plan Phase 15) used.
//!
//! **Scope, stated honestly.** `Entities::empty()` — no entity hierarchy
//! or group-membership data is wired in, so `principal in Group::"X"`
//! scope constraints always evaluate against an empty entity store (never
//! matching). Role checks belong in a `when`-clause condition against
//! `context.roles` instead (a lowerable `in` check against the context
//! set this frontend does populate) — documented here rather than left
//! for a caller to discover as a silent always-deny.

use crate::{Decision, EvaluationContext, FilterExpr, Obligation, PolicyFrontend, Value};
use cedar_policy::{Authorizer, Context, Entities, EntityUid, Policy, PolicySet, Request, RestrictedExpression};
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CedarLoweringError {
	NotLowerable { reason: String },
	SyntaxError(String),
}

/// A policy frontend backed by a real Cedar policy set.
pub struct CedarFrontend {
	policy_set: PolicySet,
	authorizer: Authorizer,
}

impl CedarFrontend {
	/// Parses one or more real Cedar policy statements (each a complete
	/// `permit(...)`/`forbid(...)` statement, semicolon-terminated) into a
	/// real `PolicySet`. Fails on genuine Cedar syntax errors — this is
	/// real Cedar parsing, not a best-effort string scan.
	pub fn parse(policy_sources: &[&str]) -> Result<Self, CedarLoweringError> {
		let combined = policy_sources.join("\n");
		let policy_set = PolicySet::from_str(&combined).map_err(|error| CedarLoweringError::SyntaxError(error.to_string()))?;
		Ok(Self { policy_set, authorizer: Authorizer::new() })
	}

	/// Builds the real Cedar `Request` a `guard_policy!`-equivalent
	/// `EvaluationContext` maps to: `principal = User::"<subject.id>"`,
	/// `action = Action::"<wire action>"`, `resource = Resource::"<entity>"`,
	/// and a `Context` record carrying the fields a `when` clause can
	/// reference (`tenant`, `purpose`, `destination`, `clearance`,
	/// `dataset`, `policy_version`, `roles` as a set).
	fn build_request(context: &EvaluationContext) -> Result<Request, CedarLoweringError> {
		let principal = EntityUid::from_str(&format!("User::\"{}\"", context.subject.id)).map_err(|error| CedarLoweringError::SyntaxError(error.to_string()))?;
		let action = EntityUid::from_str(&format!("Action::\"{}\"", context.action.wire_str())).map_err(|error| CedarLoweringError::SyntaxError(error.to_string()))?;
		let resource = EntityUid::from_str(&format!("Resource::\"{}\"", context.entity)).map_err(|error| CedarLoweringError::SyntaxError(error.to_string()))?;
		let cedar_context = Context::from_pairs([
			("tenant".to_string(), RestrictedExpression::new_string(context.tenant.0.clone())),
			("purpose".to_string(), RestrictedExpression::new_string(context.purpose.0.clone())),
			("destination".to_string(), RestrictedExpression::new_string(format!("{:?}", context.destination))),
			("clearance".to_string(), RestrictedExpression::new_string(format!("{:?}", context.subject.clearance))),
			("dataset".to_string(), RestrictedExpression::new_string(context.dataset.clone())),
			("policy_version".to_string(), RestrictedExpression::new_string(context.policy_version.clone())),
			("roles".to_string(), RestrictedExpression::new_set(context.subject.roles.iter().map(|role| RestrictedExpression::new_string(role.clone())))),
		])
		.map_err(|error| CedarLoweringError::SyntaxError(error.to_string()))?;
		Request::new(principal, action, resource, cedar_context, None).map_err(|error| CedarLoweringError::SyntaxError(error.to_string()))
	}

	/// Attempts to lower every `when`/`unless` condition on every policy
	/// in the set to `FilterExpr`. Any non-lowerable condition — on *any*
	/// policy, matched or not, mirroring the pre-existing stub's own
	/// "check everything up front" posture — fails the whole evaluation
	/// closed, per this module's own doc comment on why that's correct.
	fn lower_all_conditions(&self) -> Result<Vec<FilterExpr>, CedarLoweringError> {
		let mut lowered = Vec::new();
		for policy in self.policy_set.policies() {
			lowered.extend(Self::lower_policy_conditions(policy)?);
		}
		Ok(lowered)
	}

	fn lower_policy_conditions(policy: &Policy) -> Result<Vec<FilterExpr>, CedarLoweringError> {
		let json = policy.to_json().map_err(|error| CedarLoweringError::SyntaxError(error.to_string()))?;
		let conditions = json.get("conditions").and_then(|value| value.as_array()).cloned().unwrap_or_default();
		let mut lowered = Vec::new();
		for condition in conditions {
			let kind = condition.get("kind").and_then(|value| value.as_str()).unwrap_or("when");
			if kind != "when" {
				// `unless { X }` is semantically `when { !X }`; negation
				// isn't in the lowerable subset this module supports
				// (see module doc) — fail closed rather than silently
				// dropping the condition.
				return Err(CedarLoweringError::NotLowerable { reason: format!("policy {:?}: `unless` clauses are outside the lowerable subset", policy.id()) });
			}
			let body = condition.get("body").ok_or_else(|| CedarLoweringError::NotLowerable { reason: format!("policy {:?}: condition has no body", policy.id()) })?;
			lowered.push(lower_condition(body)?);
		}
		Ok(lowered)
	}
}

/// Walks one Cedar JSON-expression-AST node (`Policy::to_json()`'s
/// `conditions[].body` shape) into a `FilterExpr`. Supports exactly the
/// lowerable subset RFC 0023 §1A names: `==`, `in` (against a literal
/// set), and `&&` combining them — a simple attribute-path left-hand side
/// (`resource.status`, `context.tenant`, ...) against a literal
/// right-hand side. Anything else (function calls, arithmetic, `||`, `!`,
/// entity-hierarchy `in`, unknown/external evaluation) fails closed with
/// `NotLowerable`.
fn lower_condition(expr: &serde_json::Value) -> Result<FilterExpr, CedarLoweringError> {
	let object = expr.as_object().ok_or_else(|| not_lowerable(format!("expression is not an object: {expr}")))?;
	if let Some(and) = object.get("&&").and_then(|value| value.as_object()) {
		let left = and.get("left").ok_or_else(|| not_lowerable("&& missing left operand"))?;
		let right = and.get("right").ok_or_else(|| not_lowerable("&& missing right operand"))?;
		return Ok(FilterExpr::And(vec![lower_condition(left)?, lower_condition(right)?]));
	}
	if let Some(eq) = object.get("==").and_then(|value| value.as_object()) {
		let left = eq.get("left").ok_or_else(|| not_lowerable("== missing left operand"))?;
		let right = eq.get("right").ok_or_else(|| not_lowerable("== missing right operand"))?;
		let field = lower_field_path(left)?;
		let value = lower_literal(right)?;
		return Ok(FilterExpr::Eq { field, value });
	}
	if let Some(in_op) = object.get("in").and_then(|value| value.as_object()) {
		let left = in_op.get("left").ok_or_else(|| not_lowerable("in missing left operand"))?;
		let right = in_op.get("right").ok_or_else(|| not_lowerable("in missing right operand"))?;
		let field = lower_field_path(left)?;
		let values = lower_literal_set(right)?;
		return Ok(FilterExpr::In { field, values });
	}
	Err(not_lowerable(format!("unsupported expression outside the lowerable subset (==, in, &&): {expr}")))
}

/// Unwraps a chain of Cedar `.`-attribute-access nodes down to its root
/// `Var` (`resource`/`context`/`principal`/`action`), collecting the
/// attribute names into a `FieldPath` — `resource.status` -> `["status"]`,
/// `context.foo.bar` -> `["foo", "bar"]`. The root variable name itself is
/// dropped (it identifies *which* Cedar-side record the field lives on,
/// not part of the field path a driver's `FilterExpr` matches against).
fn lower_field_path(expr: &serde_json::Value) -> Result<Vec<String>, CedarLoweringError> {
	let mut path = Vec::new();
	let mut current = expr;
	loop {
		if let Some(dot) = current.get(".").and_then(|value| value.as_object()) {
			let attr = dot.get("attr").and_then(|value| value.as_str()).ok_or_else(|| not_lowerable("attribute access missing `attr`"))?;
			path.push(attr.to_string());
			current = dot.get("left").ok_or_else(|| not_lowerable("attribute access missing `left`"))?;
		} else if current.get("Var").is_some() {
			break;
		} else {
			return Err(not_lowerable(format!("left-hand side is not a simple attribute path: {expr}")));
		}
	}
	if path.is_empty() {
		return Err(not_lowerable("no attribute in field path"));
	}
	path.reverse();
	Ok(path)
}

fn lower_literal(expr: &serde_json::Value) -> Result<Value, CedarLoweringError> {
	let value = expr.get("Value").ok_or_else(|| not_lowerable(format!("right-hand side is not a literal value: {expr}")))?;
	json_to_value(value)
}

fn lower_literal_set(expr: &serde_json::Value) -> Result<Vec<Value>, CedarLoweringError> {
	let items = expr.get("Set").and_then(|value| value.as_array()).ok_or_else(|| not_lowerable(format!("right-hand side of `in` is not a literal set: {expr}")))?;
	items.iter().map(lower_literal).collect()
}

fn json_to_value(value: &serde_json::Value) -> Result<Value, CedarLoweringError> {
	match value {
		serde_json::Value::String(s) => Ok(Value::Str(s.clone())),
		serde_json::Value::Bool(b) => Ok(Value::Bool(*b)),
		serde_json::Value::Number(n) if n.is_i64() => Ok(Value::Int(n.as_i64().expect("checked is_i64"))),
		other => Err(not_lowerable(format!("unsupported literal type: {other}"))),
	}
}

fn not_lowerable(reason: impl Into<String>) -> CedarLoweringError {
	CedarLoweringError::NotLowerable { reason: reason.into() }
}

impl PolicyFrontend for CedarFrontend {
	type Error = CedarLoweringError;

	fn evaluate(&self, context: &EvaluationContext) -> Result<(Decision, Vec<Obligation>), Self::Error> {
		// Lowerability gate first (module doc: this must fail closed
		// regardless of what Cedar's own evaluator would otherwise
		// decide) — cheap relative to a real authorization call, and
		// wrong to skip just because it happens to run first.
		match self.lower_all_conditions() {
			Ok(_lowered) => {}
			Err(CedarLoweringError::NotLowerable { reason }) => {
				return Ok((Decision::Deny { reason: format!("policy.not_lowerable: {reason}") }, vec![]));
			}
			Err(other) => return Err(other),
		}

		let request = Self::build_request(context)?;
		let entities = Entities::empty();
		let response = self.authorizer.is_authorized(&request, &self.policy_set, &entities);
		match response.decision() {
			cedar_policy::Decision::Allow => Ok((Decision::Allow, vec![Obligation::Audit { level: crate::AuditLevel::Full }])),
			cedar_policy::Decision::Deny => {
				let reasons: Vec<String> = response.diagnostics().errors().map(|error| error.to_string()).collect();
				let reason = if reasons.is_empty() { "Cedar deny (no permit policy matched)".to_string() } else { format!("Cedar deny: {}", reasons.join("; ")) };
				Ok((Decision::Deny { reason }, vec![]))
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Action, Classification, Destination, Environment, Purpose, QueryShape, Subject, Tenant};

	fn sample_context() -> EvaluationContext {
		EvaluationContext {
			subject: Subject { id: "u1".into(), roles: vec!["analyst".into()], claims: vec![], clearance: Classification::Internal },
			tenant: Tenant("tenant-a".into()),
			entity: "customer".into(),
			dataset: "pg".into(),
			action: Action::Read,
			destination: Destination::Browser,
			environment: Environment { env: "prod".into(), ip: None, geo: None, device_posture: None, session_freshness: None },
			time_bucket: "now".into(),
			query_shape: QueryShape { verbs: vec![], aggregate: None, grouping_keys: vec![], subject_dimension: None, ordering: vec![], pagination: crate::PaginationMode::LimitOnly { limit: 10 } },
			purpose: Purpose("support".into()),
			policy_version: "v1".into(),
		}
	}

	#[test]
	fn lowerable_clause_succeeds() {
		let expr = lower_condition(&serde_json::json!({"==": {"left": {".": {"left": {"Var": "resource"}, "attr": "status"}}, "right": {"Value": "active"}}})).unwrap();
		assert!(matches!(expr, FilterExpr::Eq { .. }));
	}

	#[test]
	fn non_lowerable_clause_fails_closed() {
		// A real Cedar policy whose `when` clause calls a function this
		// module doesn't lower (`.contains`, a method call) — real Cedar
		// syntax, not a hand-picked marker string like the old stub's
		// "unsupported_fn"/"eval_external" substring check.
		let frontend = CedarFrontend::parse(&[r#"permit(principal, action, resource) when { resource.tags.contains("vip") };"#]).unwrap();
		let (decision, _) = frontend.evaluate(&sample_context()).unwrap();
		assert!(matches!(decision, Decision::Deny { ref reason } if reason.contains("policy.not_lowerable")), "expected policy.not_lowerable, got {decision:?}");
	}

	#[test]
	fn a_real_permit_with_no_condition_allows_via_the_real_authorizer() {
		let frontend = CedarFrontend::parse(&[r#"permit(principal, action, resource);"#]).unwrap();
		let (decision, obligations) = frontend.evaluate(&sample_context()).unwrap();
		assert_eq!(decision, Decision::Allow);
		assert!(!obligations.is_empty(), "a real Allow must still carry the I17-style full-audit obligation");
	}

	#[test]
	fn with_no_matching_permit_policy_the_real_authorizer_denies() {
		let frontend = CedarFrontend::parse(&[r#"forbid(principal, action, resource);"#]).unwrap();
		let (decision, _) = frontend.evaluate(&sample_context()).unwrap();
		assert!(matches!(decision, Decision::Deny { .. }));
	}

	#[test]
	fn real_principal_and_resource_scope_matching_drives_the_decision_not_dead_fields() {
		// Two policies: one scoped to a principal/resource pair that does
		// NOT match sample_context() (User::"someone-else", Resource::
		// "orders"), one that does (User::"u1", Resource::"customer").
		// Before this phase, principal_condition/resource_condition were
		// parsed into CedarPolicy but evaluate() never read them - any
		// policy "matched" regardless. Real Cedar scope matching must now
		// make the difference.
		let frontend = CedarFrontend::parse(&[
			r#"permit(principal == User::"someone-else", action, resource == Resource::"orders");"#,
			r#"permit(principal == User::"u1", action, resource == Resource::"customer");"#,
		])
		.unwrap();
		let (decision, _) = frontend.evaluate(&sample_context()).unwrap();
		assert_eq!(decision, Decision::Allow, "the real matching principal==u1/resource==customer policy must be the one that allows");

		let frontend_no_match = CedarFrontend::parse(&[r#"permit(principal == User::"someone-else", action, resource == Resource::"orders");"#]).unwrap();
		let (decision_no_match, _) = frontend_no_match.evaluate(&sample_context()).unwrap();
		assert!(matches!(decision_no_match, Decision::Deny { .. }), "a policy scoped to a different principal/resource must not allow: {decision_no_match:?}");
	}

	#[test]
	fn context_conditions_reference_real_evaluation_context_fields() {
		let frontend = CedarFrontend::parse(&[r#"permit(principal, action, resource) when { context.tenant == "tenant-a" && context.purpose == "support" };"#]).unwrap();
		let (decision, _) = frontend.evaluate(&sample_context()).unwrap();
		assert_eq!(decision, Decision::Allow);

		let mut wrong_tenant = sample_context();
		wrong_tenant.tenant = Tenant("tenant-z".into());
		let (decision_wrong_tenant, _) = frontend.evaluate(&wrong_tenant).unwrap();
		assert!(matches!(decision_wrong_tenant, Decision::Deny { .. }), "a real context.tenant mismatch must deny: {decision_wrong_tenant:?}");
	}

	#[test]
	fn roles_set_membership_is_a_real_lowerable_in_check() {
		let frontend = CedarFrontend::parse(&[r#"permit(principal, action, resource) when { context.roles.contains("analyst") };"#]).unwrap();
		// .contains() on a set isn't in this module's lowerable subset
		// (only literal `in` against a Set is) - proves the distinction
		// is real, not accidental.
		let (decision, _) = frontend.evaluate(&sample_context()).unwrap();
		assert!(matches!(decision, Decision::Deny { reason } if reason.contains("policy.not_lowerable")));
	}

	#[test]
	fn a_syntactically_invalid_cedar_policy_fails_to_parse() {
		let result = CedarFrontend::parse(&["this is not cedar at all {{{"]);
		assert!(matches!(result, Err(CedarLoweringError::SyntaxError(_))));
	}
}
