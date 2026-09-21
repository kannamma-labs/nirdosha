//! Plan Phase 19's real-driver proof: `nirdosha_lineage::external_admission::promote`
//! against the real, working `EmbeddedGraphStore` — not the minimal
//! mock `nirdosha-lineage`'s own test module uses (a dev-dependency on
//! this crate would create a Cargo dev-dependency cycle that duplicates
//! `nirdosha-lineage`'s own type identity, confirmed by direct
//! compilation while building Phase 19 — see
//! `nirdosha-lineage/src/external_admission.rs`'s test module doc
//! comment). This crate already depends on `nirdosha-lineage` normally,
//! so no such cycle exists here.

use nirdosha_audit::envelope::ModuleAuditChain;
use nirdosha_guard_core::evaluator::{PolicyCandidate, PolicyEffect};
use nirdosha_guard_core::{Action, Classification, Environment, EvaluationContext, PaginationMode, QueryShape, Subject, Tenant};
use nirdosha_lineage::external_admission::{link_external_lineage, promote, AdmissionError, InMemoryQuarantineStore, QuarantineStatus, QuarantineStore, ExternalLineageClaim};
use nirdosha_lineage::{Authority, DriverRef, EdgeQueryFilter, EdgeType, GraphStore, NodeId};
use nirdosha_lineage_store_embedded::EmbeddedGraphStore;

fn context(subject_id: &str, action: Action) -> EvaluationContext {
	EvaluationContext {
		subject: Subject { id: subject_id.into(), roles: vec!["lineage_admin".into()], claims: vec![], clearance: Classification::Internal },
		tenant: Tenant("tenant-a".into()),
		entity: "external_lineage_claim".into(),
		dataset: "lineage".into(),
		action,
		destination: nirdosha_lineage::Destination::Browser,
		environment: Environment { env: "test".into(), ip: None, geo: None, device_posture: None, session_freshness: None },
		time_bucket: "now".into(),
		query_shape: QueryShape { verbs: vec![], aggregate: None, grouping_keys: vec![], subject_dimension: None, ordering: vec![], pagination: PaginationMode::LimitOnly { limit: 1 } },
		purpose: nirdosha_lineage::Purpose("lineage_admin".into()),
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
		purpose: nirdosha_lineage::Purpose("lineage_admin".into()),
		destination: nirdosha_lineage::Destination::Browser,
		driver: DriverRef { port: "external".into(), vendor: "third-party".into(), version: "1.0".into() },
		transformation: nirdosha_lineage::TransformId::Policy("v1".into()),
	}
}

fn scratch_audit(name: &str) -> ModuleAuditChain {
	let root = std::env::temp_dir().join(format!("nir-lineage-embedded-admission-{name}-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&root);
	std::fs::create_dir_all(&root).unwrap();
	ModuleAuditChain::new("external-lineage-embedded-test", root.join("audit.jsonl"))
}

#[test]
fn quarantined_claim_is_absent_from_the_real_embedded_store_until_promoted() {
	let mut quarantine = InMemoryQuarantineStore::new();
	let audit = scratch_audit("absent-until-promoted");
	let policies = [allow_policy(Action::Create), allow_policy(Action::Update)];
	let store = EmbeddedGraphStore::new();

	let id = link_external_lineage(claim(), "vendor attestation doc #123".into(), &context("u1", Action::Create), &policies, &mut quarantine, &audit, "t1", 1_000).expect("a real allowed claim must be admitted");

	// Structural exclusion: nothing was ever written to the real store —
	// this is the actual guarantee (Plan Phase 19), not a filter that has
	// to remember to apply.
	let edges_before = store.get_edges(&EdgeQueryFilter::default()).expect("real store query must succeed");
	assert!(edges_before.is_empty(), "a quarantined-but-not-yet-promoted claim must be structurally absent from the real store: {edges_before:?}");

	promote(&id, &store, &context("approver-1", Action::Update), &policies, &mut quarantine, &audit, "t2", 2_000).expect("a real allowed promotion must succeed");

	assert_eq!(quarantine.get(&id).unwrap().status, QuarantineStatus::Promoted);
	let edges_after = store.get_edges(&EdgeQueryFilter { authority: Some(Authority::ExternalClaimed), ..Default::default() }).expect("real store query must succeed");
	assert_eq!(edges_after.len(), 1, "the promoted edge must now be real, queryable data in the real embedded store");
	assert_eq!(edges_after[0].authority, Authority::ExternalClaimed);
	assert_eq!(edges_after[0].src, claim().src);
	assert_eq!(edges_after[0].dst, claim().dst);
}

#[test]
fn a_denied_promotion_never_reaches_the_real_embedded_store() {
	let mut quarantine = InMemoryQuarantineStore::new();
	let audit = scratch_audit("denied-promotion");
	// Only admission is allowed, not promotion.
	let policies = [allow_policy(Action::Create)];
	let store = EmbeddedGraphStore::new();

	let id = link_external_lineage(claim(), "vendor attestation doc #123".into(), &context("u1", Action::Create), &policies, &mut quarantine, &audit, "t1", 1_000).unwrap();
	let result = promote(&id, &store, &context("approver-1", Action::Update), &policies, &mut quarantine, &audit, "t2", 2_000);

	assert!(matches!(result, Err(AdmissionError::Denied { .. })));
	assert!(store.get_edges(&EdgeQueryFilter::default()).unwrap().is_empty(), "a denied promotion must never reach the real embedded store");
}
