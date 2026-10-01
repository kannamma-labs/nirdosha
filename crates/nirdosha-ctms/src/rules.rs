//! One deterministic, human-authored rule shape -- `velocity_24h` being
//! this crate's own reference instance -- exactly as the design note asks
//! for the first slice ("these rules can be deterministic and
//! human-authored; the model-influence layer is not necessary for the
//! first CTMS demo").
//!
//! **Thresholds are policy data, not a Rust constant.** A velocity
//! threshold (how many debits, how much money, what window) is exactly as
//! org/jurisdiction-specific as the case-workflow role routing in
//! `crate::policy` -- a small credit union and a large bank do not share
//! one number. [`RuleCatalog::from_toml_str`] parses `[rule.<id>]` entries
//! (mirroring the design note's own sketch) into [`VelocityRuleConfig`]s;
//! [`RuleCatalog::default_v1`] is only this crate's own reference catalog,
//! not a value every deployment must use.
//!
//! [`VelocityWindowStore::observe_and_evaluate`] recomputes the window from
//! scratch on every observation rather than maintaining a running total,
//! so a late/out-of-order event (flagged by `crate::stream::StreamAdapter`
//! but not otherwise special-cased) is handled correctly for free: nothing
//! here depends on insertion order, only on each event's own
//! `event_time_ms` relative to the current window -- the design note's
//! `late_event_policy = "recompute"`, implemented literally rather than
//! merely declared.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::Deserialize;

use crate::event::{Direction, TransactionEvent};

#[derive(Debug, Clone, PartialEq)]
pub struct VelocityRuleConfig {
    pub rule_id: String,
    pub version: u32,
    pub window_ms: u64,
    pub min_debit_count: usize,
    pub min_debit_total_minor: u64,
    pub severity: String,
}

#[derive(Debug, Deserialize)]
struct RawRuleCatalogFile {
    #[serde(default)]
    rule: HashMap<String, RawVelocityRule>,
}

#[derive(Debug, Deserialize)]
struct RawVelocityRule {
    version: u32,
    window_hours: u64,
    min_debit_count: usize,
    min_debit_total_minor: u64,
    severity: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleCatalogError {
    Parse(String),
    EmptyField { rule_id: String, field: &'static str },
}

impl std::fmt::Display for RuleCatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RuleCatalogError::Parse(e) => write!(f, "rule catalog is invalid: {e}"),
            RuleCatalogError::EmptyField { rule_id, field } => write!(f, "rule `{rule_id}` field `{field}` must not be empty"),
        }
    }
}

/// This deployment's governed velocity-rule thresholds, loaded from data.
/// Not tied to any single rule id or count -- an org can define as many
/// `[rule.<id>]` entries as it wants, each with its own window/thresholds/
/// severity.
#[derive(Debug, Clone, PartialEq)]
pub struct RuleCatalog {
    velocity_rules: HashMap<String, VelocityRuleConfig>,
    /// `sha256:<hex>` of the exact TOML source this catalog was parsed
    /// from -- a batch run's "rule bundle hash" provenance field, the same
    /// idea as `PolicyBundle::bundle_hash`.
    catalog_hash: String,
}

impl RuleCatalog {
    pub fn from_toml_str(src: &str) -> Result<Self, RuleCatalogError> {
        use sha2::{Digest, Sha256};
        let raw: RawRuleCatalogFile = toml::from_str(src).map_err(|e| RuleCatalogError::Parse(e.to_string()))?;
        let catalog_hash = format!("sha256:{:x}", Sha256::digest(src.as_bytes()));
        let mut velocity_rules = HashMap::new();
        for (rule_id, r) in raw.rule {
            if r.severity.trim().is_empty() {
                return Err(RuleCatalogError::EmptyField { rule_id, field: "severity" });
            }
            velocity_rules.insert(
                rule_id.clone(),
                VelocityRuleConfig {
                    rule_id,
                    version: r.version,
                    window_ms: r.window_hours * 60 * 60 * 1000,
                    min_debit_count: r.min_debit_count,
                    min_debit_total_minor: r.min_debit_total_minor,
                    severity: r.severity,
                },
            );
        }
        Ok(Self { velocity_rules, catalog_hash })
    }

    pub fn catalog_hash(&self) -> &str {
        &self.catalog_hash
    }

    /// This slice's own governed reference rule, matching the design
    /// note's `[rule.velocity_24h]` sketch verbatim (`debit_count >= 5 AND
    /// debit_total_minor >= 500000` within a 24h window, severity
    /// `"high"`). A real deployment supplies its own TOML instead of
    /// calling this.
    pub fn default_v1() -> Self {
        Self::from_toml_str(DEFAULT_V1_TOML).expect("crate's own default rule catalog TOML must parse")
    }

    pub fn velocity_rule(&self, rule_id: &str) -> Option<&VelocityRuleConfig> {
        self.velocity_rules.get(rule_id)
    }
}

const DEFAULT_V1_TOML: &str = r#"
[rule.velocity_24h]
version = 1
window_hours = 24
min_debit_count = 5
min_debit_total_minor = 500000
severity = "high"
"#;

/// A rule firing -- not yet an [`crate::alert::Alert`]; `crate::alert`'s
/// gateway is the only thing allowed to turn this into one, keyed by
/// `(rule_id, rule_version, key, window_start_ms)` so a repeated
/// evaluation over the same window never double-alerts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertCandidate {
    pub rule_id: String,
    pub rule_version: u32,
    pub key: String,
    pub window_start_ms: u64,
    pub window_end_ms: u64,
    pub severity: String,
    pub matched_event_ids: Vec<String>,
    pub debit_count: usize,
    pub debit_total_minor: u64,
}

#[derive(Default)]
pub struct VelocityWindowStore {
    /// `customer_id` -> observed debit events `(event_time_ms,
    /// amount_minor, event_id)`, unsorted -- window membership is
    /// recomputed by filtering on `event_time_ms`, never by relying on
    /// vector order.
    debits_by_key: Mutex<HashMap<String, Vec<(u64, u64, String)>>>,
}

impl VelocityWindowStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ingests one event (a no-op for credits) and re-evaluates
    /// `cfg` over the full window ending at the newest `event_time_ms`
    /// this key has seen so far. Returns `Some(AlertCandidate)` exactly
    /// when the condition is met; the caller (the alert gateway's
    /// idempotent store) is what prevents a second, identical candidate
    /// for the same window from creating a second alert.
    pub fn observe_and_evaluate(&self, cfg: &VelocityRuleConfig, event: &TransactionEvent) -> Option<AlertCandidate> {
        if event.direction != Direction::Debit {
            return None;
        }
        let mut debits_by_key = self.debits_by_key.lock().expect("velocity window lock poisoned");
        let entries = debits_by_key.entry(event.customer_id.clone()).or_default();
        if !entries.iter().any(|(_, _, id)| id == &event.event_id) {
            entries.push((event.event_time_ms, event.amount_minor, event.event_id.clone()));
        }

        let window_end_ms = entries.iter().map(|(t, _, _)| *t).max().unwrap_or(event.event_time_ms);
        let window_start_ms = window_end_ms.saturating_sub(cfg.window_ms);
        let in_window: Vec<&(u64, u64, String)> =
            entries.iter().filter(|(t, _, _)| *t >= window_start_ms && *t <= window_end_ms).collect();
        let debit_count = in_window.len();
        let debit_total_minor: u64 = in_window.iter().map(|(_, amount, _)| amount).sum();

        if debit_count >= cfg.min_debit_count && debit_total_minor >= cfg.min_debit_total_minor {
            Some(AlertCandidate {
                rule_id: cfg.rule_id.to_string(),
                rule_version: cfg.version,
                key: event.customer_id.clone(),
                window_start_ms,
                window_end_ms,
                severity: cfg.severity.to_string(),
                matched_event_ids: in_window.iter().map(|(_, _, id)| id.clone()).collect(),
                debit_count,
                debit_total_minor,
            })
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::EventType;

    fn debit(event_id: &str, customer_id: &str, event_time_ms: u64, amount_minor: u64) -> TransactionEvent {
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
            jurisdiction: "IN".to_string(),
            ingestion_time_ms: 0,
            partition: 0,
            offset: 0,
        }
    }

    fn velocity_24h() -> VelocityRuleConfig {
        RuleCatalog::default_v1().velocity_rule("velocity_24h").unwrap().clone()
    }

    #[test]
    fn does_not_fire_below_threshold() {
        let cfg = velocity_24h();
        let store = VelocityWindowStore::new();
        for i in 0..4 {
            let candidate = store.observe_and_evaluate(&cfg, &debit(&format!("evt-{i}"), "cust-1", 1_000 * i, 200_000));
            assert!(candidate.is_none(), "must not fire before 5 debits: {candidate:?}");
        }
    }

    #[test]
    fn fires_once_both_count_and_total_thresholds_are_met() {
        let cfg = velocity_24h();
        let store = VelocityWindowStore::new();
        let mut last = None;
        for i in 0..5 {
            last = store.observe_and_evaluate(&cfg, &debit(&format!("evt-{i}"), "cust-1", 1_000 * i, 200_000));
        }
        let candidate = last.expect("5th debit crosses both thresholds (5 events, 1,000,000 minor)");
        assert_eq!(candidate.debit_count, 5);
        assert_eq!(candidate.debit_total_minor, 1_000_000);
        assert_eq!(candidate.rule_id, "velocity_24h");
    }

    #[test]
    fn does_not_fire_when_count_met_but_total_amount_below_threshold() {
        let cfg = velocity_24h();
        let store = VelocityWindowStore::new();
        let mut last = None;
        for i in 0..5 {
            last = store.observe_and_evaluate(&cfg, &debit(&format!("evt-{i}"), "cust-1", 1_000 * i, 1_000));
        }
        assert!(last.is_none(), "5 tiny debits must not cross the amount threshold");
    }

    #[test]
    fn a_late_event_outside_the_window_does_not_count_towards_it() {
        let cfg = velocity_24h();
        let store = VelocityWindowStore::new();
        // 4 debits now, then one far in the past (outside any 24h window
        // ending at "now") -- recompute-from-scratch means this late event
        // simply doesn't fall in the window, it doesn't corrupt state.
        for i in 0..4 {
            store.observe_and_evaluate(&cfg, &debit(&format!("evt-{i}"), "cust-1", 100_000_000 + 1_000 * i, 200_000));
        }
        let candidate = store.observe_and_evaluate(&cfg, &debit("evt-late", "cust-1", 1, 200_000));
        assert!(candidate.is_none(), "a debit far outside the window must not push the count over threshold");
    }

    #[test]
    fn credits_never_count_towards_the_velocity_rule() {
        let cfg = velocity_24h();
        let store = VelocityWindowStore::new();
        let mut credit_event = debit("evt-credit", "cust-1", 1_000, 10_000_000);
        credit_event.direction = Direction::Credit;
        assert!(store.observe_and_evaluate(&cfg, &credit_event).is_none());
    }

    #[test]
    fn a_deployment_can_supply_different_thresholds_without_a_recompile() {
        let catalog = RuleCatalog::from_toml_str(
            r#"
[rule.velocity_1h_small_institution]
version = 1
window_hours = 1
min_debit_count = 2
min_debit_total_minor = 10_000
severity = "medium"
"#,
        )
        .unwrap();
        let cfg = catalog.velocity_rule("velocity_1h_small_institution").unwrap();
        let store = VelocityWindowStore::new();
        store.observe_and_evaluate(cfg, &debit("evt-0", "cust-1", 0, 6_000));
        let candidate = store.observe_and_evaluate(cfg, &debit("evt-1", "cust-1", 1_000, 6_000)).expect("2 debits should cross this org's much lower threshold");
        assert_eq!(candidate.severity, "medium");
    }
}
