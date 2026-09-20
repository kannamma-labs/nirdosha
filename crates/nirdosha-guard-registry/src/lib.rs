//! RFC 0025 catalog registry foundation.

use nirdosha_guard_core::{Cap, Condition, EscalateTarget, FieldPolicy, FilterExpr, Obligation};
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
	pub obligations: Vec<Obligation>,
	pub escalation: Option<EscalateTarget>,
	pub field_policy: Vec<FieldPolicy>,
	pub conditions: Vec<Condition>,
	pub filter: Option<FilterExpr>,
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
		Some(PolicyCandidate {
			effect,
			subjects: self.subjects.iter().map(|s| s.to_string()).collect(),
			action,
			resource: self.resource.to_string(),
			purpose: self.purpose.map(|p| p.to_string()),
			conditions: Vec::new(),
			filter: None,
			obligations: Vec::new(),
			escalation: None,
			id: self.id.to_string(),
		})
	}
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
	pub policies: Vec<PolicyRegistration>,
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
		policies: POLICIES.to_vec(), datasets: DATASETS.to_vec(), roles: ROLES.to_vec(),
		ports: PORTS.to_vec(), models: MODELS.to_vec(), workflows: WORKFLOWS.to_vec(),
		approval_chains: APPROVAL_CHAINS.to_vec(), invariants: INVARIANTS.to_vec(),
		purposes: PURPOSES.to_vec(), driver_manifests: DRIVER_MANIFESTS.to_vec(),
	}
}

pub fn dump_json() -> Result<String, serde_json::Error> { serde_json::to_string(&dump()) }

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
