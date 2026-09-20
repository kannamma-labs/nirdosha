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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortRecord { pub name: String }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelRecord { pub name: String, pub version: String }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowRecord { pub name: String, pub states: Vec<String> }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalChainRecord { pub name: String, pub quorum: u8, pub approvers: Vec<String> }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvariantRecord { pub name: String }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PurposeRecord { pub code: String }

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
pub static PORTS: [PortRecord] = [..];
#[linkme::distributed_slice]
pub static MODELS: [ModelRecord] = [..];
#[linkme::distributed_slice]
pub static WORKFLOWS: [WorkflowRecord] = [..];
#[linkme::distributed_slice]
pub static APPROVAL_CHAINS: [ApprovalChainRecord] = [..];
#[linkme::distributed_slice]
pub static INVARIANTS: [InvariantRecord] = [..];
#[linkme::distributed_slice]
pub static PURPOSES: [PurposeRecord] = [..];
#[linkme::distributed_slice]
pub static DRIVER_MANIFESTS: [DriverManifestRecord] = [..];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
}

pub fn dump() -> RegistryDump {
	RegistryDump {
		policies: records(), datasets: DATASETS.to_vec(), roles: ROLES.to_vec(),
		ports: PORTS.to_vec(), models: MODELS.to_vec(), workflows: WORKFLOWS.to_vec(),
		approval_chains: APPROVAL_CHAINS.to_vec(), invariants: INVARIANTS.to_vec(),
		purposes: PURPOSES.to_vec(), driver_manifests: DRIVER_MANIFESTS.to_vec(),
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
	vec!["policies", "datasets", "roles", "ports", "models", "workflows", "approval_chains", "invariants", "purposes", "driver_manifests"]
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
}
