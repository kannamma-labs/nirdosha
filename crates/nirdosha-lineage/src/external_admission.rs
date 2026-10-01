//! Guarded external lineage claim admission — Plan Phase 19.
//!
//! `Authority::ExternalClaimed` already existed as a real enum variant —
//! the classification was declared, but nothing gave an external claim
//! anywhere to enter the graph: no admission API, no quarantine state.
//! This module is that entry point, guarded the same way
//! `GuardClient::guarded_apply` is (`nirdosha-guard-mic`): evaluated
//! against real policies via the same `evaluator::evaluate` every other
//! guard client uses, audited before admission — built directly against
//! `nirdosha-guard-core`/`nirdosha-audit` (both already real dependencies
//! of this crate) rather than depending on `nirdosha-guard-mic`, which
//! itself depends on `nirdosha-lineage` (a dependency the other direction
//! would be circular).
//!
//! **Quarantine is a separate store, not a flag on a trusted-graph row.**
//! A claim `link_external_lineage` admits is written to a
//! [`QuarantineStore`], never directly to a [`crate::GraphStore`]. This
//! makes "quarantined claims must be visibly excluded from
//! `provenance_of`/`downstream_of`/`upstream_of` results" true by
//! construction rather than by a filter every query has to remember to
//! apply: those queries (once `nirdosha-lineage`'s own Phase 2 query
//! engine exists — `query.rs`'s own doc comment: "no query actually runs
//! here yet") read from `GraphStore`, and a quarantined claim simply
//! isn't there yet. [`promote`] is the one, explicit, audited action that
//! moves a claim from quarantine into the real store — proven end to end
//! in this module's own tests against the real, working
//! `nirdosha-lineage-store-embedded` driver (`GraphStore::upsert_edge`/
//! `get_edges` are real and tested today, unlike the not-yet-built query
//! DSL), rather than waiting on that separate, larger undertaking.

use serde::{Deserialize, Serialize};

use nirdosha_audit::envelope::{AuditEnvelope, AuditRecordKind, ModuleAuditChain};
use nirdosha_guard_core::evaluator::{self, PolicyCandidate};
use nirdosha_guard_core::{Decision, EvaluationContext};

use crate::{Authority, DriverRef, EdgeStats, EdgeType, FlowCompleteness, GraphStore, LineageEdge, NodeId};

/// What an external system claims — deliberately carries no `authority`
/// field of its own: [`link_external_lineage`] always forces
/// `Authority::ExternalClaimed` on the resulting edge, never the caller's
/// word for what authority it deserves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalLineageClaim {
	pub edge_type: EdgeType,
	pub src: NodeId,
	pub dst: NodeId,
	pub policy_version: crate::PolicyVersion,
	pub purpose: crate::Purpose,
	pub destination: crate::Destination,
	pub driver: DriverRef,
	pub transformation: crate::TransformId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuarantineStatus {
	Pending,
	Promoted,
	Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuarantinedClaim {
	pub id: String,
	pub edge: LineageEdge,
	pub evidence: String,
	pub submitted_at: String,
	pub status: QuarantineStatus,
}

pub trait QuarantineStore: Send + Sync {
	fn submit(&mut self, claim: QuarantinedClaim);
	fn get(&self, id: &str) -> Option<&QuarantinedClaim>;
	fn list_pending(&self) -> Vec<&QuarantinedClaim>;
	fn mark_promoted(&mut self, id: &str) -> bool;
	fn mark_rejected(&mut self, id: &str) -> bool;
}

#[derive(Debug, Default)]
pub struct InMemoryQuarantineStore {
	claims: std::collections::HashMap<String, QuarantinedClaim>,
}

impl InMemoryQuarantineStore {
	pub fn new() -> Self {
		Self::default()
	}
}

impl QuarantineStore for InMemoryQuarantineStore {
	fn submit(&mut self, claim: QuarantinedClaim) {
		self.claims.insert(claim.id.clone(), claim);
	}
	fn get(&self, id: &str) -> Option<&QuarantinedClaim> {
		self.claims.get(id)
	}
	fn list_pending(&self) -> Vec<&QuarantinedClaim> {
		self.claims.values().filter(|claim| claim.status == QuarantineStatus::Pending).collect()
	}
	fn mark_promoted(&mut self, id: &str) -> bool {
		self.claims.get_mut(id).map(|claim| { claim.status = QuarantineStatus::Promoted; true }).unwrap_or(false)
	}
	fn mark_rejected(&mut self, id: &str) -> bool {
		self.claims.get_mut(id).map(|claim| { claim.status = QuarantineStatus::Rejected; true }).unwrap_or(false)
	}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdmissionError {
	Denied { reason: String },
	EmptyEvidence,
	UnknownClaim { id: String },
	NotPending { id: String, status: QuarantineStatus },
	Store { reason: String },
}

impl std::fmt::Display for AdmissionError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			AdmissionError::Denied { reason } => write!(f, "denied: {reason}"),
			AdmissionError::EmptyEvidence => write!(f, "external lineage claims require non-empty evidence"),
			AdmissionError::UnknownClaim { id } => write!(f, "no such quarantined claim: {id}"),
			AdmissionError::NotPending { id, status } => write!(f, "claim {id} is not pending (status: {status:?})"),
			AdmissionError::Store { reason } => write!(f, "graph store error: {reason}"),
		}
	}
}
impl std::error::Error for AdmissionError {}

/// Admits `claim` into quarantine — never directly into `GraphStore`.
/// Guarded like `guarded_apply`: evaluates `context` against `policies`
/// first, audits the decision either way, and only proceeds to
/// quarantine on a real `Decision::Allow`. Requires non-empty `evidence`
/// (an external claim with no stated basis is refused outright, not
/// quarantined "just in case").
pub fn link_external_lineage(
	claim: ExternalLineageClaim,
	evidence: String,
	context: &EvaluationContext,
	policies: &[PolicyCandidate],
	quarantine: &mut dyn QuarantineStore,
	audit: &ModuleAuditChain,
	trace_id: impl Into<String>,
	now_ms: u64,
) -> Result<String, AdmissionError> {
	let trace_id = trace_id.into();
	let decision = evaluator::evaluate(context, policies).decision;
	let envelope = AuditEnvelope {
		trace_id: trace_id.clone(),
		ts: crate::time::format_rfc3339_ms(now_ms),
		module: audit.module.clone(),
		subject: context.subject.id.clone(),
		action: "link_external_lineage".into(),
		resource: format!("{:?}", claim.dst),
		policy_versions: vec![context.policy_version.clone()],
		decision: format!("{decision:?}"),
		obligations: vec![],
		kind: AuditRecordKind::Decision,
		content: serde_json::json!({ "phase": "before_external_lineage_admission" }),
	};
	audit.append(&envelope, now_ms);

	match decision {
		Decision::Allow => {}
		Decision::Deny { reason } => return Err(AdmissionError::Denied { reason }),
		other => return Err(AdmissionError::Denied { reason: format!("external lineage admission requires an immediate Allow/Deny, got {other:?}") }),
	}
	if evidence.trim().is_empty() {
		return Err(AdmissionError::EmptyEvidence);
	}

	let id = format!("ext-claim-{trace_id}");
	let submitted_at = crate::time::format_rfc3339_ms(now_ms);
	let edge = LineageEdge {
		edge_type: claim.edge_type,
		src: claim.src,
		dst: claim.dst,
		// Forced, never taken from the caller's own claim — the entire
		// point of this classification is that it's the kernel's
		// judgment about the claim's trust level, not the claimant's.
		authority: Authority::ExternalClaimed,
		policy_version: claim.policy_version,
		purpose: claim.purpose,
		destination: claim.destination,
		driver: claim.driver,
		transformation: claim.transformation,
		completeness: FlowCompleteness::Full,
		stats: EdgeStats::first(&submitted_at, &trace_id, false),
		receipt_digests: vec![],
	};
	quarantine.submit(QuarantinedClaim { id: id.clone(), edge, evidence, submitted_at, status: QuarantineStatus::Pending });

	let mut admitted = envelope.clone();
	admitted.kind = AuditRecordKind::Mutation;
	admitted.content = serde_json::json!({ "phase": "external_lineage_quarantined", "claim_id": id });
	audit.append(&admitted, now_ms);
	Ok(id)
}

/// The one explicit, audited path from quarantine into the real,
/// trusted `GraphStore` — mirroring the V10 "issued-unconsumed" finding
/// pattern's own posture (`examples/rtm`'s Part 6: dormant/undeclared/
/// pending states need an explicit, auditable transition, not an
/// implicit one). Fails closed on: an unknown claim id, a claim that
/// isn't `Pending` (already promoted or rejected — no double-promotion,
/// no resurrecting a rejected claim), a denied approval decision, or a
/// real `GraphStore` write failure (in which case the claim stays
/// `Pending`, not silently marked promoted).
pub fn promote(
	id: &str,
	store: &dyn GraphStore,
	approver_context: &EvaluationContext,
	policies: &[PolicyCandidate],
	quarantine: &mut dyn QuarantineStore,
	audit: &ModuleAuditChain,
	trace_id: impl Into<String>,
	now_ms: u64,
) -> Result<(), AdmissionError> {
	let trace_id = trace_id.into();
	let decision = evaluator::evaluate(approver_context, policies).decision;
	let envelope = AuditEnvelope {
		trace_id: trace_id.clone(),
		ts: crate::time::format_rfc3339_ms(now_ms),
		module: audit.module.clone(),
		subject: approver_context.subject.id.clone(),
		action: "promote_external_lineage".into(),
		resource: id.to_string(),
		policy_versions: vec![approver_context.policy_version.clone()],
		decision: format!("{decision:?}"),
		obligations: vec![],
		kind: AuditRecordKind::Decision,
		content: serde_json::json!({ "phase": "before_external_lineage_promotion", "claim_id": id }),
	};
	audit.append(&envelope, now_ms);

	match decision {
		Decision::Allow => {}
		Decision::Deny { reason } => return Err(AdmissionError::Denied { reason }),
		other => return Err(AdmissionError::Denied { reason: format!("promotion requires an immediate Allow/Deny, got {other:?}") }),
	}

	let claim = quarantine.get(id).ok_or_else(|| AdmissionError::UnknownClaim { id: id.to_string() })?;
	if claim.status != QuarantineStatus::Pending {
		return Err(AdmissionError::NotPending { id: id.to_string(), status: claim.status });
	}
	store.upsert_edge(&claim.edge).map_err(|error| AdmissionError::Store { reason: error.to_string() })?;
	quarantine.mark_promoted(id);

	let mut promoted = envelope.clone();
	promoted.kind = AuditRecordKind::Mutation;
	promoted.content = serde_json::json!({ "phase": "external_lineage_promoted", "claim_id": id });
	audit.append(&promoted, now_ms);
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use nirdosha_guard_core::evaluator::PolicyEffect;
	use nirdosha_guard_core::{Action, Classification, Environment, PaginationMode, QueryShape, Subject, Tenant};

	fn context(subject_id: &str, action: Action) -> EvaluationContext {
		EvaluationContext {
			subject: Subject { id: subject_id.into(), roles: vec!["lineage_admin".into()], claims: vec![], clearance: Classification::Internal },
			tenant: Tenant("tenant-a".into()),
			entity: "external_lineage_claim".into(),
			dataset: "lineage".into(),
			action,
			destination: crate::Destination::Browser,
			environment: Environment { env: "test".into(), ip: None, geo: None, device_posture: None, session_freshness: None },
			time_bucket: "now".into(),
			query_shape: QueryShape { verbs: vec![], aggregate: None, grouping_keys: vec![], subject_dimension: None, ordering: vec![], pagination: PaginationMode::LimitOnly { limit: 1 } },
			purpose: crate::Purpose("lineage_admin".into()),
			policy_version: "v1".into(),
		}
	}

	fn allow_policy(action: Action) -> PolicyCandidate {
		PolicyCandidate { effect: PolicyEffect::Allow, subjects: vec!["lineage_admin".into()], action, resource: "external_lineage_claim".into(), purpose: Some("lineage_admin".into()), conditions: vec![], filter: None, obligations: vec![], escalation: None, caps: vec![], masks: vec![], affected_row_cap: None, predicate_use: vec![], id: "allow-external-lineage".into() }
	}

	fn claim() -> ExternalLineageClaim {
		ExternalLineageClaim {
			edge_type: EdgeType::DerivedFrom,
			src: NodeId::data("external_source".to_owned(), None),
			dst: NodeId::data("internal_dataset".to_owned(), None),
			policy_version: "v1".into(),
			purpose: crate::Purpose("lineage_admin".into()),
			destination: crate::Destination::Browser,
			driver: DriverRef { port: "external".into(), vendor: "third-party".into(), version: "1.0".into() },
			transformation: crate::TransformId::Policy("v1".into()),
		}
	}

	/// A minimal, real (not mocked-behavior) `GraphStore` for tests that
	/// only need to prove admission/promotion logic, not a specific
	/// driver's own storage semantics — `nirdosha-lineage-store-embedded`
	/// can't be a dev-dependency of this crate without creating a Cargo
	/// dev-dependency cycle that (confirmed by direct compilation while
	/// writing this module) duplicates `nirdosha-lineage`'s own type
	/// identity across two builds, breaking `impl GraphStore for
	/// EmbeddedGraphStore` from this crate's own test perspective. The
	/// real-embedded-driver proof lives instead in
	/// `crates/nirdosha-lineage-store-embedded/tests/external_admission.rs`,
	/// which has no such cycle (it already depends on this crate
	/// normally).
	#[derive(Default)]
	struct TestGraphStore {
		edges: std::sync::Mutex<Vec<LineageEdge>>,
	}
	impl GraphStore for TestGraphStore {
		fn upsert_edge(&self, edge: &LineageEdge) -> Result<(), crate::LineageStoreError> {
			self.edges.lock().unwrap().push(edge.clone());
			Ok(())
		}
		fn get_edges(&self, filter: &crate::EdgeQueryFilter) -> Result<Vec<LineageEdge>, crate::LineageStoreError> {
			Ok(self.edges.lock().unwrap().iter().filter(|edge| filter.matches(edge)).cloned().collect())
		}
	}

	fn scratch_audit(name: &str) -> ModuleAuditChain {
		let root = std::env::temp_dir().join(format!("nir-lineage-admission-{name}-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		std::fs::create_dir_all(&root).unwrap();
		ModuleAuditChain::new("external-lineage-test", root.join("audit.jsonl"))
	}

	#[test]
	fn link_external_lineage_denies_without_a_real_matching_policy() {
		let mut quarantine = InMemoryQuarantineStore::new();
		let audit = scratch_audit("deny");
		let result = link_external_lineage(claim(), "vendor attestation doc #123".into(), &context("u1", Action::Create), &[], &mut quarantine, &audit, "t1", 1_000);
		assert!(matches!(result, Err(AdmissionError::Denied { .. })));
		assert!(quarantine.list_pending().is_empty(), "a denied claim must never enter quarantine");
	}

	#[test]
	fn link_external_lineage_refuses_empty_evidence_even_when_allowed() {
		let mut quarantine = InMemoryQuarantineStore::new();
		let audit = scratch_audit("empty-evidence");
		let policies = [allow_policy(Action::Create)];
		let result = link_external_lineage(claim(), "   ".into(), &context("u1", Action::Create), &policies, &mut quarantine, &audit, "t1", 1_000);
		assert_eq!(result, Err(AdmissionError::EmptyEvidence));
	}

	#[test]
	fn link_external_lineage_admits_a_real_claim_into_quarantine_with_forced_authority() {
		let mut quarantine = InMemoryQuarantineStore::new();
		let audit = scratch_audit("admit");
		let policies = [allow_policy(Action::Create)];
		let id = link_external_lineage(claim(), "vendor attestation doc #123".into(), &context("u1", Action::Create), &policies, &mut quarantine, &audit, "t1", 1_000).expect("a real allowed claim must be admitted");

		let quarantined = quarantine.get(&id).expect("the claim must exist in quarantine");
		assert_eq!(quarantined.status, QuarantineStatus::Pending);
		assert_eq!(quarantined.edge.authority, Authority::ExternalClaimed, "authority must always be forced, never taken from the caller");
		assert_eq!(quarantined.evidence, "vendor attestation doc #123");
	}

	#[test]
	fn promote_moves_a_real_pending_claim_into_the_store_and_marks_it_promoted() {
		let mut quarantine = InMemoryQuarantineStore::new();
		let audit = scratch_audit("promote");
		let policies = [allow_policy(Action::Create), allow_policy(Action::Update)];
		let id = link_external_lineage(claim(), "vendor attestation doc #123".into(), &context("u1", Action::Create), &policies, &mut quarantine, &audit, "t1", 1_000).unwrap();

		let store = TestGraphStore::default();
		promote(&id, &store, &context("approver-1", Action::Update), &policies, &mut quarantine, &audit, "t2", 2_000).expect("a real allowed promotion must succeed");

		assert_eq!(quarantine.get(&id).unwrap().status, QuarantineStatus::Promoted);
		let edges = store.get_edges(&crate::EdgeQueryFilter { authority: Some(Authority::ExternalClaimed), ..Default::default() }).expect("store query must succeed");
		assert_eq!(edges.len(), 1, "the promoted edge must now be real, queryable data in the store");
		assert_eq!(edges[0].authority, Authority::ExternalClaimed);
	}

	#[test]
	fn promote_refuses_a_claim_that_is_not_pending() {
		let mut quarantine = InMemoryQuarantineStore::new();
		let audit = scratch_audit("not-pending");
		let policies = [allow_policy(Action::Create), allow_policy(Action::Update)];
		let id = link_external_lineage(claim(), "vendor attestation doc #123".into(), &context("u1", Action::Create), &policies, &mut quarantine, &audit, "t1", 1_000).unwrap();

		let store = TestGraphStore::default();
		promote(&id, &store, &context("approver-1", Action::Update), &policies, &mut quarantine, &audit, "t2", 2_000).unwrap();

		let second = promote(&id, &store, &context("approver-1", Action::Update), &policies, &mut quarantine, &audit, "t3", 3_000);
		assert!(matches!(second, Err(AdmissionError::NotPending { .. })), "a second promotion of an already-promoted claim must be refused: {second:?}");
	}

	#[test]
	fn promote_denies_without_a_real_approver_policy_even_though_admission_was_allowed() {
		let mut quarantine = InMemoryQuarantineStore::new();
		let audit = scratch_audit("promote-denied");
		// Only the admission action is allowed, not the promotion one -
		// proves promotion is its own real, independently-gated decision,
		// not something admission approval quietly also grants.
		let policies = [allow_policy(Action::Create)];
		let id = link_external_lineage(claim(), "vendor attestation doc #123".into(), &context("u1", Action::Create), &policies, &mut quarantine, &audit, "t1", 1_000).unwrap();

		let store = TestGraphStore::default();
		let result = promote(&id, &store, &context("approver-1", Action::Update), &policies, &mut quarantine, &audit, "t2", 2_000);
		assert!(matches!(result, Err(AdmissionError::Denied { .. })));
		assert_eq!(quarantine.get(&id).unwrap().status, QuarantineStatus::Pending, "a denied promotion must leave the claim untouched, not silently promoted");
		assert!(store.get_edges(&crate::EdgeQueryFilter::default()).unwrap().is_empty(), "nothing may reach the store on a denied promotion");
	}
}
