//! The swappable GraphStore port (RFC 0026 §6.4). Two-driver rule (V8) applies:
//! `nirdosha-lineage-store-embedded` and `nirdosha-lineage-store-remote` both
//! implement this trait. MD-1: the store is a projection cache — never a
//! second source of truth.

use crate::{Authority, EdgeType, LineageEdge, NodeId, PolicyVersion};

/// Merge identity for type-edges (RFC 0026 §6.2, as amended): purpose and
/// authority are key dimensions, so per-key metadata is immutable and trust
/// levels can never merge (MP-7).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EdgeKey {
    pub edge_type: EdgeType,
    pub src: NodeId,
    pub dst: NodeId,
    pub policy_version: PolicyVersion,
    /// Canonical purpose code (`Purpose::0`), keyed as a string so the key
    /// needs no Ord/derive changes in guard-core.
    pub purpose: String,
    pub authority: Authority,
}

#[derive(Debug, Clone, Default)]
pub struct EdgeQueryFilter {
    pub edge_type: Option<EdgeType>,
    pub policy_version: Option<PolicyVersion>,
    pub purpose: Option<String>,
    pub authority: Option<Authority>,
    pub src: Option<NodeId>,
    pub limit: Option<u64>,
}

impl EdgeQueryFilter {
    pub fn matches(&self, edge: &LineageEdge) -> bool {
        if let Some(t) = &self.edge_type {
            if edge.edge_type != *t {
                return false;
            }
        }
        if let Some(pv) = &self.policy_version {
            if &edge.policy_version != pv {
                return false;
            }
        }
        if let Some(p) = &self.purpose {
            if &edge.purpose.0 != p {
                return false;
            }
        }
        if let Some(a) = &self.authority {
            if edge.authority != *a {
                return false;
            }
        }
        if let Some(s) = &self.src {
            if &edge.src != s {
                return false;
            }
        }
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineageStoreError {
    /// Phase-1 remote driver: transport lands in Phase 2 (structural
    /// two-driver placeholder; does NOT count toward V8 until it does work).
    Unconfigured,
    Unsupported { reason: String },
}

impl std::fmt::Display for LineageStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LineageStoreError::Unconfigured => write!(f, "graph store is unconfigured"),
            LineageStoreError::Unsupported { reason } => {
                write!(f, "graph store unsupported operation: {reason}")
            }
        }
    }
}

impl std::error::Error for LineageStoreError {}

/// Sync by deliberate Phase-1 decision: both Phase-1 drivers are in-process
/// or stubs; the RFC's `execute().await` belongs to the guard query plan
/// (Phase 2), not this port. Widening later is additive.
pub trait GraphStore: Send + Sync {
    /// Upsert expects chain-ordered edges (the projection service's contract);
    /// merge semantics come from `LineageEdge::merge_edge` — keep-first
    /// metadata, merged stats.
    fn upsert_edge(&self, edge: &LineageEdge) -> Result<(), LineageStoreError>;
    fn get_edges(&self, filter: &EdgeQueryFilter)
        -> Result<Vec<LineageEdge>, LineageStoreError>;
}

#[cfg(test)]
mod tests {
    use crate::collector::test_support::sample_observation;
    use crate::{Authority, LineageEdge};

    use super::*;

    #[test]
    fn filter_matches_on_all_dimensions() {
        let obs = sample_observation(Authority::KernelExecution, "fraud_monitoring", "trace-1");
        let src = obs.sources.first().expect("sample has a source").clone();
        let edge = LineageEdge::from_observation(&obs, &src, "2025-11-27T10:15:32.114Z");

        assert!(EdgeQueryFilter { authority: Some(Authority::KernelExecution), ..Default::default() }
            .matches(&edge));
        assert!(!EdgeQueryFilter { authority: Some(Authority::BreakGlass), ..Default::default() }
            .matches(&edge));
        assert!(EdgeQueryFilter { purpose: Some("fraud_monitoring".to_owned()), ..Default::default() }
            .matches(&edge));
        assert!(!EdgeQueryFilter { purpose: Some("billing".to_owned()), ..Default::default() }
            .matches(&edge));
    }
}
