//! Guard-down policy and reconciliation primitives.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Classification, Destination, Purpose};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardDownConfig {
    pub enabled: bool,
    pub max_classification: Classification,
    pub blocked_destinations: Vec<Destination>,
    pub blocked_purposes: Vec<Purpose>,
    pub ttl_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardDownConfigError { TtlTooLong(u64) }

impl GuardDownConfig {
    pub fn validate(&self) -> Result<(), GuardDownConfigError> {
        if self.ttl_seconds > 60 { Err(GuardDownConfigError::TtlTooLong(self.ttl_seconds)) } else { Ok(()) }
    }
}

#[derive(Debug, Clone, Default)]
pub struct KillSwitch { enabled: bool }

impl KillSwitch {
    pub fn trip(&mut self) { self.enabled = true; }
    pub fn reset(&mut self) { self.enabled = false; }
    pub fn is_tripped(&self) -> bool { self.enabled }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DegradedReadAudit {
    pub trace_id: String,
    pub entity: String,
    pub policy_version: String,
    pub reason: String,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone)]
pub struct DegradedReadWriter { path: PathBuf }

impl DegradedReadWriter {
    pub fn new(path: impl Into<PathBuf>) -> Self { Self { path: path.into() } }
    pub fn append(&self, record: &DegradedReadAudit) -> Result<nirdosha_audit::audit_chain::AuditEntry, String> {
        Ok(nirdosha_audit::audit_chain::append_entry(&self.path, serde_json::to_value(record).map_err(|e| e.to_string())?, record.timestamp_ms))
    }
    pub fn path(&self) -> &Path { &self.path }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconciliationItem { pub trace_id: String, pub record: DegradedReadAudit, pub reconciled: bool }

#[derive(Debug, Default)]
pub struct ReconciliationInbox { items: Vec<ReconciliationItem> }

impl ReconciliationInbox {
    pub fn push(&mut self, record: DegradedReadAudit) { self.items.push(ReconciliationItem { trace_id: record.trace_id.clone(), record, reconciled: false }); }
    pub fn reconcile(&mut self, trace_id: &str) -> bool {
        if let Some(item) = self.items.iter_mut().find(|item| item.trace_id == trace_id && !item.reconciled) { item.reconciled = true; true } else { false }
    }
    pub fn pending(&self) -> impl Iterator<Item = &ReconciliationItem> { self.items.iter().filter(|item| !item.reconciled) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ttl_is_bounded_and_kill_switch_is_independent() {
        let config = GuardDownConfig { enabled: true, max_classification: Classification::Internal, blocked_destinations: vec![], blocked_purposes: vec![], ttl_seconds: 61 };
        assert!(matches!(config.validate(), Err(GuardDownConfigError::TtlTooLong(61))));
        let mut switch = KillSwitch::default(); switch.trip(); assert!(switch.is_tripped()); switch.reset(); assert!(!switch.is_tripped());
    }
    #[test]
    fn inbox_reconciles_once() {
        let record = DegradedReadAudit { trace_id: "t".into(), entity: "e".into(), policy_version: "v".into(), reason: "down".into(), timestamp_ms: 1 };
        let mut inbox = ReconciliationInbox::default(); inbox.push(record); assert!(inbox.reconcile("t")); assert!(!inbox.reconcile("t")); assert_eq!(inbox.pending().count(), 0);
    }
}
