//! V10 replay engine for graph & audit alignment (RFC 0026 §8).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum V10FindingKind {
    Absent,
    Dormant,
    Undeclared,
    BreakGlassUnreconciled,
    IssuedUnconsumed,
    UnissuedConsumption,
    DegradedWindowPending,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct V10Finding {
    pub kind: V10FindingKind,
    pub node_or_edge: String,
    pub details: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct V10Report {
    pub findings: Vec<V10Finding>,
}

pub struct V10ReplayChecker;

impl V10ReplayChecker {
    pub fn check_audit_chain(_chain_entries: &[serde_json::Value]) -> V10Report {
        V10Report { findings: Vec::new() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v10_checker_returns_report() {
        let report = V10ReplayChecker::check_audit_chain(&[]);
        assert!(report.findings.is_empty());
    }
}
