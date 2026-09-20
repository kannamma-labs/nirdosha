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

pub mod break_glass;
pub mod cedar;
pub mod decision_cache;
pub mod delegation;
pub mod drivers;
pub mod evaluator;
pub mod guard_down;
pub mod relation_lower;
pub mod snapshot;

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
/// and `admin-mint-delegation` uses `"delegate"`. Every one of those real
/// policies silently vanished from `PolicyRegistration::to_candidate()`
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
            "simulate" => Some(Action::Simulate),
            "delegate" => Some(Action::Delegate),
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
