//! The general rule evaluator: one `evaluate` method covering all three
//! deterministic rule types (`AmountThreshold`, `Velocity`,
//! `GeoMismatch`), consulted identically by realtime processing and by
//! `crate::batch` -- "the same evaluator, not a second rule engine,"
//! literally the same [`RuleEngine`] instance/type either way (see
//! `tests/monitoring_rules_e2e.rs`'s own realtime-then-batch-replay
//! assertion for the proof, not just the claim).
//!
//! This generalizes (does not replace) `crate::rules::VelocityWindowStore`:
//! that type and `crate::rules::RuleCatalog` remain exactly as they were
//! for the crate's original, already-proven realtime/batch slice; nothing
//! here changes their behavior or call sites. `RuleEngine` is what a
//! [`crate::rule_model::RuleDefinition`] (the governed, versioned,
//! approved-before-use rule) is evaluated against once it has a real
//! lifecycle.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::event::{Direction, TransactionEvent};
use crate::rule_model::{RuleDefinition, RuleType};

#[derive(Debug, Clone, PartialEq)]
pub struct RuleEvaluationResult {
    pub rule_id: String,
    pub rule_version: u32,
    pub matched_event_ids: Vec<String>,
    pub evaluated_at_ms: u64,
    pub decision: &'static str,
    pub severity: String,
    /// A stable pointer back to this decision (`rule:<id>:v<version>:<ts>`)
    /// -- "evidence references." The real evidence write happens where a
    /// caller turns this into an `alert.create` capability call (see
    /// `crate::alert::CtmsAlertGatewayV1`); this evaluator is a pure
    /// function plus (for `Velocity`) bounded window state, it does not
    /// write evidence itself.
    pub evidence_reference: String,
}

/// Per-`(rule_id, key)` windowed observation state for `Velocity` rules --
/// the only rule type that needs to remember anything across calls.
/// `AmountThreshold`/`GeoMismatch` are pure, single-event checks.
#[derive(Default)]
pub struct RuleEngine {
    velocity_windows: Mutex<HashMap<String, Vec<(u64, u64, String)>>>,
}

impl RuleEngine {
    pub fn new() -> Self {
        Self::default()
    }

    fn result(rule: &RuleDefinition, matched_event_ids: Vec<String>, now_ms: u64) -> RuleEvaluationResult {
        RuleEvaluationResult {
            rule_id: rule.rule_id.clone(),
            rule_version: rule.version,
            matched_event_ids,
            evaluated_at_ms: now_ms,
            decision: "match",
            severity: rule.severity.clone(),
            evidence_reference: format!("rule:{}:v{}:{now_ms}", rule.rule_id, rule.version),
        }
    }

    /// Evaluates one already-normalized `event` against `rule`. `rule`
    /// should be a snapshot from `RuleStore::active_snapshot`/`list_active`
    /// -- this function does not itself check `rule.status`, that's the
    /// caller's job (mirroring how `crate::rules::VelocityWindowStore`
    /// doesn't validate its own config either).
    pub fn evaluate(&self, rule: &RuleDefinition, event: &TransactionEvent, now_ms: u64) -> Option<RuleEvaluationResult> {
        match &rule.rule_type {
            RuleType::AmountThreshold { min_amount_minor } => {
                if event.amount_minor >= *min_amount_minor {
                    Some(Self::result(rule, vec![event.event_id.clone()], now_ms))
                } else {
                    None
                }
            }
            RuleType::GeoMismatch { allowed_jurisdictions } => {
                if !allowed_jurisdictions.iter().any(|j| j == &event.jurisdiction) {
                    Some(Self::result(rule, vec![event.event_id.clone()], now_ms))
                } else {
                    None
                }
            }
            RuleType::Velocity { window_ms, min_count, min_total_minor } => {
                if event.direction != Direction::Debit {
                    return None;
                }
                let key = format!("{}:{}", rule.rule_id, event.customer_id);
                let mut windows = self.velocity_windows.lock().expect("rule engine velocity window lock poisoned");
                let entries = windows.entry(key).or_default();
                if !entries.iter().any(|(_, _, id)| id == &event.event_id) {
                    entries.push((event.event_time_ms, event.amount_minor, event.event_id.clone()));
                }
                let window_end_ms = entries.iter().map(|(t, _, _)| *t).max().unwrap_or(event.event_time_ms);
                let window_start_ms = window_end_ms.saturating_sub(*window_ms);
                let in_window: Vec<&(u64, u64, String)> = entries.iter().filter(|(t, _, _)| *t >= window_start_ms && *t <= window_end_ms).collect();
                let count = in_window.len();
                let total: u64 = in_window.iter().map(|(_, amount, _)| amount).sum();
                if count >= *min_count && total >= *min_total_minor {
                    Some(Self::result(rule, in_window.iter().map(|(_, _, id)| id.clone()).collect(), now_ms))
                } else {
                    None
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::EventType;
    use crate::rule_model::RuleStatus;

    fn debit_event(event_id: &str, customer_id: &str, event_time_ms: u64, amount_minor: u64, jurisdiction: &str) -> TransactionEvent {
        TransactionEvent {
            event_id: event_id.to_string(),
            transaction_id: format!("txn-{event_id}"),
            event_type: EventType::CardDebit,
            customer_id: customer_id.to_string(),
            account_id: "acct-1".to_string(),
            direction: Direction::Debit,
            amount_minor,
            currency: "INR".to_string(),
            event_time_ms,
            source_system: "card-network-x".to_string(),
            source_version: "v1".to_string(),
            correction_of: None,
            jurisdiction: jurisdiction.to_string(),
            ingestion_time_ms: 0,
            partition: 0,
            offset: 0,
        }
    }

    fn base_rule(rule_type: RuleType) -> RuleDefinition {
        RuleDefinition {
            rule_id: "r1".to_string(),
            version: 1,
            name: "test".to_string(),
            description: "test".to_string(),
            rule_type,
            currency: "INR".to_string(),
            jurisdiction: "IN".to_string(),
            severity: "high".to_string(),
            effective_from_ms: 0,
            effective_until_ms: None,
            status: RuleStatus::Enabled,
            author: "priya".to_string(),
            approved_by: Some("arjun".to_string()),
            created_at_ms: 0,
        }
    }

    #[test]
    fn amount_threshold_matches_at_or_above_the_minimum() {
        let engine = RuleEngine::new();
        let rule = base_rule(RuleType::AmountThreshold { min_amount_minor: 100_000 });
        assert!(engine.evaluate(&rule, &debit_event("e1", "cust-1", 0, 99_999, "IN"), 0).is_none());
        let matched = engine.evaluate(&rule, &debit_event("e2", "cust-1", 0, 100_000, "IN"), 0).unwrap();
        assert_eq!(matched.matched_event_ids, vec!["e2"]);
        assert_eq!(matched.evidence_reference, "rule:r1:v1:0");
    }

    #[test]
    fn geo_mismatch_matches_a_jurisdiction_outside_the_allowlist() {
        let engine = RuleEngine::new();
        let rule = base_rule(RuleType::GeoMismatch { allowed_jurisdictions: vec!["IN".to_string(), "SG".to_string()] });
        assert!(engine.evaluate(&rule, &debit_event("e1", "cust-1", 0, 1_000, "IN"), 0).is_none());
        let matched = engine.evaluate(&rule, &debit_event("e2", "cust-1", 0, 1_000, "RU"), 0).unwrap();
        assert_eq!(matched.matched_event_ids, vec!["e2"]);
    }

    #[test]
    fn velocity_matches_once_count_and_total_thresholds_are_crossed() {
        let engine = RuleEngine::new();
        let rule = base_rule(RuleType::Velocity { window_ms: 86_400_000, min_count: 3, min_total_minor: 300_000 });
        for i in 0..2 {
            assert!(engine.evaluate(&rule, &debit_event(&format!("e{i}"), "cust-1", 1_000 * i, 100_000, "IN"), 0).is_none());
        }
        let matched = engine.evaluate(&rule, &debit_event("e2", "cust-1", 2_000, 100_000, "IN"), 0).unwrap();
        assert_eq!(matched.matched_event_ids.len(), 3);
    }

    #[test]
    fn two_different_rules_keep_independent_velocity_windows_for_the_same_customer() {
        let engine = RuleEngine::new();
        let mut rule_a = base_rule(RuleType::Velocity { window_ms: 86_400_000, min_count: 2, min_total_minor: 1, });
        rule_a.rule_id = "rule-a".to_string();
        let mut rule_b = rule_a.clone();
        rule_b.rule_id = "rule-b".to_string();

        engine.evaluate(&rule_a, &debit_event("e0", "cust-1", 0, 100, "IN"), 0);
        // rule_b has seen nothing yet -- one event is not enough for min_count 2.
        assert!(engine.evaluate(&rule_b, &debit_event("e0", "cust-1", 0, 100, "IN"), 0).is_none());
    }
}
