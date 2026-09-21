# RFC 0027 Implementation Plan

Companion to [`0027-verified-stream-compute-layer.md`](./0027-verified-stream-compute-layer.md).
File-level plan: what changes, what gets created, in dependency order.
Anchors (file:line, fn names) are from the current `v2` checkout and
will drift — `grep -n` the name, don't trust the offset.

Legend: **[M]** modify · **[C]** create · **[D]** docs-only.

---

## Phase 0 — Contract model (`nirdosha-contract-core`)

Everything downstream parses, emits, or verifies these clauses, so the
model lands first.

**[M] `crates/nirdosha-contract-core/src/model.rs`**
- Add to `pub struct Contract` (model.rs:26): `deterministic: Option<bool>`,
  `delivery: Option<Delivery>` where `#[serde(deny_unknown_fields)] pub struct Delivery { pub exactly_once: Option<ExactlyOnce>, pub at_least_once: Option<bool> }` and `pub struct ExactlyOnce { pub key: String }` — mirrors the `Nfr` pattern (model.rs:137), flat struct, no untagged enum.
- Extend `Contract::validate()` with the new clauses' cross-checks:
  `deterministic` implies a declared or inferred pure effect set (forbid
  `deterministic` + non-pure `effects` unless explicitly annotated);
  `exactly_once` and `at_least_once` are mutually exclusive; `exactly_once.key`
  non-empty. Refusals, never silent drops — same discipline as the NFR
  gate in `nirdosha-macros/src/lib.rs` expand().
- Round-trip tests beside the existing ones (~model.rs:278): token form →
  JSON → token form for all new clause shapes.

**[M] `crates/nirdosha-contract-core/src/parse.rs`**
- Extend `parse_contract`'s token grammar with `deterministic`,
  `exactly_once(key = "...")`, `at_least_once` — keyword args follow the
  existing `nfr(latency_ms = 50, ...)` parsing shape.

**[M] `crates/nirdosha-contract-core/src/docparse.rs`**
- `parse_doc` (docparse.rs:18) picks the new keys up automatically via
  serde, but add explicit negative tests: a typo'd `exaclty_once` is
  refused by `deny_unknown_fields`, not ignored.

**[M] `crates/nirdosha-contract-core/src/scan.rs`**
- `impure_calls_in_block` already heuristics-clocks; add the macro-time
  honesty check for `deterministic`: a `deterministic`-claiming fn whose
  body textually contains `Instant::now`/`SystemTime::now`/`thread_rng`
  is a `compile_error!` under plain cargo too (same precedent as the
  purity scan — fast local lie, deep check later).

**[M] `crates/nirdosha-contract-core/src/certificate.rs`**
- Add `dataflow: Option<DataflowSection>` to the certificate model so
  Stage-1 scanners can carry (and refuse-unknown) the section before the
  driver fills it.

No new files in this phase.

**Verify:** `cargo test -p nirdosha-contract-core`

---

## Phase 1 — `dataflow!` macro (`nirdosha-macros`)

**[C] `crates/nirdosha-macros/src/dataflow.rs`**
- `syn`-based parser for the topology DSL from the RFC §1: `source` /
  `operator` / `sink` items, `#[contract(...)]` attributes on operator
  fns, `keyed(by = "...")`, `tumbling(60s, event_time = "...",
  late_data = ...)`, `effects(db), exactly_once(key = "...")` on sinks.
- `expand(input: TokenStream) -> TokenStream` following the
  `dashboard::expand` delegation pattern:
  - Parse + validate: unknown keys = hard error; source/sink generic
    types resolved against declared event types; every operator and sink
    must carry `#[contract(..)]` (the strict obligation, enforced at
    expansion, not just under `cargo nirdosha`).
  - Emit the runtime wiring: one mailbox per operator with
    macro-generated bounded buffer sizes (generated constants, per the
    RFC — not constructor args), edge setup, state-handle acquisition,
    watermark/timer registration, and the `LateData` enum dispatch.
  - Inject the two-phase sink commit sequence into `exactly_once` sink
    bodies: runtime call `offset_commit(...)` *before* the sink effect,
    keyed by the declared `key` — this call order is what the driver's
    commit-order pass later proves, so the emitted code must make the
    ordering syntactically explicit.
  - Emit the topology as a static, content-addressable descriptor
    (`const` hash over the parsed topology) so certificates can bind it.
  - Emit the `#[doc = "nirdosha:contract …"]`-style topology encoding so
    the declared shape travels into the compiled crate.

**[M] `crates/nirdosha-macros/src/lib.rs`**
- Register `#[proc_macro] pub fn dataflow` (lib.rs:59-146 block, beside
  `dashboard`, `wizard`, etc.).
- In `expand()` (lib.rs:148): route the new contract clauses through the
  extended `cc::parse::parse_contract`; add the gate rejects — a
  `deterministic` claim on an `async fn` is refused (async contracts are
  not yet verified, dialect doc §4); a `deterministic` claim on a fn
  whose effects include `db`/`http`/`mq` is refused at macro time.

**Verify:** `cargo test -p nirdosha-macros`; a compile-fail trybuild-style
test if the crate already has that harness (check; if absent, cover via
the examples in Phase 5 and the demo script).

---

## Phase 2 — Runtime (`nirdosha-rt`)

New module; sits beside `resource.rs`/`nfr.rs` in the same
"recognized mint-point" idiom.

**[C] `crates/nirdosha-rt/src/dataflow/mod.rs`**
- Public surface: `Topology`, `TopologyBuilder` (what `dataflow!`
  expands into), `Source`/`Sink` traits, `LateData` enum
  (`Drop`/`SideOutput(SinkId)`/`Emit`), re-exports of the submodules.

**[C] `crates/nirdosha-rt/src/dataflow/mailbox.rs`**
- Bounded mailbox per operator; `send` back-pressures (blocking with a
  documented bound) instead of growing. One runtime-managed task per
  operator, driven by the topology builder — no user-spawnable threads
  (the dialect rules already refuse raw threads; this module must not
  re-expose them).

**[C] `crates/nirdosha-rt/src/dataflow/log.rs`**
- The `DurableLog`: append with monotonic offsets, fsync discipline
  (`synchronous=FULL` equivalent), `read_from(offset)` replay iterator,
  and the offset-commit primitive the injected sink sequence calls.
  Single-node embedded store first (RFC open question: in-memory +
  snapshot-to-disk vs. embedded KV — start in-memory with a
  snapshot-on-barrier file, behind a `StateBackend` trait so the choice
  doesn't touch operator code).

**[C] `crates/nirdosha-rt/src/dataflow/checkpoint.rs`**
- Barrier injection and completion tracking; per-operator state snapshot
  acks; savepoint = manually triggered barrier with a named handle;
  restore rewinds sources to offsets and rebuilds operator state, then
  resumes. Seeded-RNG discipline: per-record seeds derive from log
  offset (the RFC's determinism rule) — wire into the snapshot format so
  a restore cannot resurrect a different seed stream.

**[C] `crates/nirdosha-rt/src/dataflow/state.rs`**
- Affine-shaped keyed state handles: `StateHandle<K, V>` — no `Clone`,
  acquire → snapshot → release lifecycle, TTL as a lifecycle bound not a
  config string. Keyed by the `keyed(by = "...")` field; the handle
  refuses access outside the owning operator's task (runtime check, but
  the type shape makes the mistake visible).

**[C] `crates/nirdosha-rt/src/dataflow/time.rs`**
- Logical-time watermarks: derived from source offsets + the declared
  `event_time` field; no wall-clock input anywhere in the module. Late
  records route per the `late_data` declaration.

**[C] `crates/nirdosha-rt/src/dataflow/sink.rs`**
- The two-phase commit helper the macro injects:
  `offset_commit(log, offset)` then run the sink closure; idempotency
  keyed by the declared `exactly_once.key`; `at_least_once` variant
  skips ordering and dedups on the key. Also the `HttpWebhook` source
  adapter that pairs with compiled-serve (Phase 4).

**[M] `crates/nirdosha-rt/src/lib.rs`**
- `pub mod dataflow;` (lib.rs:46-60 module list) + prelude re-exports.

**[C] `crates/nirdosha-rt/tests/dataflow_runtime.rs`**
- Per-feature test file (repo convention: one file per feature in
  `tests/`): mailbox backpressure bound actually bounds; checkpoint →
  kill topology mid-stream → restore → byte-identical state (compare
  state hashes, the determinism-test precedent); watermark ordering with
  deliberately shuffled input; late-data enum dispatch for all three
  variants; state handle lifecycle (use-after-release panics visibly).

**Verify:** `cargo test -p nirdosha-rt --test dataflow_runtime`

---

## Phase 3 — Driver (`nirdosha-driver`, nightly + `rustc-dev`)

**[C] `crates/nirdosha-driver/src/determinism.rs`**
- The determinism pass: for fns whose contract claims `deterministic`,
  walk the transitive closure (reuse the `Effects::effects_of` machinery,
  main.rs:446) and refuse on resolved `DefId`s for wall-clock
  (`std::time::Instant::now`, `SystemTime::now`), OS entropy
  (`rand::thread_rng`, `getrandom`), and unseeded RNG — with the full
  chain named in the error, the `Impurity { reason, chain }` shape
  (main.rs:393). Stage-1's textual clock heuristics are the fast path;
  this is the resolved-name path that catches renamed wrappers (the
  `rt-payroll-pure-chain` demo pattern).

**[C] `crates/nirdosha-driver/src/commit_order.rs`**
- The exactly-once commit-order pass: `rustc_mir_dataflow` forward
  analysis proving the injected `offset_commit` call dominates the sink
  effect on every path through the sink body. Reuse the negated-fact
  lattice from `sequence_dataflow.rs` ("`before` has not yet run" as one
  tracked bit; union join is sound for the must-property). Report shape
  mirrors `dataflow.rs`'s `ResourceViolations` visitor
  (dataflow.rs:214,229). A sink where the effect can run before — or
  without — the offset commit is a refusal, spans from MIR.

**[M] `crates/nirdosha-driver/src/main.rs`**
- `after_analysis` (main.rs:133): discover `dataflow!`-emitted topology
  descriptors alongside the existing `#[doc = "nirdosha:contract …"]`
  discovery (main.rs:139-152 — same attribute-scan mechanism, a second
  doc prefix); run `determinism` and `commit_order` passes; fold results
  into the per-function report map.
- Numeric pass on window arithmetic: `numeric.rs` already discharges
  guarded arithmetic; ensure aggregation operator bodies (which the
  macro marks) are visited — likely zero changes, but add the operator
  marker to the discovery filter.

**[M] `crates/nirdosha-driver/src/std_effects.rs`**
- One-line effect-table additions if any clock/entropy function needs an
  explicit summary entry for the default-deny third-party rule
  (std_effects.rs:86 table).

**[M] `crates/nirdosha-driver/src/proof_certificate.rs`**
- `prepare` (proof_certificate.rs:33): add the `dataflow` section to the
  per-invocation MIR certificate — deterministic operators, exactly-once
  sinks with their checked commit-order verdicts, topology descriptor
  hash. Publishes only on success, unchanged.

**[C] `crates/nirdosha-driver/tests/determinism.rs`** and
**[C] `crates/nirdosha-driver/tests/commit_order.rs`**
- Follow the existing harness (`tests/resource_dataflow.rs`,
  `tests/sequence_dataflow.rs`): inline fixture sources written to
  tempdirs, real driver binary at O0/O3, refusal asserted with named
  spans. Fixtures: a renamed-wrapper clock smuggler (passes text scan,
  refused here); a sink with the effect before the commit (refused); a
  reordered correct sink (passes); interval-backend build
  (`--no-default-features`) coverage where meaningful.

**Verify:** `cargo test -p nirdosha-driver` (both feature sets, per
`docs/MIR_NUMERIC_PROOFS.md`).

---

## Phase 4 — CLI, certificates, serving (`cargo-nirdosha`, `compiled-serve`, `nirdosha-audit`)

**[M] `crates/cargo-nirdosha/src/lib.rs`**
- `guarantee_bundle()` (lib.rs:191): add the `dataflow` bundle section —
  per topology: descriptor hash, operator contracts, exactly-once sinks
  with key fields, edge buffer bounds. Reads the topology encoding the
  macro emitted (same rustdoc/attribute read path as contracts).
- `check_fn` (lib.rs:505): the strict-mode obligation — a `dataflow!`
  operator or sink without a contract is a strict finding, next to the
  existing "pub fn has no contract" check (lib.rs:577). Scoped to crates
  that declare a topology only (RFC Compatibility).

**[M] `crates/cargo-nirdosha/src/main.rs`**
- Usage string (main.rs:349) and `NIRDOSHA_STRICT` help text
  (main.rs:418): mention the operator-contract obligation.

**[C] `crates/cargo-nirdosha/tests/fixtures/dataflow_uncontracted.rs`** +
**[M] `crates/cargo-nirdosha/tests/guarantee_bundle.rs`**
- Fixture: a topology with one bare operator; test: bundle contains the
  `dataflow` section with correct hashes; strict mode flags the bare
  operator; non-strict passes.

**[M] `crates/compiled-serve/src/lib.rs`**
- A constructor/helper for webhook-source routes: `Route` (lib.rs:137)
  entries the `dataflow!` expansion generates for `HttpWebhook` sources,
  handing the request body to the runtime source adapter (Phase 2's
  `sink.rs`/`log.rs` pair). Deny-by-default on mutation (RFC 0010) and
  RFC 0022 hardening apply by inheritance — no new security surface,
  but the helper must route through the existing admission path.

**[M] `crates/nirdosha-audit/src/audit_chain.rs`**
- Make chain reads public: today `read_entries` (audit_chain.rs:122),
  `last_hash` (:127), `last_seq` (:131) are private and there is no
  read-by-offset API. Add `pub fn entries_from(path, from_seq)` (or make
  `read_entries` public) so a `DurableLog` source can consume
  `AuditEnvelope` chains by offset — the integration point the RFC's
  cross-system section depends on. `verify_chain` semantics unchanged.

**Verify:** `cargo test -p cargo-nirdosha -p nirdosha-audit`

---

## Phase 5 — Examples, demo, CI

**[C] `examples/rt-payroll-streaming/`** — `Cargo.toml` (dep
`nirdosha-rt` path, `dialect = true` metadata) + `src/main.rs`:
- `HttpWebhook<PaymentEvent>` source at `/hooks/payments`, a pure
  deterministic `enrich` operator (with `#[contract(effects(pure),
  deterministic)]`), `keyed(by = "account_id")`, a tumbling 60s window
  with `late_data = SideOutput`, an `exactly_once(key = "alert_id")`
  projection sink writing alerts, and a `dashboard!` read over the
  projection. Carries the doc-form contract too (the dialect's
  two-authoring-forms demo convention).
- Includes its lying twin **inline as a second bin target**
  (`src/bin/lying.rs`): an operator smuggling `Instant::now` through a
  renamed wrapper — passes Stage 1 (`--fast`), refused by Stage 2 with
  the chain named (the `rt-payroll-pure-chain` pattern, now for
  determinism), and a sink with the effect before the offset commit,
  refused by the commit-order pass.

**[M] `scripts/rt-dialect-demo.sh`**
- Add the streaming package to the refusal demo (~lines 26-60 section):
  `cargo nirdosha build` on the lying target → expect the determinism
  refusal; `--fast` on the same target → expect pass (the two-stage
  contrast is the visceral demo).

**[M] `.github/workflows/v2-guarantees.yml`**
- Extend the driver test matrix with the two new test files; add the
  streaming example to the demo/refusal job; keep the
  `deep-effects-next-nightly` daily job unchanged (the new passes ride
  the same pin-bump discipline).

**[C] `crates/nirdosha-rt/tests/dataflow_corpus.rs`** (optional but
recommended) — a doc-driven corpus regression in the style of
`rtm_policy_corpus.rs` over the streaming example's markdown, so the
topology in prose and the topology in code cannot drift.

---

## Phase 6 — Docs (same step as code, per AGENTS.md)

**[M] `docs/nirdosha-rt-dialect.md`**
- §6 crate map (line 201): `nirdosha-rt` row gains the `dataflow`
  module; `nirdosha-driver` row gains the two new passes;
  `nirdosha-macros` row gains `dataflow!`.
- §5 honest matrix (line 187): new rows — "`deterministic` claim,
  renamed `Instant::now` wrapper" (runs / passes / **refused, chain
  named**) and "`exactly_once` sink, effect before offset commit"
  (runs / passes / **refused**).
- §4 honesty notes (line 143): state the v1 scope (single-node, DAG
  topologies, logical-time only) and the async-operator exclusion.

**[M] `docs/V2_GUARANTEES.md`**
- Accepted-subset section (:19, deep-check subset :38): add what the
  determinism and commit-order passes prove and — equally load-bearing —
  what they do not (no termination proof, no whole-topology deadlock
  proof beyond the DAG + bounded-mailbox argument, no multi-node
  semantics).

**[M] `docs/V2_IMPLEMENTATION_BOOK.md`**
- New dated work-log entry under Work log (:52) recording gates passed:
  contract-core round-trips, macro expansion, runtime restore test,
  driver refusals, demo script.

**[M] `rfcs/README.md`** — already carries the RFC row; add this plan
file beside it if it isn't folded into the RFC itself.

---

## Dependency order and gates

```
Phase 0 (contract-core)
  └─► Phase 1 (macros)          — needs model + parse
        └─► Phase 2 (runtime)   — needs macro's emitted calls to compile
              └─► Phase 3 (driver)   — needs the injected commit sequence to prove
                    └─► Phase 4 (CLI/serve/audit)
                          └─► Phase 5 (examples/demo/CI) — needs all of the above
                                └─► Phase 6 (docs) — same step as its phase's code
```

Each phase gates the next: the macro can't emit `offset_commit` calls
before the runtime exports them; the driver can't prove an ordering the
macro doesn't emit; the example can't demonstrate a refusal the driver
doesn't implement. The one deliberate inversion: Phase 2's runtime tests
land before Phase 3, because restore-byte-identical is a runtime
property independent of the driver — a failure there invalidates the
checkpoint design before any MIR work is sunk into it.

**Repo-wide verification at the end:**
```
cargo test -p nirdosha-contract-core -p nirdosha-macros -p nirdosha-rt \
           -p nirdosha-driver -p cargo-nirdosha -p nirdosha-audit
cargo test -p nirdosha-driver --no-default-features   # interval fallback
./scripts/rt-dialect-demo.sh                          # two-stage refusal demo
cargo nirdosha verify --workspace                     # strict gate clean
```
