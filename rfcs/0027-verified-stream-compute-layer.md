# RFC 0027: A verified stream-compute layer (`dataflow!`) for `nirdosha-rt`

Status: **draft** — nothing built. This document is a proposal and a
work-item list, not a claim of review or approval.

## Motivation

Nearly every workload this project actually serves today is web-centric:
request/response handlers behind the compiled `serve` router (RFC 0010),
CRUD screens and dashboards (`crud_screens!`/`dashboard!`), HTTP + JSON +
db. That surface is covered well. What it cannot express safely today is
everything that happens *because of* a request but must not happen
*synchronously inside* it:

- **Async fan-out** — a payment webhook arrives; the response must return
  200 immediately, but enrichment, fraud scoring, notification, and
  ledger projection must all happen, in order, exactly once, and
  survive a process restart mid-flight.
- **CQRS projections** — read models feeding `dashboard!`/`crud_screens!`
  that are derived from a write log and must be rebuildable by replay.
- **Audit and lineage taps** — every state change flowing into the
  metadata plane (RFC 0026) without hand-written plumbing per event.
- **Exactly-once side effects** — the outbox/relay pattern, where the gap
  between "commit the row" and "send the message" is exactly the gap
  `docs/CONCURRENT_STATE_PATTERNS.md` warns corrupts naively written
  concurrent code in every language.

The mainstream answer is a Flink-class streaming runtime. Flink itself is
unusable here — JVM, GC pauses, a runtime no compiler can prove anything
about, and a wall-clock-coupled watermark model that would void the
determinism the toolchain already ships (per-thread seeded RNG, hash-bound
certificates, deterministic report bytes — `docs/nirdosha-rt-dialect.md`
§6). But the *design* of Flink — operators as pure functions over a
durable, replayable log, state as a deterministic fold, checkpoints as
barriers, backpressure as bounded channels — is the one distributed
paradigm that maps onto what `nirdosha-rt` already proves:

- Operators are plain fns → the existing contract system
  (`effects`, `requires`, `nfr`) and the driver's MIR passes
  (interprocedural purity with real `DefId` resolution, numeric proof
  discharge, `resource(..)`/`sequence(..)` dataflow analyses —
  `crates/nirdosha-driver/src/dataflow.rs`, `sequence_dataflow.rs`)
  apply to them unchanged in principle.
- Dataflow edges are message-passing over bounded channels → the dialect
  already refuses raw threads and raw locks; a streaming topology is a
  graph of the *only* concurrency the language permits, so the race and
  deadlock guarantees (rows 2–3 of `docs/goal.md`) cover the whole
  compute layer, not just one program.
- Deterministic operators + durable log + offset-ordered commit →
  exactly-once becomes a *checked claim* ("sink commit follows log
  offset commit on every path"), not a runtime hope — and it is the
  same shape as the `sequence(before, after)` analysis issue #76
  already built.

So this RFC proposes a Flink-*style* stream-compute layer for the v2
Rust dialect, declared with a proc macro, verified by the rustc driver,
and executed by a `nirdosha-rt` runtime extension. Web-centric first:
this is per-service stream processing (one service's events, one
node's topology), not big-data cluster computing. Cluster-scale
partitioning is an extension the design must not foreclose, but is
explicitly out of the first implementation.

## Design

The standing design rule from `docs/nirdosha-rt-dialect.md` governs
everything below: **no bespoke parser, ever.** Every guarantee must be
real stock `rustc` work — the type system, proc-macro expansion, or
const-eval — and every claim must degrade to `compile_error!` or a
driver refusal, never to a lint someone forgot to run.

### 1. The surface: `dataflow!`

One declarative macro, following the precedents `roles!`, `dashboard!`,
`crud_screens!`, and `categorical_actions!` already set (declarative
surface, macro emits the wiring, plain `cargo` compiles the result,
`cargo nirdosha` verifies it):

```rust
nirdosha_rt::dataflow! {
    // sources
    source payhooks:  HttpWebhook<PaymentEvent> = "/hooks/payments";
    source ledgerlog: DurableLog<LedgerTxn>     = "ledger";

    // operators: ordinary fns with contracts, referenced by name
    #[contract(effects(pure), deterministic)]
    fn enrich(ev: &PaymentEvent) -> Enriched { ... }

    operator by_account: keyed(by = "account_id");
    operator per_minute: tumbling(60s, event_time = "occurred_at",
                                  late_data = SideOutput(late_sink));

    // sinks
    sink alert_store: effects(db), exactly_once(key = "alert_id")
        = |agg: &WindowAgg| insert_alert(agg);
    sink metrics: effects(http), at_least_once = |agg: &WindowAgg| post_metrics(agg);
}
```

Properties that are structural, not stylistic:

- **Operators are plain `fn`s.** They take references, return values, and
  carry `#[contract(..)]` exactly like today's request handlers. This is
  deliberate: it sidesteps "async fns are not yet contract-verified"
  (the macro refuses async contracts today) — the *runtime* drives
  scheduling; operator bodies stay synchronous, checkable MIR. An
  operator that needs I/O declares it in `effects(..)` and the driver
  checks the transitive closure, the same ladder the honest matrix in
  `docs/nirdosha-rt-dialect.md` §5 already advertises.
- **`deterministic` is a positive claim, checked.** Today's impure
  scanner rejects `std::fs`, `Instant`, `.spawn()` inside `effects(pure)`
  bodies. The dataflow layer adds the inverse discipline: an operator
  claimed `deterministic` (the default for anything feeding keyed state
  or windows) must not read wall-clock, OS entropy, or unseeded RNG on
  any path the driver explores — checked over MIR with real name
  resolution, so a renamed wrapper cannot smuggle `Instant::now` past
  a text scan. Randomness that is genuinely needed goes through a seeded
  generator whose seed derives from the input record's log offset, which
  is what makes replay byte-identical.
- **Windows and late data are enums, not config folklore.** The closed
  set `{Drop, SideOutput(sink), Emit}` is a real `enum` in
  `nirdosha_rt`; a `match` over it is exhaustive by construction (row 11
  of `docs/goal.md`). Timers are registered through the runtime's
  logical clock (event time + watermark), never `Instant`.
- **Delivery semantics are per-sink, declared, and checked.**
  `exactly_once(key = "...")` expands to the two-phase pattern the
  runtime provides: the sink writes its effect idempotently under the
  declared key, then commits the source log offset. The driver checks
  the ordering half as a MIR dataflow claim — *sink commit follows
  offset commit on every path reaching the operator's return* — which
  is precisely the "has `before` definitely already run" analysis
  `sequence_dataflow.rs` already implements, with `before` = offset
  commit and `after` = sink effect. `at_least_once` skips the ordering
  obligation and the runtime deduplicates at the key.
- **Sources are typed.** `HttpWebhook` integrates with the compiled
  `serve` router and inherits RFC 0022's hardening (cookies, CORS,
  rate limiting, DPoP) by construction — a webhook source *is* a route.
  `DurableLog` is an append-only offset log following the native
  `transact` design's WAL/`txn_id` crash-replay discipline
  (specified in the archived `docs/TRANSACT.md`, retained here as the
  normative reference), embedded in-process for the single-node scope.

### 2. The runtime: `nirdosha-rt` extension crate

A new module in `crates/nirdosha-rt` (not a new toolchain crate — the
runtime is where types already live):

- **Mailbox-per-operator execution.** Each operator is a runtime-managed
  task with a bounded mailbox; edges are the only communication. Bounded
  mailboxes give credit-based backpressure for free: a slow sink fills
  its upstream mailboxes and the pressure propagates to the source,
  which stalls its log reads. No unbounded queues anywhere is an
  invariant the macro enforces at expansion time (buffer sizes are
  generated constants, not constructor args a caller can pass `usize::MAX`
  to).
- **Checkpoint coordinator.** Periodic barrier injection through the
  topology; on barrier arrival each operator snapshots its keyed state
  to the state store and acks. Because operators are driver-verified
  deterministic and RNG is offset-seeded, restore-from-checkpoint plus
  log replay reproduces state exactly — the determinism property the
  native toolchain already tests (`crates/runtime-kernels` seeded RNG,
  deterministic certificate bytes) extended from one process to a
  topology.
- **State handles are affine-shaped.** Keyed state is obtained from the
  runtime by handle, has a lifecycle (acquire → snapshot → release), and
  is not `Clone`; the shape mirrors the `box`/`db` handle discipline of
  the native language and keeps state ownership a type-level fact.
- **Logical time only.** Watermarks are derived from source offsets and
  declared event-time fields, never from `Instant`. This is what keeps
  savepoint restore reproducible: replaying the same log range yields
  the same windows.

### 3. What the driver verifies (rustc + `rustc_private`, Stage 2)

New checks land as passes beside the existing ones in
`crates/nirdosha-driver/src/`:

1. **Determinism pass** — extension of the impure scanner from text
   paths to resolved `DefId`s over the operator's transitive closure:
   wall-clock, OS entropy, unseeded RNG, and ambient global mutation are
   refusals with the chain named (the `net_total -> ledger_overrides ->
   read_ledger -> std::fs::read_to_string` error style, already shipped
   for purity).
2. **Commit-order dataflow check** — for every `exactly_once` sink, a
   `rustc_mir_dataflow` forward analysis proving the offset commit
   dominates the sink effect on every path. Reuses the negated-fact
   lattice trick `sequence_dataflow.rs` documents; a sink body where the
   effect can run before (or without) the offset commit is a refusal.
3. **Numeric window proofs** — existing `nirdosha-smt-core` discharge
   applies to window arithmetic (counts, sums, bounds on keyed
   aggregations) for the same Tier-1/Tier-2 discipline as
   `docs/MIR_NUMERIC_PROOFS.md`; `effects(pure)` + checked arithmetic in
   an aggregation operator is the target case.
4. **Dialect rules, unchanged** — unsafe, raw threads, raw locks inside
   any operator or sink remain build errors, now with the topology name
   in the span context.

### 4. Certificates and the guarantee bundle

The topology is data, so it goes in the attestation artifacts: the
`nirdosha.certificate/v1` report gains a `dataflow` section (sources,
operators, per-operator contract, delivery semantics, edge buffer
bounds, the macro expansion's generated-source hash), and the guarantee
bundle (`target/nirdosha/guarantees-<pkg>.json`) gains
`dataflow: { deterministic_operators: [...], exactly_once_sinks: [...],
checkpoints: {...} }`. Two properties are preserved by construction:
certificates stay deterministic (topology is declared, not discovered at
runtime), and `verify --audit` binds the topology's source hash like any
other listed source.

### 5. Why this is web-centric first

The motivating workloads in §Motivation are all single-service,
single-node: one webhook endpoint, one ledger log, a handful of
operators, two sinks. The design does not pay for cluster machinery it
won't use — no job manager, no network shuffle, no external state
backend in v1 — but every choice survives scaling: operators are
already pure and keyed, edges are already the only coupling, and
partition-aware routing of `keyed(by = ...)` operators is a runtime
deployment change, not a language change. Multi-node is an open
question below, deliberately.

## What must change

In dependency order, each item concrete enough to start from:

1. **`nirdosha-contract-core`** — extend the contract model with
   `deterministic`, `exactly_once(key)`, `at_least_once`, and the
   `sequence(..)`-style `commit_order` claim shape; `deny_unknown_fields`
   so old verifiers refuse rather than ignore new clauses.
2. **`crates/nirdosha-macros`** — the `dataflow!` macro: parse the
   topology, type-check source/sink generics against the declared
   event types at expansion time, emit the runtime wiring (mailboxes,
   barriers, state handles) plus the doc encoding so the topology
   travels into rustdoc JSON like contracts do today.
3. **`crates/nirdosha-rt`** — runtime: mailbox executor, bounded-channel
   edges, checkpoint coordinator, embedded offset log, logical-time
   watermark builder, idempotent two-phase sink commit helper, the
   `LateData` enum.
4. **`crates/nirdosha-driver`** — determinism pass over resolved MIR
   (item 1 of §3), commit-order dataflow check (item 2, reusing
   `sequence_dataflow.rs`'s lattice), window-arithmetic proof records in
   the MIR certificate; async operator support is *not* required, by
   design.
5. **`crates/cargo-nirdosha`** — include the `dataflow` section in
   certificates and the guarantee bundle; `verify --workspace` covers
   topology-bearing crates; `NIRDOSHA_STRICT=1` gains "every operator
   carries a contract."
6. **Compiled-serve integration** — webhook sources register as routes
   under the RFC 0010 deny-by-default exposure model and RFC 0022's
   transport hardening.
7. **Examples + tests** — a `rt-payroll-streaming` example mirroring
   `rt-payroll`: webhook in, enrichment operator (with a lying twin that
   smuggles `Instant::now` past the text scanner and is refused by the
   driver's resolved-DefId check — the same demo shape as
   `rt-payroll-pure-chain`), exactly-once projection sink, checkpoint/
   restore test asserting byte-identical state after replay.

## Effect on the permission model

Real, and mostly inherited. `requires(role/claim)` applies to operators
and sinks exactly as to handlers: an `exactly_once` sink writing payroll
rows expands to take the minted `RoleProof` parameter, so an operator
wired to it that cannot hold the proof fails to type-check at expansion
time. Sources exposed over HTTP are routes, so RFC 0010's
deny-by-default-on-mutation exposure model and RFC 0022's
`requires(claim = ..)` authorization apply without new mechanism. One
genuinely new surface: **read-side propagation** — an operator reading a
stream produced upstream of a role-gated source must itself carry the
proof if its output is served, and the macro should propagate that
requirement through the topology at expansion time (type-inference
style, over the declared edges). Whether propagation is macro-time
(eager, simple) or driver-checked (lazy, precise) is an implementation
decision; the obligation itself is not optional.

## Relationship to RFC 0021.b (the workflow/approval runtime)

The stream layer and the workflow runtime share the same philosophy —
outbox staging, idempotency keys, content-addressed definitions, durable
ledgers — so they compose cleanly, with one hard boundary.

**What moves to the stream layer.** Side-effect delivery today is a
manual lifecycle: intents are staged in `wf_outbox`
(`crates/nirdosha-workflow/src/outbox.rs`), an external poller claims
them under 1–300s leases, and a crash mid-delivery leaves
`outcome_unknown` requiring operator `reconcile_outbox`. The outbox
table is already a durable, offset-ordered log — making it the
`DurableLog` source of a small topology, with delivery sinks declared
`exactly_once(key = "outbox_id")`, replaces that poll/lease/reconcile
loop with a driver-checked commit-order claim. The same applies to
everything currently done by scheduled scans of `wf_instances`:
per-state SLA deadlines and escalation timers become event-time windows
over the transition/decision stream (`wf_audit` is already written per
event), and case analytics (aging, turnaround, quorum bottlenecks)
become replayable `dashboard!` projections instead of hand-written
queries. Cross-instance rules the runtime cannot express — duplicate-case
detection, concurrent-case caps per customer — get a mechanism in keyed
stream joins, which per-instance invariant predicates
(`Runtime`'s `invariants()` check) structurally cannot provide.

**What must not move.** The decision core stays transactional:
transitions, quorum, maker-exclusion, and revision-conflict checks are
synchronous per-instance consistency requirements, and stream delivery
is eventual — an approval decision routed through a mailbox has a
window in which two concurrent decisions could both pass quorum. The
state machine keeps its `BEGIN IMMEDIATE` transaction semantics
untouched; the stream layer consumes its *event log*, never its write
path.

## Where this lands across the rest of the system

Surveying the existing subsystems against this design, the pattern repeats:
several of them already *are* hand-rolled, less-safe fragments of the same
idea (append-only logs, replayable projections, outbox delivery), and a few
have synchronous paths the stream layer must never touch.

**Already-real integration points — the stream layer formalizes what
exists:**

- **`nirdosha-audit`'s hash chain is a ready-made `DurableLog` source.**
  `AuditEnvelope` (`kind: Decision|Mutation|Lineage|...`) is an
  append-only, hash-chained log with per-entry `seq`/`prev_hash`. A
  topology reading module chains by offset gets replayable decision and
  mutation streams for free — no new instrumentation.
- **`nirdosha-graph::streams` is prior art for this RFC's core contract.**
  `store/streams.rs` already implements contiguous per-stream sequences
  with idempotency receipts ("SEQUENCE_CONFLICT" / "OUT_OF_ORDER") — an
  offset log + idempotent sink, hand-built. The RFC generalizes it into
  a language-level, driver-checked claim.
- **The lineage projection is a replayable fold missing its machinery.**
  `project_observations` is a pure function over the chain order (same
  observations in, same edges out), and MD-1 already states the doctrine
  ("log is truth, graph is a rebuildable projection"). What it lacks —
  offsets, checkpoints, event time (timestamps are wall-clock today,
  and late/out-of-order observations merge silently) — is exactly what
  the runtime provides.
- **`guard-verify` becomes incremental.** Today it is a point-in-time,
  whole-catalog batch validator (V1–V8 passes). Over the policy-change
  log it becomes a continuously maintained, replayable projection.
- **`AsyncScoringDepth` gets an engine.** The admission controller names
  an async work class that currently has no async execution engine; a
  sheddable side-topology is its obvious home.
- **Case/workflow integration** (the section above): `wf_outbox` as
  `DurableLog` source, escalation as event-time windows.

**Stubs and losses the stream layer fixes:**

- **Lineage sinks are stubs.** `HttpLineageSink::flush()` currently
  *clears the batch without sending anything*; `JsonlLineageSink`
  reopens and appends per observation. Real exactly-once sink drivers
  replace both, with delivery semantics in the certificate.
- **`presence-gateway` is the single best loss story.** Delivery is
  explicitly at-most-once fire-and-forget ("no persistence for a late
  subscriber"), and no presence event is persisted anywhere. A durable
  log behind the publish path with the WebSocket send as a keyed
  exactly-once sink converts this to replayable delivery; presence
  state becomes a keyed projection.
- **`ChainReconciler` is a hand-rolled merge operator.** It merge-sorts
  module chains by `(wall_clock, module, seq)` with no total-order
  guarantee on timestamp ties. Offset semantics fix the ordering; the
  reconciler becomes a merge/window operator that restarts cleanly.
- **`nirdosha-graph`'s analysis is full-graph batch** with a hard
  1000-finding cap; findings could be maintained incrementally per
  `(stream_id, sequence)` instead.
- **`isolation-core` detects anomalies offline.** Its op history is
  already an ordered effect log; streaming the log turns
  a-posteriori batch detection into continuous detection.
- **`nirdosha-hi` prompt/response events vanish today** (only MCP tool
  calls are logged); a lineage tap over the LLM call stream captures
  the highest-cardinality audit surface the system currently loses.

**Cross-RFC interactions:**

- **RFC 0007 (APM kernel):** the stream layer is the natural
  *cross-process* consumer of NFR/telemetry events; per-request
  admission control stays in-process (same boundary rule as the MIC).
- **RFC 0011 (service providers):** a `DurableLog` source or lineage
  sink plugs in as another provider behind a port, per the two-driver
  rule.
- **RFC 0017 (guarantee manifest):** the `dataflow` certificate section
  slots into the same manifest/`verify-binary` machinery, so topology
  guarantees are checked where capability ceilings are checked today.

**Bad-fit flags — synchronous, transactional paths that must stay off
the stream:** `GuardClient::guarded_apply`'s inline
evaluate→audit→commit sequence (a test pins "audit before commit" as
load-bearing), the 30s `DecisionCache`, the graph journal's
generation-checked `prepare`, and the workflow decision core (above).
The stream layer consumes these components' *logs*; it never sits in
their write paths.

## What we take from Flink's distributed model — and what we don't

Flink's contribution to distributed computation is not any single feature
— it is the first mainstream engine to take *continuous, unbounded*
computation as the primary model and make it correct, recoverable, and
operable. The capabilities, and this RFC's position on each:

| Flink capability | Adopted in this RFC? | Position |
|---|---|---|
| Checkpoint barriers (Chandy–Lamport) over operator state | Yes — runtime core | Same design, single-node scope |
| Two-phase-commit sink alignment | Yes — but compile-time checked | Flink trusts the operator author to implement commit ordering; here the driver proves it in MIR |
| Event time + watermarks | Yes — logical time only | Flink's watermarks are wall-clock-coupled; banning wall-clock keeps replay byte-identical |
| Massive keyed state, TTL'd and snapshotted | Yes — affine-shaped handles | Ownership discipline instead of backend config |
| Credit-based backpressure | Yes — bounded mailboxes | Same mechanism; buffer bounds are macro-generated constants, not tunables |
| Savepoints, rescaling, slot allocation | **No — v1 out of scope** | Carries a scale argument, not a correctness argument; adopted later as a deployment change, not a language change |
| Table API/SQL, CEP | No | The `dataflow!` macro surface is the dialect's answer to declarative |
| Iterative dataflows (cycles) | **No — deliberately rejected** | Cycles reintroduce the ordering hazards bounded channels cannot reason about; topologies are DAGs in v1 |

### Why the dataflow model composes with the ownership / race / deadlock rows

Flink's effect on these three concerns is asymmetric — strong on races,
structural on deadlock, weakest on ownership — and the asymmetry is
exactly where the type system has to add what a JVM framework cannot:

**Races.** Flink eliminates shared-memory races by construction: each
parallel subtask owns its state partition and processes records
single-threaded, key-partitioning serializes concurrent events for the
same key, and there is no shared mutable memory between operators at
all — an in-flight record has exactly one owner. What Flink leaves open:
races with *external* systems (two sinks writing the same external row),
and races the application writes inside a UDF (shared mutable objects,
`static` caches, aliased records sent to two side outputs). The RFC
closes the first with the driver-checked `exactly_once` commit order,
and the second is unrepresentable — the dialect bans `unsafe` and raw
shared mutation outright, so "user wrote a race in an operator" is a
compile error, not an incident.

**Deadlock.** There are no locks in the dataflow model, so no wait-for
cycle can form — the same argument as rows 2–3 of `docs/goal.md`, and
the guarantee is identical. What Flink relocates rather than removes:
barrier-alignment stalls (an operator waits for barriers from all
inputs), hung two-phase commits against external systems, and blocking
calls inside UDFs. The RFC's position is *visibility*: a blocking call
in an operator is a driver refusal (impure effect / dialect rule), not
a stall discovered in a flame graph, and bounded queues are enforced at
expansion time, so unbounded-buffer liveness collapse is not a config
mistake a user can make. One deliberate divergence: v1 topologies are
DAGs, so iteration-feedback deadlock is unrepresentable rather than
merely unlikely.

**Ownership — Flink's weakest point, and the largest delta here.**
Flink's ownership is operational convention, not enforcement: state is
framework-owned, but operators can close over shared mutable objects,
mutate records handed to multiple side outputs, and leak backend handles
across restarts; nothing in the type system stops any of it. The RFC
makes ownership load-bearing in three ways: state handles are affine
(acquire → snapshot → release is a type-level lifecycle, not a TTL
config), operators take `&T` and return owned values (no API surface to
alias a record into two mutation sites), and — the part that composes —
an operator that cannot mutate shared state *and* cannot read wall-clock
is replayable, which is what makes checkpoint restore byte-identical.
Flink can checkpoint state it cannot prove deterministic; here,
determinism is the checked precondition for state that will be restored.

The pattern across all three: Flink's model makes the right thing the
path of least resistance; this RFC makes the wrong thing
unrepresentable. That is the difference between a framework convention
and a language guarantee, and it is why the proposal insists on the
rustc driver rather than a runtime library.

## Compatibility

Additive. `dataflow!` is a new macro; the new contract clauses are new
optional keys, refused on typo by `deny_unknown_fields` exactly as
today. Existing programs are untouched: no grammar change (the dialect
*is* Rust), no runtime cost unless the macro is used, plain `cargo`
still compiles everything (the macro's honesty checks fire under plain
cargo too, per the `effects(pure)` precedent). The one behavioral
surface to be careful with is `NIRDOSHA_STRICT=1` workspaces, which
gain a new obligation ("operators carry contracts") — scoped to crates
that actually declare a topology, and a major-version-style note in the
strict-mode docs, not a silent tightening.

## Rejected alternatives

- **Adopt or embed Apache Flink (or a JVM runtime).** Violates the
  standing design rule outright: GC pauses and an unverifiable runtime
  inside a toolchain whose category claim is "guarantees about the
  language, not the model." Also voids deterministic certificates. The
  design ideas are adopted; the codebase is not — the section above
  ("What we take from Flink's distributed model") is the per-capability
  adopt/reject map, and the ownership discussion there is why "Flink
  the framework" could never supply what the driver checks here.
- **`async`/futures-based stream combinators without driver checks.**
  Idiomatic Rust, and exactly the adoption trapdoor the dialect exists
  to close: a topology built from `StreamExt` combinators has no
  declared shape to verify, no checked determinism, no certifiable
  delivery semantics. Lies would be invisible to Stage 2.
- **Build this only in the native `.nir` compiler.** The `.nir`
  frontend is the reference implementation of the spec, but the
  adoption wedge is the Rust dialect — new compute capability that
  only exists in `.nir` contradicts the strategy in
  `docs/nirdosha-rt-dialect.md` §8. The native compiler may inherit
  the design later; the RFC does not depend on it.
- **Process-per-operator with raw threads.** The dialect refuses raw
  threads and locks; a compute layer that required `unsafe` exemption
  to exist would be paying for cluster assumptions in a single-node
  scope. Runtime-managed mailboxes give the same isolation with the
  race/deadlock guarantees intact.

## Open questions

- **State backend for v1** — in-memory with snapshot-to-disk, or the
  embedded store that already exists behind the guard kernel? Leaning
  in-memory + snapshots (single-node scope), but checkpoint latency
  numbers could flip this.
- **Multi-node** — out of scope for v1 by design; the open question is
  only whether the `DurableLog` source abstraction should reserve a
  partition/shard parameter now to avoid a surface change later.
  See [RFC 0027.a](./0027.a-cluster-native-dataflow.md) for the
  cluster-native amendment: identical processes over a partitioned
  durable log, no task-manager/worker split.
- **`nfr` on streams** — `nfr(latency_ms)` on an operator currently
  means per-call measurement, which is meaningless for a continuously
  running operator. A queue-depth/end-to-end-lag NFR clause needs new
  semantics before it means anything; v1 should probably omit it rather
  than ship a hollow claim.
- **Interaction with RFC 0011** (uniform service-provider model) — if
  sources/sinks become provider-backed (a Kafka-shaped log behind
  `DurableLog`), the exactly-once ordering obligation moves partially
  into the provider. Whether the contract language needs a
  `provider(...)` clause then is deferred until RFC 0011 lands.
- **Watermark policy as data** — declaring "event time may lag wall
  clock by at most X" per source is easy; checking that the declared
  field actually is a timestamp and monotonic-per-key is a driver pass
  whose cost is unestimated.
