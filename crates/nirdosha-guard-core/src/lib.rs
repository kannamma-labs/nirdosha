//! `nirdosha-guard-core` — store-agnostic access-control IR.
//!
//! This crate owns the intermediate representation and invariants described
//! in RFC 0023 ("Nirdosha Guard — Store-Agnostic Access Control"). It is
//! deliberately storage-agnostic: every driver translates these typed
//! plans into the strongest native enforcement the store supports.
//!
//! Design constraints carried through the types:
//! - Request-level context only for policy evaluation (`EvaluationContext`).
//!   Row-level facts live in `FilterExpr`, not the context, so decision
//!   caches remain cacheable and safe.
//! - All filters compile from a typed, parameterized AST (`FilterExpr`).
//!   No string-level query rewriting happens in this crate.
//! - Relations never reach drivers; `RelationIn` is resolved at plan-compile
//!   time into bounded value sets or materialized columns.
//! - The IR variants are intentionally closed sets so a `coverage_matrix!`
//!   can enforce that every new IR concept gets canonical syntax and a
//!   compiled example test.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub mod approval_chain;
pub mod break_glass;
pub mod cedar;
pub mod decision_cache;
pub mod delegation;
pub mod drivers;
pub mod evaluator;
pub mod guard_down;
pub mod relation_lower;
pub mod snapshot;

/// Collects every field path a `FilterExpr` references, recursing through
/// boolean combinators. Used for I15 (masked fields excluded from
/// filter/join/grouping/having/ordering/window unless explicitly granted
/// via `predicate_use`) — the check needs to know which fields a filter
/// actually touches, not just that a filter exists.
pub fn filter_fields(expr: &FilterExpr) -> Vec<&FieldPath> {
    match expr {
        FilterExpr::Eq { field, .. }
        | FilterExpr::In { field, .. }
        | FilterExpr::Compare { field, .. }
        | FilterExpr::TimeRange { field, .. }
        | FilterExpr::Pattern { field, .. }
        | FilterExpr::RelationIn { field, .. } => vec![field],
        FilterExpr::TenantEq { .. } => vec![],
        FilterExpr::And(children) | FilterExpr::Or(children) => {
            children.iter().flat_map(filter_fields).collect()
        }
        FilterExpr::Not(inner) => filter_fields(inner),
    }
}

/// Extracts the tenant a `FilterExpr` scopes to, if any — the shared
/// implementation both `nirdosha-guard-store-postgres` and
/// `MemStoreDriver` need to enforce tenant isolation on `prepare`/`query`
/// (a write with no `TenantEq` anywhere in its filter is rejected by both
/// rather than written un-scoped). Lived as a private duplicate inside the
/// Postgres driver crate alone until `MemStoreDriver` needed the identical
/// logic for its own read-path query filtering (Plan Phase 7) — moved
/// here so there's one implementation, not two that can drift.
pub fn extract_tenant(filter: &FilterExpr) -> Option<String> {
    match filter {
        FilterExpr::TenantEq { value: Value::Str(tenant) } => Some(tenant.clone()),
        FilterExpr::And(children) | FilterExpr::Or(children) => children.iter().find_map(extract_tenant),
        FilterExpr::Not(inner) => extract_tenant(inner),
        _ => None,
    }
}

/// Logical-to-physical field-name mapping for a dataset, used by the
/// logging policy guard to resolve canonical compliance concepts (e.g.
/// `cvv`) into the actual column names a store uses (e.g. `_CCV`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DatasetFieldMap {
    pub entity: EntityId,
    /// concept name -> list of physical field paths that carry that concept.
    pub concept_to_physical: BTreeMap<String, Vec<FieldPath>>,
}

impl DatasetFieldMap {
    pub fn physical_for(&self, concept: &str) -> &[FieldPath] {
        self.concept_to_physical
            .get(concept)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }
}

/// A path to a field inside a record, e.g. `customer.address.zip`.
pub type FieldPath = Vec<String>;

/// Logical entity name, e.g. `customer`.
pub type EntityId = String;

/// A specific physical binding of an entity, e.g. `parquet_lake`.
pub type DatasetId = String;

/// Identifier for one physical binding inside a federated plan.
pub type BindingId = String;

/// Opaque policy-version identifier; part of every cache key.
pub type PolicyVersion = String;

/// Opaque source-epoch token for relation freshness.
pub type SourceEpoch = String;

/// One entity touched at row level, with pre-tokenized keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineageEntity {
    pub entity: EntityId,
    pub keys: Vec<String>,
}

/// Driver-reported lineage enrichment. Kernel-owned fields are intentionally
/// absent: the driver can report sources and refine sink keys only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LineageFacts {
    pub sources: Vec<LineageEntity>,
    pub sink_keys: Vec<String>,
}

/// Subject identity: the actor requesting access.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Subject {
    pub id: String,
    pub roles: Vec<String>,
    pub claims: Vec<String>,
    /// Maximum classification this subject is cleared for.
    pub clearance: Classification,
}

/// Tenant identifier. Mandatory in every evaluation context.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Tenant(pub String);

/// Where the result will be sent. A first-class policy dimension.
///
/// `FeaturePipeline`/`Warehouse` added alongside the original five: real
/// `destination(...)` clauses in `examples/rtm/roles-N-guard_policy.md`
/// (`destination(feature_pipeline)` on the feature-engine read,
/// `destination(warehouse)` on the BI aggregate) referenced destinations
/// this closed set didn't cover — found while lowering that corpus's
/// clauses into this IR, not invented speculatively.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Destination {
    Browser,
    ApiClient,
    LlmContext,
    ExportFile,
    Webhook,
    FeaturePipeline,
    Warehouse,
}

/// Runtime environment attributes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Environment {
    pub env: String,
    pub ip: Option<String>,
    pub geo: Option<String>,
    pub device_posture: Option<String>,
    pub session_freshness: Option<String>,
}

/// Classification levels. Used for masking, audit sampling, and destination control.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Classification {
    Public,
    Internal,
    Confidential,
    Restricted,
}

/// Declared purpose code. Closed enum core + tenant-extensible registry.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Purpose(pub String);

/// The request-level context fed to policy evaluation.
/// Row-level attributes are **not** here; they belong in `FilterExpr`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EvaluationContext {
    pub subject: Subject,
    pub tenant: Tenant,
    pub entity: EntityId,
    pub dataset: DatasetId,
    pub action: Action,
    pub destination: Destination,
    pub environment: Environment,
    /// Evaluation timestamp truncated to a time bucket.
    pub time_bucket: String,
    pub query_shape: QueryShape,
    pub purpose: Purpose,
    pub policy_version: PolicyVersion,
}

/// Actions the guard decides on.
///
/// `Aggregate`/`LineageQuery`/`Simulate`/`Delegate` added alongside the
/// original seven: found by running RTM's real corpus
/// (`examples/rtm/roles-N-guard_policy.md`) through `Action::parse_wire` —
/// `bi-aggregate`/`lead-dashboard`/`hold-stats`/`lead-sla-aggregate` use
/// `action == "aggregate"`, `lineage-explore`/`lineage-audit` use
/// `"lineage_query"` (matching `lineage_query!`'s own vocabulary),
/// `policy-simulate` uses `"simulate"` (matching `policy_simulation!`),
/// `admin-mint-delegation` uses `"delegate"`, and the RTM network graph
/// screens (`7.1`/`7.2`/`7.3`/`7.5`) use `"link_query"` (matching the
/// menu guard action that was previously unparseable). Every one of those
/// real policies silently vanished from `PolicyRegistration::to_candidate()`
/// (which `filter_map`s out an unparseable action) before this — not a
/// compile error, not a runtime panic, just a policy that looked live in
/// every listing and never matched a request. `nirdosha-guard-verify`'s V3
/// pass now catches exactly this class of gap for whatever this enum
/// still doesn't cover.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Action {
    Read,
    Create,
    Update,
    Delete,
    Migrate,
    Export,
    Enumerate,
    Aggregate,
    LineageQuery,
    LinkQuery,
    Simulate,
    Delegate,
}

impl Action {
    /// Parses the lowercase wire string a `guard_policy!` source literal
    /// carries (`action == "read"`, `action in ["create", "update"]`, ...)
    /// into the closed `Action` enum. Returns `None` for anything outside
    /// the closed set — the caller decides whether that's a hard error or
    /// a skip; this type has no opinion.
    pub fn parse_wire(value: &str) -> Option<Self> {
        match value {
            "read" => Some(Action::Read),
            "create" => Some(Action::Create),
            "update" => Some(Action::Update),
            "delete" => Some(Action::Delete),
            "migrate" => Some(Action::Migrate),
            "export" => Some(Action::Export),
            "enumerate" => Some(Action::Enumerate),
            "aggregate" => Some(Action::Aggregate),
            "lineage_query" => Some(Action::LineageQuery),
            "link_query" => Some(Action::LinkQuery),
            "simulate" => Some(Action::Simulate),
            "delegate" => Some(Action::Delegate),
            _ => None,
        }
    }

    /// The inverse of `parse_wire` — the lowercase wire string this
    /// `Action` came from (or would parse back from). Used anywhere a
    /// real wire-format string is needed for an `Action` (the Cedar
    /// frontend's `Action::"<wire>"` entity id, for one) instead of
    /// `Debug`'s PascalCase, which isn't the same vocabulary
    /// `parse_wire` accepts.
    pub fn wire_str(&self) -> &'static str {
        match self {
            Action::Read => "read",
            Action::Create => "create",
            Action::Update => "update",
            Action::Delete => "delete",
            Action::Migrate => "migrate",
            Action::Export => "export",
            Action::Enumerate => "enumerate",
            Action::Aggregate => "aggregate",
            Action::LineageQuery => "lineage_query",
            Action::LinkQuery => "link_query",
            Action::Simulate => "simulate",
            Action::Delegate => "delegate",
        }
    }

    /// Maps to the closed `WriteAction` set `WritePlan`/`guarded_apply`
    /// operate on. `None` for every non-mutating `Action` — `guarded_apply`
    /// uses this to refuse a call whose context action isn't actually a
    /// write, rather than silently treating (say) a `Read` as an implicit
    /// `Update`.
    pub fn as_write_action(&self) -> Option<WriteAction> {
        match self {
            Action::Create => Some(WriteAction::Create),
            Action::Update => Some(WriteAction::Update),
            Action::Delete => Some(WriteAction::Delete),
            Action::Migrate => Some(WriteAction::Migrate),
            Action::Export => Some(WriteAction::Export),
            _ => None,
        }
    }
}

/// Shape of the incoming query, used for policy context and aggregate safety.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct QueryShape {
    pub verbs: Vec<QueryVerb>,
    pub aggregate: Option<AggregateSpec>,
    pub grouping_keys: Vec<FieldPath>,
    pub subject_dimension: Option<FieldPath>,
    pub ordering: Vec<OrderSpec>,
    pub pagination: PaginationMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum QueryVerb {
    Select,
    Count,
    Exists,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AggregateSpec {
    pub function: String,
    pub field: FieldPath,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OrderSpec {
    pub field: FieldPath,
    pub descending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PaginationMode {
    OpaqueCursor(String),
    LimitOnly { limit: u64 },
    /// Offset pagination is rejected for guarded resources.
    Rejected,
}

/// Terminal and non-terminal decisions.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Decision {
    Allow,
    Deny { reason: String },
    Escalate { to: EscalateTarget },
    Pending {
        handle: String,
        expires_at: String,
        cause: PendingCause,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PendingCause {
    DuplicateInFlight,
    AwaitingObligation,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EscalateTarget {
    Approval { chain: String },
    StepUp { method: String },
    Materialization { job: String },
    StrongerDriver,
}

/// Typed, parameterized filter expression.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FilterExpr {
    Eq { field: FieldPath, value: Value },
    In { field: FieldPath, values: Vec<Value> },
    Compare { field: FieldPath, op: CompareOp, value: Value },
    And(Vec<FilterExpr>),
    Or(Vec<FilterExpr>),
    Not(Box<FilterExpr>),
    TimeRange { field: FieldPath, from: String, to: String },
    Pattern { field: FieldPath, matcher: PatternMatcher },
    TenantEq { value: Value },
    /// Erased at plan-compile time: relations never reach drivers.
    RelationIn { field: FieldPath, relation: RelationExpr },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CompareOp {
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PatternMatcher {
    Glob(String),
    Prefix(String),
    Exact(String),
}

/// A relation to resolve at plan-compile time.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RelationExpr {
    pub name: String,
    pub source: String,
    pub max_cardinality: u64,
    pub ttl_seconds: u64,
}

/// A literal value in a filter. Intentionally closed: no runtime string building.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Value {
    Int(i64),
    Bool(bool),
    Str(String),
    Dec(String), // canonical string form; precise type enforced by schema
    Null,
}

/// Read plan for a single dataset binding.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AccessPlan {
    pub decision: Decision,
    pub filter: Option<FilterExpr>,
    pub masks: Vec<FieldMask>,
    pub caps: Vec<Cap>,
    pub obligations: Vec<Obligation>,
    pub policy_version: PolicyVersion,
}

/// Mask applied to a field before data leaves the process.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FieldMask {
    pub field: FieldPath,
    pub transform: MaskTransform,
}

/// Closed registry of mask transforms. Values are source literals.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MaskTransform {
    Full,
    PartialLast4,
    Hash,
    Drop,
    Custom { name: String },
}

/// Cost and correctness caps.
///
/// `MaxDepth`/`MaxNodes` added alongside the original six: RTM's
/// `lineage-explore`/`lineage-audit` policies cap graph-traversal depth and
/// node count (`cap(max_depth = 5, max_nodes = 1_000, ...)`) — a real cap
/// kind this enum didn't have room for, found while lowering that policy's
/// clauses, not invented speculatively. `affected_rows` (write scope) is
/// deliberately NOT here — it's `WritePlan::affected_row_cap`, a different
/// concept (bounding rows touched by a mutation, not rows scanned by a
/// read).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Cap {
    RowCap(u64),
    MaxScanRows(u64),
    MaxScanBytes(u64),
    MaxExecutionTimeMs(u64),
    MaxResultBytes(u64),
    CohortFloor(u64),
    MaxDepth(u64),
    MaxNodes(u64),
}

/// Blocking or non-blocking obligation attached to a decision.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Obligation {
    Audit { level: AuditLevel },
    Notify { channel: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AuditLevel {
    Full,
    /// Sampling rate in parts per thousand (0–1000).
    Sample { rate_permille: u64 },
}

/// Federated read plan spanning multiple physical bindings.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FederatedPlan {
    pub decision: Decision,
    pub sub_plans: Vec<(BindingId, AccessPlan)>,
    pub merge: MergeSpec,
    pub budget: BudgetToken,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MergeSpec {
    pub mode: MergeMode,
    pub dedup_keys: Vec<FieldPath>,
    pub provenance: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MergeMode {
    Union,
    Join,
}

/// Per-request budget issued by the global caps coordinator.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BudgetToken {
    pub max_scan_rows: u64,
    pub max_scan_bytes: u64,
    pub max_execution_time_ms: u64,
    pub max_result_bytes: u64,
}

/// Write plan: a decision plus store-enforced scope and invariants.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WritePlan {
    pub decision: Decision,
    pub action: WriteAction,
    pub row_scope: Option<FilterExpr>,
    pub preconditions: Vec<Condition>,
    pub postconditions: Vec<Condition>,
    pub field_policy: Vec<FieldPolicy>,
    pub affected_row_cap: u64,
    pub obligations: Vec<Obligation>,
    pub policy_version: PolicyVersion,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WriteAction {
    Create,
    Update,
    Delete,
    Migrate,
    Export,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Condition {
    Expr(FilterExpr),
    Custom(InvariantId),
}

/// Reference to a pure, schema-typed invariant registered at compile time.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InvariantId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FieldPolicy {
    Required(FieldPath),
    Allowed(FieldPath),
    Forbidden(FieldPath),
    Default(FieldPath, Value),
}

/// Resolution tier for a relation. Relations are erased before drivers see them.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ResolutionTier {
    /// Predicate on stored columns.
    Tier0Native,
    /// Bounded value set resolved at decision time.
    Tier1InList,
    /// Materialized column maintained write-through.
    Tier2Materialized,
    /// Deny or approved materialization job.
    Tier3DenyOrEscalate,
}

/// Freshness contract for a resolved relation set.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Freshness {
    pub source: String,
    pub source_epoch: SourceEpoch,
    pub resolved_at: String,
    pub ttl_seconds: u64,
}

/// A relation resolver provider. OpenFGA / SpiceDB implement this trait.
pub trait RelationResolver {
    /// Resolve one subject/relation pair to a bounded value set.
    fn resolve(
        &self,
        subject: &Subject,
        relation: &RelationExpr,
    ) -> Result<ResolvedRelation, RelationError>;

    /// Batch resolution for bulk flows.
    fn resolve_many(
        &self,
        subjects: &[Subject],
        relation: &RelationExpr,
    ) -> Result<Vec<ResolvedRelation>, RelationError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ResolvedRelation {
    pub values: Vec<Value>,
    pub freshness: Freshness,
    pub tier: ResolutionTier,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RelationError {
    Unresolvable,
    CardinalityExceeded { max: u64, actual: u64 },
    Stale { source_epoch: SourceEpoch },
    SourceUnavailable,
}

/// Policy front-end trait. Cedar is one implementation; the IR is the contract.
pub trait PolicyFrontend {
    type Error;

    /// Evaluate a policy set + schema against the request context.
    /// Returns a decision plus any residual obligations.
    fn evaluate(
        &self,
        context: &EvaluationContext,
    ) -> Result<(Decision, Vec<Obligation>), Self::Error>;
}

/// Driver capability manifest. Emitted by each driver crate.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CapabilityManifest {
    pub schema_version: u32,
    pub driver_name: String,
    pub supported_filter_nodes: Vec<FilterNodeKind>,
    pub masking_points: Vec<MaskingPoint>,
    pub aggregate_semantics: AggregateSemantics,
    pub supports_tenant_eq_native: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FilterNodeKind {
    Eq,
    In,
    Compare,
    And,
    Or,
    Not,
    TimeRange,
    Pattern,
    TenantEq,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MaskingPoint {
    Projection,
    ScanTime,
    DelegatedDdl,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AggregateSemantics {
    Inline,
    PushdownFilterThenAggregate,
    Unsupported,
}

/// Driver attestation methods required by I12.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DriverAttestation {
    CanaryRows,
    DifferentialTests,
    QueryPlanInspection,
}

/// Which `FilterNodeKind`s a `FilterExpr` actually uses — the input
/// `compute_enforcement_level` needs to decide how much of a filter a
/// driver's manifest can honestly push down. `RelationIn` needs none:
/// relations are erased before a driver ever sees a plan (RFC 0023 §4),
/// so they carry no pushdown requirement of their own by the time this
/// runs.
pub fn required_filter_node_kinds(expr: &FilterExpr) -> Vec<FilterNodeKind> {
    match expr {
        FilterExpr::Eq { .. } => vec![FilterNodeKind::Eq],
        FilterExpr::In { .. } => vec![FilterNodeKind::In],
        FilterExpr::Compare { .. } => vec![FilterNodeKind::Compare],
        FilterExpr::TimeRange { .. } => vec![FilterNodeKind::TimeRange],
        FilterExpr::Pattern { .. } => vec![FilterNodeKind::Pattern],
        FilterExpr::TenantEq { .. } => vec![FilterNodeKind::TenantEq],
        FilterExpr::RelationIn { .. } => vec![],
        FilterExpr::And(children) => {
            let mut kinds: Vec<FilterNodeKind> = children.iter().flat_map(required_filter_node_kinds).collect();
            kinds.push(FilterNodeKind::And);
            kinds
        }
        FilterExpr::Or(children) => {
            let mut kinds: Vec<FilterNodeKind> = children.iter().flat_map(required_filter_node_kinds).collect();
            kinds.push(FilterNodeKind::Or);
            kinds
        }
        FilterExpr::Not(inner) => {
            let mut kinds = required_filter_node_kinds(inner);
            kinds.push(FilterNodeKind::Not);
            kinds
        }
    }
}

/// Selects the capability-ladder level a driver actually achieves for a
/// given filter, per its `CapabilityManifest`'s claimed
/// `supported_filter_nodes` — not the driver's own opinion of itself, so
/// this is the same computation whether the manifest is honest or (see
/// `DriverAttestation`) has been caught lying and downgraded first.
/// `L3Pruning` never comes out of this today: it's specific to
/// partition-pruned columnar storage (RFC 0023's Arrow/Parquet driver,
/// `[ ]` in the checklist), which neither existing driver is — producing
/// it here without a driver that actually does pruning would be exactly
/// the kind of aspirational-not-honest claim this ladder exists to
/// prevent.
pub fn compute_enforcement_level(filter: Option<&FilterExpr>, manifest: &CapabilityManifest) -> EnforcementLevel {
    let Some(filter) = filter else {
        return EnforcementLevel::L4FullPushdown;
    };
    let needed = required_filter_node_kinds(filter);
    if needed.is_empty() {
        return EnforcementLevel::L4FullPushdown;
    }
    let supported: std::collections::HashSet<&FilterNodeKind> = manifest.supported_filter_nodes.iter().collect();
    let supported_count = needed.iter().filter(|kind| supported.contains(kind)).count();
    if supported_count == needed.len() {
        EnforcementLevel::L4FullPushdown
    } else if supported_count > 0 {
        EnforcementLevel::L2ScanTime
    } else {
        EnforcementLevel::L1PostRead
    }
}

/// A compiled plan plus the driver ladder level it achieved.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CompiledPlan<L> {
    pub plan: L,
    pub level: EnforcementLevel,
    pub manifest: CapabilityManifest,
}

/// Capability ladder: L4 full pushdown → L0 deny.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EnforcementLevel {
    L4FullPushdown,
    L3Pruning,
    L2ScanTime,
    L1PostRead,
    L0Deny,
}

/// A cacheable, parameterized filter fragment.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FilterFragment {
    pub expr: FilterExpr,
    pub parameters: Vec<String>,
}

/// Decision cache key: request-level only, never row-level.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DecisionCacheKey {
    pub subject_id: String,
    pub roles_hash: u64,
    pub tenant: Tenant,
    pub entity: EntityId,
    pub dataset: DatasetId,
    pub action: Action,
    pub environment_hash: u64,
    pub destination: Destination,
    pub time_bucket: String,
    pub query_shape_hash: u64,
    pub purpose: Purpose,
    pub policy_version: PolicyVersion,
}

/// Audit-sampling table: classification → rate; denials/exports/etc. are always full.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AuditSamplingTable {
    pub rates: BTreeMap<Classification, u64>,
    pub always_full: Vec<String>,
}

/// Catalog routing metadata for a dataset binding.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BindingRouting {
    pub binding_id: BindingId,
    pub freshness_lag_seconds: u64,
    pub latency_class: String,
    pub cost_tier: String,
    pub affinities: Vec<QueryShape>,
    pub authoritative_for: Vec<QueryShape>,
}

/// A dataset registry entry.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DatasetRegistryEntry {
    pub entity: EntityId,
    pub dataset: DatasetId,
    pub store: String,
    pub routing: BindingRouting,
    pub primary_write_binding: bool,
}

// LineageFacts in scope note: `keys` MUST be pre-tokenized by the driver (the
// plane never receives raw RESTRICTED values — RFC 0026 §6.1/§13); node
// resolution is collector-side (nirdosha-lineage), not driver work.
#[cfg(test)]
mod lineage_facts_tests {
    use super::*;

    #[test]
    fn lineage_facts_default_has_no_enrichment() {
        let facts = LineageFacts::default();
        assert!(facts.sources.is_empty());
        assert!(facts.sink_keys.is_empty());
    }

    #[test]
    fn lineage_facts_round_trip_json() {
        let facts = LineageFacts {
            sources: vec![LineageEntity {
                entity: "txn_events".into(),
                keys: vec!["tok_1".into()],
            }],
            sink_keys: vec!["tok_9".into()],
        };
        let json = serde_json::to_string(&facts).unwrap();
        let back: LineageFacts = serde_json::from_str(&json).unwrap();
        assert_eq!(facts, back);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_str_round_trips_through_parse_wire_for_every_action() {
        for action in [Action::Read, Action::Create, Action::Update, Action::Delete, Action::Migrate, Action::Export, Action::Enumerate, Action::Aggregate, Action::LineageQuery, Action::LinkQuery, Action::Simulate, Action::Delegate] {
            assert_eq!(Action::parse_wire(action.wire_str()), Some(action.clone()), "wire_str()/parse_wire() must round-trip for {action:?}");
        }
    }

    fn manifest_supporting(kinds: &[FilterNodeKind]) -> CapabilityManifest {
        CapabilityManifest {
            schema_version: 1,
            driver_name: "test".into(),
            supported_filter_nodes: kinds.to_vec(),
            masking_points: vec![],
            aggregate_semantics: AggregateSemantics::Inline,
            supports_tenant_eq_native: true,
        }
    }

    #[test]
    fn enforcement_level_is_l4_when_every_node_is_supported() {
        let filter = FilterExpr::And(vec![
            FilterExpr::TenantEq { value: Value::Str("t1".into()) },
            FilterExpr::Eq { field: vec!["resource".into()], value: Value::Str("r1".into()) },
        ]);
        let manifest = manifest_supporting(&[FilterNodeKind::TenantEq, FilterNodeKind::Eq, FilterNodeKind::And]);
        assert_eq!(compute_enforcement_level(Some(&filter), &manifest), EnforcementLevel::L4FullPushdown);
    }

    #[test]
    fn enforcement_level_is_l1_when_no_node_is_supported() {
        let filter = FilterExpr::Compare { field: vec!["amount".into()], op: CompareOp::Gt, value: Value::Int(0) };
        let manifest = manifest_supporting(&[FilterNodeKind::TenantEq]);
        assert_eq!(compute_enforcement_level(Some(&filter), &manifest), EnforcementLevel::L1PostRead);
    }

    #[test]
    fn enforcement_level_is_l2_when_some_but_not_all_nodes_are_supported() {
        let filter = FilterExpr::And(vec![
            FilterExpr::TenantEq { value: Value::Str("t1".into()) },
            FilterExpr::Compare { field: vec!["amount".into()], op: CompareOp::Gt, value: Value::Int(0) },
        ]);
        let manifest = manifest_supporting(&[FilterNodeKind::TenantEq, FilterNodeKind::And]);
        assert_eq!(compute_enforcement_level(Some(&filter), &manifest), EnforcementLevel::L2ScanTime);
    }

    #[test]
    fn enforcement_level_is_l4_with_no_filter_at_all() {
        let manifest = manifest_supporting(&[]);
        assert_eq!(compute_enforcement_level(None, &manifest), EnforcementLevel::L4FullPushdown);
    }

    #[test]
    fn round_trip_access_plan_json() {
        let plan = AccessPlan {
            decision: Decision::Allow,
            filter: Some(FilterExpr::TenantEq {
                value: Value::Str("tenant-42".into()),
            }),
            masks: vec![FieldMask {
                field: vec!["salary".into()],
                transform: MaskTransform::Full,
            }],
            caps: vec![Cap::RowCap(100)],
            obligations: vec![Obligation::Audit { level: AuditLevel::Full }],
            policy_version: "v1".into(),
        };
        let json = serde_json::to_string(&plan).unwrap();
        let back: AccessPlan = serde_json::from_str(&json).unwrap();
        assert_eq!(plan, back);
    }

    #[test]
    fn write_plan_default_field_policy_is_forbidden_unless_listed() {
        // The IR itself does not enforce the "unlisted = forbidden" rule;
        // this test documents the contract the macro layer must enforce.
        let plan = WritePlan {
            decision: Decision::Allow,
            action: WriteAction::Update,
            row_scope: Some(FilterExpr::Eq {
                field: vec!["id".into()],
                value: Value::Int(7),
            }),
            preconditions: vec![Condition::Expr(FilterExpr::In {
                field: vec!["status".into()],
                values: vec![Value::Str("pending".into()), Value::Str("active".into())],
            })],
            postconditions: vec![Condition::Expr(FilterExpr::Compare {
                field: vec!["balance".into()],
                op: CompareOp::Le,
                value: Value::Dec("100.00".into()),
            })],
            field_policy: vec![
                FieldPolicy::Required(vec!["tenant_id".into()]),
                FieldPolicy::Allowed(vec!["balance".into()]),
                FieldPolicy::Forbidden(vec!["credit_limit".into()]),
            ],
            affected_row_cap: 1,
            obligations: vec![Obligation::Audit { level: AuditLevel::Full }],
            policy_version: "v1".into(),
        };
        assert!(matches!(plan.action, WriteAction::Update));
        assert_eq!(plan.affected_row_cap, 1);
    }

    #[test]
    fn decision_cache_key_contains_no_row_level_data() {
        let key = DecisionCacheKey {
            subject_id: "u1".into(),
            roles_hash: 0,
            tenant: Tenant("t1".into()),
            entity: "customer".into(),
            dataset: "pg-primary".into(),
            action: Action::Read,
            environment_hash: 0,
            destination: Destination::Browser,
            time_bucket: "2026-09-18T12:00:00Z".into(),
            query_shape_hash: 0,
            purpose: Purpose("support".into()),
            policy_version: "v1".into(),
        };
        let json = serde_json::to_string(&key).unwrap();
        // Row-level facts must not appear literally in the cache key.
        assert!(!json.contains("row_id"));
        assert!(!json.contains("record_owner"));
    }
}
