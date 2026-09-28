//! Pipeline-health metrics -- consumer lag, malformed/late/duplicate
//! counts, degraded status -- the design note's "pipeline health" screen
//! concern, minus the screen (not built) and minus any real SLA/paging
//! guarantee (this crate has no alerting/on-call infrastructure to page
//! anyone; a snapshot struct is the honest ceiling of what pure local
//! code can provide -- "monitor-health guarantees" as an operational SLA
//! is not something this crate can honestly claim).
//!
//! [`PipelineHealthTracker`] is a plain counter aggregator a caller feeds
//! from `crate::stream::StreamAdapter::process_batch`'s own return value
//! (kept decoupled from `StreamAdapter` itself so tracking health is
//! opt-in, not a hidden side effect of every call).

use std::sync::Mutex;

use crate::stream::{ConsumedRecord, NormalizedRecord};

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PipelineHealthSnapshot {
    pub events_processed: u64,
    pub duplicate_count: u64,
    pub malformed_count: u64,
    pub late_count: u64,
    pub last_processed_at_ms: Option<u64>,
    /// `high_watermark - next_offset_to_consume`, as last reported via
    /// [`PipelineHealthTracker::record_lag`] -- this crate has no real
    /// broker connection of its own to compute this from; a caller wires
    /// a real `StreamSource`'s high-watermark reporting in.
    pub consumer_lag: i64,
}

#[derive(Default)]
pub struct PipelineHealthTracker {
    snapshot: Mutex<PipelineHealthSnapshot>,
}

impl PipelineHealthTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_batch(&self, outcomes: &[ConsumedRecord], normalized: &[NormalizedRecord], now_ms: u64) {
        let mut snapshot = self.snapshot.lock().expect("pipeline health lock poisoned");
        snapshot.events_processed += outcomes.len() as u64;
        snapshot.duplicate_count += outcomes.iter().filter(|o| matches!(o, ConsumedRecord::Duplicate { .. })).count() as u64;
        snapshot.malformed_count += outcomes.iter().filter(|o| matches!(o, ConsumedRecord::Malformed { .. })).count() as u64;
        snapshot.late_count += normalized.iter().filter(|r| r.late).count() as u64;
        snapshot.last_processed_at_ms = Some(now_ms);
    }

    pub fn record_lag(&self, consumer_lag: i64) {
        self.snapshot.lock().expect("pipeline health lock poisoned").consumer_lag = consumer_lag;
    }

    pub fn snapshot(&self) -> PipelineHealthSnapshot {
        *self.snapshot.lock().expect("pipeline health lock poisoned")
    }

    /// A simple, real (not fabricated) heuristic: degraded if more than
    /// `malformed_ratio_threshold` of processed events were malformed, or
    /// lag exceeds `max_acceptable_lag`. Not a production SLA definition
    /// -- a real deployment tunes its own thresholds and wires real
    /// alerting on top of this snapshot.
    pub fn is_degraded(&self, malformed_ratio_threshold: f64, max_acceptable_lag: i64) -> bool {
        let snapshot = self.snapshot();
        if snapshot.events_processed == 0 {
            return false;
        }
        let malformed_ratio = snapshot.malformed_count as f64 / snapshot.events_processed as f64;
        malformed_ratio > malformed_ratio_threshold || snapshot.consumer_lag > max_acceptable_lag
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::{InMemoryTopic, StreamAdapter};
    use nirdosha_ingestion_topic_kafka::{RawRecord, StreamSink, StreamSource};

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
    }

    #[test]
    fn tracks_malformed_and_duplicate_counts_across_batches() {
        let topic = InMemoryTopic::new();
        let good = serde_json::json!({
            "event_id": "evt-1", "transaction_id": "txn-1", "event_type": "card_debit", "customer_id": "cust-1",
            "account_id": "acct-1", "direction": "debit", "amount_minor": 1000, "currency": "INR",
            "event_time_ms": 0, "source_system": "x", "source_version": "v1", "jurisdiction": "IN",
        })
        .to_string();
        runtime().block_on(async {
            topic.publish("t", vec![RawRecord { key: None, payload: good.clone().into_bytes() }, RawRecord { key: None, payload: b"{bad".to_vec() }, RawRecord { key: None, payload: good.into_bytes() }]).await.unwrap();
        });
        let batch = runtime().block_on(async { topic.poll_batch("t", 0, 0).await.unwrap() });

        let adapter = StreamAdapter::new();
        let health = PipelineHealthTracker::new();
        let (outcomes, normalized) = adapter.process_batch(&batch, 0, 100);
        health.record_batch(&outcomes, &normalized, 100);

        let snapshot = health.snapshot();
        assert_eq!(snapshot.events_processed, 3);
        assert_eq!(snapshot.malformed_count, 1);
        assert_eq!(snapshot.duplicate_count, 1, "the second identical event_id is a duplicate");
        assert_eq!(snapshot.last_processed_at_ms, Some(100));
    }

    #[test]
    fn is_degraded_when_malformed_ratio_exceeds_threshold() {
        use crate::stream::NormalizedRecordKey;
        let health = PipelineHealthTracker::new();
        // 2 malformed out of 10 total = 20% malformed ratio.
        let mut outcomes = vec![ConsumedRecord::Malformed { partition: 0, offset: 0, reason: "x".to_string() }, ConsumedRecord::Malformed { partition: 0, offset: 1, reason: "x".to_string() }];
        for i in 2..10 {
            outcomes.push(ConsumedRecord::Accepted(NormalizedRecordKey { partition: 0, offset: i, event_id: format!("evt-{i}") }));
        }
        health.record_batch(&outcomes, &[], 0);
        assert!(health.is_degraded(0.1, 1_000_000), "20% > 10% threshold must be degraded");
        assert!(!health.is_degraded(0.5, 1_000_000), "20% < 50% threshold must not be degraded");
    }

    #[test]
    fn is_degraded_when_consumer_lag_exceeds_the_acceptable_maximum() {
        let health = PipelineHealthTracker::new();
        health.record_batch(&[ConsumedRecord::Malformed { partition: 0, offset: 0, reason: "x".to_string() }], &[], 0);
        health.record_lag(10_000);
        assert!(health.is_degraded(1.0, 100));
        assert!(!health.is_degraded(1.0, 100_000));
    }
}
