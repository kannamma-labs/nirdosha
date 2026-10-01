//! Versioned policy snapshots for replay and commit-time revalidation.

use serde::{Deserialize, Serialize};

use crate::PolicyVersion;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicySnapshot { pub version: PolicyVersion, pub records_hash: String, pub issued_at: u64 }

pub trait PolicyStore {
    fn current(&self) -> PolicySnapshot;
    fn replace(&mut self, snapshot: PolicySnapshot);
    fn on_change(&mut self, callback: Box<dyn Fn(&PolicySnapshot) + Send + Sync>);
}

pub struct InMemoryPolicyStore { current: PolicySnapshot, callbacks: Vec<Box<dyn Fn(&PolicySnapshot) + Send + Sync>> }

impl std::fmt::Debug for InMemoryPolicyStore { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.debug_struct("InMemoryPolicyStore").field("current", &self.current).field("callbacks", &self.callbacks.len()).finish() } }

impl InMemoryPolicyStore { pub fn new(snapshot: PolicySnapshot) -> Self { Self { current: snapshot, callbacks: Vec::new() } } }

impl PolicyStore for InMemoryPolicyStore {
    fn current(&self) -> PolicySnapshot { self.current.clone() }
    fn replace(&mut self, snapshot: PolicySnapshot) { self.current = snapshot.clone(); for callback in &self.callbacks { callback(&snapshot); } }
    fn on_change(&mut self, callback: Box<dyn Fn(&PolicySnapshot) + Send + Sync>) { self.callbacks.push(callback); }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacement_notifies_callbacks() { let mut store = InMemoryPolicyStore::new(PolicySnapshot { version: "v1".into(), records_hash: "h1".into(), issued_at: 1 }); store.on_change(Box::new(|snapshot| assert_eq!(snapshot.version, "v2"))); store.replace(PolicySnapshot { version: "v2".into(), records_hash: "h2".into(), issued_at: 2 }); assert_eq!(store.current().version, "v2"); }
}
