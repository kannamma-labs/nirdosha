//! RFC 0025 audit envelope and module-chain adapters.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::audit_chain::{self, AuditEntry, ChainError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuditRecordKind {
    Mutation,
    Decision,
    Lineage,
    BreakGlass,
    Delegation,
    Admin,
    Reconciliation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEnvelope {
    pub trace_id: String,
    pub ts: String,
    pub module: String,
    pub subject: String,
    pub action: String,
    pub resource: String,
    pub policy_versions: Vec<String>,
    pub decision: String,
    pub obligations: Vec<String>,
    pub kind: AuditRecordKind,
    pub content: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ModuleAuditChain {
    pub module: String,
    pub path: PathBuf,
}

impl ModuleAuditChain {
    pub fn new(module: impl Into<String>, path: impl Into<PathBuf>) -> Self { Self { module: module.into(), path: path.into() } }

    pub fn append(&self, envelope: &AuditEnvelope, timestamp_ms: u64) -> AuditEntry {
        assert_eq!(envelope.module, self.module, "envelope module must match chain module");
        audit_chain::append_entry(&self.path, serde_json::to_value(envelope).expect("audit envelope is serializable"), timestamp_ms)
    }

    pub fn verify(&self) -> Result<usize, ChainError> { audit_chain::verify_chain(&self.path) }
    pub fn path(&self) -> &Path { &self.path }
    /// Real entries, for a projection to read (T-11) -- `verify` alone
    /// only proves the chain is intact, it doesn't hand the entries back.
    pub fn entries(&self) -> Vec<AuditEntry> { audit_chain::read_entries(&self.path) }
}

#[derive(Debug, Default)]
pub struct ChainReconciler;

impl ChainReconciler {
    /// Merge entries deterministically by timestamp, module, then local seq.
    /// The resulting vector is a projection input; entries remain unchanged.
    pub fn reconcile(&self, chains: impl IntoIterator<Item = Vec<AuditEntry>>) -> Vec<AuditEntry> {
        let mut entries = chains.into_iter().flatten().collect::<Vec<_>>();
        entries.sort_by(|left, right| {
            let left_module = envelope_module(left).unwrap_or_default();
            let right_module = envelope_module(right).unwrap_or_default();
            left.timestamp.cmp(&right.timestamp).then_with(|| left_module.cmp(&right_module)).then_with(|| left.seq.cmp(&right.seq))
        });
        entries
    }
}

fn envelope_module(entry: &AuditEntry) -> Option<String> {
    entry.content.get("module").and_then(serde_json::Value::as_str).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(module: &str, trace_id: &str) -> AuditEnvelope {
        AuditEnvelope { trace_id: trace_id.into(), ts: "2026-09-19T00:00:00.000Z".into(), module: module.into(), subject: "u".into(), action: "read".into(), resource: "e".into(), policy_versions: vec!["v1".into()], decision: "allow".into(), obligations: vec![], kind: AuditRecordKind::Decision, content: serde_json::json!({}) }
    }

    #[test]
    fn kind_serializes_lowercase() { assert_eq!(serde_json::to_value(AuditRecordKind::Lineage).unwrap(), "lineage"); }

    #[test]
    fn append_verify_and_reconcile() {
        let root = std::env::temp_dir().join(format!("nirdosha-envelope-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let first = ModuleAuditChain::new("a", root.join("a.jsonl"));
        let second = ModuleAuditChain::new("b", root.join("b.jsonl"));
        let a = first.append(&envelope("a", "a1"), 20);
        let b = second.append(&envelope("b", "b1"), 10);
        assert_eq!(first.verify(), Ok(1));
        assert_eq!(second.verify(), Ok(1));
        let reconciled = ChainReconciler.reconcile(vec![vec![a], vec![b]]);
        assert_eq!(reconciled[0].timestamp, 10);
        let _ = std::fs::remove_dir_all(root);
    }
}