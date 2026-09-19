//! Chain → graph projection (RFC 0026 §6.3; MD-1, MD-2).
//!
//! `project_observations` is the pure, deterministic core: same observations
//! in the same chain order → same edges. `project_edges` is the thin adapter
//! over audit-chain entries. Degraded observations still produce edges (data
//! did flow); the degraded flag rides the chain and surfaces via
//! `provenance_of`. Zero-source observations are recorded in the chain but
//! produce no edge — edges require a known source node; the external-ingress
//! node convention lands in Phase 2 (checklist).

use std::collections::BTreeMap;

use nirdosha_audit::audit_chain::AuditEntry;

use crate::collector::LineageObservation;
use crate::store::EdgeKey;
use crate::{time, LineageEdge};

pub fn project_observations(items: &[(LineageObservation, u64)]) -> Vec<LineageEdge> {
    let mut acc: BTreeMap<EdgeKey, LineageEdge> = BTreeMap::new();
    for (obs, ts_ms) in items {
        let ts = time::format_rfc3339_ms(*ts_ms);
        for src in &obs.sources {
            let edge = LineageEdge::from_observation(obs, src, &ts);
            match acc.entry(edge.key()) {
                std::collections::btree_map::Entry::Occupied(mut slot) => {
                    slot.get_mut().merge_edge(&edge);
                }
                std::collections::btree_map::Entry::Vacant(slot) => {
                    slot.insert(edge);
                }
            }
        }
    }
    acc.into_values().collect()
}

/// Adapter over audit-chain entries. `entry.timestamp` is unix milliseconds
/// (pinned contract with `nirdosha_audit` — see checklist verify note).
pub fn project_edges(entries: &[AuditEntry]) -> Vec<LineageEdge> {
    let items = entries
        .iter()
        .filter_map(|e| {
            LineageObservation::from_audit_content(&e.content).map(|obs| (obs, e.timestamp))
        })
        .collect::<Vec<_>>();
    project_observations(&items)
}

#[cfg(test)]
mod tests {
    use crate::collector::test_support::sample_observation;
    use crate::Authority;

    use super::*;

    #[test]
    fn same_key_merges_different_authority_does_not() {
        let exec = sample_observation(Authority::KernelExecution, "fraud_monitoring", "trace-1");
        let mut exec2 = sample_observation(Authority::KernelExecution, "fraud_monitoring", "trace-1");
        exec2.receipt_digest = [9u8; 32];
        let glass = sample_observation(Authority::BreakGlass, "fraud_monitoring", "trace-3");

        let edges = project_observations(&[
            (exec.clone(), 1_000),
            (exec2, 2_000),
            (glass, 3_000),
        ]);

        // exec + exec2 share (key incl. authority) → 1 merged edge, count 2.
        // glass differs in authority → separate edge, count 1.
        assert_eq!(edges.len(), 2);
        let merged = edges
            .iter()
            .find(|e| e.authority == Authority::KernelExecution)
            .expect("kernel edge");
        assert_eq!(merged.stats.count, 2);
        assert_eq!(merged.stats.first_seen, "1970-01-01T00:00:01.000Z");
        assert_eq!(merged.stats.last_seen, "1970-01-01T00:00:02.000Z");
        let bg = edges
            .iter()
            .find(|e| e.authority == Authority::BreakGlass)
            .expect("break-glass edge");
        assert_eq!(bg.stats.count, 1, "MP-7: trust levels never merge");
    }

    #[test]
    fn projection_is_deterministic_under_chain_order() {
        let a = sample_observation(Authority::KernelExecution, "fraud_monitoring", "trace-1");
        let b = sample_observation(Authority::KernelExecution, "fraud_monitoring", "trace-2");
        let items = [(a, 1_000), (b, 2_000)];
        let e1 = project_observations(&items);
        let e2 = project_observations(&items);
        assert_eq!(e1, e2);
    }
}
