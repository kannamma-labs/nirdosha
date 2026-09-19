//! `nirdosha-lineage` — RFC 0026: governed lineage over the guard kernel.
//!
//! Phase 1 scope (pre-phase scaffold — payload + plane, no emission):
//! - node/edge/observation types (RFC 0026 §6.1–§6.2, §7.1–§7.2)
//! - the I18-unconditional collector seam (`LineageObservation::from_context`)
//! - deterministic chain→graph projection (`projection::project_edges`)
//! - the swappable `GraphStore` port (§6.4; two-driver rule, V8)
//!
//! NOT here yet (substrate-gated — see `rfcs/0026-metadata-plane-checklist.md`):
//! the emission hook in the MIC (`guarded_apply`), I18 enforcement, the MP-9
//! decision-equivalence probe, live query execution through the guard, V10.

pub mod collector;
pub mod projection;
pub mod query;
pub mod store;
pub mod time;
pub mod v10;

pub use collector::{mp9_decision_equivalence, resolve_entity, CollectorConfig, KernelCollector, LineageObservation, ObservationContext, PlanFacts};
pub use projection::{project_edges, project_observations};
pub use query::{
    filter_eq, param, IntoLineageFilter, IntoLineageParam, LineageParamValue, LineageQueryError,
    LineageQueryRow, LineageQueryRunner, LineageQueryShape, LineageValue,
};
pub use store::{EdgeKey, EdgeQueryFilter, GraphStore, LineageStoreError};

use serde::{Deserialize, Serialize};

// One import site for the guard-core types the plane speaks.
pub use nirdosha_guard_core::{Destination, FilterExpr, PolicyVersion, Purpose};

/// Correlation id of the originating guarded execution (audit-chain trace id).
pub type TraceId = String;

// ─────────────────────────────────────────────────────────────────────────────
// Node model — RFC 0026 §6.1
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum NodeKind {
    Data,
    Policy,
    Runtime,
}

impl NodeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            NodeKind::Data => "data",
            NodeKind::Policy => "policy",
            NodeKind::Runtime => "runtime",
        }
    }
}

/// Stable node identity — survives driver and vendor swaps (RFC 0026 §6.5).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NodeId {
    pub kind: NodeKind,
    pub catalog_id: String,
    pub version: Option<String>,
}

impl NodeId {
    pub fn data(catalog_id: impl Into<String>, version: Option<String>) -> Self {
        NodeId { kind: NodeKind::Data, catalog_id: catalog_id.into(), version }
    }
    pub fn policy(catalog_id: impl Into<String>, version: Option<String>) -> Self {
        NodeId { kind: NodeKind::Policy, catalog_id: catalog_id.into(), version }
    }
    pub fn runtime(catalog_id: impl Into<String>, version: Option<String>) -> Self {
        NodeId { kind: NodeKind::Runtime, catalog_id: catalog_id.into(), version }
    }
    /// Models are Data nodes (RFC 0026 §6.1 table); version = artifact version.
    pub fn model(catalog_id: impl Into<String>, version: Option<String>) -> Self {
        NodeId::data(catalog_id, version)
    }
}

/// Pre-tokenized key reference. The plane NEVER receives raw RESTRICTED
/// values — drivers pre-tokenize by contract (guard-core `LineageEntity`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct KeyRef {
    pub token: String,
}

/// A resolved graph node plus optional row-level (tokenized) keys.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EntityRef {
    pub node: NodeId,
    pub keys: Option<Vec<KeyRef>>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Edge model — RFC 0026 §6.2
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EdgeType {
    // data-flow
    Read,
    DerivedFrom,
    MergedFrom,
    ScoredBy,
    ScreenedBy,
    AlertedBy,
    ExportedTo,
    // governance
    GovernedBy,
    EvaluatedUnder,
    ApprovedBy,
    DelegatedTo,
    // lifecycle
    VersionOf,
    DeployedOn,
    InstalledAs,
    Consumes,
}

impl EdgeType {
    pub fn as_str(&self) -> &'static str {
        match self {
            EdgeType::Read => "read",
            EdgeType::DerivedFrom => "derived_from",
            EdgeType::MergedFrom => "merged_from",
            EdgeType::ScoredBy => "scored_by",
            EdgeType::ScreenedBy => "screened_by",
            EdgeType::AlertedBy => "alerted_by",
            EdgeType::ExportedTo => "exported_to",
            EdgeType::GovernedBy => "governed_by",
            EdgeType::EvaluatedUnder => "evaluated_under",
            EdgeType::ApprovedBy => "approved_by",
            EdgeType::DelegatedTo => "delegated_to",
            EdgeType::VersionOf => "version_of",
            EdgeType::DeployedOn => "deployed_on",
            EdgeType::InstalledAs => "installed_as",
            EdgeType::Consumes => "consumes",
        }
    }
}

/// Trust level of an edge. NEVER merged silently (MP-7): `BreakGlass` stats
/// must not dissolve into ordinary-flow stats, which is why `authority` is a
/// key dimension of `EdgeKey`, not mergeable metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Authority {
    KernelExecution,
    KernelIssuance,
    BreakGlass,
    ExternalClaimed,
}

impl Authority {
    pub fn as_str(&self) -> &'static str {
        match self {
            Authority::KernelExecution => "kernel_execution",
            Authority::KernelIssuance => "kernel_issuance",
            Authority::BreakGlass => "break_glass",
            Authority::ExternalClaimed => "external_claimed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum FlowCompleteness {
    Full,
    Watermark,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DriverRef {
    pub port: String,
    pub vendor: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TransformId {
    Policy(String),
    Model(String),
    Window(String),
    Matcher(String),
    MergeSpec,
    Invariant(String),
    BreakGlass { reason: String },
}

// ─────────────────────────────────────────────────────────────────────────────
// Edge statistics + merge semantics (single source of truth for both the
// projection and the GraphStore drivers)
// ─────────────────────────────────────────────────────────────────────────────

pub const EXEMPLAR_TRACE_ID_CAP: usize = 8;
pub const RECEIPT_DIGEST_CAP: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgeStats {
    pub count: u64,
    pub first_seen: String,
    pub last_seen: String,
    pub exemplar_trace_ids: Vec<String>,
    pub sampled: bool,
}

impl EdgeStats {
    /// `ts` MUST be canonical fixed-width RFC 3339 UTC
    /// (`time::format_rfc3339_ms`) — lexical comparison is chronological only
    /// under that invariant, and merge correctness depends on it.
    pub fn first(ts: &str, trace_id: &TraceId, sampled: bool) -> Self {
        EdgeStats {
            count: 1,
            first_seen: ts.to_owned(),
            last_seen: ts.to_owned(),
            exemplar_trace_ids: vec![trace_id.clone()],
            sampled,
        }
    }

    /// Absorb one execution (dedupe-then-truncate exemplars).
    pub fn absorb(&mut self, ts: &str, trace_id: &TraceId, sampled: bool) {
        self.count += 1;
        if ts < self.first_seen.as_str() {
            self.first_seen = ts.to_owned();
        }
        if ts > self.last_seen.as_str() {
            self.last_seen = ts.to_owned();
        }
        if !self.exemplar_trace_ids.iter().any(|t| t == trace_id) {
            self.exemplar_trace_ids.push(trace_id.clone());
            self.exemplar_trace_ids.truncate(EXEMPLAR_TRACE_ID_CAP);
        }
        self.sampled |= sampled;
    }

    /// Merge another edge's stats (dedupe-then-truncate exemplars).
    pub fn merge(&mut self, other: &EdgeStats) {
        self.count += other.count;
        if other.first_seen < self.first_seen {
            self.first_seen = other.first_seen.clone();
        }
        if other.last_seen > self.last_seen {
            self.last_seen = other.last_seen.clone();
        }
        for t in &other.exemplar_trace_ids {
            if !self.exemplar_trace_ids.contains(t) {
                self.exemplar_trace_ids.push(t.clone());
            }
        }
        self.exemplar_trace_ids.truncate(EXEMPLAR_TRACE_ID_CAP);
        self.sampled |= other.sampled;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineageEdge {
    pub edge_type: EdgeType,
    pub src: NodeId,
    pub dst: NodeId,
    pub authority: Authority,
    pub policy_version: PolicyVersion,
    pub purpose: Purpose,
    pub destination: Destination,
    pub driver: DriverRef,
    pub transformation: TransformId,
    pub completeness: FlowCompleteness,
    pub stats: EdgeStats,
    pub receipt_digests: Vec<[u8; 32]>,
}

impl LineageEdge {
    /// Key dimensions (RFC 0026 §6.2, as amended): `(edge_type, src, dst,
    /// policy_version, purpose, authority)`. Purpose and authority are key
    /// dimensions, not mergeable metadata — worked queries filter by purpose,
    /// and MP-7 forbids merging trust levels. Per-key metadata is therefore
    /// immutable; per-observation variance (completeness, degraded) remains
    /// queryable via `provenance_of` chain replay.
    pub fn key(&self) -> crate::store::EdgeKey {
        crate::store::EdgeKey {
            edge_type: self.edge_type,
            src: self.src.clone(),
            dst: self.dst.clone(),
            policy_version: self.policy_version.clone(),
            purpose: self.purpose.0.clone(),
            authority: self.authority,
        }
    }

    /// Build one edge from a single observation and one of its sources.
    /// `ts` is the audit entry's canonical RFC 3339 timestamp.
    pub fn from_observation(obs: &LineageObservation, source: &EntityRef, ts: &str) -> Self {
        LineageEdge {
            edge_type: obs.edge_type,
            src: source.node.clone(),
            dst: obs.sink.node.clone(),
            authority: obs.authority,
            policy_version: obs.policy_version.clone(),
            purpose: obs.purpose.clone(),
            destination: obs.destination.clone(),
            driver: obs.driver.clone(),
            transformation: obs.transformation.clone(),
            completeness: obs.completeness,
            stats: EdgeStats::first(ts, &obs.trace_id, obs.sampled),
            receipt_digests: vec![obs.receipt_digest],
        }
    }

    /// Merge an incoming edge with an identical key. Keep-first metadata
    /// (deterministic under chain-ordered input); stats merge; digests
    /// dedupe-then-truncate.
    pub fn merge_edge(&mut self, incoming: &LineageEdge) {
        debug_assert_eq!(
            self.key(),
            incoming.key(),
            "merge_edge requires identical EdgeKey"
        );
        self.stats.merge(&incoming.stats);
        for d in &incoming.receipt_digests {
            if !self.receipt_digests.contains(d) {
                self.receipt_digests.push(*d);
            }
        }
        self.receipt_digests.truncate(RECEIPT_DIGEST_CAP);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests — merge semantics (shared observation helper lives in
// collector::test_support to keep Destination naming in one place)
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use crate::collector::test_support::sample_observation;
    use crate::{Authority, LineageEdge, EXEMPLAR_TRACE_ID_CAP};

    #[test]
    fn merge_edge_accumulates_stats_and_dedupes_exemplars() {
        let a = sample_observation(Authority::KernelExecution, "fraud_monitoring", "trace-1");
        let mut b = sample_observation(Authority::KernelExecution, "fraud_monitoring", "trace-2");
        b.receipt_digest = [9u8; 32];

        let src = a.sources.first().expect("sample has a source").clone();
        let ts = "2025-11-27T10:15:32.114Z";

        let mut edge = LineageEdge::from_observation(&a, &src, ts);
        let incoming = LineageEdge::from_observation(&b, &src, "2025-11-27T10:16:32.114Z");
        edge.merge_edge(&incoming);

        assert_eq!(edge.stats.count, 2);
        assert_eq!(edge.stats.first_seen, ts);
        assert_eq!(edge.stats.last_seen, "2025-11-27T10:16:32.114Z");
        assert_eq!(edge.stats.exemplar_trace_ids.len(), 2);
        assert_eq!(edge.receipt_digests.len(), 2);
    }

    #[test]
    fn exemplar_cap_is_enforced_with_dedupe() {
        let mut a = sample_observation(Authority::KernelExecution, "fraud_monitoring", "trace-0");
        let src = a.sources.first().expect("sample has a source").clone();
        let ts = "2025-11-27T10:15:32.114Z";
        let mut edge = LineageEdge::from_observation(&a, &src, ts);

        for i in 1..=(EXEMPLAR_TRACE_ID_CAP as u64 + 4) {
            a.trace_id = format!("trace-{i}");
            let incoming = LineageEdge::from_observation(&a, &src, ts);
            edge.merge_edge(&incoming);
        }
        assert_eq!(edge.stats.exemplar_trace_ids.len(), EXEMPLAR_TRACE_ID_CAP);
    }

    #[test]
    fn authority_never_merges_mp7() {
        let exec = sample_observation(Authority::KernelExecution, "fraud_monitoring", "trace-1");
        let glass = sample_observation(Authority::BreakGlass, "fraud_monitoring", "trace-2");
        let src = exec.sources.first().expect("sample has a source").clone();
        let ts = "2025-11-27T10:15:32.114Z";

        let e1 = LineageEdge::from_observation(&exec, &src, ts);
        let e2 = LineageEdge::from_observation(&glass, &src, ts);
        assert_ne!(e1.key(), e2.key(), "authority is a key dimension");
        assert_ne!(e1.key().authority, e2.key().authority);
    }

    #[test]
    fn purpose_is_a_key_dimension() {
        let a = sample_observation(Authority::KernelExecution, "fraud_monitoring", "trace-1");
        let b = sample_observation(Authority::KernelExecution, "billing", "trace-2");
        let src = a.sources.first().expect("sample has a source").clone();
        let ts = "2025-11-27T10:15:32.114Z";

        let e1 = LineageEdge::from_observation(&a, &src, ts);
        let e2 = LineageEdge::from_observation(&b, &src, ts);
        assert_ne!(e1.key(), e2.key());
    }
}
