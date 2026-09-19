//! Synchronous mutation-integrity controller for RFC 0023/0025.

pub mod exec;
pub mod shed;

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::Mutex;

use nirdosha_audit::envelope::{AuditEnvelope, AuditRecordKind, ModuleAuditChain};
use nirdosha_guard_core::decision_cache::DecisionCache;
use nirdosha_guard_core::evaluator::{self, EvaluationResult, PolicyCandidate};
use nirdosha_guard_core::{CapabilityManifest, Decision, DecisionCacheKey, EvaluationContext, FilterExpr, FilterNodeKind, LineageFacts};
use nirdosha_lineage::collector::{KernelCollector, ObservationContext, PlanFacts};
use nirdosha_lineage::{Authority, DriverRef, EdgeType, FlowCompleteness, TransformId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalRequest { pub context: EvaluationContext }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanIr { pub resource: String, pub dataset: String, pub filter: Option<FilterExpr>, pub policy_version: String }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared { pub resource: String, pub policy_version: String }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityBytes(pub Vec<u8>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt { pub store_commit_id: String, pub digest: [u8; 32] }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError { CapabilityUnsupported { required: String, attested: String }, Rejected(String), Store(String) }

pub trait StoreDriver: Send + Sync {
	fn manifest(&self) -> &CapabilityManifest;
	fn prepare(&self, ir: &PlanIr) -> Result<Prepared, PlanError>;
	fn commit(&self, prepared: Prepared, entity: EntityBytes) -> Result<Receipt, PlanError>;
	fn lineage(&self) -> LineageFacts { LineageFacts::default() }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome { Committed { trace_id: String }, Pending { expires_at: String }, Duplicate { trace_id: String } }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rejected { Denied { reason: String }, Escalated { target: String }, Failed { reason: String } }

/// In-memory idempotency tracker preventing double-execution of trace_ids.
#[derive(Debug, Default)]
pub struct IdempotencyStore {
	seen: Mutex<HashSet<String>>,
}

impl IdempotencyStore {
	pub fn new() -> Self { Self { seen: Mutex::new(HashSet::new()) } }
	pub fn insert_if_new(&self, trace_id: &str) -> bool {
		if let Ok(mut set) = self.seen.lock() {
			set.insert(trace_id.to_string())
		} else {
			false
		}
	}
}

pub struct GuardClient {
	policies: Vec<PolicyCandidate>,
	cache: DecisionCache,
	pub audit: ModuleAuditChain,
	pub idempotency: IdempotencyStore,
}

impl std::fmt::Debug for GuardClient {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { formatter.debug_struct("GuardClient").field("policies", &self.policies.len()).field("audit", &self.audit).finish() }
}

impl GuardClient {
	pub fn new(policies: Vec<PolicyCandidate>, module: impl Into<String>, audit_path: impl AsRef<Path>) -> Self {
		Self {
			policies,
			cache: DecisionCache::new(std::time::Duration::from_secs(30)),
			audit: ModuleAuditChain::new(module, audit_path.as_ref()),
			idempotency: IdempotencyStore::new(),
		}
	}

	pub fn evaluate(&mut self, request: &EvalRequest) -> EvaluationResult {
		let key = cache_key(&request.context);
		if let Some((decision, obligations)) = self.cache.get(&key) { return EvaluationResult { decision, obligations, residual_filter: None }; }
		let result = evaluator::evaluate(&request.context, &self.policies);
		if matches!(result.decision, Decision::Allow) { let _ = self.cache.insert(key, result.decision.clone(), result.obligations.clone(), request.context.subject.clearance.clone()); }
		result
	}

	pub fn guarded_apply<D: StoreDriver>(&mut self, request: &EvalRequest, driver: &D, payload: EntityBytes, trace_id: impl Into<String>, now_ms: u64) -> Result<Outcome, Rejected> {
		let trace_id = trace_id.into();
		if !self.idempotency.insert_if_new(&trace_id) {
			return Ok(Outcome::Duplicate { trace_id });
		}
		let evaluation = self.evaluate(request);
		let envelope = AuditEnvelope { trace_id: trace_id.clone(), ts: nirdosha_lineage::time::format_rfc3339_ms(now_ms), module: self.audit.module.clone(), subject: request.context.subject.id.clone(), action: format!("{:?}", request.context.action), resource: request.context.entity.clone(), policy_versions: vec![request.context.policy_version.clone()], decision: format!("{:?}", evaluation.decision), obligations: evaluation.obligations.iter().map(|item| format!("{item:?}")).collect(), kind: AuditRecordKind::Decision, content: serde_json::json!({ "phase": "before_commit" }) };
		match evaluation.decision {
			Decision::Deny { reason } => { self.audit.append(&envelope, now_ms); Err(Rejected::Denied { reason }) }
			Decision::Escalate { to } => { self.audit.append(&envelope, now_ms); Err(Rejected::Escalated { target: format!("{to:?}") }) }
			Decision::Pending { expires_at, .. } => { self.audit.append(&envelope, now_ms); Ok(Outcome::Pending { expires_at }) }
			Decision::Allow => {
				let plan = PlanIr { resource: request.context.entity.clone(), dataset: request.context.dataset.clone(), filter: evaluation.residual_filter, policy_version: request.context.policy_version.clone() };
				let prepared = driver.prepare(&plan).map_err(|error| Rejected::Failed { reason: format!("{error:?}") })?;
				self.audit.append(&envelope, now_ms);
				let receipt = driver.commit(prepared, payload).map_err(|error| Rejected::Failed { reason: format!("{error:?}") })?;
				let mut committed = envelope;
				committed.kind = AuditRecordKind::Mutation;
				committed.content = serde_json::json!({ "receipt": receipt.store_commit_id, "digest": receipt.digest });
				self.audit.append(&committed, now_ms);
				let collector = KernelCollector { config: nirdosha_lineage::collector::CollectorConfig { enabled: true } };
				if let Some(observation) = collector.observe(
					PlanFacts {
						edge_type: EdgeType::Read,
						transformation: TransformId::Policy(request.context.policy_version.clone()),
						driver: DriverRef { port: "store".into(), vendor: driver.manifest().driver_name.clone(), version: "0.1.0".into() },
						authority: Authority::KernelExecution,
						completeness: FlowCompleteness::Full,
						sampled: false,
						degraded: false,
						sink: nirdosha_guard_core::LineageEntity { entity: request.context.entity.clone(), keys: Vec::new() },
					},
					Some(&driver.lineage()),
					ObservationContext {
						module: self.audit.module.clone(),
						trace_id: trace_id.clone(),
						subject_id: request.context.subject.id.clone(),
						tenant_id: request.context.tenant.0.clone(),
						policy_version: request.context.policy_version.clone(),
						purpose: request.context.purpose.clone(),
						destination: request.context.destination.clone(),
						receipt_digest: receipt.digest,
					},
				) {
					let lineage = AuditEnvelope { trace_id: observation.trace_id.clone(), ts: nirdosha_lineage::time::format_rfc3339_ms(now_ms), module: self.audit.module.clone(), subject: observation.subject_id.clone(), action: format!("{:?}", request.context.action), resource: observation.sink.node.catalog_id.clone(), policy_versions: vec![observation.policy_version.clone()], decision: "allow".into(), obligations: vec![], kind: AuditRecordKind::Lineage, content: observation.to_audit_content() };
					self.audit.append(&lineage, now_ms);
				}
				Ok(Outcome::Committed { trace_id })
			}
		}
	}

	pub fn verify_audit(&self) -> Result<usize, nirdosha_audit::audit_chain::ChainError> { self.audit.verify() }
}

fn cache_key(context: &EvaluationContext) -> DecisionCacheKey {
	let mut hasher = DefaultHasher::new();
	context.subject.roles.hash(&mut hasher);
	context.environment.hash(&mut hasher);
	context.query_shape.hash(&mut hasher);
	DecisionCacheKey { subject_id: context.subject.id.clone(), roles_hash: hasher.finish(), tenant: context.tenant.clone(), entity: context.entity.clone(), dataset: context.dataset.clone(), action: context.action.clone(), environment_hash: 0, destination: context.destination.clone(), time_bucket: context.time_bucket.clone(), query_shape_hash: 0, purpose: context.purpose.clone(), policy_version: context.policy_version.clone() }
}

#[derive(Debug)]
pub struct MemStoreDriver { manifest: CapabilityManifest, rows: std::sync::Mutex<BTreeMap<String, Vec<u8>>>, next_commit: std::sync::atomic::AtomicU64 }

impl Default for MemStoreDriver { fn default() -> Self { Self::new() } }

impl MemStoreDriver {
	pub fn new() -> Self { Self { manifest: CapabilityManifest { schema_version: 1, driver_name: "memory".into(), supported_filter_nodes: vec![FilterNodeKind::Eq, FilterNodeKind::In, FilterNodeKind::Compare, FilterNodeKind::And, FilterNodeKind::Or, FilterNodeKind::Not, FilterNodeKind::TimeRange, FilterNodeKind::Pattern, FilterNodeKind::TenantEq], masking_points: vec![], aggregate_semantics: nirdosha_guard_core::AggregateSemantics::Inline, supports_tenant_eq_native: true }, rows: std::sync::Mutex::new(BTreeMap::new()), next_commit: std::sync::atomic::AtomicU64::new(0) } }
	pub fn get(&self, resource: &str) -> Option<Vec<u8>> { self.rows.lock().ok()?.get(resource).cloned() }
}

impl StoreDriver for MemStoreDriver {
	fn manifest(&self) -> &CapabilityManifest { &self.manifest }
	fn prepare(&self, ir: &PlanIr) -> Result<Prepared, PlanError> { Ok(Prepared { resource: ir.resource.clone(), policy_version: ir.policy_version.clone() }) }
	fn commit(&self, prepared: Prepared, entity: EntityBytes) -> Result<Receipt, PlanError> { self.rows.lock().map_err(|_| PlanError::Store("memory store poisoned".into()))?.insert(prepared.resource, entity.0); let id = self.next_commit.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1; Ok(Receipt { store_commit_id: format!("mem-{id}"), digest: [id as u8; 32] }) }
}

#[cfg(test)]
mod tests {
	use super::*;
	use nirdosha_guard_core::evaluator::PolicyEffect;
	use nirdosha_guard_core::{Action, Classification, Destination, Environment, PaginationMode, Purpose, QueryShape, Subject, Tenant};

	fn request() -> EvalRequest { EvalRequest { context: EvaluationContext { subject: Subject { id: "u".into(), roles: vec!["analyst".into()], claims: vec![], clearance: Classification::Internal }, tenant: Tenant("t".into()), entity: "orders".into(), dataset: "memory".into(), action: Action::Update, destination: Destination::Browser, environment: Environment { env: "test".into(), ip: None, geo: None, device_posture: None, session_freshness: None }, time_bucket: "now".into(), query_shape: QueryShape { verbs: vec![], aggregate: None, grouping_keys: vec![], subject_dimension: None, ordering: vec![], pagination: PaginationMode::LimitOnly { limit: 1 } }, purpose: Purpose("support".into()), policy_version: "v1".into() } } }
	fn policy() -> PolicyCandidate { PolicyCandidate { effect: PolicyEffect::Allow, subjects: vec!["analyst".into()], action: Action::Update, resource: "orders".into(), purpose: Some("support".into()), conditions: vec![], filter: None, obligations: vec![], escalation: None, id: "update-orders".into() } }

	#[test]
	fn guarded_apply_commits_only_after_audit_decision() {
		let root = std::env::temp_dir().join(format!("nirdosha-mic-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		let mut client = GuardClient::new(vec![policy()], "test", root.join("audit.jsonl"));
		let driver = MemStoreDriver::new();
		let result = client.guarded_apply(&request(), &driver, EntityBytes(b"row".to_vec()), "trace-1", 1_000).unwrap();
		assert!(matches!(result, Outcome::Committed { .. }));
		assert_eq!(driver.get("orders"), Some(b"row".to_vec()));
		assert_eq!(client.verify_audit(), Ok(3));

		// Idempotency duplicate check
		let dup_result = client.guarded_apply(&request(), &driver, EntityBytes(b"row".to_vec()), "trace-1", 1_001).unwrap();
		assert!(matches!(dup_result, Outcome::Duplicate { .. }));

		let _ = std::fs::remove_dir_all(root);
	}
}

