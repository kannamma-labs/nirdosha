//! RFC 0025 catalog registry foundation.

mod clauses;
pub use clauses::{lower as lower_clauses, LoweredClauses};

use nirdosha_guard_core::{
	Cap, Classification, Condition, Destination, EscalateTarget, FieldMask, FieldPolicy,
	FilterExpr, Obligation,
};
use serde::{Deserialize, Serialize};

pub use linkme;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyRecord {
	pub id: String,
	pub effect: Effect,
	pub subjects: Vec<String>,
	pub action: String,
	pub resource: String,
	pub purpose: Option<String>,
	pub caps: Vec<Cap>,
	/// `cap(affected_rows = N)` — `WritePlan::affected_row_cap`'s source,
	/// a different concept from `caps` (rows a mutation may touch, not
	/// rows a read may scan).
	pub affected_row_cap: Option<u64>,
	pub obligations: Vec<Obligation>,
	pub escalation: Option<EscalateTarget>,
	pub field_policy: Vec<FieldPolicy>,
	pub conditions: Vec<Condition>,
	pub filter: Option<FilterExpr>,
	/// A `filter <scope-fn>(...)` clause this lowering pass can name but
	/// not resolve to a concrete `FilterExpr` without a live
	/// `EvaluationContext` (`tenant_scope()`, `subject_scope()`,
	/// `delegation_scope()`, `time_range(...)`). See `clauses.rs`.
	pub filter_ref: Option<String>,
	pub masks: Vec<FieldMask>,
	/// `reason(sod.xxx)` — the deny-justification code on `deny` policies.
	pub reason: Option<String>,
	pub destination: Option<Destination>,
	pub destination_denied_above: Option<Classification>,
	/// `grant predicate_use(...)` / `grant count_allowed`'s original
	/// clause text, kept alongside the structured `predicate_use`/
	/// `count_allowed` fields below for anything that still wants the raw
	/// source.
	pub grants: Vec<String>,
	/// Field names `grant predicate_use(...)` allows in non-projection
	/// clauses (filter/join/grouping/having/ordering/window) despite
	/// being masked — I15's exception list.
	pub predicate_use: Vec<String>,
	pub count_allowed: bool,
	pub policy_src: String,
	pub line: u32,
}

/// Serialized lowercase (`"allow"`/`"deny"`) to match the wire vocabulary
/// `guard_policy!` source uses and `nirdosha-guard-verify::PolicyView`
/// expects — the previous default (PascalCase `"Allow"`/`"Deny"`) meant
/// `nirdosha-guard-verify`'s V2 pass (`effect != "allow" && effect != "deny"`)
/// would reject every real registration once fed through `dump_json()`,
/// a mismatch nothing caught because nothing called `dump_json()` end to
/// end until this phase wired it up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effect { Allow, Deny }

/// Const-friendly registration emitted by proc macros. Gate 2 expands this
/// into the owned `PolicyRecord` model before running whole-catalog checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PolicyRegistration {
	pub id: &'static str,
	pub effect: Effect,
	/// Role names from `for <Role>, <Role>...`. Empty means "any subject"
	/// (matches `PolicyCandidate::subjects.is_empty()` semantics in
	/// `nirdosha_guard_core::evaluator::matches_context`).
	pub subjects: &'static [&'static str],
	pub action: &'static str,
	pub resource: &'static str,
	pub purpose: Option<&'static str>,
	pub clauses_json: Option<&'static str>,
	pub source: &'static str,
	pub line: u32,
}

impl PolicyRegistration {
	/// Expands this const registration into the runtime `PolicyCandidate`
	/// the evaluator actually matches against. Only identity/routing fields
	/// (subjects/action/resource/purpose/effect) are populated here —
	/// clause-derived fields (`filter`/`caps`/`obligations`/`escalation`)
	/// come from real clause lowering, not yet wired into this
	/// registration shape.
	///
	/// Returns `None` if `action` isn't one of the closed `Action` wire
	/// strings (`read`/`create`/`update`/`delete`/`migrate`/`export`/
	/// `enumerate`) — a policy source typo surfaces here, not as a silent
	/// always-deny.
	pub fn to_candidate(&self) -> Option<nirdosha_guard_core::evaluator::PolicyCandidate> {
		use nirdosha_guard_core::evaluator::{PolicyCandidate, PolicyEffect};
		let action = nirdosha_guard_core::Action::parse_wire(self.action)?;
		let effect = match self.effect {
			Effect::Allow => PolicyEffect::Allow,
			Effect::Deny => PolicyEffect::Deny,
		};
		let lowered = clauses::lower(self.clauses_json);
		Some(PolicyCandidate {
			effect,
			subjects: self.subjects.iter().map(|s| s.to_string()).collect(),
			action,
			resource: self.resource.to_string(),
			purpose: self.purpose.map(|p| p.to_string()),
			conditions: lowered.conditions,
			filter: lowered.filter,
			obligations: lowered.obligations,
			escalation: lowered.escalation,
			caps: lowered.caps,
			masks: lowered.masks,
			affected_row_cap: lowered.affected_row_cap,
			predicate_use: lowered.predicate_use,
			id: self.id.to_string(),
		})
	}

	/// Full "Gate 2" expansion: every clause this registration's macro
	/// invocation carried, lowered into `PolicyRecord`'s structured form.
	/// Unlike [`to_candidate`], which only carries what the evaluator
	/// needs (decision-time concerns), this also carries execution-time
	/// concerns (`caps`, `masks`, `field_policy`, `destination`, ...) —
	/// the `AccessPlan`/`WritePlan` building work later phases do.
	pub fn to_record(&self) -> PolicyRecord {
		let lowered = clauses::lower(self.clauses_json);
		PolicyRecord {
			id: self.id.to_string(),
			effect: self.effect,
			subjects: self.subjects.iter().map(|s| s.to_string()).collect(),
			action: self.action.to_string(),
			resource: self.resource.to_string(),
			purpose: self.purpose.map(|p| p.to_string()),
			caps: lowered.caps,
			affected_row_cap: lowered.affected_row_cap,
			obligations: lowered.obligations,
			escalation: lowered.escalation,
			field_policy: lowered.field_policy,
			conditions: lowered.conditions,
			filter: lowered.filter,
			filter_ref: lowered.filter_ref,
			masks: lowered.masks,
			reason: lowered.reason,
			destination: lowered.destination,
			destination_denied_above: lowered.destination_denied_above,
			grants: lowered.grants,
			predicate_use: lowered.predicate_use,
			count_allowed: lowered.count_allowed,
			policy_src: self.source.to_string(),
			line: self.line,
		}
	}
}

impl PolicyRecord {
	/// The inverse direction of [`PolicyRegistration::to_candidate`]: a
	/// fully Gate-2-lowered `PolicyRecord` (e.g. from a `RegistryDump` a
	/// caller loaded from disk, not a live `linkme` slice) into the
	/// `PolicyCandidate` shape `evaluator::evaluate` matches against.
	/// Needed by `nirdosha-guard-mcp` (Plan Phase 14) to evaluate real
	/// registry-sourced policies against MCP tool calls — previously
	/// nothing outside a live macro-linked process could construct a
	/// `PolicyCandidate` from a plain registry dump at all.
	///
	/// Returns `None` for the same reason `PolicyRegistration::to_candidate`
	/// does: an `action` string outside the closed `Action` wire
	/// vocabulary surfaces here, not as a silent always-deny.
	pub fn to_candidate(&self) -> Option<nirdosha_guard_core::evaluator::PolicyCandidate> {
		use nirdosha_guard_core::evaluator::{PolicyCandidate, PolicyEffect};
		let action = nirdosha_guard_core::Action::parse_wire(&self.action)?;
		let effect = match self.effect {
			Effect::Allow => PolicyEffect::Allow,
			Effect::Deny => PolicyEffect::Deny,
		};
		Some(PolicyCandidate {
			effect,
			subjects: self.subjects.clone(),
			action,
			resource: self.resource.clone(),
			purpose: self.purpose.clone(),
			conditions: self.conditions.clone(),
			filter: self.filter.clone(),
			obligations: self.obligations.clone(),
			escalation: self.escalation.clone(),
			caps: self.caps.clone(),
			masks: self.masks.clone(),
			affected_row_cap: self.affected_row_cap,
			predicate_use: self.predicate_use.clone(),
			id: self.id.clone(),
		})
	}
}

/// All registered policies, fully expanded into [`PolicyRecord`]s — the
/// "owned `PolicyRecord` model" this crate's registration type has, since
/// its introduction, said Gate 2 would produce.
pub fn records() -> Vec<PolicyRecord> {
	POLICIES.iter().map(PolicyRegistration::to_record).collect()
}

/// All registered policies, expanded into evaluator-ready candidates.
/// Registrations whose `action` doesn't parse are skipped (silently
/// dropped, not silently mis-evaluated) — callers that need to catch that
/// case should compare `POLICIES.len()` against this Vec's length, which is
/// exactly what `nirdosha-guard-verify`'s V1 pass is for.
pub fn candidates() -> Vec<nirdosha_guard_core::evaluator::PolicyCandidate> {
	POLICIES.iter().filter_map(PolicyRegistration::to_candidate).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CatalogRegistration {
	pub kind: &'static str,
	pub name: &'static str,
	pub source: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatasetRecord { pub entity: String, pub store: String, pub fields: Vec<String>, pub classification: nirdosha_guard_core::Classification }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleRecord { pub name: String }

/// `stream_port!`'s owned record — a `bind`/`publish` binding for one named
/// port (RFC 0025 §8.1/§6.5). `direction` is `"bind"` or `"publish"`;
/// `target` is the bound topic/rail literal. `format`/`schema`/`semantics`
/// mirror the doc's optional `format = "avro"; schema = "transaction";
/// semantics = at_least_once;` clauses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortRecord {
	pub name: String,
	pub direction: String,
	pub target: String,
	pub format: Option<String>,
	pub schema: Option<String>,
	pub semantics: Option<String>,
}

/// Const-constructible counterpart to [`PortRecord`] — see
/// [`ApprovalChainRegistration`]'s doc comment for why this split exists
/// (owned `String`/`Option<String>` fields can't be built in a `const`
/// context, so the macro-facing type is all `&'static str`/`Option<&'static str>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortRegistration {
	pub name: &'static str,
	pub direction: &'static str,
	pub target: &'static str,
	pub format: Option<&'static str>,
	pub schema: Option<&'static str>,
	pub semantics: Option<&'static str>,
}

impl PortRegistration {
	pub fn to_record(&self) -> PortRecord {
		PortRecord {
			name: self.name.to_string(),
			direction: self.direction.to_string(),
			target: self.target.to_string(),
			format: self.format.map(str::to_string),
			schema: self.schema.map(str::to_string),
			semantics: self.semantics.map(str::to_string),
		}
	}
}

/// `model_artifact!`'s owned record (RFC 0025 §8.3). `version` defaults to
/// `""` when the doc's `model { ... }` block never sets one — this grammar
/// has no `version = ...` clause today, unlike the pre-existing
/// `ModelRecord.version` field this record still carries for whatever
/// future macro/tooling wants to set it explicitly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelRecord {
	pub name: String,
	pub version: String,
	pub format: String,
	pub inputs: Vec<String>,
	pub outputs: Vec<String>,
	pub threshold_alert: Option<f64>,
}

/// Const-constructible counterpart to [`ModelRecord`] — see
/// [`ApprovalChainRegistration`]'s doc comment for why this split exists.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelRegistration {
	pub name: &'static str,
	pub version: &'static str,
	pub format: &'static str,
	pub inputs: &'static [&'static str],
	pub outputs: &'static [&'static str],
	pub threshold_alert: Option<f64>,
}

impl ModelRegistration {
	pub fn to_record(&self) -> ModelRecord {
		ModelRecord {
			name: self.name.to_string(),
			version: self.version.to_string(),
			format: self.format.to_string(),
			inputs: self.inputs.iter().map(|s| s.to_string()).collect(),
			outputs: self.outputs.iter().map(|s| s.to_string()).collect(),
			threshold_alert: self.threshold_alert,
		}
	}
}

/// `matcher!`'s owned record (RFC 0025 §8.4) — a fuzzy-matching algorithm
/// bound to a set of watchlist ids. No pre-existing `MatcherRecord`/slice
/// existed before this phase (unlike `PORTS`/`MODELS`/`PURPOSES`, which
/// were declared-but-unpopulated); both are new.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatcherRecord { pub name: String, pub algorithm: String, pub threshold: f64, pub lists: Vec<String> }

/// Const-constructible counterpart to [`MatcherRecord`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MatcherRegistration {
	pub name: &'static str,
	pub algorithm: &'static str,
	pub threshold: f64,
	pub lists: &'static [&'static str],
}

impl MatcherRegistration {
	pub fn to_record(&self) -> MatcherRecord {
		MatcherRecord {
			name: self.name.to_string(),
			algorithm: self.algorithm.to_string(),
			threshold: self.threshold,
			lists: self.lists.iter().map(|s| s.to_string()).collect(),
		}
	}
}

/// `window!`'s owned record (RFC 0025 §8.2) — a feature window's `name`,
/// `key`, and `kind` (`sliding`/`hopping`/`session`) are pulled out
/// structurally; the remainder (the window/period, `keys=[...]`,
/// `aggs=[...]`/`expr=...`) is kept as opaque `spec` source text — see
/// `nirdosha-guard-macros::window!`'s own doc comment for why that
/// remainder isn't parsed further.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowRecord { pub name: String, pub key: String, pub kind: String, pub spec: String }

/// Const-constructible counterpart to [`WindowRecord`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowRegistration {
	pub name: &'static str,
	pub key: &'static str,
	pub kind: &'static str,
	pub spec: &'static str,
}

impl WindowRegistration {
	pub fn to_record(&self) -> WindowRecord {
		WindowRecord { name: self.name.to_string(), key: self.key.to_string(), kind: self.kind.to_string(), spec: self.spec.to_string() }
	}
}

/// `mcp_tools!`'s owned record (RFC 0024/0025 §8.8) — one registered MCP
/// server's identity, tool roster, delegation limits, and I17 defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerRecord {
	pub name: String,
	pub identity: String,
	pub tools: Vec<String>,
	pub ttl: Option<String>,
	pub max_tool_calls: Option<u32>,
	pub rate: Option<String>,
	pub audit_default: Option<String>,
	pub row_cap_default: Option<u64>,
	pub destination_default: Option<String>,
}

/// Const-constructible counterpart to [`McpServerRecord`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpServerRegistration {
	pub name: &'static str,
	pub identity: &'static str,
	pub tools: &'static [&'static str],
	pub ttl: Option<&'static str>,
	pub max_tool_calls: Option<u32>,
	pub rate: Option<&'static str>,
	pub audit_default: Option<&'static str>,
	pub row_cap_default: Option<u64>,
	pub destination_default: Option<&'static str>,
}

impl McpServerRegistration {
	pub fn to_record(&self) -> McpServerRecord {
		McpServerRecord {
			name: self.name.to_string(),
			identity: self.identity.to_string(),
			tools: self.tools.iter().map(|s| s.to_string()).collect(),
			ttl: self.ttl.map(str::to_string),
			max_tool_calls: self.max_tool_calls,
			rate: self.rate.map(str::to_string),
			audit_default: self.audit_default.map(str::to_string),
			row_cap_default: self.row_cap_default,
			destination_default: self.destination_default.map(str::to_string),
		}
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowRecord { pub name: String, pub states: Vec<String> }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalChainRecord { pub name: String, pub quorum: u8, pub approvers: Vec<String> }

/// Const-constructible counterpart to [`ApprovalChainRecord`] — what
/// `approval_chain!` actually emits into a `static` (`APPROVAL_CHAINS`
/// distributed-slice item), since `String`/`Vec<String>` can't be built
/// in a `const`/`static` context. `ApprovalChainRecord` (owned) is what a
/// `RegistryDump` carries and what `nirdosha-guard-core`'s
/// `ApprovalChainRuntime` (Plan Phase 15) is built from — the same
/// `*Registration` (const, macro-facing) vs. `*Record` (owned,
/// runtime-facing) split `PolicyRegistration`/`PolicyRecord` already
/// establishes for policies.
///
/// Before this phase, `ApprovalChainRecord` itself was the slice's item
/// type — meaning `APPROVAL_CHAINS` could never actually be populated at
/// all, regardless of whether `approval_chain!` tried to (a `static`
/// item's value must be a `const` expression, and owned `String`/`Vec`
/// values aren't). The same construction problem affects `RoleRecord`/
/// `PortRecord`/`ModelRecord`/`WorkflowRecord`/`DatasetRecord` — none of
/// which anything in `nirdosha-guard-macros` populates either, for the
/// identical reason. Out of scope for this phase (which fixes only the
/// one slice `approval_chain!` reachability actually needs), noted here
/// so it isn't rediscovered as a surprise later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalChainRegistration {
	pub name: &'static str,
	pub quorum: u8,
	pub approvers: &'static [&'static str],
}

impl ApprovalChainRegistration {
	pub fn to_record(&self) -> ApprovalChainRecord {
		ApprovalChainRecord { name: self.name.to_string(), quorum: self.quorum, approvers: self.approvers.iter().map(|s| s.to_string()).collect() }
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvariantRecord { pub name: String }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PurposeRecord { pub code: String }

/// Const-constructible counterpart to [`PurposeRecord`] — what
/// `purpose_taxonomy!` emits, one per `enum Purpose { Variant, ... }`
/// variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PurposeRegistration { pub code: &'static str }

impl PurposeRegistration {
	pub fn to_record(&self) -> PurposeRecord {
		PurposeRecord { code: self.code.to_string() }
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriverManifestRecord {
	pub port: String,
	pub vendor: String,
	pub version: String,
	pub capabilities: Vec<String>,
	pub lineage_support: String,
	pub guard_min: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntitySchema { pub entity: String, pub fields: Vec<String> }

#[linkme::distributed_slice]
pub static POLICIES: [PolicyRegistration] = [..];
#[linkme::distributed_slice]
pub static CATALOG: [CatalogRegistration] = [..];
#[linkme::distributed_slice]
pub static DATASETS: [DatasetRecord] = [..];
#[linkme::distributed_slice]
pub static ROLES: [RoleRecord] = [..];
#[linkme::distributed_slice]
pub static PORTS: [PortRegistration] = [..];
#[linkme::distributed_slice]
pub static MODELS: [ModelRegistration] = [..];
#[linkme::distributed_slice]
pub static WORKFLOWS: [WorkflowRecord] = [..];
#[linkme::distributed_slice]
pub static APPROVAL_CHAINS: [ApprovalChainRegistration] = [..];
#[linkme::distributed_slice]
pub static INVARIANTS: [InvariantRecord] = [..];
#[linkme::distributed_slice]
pub static PURPOSES: [PurposeRegistration] = [..];
#[linkme::distributed_slice]
pub static DRIVER_MANIFESTS: [DriverManifestRecord] = [..];
#[linkme::distributed_slice]
pub static MATCHERS: [MatcherRegistration] = [..];
#[linkme::distributed_slice]
pub static WINDOWS: [WindowRegistration] = [..];
#[linkme::distributed_slice]
pub static MCP_SERVERS: [McpServerRegistration] = [..];

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RegistryDump {
	/// Fully lowered ("Gate 2" expanded) records, not the raw
	/// `PolicyRegistration` — a consumer of the dump (`cargo nirdosha
	/// verify --guard` foremost) needs the real clause data (subjects,
	/// caps, field_policy, conditions, ...) to check anything meaningful;
	/// the opaque `clauses_json` blob on `PolicyRegistration` was never
	/// meant to be that consumer's input.
	pub policies: Vec<PolicyRecord>,
	pub datasets: Vec<DatasetRecord>,
	pub roles: Vec<RoleRecord>,
	pub ports: Vec<PortRecord>,
	pub models: Vec<ModelRecord>,
	pub workflows: Vec<WorkflowRecord>,
	pub approval_chains: Vec<ApprovalChainRecord>,
	pub invariants: Vec<InvariantRecord>,
	pub purposes: Vec<PurposeRecord>,
	pub driver_manifests: Vec<DriverManifestRecord>,
	pub matchers: Vec<MatcherRecord>,
	pub windows: Vec<WindowRecord>,
	pub mcp_servers: Vec<McpServerRecord>,
}

pub fn dump() -> RegistryDump {
	RegistryDump {
		policies: records(), datasets: DATASETS.to_vec(), roles: ROLES.to_vec(),
		ports: PORTS.iter().map(PortRegistration::to_record).collect(),
		models: MODELS.iter().map(ModelRegistration::to_record).collect(),
		workflows: WORKFLOWS.to_vec(),
		approval_chains: APPROVAL_CHAINS.iter().map(ApprovalChainRegistration::to_record).collect(), invariants: INVARIANTS.to_vec(),
		purposes: PURPOSES.iter().map(PurposeRegistration::to_record).collect(),
		driver_manifests: DRIVER_MANIFESTS.to_vec(),
		matchers: MATCHERS.iter().map(MatcherRegistration::to_record).collect(),
		windows: WINDOWS.iter().map(WindowRegistration::to_record).collect(),
		mcp_servers: MCP_SERVERS.iter().map(McpServerRegistration::to_record).collect(),
	}
}

pub fn dump_json() -> Result<String, serde_json::Error> { serde_json::to_string(&dump()) }

/// The env var `cargo nirdosha verify --guard` sets before running a
/// dialect crate's dump test, and the default path used when it's unset
/// (matching the CLI's own `--registry-json` default,
/// `nirdosha-registry.json`).
pub const GUARD_DUMP_PATH_ENV: &str = "NIRDOSHA_GUARD_DUMP_PATH";
const DEFAULT_GUARD_DUMP_PATH: &str = "nirdosha-registry.json";

/// The one-line convention a dialect crate needs so `cargo nirdosha
/// verify --guard` can produce its own input without a manual step: a
/// `#[test]` (any name, any file — `cargo test` finds it by content, not
/// by a fixed path) that calls this function. `POLICIES`/`DATASETS`/etc.
/// only populate once the crate that declared them has actually been
/// *linked* into a running binary — a plain `cargo build` doesn't run
/// anything, so the dump has to happen from inside a `cargo test`/`cargo
/// run` process, not be synthesized by the CLI tool from source alone.
///
/// ```ignore
/// #[test]
/// fn nirdosha_guard_dump() {
///     nirdosha_guard_registry::write_dump_from_env().unwrap();
/// }
/// ```
pub fn write_dump_from_env() -> std::io::Result<std::path::PathBuf> {
	let path = std::env::var(GUARD_DUMP_PATH_ENV)
		.map(std::path::PathBuf::from)
		.unwrap_or_else(|_| std::path::PathBuf::from(DEFAULT_GUARD_DUMP_PATH));
	let json = dump_json().map_err(std::io::Error::other)?;
	std::fs::write(&path, json)?;
	Ok(path)
}

pub fn coverage_matrix() -> Vec<&'static str> {
	vec!["policies", "datasets", "roles", "ports", "models", "workflows", "approval_chains", "invariants", "purposes", "driver_manifests", "matchers", "windows", "mcp_servers"]
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn empty_slices_dump_as_stable_json() {
		let value: serde_json::Value = serde_json::from_str(&dump_json().unwrap()).unwrap();
		assert_eq!(value["policies"], serde_json::json!([]));
		assert!(coverage_matrix().contains(&"driver_manifests"));
	}

	#[test]
	fn policy_record_to_candidate_round_trips_a_real_record() {
		let record = PolicyRecord {
			id: "allow-read-orders".into(),
			effect: Effect::Allow,
			subjects: vec!["Analyst".into()],
			action: "read".into(),
			resource: "orders".into(),
			purpose: Some("support".into()),
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
		};
		let candidate = record.to_candidate().expect("a real \"read\" action must parse");
		assert_eq!(candidate.id, "allow-read-orders");
		assert_eq!(candidate.subjects, vec!["Analyst".to_string()]);
		assert_eq!(candidate.action, nirdosha_guard_core::Action::Read);
		assert_eq!(candidate.resource, "orders");
	}

	#[test]
	fn policy_record_to_candidate_rejects_an_unparseable_action() {
		let record = PolicyRecord {
			id: "bad".into(),
			effect: Effect::Allow,
			subjects: vec![],
			action: "not_a_real_action".into(),
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
		};
		assert!(record.to_candidate().is_none(), "an unparseable action must surface as None, not a silent default");
	}
}
