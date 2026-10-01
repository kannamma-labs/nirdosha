# 0022: Async/networked policy store — spawn_blocking AsyncStoreDriver bridge, WebSocket snapshot push, live GuardClient hot-reload

Date: 2026-09-21
Status: accepted

## Context

`GuardClient`/`StoreDriver` are synchronous throughout, and stay that way
— every other phase this session built (Phases 7-17) depends on it, and
nothing about "async uplift" should require rewriting that surface.
Policy snapshots (`nirdosha-guard-core/src/snapshot.rs`'s
`PolicySnapshot`/`PolicyStore`/`InMemoryPolicyStore`) are genuinely
`[DONE]` — real, tested types — but had no network transport, so a policy
change required a redeploy, not a hot reload. `tokio` is already a
workspace dependency (`nirdosha-rt`, `crates/presence-gateway`), so this
follows existing precedent rather than introducing a new async runtime
choice.

## Decision

**`AsyncStoreDriverAdapter<D>`, new in `nirdosha-guard-mic::async_driver`** —
the plan's own explicitly-sanctioned option ("Add an async `StoreDriver`
variant (**or an async wrapper trait**) alongside the sync one"). Wraps any
real `D: StoreDriver` and offloads every call to
`tokio::task::spawn_blocking` — genuine thread-pool offload of synchronous
(and, for `PostgresStoreDriver`, real blocking I/O) work, not a fake
`async fn` that never actually awaits anything. `AsyncStoreDriver`'s
`manifest()` returns an owned `CapabilityManifest` (not `&CapabilityManifest`
like the sync trait) — a reference tied to `&self`'s lifetime can't cross
the `spawn_blocking` boundary, which needs `'static` owned data to move
onto the blocking pool.

**`nirdosha-guard-snapshot-ws`, new crate: real WebSocket snapshot
push/pull**, matching `crates/presence-gateway`'s existing transport
choice (`tokio-tungstenite`) — this workspace's only other real-time push
service — for consistency rather than a second, independent transport
decision. `SnapshotServer::publish` broadcasts a real `PolicySnapshot` to
every connected client; a newly-connecting client is sent the current
snapshot immediately (not just future updates) so a client that connects
between publishes still learns real current state.
`connect_and_apply(url, on_snapshot)` is the client half — every received
snapshot (including the connect-time one) invokes a caller-supplied
callback, wired to `PolicyStore::replace`'s existing change-callback hook.

**`PolicySnapshot` still carries a version/hash fingerprint, not the
policy set itself** — that type's own pre-existing, established design
(unchanged by this phase). A real deployment's `on_change` callback reacts
to a new fingerprint by re-fetching the corresponding policy content from
wherever it's authoritative; this crate's job is getting that fingerprint
to a running `GuardClient` promptly and reliably, not owning how policy
content itself is distributed.

**`GuardClient::apply_policy_snapshot`, new in `nirdosha-guard-mic`** — the
real effect a live `on_change` callback needs to produce for "picks up a
new policy version without restart" to mean anything beyond metadata:
atomically swaps the active policy set. Also invalidates the decision
cache on swap — a real correctness fix found while building this (a stale
cached decision from the old policy set could otherwise be served after a
swap that should have changed it), not something the pre-existing
`DecisionCache` needed until a policy set could change underneath it at
runtime, which nothing did before this phase.

## Consequences

**What this makes possible.** `cargo test -p nirdosha-guard-mic
async_driver` proves the adapter genuinely round-trips a real write/read
through `spawn_blocking` and correctly surfaces a real driver rejection
(not a panic) across the async boundary. `cargo test -p
nirdosha-guard-snapshot-ws` proves the real WebSocket transport itself
(connect-time snapshot delivery, live publish reaching an already-
connected client) and — the plan's own specific verification target — a
full integration test (`tests/guard_client_hot_reload.rs`) with a real
`GuardClient` that genuinely denies a request under an empty policy set,
receives a real pushed `v2` snapshot over a real WebSocket connection,
and then genuinely allows the same shape of request without any restart —
while the audit chain still shows the original `v1` decision under its
original `policy_version`, proving I4's replay guarantee survives a live
reload rather than being silently invalidated by it.

**What this does *not* make possible, stated so it's never misread later.**
`PostgresStoreDriver` itself was not rewritten to use `tokio-postgres` —
`AsyncStoreDriverAdapter` makes any existing sync driver usable from an
async caller without that rewrite, which stays a separate, larger
undertaking if a fully-async Postgres path is ever needed. `SnapshotServer`
has no authentication/authorization of its own (any TCP client that can
reach it can subscribe to policy-version fingerprints) — a real production
deployment would need to layer that on, the same arm's-length relationship
`presence-gateway`'s own JWT verification already models for its transport.
No consumer-group/multi-server fan-out exists — one `SnapshotServer`
process, `broadcast::channel`-fan-out to its own directly-connected clients
only.
