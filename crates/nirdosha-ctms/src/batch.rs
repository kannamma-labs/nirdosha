//! Batch replay: the design note's "batch mode should reuse the same
//! evaluator, not a second rule engine." [`run_batch`] runs the *exact
//! same* [`crate::stream::StreamAdapter`] (dedup/malformed/late detection)
//! and [`crate::rules::VelocityWindowStore`] (the rule engine) the
//! realtime path uses, over an already-fetched
//! [`nirdosha_ingestion_topic_kafka::RawBatch`] (an offset range, a
//! historical topic replay, or a batch built from a CSV/JSON file --
//! anything that can be shaped into a `RawBatch` uses this same function),
//! and creates alerts through the *same* [`crate::alert::CtmsAlertGatewayV1`]
//! realtime uses -- there is no batch-only, ungated insert path.
//!
//! **Idempotent at two levels.** [`crate::alert::AlertStore`]'s own
//! idempotency key (`rule_id`, `rule_version`, `key`, `window_start_ms`)
//! doesn't depend on `origin`, so replaying the same events in batch after
//! they already alerted in realtime (or replaying the same batch input
//! twice) never creates a duplicate alert -- "first writer's `origin`
//! wins" (see `crate::alert::Alert::origin`'s own doc). On top of that,
//! [`run_batch`] itself is idempotent by `run_id`: calling it twice with
//! the same `run_id` returns the exact same [`BatchRunReport`] the first
//! call produced, without reprocessing a single event a second time.

use std::collections::HashMap;
use std::sync::Mutex;

use nirdosha_guard_rfc0029::{CapabilityIssuer, EffectGateway as _};
use nirdosha_ingestion_topic_kafka::RawBatch;
use nirdosha_rt::Auth;

use crate::alert::{AlertStore, CtmsAlertGatewayV1};
use crate::rules::RuleCatalog;
use crate::stream::{ConsumedRecord, StreamAdapter};

/// One batch run's provenance and result counters -- exactly the fields
/// the design note requires ("Every batch run needs: run_id, input
/// source, offset/date range, policy bundle hash, rule bundle hash,
/// started_at, completed_at, event count, duplicate count, late count,
/// alert count, error count").
#[derive(Debug, Clone, PartialEq)]
pub struct BatchRunReport {
    pub run_id: String,
    pub input_source: String,
    pub policy_bundle_hash: String,
    pub rule_bundle_hash: String,
    pub started_at_ms: u64,
    pub completed_at_ms: u64,
    pub event_count: usize,
    pub duplicate_count: usize,
    pub late_count: usize,
    pub alert_count: usize,
    pub error_count: usize,
    pub alert_ids: Vec<String>,
}

pub trait BatchRunStore: Send + Sync {
    fn get(&self, run_id: &str) -> Option<BatchRunReport>;
    fn save(&self, report: &BatchRunReport);
}

#[derive(Default)]
pub struct InMemoryBatchRunStore {
    runs: Mutex<HashMap<String, BatchRunReport>>,
}

impl InMemoryBatchRunStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl BatchRunStore for InMemoryBatchRunStore {
    fn get(&self, run_id: &str) -> Option<BatchRunReport> {
        self.runs.lock().expect("batch run store lock poisoned").get(run_id).cloned()
    }

    fn save(&self, report: &BatchRunReport) {
        self.runs.lock().expect("batch run store lock poisoned").insert(report.run_id.clone(), report.clone());
    }
}

/// Runs `batch` (already-fetched raw records with real offsets, from
/// `nirdosha_ingestion_topic_kafka::StreamSource::poll_batch`, a
/// historical replay, or a file loader shaped the same way) through the
/// realtime pipeline's own adapter, rule engine, and gateway. See module
/// doc for the two idempotency levels.
#[allow(clippy::too_many_arguments)]
pub fn run_batch(
    run_store: &dyn BatchRunStore,
    run_id: &str,
    input_source: &str,
    batch: &RawBatch,
    partition: i32,
    policy_bundle_hash: &str,
    rule_catalog: &RuleCatalog,
    rule_id: &str,
    issuer: &CapabilityIssuer,
    alert_gateway: &CtmsAlertGatewayV1,
    alert_store: &dyn AlertStore,
    now_ms: u64,
) -> BatchRunReport {
    if let Some(existing) = run_store.get(run_id) {
        return existing;
    }

    let cfg = rule_catalog.velocity_rule(rule_id).unwrap_or_else(|| panic!("rule `{rule_id}` must exist in the given catalog"));
    let adapter = StreamAdapter::new();
    let (outcomes, normalized) = adapter.process_batch(batch, partition, now_ms);
    let duplicate_count = outcomes.iter().filter(|o| matches!(o, ConsumedRecord::Duplicate { .. })).count();
    let error_count = outcomes.iter().filter(|o| matches!(o, ConsumedRecord::Malformed { .. })).count();
    let late_count = normalized.iter().filter(|record| record.late).count();

    let window_store = crate::rules::VelocityWindowStore::new();
    let batch_subject = Auth::login(format!("ctms-batch-runner:{run_id}"), &["System"]);
    let mut alert_ids = Vec::new();
    for record in &normalized {
        if let Some(candidate) = window_store.observe_and_evaluate(cfg, &record.event) {
            let capability = issuer
                .mint(&batch_subject, CtmsAlertGatewayV1::RESOURCE, CtmsAlertGatewayV1::EFFECT, now_ms)
                .expect("the governing bundle must be active for a batch run to mint alert.create capabilities");
            if let Ok(alert) = alert_gateway.create_with_origin(&capability, alert_store, &candidate, "batch", now_ms) {
                if !alert_ids.contains(&alert.alert_id) {
                    alert_ids.push(alert.alert_id);
                }
            }
        }
    }

    let report = BatchRunReport {
        run_id: run_id.to_string(),
        input_source: input_source.to_string(),
        policy_bundle_hash: policy_bundle_hash.to_string(),
        rule_bundle_hash: rule_catalog.catalog_hash().to_string(),
        started_at_ms: now_ms,
        completed_at_ms: now_ms,
        event_count: batch.records.len(),
        duplicate_count,
        late_count,
        alert_count: alert_ids.len(),
        error_count,
        alert_ids,
    };
    run_store.save(&report);
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alert::InMemoryAlertStore;
    use nirdosha_guard_rfc0029::PolicyBundle;
    use nirdosha_ingestion_topic_kafka::{RawRecord, StreamSink, StreamSource};

    const VALID_BUNDLE: &str = r#"
[bundle]
authority_id = "ctms-domain-authority"
policy_owner = "risk-platform-team"
jurisdiction = "IN"
approved_by = "ctms-domain-authority"
signed_by = "ctms-domain-authority"
effective_from = "2026-01-01T00:00:00Z"
expires_at = "2027-01-01T00:00:00Z"
"#;

    fn now_ms() -> u64 {
        1_780_000_000_000
    }

    fn issuer() -> CapabilityIssuer {
        let bundle = PolicyBundle::from_toml_str(VALID_BUNDLE).unwrap();
        CapabilityIssuer::new(bundle, 5 * 60 * 1000)
    }

    fn debit_json(event_id: &str, event_time_ms: u64) -> String {
        serde_json::json!({
            "event_id": event_id,
            "transaction_id": format!("txn-{event_id}"),
            "event_type": "card_debit",
            "customer_id": "cust-1",
            "account_id": "acct-1",
            "direction": "debit",
            "amount_minor": 200_000,
            "currency": "INR",
            "event_time_ms": event_time_ms,
            "source_system": "card-network-x",
            "source_version": "v1",
            "jurisdiction": "IN",
        })
        .to_string()
    }

    fn five_debit_batch() -> RawBatch {
        let topic = crate::stream::InMemoryTopic::new();
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(async {
            let records = (0..5).map(|i| RawRecord { key: None, payload: debit_json(&format!("evt-{i}"), 1_000 * i as u64).into_bytes() }).collect();
            topic.publish("t", records).await.unwrap();
            topic.poll_batch("t", 0, 0).await.unwrap()
        })
    }

    #[test]
    fn a_batch_run_reuses_the_rule_evaluator_and_produces_an_origin_batch_alert() {
        let issuer = issuer();
        let alert_store = InMemoryAlertStore::new();
        let alert_gateway = CtmsAlertGatewayV1::new(std::env::temp_dir().join(format!("ctms_batch_test_{}", std::process::id())), &issuer);
        let run_store = InMemoryBatchRunStore::new();
        let catalog = RuleCatalog::default_v1();
        let batch = five_debit_batch();

        let report = run_batch(&run_store, "run-1", "csv:2026-09-01.csv", &batch, 0, &issuer.bundle().bundle_hash, &catalog, "velocity_24h", &issuer, &alert_gateway, &alert_store, now_ms());

        assert_eq!(report.event_count, 5);
        assert_eq!(report.duplicate_count, 0);
        assert_eq!(report.error_count, 0);
        assert_eq!(report.alert_count, 1);
        let alert = alert_store.get(&report.alert_ids[0]).unwrap();
        assert_eq!(alert.origin, "batch");
        assert_eq!(alert.rule_id, "velocity_24h");
    }

    #[test]
    fn rerunning_the_same_run_id_returns_the_identical_report_without_reprocessing() {
        let issuer = issuer();
        let alert_store = InMemoryAlertStore::new();
        let alert_gateway = CtmsAlertGatewayV1::new(std::env::temp_dir().join(format!("ctms_batch_test_rerun_{}", std::process::id())), &issuer);
        let run_store = InMemoryBatchRunStore::new();
        let catalog = RuleCatalog::default_v1();
        let batch = five_debit_batch();

        let first = run_batch(&run_store, "run-2", "csv", &batch, 0, "bundle-hash", &catalog, "velocity_24h", &issuer, &alert_gateway, &alert_store, now_ms());
        let second = run_batch(&run_store, "run-2", "csv", &batch, 0, "bundle-hash", &catalog, "velocity_24h", &issuer, &alert_gateway, &alert_store, now_ms() + 1);

        assert_eq!(first, second, "the second call must return the exact stored report, not reprocess");
        assert_eq!(alert_store.list().len(), 1, "no duplicate alert from the rerun");
    }

    #[test]
    fn a_batch_replay_of_events_already_alerted_in_realtime_does_not_duplicate_the_alert() {
        let issuer = issuer();
        let alert_store = InMemoryAlertStore::new();
        let evidence_path = std::env::temp_dir().join(format!("ctms_batch_test_crosspath_{}", std::process::id()));
        let alert_gateway = CtmsAlertGatewayV1::new(&evidence_path, &issuer);
        let catalog = RuleCatalog::default_v1();
        let batch = five_debit_batch();
        let cfg = catalog.velocity_rule("velocity_24h").unwrap();

        // Realtime path: process the same 5 events one at a time first.
        let realtime_adapter = crate::stream::StreamAdapter::new();
        let (_o, normalized) = realtime_adapter.process_batch(&batch, 0, now_ms());
        let window_store = crate::rules::VelocityWindowStore::new();
        let mut realtime_alert_id = None;
        for record in &normalized {
            if let Some(candidate) = window_store.observe_and_evaluate(cfg, &record.event) {
                let auth = Auth::login("ctms-evaluator", &["System"]);
                let cap = issuer.mint(&auth, CtmsAlertGatewayV1::RESOURCE, CtmsAlertGatewayV1::EFFECT, now_ms()).unwrap();
                let alert = alert_gateway.create(&cap, &alert_store, &candidate, now_ms()).unwrap();
                realtime_alert_id = Some(alert.alert_id);
            }
        }
        assert!(realtime_alert_id.is_some());
        assert_eq!(alert_store.list().len(), 1);

        // Now batch-replay the identical events -- must reuse the same
        // alert, not create a second one with origin = "batch".
        let run_store = InMemoryBatchRunStore::new();
        let report = run_batch(&run_store, "replay-run", "kafka-replay", &batch, 0, &issuer.bundle().bundle_hash, &catalog, "velocity_24h", &issuer, &alert_gateway, &alert_store, now_ms());
        assert_eq!(report.alert_ids, vec![realtime_alert_id.unwrap()], "batch replay must resolve to the same alert identity realtime already created");
        assert_eq!(alert_store.list().len(), 1, "still exactly one alert -- realtime's origin wins");
        assert_eq!(alert_store.list()[0].origin, "realtime");
    }
}
