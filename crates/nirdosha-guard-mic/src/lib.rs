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
use nirdosha_guard_core::{Cap, CapabilityManifest, Decision, DecisionCacheKey, EvaluationContext, FieldMask, FilterExpr, FilterNodeKind, LineageFacts, PaginationMode, Value, WriteAction};
use nirdosha_lineage::collector::{KernelCollector, ObservationContext, PlanFacts};
use nirdosha_lineage::{Authority, DriverRef, EdgeType, FlowCompleteness, TransformId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalRequest { pub context: EvaluationContext }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanIr {
	pub resource: String,
	pub dataset: String,
	/// The tenant/scope this write's *new* value must carry (bound into
	/// the row on write — e.g. the RLS session var an INSERT/UPDATE runs
	/// under).
	pub filter: Option<FilterExpr>,
	/// A precondition on the *pre-existing* row (`Update`/`Delete` only):
	/// the current row must match this before the write is allowed to
	/// proceed. Distinct from `filter` — a policy could in principle scope
	/// what a write is allowed to *become* differently from what it
	/// requires the row to already *be* — but in the corpus and both
	/// existing drivers, both come from the same matching policy's
	/// `filter tenant_scope()`, so this is currently always `== filter`.
	/// A real schema-level precondition (`requires field(status) ==
	/// "pending"`) is out of scope here: `EntityBytes` is opaque, so
	/// checking arbitrary entity-internal fields needs a typed schema
	/// this layer doesn't have (same limitation `ReadOutcome::masks`
	/// documents on the read side).
	pub row_scope: Option<FilterExpr>,
	pub action: WriteAction,
	pub affected_row_cap: Option<u64>,
	pub policy_version: String,
}

/// The read-side counterpart to `PlanIr`. Separate from it (rather than
/// widening `PlanIr` itself) because reads carry `caps`/`masks` that a
/// write's `prepare`/`commit` never needed and, before this phase,
/// evaluation itself never produced (`PolicyCandidate`/`EvaluationResult`
/// had no `caps`/`masks` fields at all — added alongside this).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadPlanIr {
	pub resource: String,
	pub dataset: String,
	pub filter: Option<FilterExpr>,
	pub caps: Vec<Cap>,
	/// From the *request's* `query_shape.pagination`, not the policy —
	/// pagination is a property of how the caller wants results shaped,
	/// same as `AccessPlan`'s own design keeps row-level/request-shape
	/// facts out of policy grants. `PaginationMode::Rejected` never
	/// reaches a driver: `guarded_read` refuses the request before
	/// building a plan at all (see its own body).
	pub pagination: PaginationMode,
	pub policy_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
	pub resource: String,
	pub policy_version: String,
	pub action: WriteAction,
	/// Threaded from `PlanIr::row_scope` to `commit()` — the
	/// existence/row_scope check happens *inside* `commit()`, not here in
	/// `prepare()`, so it's checked atomically with the write itself
	/// (same lock/transaction). Splitting "check" (prepare) from "act"
	/// (commit) across two separate trait calls with no shared lock
	/// between them would be a TOCTOU race: another commit could land in
	/// the gap. `prepare()` stays validation-only (filter translatable,
	/// tenant extractable) — see each driver's `prepare()` for what it
	/// still does.
	pub row_scope: Option<FilterExpr>,
}

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
	/// Executes a read plan and returns the matching rows, capped per
	/// `plan.caps` (`RowCap`, `MaxScanRows`, `MaxScanBytes`,
	/// `MaxExecutionTimeMs`, `MaxResultBytes`, `CohortFloor` — a driver
	/// honors what its own `CapabilityManifest` attests to supporting;
	/// see each implementor for which are real pushdown vs. an L1
	/// post-read enforcement). Rows are returned as opaque `EntityBytes`,
	/// same as the write side — this layer doesn't know a concrete
	/// entity's field shape (see `GuardClient::guarded_read`'s doc
	/// comment on why field masking therefore can't happen here either).
	fn query(&self, plan: &ReadPlanIr) -> Result<QueryResult, PlanError>;
	fn lineage(&self) -> LineageFacts { LineageFacts::default() }
}

/// What `StoreDriver::query` returns: the matching rows plus an opaque
/// continuation token if there are more (`PaginationMode::OpaqueCursor`
/// support — see `ReadOutcome::next_cursor`'s doc comment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryResult { pub rows: Vec<EntityBytes>, pub next_cursor: Option<String> }

/// A successful `guarded_read`: the rows the driver returned, plus the
/// `FieldMask`s the matching policy granted. Masks are *not* applied to
/// `rows` here — `EntityBytes` is opaque (`Vec<u8>`, no assumed schema,
/// same as the write side's `EntityBytes` payload), so this layer has no
/// way to locate a `FieldMask.field` path inside it. The caller, which
/// deserializes `rows` into its own concrete entity type, is the only
/// party that can actually redact a field — `masks` is what it must apply
/// before the result reaches its `destination`. This is not a weaker
/// contract than the write side's: `guarded_apply` has the identical
/// limitation for `FieldPolicy` today (evaluation doesn't enforce it
/// against payload bytes either — there's no `field_policy` on
/// `PolicyCandidate`/`EvaluationResult` at all yet).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadOutcome {
	pub rows: Vec<EntityBytes>,
	pub masks: Vec<FieldMask>,
	/// Opaque continuation token for `PaginationMode::OpaqueCursor` — pass
	/// it back as the next request's cursor to resume. `None` means there
	/// is no further page. Never a raw offset (I10/§8.4: "opaque cursors
	/// only; offset rejected") — each driver defines its own token shape
	/// and nothing outside that driver is meant to parse it.
	pub next_cursor: Option<String>,
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
	/// `trace_id`s that have passed `dry_run()` — `WriteAction::Migrate`'s
	/// mandatory-dry-run-before-apply gate (RFC 0023 §2). Single-use:
	/// `guarded_apply` removes the entry on the matching apply, the same
	/// "consume once" shape `idempotency` has for trace replay.
	dry_runs: HashSet<String>,
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
			dry_runs: HashSet::new(),
		}
	}

	/// Records a dry run for `trace_id` — `guarded_apply` requires exactly
	/// this before it will accept a `Migrate` action under the same
	/// `trace_id`. Evaluates the request for real (a dry run that skips
	/// authorization would prove nothing) but never touches a driver.
	pub fn dry_run(&mut self, request: &EvalRequest, trace_id: impl Into<String>, now_ms: u64) -> Result<(), Rejected> {
		let trace_id = trace_id.into();
		let evaluation = self.evaluate(request);
		let envelope = AuditEnvelope { trace_id: trace_id.clone(), ts: nirdosha_lineage::time::format_rfc3339_ms(now_ms), module: self.audit.module.clone(), subject: request.context.subject.id.clone(), action: format!("{:?}", request.context.action), resource: request.context.entity.clone(), policy_versions: vec![request.context.policy_version.clone()], decision: format!("{:?}", evaluation.decision), obligations: evaluation.obligations.iter().map(|item| format!("{item:?}")).collect(), kind: AuditRecordKind::Decision, content: serde_json::json!({ "phase": "dry_run" }) };
		self.audit.append(&envelope, now_ms);
		match evaluation.decision {
			Decision::Deny { reason } => Err(Rejected::Denied { reason }),
			Decision::Escalate { to } => Err(Rejected::Escalated { target: format!("{to:?}") }),
			Decision::Pending { .. } => Err(Rejected::Failed { reason: "dry run cannot be pending".into() }),
			Decision::Allow => { self.dry_runs.insert(trace_id); Ok(()) }
		}
	}

	pub fn evaluate(&mut self, request: &EvalRequest) -> EvaluationResult {
		let key = cache_key(&request.context);
		if let Some(cached) = self.cache.get(&key) {
			return EvaluationResult { decision: cached.decision, obligations: cached.obligations, residual_filter: cached.residual_filter, caps: cached.caps, masks: cached.masks, affected_row_cap: cached.affected_row_cap, predicate_use: cached.predicate_use };
		}
		let result = evaluator::evaluate(&request.context, &self.policies);
		if matches!(result.decision, Decision::Allow) {
			let value = nirdosha_guard_core::decision_cache::CacheableDecision {
				decision: result.decision.clone(),
				obligations: result.obligations.clone(),
				residual_filter: result.residual_filter.clone(),
				caps: result.caps.clone(),
				masks: result.masks.clone(),
				affected_row_cap: result.affected_row_cap,
				predicate_use: result.predicate_use.clone(),
			};
			let _ = self.cache.insert(key, value, request.context.subject.clearance.clone());
		}
		result
	}

	pub fn guarded_apply<D: StoreDriver>(&mut self, request: &EvalRequest, driver: &D, payload: EntityBytes, trace_id: impl Into<String>, now_ms: u64) -> Result<Outcome, Rejected> {
		let trace_id = trace_id.into();
		// Idempotency first, before any other gate: a replayed trace_id
		// means "this exact call already happened," which should
		// short-circuit ahead of business-logic checks like the migrate
		// dry-run gate below — otherwise a legitimate replay of an
		// already-applied migrate would see its (already-consumed) dry
		// run missing and get a confusing "no dry run" rejection instead
		// of the correct "this already happened" signal.
		if !self.idempotency.insert_if_new(&trace_id) {
			return Ok(Outcome::Duplicate { trace_id });
		}
		let Some(write_action) = request.context.action.as_write_action() else {
			return Err(Rejected::Failed { reason: format!("{:?} is not a write action — guarded_apply only accepts Create/Update/Delete/Migrate", request.context.action) });
		};
		if write_action == WriteAction::Export {
			// Export has no new value to write — it's egress over
			// *existing* data, not a mutation. Route through
			// `guarded_read`, which forces full audit for it (RFC 0023
			// §2: egress is never sampled) rather than reusing this
			// upsert-shaped path for a fundamentally different action.
			return Err(Rejected::Failed { reason: "export is not a write — call guarded_read with Action::Export instead".into() });
		}
		if write_action == WriteAction::Migrate && !self.dry_runs.remove(&trace_id) {
			return Err(Rejected::Failed { reason: "migrate requires a prior dry_run() under the same trace_id (RFC 0023 §2: dry-run mandatory before apply)".into() });
		}
		let evaluation = self.evaluate(request);
		let envelope = AuditEnvelope { trace_id: trace_id.clone(), ts: nirdosha_lineage::time::format_rfc3339_ms(now_ms), module: self.audit.module.clone(), subject: request.context.subject.id.clone(), action: format!("{:?}", request.context.action), resource: request.context.entity.clone(), policy_versions: vec![request.context.policy_version.clone()], decision: format!("{:?}", evaluation.decision), obligations: evaluation.obligations.iter().map(|item| format!("{item:?}")).collect(), kind: AuditRecordKind::Decision, content: serde_json::json!({ "phase": "before_commit" }) };
		match evaluation.decision {
			Decision::Deny { reason } => { self.audit.append(&envelope, now_ms); Err(Rejected::Denied { reason }) }
			Decision::Escalate { to } => { self.audit.append(&envelope, now_ms); Err(Rejected::Escalated { target: format!("{to:?}") }) }
			Decision::Pending { expires_at, .. } => { self.audit.append(&envelope, now_ms); Ok(Outcome::Pending { expires_at }) }
			Decision::Allow => {
				let plan = PlanIr {
					resource: request.context.entity.clone(),
					dataset: request.context.dataset.clone(),
					row_scope: evaluation.residual_filter.clone(),
					filter: evaluation.residual_filter,
					action: write_action,
					affected_row_cap: evaluation.affected_row_cap,
					policy_version: request.context.policy_version.clone(),
				};
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

	/// The read-side counterpart to `guarded_apply`, same shape: evaluate
	/// → audit the decision → on Allow, build a plan and hand it to the
	/// driver → audit again → emit a lineage observation (`EdgeType::Read`,
	/// matching what an actual read is, rather than `guarded_apply`'s
	/// always-`Read` edge type, which was written for the write path's
	/// own pre-image-read framing and is left as-is there).
	///
	/// No idempotency check here (unlike `guarded_apply`) — a read has no
	/// side effect to double-apply; replaying one is harmless by
	/// construction, so `trace_id` is only for audit correlation.
	pub fn guarded_read<D: StoreDriver>(&mut self, request: &EvalRequest, driver: &D, trace_id: impl Into<String>, now_ms: u64) -> Result<ReadOutcome, Rejected> {
		let trace_id = trace_id.into();
		// I10/§8.4: "opaque cursors only; offset rejected." A request
		// whose pagination was already classified `Rejected` (upstream of
		// this call — whatever built the EvaluationContext decided the
		// caller asked for raw offset pagination) is refused before
		// policies are even evaluated: this is a request-shape violation,
		// not a decision `evaluate()` should be asked to make.
		if matches!(request.context.query_shape.pagination, PaginationMode::Rejected) {
			return Err(Rejected::Failed { reason: "offset pagination is rejected — use an opaque cursor (I10/§8.4)".into() });
		}
		let evaluation = self.evaluate(request);
		let envelope = AuditEnvelope { trace_id: trace_id.clone(), ts: nirdosha_lineage::time::format_rfc3339_ms(now_ms), module: self.audit.module.clone(), subject: request.context.subject.id.clone(), action: format!("{:?}", request.context.action), resource: request.context.entity.clone(), policy_versions: vec![request.context.policy_version.clone()], decision: format!("{:?}", evaluation.decision), obligations: evaluation.obligations.iter().map(|item| format!("{item:?}")).collect(), kind: AuditRecordKind::Decision, content: serde_json::json!({ "phase": "before_read" }) };
		match evaluation.decision {
			Decision::Deny { reason } => { self.audit.append(&envelope, now_ms); Err(Rejected::Denied { reason }) }
			Decision::Escalate { to } => { self.audit.append(&envelope, now_ms); Err(Rejected::Escalated { target: format!("{to:?}") }) }
			Decision::Pending { .. } => { self.audit.append(&envelope, now_ms); Err(Rejected::Failed { reason: "read cannot be pending — escalation/approval applies to writes, not reads".into() }) }
			Decision::Allow => {
				// I15: a masked field may not drive a non-projection
				// clause (filter/join/grouping/having/ordering/window)
				// unless explicitly granted via `predicate_use`. Checked
				// here, at plan-build time, before the driver ever sees
				// the filter — a masked field silently leaking through a
				// WHERE clause is exactly the class of bug a guard exists
				// to catch before it reaches a query planner.
				if let Some(filter) = &evaluation.residual_filter {
					let masked_fields: std::collections::HashSet<&nirdosha_guard_core::FieldPath> =
						evaluation.masks.iter().map(|mask| &mask.field).collect();
					let granted: std::collections::HashSet<&str> =
						evaluation.predicate_use.iter().map(String::as_str).collect();
					for field in nirdosha_guard_core::filter_fields(filter) {
						let is_masked = masked_fields.contains(field);
						let is_granted = field.len() == 1 && granted.contains(field[0].as_str());
						if is_masked && !is_granted {
							self.audit.append(&envelope, now_ms);
							return Err(Rejected::Failed { reason: format!("I15: field {field:?} is masked and not granted via predicate_use — cannot drive a filter clause") });
						}
					}
				}
				let plan = ReadPlanIr { resource: request.context.entity.clone(), dataset: request.context.dataset.clone(), filter: evaluation.residual_filter, caps: evaluation.caps, pagination: request.context.query_shape.pagination.clone(), policy_version: request.context.policy_version.clone() };
				let query_result = driver.query(&plan).map_err(|error| Rejected::Failed { reason: format!("{error:?}") })?;
				let rows = query_result.rows;
				let mut read_envelope = envelope.clone();
				self.audit.append(&envelope, now_ms);
				read_envelope.kind = AuditRecordKind::Mutation; // reuses the "effect happened" kind; no separate Read kind exists yet
				read_envelope.content = serde_json::json!({ "rows_returned": rows.len() });
				self.audit.append(&read_envelope, now_ms);
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
						receipt_digest: [0u8; 32],
					},
				) {
					let lineage = AuditEnvelope { trace_id: observation.trace_id.clone(), ts: nirdosha_lineage::time::format_rfc3339_ms(now_ms), module: self.audit.module.clone(), subject: observation.subject_id.clone(), action: format!("{:?}", request.context.action), resource: observation.sink.node.catalog_id.clone(), policy_versions: vec![observation.policy_version.clone()], decision: "allow".into(), obligations: vec![], kind: AuditRecordKind::Lineage, content: observation.to_audit_content() };
					self.audit.append(&lineage, now_ms);
				}
				Ok(ReadOutcome { rows, masks: evaluation.masks, next_cursor: query_result.next_cursor })
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

/// One stored row: the resource key doubles as its only "column" besides
/// tenant (this driver's whole schema is `resource -> (tenant, payload)`,
/// matching `PostgresStoreDriver`'s real `guard_entities(resource, tenant,
/// policy_version, payload)` table shape one-for-one).
#[derive(Debug, Clone)]
struct MemRow { tenant: Option<String>, payload: Vec<u8> }

#[derive(Debug)]
pub struct MemStoreDriver {
	manifest: CapabilityManifest,
	rows: std::sync::Mutex<BTreeMap<String, MemRow>>,
	next_commit: std::sync::atomic::AtomicU64,
	/// Tenant extracted at `prepare()` time, consumed by `commit()` — the
	/// same `Prepared{resource, policy_version}`-has-no-room-for-it
	/// constraint `PostgresStoreDriver` documents on its own identical
	/// field.
	pending_tenants: std::sync::Mutex<std::collections::HashMap<String, String>>,
}

impl Default for MemStoreDriver { fn default() -> Self { Self::new() } }

impl MemStoreDriver {
	pub fn new() -> Self { Self { manifest: CapabilityManifest { schema_version: 1, driver_name: "memory".into(), supported_filter_nodes: vec![FilterNodeKind::Eq, FilterNodeKind::In, FilterNodeKind::Compare, FilterNodeKind::And, FilterNodeKind::Or, FilterNodeKind::Not, FilterNodeKind::TimeRange, FilterNodeKind::Pattern, FilterNodeKind::TenantEq], masking_points: vec![], aggregate_semantics: nirdosha_guard_core::AggregateSemantics::Inline, supports_tenant_eq_native: true }, rows: std::sync::Mutex::new(BTreeMap::new()), next_commit: std::sync::atomic::AtomicU64::new(0), pending_tenants: std::sync::Mutex::new(std::collections::HashMap::new()) } }
	pub fn get(&self, resource: &str) -> Option<Vec<u8>> { self.rows.lock().ok()?.get(resource).map(|row| row.payload.clone()) }
}

impl StoreDriver for MemStoreDriver {
	fn manifest(&self) -> &CapabilityManifest { &self.manifest }
	fn prepare(&self, ir: &PlanIr) -> Result<Prepared, PlanError> {
		// Validation only — see `Prepared::row_scope`'s doc comment on why
		// the actual existence/row_scope check happens inside `commit()`,
		// not here.
		if let Some(filter) = &ir.filter {
			if let Some(tenant) = nirdosha_guard_core::extract_tenant(filter) {
				let mut pending = self.pending_tenants.lock().map_err(|_| PlanError::Store("pending-tenant lock poisoned".into()))?;
				pending.insert(ir.resource.clone(), tenant);
			}
		}
		Ok(Prepared { resource: ir.resource.clone(), policy_version: ir.policy_version.clone(), action: ir.action.clone(), row_scope: ir.row_scope.clone() })
	}
	fn commit(&self, prepared: Prepared, entity: EntityBytes) -> Result<Receipt, PlanError> {
		let tenant = self.pending_tenants.lock().map_err(|_| PlanError::Store("pending-tenant lock poisoned".into()))?.remove(&prepared.resource);
		// One lock held across the existence/row_scope check AND the
		// write itself — the atomicity contract (Plan Phase 8): no other
		// commit can land in a gap between "checked" and "wrote" the way
		// it could if this were split across `prepare()`/`commit()`.
		let mut rows = self.rows.lock().map_err(|_| PlanError::Store("memory store poisoned".into()))?;
		match prepared.action {
			WriteAction::Create => {
				if rows.contains_key(&prepared.resource) {
					return Err(PlanError::Rejected(format!("create: resource {:?} already exists", prepared.resource)));
				}
			}
			WriteAction::Update | WriteAction::Delete => {
				let Some(existing) = rows.get(&prepared.resource) else {
					return Err(PlanError::Rejected(format!("{:?}: resource {:?} does not exist", prepared.action, prepared.resource)));
				};
				if let Some(row_scope) = &prepared.row_scope {
					if let Some(required_tenant) = nirdosha_guard_core::extract_tenant(row_scope) {
						if existing.tenant.as_deref() != Some(required_tenant.as_str()) {
							return Err(PlanError::Rejected("row_scope: existing row's tenant does not match".into()));
						}
					}
				}
			}
			WriteAction::Migrate | WriteAction::Export => {}
		}
		if matches!(prepared.action, WriteAction::Delete) {
			rows.remove(&prepared.resource);
		} else {
			rows.insert(prepared.resource.clone(), MemRow { tenant, payload: entity.0 });
		}
		let id = self.next_commit.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
		Ok(Receipt { store_commit_id: format!("mem-{id}"), digest: [id as u8; 32] })
	}
	fn query(&self, plan: &ReadPlanIr) -> Result<QueryResult, PlanError> {
		let rows = self.rows.lock().map_err(|_| PlanError::Store("memory store poisoned".into()))?;
		let filter = plan
			.filter
			.as_ref()
			// No filter at all: RFC 0023's own posture (mirrored by
			// PostgresStoreDriver::prepare's write-side check) is that an
			// ungoverned scan is a bug to surface, not a silent
			// full-table return.
			.ok_or_else(|| PlanError::Rejected("read plan has no filter — refusing an unscoped scan".into()))?;

		let row_cap = plan.caps.iter().find_map(|cap| match cap { Cap::RowCap(n) => Some(*n as usize), _ => None });
		let max_scan_rows = plan.caps.iter().find_map(|cap| match cap { Cap::MaxScanRows(n) => Some(*n as usize), _ => None });
		let max_scan_bytes = plan.caps.iter().find_map(|cap| match cap { Cap::MaxScanBytes(n) => Some(*n as usize), _ => None });
		let max_result_bytes = plan.caps.iter().find_map(|cap| match cap { Cap::MaxResultBytes(n) => Some(*n as usize), _ => None });
        let max_execution = plan.caps.iter().find_map(|cap| match cap { Cap::MaxExecutionTimeMs(n) => Some(*n), _ => None });
		let cohort_floor = plan.caps.iter().find_map(|cap| match cap { Cap::CohortFloor(n) => Some(*n as usize), _ => None });

		let start_after = match &plan.pagination {
			PaginationMode::OpaqueCursor(token) => Some(decode_cursor(token)?),
			_ => None,
		};

		// CohortFloor (k-anonymity): count matches first — a query that
		// would identify fewer than `cohort_floor` entities is refused
		// outright, not silently truncated or padded. Bounded by the same
		// scan caps as the real pass below, so counting can't itself
		// become an unbounded scan.
		if let Some(floor) = cohort_floor {
			let count = scan(&rows, filter, start_after.as_deref(), max_scan_rows, max_scan_bytes, max_execution, |_, _| true).0.len();
			if count < floor {
				return Err(PlanError::Rejected(format!("cohort_floor: query would identify {count} entities, below the floor of {floor}")));
			}
		}

		let (matched, scanned_past_cap, last_key) = scan(&rows, filter, start_after.as_deref(), max_scan_rows, max_scan_bytes, max_execution, move |matched_so_far, _| {
			row_cap.map(|cap| matched_so_far < cap).unwrap_or(true)
		});

		let mut result_bytes = 0usize;
		let mut out = Vec::new();
		for (_, payload) in &matched {
			result_bytes += payload.len();
			if max_result_bytes.is_some_and(|cap| result_bytes > cap) {
				break;
			}
			out.push(EntityBytes(payload.clone()));
		}

		// A cursor is only worth returning if the driver actually stopped
		// short of the end of the store (a cap truncated it, or a row_cap
		// stopped collection) — otherwise there's nothing left to resume.
		let next_cursor = if scanned_past_cap || out.len() < matched.len() {
			last_key.map(|key| encode_cursor(&key))
		} else {
			None
		};

		Ok(QueryResult { rows: out, next_cursor })
	}
}

/// Base64-encodes a resource key as an opaque cursor token. `pub` (not
/// crate-only) so every `StoreDriver` implementor uses the same scheme —
/// `PostgresStoreDriver::query` reuses this rather than growing its own
/// encoding that would decode differently for a cursor produced by a
/// different driver.
pub fn encode_cursor(resource: &str) -> String {
	use base64::Engine;
	base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(resource.as_bytes())
}

pub fn decode_cursor(token: &str) -> Result<String, PlanError> {
	use base64::Engine;
	let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
		.decode(token)
		.map_err(|_| PlanError::Rejected("malformed opaque cursor".into()))?;
	String::from_utf8(bytes).map_err(|_| PlanError::Rejected("malformed opaque cursor".into()))
}

/// Shared scan loop: walks `rows` in key order (skipping past `start_after`
/// for cursor resumption), applying `filter`, `max_scan_rows`,
/// `max_scan_bytes`, and a wall-clock `max_execution_ms` deadline, and
/// calling `keep_going(matches_so_far, resource)` to decide whether to
/// keep collecting (this is how `RowCap` and the cohort-floor counting
/// pass share this one loop despite wanting different stop conditions).
/// Returns the matched `(resource, payload)` pairs, whether the scan
/// stopped early because of a cap (vs. reaching the end of the store),
/// and the last resource key examined (for the next cursor).
#[allow(clippy::type_complexity)]
fn scan(
	rows: &BTreeMap<String, MemRow>,
	filter: &FilterExpr,
	start_after: Option<&str>,
	max_scan_rows: Option<usize>,
	max_scan_bytes: Option<usize>,
	max_execution_ms: Option<u64>,
	keep_going: impl Fn(usize, &str) -> bool,
) -> (Vec<(String, Vec<u8>)>, bool, Option<String>) {
	let started_at = std::time::Instant::now();
	let iter: Box<dyn Iterator<Item = (&String, &MemRow)>> = match start_after {
		Some(cursor) => Box::new(rows.range((std::ops::Bound::Excluded(cursor.to_string()), std::ops::Bound::Unbounded))),
		None => Box::new(rows.iter()),
	};
	let mut matched = Vec::new();
	let mut scanned_bytes = 0usize;
	let mut last_examined_key = None;
	let mut stopped_early = false;
	// Resume point on an early stop — set explicitly at whichever break
	// fires, because the right value depends on *why* the scan stopped:
	// a scan cap (rows/bytes/time) means we genuinely didn't look past
	// this row, so the next page resumes right after it
	// (`last_examined_key`, i.e. the row that triggered the cap, updated
	// below only on the paths that reach it). A `RowCap` stop (via
	// `keep_going`) means we peeked at a row that *matched* but excluded
	// it purely because we already had enough results — the next page
	// must resume after the last row actually *returned*, or it would
	// silently skip the excluded one. Using the "last examined" row for
	// both was the bug the opaque-cursor test caught: a RowCap-triggered
	// stop advanced the cursor one matching row too far, past a row that
	// was never returned.
	let mut resume_key = None;
	for (index, (resource, row)) in iter.enumerate() {
		if max_scan_rows.is_some_and(|cap| index >= cap) {
			stopped_early = true;
			resume_key = last_examined_key.clone();
			break;
		}
		if max_execution_ms.is_some_and(|cap| started_at.elapsed().as_millis() as u64 >= cap) {
			stopped_early = true;
			resume_key = last_examined_key.clone();
			break;
		}
		scanned_bytes += row.payload.len();
		if max_scan_bytes.is_some_and(|cap| scanned_bytes > cap) {
			stopped_early = true;
			resume_key = last_examined_key.clone();
			break;
		}
		let candidate = MemRowView { resource, tenant: row.tenant.as_deref() };
		if match_filter(filter, &candidate) {
			if !keep_going(matched.len(), resource) {
				stopped_early = true;
				resume_key = matched.last().map(|(key, _): &(String, Vec<u8>)| key.clone());
				break;
			}
			matched.push((resource.clone(), row.payload.clone()));
		}
		last_examined_key = Some(resource.clone());
	}
	(matched, stopped_early, resume_key)
}

/// The only two "columns" `MemStoreDriver` actually has to filter against
/// — see `MemRow`'s doc comment on why that's an honest reflection of its
/// schema, not an arbitrary restriction.
struct MemRowView<'a> { resource: &'a str, tenant: Option<&'a str> }

/// A small, direct `FilterExpr` evaluator against `MemRowView` — the
/// in-memory driver's equivalent of what `drivers::rdbms::RdbmsEmitter`
/// does for SQL (translate the same closed IR into an executable check),
/// just interpreted instead of compiled to a query string, since there's
/// no query engine underneath an in-memory `BTreeMap` to compile *to*.
/// `resource` is matched under `field: ["resource"]` — the only row
/// identity this driver's flat schema has; any other field name never
/// matches (honest, not silently permissive: a filter naming a column
/// this driver doesn't have excludes every row rather than including all
/// of them).
fn match_filter(expr: &FilterExpr, row: &MemRowView) -> bool {
	match expr {
		FilterExpr::TenantEq { value: Value::Str(tenant) } => row.tenant == Some(tenant.as_str()),
		FilterExpr::TenantEq { .. } => false,
		FilterExpr::Eq { field, value } => field_str(row, field).is_some_and(|actual| value_eq_str(value, actual)),
		FilterExpr::In { field, values } => field_str(row, field).is_some_and(|actual| values.iter().any(|value| value_eq_str(value, actual))),
		FilterExpr::Pattern { field, matcher } => field_str(row, field).is_some_and(|actual| pattern_matches(matcher, actual)),
		FilterExpr::And(children) => children.iter().all(|child| match_filter(child, row)),
		FilterExpr::Or(children) => children.iter().any(|child| match_filter(child, row)),
		FilterExpr::Not(inner) => !match_filter(inner, row),
		// Compare/TimeRange/RelationIn need a typed or resolved value this
		// driver's two string-only columns can't express — excluded
		// rather than guessed at (same "honest, not permissive" rule as
		// an unknown field name).
		FilterExpr::Compare { .. } | FilterExpr::TimeRange { .. } | FilterExpr::RelationIn { .. } => false,
	}
}

fn field_str<'a>(row: &MemRowView<'a>, field: &[String]) -> Option<&'a str> {
	match field {
		[name] if name == "resource" => Some(row.resource),
		[name] if name == "tenant" => row.tenant,
		_ => None,
	}
}

fn value_eq_str(value: &Value, actual: &str) -> bool {
	matches!(value, Value::Str(expected) if expected == actual)
}

fn pattern_matches(matcher: &nirdosha_guard_core::PatternMatcher, actual: &str) -> bool {
	use nirdosha_guard_core::PatternMatcher;
	match matcher {
		PatternMatcher::Exact(value) => value == actual,
		PatternMatcher::Prefix(value) => actual.starts_with(value.as_str()),
		PatternMatcher::Glob(pattern) => glob_match(pattern, actual),
	}
}

/// Minimal `*`/`?` glob matcher — no external dependency for two wildcard
/// characters. Anchored (the whole string must match, not a substring).
fn glob_match(pattern: &str, text: &str) -> bool {
	fn go(p: &[u8], t: &[u8]) -> bool {
		match (p.first(), t.first()) {
			(None, None) => true,
			(Some(b'*'), _) => go(&p[1..], t) || (!t.is_empty() && go(p, &t[1..])),
			(Some(b'?'), Some(_)) => go(&p[1..], &t[1..]),
			(Some(pc), Some(tc)) if pc == tc => go(&p[1..], &t[1..]),
			_ => false,
		}
	}
	go(pattern.as_bytes(), text.as_bytes())
}

#[cfg(test)]
mod tests {
	use super::*;
	use nirdosha_guard_core::evaluator::PolicyEffect;
	use nirdosha_guard_core::{Action, Classification, Destination, Environment, MaskTransform, PaginationMode, Purpose, QueryShape, Subject, Tenant};

	fn request() -> EvalRequest { EvalRequest { context: EvaluationContext { subject: Subject { id: "u".into(), roles: vec!["analyst".into()], claims: vec![], clearance: Classification::Internal }, tenant: Tenant("t".into()), entity: "orders".into(), dataset: "memory".into(), action: Action::Create, destination: Destination::Browser, environment: Environment { env: "test".into(), ip: None, geo: None, device_posture: None, session_freshness: None }, time_bucket: "now".into(), query_shape: QueryShape { verbs: vec![], aggregate: None, grouping_keys: vec![], subject_dimension: None, ordering: vec![], pagination: PaginationMode::LimitOnly { limit: 1 } }, purpose: Purpose("support".into()), policy_version: "v1".into() } } }
	fn policy() -> PolicyCandidate { PolicyCandidate { effect: PolicyEffect::Allow, subjects: vec!["analyst".into()], action: Action::Create, resource: "orders".into(), purpose: Some("support".into()), conditions: vec![], filter: None, obligations: vec![], escalation: None, caps: vec![], masks: vec![], affected_row_cap: None, predicate_use: vec![], id: "create-orders".into() } }

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

	fn request_for(action: Action, entity: &str, tenant: &str) -> EvalRequest {
		EvalRequest { context: EvaluationContext { subject: Subject { id: "u".into(), roles: vec!["analyst".into()], claims: vec![], clearance: Classification::Internal }, tenant: Tenant(tenant.into()), entity: entity.into(), dataset: "memory".into(), action, destination: Destination::Browser, environment: Environment { env: "test".into(), ip: None, geo: None, device_posture: None, session_freshness: None }, time_bucket: "now".into(), query_shape: QueryShape { verbs: vec![], aggregate: None, grouping_keys: vec![], subject_dimension: None, ordering: vec![], pagination: PaginationMode::LimitOnly { limit: 10 } }, purpose: Purpose("support".into()), policy_version: "v1".into() } }
	}
	fn read_request(entity: &str, tenant: &str) -> EvalRequest { request_for(Action::Read, entity, tenant) }
	fn write_request(entity: &str, tenant: &str) -> EvalRequest { request_for(Action::Create, entity, tenant) }

	fn tenant_scoped_write_policy(resource: &str, tenant: &str) -> PolicyCandidate {
		PolicyCandidate { effect: PolicyEffect::Allow, subjects: vec!["analyst".into()], action: Action::Create, resource: resource.into(), purpose: Some("support".into()), conditions: vec![], filter: Some(FilterExpr::TenantEq { value: Value::Str(tenant.into()) }), obligations: vec![], escalation: None, caps: vec![], masks: vec![], affected_row_cap: None, predicate_use: vec![], id: format!("write-{resource}") }
	}

	fn read_policy(resource: &str, tenant: &str, caps: Vec<Cap>, masks: Vec<FieldMask>) -> PolicyCandidate {
		PolicyCandidate { effect: PolicyEffect::Allow, subjects: vec!["analyst".into()], action: Action::Read, resource: resource.into(), purpose: Some("support".into()), conditions: vec![], filter: Some(FilterExpr::TenantEq { value: Value::Str(tenant.into()) }), obligations: vec![], escalation: None, caps, masks, affected_row_cap: None, predicate_use: vec![], id: format!("read-{resource}") }
	}

	fn scratch_dir(name: &str) -> std::path::PathBuf {
		let root = std::env::temp_dir().join(format!("nirdosha-mic-{name}-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		root
	}

	#[test]
	fn guarded_read_returns_the_row_a_prior_write_committed() {
		let root = scratch_dir("read-basic");
		let driver = MemStoreDriver::new();
		let mut writer = GuardClient::new(vec![tenant_scoped_write_policy("orders", "tenant-a")], "w", root.join("w.jsonl"));
		writer.guarded_apply(&write_request("orders", "tenant-a"), &driver, EntityBytes(b"row-1".to_vec()), "t1", 1_000).unwrap();

		let mut reader = GuardClient::new(vec![read_policy("orders", "tenant-a", vec![], vec![])], "r", root.join("r.jsonl"));
		let outcome = reader.guarded_read(&read_request("orders", "tenant-a"), &driver, "t2", 2_000).unwrap();
		assert_eq!(outcome.rows, vec![EntityBytes(b"row-1".to_vec())]);
		assert_eq!(reader.verify_audit(), Ok(3)); // Decision + Mutation(read) + Lineage

		let _ = std::fs::remove_dir_all(root);
	}

	#[test]
	fn guarded_read_denies_a_subject_with_no_matching_policy() {
		let root = scratch_dir("read-deny");
		let driver = MemStoreDriver::new();
		let mut client = GuardClient::new(vec![], "r", root.join("r.jsonl"));
		let result = client.guarded_read(&read_request("orders", "tenant-a"), &driver, "t1", 1_000);
		assert!(matches!(result, Err(Rejected::Denied { .. })));
		let _ = std::fs::remove_dir_all(root);
	}

	#[test]
	fn guarded_read_enforces_tenant_isolation() {
		let root = scratch_dir("read-tenant");
		let driver = MemStoreDriver::new();
		let mut writer = GuardClient::new(vec![tenant_scoped_write_policy("orders", "tenant-a")], "w", root.join("w.jsonl"));
		writer.guarded_apply(&write_request("orders", "tenant-a"), &driver, EntityBytes(b"a-row".to_vec()), "t1", 1_000).unwrap();

		// A read scoped to a DIFFERENT tenant must not see tenant-a's row,
		// even though the row exists in the same (shared, in-memory) store.
		let mut reader = GuardClient::new(vec![read_policy("orders", "tenant-b", vec![], vec![])], "r", root.join("r.jsonl"));
		let outcome = reader.guarded_read(&read_request("orders", "tenant-b"), &driver, "t2", 2_000).unwrap();
		assert!(outcome.rows.is_empty());
		let _ = std::fs::remove_dir_all(root);
	}

	#[test]
	fn guarded_read_respects_row_cap_and_returns_masks_unapplied() {
		let root = scratch_dir("read-cap");
		let driver = MemStoreDriver::new();
		// Authorization is at the entity-kind level ("orders"); which
		// specific stored rows come back is the filter's job (here:
		// tenant scope only) — so writing three *distinct* rows needs
		// three write policies, one per physical resource key, exactly
		// like three different real transaction rows would each need
		// their own write to exist. The read below is authorized once,
		// against "orders" as a kind, and its filter then matches all
		// three.
		let mut writer = GuardClient::new(
			vec![
				tenant_scoped_write_policy("o1", "tenant-a"),
				tenant_scoped_write_policy("o2", "tenant-a"),
				tenant_scoped_write_policy("o3", "tenant-a"),
			],
			"w", root.join("w.jsonl"),
		);
		for (i, resource) in ["o1", "o2", "o3"].iter().enumerate() {
			writer.guarded_apply(&write_request(resource, "tenant-a"), &driver, EntityBytes(vec![i as u8]), format!("t-{resource}"), 1_000).unwrap();
		}
		let mask = FieldMask { field: vec!["ssn".into()], transform: MaskTransform::Full };
		let mut reader = GuardClient::new(vec![read_policy("orders", "tenant-a", vec![Cap::RowCap(2)], vec![mask.clone()])], "r", root.join("r.jsonl"));
		let outcome = reader.guarded_read(&read_request("orders", "tenant-a"), &driver, "t2", 2_000).unwrap();
		assert_eq!(outcome.rows.len(), 2, "RowCap(2) must cap the returned rows even though 3 exist");
		assert_eq!(outcome.masks, vec![mask], "masks are handed to the caller, not silently applied to opaque bytes — see ReadOutcome's doc comment");
		let _ = std::fs::remove_dir_all(root);
	}

	fn action_policy(action: Action, resource: &str, tenant: &str) -> PolicyCandidate {
		PolicyCandidate { effect: PolicyEffect::Allow, subjects: vec!["analyst".into()], action, resource: resource.into(), purpose: Some("support".into()), conditions: vec![], filter: Some(FilterExpr::TenantEq { value: Value::Str(tenant.into()) }), obligations: vec![], escalation: None, caps: vec![], masks: vec![], affected_row_cap: None, predicate_use: vec![], id: format!("{resource}-policy") }
	}

	#[test]
	fn create_rejects_a_resource_that_already_exists() {
		let root = scratch_dir("create-twice");
		let driver = MemStoreDriver::new();
		let mut client = GuardClient::new(vec![action_policy(Action::Create, "orders", "tenant-a")], "c", root.join("c.jsonl"));
		client.guarded_apply(&write_request("orders", "tenant-a"), &driver, EntityBytes(b"v1".to_vec()), "t1", 1_000).unwrap();
		let second = client.guarded_apply(&write_request("orders", "tenant-a"), &driver, EntityBytes(b"v2".to_vec()), "t2", 2_000);
		assert!(matches!(second, Err(Rejected::Failed { .. })), "a second create on the same resource must be rejected, not silently overwrite");
		assert_eq!(driver.get("orders"), Some(b"v1".to_vec()), "the rejected create must not have touched the stored value");
		let _ = std::fs::remove_dir_all(root);
	}

	#[test]
	fn update_rejects_a_resource_that_does_not_exist() {
		let root = scratch_dir("update-missing");
		let driver = MemStoreDriver::new();
		let mut client = GuardClient::new(vec![action_policy(Action::Update, "orders", "tenant-a")], "u", root.join("u.jsonl"));
		let result = client.guarded_apply(&request_for(Action::Update, "orders", "tenant-a"), &driver, EntityBytes(b"v1".to_vec()), "t1", 1_000);
		assert!(matches!(result, Err(Rejected::Failed { .. })), "update on a resource that was never created must be rejected");
		let _ = std::fs::remove_dir_all(root);
	}

	#[test]
	fn update_rejects_when_row_scope_tenant_does_not_match_the_existing_row() {
		let root = scratch_dir("update-wrong-tenant");
		let driver = MemStoreDriver::new();
		let mut creator = GuardClient::new(vec![action_policy(Action::Create, "orders", "tenant-a")], "c", root.join("c.jsonl"));
		creator.guarded_apply(&write_request("orders", "tenant-a"), &driver, EntityBytes(b"v1".to_vec()), "t1", 1_000).unwrap();

		// An Update policy scoped to a DIFFERENT tenant must not be able
		// to touch tenant-a's existing row, even though it names the same
		// resource key.
		let mut updater = GuardClient::new(vec![action_policy(Action::Update, "orders", "tenant-b")], "u", root.join("u.jsonl"));
		let result = updater.guarded_apply(&request_for(Action::Update, "orders", "tenant-b"), &driver, EntityBytes(b"v2".to_vec()), "t2", 2_000);
		assert!(matches!(result, Err(Rejected::Failed { .. })), "row_scope must reject an update whose tenant doesn't match the existing row");
		assert_eq!(driver.get("orders"), Some(b"v1".to_vec()));
		let _ = std::fs::remove_dir_all(root);
	}

	#[test]
	fn update_succeeds_when_row_scope_tenant_matches() {
		let root = scratch_dir("update-ok");
		let driver = MemStoreDriver::new();
		let mut creator = GuardClient::new(vec![action_policy(Action::Create, "orders", "tenant-a")], "c", root.join("c.jsonl"));
		creator.guarded_apply(&write_request("orders", "tenant-a"), &driver, EntityBytes(b"v1".to_vec()), "t1", 1_000).unwrap();

		let mut updater = GuardClient::new(vec![action_policy(Action::Update, "orders", "tenant-a")], "u", root.join("u.jsonl"));
		updater.guarded_apply(&request_for(Action::Update, "orders", "tenant-a"), &driver, EntityBytes(b"v2".to_vec()), "t2", 2_000).unwrap();
		assert_eq!(driver.get("orders"), Some(b"v2".to_vec()));
		let _ = std::fs::remove_dir_all(root);
	}

	#[test]
	fn delete_removes_the_row() {
		let root = scratch_dir("delete-ok");
		let driver = MemStoreDriver::new();
		let mut creator = GuardClient::new(vec![action_policy(Action::Create, "orders", "tenant-a")], "c", root.join("c.jsonl"));
		creator.guarded_apply(&write_request("orders", "tenant-a"), &driver, EntityBytes(b"v1".to_vec()), "t1", 1_000).unwrap();
		assert!(driver.get("orders").is_some());

		let mut deleter = GuardClient::new(vec![action_policy(Action::Delete, "orders", "tenant-a")], "d", root.join("d.jsonl"));
		deleter.guarded_apply(&request_for(Action::Delete, "orders", "tenant-a"), &driver, EntityBytes(vec![]), "t2", 2_000).unwrap();
		assert_eq!(driver.get("orders"), None, "delete must actually remove the row");
		let _ = std::fs::remove_dir_all(root);
	}

	#[test]
	fn delete_rejects_a_resource_that_does_not_exist() {
		let root = scratch_dir("delete-missing");
		let driver = MemStoreDriver::new();
		let mut client = GuardClient::new(vec![action_policy(Action::Delete, "orders", "tenant-a")], "d", root.join("d.jsonl"));
		let result = client.guarded_apply(&request_for(Action::Delete, "orders", "tenant-a"), &driver, EntityBytes(vec![]), "t1", 1_000);
		assert!(matches!(result, Err(Rejected::Failed { .. })));
		let _ = std::fs::remove_dir_all(root);
	}

	#[test]
	fn migrate_requires_a_prior_dry_run_under_the_same_trace_id() {
		let root = scratch_dir("migrate-no-dry-run");
		let driver = MemStoreDriver::new();
		let mut client = GuardClient::new(vec![action_policy(Action::Migrate, "config", "tenant-a")], "m", root.join("m.jsonl"));
		let result = client.guarded_apply(&request_for(Action::Migrate, "config", "tenant-a"), &driver, EntityBytes(b"v1".to_vec()), "t1", 1_000);
		assert!(matches!(result, Err(Rejected::Failed { .. })), "migrate without a prior dry_run under this trace_id must be rejected");
	}

	#[test]
	fn migrate_succeeds_after_a_matching_dry_run() {
		let root = scratch_dir("migrate-ok");
		let driver = MemStoreDriver::new();
		let mut client = GuardClient::new(vec![action_policy(Action::Migrate, "config", "tenant-a")], "m", root.join("m.jsonl"));
		client.dry_run(&request_for(Action::Migrate, "config", "tenant-a"), "t1", 500).expect("dry run must be allowed");
		let outcome = client.guarded_apply(&request_for(Action::Migrate, "config", "tenant-a"), &driver, EntityBytes(b"v1".to_vec()), "t1", 1_000).unwrap();
		assert!(matches!(outcome, Outcome::Committed { .. }));

		// The dry run is single-use: the SAME trace_id can't apply twice
		// off one dry run (the second apply hits the idempotency
		// short-circuit first, which is also correct, but the dry-run set
		// itself must be empty by now too).
		let second = client.guarded_apply(&request_for(Action::Migrate, "other_config", "tenant-a"), &driver, EntityBytes(b"v2".to_vec()), "t1", 2_000);
		assert!(matches!(second, Ok(Outcome::Duplicate { .. })), "same trace_id replays as Duplicate before the action gate is re-checked");
		let _ = std::fs::remove_dir_all(root);
	}

	#[test]
	fn export_is_rejected_from_guarded_apply() {
		let root = scratch_dir("export-rejected");
		let driver = MemStoreDriver::new();
		let mut client = GuardClient::new(vec![action_policy(Action::Export, "report", "tenant-a")], "e", root.join("e.jsonl"));
		let result = client.guarded_apply(&request_for(Action::Export, "report", "tenant-a"), &driver, EntityBytes(vec![]), "t1", 1_000);
		assert!(matches!(result, Err(Rejected::Failed { .. })), "export has no new value to write — guarded_apply must refuse it, not silently no-op");
		let _ = std::fs::remove_dir_all(root);
	}

	fn read_request_paginated(entity: &str, tenant: &str, pagination: PaginationMode) -> EvalRequest {
		let mut req = read_request(entity, tenant);
		req.context.query_shape.pagination = pagination;
		req
	}

	fn seed_rows(driver: &MemStoreDriver, root: &std::path::Path, resources: &[&str], tenant: &str) {
		let policies: Vec<PolicyCandidate> = resources.iter().map(|r| tenant_scoped_write_policy(r, tenant)).collect();
		let mut writer = GuardClient::new(policies, "seed", root.join("seed.jsonl"));
		for resource in resources {
			writer.guarded_apply(&write_request(resource, tenant), driver, EntityBytes(resource.as_bytes().to_vec()), format!("seed-{resource}"), 1_000).unwrap();
		}
	}

	#[test]
	fn guarded_read_rejects_offset_pagination() {
		let root = scratch_dir("pagination-rejected");
		let driver = MemStoreDriver::new();
		let mut client = GuardClient::new(vec![read_policy("orders", "tenant-a", vec![], vec![])], "r", root.join("r.jsonl"));
		let result = client.guarded_read(&read_request_paginated("orders", "tenant-a", PaginationMode::Rejected), &driver, "t1", 1_000);
		assert!(matches!(result, Err(Rejected::Failed { .. })), "PaginationMode::Rejected must be refused before policies are even evaluated");
		let _ = std::fs::remove_dir_all(root);
	}

	#[test]
	fn guarded_read_opaque_cursor_resumes_where_the_previous_page_left_off() {
		let root = scratch_dir("cursor-resume");
		let driver = MemStoreDriver::new();
		seed_rows(&driver, &root, &["p1", "p2", "p3", "p4"], "tenant-a");

		let mut reader = GuardClient::new(vec![read_policy("orders", "tenant-a", vec![Cap::RowCap(2)], vec![])], "r", root.join("r.jsonl"));
		let page1 = reader.guarded_read(&read_request("orders", "tenant-a"), &driver, "t1", 2_000).unwrap();
		assert_eq!(page1.rows.len(), 2);
		let cursor = page1.next_cursor.clone().expect("a truncated page must return a cursor");

		let page2 = reader
			.guarded_read(&read_request_paginated("orders", "tenant-a", PaginationMode::OpaqueCursor(cursor)), &driver, "t2", 3_000)
			.unwrap();
		assert_eq!(page2.rows.len(), 2, "the second page must pick up the remaining two rows");

		// No overlap between the two pages.
		let mut all: Vec<Vec<u8>> = page1.rows.into_iter().map(|r| r.0).collect();
		all.extend(page2.rows.into_iter().map(|r| r.0));
		all.sort();
		all.dedup();
		assert_eq!(all.len(), 4, "the two pages together must cover all four rows exactly once");

		// The final page has no more data, so no cursor.
		assert!(page2.next_cursor.is_none() || {
			let page3 = reader.guarded_read(&read_request_paginated("orders", "tenant-a", PaginationMode::OpaqueCursor(page2.next_cursor.clone().unwrap())), &driver, "t3", 4_000).unwrap();
			page3.rows.is_empty()
		});
		let _ = std::fs::remove_dir_all(root);
	}

	#[test]
	fn guarded_read_enforces_cohort_floor() {
		let root = scratch_dir("cohort-floor");
		let driver = MemStoreDriver::new();
		seed_rows(&driver, &root, &["c1", "c2"], "tenant-a");

		// Only 2 rows exist; a floor of 5 must refuse the whole query, not
		// return the 2 it found.
		let mut reader = GuardClient::new(vec![read_policy("orders", "tenant-a", vec![Cap::CohortFloor(5)], vec![])], "r", root.join("r.jsonl"));
		let result = reader.guarded_read(&read_request("orders", "tenant-a"), &driver, "t1", 2_000);
		assert!(result.is_err(), "cohort_floor must refuse a query that would identify fewer entities than the floor");
	}

	#[test]
	fn guarded_read_allows_when_cohort_floor_is_met() {
		let root = scratch_dir("cohort-floor-ok");
		let driver = MemStoreDriver::new();
		seed_rows(&driver, &root, &["c1", "c2", "c3"], "tenant-a");

		let mut reader = GuardClient::new(vec![read_policy("orders", "tenant-a", vec![Cap::CohortFloor(3)], vec![])], "r", root.join("r.jsonl"));
		let outcome = reader.guarded_read(&read_request("orders", "tenant-a"), &driver, "t1", 2_000).expect("floor of 3 met by exactly 3 rows");
		assert_eq!(outcome.rows.len(), 3);
		let _ = std::fs::remove_dir_all(root);
	}

	#[test]
	fn guarded_read_max_scan_rows_stops_scanning_before_the_end_of_the_store() {
		let root = scratch_dir("max-scan-rows");
		let driver = MemStoreDriver::new();
		// Ten rows exist; MaxScanRows(3) means the driver examines only
		// the first 3 (in key order) regardless of how many would
		// otherwise match — a real bound on scan *work*, distinct from
		// RowCap's bound on results *returned*.
		seed_rows(&driver, &root, &["s0", "s1", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9"], "tenant-a");

		let mut reader = GuardClient::new(vec![read_policy("orders", "tenant-a", vec![Cap::MaxScanRows(3)], vec![])], "r", root.join("r.jsonl"));
		let outcome = reader.guarded_read(&read_request("orders", "tenant-a"), &driver, "t1", 2_000).unwrap();
		assert_eq!(outcome.rows.len(), 3, "only the first 3 scanned rows can possibly match, even though all 10 rows satisfy the tenant filter");
		let _ = std::fs::remove_dir_all(root);
	}

	fn read_policy_with_predicate_use(resource: &str, tenant: &str, predicate_use: Vec<String>) -> PolicyCandidate {
		let mut policy = read_policy(resource, tenant, vec![], vec![FieldMask { field: vec!["amount".into()], transform: MaskTransform::Full }]);
		policy.predicate_use = predicate_use;
		policy
	}

	#[test]
	fn i15_rejects_a_masked_field_driving_the_filter() {
		let root = scratch_dir("i15-reject");
		let driver = MemStoreDriver::new();
		// "amount" is masked by the policy below AND is the field the
		// filter itself keys on — I15 must refuse this, not silently run
		// the filter over a masked field.
		let mut policy = read_policy("orders", "tenant-a", vec![], vec![FieldMask { field: vec!["amount".into()], transform: MaskTransform::Full }]);
		policy.filter = Some(FilterExpr::And(vec![
			FilterExpr::TenantEq { value: Value::Str("tenant-a".into()) },
			FilterExpr::Eq { field: vec!["amount".into()], value: Value::Int(100) },
		]));
		let mut client = GuardClient::new(vec![policy], "r", root.join("r.jsonl"));
		let result = client.guarded_read(&read_request("orders", "tenant-a"), &driver, "t1", 1_000);
		assert!(matches!(result, Err(Rejected::Failed { .. })), "a masked field driving a filter clause must be rejected under I15");
		let _ = std::fs::remove_dir_all(root);
	}

	#[test]
	fn i15_allows_a_masked_field_in_the_filter_when_predicate_use_grants_it() {
		let root = scratch_dir("i15-allow");
		let driver = MemStoreDriver::new();
		let mut policy = read_policy_with_predicate_use("orders", "tenant-a", vec!["amount".to_string()]);
		policy.filter = Some(FilterExpr::And(vec![
			FilterExpr::TenantEq { value: Value::Str("tenant-a".into()) },
			FilterExpr::Eq { field: vec!["amount".into()], value: Value::Int(100) },
		]));
		let mut client = GuardClient::new(vec![policy], "r", root.join("r.jsonl"));
		// The filter itself won't match anything real in this store
		// (MemStoreDriver only understands resource/tenant fields, so an
		// "amount" filter always excludes every row) — the point here is
		// only that I15 doesn't reject the *plan*, matching
		// grant predicate_use's actual purpose.
		let outcome = client.guarded_read(&read_request("orders", "tenant-a"), &driver, "t1", 1_000);
		assert!(outcome.is_ok(), "predicate_use must let a masked field pass the I15 check: {outcome:?}");
		let _ = std::fs::remove_dir_all(root);
	}
}

