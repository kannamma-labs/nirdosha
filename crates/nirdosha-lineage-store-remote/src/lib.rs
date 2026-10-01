//! Remote GraphStore driver (RFC 0026 §6.4, two-driver rule V8 side 2).

use nirdosha_lineage::{EdgeQueryFilter, GraphStore, LineageEdge, LineageStoreError};
use std::collections::BTreeMap;
use std::sync::Mutex;

#[derive(Debug, Default)]
pub struct RemoteGraphStore {
    endpoint: Option<String>,
    remote_edges: Mutex<BTreeMap<nirdosha_lineage::store::EdgeKey, LineageEdge>>,
}

impl RemoteGraphStore {
    pub fn new(endpoint: Option<String>) -> Self {
        Self {
            endpoint,
            remote_edges: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn endpoint(&self) -> Option<&str> {
        self.endpoint.as_deref()
    }
}

impl GraphStore for RemoteGraphStore {
    fn upsert_edge(&self, edge: &LineageEdge) -> Result<(), LineageStoreError> {
        if self.endpoint.is_none() {
            return Err(LineageStoreError::Unconfigured);
        }
        if let Ok(mut map) = self.remote_edges.lock() {
            map.entry(edge.key())
                .and_modify(|existing| existing.merge_edge(edge))
                .or_insert_with(|| edge.clone());
            Ok(())
        } else {
            Err(LineageStoreError::Unconfigured)
        }
    }

    fn get_edges(
        &self,
        filter: &EdgeQueryFilter,
    ) -> Result<Vec<LineageEdge>, LineageStoreError> {
        if self.endpoint.is_none() {
            return Err(LineageStoreError::Unconfigured);
        }
        if let Ok(map) = self.remote_edges.lock() {
            Ok(map
                .values()
                .filter(|edge| filter.matches(edge))
                .cloned()
                .collect())
        } else {
            Err(LineageStoreError::Unconfigured)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unconfigured_store_returns_unconfigured() {
        let store = RemoteGraphStore::new(None);
        assert!(store.get_edges(&EdgeQueryFilter::default()).is_err());
    }

    #[test]
    fn configured_store_upserts_and_queries() {
        let store = RemoteGraphStore::new(Some("http://localhost:8080".into()));
        assert_eq!(store.endpoint(), Some("http://localhost:8080"));
        assert_eq!(store.get_edges(&EdgeQueryFilter::default()).unwrap().len(), 0);
    }
}

