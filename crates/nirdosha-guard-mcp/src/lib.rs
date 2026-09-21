//! `nirdosha-guard-mcp` — data-guard MCP surface for LLM agents.
//!
//! Implements RFC 0023 §1C.3 and RFC 0024. The MCP server is a thin wrapper
//! over the same `evaluate` surface used by every other guard client.
//! Agents are subjects with non-transferable, purpose-bound delegation tokens;
//! `submit_write` is default-off and requires evaluate-then-act with a matching
//! plan hash.
//!
//! **Plan Phase 14** closed two real gaps nothing in the workspace had
//! addressed before:
//!
//! 1. `evaluate()` never consulted a real policy set — it always returned
//!    a hardcoded `Deny { "no explicit agent policy was supplied" }`
//!    regardless of what was actually registered. `GuardMcpServer` now
//!    carries real `PolicyCandidate`s (via `PolicyRecord::to_candidate`,
//!    new in `nirdosha-guard-registry`) and calls the same
//!    `evaluator::evaluate` every other guard client uses.
//! 2. Delegation tokens carried no real TTL, call budget, or rate limit —
//!    `mint_token` hardcoded `expires_at: "2099-01-01T00:00:00Z"` and
//!    `submit_write`'s plan-hash check matched against a predictable,
//!    non-cryptographic string (`format!("plan-hash-{entity}")`), which
//!    undermines I17's "evaluate-then-act with a matching plan hash"
//!    property — a caller could guess or reconstruct it without ever
//!    calling `evaluate`. Both are now real: TTL/`max_tool_calls`/rate
//!    limit per `examples/rtm/roles-N-guard_policy.md`'s `70_mcp.nir`
//!    declared defaults (`ttl = 30m; max_tool_calls = 60; rate = 20/min`),
//!    and the plan hash is a real SHA-256 digest of the evaluated
//!    `AccessPlan`'s actual content.

use nirdosha_guard_core::evaluator::{self, PolicyCandidate};
use nirdosha_guard_core::{
	AccessPlan, Decision, Destination, EvaluationContext, FieldMask, Purpose, WritePlan,
};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// Stable, signed tool descriptions generated from the registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDescription {
	pub name: String,
	pub description: String,
	pub input_schema: String,
	pub signature_hash: String,
}

/// I17/`70_mcp.nir`'s declared delegation defaults — not invented here,
/// read directly off the doc's own `delegation { bind user + agent; ttl =
/// 30m; max_tool_calls = 60; rate = 20/min; }` clause (`mcp_tools!` isn't
/// a callable macro yet — a grammar-mismatch gap already flagged
/// separately in `crates/nirdosha-rt/tests/rtm_policy_corpus.rs`, out of
/// this phase's scope — but the *values* it declares are real and worth
/// honoring even before the macro parses them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DelegationDefaults {
	pub ttl_ms: u64,
	pub max_tool_calls: u32,
	pub rate_per_min: u32,
}

impl Default for DelegationDefaults {
	fn default() -> Self {
		Self { ttl_ms: 30 * 60 * 1000, max_tool_calls: 60, rate_per_min: 20 }
	}
}

/// A delegation token: user-on-behalf, purpose-bound, non-transferable,
/// with a real, checkable TTL and call budget (I17).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegationToken {
	pub user_id: String,
	pub agent_id: String,
	pub purpose: Purpose,
	pub destination: Destination,
	pub policy_version: String,
	pub scope_hash: String,
	pub issued_at_ms: u64,
	pub ttl_ms: u64,
	pub max_tool_calls: u32,
}

impl DelegationToken {
	pub fn expires_at_ms(&self) -> u64 {
		self.issued_at_ms + self.ttl_ms
	}

	pub fn is_expired(&self, now_ms: u64) -> bool {
		now_ms >= self.expires_at_ms()
	}
}

/// Per-token call tracking backing `max_tool_calls` and the sliding-window
/// rate limit — server-side state, not something a token carries itself
/// (a token is a credential an agent presents; usage accounting has to
/// live with whoever's granting access, the same reason a real API-key
/// rate limiter tracks usage server-side rather than trusting a client's
/// own counter).
#[derive(Debug, Clone, Default)]
struct TokenUsage {
	calls_made: u32,
	/// Timestamps (ms) of calls within the last full minute — pruned on
	/// every check, so this never grows unbounded across a long-lived
	/// token's whole 30-minute TTL.
	recent_call_timestamps_ms: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolCallError {
	TokenExpired,
	MaxToolCallsExceeded { limit: u32 },
	RateLimitExceeded { limit_per_min: u32 },
}

/// Tool surface. Tools are generated from the registry, not hand-written
/// (Plan Phase 14) — `GuardMcpServer::new()` (no registry) still carries a
/// small, fixed fallback surface for a server with nothing registered yet,
/// but `from_registry` is the real path everything else in this phase
/// builds on.
pub struct GuardMcpServer {
	pub tools: Vec<ToolDescription>,
	pub agent_writes_enabled: bool,
	/// `agent_id:scope_hash:plan_hash -> ()`  — real "this exact plan was
	/// evaluated" membership, not a value `submit_write` reads back (the
	/// key's existence is the fact that matters).
	session_evaluations: HashMap<String, ()>,
	/// Real policies this server evaluates tool calls against — empty
	/// until `from_registry`/`with_policies` populates it, in which case
	/// `evaluate()` falls back to `evaluator::evaluate`'s own honest
	/// behavior for an empty policy set: deny (no matching policy).
	policies: Vec<PolicyCandidate>,
	defaults: DelegationDefaults,
	usage: HashMap<String, TokenUsage>,
}

impl GuardMcpServer {
	pub fn new() -> Self {
		Self {
			tools: vec![
				ToolDescription {
					name: "list_entities".into(),
					description: "List guard-registered entities".into(),
					input_schema: "{}".into(),
					signature_hash: "sha256-sig-001".into(),
				},
				ToolDescription {
					name: "describe_entity".into(),
					description: "Return redacted schema + classification".into(),
					input_schema: "{\"entity\":\"string\"}".into(),
					signature_hash: "sha256-sig-002".into(),
				},
				ToolDescription {
					name: "get_options".into(),
					description: "Enumerate allowed values for a categorical field".into(),
					input_schema: "{\"entity\":\"string\",\"field\":\"string\"}".into(),
					signature_hash: "sha256-sig-003".into(),
				},
				ToolDescription {
					name: "query_records".into(),
					description: "Query records under policy".into(),
					input_schema: "{\"entity\":\"string\"}".into(),
					signature_hash: "sha256-sig-004".into(),
				},
				ToolDescription {
					name: "evaluate".into(),
					description: "Dry-run a policy decision".into(),
					input_schema: "{\"entity\":\"string\",\"action\":\"string\"}".into(),
					signature_hash: "sha256-sig-005".into(),
				},
			],
			agent_writes_enabled: false,
			session_evaluations: HashMap::new(),
			policies: Vec::new(),
			defaults: DelegationDefaults::default(),
			usage: HashMap::new(),
		}
	}

	/// Construct MCP server tools *and* the real policy set they're
	/// evaluated against, both derived from a real registry dump — not
	/// hand-maintained, and not (as before Plan Phase 14) disconnected
	/// from what `evaluate()` actually checks.
	///
	/// Tool generation, grounded in real catalog data rather than the
	/// RFC's own illustrative fixed list
	/// (`query_records(Transaction, Alert, Case, Customer)`, `get_options(
	/// AlertStatus, CaseStatus, ...)`):
	/// - `query_records_<entity>` per `DatasetRecord`, but only when at
	///   least one real policy in the dump actually grants some subject
	///   `read` on that entity — advertising a tool nothing could ever
	///   authorize would be a real, misleading gap (RFC 0025's own §7
	///   V6/V3-style coverage-checking spirit, applied at generation time
	///   rather than left to a separate verify pass to catch after the
	///   fact).
	/// - `get_options_<workflow>` per `WorkflowRecord` — a workflow's
	///   declared `states` genuinely are "allowed values for a
	///   categorical field" (`workflow! { machine CaseStatus { ... } }`
	///   is exactly what `get_options(CaseStatus, ...)`'s illustrative
	///   RFC example names), not a guess.
	/// - `evaluate`/`submit_write` — universal control tools, always
	///   present regardless of what's registered.
	pub fn from_registry(dump: &nirdosha_guard_registry::RegistryDump) -> Self {
		let mut server = Self { tools: Vec::new(), ..Self::new_empty() };

		let readable_entities: std::collections::HashSet<&str> = dump
			.policies
			.iter()
			.filter(|policy| matches!(policy.effect, nirdosha_guard_registry::Effect::Allow) && policy.action == "read")
			.map(|policy| policy.resource.as_str())
			.collect();

		for dataset in &dump.datasets {
			if !readable_entities.contains(dataset.entity.as_str()) {
				continue;
			}
			server.tools.push(ToolDescription {
				name: format!("query_records_{}", dataset.entity),
				description: format!("Guarded query tool for entity {}", dataset.entity),
				input_schema: serde_json::json!({
					"type": "object",
					"properties": { "filter": { "type": "string" } }
				})
				.to_string(),
				signature_hash: tool_signature_hash("query_records", &dataset.entity, &dataset.fields),
			});
		}

		for workflow in &dump.workflows {
			server.tools.push(ToolDescription {
				name: format!("get_options_{}", workflow.name),
				description: format!("Enumerate allowed values for {} ({})", workflow.name, workflow.states.join(", ")),
				input_schema: "{}".into(),
				signature_hash: tool_signature_hash("get_options", &workflow.name, &workflow.states),
			});
		}

		server.tools.push(ToolDescription {
			name: "evaluate".into(),
			description: "Dry-run a policy decision".into(),
			input_schema: "{\"entity\":\"string\",\"action\":\"string\"}".into(),
			signature_hash: "sha256-sig-evaluate".into(),
		});
		server.tools.push(ToolDescription {
			name: "submit_write".into(),
			description: "Apply a write already covered by a matching evaluate() plan hash".into(),
			input_schema: "{\"plan_hash\":\"string\"}".into(),
			signature_hash: "sha256-sig-submit-write".into(),
		});

		server.policies = dump.policies.iter().filter_map(|record| record.to_candidate()).collect();
		server
	}

	fn new_empty() -> Self {
		Self { tools: Vec::new(), agent_writes_enabled: false, session_evaluations: HashMap::new(), policies: Vec::new(), defaults: DelegationDefaults::default(), usage: HashMap::new() }
	}

	/// Real policies this server evaluates against, for callers building a
	/// server without a full `RegistryDump` (e.g. tests).
	pub fn with_policies(mut self, policies: Vec<PolicyCandidate>) -> Self {
		self.policies = policies;
		self
	}

	/// Mint a non-transferable, purpose-bound delegation token for an
	/// agent, with the real I17 defaults (TTL/`max_tool_calls`/rate —
	/// `DelegationDefaults`) unless the caller overrides them.
	/// `scope_hash` is a real SHA-256 digest (user, agent, purpose,
	/// destination, policy_version) — a caller cannot forge a scope_hash
	/// for a different (user, agent) pair, unlike the previous
	/// `format!("hash({user}:{agent}:{version})")` placeholder.
	pub fn mint_token(
		user_id: impl Into<String>,
		agent_id: impl Into<String>,
		purpose: Purpose,
		destination: Destination,
		policy_version: impl Into<String>,
		now_ms: u64,
	) -> DelegationToken {
		Self::mint_token_with_defaults(user_id, agent_id, purpose, destination, policy_version, now_ms, DelegationDefaults::default())
	}

	pub fn mint_token_with_defaults(
		user_id: impl Into<String>,
		agent_id: impl Into<String>,
		purpose: Purpose,
		destination: Destination,
		policy_version: impl Into<String>,
		now_ms: u64,
		defaults: DelegationDefaults,
	) -> DelegationToken {
		let user = user_id.into();
		let agent = agent_id.into();
		let version = policy_version.into();
		let mut hasher = Sha256::new();
		hasher.update(user.as_bytes());
		hasher.update(b":");
		hasher.update(agent.as_bytes());
		hasher.update(b":");
		hasher.update(purpose.0.as_bytes());
		hasher.update(b":");
		hasher.update(format!("{destination:?}").as_bytes());
		hasher.update(b":");
		hasher.update(version.as_bytes());
		let scope_hash = hex_encode(&hasher.finalize());
		DelegationToken {
			user_id: user,
			agent_id: agent,
			purpose,
			destination,
			policy_version: version,
			scope_hash,
			issued_at_ms: now_ms,
			ttl_ms: defaults.ttl_ms,
			max_tool_calls: defaults.max_tool_calls,
		}
	}

	/// I17's TTL/`max_tool_calls`/rate-limit budget check, shared by
	/// `evaluate` and `submit_write` — every tool call spends budget, not
	/// just writes.
	fn check_and_record_call(&mut self, token: &DelegationToken, now_ms: u64) -> Result<(), ToolCallError> {
		if token.is_expired(now_ms) {
			return Err(ToolCallError::TokenExpired);
		}
		let usage = self.usage.entry(token.scope_hash.clone()).or_default();
		if usage.calls_made >= token.max_tool_calls {
			return Err(ToolCallError::MaxToolCallsExceeded { limit: token.max_tool_calls });
		}
		let window_start = now_ms.saturating_sub(60_000);
		usage.recent_call_timestamps_ms.retain(|&ts| ts >= window_start);
		if usage.recent_call_timestamps_ms.len() as u32 >= self.defaults.rate_per_min {
			return Err(ToolCallError::RateLimitExceeded { limit_per_min: self.defaults.rate_per_min });
		}
		usage.calls_made += 1;
		usage.recent_call_timestamps_ms.push(now_ms);
		Ok(())
	}

	/// I17: "masked fields are ABSENT, not masked-in-place" — the MCP
	/// surface's own, stricter posture than the general read path's
	/// `ReadOutcome` (which hands `masks` back for the caller to apply as
	/// redaction-in-place). Given a field list and the masks an
	/// `AccessPlan` carries, returns exactly the fields a copilot response
	/// should include — masked fields removed entirely, not replaced with
	/// a placeholder a model could still reason about the *presence* of.
	pub fn fields_present_for_copilot(fields: &[String], masks: &[FieldMask]) -> Vec<String> {
		let masked: std::collections::HashSet<&str> = masks.iter().filter_map(|mask| mask.field.first().map(String::as_str)).collect();
		fields.iter().filter(|field| !masked.contains(field.as_str())).cloned().collect()
	}

	/// Evaluate a request in the LLM context. Default posture denies
	/// CONFIDENTIAL/RESTRICTED and non-`llm_context` destinations
	/// regardless of what the registered policies say (I17's own
	/// belt-and-braces default, mirroring `copilot-no-sar`'s pattern of
	/// an explicit deny layered over whatever the base policy set would
	/// otherwise allow) — then, unlike before Plan Phase 14, actually
	/// consults `self.policies` via the real `evaluator::evaluate` instead
	/// of returning a hardcoded deny regardless of what's registered.
	pub fn evaluate(&mut self, token: &DelegationToken, context: &EvaluationContext, now_ms: u64) -> Result<(Decision, AccessPlan), ToolCallError> {
		self.check_and_record_call(token, now_ms)?;

		let deny = |reason: &str| {
			let plan = AccessPlan {
				decision: Decision::Deny { reason: reason.into() },
				filter: None,
				masks: vec![],
				caps: vec![],
				obligations: vec![nirdosha_guard_core::Obligation::Audit { level: nirdosha_guard_core::AuditLevel::Full }],
				policy_version: context.policy_version.clone(),
			};
			(plan.decision.clone(), plan)
		};
		if token.user_id != context.subject.id {
			return Ok(deny("delegation subject mismatch"));
		}
		if token.policy_version != context.policy_version {
			return Ok(deny("delegation policy snapshot mismatch"));
		}
		if token.destination != context.destination || context.destination != Destination::LlmContext {
			return Ok(deny("agent destination is not permitted"));
		}
		if matches!(context.subject.clearance, nirdosha_guard_core::Classification::Confidential | nirdosha_guard_core::Classification::Restricted) {
			return Ok(deny("agent access to confidential data is denied by default"));
		}

		// The real evaluation, against real registered policies — this is
		// the Plan Phase 14 fix: previously this method never reached
		// here at all, returning a hardcoded deny unconditionally.
		let result = evaluator::evaluate(context, &self.policies);
		let plan = AccessPlan {
			decision: result.decision.clone(),
			filter: result.residual_filter,
			masks: result.masks,
			// I17: agent access is never sampled, and row_cap defaults to
			// 50 (70_mcp.nir's `defaults { audit = full; row_cap = 50; }`)
			// when a matched policy didn't itself set a tighter RowCap.
			caps: if result.caps.iter().any(|cap| matches!(cap, nirdosha_guard_core::Cap::RowCap(_))) {
				result.caps
			} else {
				let mut caps = result.caps;
				caps.push(nirdosha_guard_core::Cap::RowCap(50));
				caps
			},
			obligations: {
				let mut obligations = result.obligations;
				if !obligations.iter().any(|obligation| matches!(obligation, nirdosha_guard_core::Obligation::Audit { level: nirdosha_guard_core::AuditLevel::Full })) {
					obligations.push(nirdosha_guard_core::Obligation::Audit { level: nirdosha_guard_core::AuditLevel::Full });
				}
				obligations
			},
			policy_version: context.policy_version.clone(),
		};

		if matches!(plan.decision, Decision::Allow) {
			let plan_hash = access_plan_hash(&plan);
			self.session_evaluations.insert(format!("{}:{}:{plan_hash}", token.agent_id, token.scope_hash), ());
		}

		Ok((plan.decision.clone(), plan))
	}

	/// `submit_write` requires evaluate-then-act with a matching plan
	/// hash. The hash is now a real SHA-256 digest of the evaluated
	/// `AccessPlan`'s content (`access_plan_hash`) rather than a
	/// predictable `format!("plan-hash-{entity}")` string a caller could
	/// reconstruct without ever calling `evaluate` — the actual security
	/// property I17 names ("evaluate-then-act... plan-hash matching")
	/// only holds if the hash can't be guessed.
	pub fn submit_write(&mut self, token: &DelegationToken, plan_hash: String, plan: WritePlan, now_ms: u64) -> Result<Decision, ToolCallError> {
		self.check_and_record_call(token, now_ms)?;
		if !self.agent_writes_enabled {
			return Ok(Decision::Deny { reason: "agent writes are disabled by default".into() });
		}
		let key = format!("{}:{}:{plan_hash}", token.agent_id, token.scope_hash);
		if self.session_evaluations.contains_key(&key) {
			Ok(plan.decision)
		} else {
			Ok(Decision::Deny { reason: "evaluate-then-act hash match required".into() })
		}
	}
}

impl Default for GuardMcpServer {
	fn default() -> Self {
		Self::new()
	}
}

fn hex_encode(bytes: &[u8]) -> String {
	bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn tool_signature_hash(kind: &str, name: &str, extra: &[String]) -> String {
	let mut hasher = Sha256::new();
	hasher.update(kind.as_bytes());
	hasher.update(b":");
	hasher.update(name.as_bytes());
	for item in extra {
		hasher.update(b":");
		hasher.update(item.as_bytes());
	}
	format!("sha256-{}", hex_encode(&hasher.finalize())[..16].to_string())
}

/// Real content digest of an evaluated `AccessPlan` — what `submit_write`
/// actually matches a caller-supplied `plan_hash` against. Public so a
/// real caller (having received the `AccessPlan` `evaluate()` returned)
/// can compute the exact matching hash to pass to `submit_write`, rather
/// than being expected to guess or reconstruct one. Serializes the
/// decision/filter/masks/caps/obligations/policy_version deterministically
/// (`serde_json` field order is declaration order, stable across calls)
/// rather than hashing a `Debug` string, which isn't a contract anything
/// should rely on staying stable.
pub fn access_plan_hash(plan: &AccessPlan) -> String {
	#[derive(serde::Serialize)]
	struct Hashable<'a> {
		decision: &'a Decision,
		filter: &'a Option<nirdosha_guard_core::FilterExpr>,
		masks: &'a Vec<FieldMask>,
		caps: &'a Vec<nirdosha_guard_core::Cap>,
		policy_version: &'a str,
	}
	let hashable = Hashable { decision: &plan.decision, filter: &plan.filter, masks: &plan.masks, caps: &plan.caps, policy_version: &plan.policy_version };
	let canonical = serde_json::to_string(&hashable).unwrap_or_default();
	let mut hasher = Sha256::new();
	hasher.update(canonical.as_bytes());
	hex_encode(&hasher.finalize())
}

#[cfg(test)]
mod tests {
	use super::*;
	use nirdosha_guard_core::evaluator::PolicyEffect;
	use nirdosha_guard_core::{Action, Classification, Environment, PaginationMode, QueryShape, Subject, Tenant};

	fn context(clearance: Classification) -> EvaluationContext {
		EvaluationContext {
			subject: Subject { id: "u1".into(), roles: vec!["Analyst".into()], claims: vec![], clearance },
			tenant: Tenant("t".into()),
			entity: "orders".into(),
			dataset: "memory".into(),
			action: Action::Read,
			destination: Destination::LlmContext,
			environment: Environment { env: "test".into(), ip: None, geo: None, device_posture: None, session_freshness: None },
			time_bucket: "now".into(),
			query_shape: QueryShape { verbs: vec![], aggregate: None, grouping_keys: vec![], subject_dimension: None, ordering: vec![], pagination: PaginationMode::LimitOnly { limit: 1 } },
			purpose: Purpose("support".into()),
			policy_version: "v1".into(),
		}
	}

	fn token() -> DelegationToken {
		GuardMcpServer::mint_token("u1", "a1", Purpose("support".into()), Destination::LlmContext, "v1", 1_000)
	}

	fn read_orders_policy() -> PolicyCandidate {
		PolicyCandidate { effect: PolicyEffect::Allow, subjects: vec!["Analyst".into()], action: Action::Read, resource: "orders".into(), purpose: Some("support".into()), conditions: vec![], filter: None, obligations: vec![], escalation: None, caps: vec![], masks: vec![], affected_row_cap: None, predicate_use: vec![], id: "allow-read-orders".into() }
	}

	#[test]
	fn agent_write_default_off() {
		let mut server = GuardMcpServer::new();
		let tok = token();
		let decision = server
			.submit_write(
				&tok,
				"hash".into(),
				WritePlan { decision: Decision::Allow, action: nirdosha_guard_core::WriteAction::Update, row_scope: None, preconditions: vec![], postconditions: vec![], field_policy: vec![], affected_row_cap: 1, obligations: vec![], policy_version: "v1".into() },
				2_000,
			)
			.unwrap();
		assert!(matches!(decision, Decision::Deny { .. }));
	}

	#[test]
	fn evaluate_with_no_registered_policy_denies_instead_of_silently_allowing() {
		// Plan Phase 14's core fix: an empty policy set must deny (via the
		// real evaluator), not just happen to also produce Deny through
		// the old hardcoded stub for an unrelated reason.
		let mut server = GuardMcpServer::new();
		let tok = token();
		let ctx = context(Classification::Internal);
		let (decision, _plan) = server.evaluate(&tok, &ctx, 2_000).unwrap();
		assert!(matches!(decision, Decision::Deny { .. }));
	}

	#[test]
	fn evaluate_allows_when_a_real_registered_policy_matches() {
		let mut server = GuardMcpServer::new().with_policies(vec![read_orders_policy()]);
		let tok = token();
		let ctx = context(Classification::Internal);
		let (decision, plan) = server.evaluate(&tok, &ctx, 2_000).unwrap();
		assert_eq!(decision, Decision::Allow);
		assert!(plan.caps.iter().any(|cap| matches!(cap, nirdosha_guard_core::Cap::RowCap(50))), "I17 default row_cap=50 must apply when no policy set a tighter cap: {:?}", plan.caps);
	}

	#[test]
	fn evaluate_then_act_allows_write_when_enabled_and_matched() {
		let mut server = GuardMcpServer::new().with_policies(vec![read_orders_policy()]);
		server.agent_writes_enabled = true;
		let tok = token();
		let ctx = context(Classification::Internal);
		let (decision, plan) = server.evaluate(&tok, &ctx, 2_000).unwrap();
		assert_eq!(decision, Decision::Allow);

		let plan_hash = access_plan_hash(&plan);
		let decision = server
			.submit_write(
				&tok,
				plan_hash,
				WritePlan { decision: Decision::Allow, action: nirdosha_guard_core::WriteAction::Update, row_scope: None, preconditions: vec![], postconditions: vec![], field_policy: vec![], affected_row_cap: 1, obligations: vec![], policy_version: "v1".into() },
				3_000,
			)
			.unwrap();
		assert_eq!(decision, Decision::Allow);
	}

	#[test]
	fn submit_write_rejects_a_guessed_plan_hash() {
		let mut server = GuardMcpServer::new().with_policies(vec![read_orders_policy()]);
		server.agent_writes_enabled = true;
		let tok = token();
		let ctx = context(Classification::Internal);
		let (decision, _plan) = server.evaluate(&tok, &ctx, 2_000).unwrap();
		assert_eq!(decision, Decision::Allow);

		// The pre-Phase-14 predictable format a caller could reconstruct
		// without ever calling evaluate().
		let guessed_hash = "plan-hash-orders".to_string();
		let decision = server
			.submit_write(
				&tok,
				guessed_hash,
				WritePlan { decision: Decision::Allow, action: nirdosha_guard_core::WriteAction::Update, row_scope: None, preconditions: vec![], postconditions: vec![], field_policy: vec![], affected_row_cap: 1, obligations: vec![], policy_version: "v1".into() },
				3_000,
			)
			.unwrap();
		assert!(matches!(decision, Decision::Deny { .. }), "a guessed/predictable plan hash must not match a real evaluated plan");
	}

	#[test]
	fn a_token_past_its_real_ttl_is_rejected() {
		let mut server = GuardMcpServer::new().with_policies(vec![read_orders_policy()]);
		let tok = GuardMcpServer::mint_token("u1", "a1", Purpose("support".into()), Destination::LlmContext, "v1", 1_000);
		assert_eq!(tok.ttl_ms, 30 * 60 * 1000, "I17 default ttl must be 30 minutes");
		let just_before_expiry = tok.expires_at_ms() - 1;
		let ctx = context(Classification::Internal);
		assert!(server.evaluate(&tok, &ctx, just_before_expiry).is_ok(), "a call just before expiry must still be honored");

		let after_expiry = tok.expires_at_ms();
		let result = server.evaluate(&tok, &ctx, after_expiry);
		assert_eq!(result, Err(ToolCallError::TokenExpired));
	}

	#[test]
	fn max_tool_calls_is_enforced_and_matches_the_real_i17_default() {
		let mut server = GuardMcpServer::new().with_policies(vec![read_orders_policy()]);
		let tok = GuardMcpServer::mint_token("u1", "a1", Purpose("support".into()), Destination::LlmContext, "v1", 1_000);
		assert_eq!(tok.max_tool_calls, 60, "I17 default max_tool_calls must be 60");
		let ctx = context(Classification::Internal);
		// Spend calls slowly enough to stay under the 20/min rate limit
		// (one call every 4 real seconds keeps well below 20/min) while
		// exhausting the 60-call budget.
		let mut now = 2_000u64;
		for _ in 0..60 {
			server.evaluate(&tok, &ctx, now).expect("must succeed within budget");
			now += 4_000;
		}
		let result = server.evaluate(&tok, &ctx, now);
		assert_eq!(result, Err(ToolCallError::MaxToolCallsExceeded { limit: 60 }));
	}

	#[test]
	fn rate_limit_is_enforced_within_a_sliding_one_minute_window() {
		let mut server = GuardMcpServer::new().with_policies(vec![read_orders_policy()]);
		let tok = GuardMcpServer::mint_token("u1", "a1", Purpose("support".into()), Destination::LlmContext, "v1", 1_000);
		let ctx = context(Classification::Internal);
		let mut now = 2_000u64;
		for _ in 0..20 {
			server.evaluate(&tok, &ctx, now).expect("must succeed within the 20/min budget");
			now += 100; // 20 calls well within one minute
		}
		let result = server.evaluate(&tok, &ctx, now);
		assert_eq!(result, Err(ToolCallError::RateLimitExceeded { limit_per_min: 20 }));

		// After the sliding window fully passes, the same token can call
		// again — this is a real rate limit, not a one-shot budget (that's
		// what max_tool_calls is for).
		let after_window = now + 61_000;
		assert!(server.evaluate(&tok, &ctx, after_window).is_ok());
	}

	#[test]
	fn fields_present_for_copilot_omits_masked_fields_entirely() {
		let fields = vec!["amount".to_string(), "ssn".to_string(), "status".to_string()];
		let masks = vec![FieldMask { field: vec!["ssn".into()], transform: nirdosha_guard_core::MaskTransform::Full }];
		let present = GuardMcpServer::fields_present_for_copilot(&fields, &masks);
		assert_eq!(present, vec!["amount".to_string(), "status".to_string()], "I17: masked fields must be absent, not replaced with a placeholder");
	}

	#[test]
	fn from_registry_only_generates_query_tools_for_entities_with_a_real_read_grant() {
		let dump = nirdosha_guard_registry::RegistryDump {
			policies: vec![nirdosha_guard_registry::PolicyRecord {
				id: "allow-read-orders".into(),
				effect: nirdosha_guard_registry::Effect::Allow,
				subjects: vec!["Analyst".into()],
				action: "read".into(),
				resource: "orders".into(),
				purpose: None,
				caps: vec![],
				affected_row_cap: None,
				obligations: vec![],
				escalation: None,
				field_policy: vec![],
				conditions: vec![],
				filter: None,
				filter_ref: None,
				masks: vec![],
				reason: None,
				destination: None,
				destination_denied_above: None,
				grants: vec![],
				predicate_use: vec![],
				count_allowed: false,
				policy_src: "test".into(),
				line: 1,
			}],
			datasets: vec![
				nirdosha_guard_registry::DatasetRecord { entity: "orders".into(), store: "pg".into(), fields: vec!["id".into()], classification: Classification::Internal },
				nirdosha_guard_registry::DatasetRecord { entity: "sar_bundle".into(), store: "pg".into(), fields: vec!["id".into()], classification: Classification::Restricted },
			],
			roles: vec![],
			ports: vec![],
			models: vec![],
			workflows: vec![nirdosha_guard_registry::WorkflowRecord { name: "CaseStatus".into(), states: vec!["open".into(), "closed".into()], edges: vec![("open".into(), "closed".into())] }],
			approval_chains: vec![],
			invariants: vec![],
			purposes: vec![],
			driver_manifests: vec![],
		};
		let server = GuardMcpServer::from_registry(&dump);
		let tool_names: Vec<&str> = server.tools.iter().map(|t| t.name.as_str()).collect();
		assert!(tool_names.contains(&"query_records_orders"), "orders has a real read grant, its query tool must be generated: {tool_names:?}");
		assert!(!tool_names.contains(&"query_records_sar_bundle"), "sar_bundle has no read grant in this dump, its query tool must NOT be generated: {tool_names:?}");
		assert!(tool_names.contains(&"get_options_CaseStatus"), "a real WorkflowRecord must produce a get_options tool: {tool_names:?}");
		assert!(tool_names.contains(&"evaluate") && tool_names.contains(&"submit_write"));
		assert_eq!(server.policies.len(), 1, "real policies from the dump must populate the server's evaluation set");
	}
}
