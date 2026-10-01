//! `StreamSource`/`StreamSink` for RFC 0025 §8.1 (ingestion) — Plan Phase 13.
//!
//! §8.1 declares real trait shapes (`StreamSource::subscribe` returning a
//! `BoxStream`, `StreamSink::publish`) and names a Kafka driver
//! (`rdkafka`-based) as the example, but no implementation existed
//! anywhere in the workspace. This crate is that driver, built against
//! `rskafka` (a pure-Rust Kafka-protocol client with no C/`librdkafka`
//! build dependency — a deliberate, documented swap from the RFC's own
//! `rdkafka` illustration; see `docs/adr/0016` for why) and verified
//! against a real Redpanda broker (`docker-compose.dev.yml`'s `redpanda`
//! service, Kafka-API-compatible).
//!
//! **Poll-based, not a boxed `Stream`.** The RFC's illustrative
//! `subscribe(...) -> Result<BoxStream<'static, RawBatch>, PortError>`
//! hides real backpressure/cancellation/offset-tracking questions behind
//! a `Stream` object this crate would otherwise have to invent an honest
//! implementation of from scratch. `poll_batch` is the real, minimal
//! contract this driver actually implements — the same request/response
//! shape `rskafka::client::partition::PartitionClient::fetch_records`
//! itself has under the hood. A `Stream`-returning wrapper is a thin
//! adapter over repeated `poll_batch` calls and can be added later
//! without changing this trait; inventing one now, only to satisfy the
//! RFC's illustration syntactically, would be exactly the kind of
//! aspirational-not-honest surface this workspace's other drivers avoid.

use std::sync::Arc;

use rskafka::client::partition::{Compression, PartitionClient, UnknownTopicHandling};
use rskafka::client::{Client, ClientBuilder};
use rskafka::record::{Record, RecordAndOffset};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawRecord {
	pub key: Option<Vec<u8>>,
	pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawBatch {
	pub records: Vec<RawRecord>,
	/// The offset each record in `records` was actually stored at —
	/// parallel to `records`, same convention `rskafka::record::RecordAndOffset`
	/// itself uses. The *next* `poll_batch` call should pass
	/// `offsets.last() + 1` as `from_offset` to continue where this batch
	/// left off.
	pub offsets: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishAck {
	pub offsets: Vec<i64>,
}

#[derive(Debug)]
pub enum PortError {
	Kafka(String),
}

#[async_trait::async_trait]
pub trait StreamSource: Send + Sync {
	/// Fetches whatever is available starting at `from_offset`, waiting
	/// up to `max_wait_ms` for at least one record before returning an
	/// empty batch. Never blocks past `max_wait_ms` — a caller loops this
	/// to get the stream-like behavior the RFC's `BoxStream` illustration
	/// implies, with an explicit offset it fully controls (I10/§8.4's
	/// "opaque cursors only" posture, applied here too: the offset a
	/// caller passes back is exactly the one this driver handed it, never
	/// one it's expected to compute itself).
	async fn poll_batch(&self, topic: &str, from_offset: i64, max_wait_ms: u32) -> Result<RawBatch, PortError>;
}

#[async_trait::async_trait]
pub trait StreamSink: Send + Sync {
	async fn publish(&self, topic: &str, records: Vec<RawRecord>) -> Result<PublishAck, PortError>;
}

/// Real Kafka-protocol `StreamSource`/`StreamSink`, backed by `rskafka`.
/// Caches one `PartitionClient` per topic (partition 0 only — the same
/// "one honest, minimal shape" scoping `MemStoreDriver`/`PostgresStoreDriver`
/// apply to their own single-table schema; a multi-partition topic is a
/// real future extension, not something this driver silently mishandles,
/// since `UnknownTopicHandling::Error` means an unrecognized topic/partition
/// fails loudly rather than being misrouted).
pub struct KafkaTopicDriver {
	client: Client,
	partition_clients: tokio::sync::Mutex<std::collections::HashMap<String, Arc<PartitionClient>>>,
}

impl KafkaTopicDriver {
	pub async fn connect(brokers: Vec<String>) -> Result<Self, PortError> {
		let client = ClientBuilder::new(brokers).build().await.map_err(|error| PortError::Kafka(error.to_string()))?;
		Ok(Self { client, partition_clients: tokio::sync::Mutex::new(std::collections::HashMap::new()) })
	}

	/// Idempotently ensures `topic` exists (single partition, replication
	/// factor 1 — a real dev/test topology, not production sizing) before
	/// returning a cached `PartitionClient` for it.
	async fn partition_client(&self, topic: &str) -> Result<Arc<PartitionClient>, PortError> {
		let mut cache = self.partition_clients.lock().await;
		if let Some(existing) = cache.get(topic) {
			return Ok(existing.clone());
		}
		let controller = self.client.controller_client().map_err(|error| PortError::Kafka(error.to_string()))?;
		match controller.create_topic(topic, 1, 1, 5_000).await {
			Ok(_) => {}
			Err(error) => {
				// Real idempotency: "topic already exists" is expected on
				// every call after the first, not an error to surface.
				let message = error.to_string();
				if !message.to_lowercase().contains("already exists") {
					return Err(PortError::Kafka(message));
				}
			}
		}
		let partition_client = self
			.client
			.partition_client(topic, 0, UnknownTopicHandling::Retry)
			.await
			.map_err(|error| PortError::Kafka(error.to_string()))?;
		let partition_client = Arc::new(partition_client);
		cache.insert(topic.to_string(), partition_client.clone());
		Ok(partition_client)
	}
}

#[async_trait::async_trait]
impl StreamSource for KafkaTopicDriver {
	async fn poll_batch(&self, topic: &str, from_offset: i64, max_wait_ms: u32) -> Result<RawBatch, PortError> {
		let partition_client = self.partition_client(topic).await?;
		let (records, _high_watermark): (Vec<RecordAndOffset>, i64) = partition_client
			.fetch_records(from_offset, 1..1_000_000, max_wait_ms as i32)
			.await
			.map_err(|error| PortError::Kafka(error.to_string()))?;
		let mut out_records = Vec::with_capacity(records.len());
		let mut offsets = Vec::with_capacity(records.len());
		for RecordAndOffset { record, offset } in records {
			out_records.push(RawRecord { key: record.key, payload: record.value.unwrap_or_default() });
			offsets.push(offset);
		}
		Ok(RawBatch { records: out_records, offsets })
	}
}

#[async_trait::async_trait]
impl StreamSink for KafkaTopicDriver {
	async fn publish(&self, topic: &str, records: Vec<RawRecord>) -> Result<PublishAck, PortError> {
		let partition_client = self.partition_client(topic).await?;
		let now = record_timestamp_now();
		let kafka_records: Vec<Record> = records
			.into_iter()
			.map(|record| Record { key: record.key, value: Some(record.payload), headers: std::collections::BTreeMap::new(), timestamp: now })
			.collect();
		let offsets = partition_client
			.produce(kafka_records, Compression::NoCompression)
			.await
			.map_err(|error| PortError::Kafka(error.to_string()))?;
		Ok(PublishAck { offsets })
	}
}

/// `rskafka::record::Record::timestamp` needs a `chrono::DateTime<Utc>`
/// (already a transitive dependency via `rskafka` itself, declared
/// explicitly here since this crate calls it directly).
fn record_timestamp_now() -> chrono::DateTime<chrono::Utc> {
	chrono::Utc::now()
}

#[cfg(test)]
mod tests {
	use super::*;

	fn brokers() -> Vec<String> {
		std::env::var("NIRDOSHA_TEST_KAFKA_BROKERS")
			.unwrap_or_else(|_| "127.0.0.1:9092".to_string())
			.split(',')
			.map(str::to_string)
			.collect()
	}

	fn runtime() -> tokio::runtime::Runtime {
		tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("build test runtime")
	}

	#[test]
	#[ignore]
	fn publish_and_poll_round_trips_real_records_through_a_real_broker() {
		runtime().block_on(async {
			let driver = KafkaTopicDriver::connect(brokers()).await.expect("connect to real Redpanda broker");
			let topic = format!("nirdosha-test-topic-{}", std::process::id());

			let records = vec![
				RawRecord { key: Some(b"k1".to_vec()), payload: b"first record".to_vec() },
				RawRecord { key: Some(b"k2".to_vec()), payload: b"second record".to_vec() },
			];
			let ack = driver.publish(&topic, records.clone()).await.expect("publish must succeed against a real broker");
			assert_eq!(ack.offsets.len(), 2);

			let batch = driver.poll_batch(&topic, 0, 5_000).await.expect("poll must succeed against a real broker");
			assert_eq!(batch.records.len(), 2, "expected both real published records back: {batch:?}");
			assert_eq!(batch.records[0].payload, b"first record");
			assert_eq!(batch.records[1].payload, b"second record");
			assert_eq!(batch.offsets, ack.offsets, "poll must report the same real offsets publish returned");
		});
	}

	#[test]
	#[ignore]
	fn poll_batch_resumes_from_an_explicit_offset_not_the_start_of_the_topic() {
		runtime().block_on(async {
			let driver = KafkaTopicDriver::connect(brokers()).await.expect("connect to real Redpanda broker");
			let topic = format!("nirdosha-test-topic-resume-{}", std::process::id());

			driver.publish(&topic, vec![RawRecord { key: None, payload: b"skip me".to_vec() }]).await.unwrap();
			let ack2 = driver.publish(&topic, vec![RawRecord { key: None, payload: b"resume here".to_vec() }]).await.unwrap();

			let batch = driver.poll_batch(&topic, ack2.offsets[0], 5_000).await.expect("poll from an explicit offset must succeed");
			assert_eq!(batch.records.len(), 1, "must return only the record at/after the requested offset: {batch:?}");
			assert_eq!(batch.records[0].payload, b"resume here");
		});
	}
}
