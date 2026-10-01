//! A demo-mode, in-process stream adapter -- the CTMS design note's own
//! instruction: "demo mode should use a local Kafka-compatible adapter or
//! embedded test stream rather than claim production Kafka guarantees."
//! [`InMemoryTopic`] implements the *exact same*
//! [`StreamSource`]/[`StreamSink`] port `nirdosha-ingestion-topic-kafka`'s
//! real `rskafka`-backed `KafkaTopicDriver` implements, so swapping in a
//! real broker later is a type substitution, not a rewrite -- and,
//! single-partition-only, matches that driver's own documented scope.
//!
//! [`StreamAdapter`] is the "consumer" side of the design note's pipeline
//! (deserialize -> validate schema -> deduplicate event_id -> normalize),
//! composed on top of whatever [`StreamSource`] it's given. It demonstrates,
//! with real code exercised by this module's tests, every property the
//! design note calls out: duplicate-event handling, out-of-order/late-event
//! handling, malformed-event rejection, offset/partition recording, and
//! consumer-restart replay.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use nirdosha_ingestion_topic_kafka::{PortError, PublishAck, RawBatch, RawRecord, StreamSink, StreamSource};

use crate::event::{normalize, NormalizeError, TransactionEvent};

/// Single-partition in-process topic: an append-only `Vec<RawRecord>`
/// behind a mutex, offsets are simply indices -- the same convention
/// `KafkaTopicDriver` uses for a real broker's partition offsets.
#[derive(Default)]
pub struct InMemoryTopic {
    log: Mutex<Vec<RawRecord>>,
}

impl InMemoryTopic {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl StreamSource for InMemoryTopic {
    async fn poll_batch(&self, _topic: &str, from_offset: i64, _max_wait_ms: u32) -> Result<RawBatch, PortError> {
        let log = self.log.lock().expect("in-memory topic lock poisoned");
        let start = from_offset.max(0) as usize;
        if start >= log.len() {
            return Ok(RawBatch { records: vec![], offsets: vec![] });
        }
        let records = log[start..].to_vec();
        let offsets = (start as i64..log.len() as i64).collect();
        Ok(RawBatch { records, offsets })
    }
}

#[async_trait::async_trait]
impl StreamSink for InMemoryTopic {
    async fn publish(&self, _topic: &str, records: Vec<RawRecord>) -> Result<PublishAck, PortError> {
        let mut log = self.log.lock().expect("in-memory topic lock poisoned");
        let mut offsets = Vec::with_capacity(records.len());
        for record in records {
            offsets.push(log.len() as i64);
            log.push(record);
        }
        Ok(PublishAck { offsets })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedRecord {
    pub event: TransactionEvent,
    /// `true` when this event's `event_time_ms` is not the newest this
    /// adapter has seen for its `customer_id` -- i.e. it arrived
    /// out-of-order/late relative to processing order. The rule engine
    /// (`crate::rules`) always recomputes its window from scratch, so a
    /// late event is handled correctly regardless of this flag; it's
    /// surfaced here purely for observability (the design note's
    /// `late_event_policy` / pipeline-health screen concern).
    pub late: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsumedRecord {
    Accepted(NormalizedRecordKey),
    Duplicate { partition: i32, offset: i64, event_id: String },
    Malformed { partition: i32, offset: i64, reason: String },
}

/// A lightweight, comparable stand-in used only by `ConsumedRecord` so the
/// enum can derive `Eq` for test assertions; callers get the real
/// [`NormalizedRecord`] (with the full event) from
/// [`StreamAdapter::process_batch`]'s other return value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedRecordKey {
    pub partition: i32,
    pub offset: i64,
    pub event_id: String,
}

/// Consumer-side pipeline state: which `event_id`s have already been
/// normalized (dedup) and each customer's newest `event_time_ms` seen so
/// far (late-event detection). Both live only in this adapter instance --
/// a fresh instance against the same [`InMemoryTopic`] has none of this
/// memory, which is exactly what lets a test demonstrate "consumer restart
/// replay": the raw log is unaffected by the adapter that reads it.
#[derive(Default)]
pub struct StreamAdapter {
    seen_event_ids: Mutex<HashSet<String>>,
    watermark_ms: Mutex<HashMap<String, u64>>,
}

impl StreamAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs one already-fetched [`RawBatch`] through
    /// deserialize -> validate -> dedup -> normalize, returning both the
    /// per-record outcome (for offset/partition-level observability) and
    /// the accepted, normalized events (what the rule engine actually
    /// consumes).
    pub fn process_batch(&self, batch: &RawBatch, partition: i32, now_ms: u64) -> (Vec<ConsumedRecord>, Vec<NormalizedRecord>) {
        let mut outcomes = Vec::with_capacity(batch.records.len());
        let mut accepted = Vec::new();
        for (record, offset) in batch.records.iter().zip(batch.offsets.iter()) {
            match normalize(&record.payload, partition, *offset, now_ms) {
                Err(NormalizeError::Malformed(reason)) => {
                    outcomes.push(ConsumedRecord::Malformed { partition, offset: *offset, reason });
                }
                Ok(event) => {
                    let is_new = self.seen_event_ids.lock().expect("dedup lock poisoned").insert(event.event_id.clone());
                    if !is_new {
                        outcomes.push(ConsumedRecord::Duplicate { partition, offset: *offset, event_id: event.event_id.clone() });
                        continue;
                    }
                    let mut watermark = self.watermark_ms.lock().expect("watermark lock poisoned");
                    let current = watermark.entry(event.customer_id.clone()).or_insert(0);
                    let late = event.event_time_ms < *current;
                    if event.event_time_ms > *current {
                        *current = event.event_time_ms;
                    }
                    drop(watermark);
                    outcomes.push(ConsumedRecord::Accepted(NormalizedRecordKey {
                        partition,
                        offset: *offset,
                        event_id: event.event_id.clone(),
                    }));
                    accepted.push(NormalizedRecord { event, late });
                }
            }
        }
        (outcomes, accepted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
    }

    fn multi_thread_runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap()
    }

    fn kafka_brokers() -> Vec<String> {
        std::env::var("NIRDOSHA_TEST_KAFKA_BROKERS").unwrap_or_else(|_| "127.0.0.1:9092".to_string()).split(',').map(str::to_string).collect()
    }

    /// Proves the exact same `StreamAdapter` this module's other tests
    /// exercise against `InMemoryTopic` works unchanged against a real
    /// Kafka-protocol broker (`nirdosha_ingestion_topic_kafka::KafkaTopicDriver`,
    /// backed by `rskafka`, verified against Redpanda) -- `StreamAdapter`
    /// only ever consumes a `RawBatch`, so it does not know or care which
    /// `StreamSource` produced it. Gated behind `#[ignore]` + a live
    /// broker, same convention `nirdosha-ingestion-topic-kafka`'s own
    /// tests use -- this sandbox has no broker to run it against, but the
    /// real integration point is real code, not aspirational.
    #[test]
    #[ignore]
    fn the_same_stream_adapter_processes_records_from_a_real_kafka_protocol_broker() {
        use nirdosha_ingestion_topic_kafka::KafkaTopicDriver;

        multi_thread_runtime().block_on(async {
            let driver = KafkaTopicDriver::connect(kafka_brokers()).await.expect("connect to a real Redpanda/Kafka-protocol broker");
            let topic = format!("nirdosha-ctms-test-topic-{}", std::process::id());

            let records = (0..5)
                .map(|i| RawRecord { key: None, payload: event_json(&format!("evt-{i}"), "cust-1", 1_000 * i as u64).into_bytes() })
                .collect();
            driver.publish(&topic, records).await.expect("publish to a real broker must succeed");

            let batch = driver.poll_batch(&topic, 0, 5_000).await.expect("poll from a real broker must succeed");
            assert_eq!(batch.records.len(), 5, "expected all 5 real published records back: {batch:?}");

            let adapter = StreamAdapter::new();
            let (outcomes, normalized) = adapter.process_batch(&batch, 0, 0);
            assert_eq!(normalized.len(), 5, "the same StreamAdapter must accept real broker-delivered records identically to InMemoryTopic ones");
            assert!(outcomes.iter().all(|o| matches!(o, ConsumedRecord::Accepted(_))));
        });
    }

    fn event_json(event_id: &str, customer_id: &str, event_time_ms: u64) -> String {
        serde_json::json!({
            "event_id": event_id,
            "transaction_id": format!("txn-{event_id}"),
            "event_type": "card_debit",
            "customer_id": customer_id,
            "account_id": "acct-1",
            "direction": "debit",
            "amount_minor": 100_000,
            "currency": "INR",
            "event_time_ms": event_time_ms,
            "source_system": "card-network-x",
            "source_version": "v1",
            "jurisdiction": "IN",
        })
        .to_string()
    }

    #[test]
    fn offsets_and_partitions_are_recorded_and_malformed_records_are_rejected_without_stopping_the_batch() {
        runtime().block_on(async {
            let topic = InMemoryTopic::new();
            topic
                .publish(
                    "ctms.transactions",
                    vec![
                        RawRecord { key: None, payload: event_json("evt-1", "cust-1", 1_000).into_bytes() },
                        RawRecord { key: None, payload: b"{not valid json".to_vec() },
                        RawRecord { key: None, payload: event_json("evt-2", "cust-1", 2_000).into_bytes() },
                    ],
                )
                .await
                .unwrap();

            let batch = topic.poll_batch("ctms.transactions", 0, 0).await.unwrap();
            assert_eq!(batch.offsets, vec![0, 1, 2]);

            let adapter = StreamAdapter::new();
            let (outcomes, accepted) = adapter.process_batch(&batch, 0, 10_000);
            assert_eq!(accepted.len(), 2, "the two well-formed events must still be accepted");
            assert!(matches!(&outcomes[1], ConsumedRecord::Malformed { partition: 0, offset: 1, .. }), "{outcomes:?}");
            assert!(matches!(&outcomes[0], ConsumedRecord::Accepted(NormalizedRecordKey { partition: 0, offset: 0, .. })));
            assert!(matches!(&outcomes[2], ConsumedRecord::Accepted(NormalizedRecordKey { partition: 0, offset: 2, .. })));
        });
    }

    #[test]
    fn duplicate_event_ids_are_flagged_and_not_normalized_twice() {
        runtime().block_on(async {
            let topic = InMemoryTopic::new();
            topic
                .publish(
                    "ctms.transactions",
                    vec![
                        RawRecord { key: None, payload: event_json("evt-1", "cust-1", 1_000).into_bytes() },
                        RawRecord { key: None, payload: event_json("evt-1", "cust-1", 1_000).into_bytes() },
                    ],
                )
                .await
                .unwrap();
            let batch = topic.poll_batch("ctms.transactions", 0, 0).await.unwrap();

            let adapter = StreamAdapter::new();
            let (outcomes, accepted) = adapter.process_batch(&batch, 0, 0);
            assert_eq!(accepted.len(), 1);
            assert!(matches!(&outcomes[1], ConsumedRecord::Duplicate { event_id, .. } if event_id == "evt-1"));
        });
    }

    #[test]
    fn an_out_of_order_event_older_than_the_watermark_is_flagged_late() {
        runtime().block_on(async {
            let topic = InMemoryTopic::new();
            topic
                .publish(
                    "ctms.transactions",
                    vec![
                        RawRecord { key: None, payload: event_json("evt-2", "cust-1", 5_000).into_bytes() },
                        RawRecord { key: None, payload: event_json("evt-1", "cust-1", 1_000).into_bytes() },
                    ],
                )
                .await
                .unwrap();
            let batch = topic.poll_batch("ctms.transactions", 0, 0).await.unwrap();

            let adapter = StreamAdapter::new();
            let (_outcomes, accepted) = adapter.process_batch(&batch, 0, 0);
            assert!(!accepted[0].late, "the first (newest) event establishes the watermark, not late");
            assert!(accepted[1].late, "the second event is older than the watermark already set by evt-2");
        });
    }

    #[test]
    fn a_consumer_restart_can_replay_from_offset_zero_and_reprocesses_deterministically() {
        runtime().block_on(async {
            let topic = InMemoryTopic::new();
            topic
                .publish(
                    "ctms.transactions",
                    vec![
                        RawRecord { key: None, payload: event_json("evt-1", "cust-1", 1_000).into_bytes() },
                        RawRecord { key: None, payload: event_json("evt-2", "cust-1", 2_000).into_bytes() },
                    ],
                )
                .await
                .unwrap();

            let first_adapter = StreamAdapter::new();
            let first_batch = topic.poll_batch("ctms.transactions", 0, 0).await.unwrap();
            let (_o1, accepted_first_run) = first_adapter.process_batch(&first_batch, 0, 0);
            assert_eq!(accepted_first_run.len(), 2);

            // A restart builds a brand new adapter (no dedup/watermark memory)
            // and replays the same topic from offset 0 -- the raw log itself
            // is untouched by the adapter that reads it, so the replay
            // reprocesses the identical two events deterministically.
            let restarted_adapter = StreamAdapter::new();
            let replay_batch = topic.poll_batch("ctms.transactions", 0, 0).await.unwrap();
            let (_o2, accepted_replay) = restarted_adapter.process_batch(&replay_batch, 0, 0);
            assert_eq!(accepted_replay.len(), 2, "replay reprocesses the same two events");
            assert_eq!(
                accepted_replay.iter().map(|r| r.event.event_id.clone()).collect::<Vec<_>>(),
                accepted_first_run.iter().map(|r| r.event.event_id.clone()).collect::<Vec<_>>(),
            );
        });
    }
}
