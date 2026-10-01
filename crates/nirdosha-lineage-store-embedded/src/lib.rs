//! Embedded GraphStore driver (RFC 0026 §6.4, two-driver rule V8 side 1).
//!
//! In-process store over a `BTreeMap<EdgeKey, LineageEdge>`. Merge is a single
//! call to `LineageEdge::merge_edge` — the one source of truth for stats
//! semantics, shared with the projection. Suitable for a single-process
//! projection cache; the record of truth remains the audit chain (MD-1).

use std::collections::BTreeMap;
use std::sync::Mutex;

use nirdosha_lineage::{EdgeKey, EdgeQueryFilter, GraphStore, LineageEdge, LineageStoreError};

#[derive(Default)]
pub struct EmbeddedGraphStore {
    edges: Mutex<BTreeMap<EdgeKey, LineageEdge>>,
}

impl EmbeddedGraphStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl GraphStore for EmbeddedGraphStore {
    fn upsert_edge(&self, edge: &LineageEdge) -> Result<(), LineageStoreError> {
        let mut edges = self
            .edges
            .lock()
            .map_err(|_| LineageStoreError::Unsupported { reason: "store lock poisoned".into() })?;
        let key = edge.key();
        match edges.entry(key) {
            std::collections::btree_map::Entry::Occupied(mut slot) => {
                slot.get_mut().merge_edge(edge);
            }
            std::collections::btree_map::Entry::Vacant(slot) => {
                slot.insert(edge.clone());
            }
        }
        Ok(())
    }

    fn get_edges(
        &self,
        filter: &EdgeQueryFilter,
    ) -> Result<Vec<LineageEdge>, LineageStoreError> {
        let edges = self
            .edges
            .lock()
            .map_err(|_| LineageStoreError::Unsupported { reason: "store lock poisoned".into() })?;
        Ok(edges
            .values()
            .filter(|e| filter.matches(e))
            .take(filter.limit.unwrap_or(u64::MAX) as usize)
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use nirdosha_lineage::{
        project_observations, Authority,
        collector::{LineageObservation, PlanFacts},
        EdgeType, FlowCompleteness, TransformId, DriverRef,
    };
    use nirdosha_guard_core::{Destination, LineageEntity, PolicyVersion, Purpose};

    use super::*;

    fn obs(trace: &str) -> LineageObservation {
        let mut o = LineageObservation::from_context(
            "svc:features",
            &trace.to_owned(),
            "user-42",
            "tenant-1",
            PolicyVersion::from("2025.11.4"),
            Purpose("fraud_monitoring".to_owned()),
            Destination::ApiClient,
            [7u8; 32],
            PlanFacts {
                edge_type: EdgeType::Read,
                transformation: TransformId::Window("velocity_1h".to_owned()),
                driver: DriverRef {
                    port: "StreamPort".to_owned(),
                    vendor: "kafka".to_owned(),
                    version: "0.12.0".to_owned(),
                },
                authority: Authority::KernelExecution,
                completeness: FlowCompleteness::Full,
                sampled: false,
                degraded: false,
                sink: LineageEntity { entity: "txn_events".to_owned(), keys: vec![] },
            },
        );
        o.enrich(&nirdosha_guard_core::LineageFacts {
            sources: vec![LineageEntity { entity: "txn_events".to_owned(), keys: vec![] }],
            sink_keys: vec![],
        });
        o
    }

    #[test]
    fn upsert_merges_through_merge_edge() {
        let store = EmbeddedGraphStore::new();
        let edges = project_observations(&[(obs("t1"), 1_000), (obs("t2"), 2_000)]);
        assert_eq!(edges.len(), 1);
        for e in &edges {
            store.upsert_edge(e).unwrap();
        }
        // Re-project and upsert the fresh projection over overlapping window:
        // same key → stats merge again through the single merge path.
        let again = project_observations(&[(obs("t3"), 3_000)]);
        store.upsert_edge(&again[0]).unwrap();

        let got = store.get_edges(&EdgeQueryFilter::default()).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].stats.count, 3);
        assert_eq!(got[0].stats.first_seen, "1970-01-01T00:00:01.000Z");
        assert_eq!(got[0].stats.last_seen, "1970-01-01T00:00:03.000Z");
    }

    #[test]
    fn get_edges_filters() {
        let store = EmbeddedGraphStore::new();
        let edges = project_observations(&[(obs("t1"), 1_000)]);
        store.upsert_edge(&edges[0]).unwrap();

        let hit = EdgeQueryFilter { edge_type: Some(EdgeType::Read), ..Default::default() };
        let miss = EdgeQueryFilter { edge_type: Some(EdgeType::ScoredBy), ..Default::default() };
        assert_eq!(store.get_edges(&hit).unwrap().len(), 1);
        assert!(store.get_edges(&miss).unwrap().is_empty());
    }
}
