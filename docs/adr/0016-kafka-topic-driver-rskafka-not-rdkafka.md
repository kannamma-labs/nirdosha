# 0016: `StreamSource`/`StreamSink` `TopicDriver` — `rskafka` instead of `rdkafka`, poll-based instead of a boxed `Stream`, Redpanda for the dev/test broker

Date: 2026-09-21
Status: accepted

## Context

`rfcs/0025-nirdosha-rtm-ecosystem.md` §8.1 declares real trait shapes
(`StreamSource::subscribe(...) -> Result<BoxStream<'static, RawBatch>, PortError>`,
`StreamSink::publish`) and names a Kafka driver (built on `rdkafka`) as the
worked example, but no implementation existed anywhere in the workspace —
the RFC's own code block is illustrative async Rust (`catalog::external_name`,
a bare `StreamConsumer`, no real broker config), not a compiling type.

## Decision

**New crate `crates/nirdosha-ingestion-topic-kafka`**, same naming
convention as `docs/adr/0013`/`0014`/`0015`.

**`rskafka`, not `rdkafka`.** `rdkafka` wraps `librdkafka`, a C library
requiring `cmake` and a from-source native build unless a system package is
already present — a real, disk- and build-time-heavy dependency this
sandbox's already-tight root filesystem (documented in this session's Plan
Phase 10-12 commits: a `cargo clean` was needed mid-session after the
workspace's accumulated build-profile variants filled the disk) could not
safely absorb on top of `ort`'s own binary download (`docs/adr/0015`).
`rskafka` implements the real Kafka wire protocol directly in Rust, no C
toolchain, no native build step. It talks to any real Kafka-protocol
broker — this driver has no broker-vendor-specific code path, so choosing
`rskafka` over `rdkafka` is an implementation detail behind the
`StreamSource`/`StreamSink` traits, not a capability difference a caller
would ever observe.

**Poll-based (`poll_batch(topic, from_offset, max_wait_ms) -> RawBatch`),
not a boxed `Stream`.** The RFC's `BoxStream` return type hides real
backpressure/cancellation/offset-tracking design questions behind a
`Stream` object. `rskafka::client::partition::PartitionClient::fetch_records`
is itself a plain request/response call — `poll_batch` exposes that
directly, with the caller controlling the offset explicitly (the same
"opaque cursor, not something the caller computes independently" posture
I10/§8.4 already established for read-path pagination, applied here to a
different port). A `Stream`-returning wrapper looping `poll_batch` is a
thin, addable-later adapter; inventing one now only to satisfy the RFC
illustration syntactically would be an aspirational surface with no real
behavior behind the parts a caller can't exercise (cancellation,
backpressure) — exactly what this workspace's other drivers avoid.

**Redpanda for the dev/test broker** (`docker-compose.dev.yml`'s new
`redpanda` service, pinned by image digest — the same convention the
`postgres` service already uses), not `confluentinc/cp-kafka`. A single
self-contained binary needs no separate ZooKeeper/KRaft controller
container, lighter for this sandbox, and speaks the real Kafka wire
protocol `rskafka` targets — this driver has no Redpanda-specific code, so
a real Kafka broker is a drop-in swap (the same "vendor swap" pattern
`rfcs/0025-nirdosha-rtm-ecosystem.md` §8.1 itself describes: "Kafka →
Pulsar: new driver crate... untouched").

**Single partition per topic, replication factor 1.** A real dev/test
topology, not a claim about production sizing — `create_topic` is called
idempotently (an "already exists" error from a prior run is expected and
swallowed, any other error is not) and cached per topic in a
`tokio::sync::Mutex<HashMap<..>>` so the real broker round-trip only
happens once per topic per driver instance.

## Consequences

**What this makes possible.** `cargo test -p nirdosha-ingestion-topic-kafka
-- --ignored` against a real Redpanda broker
(`docker compose -f docker-compose.dev.yml up -d redpanda`) proves: a
published batch is really written to the broker and reappears on a fetch
from offset 0 with the same content and offsets `produce` returned, and
`poll_batch` from an explicit non-zero offset returns only the records
from that point forward — real resumable consumption, not a stub.

**What this does *not* make possible, stated so it's never misread later.**
`stream_port!` is still not a callable macro — a `guard_policy!`-declared
`port txn_in { bind ... }` has nothing to bind this driver to yet, the same
class of grammar-mismatch gap `docs/adr/0014`/`0015` document for
`matcher!`/`model_artifact!`. No consumer-group/rebalancing support exists
— this driver hands a caller an explicit offset it fully controls, so
group coordination (if ever needed) is a distinct future concern layered
on top, not something silently assumed here.
