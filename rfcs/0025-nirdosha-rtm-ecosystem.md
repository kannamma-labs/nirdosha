# RFC 0025 — The Nirdosha RTM Ecosystem: Vendor-Neutral Modules on the Guard Kernel

```
RFC:           0025
Title:         Nirdosha RTM Ecosystem — Vendor-Neutral Modules on the Guard Kernel
Status:        Draft
Depends:       RFC 0023 (guard macros) — normative for guard syntax
               nirdosha_guard_store_agnostic_design.md — normative for guard behavior
Supersedes:    nothing
Related:       case-study-rtms.md (worked scenario this RFC generalizes)
```

---

## 1. Abstract

The Nirdosha guard already exists: policy evaluation, drivers, the L1 safety net, and the audit chain. This RFC specifies everything that gets built **on top of it** for real-time transaction monitoring (RTM): ingestion, feature computation, scoring, screening, alert generation, case management, downstream projections, and the analyst copilot.

The core contributions are:

- the **Module Integration Contract (MIC)** — one standard way every module attaches to the guard;
- a **compilation architecture** (rustc + proc-macros + drivers) that makes governance artifacts ordinary Rust source, makes plans unforgeable by the type system, and makes vendor neutrality a property of the dependency graph rather than a convention;
- a vendor-neutral **port** per vendor boundary, with adapters that attest capabilities;
- a **conformance suite** — generated from macro metadata — that makes "integrates cleanly" a testable property rather than a hope;
- a worked integration example for every module.

Result: a module integrates with the guard exactly once, in exactly one way; vendors swap behind ports without touching policy, workflows, or audit; and every `.nir` file compiles under plain `cargo` today and runs unmodified under `cargo nirdosha build` post-migration.

---

## 2. Motivation

The case study (`case-study-rtms.md`) demonstrated the *topology*. Left unspecified, each module team will invent its own guard wiring: ad-hoc audit calls, direct store connections, vendor types leaking into policy, inconsistent escalation handling. Eight modules means eight drift opportunities.

This RFC exists to prevent three failure classes:

1. **Guard bypass** — a module that talks to Postgres directly, skipping evaluation.
2. **Integration drift** — modules that audit, escalate, or fail differently.
3. **Vendor lock** — a Kafka type in a policy file, a Flink dependency in a rule.

---

## 3. Goals and Non-Goals

**Goals**

- G1. One uniform module⇄guard contract (the MIC) — same traits, same shim, same lifecycle for all eight modules.
- G2. Per-module least privilege: SPIFFE identity per module, deny by default, surface = granted policies.
- G3. Vendor neutrality per module: every external dependency behind a port, adapters attest capabilities, swaps are config-level changes.
- G4. End-to-end provenance: every decision carries `policy_version` (+ `model_version` where applicable) and is replayable (I4).
- G5. Uniform obligations, holds, and fail-closed semantics across all modules.

**Non-Goals**

- Redesigning the guard (RFC 0023 + the guard HLD are normative; the guard is *given*).
- Choosing vendors — Kafka, ONNX, OpenSanctions etc. appear in examples only as illustrations.
- Batch analytics governance beyond what the projection module needs.

---

## 4. Terminology

| Term | Meaning |
|---|---|
| **Guard kernel** | The existing guard client: policy evaluation, drivers, L1 net, audit emission. Linked into every module. |
| **Module** | A deployable service (ingest, features, scoring, …) implementing the MIC. |
| **Port** | A vendor-neutral Rust trait over an external dependency (stream, model runtime, list provider). |
| **Driver** | A vendor-specific implementation of a port; the only code that touches a store, broker, model runtime, or list source. Ships a capability manifest. |
| **Adapter** | Synonym for driver in the port sense; used interchangeably in examples. |
| **Catalog binding** | Declared in `.nir`: which store/topic backs which entity. Connection config lives here, never in code. |
| **Plan** | Guard-issued `WritePlan` / `AccessPlan`. Modules execute plans; they never open connections. |
| **MIC** | Module Integration Contract — §6 of this RFC. |
| **`.nirpkg`** | Signed deploy bundle: policy records, catalog, coverage matrix, manifests. |

---

## 5. Design Principles

- **P1 — Kernel, not framework.** Governance is centralized (PolicyStore, catalog, audit chain); enforcement is embedded and local. No per-record network hop.
- **P2 — One identity per module.** `svc:ingest`, `svc:features`, … — never a shared super-service account.
- **P3 — Surface = granted policies.** A module's entire capability is its `guard_policy!` set. Anything not granted is a deny.
- **P4 — Plans, not connections.** Modules never hold database or broker credentials. All I/O executes guard-issued plans. Topics are datasets, so stream access is ordinary plan access with the same ladder and the same audit.
- **P5 — Port per vendor boundary.** Drivers **attest** capabilities via manifests; a weaker driver is rejected at plan time, never silently degraded (I12 machinery).
- **P6 — Everything is a catalog entity.** Stores, topics, models, sanction lists, workflows, modules, drivers. Installing or upgrading any of them is a guarded, versioned mutation.
- **P7 — Provenance end-to-end.** Every decision trace is replayable: inputs, policy version, model version, obligations.
- **P8 — Fail closed, degrade loudly.** Under overload, shed async scoring depth first. Never shed audit (I1) or the L1 net (I7). A paused monitoring system beats an invisible one.

---

## 6. The Module Integration Contract (MIC)

This is the answer to "smooth and clean integration": every module implements **one trait**, routes every mutation through **one shim**, handles obligations through **one dispatcher**, and proves compliance via **one test suite**.

### 6.1 Stable kernel surface (already exists — consumed, not specified here)

```rust
// Provided by the guard. Frozen surface this RFC depends on.
impl GuardClient {
    pub async fn evaluate(&self, req: EvalRequest) -> Result<Decision, GuardError>;
    pub fn snapshot(&self) -> &PolicySnapshot;                     // local, versioned
    pub async fn revalidate_at_commit(&self, h: &Hold)
        -> Result<Decision, GuardError>;                           // I3
    pub fn write_plan(&self, resource: &str) -> Result<WritePlan, PlanError>;
    pub fn read_plan(&self, resource: &str, q: &Query) -> Result<AccessPlan, PlanError>;
}
```

### 6.2 Identity and manifest

Every module ships a manifest; `cargo nirdosha verify` checks it against the catalog before anything deploys.

```rust
pub struct ModuleManifest {
    pub name:        &'static str,        // "ingest"
    pub version:     semver::Version,
    pub identity:    Principal,           // SPIFFE-backed: svc:ingest
    pub purpose:     PurposeCode,         // e.g. fraud_monitoring
    pub guard_min:   semver::Version,     // refuse to run against an older guard
    pub ports:       Vec<PortBinding>,    // declared stream_port!/store bindings
}
```

### 6.3 The canonical mutation path: `guarded_apply`

**One function every module uses for every mutation.** Idempotency, evaluation, caps, obligations, and holds all live here — modules cannot get this wrong because they never reimplement it.

```rust
// nirdosha_rt::mic — provided by the runtime; identical for every module.
pub async fn guarded_apply<E: Entity>(
    ctx: &ModuleCtx, subject: &Principal, action: &str, resource: &str,
    entity: E, plan: WritePlan,
) -> Result<Outcome, Rejected> {
    let idem = ctx.idem.begin(entity.idem_key()).await?;   // exactly-once effects
    if !idem.is_new() { return Ok(Outcome::Duplicate(idem.trace())); }

    let decision = ctx.guard.evaluate(EvalRequest {
        subject, action, resource,
        entity: entity.class_view(),                       // per classification
        purpose: ctx.manifest.purpose,
        snapshot: ctx.guard.snapshot().version(),
    }).await?;

    match decision.effect {
        Effect::Allow => {
            plan.execute(entity).await?;                   // caps enforced inside the plan
            ctx.obligations.run(&decision).await;          // audit / notify / mask
            idem.commit(decision.trace.id).await?;
            Ok(Outcome::Committed(decision.trace.id))
        }
        Effect::Deny { reason } => {
            ctx.audit.full(DenyRecord::new(&decision, reason)).await;  // denies never sampled
            Err(Rejected::Denied(reason))
        }
        Effect::Escalate(hold) => {
            idem.stash_pending(entity, hold, plan).await?; // Pending; deny-by-timeout
            Ok(Outcome::Pending(hold.expires_at))          // release ⇒ revalidate_at_commit (I3)
        }
    }
}
```

Rules the shim enforces by construction:

- **`WritePlan`/`AccessPlan` have no public constructors** (§7.6); `guarded_apply` and `read_plan` are the only type-correct mutation/read paths. A module that wants to "just talk to Postgres" has nothing to call.
- **Driver `Receipt`s are embedded in audit records** (§7.5) — "the write happened" and "the audit row exists" are one atomic fact.
- **Offset commit happens after audit flush.** An at-least-once source plus the idempotency store yields exactly-once *effects*.
- **Audit ordering per I1**: the audit record for a mutation is emitted at/with commit, before the module acknowledges.
- **Escalations become `Pending` entities** with expiry; release re-evaluates against current state.

Audit record envelope (hash-chained per module):

```json
{
  "trace_id": "01JD8WQ7...",
  "ts": "2025-11-27T10:15:32.114Z",
  "module": "ingest@1.4.2",
  "subject": "spiffe://acme/ns/rtm/sa/ingest",
  "action": "create", "resource": "transaction",
  "policy_versions": ["2025.11.4"],
  "decision": "allow",
  "obligations": ["audit.full", "notify(topic=guard.decisions)"],
  "prev_hash": "9f2c…", "hash": "1b7e…"
}
```

### 6.4 Reads

Symmetric and equally mandatory:

```rust
let plan = ctx.guard.read_plan("transaction", &q)?;   // filters, masks, caps attached
let rows = plan.execute().await?;                     // row_cap, cohort_floor enforced here
```

### 6.5 Obligations

The dispatcher handles every obligation type; modules declare them in policy and never hand-roll them:

| Obligation | Semantics |
|---|---|
| `audit(full)` | Written to the module's hash chain, never sampled |
| `audit(sampled)` | Per the sampling rules in `10_domains.nir` |
| `notify(topic(T))` | Decision trace published to topic-dataset T via the module's StreamSink |
| `notify(channel(C))` | Human channel (analyst inbox, regulatory log) |
| `escalate to Hold { … }` | Pending entity, expiry, commit-time re-eval |

### 6.6 The Module trait and lifecycle

```rust
#[async_trait]
pub trait Module: Send + Sync {
    fn manifest(&self) -> &ModuleManifest;

    /// Publish datasets, topics, ports, and policies to the catalog.
    /// Generated largely by macros; verified by `cargo nirdosha verify`.
    fn register(&self, reg: &mut CatalogRegistrar) -> Result<(), RegError>;

    async fn run(self: Arc<Self>, ctx: ModuleCtx) -> Result<(), RunError>;

    /// EVERY admin op — offset reset, rules reload, model swap — is itself
    /// a guarded mutation. Maker-checker above thresholds is policy, not code.
    async fn admin(&self, op: AdminOp, g: &GuardClient) -> Result<AdminAck, AdminError>;
}
```

Install, upgrade, and rollback of a module (or a driver) are guarded catalog mutations — the same machinery that guards a transaction guards the ecosystem itself.

### 6.7 Capability manifests

Every driver declares what it can do. The planner checks manifests against policy requirements **at plan time**:

```toml
# drivers/kafka/manifest.toml
[driver]
port = "StreamPort"
vendor = "kafka"
version = "0.12.0"

[capabilities]
delivery    = "at_least_once"   # kafka transactions would upgrade this to exactly_once
event_time  = true
replay      = true
lag_metrics = true

[requirements]
guard_min = "0.9.0"
```

A Mode B (hold-based) policy requires `event_time = true`; a driver without it is **rejected at plan time** with a named reason — not at 2 a.m. in production.

### 6.8 Conformance suite (generated from metadata)

A module is MIC-compliant iff it passes `cargo nirdosha conformance <module>`. The suite is **generated from macro metadata** (§7.8); a policy edit regenerates its own tests.

| Probe | Asserts |
|---|---|
| deny-by-default | An undeclared action returns `Deny` **and** emits a full audit record |
| audit kill-switch | Stopping the audit emitter makes mutations fail (audit never bypassed) |
| idempotency | Replaying the same batch produces no duplicate effects |
| fail-closed | Snapshot refresh stopped ⇒ writes fail closed after TTL |
| hot-reload | Policy version bump changes decisions/provenance **without restart** |
| cap atomicity | An over-cap batch never partially applies |
| SoD probe | Cross-stage writes (e.g., scoring → alert) are denied |

### 6.9 Onboarding checklist (how any new module joins)

1. Identify vendor boundaries → define or reuse a **port trait**.
2. Declare entities/topics/classifications in a `10_domains`-style `.nir`.
3. Derive the module principal; add it to `roles!`.
4. Write `guard_policy!` blocks — deny by default; enumerate the full surface.
5. Implement `Module`; route mutations through `guarded_apply`, reads through `read_plan`. Direct store access fails review, `verify`, and the type system.
6. Publish capability manifests for each driver.
7. `cargo nirdosha verify` — inventory (I6), schema↔policy coherence, coverage-matrix totality.
8. Install = guarded catalog mutation (maker-checker above threshold).
9. Run the conformance suite; use shadow mode where the module influences decisions.
10. Promote. All subsequent updates are guarded mutations.

---

## 7. Compilation Architecture — rustc + macros + drivers

### 7.1 The claims, stated normatively

The guard HLD asserts store-agnosticism; the case study asserts that every `.nir` file compiles under plain `cargo` today and runs unmodified under `cargo nirdosha build` post-migration. This RFC makes both **normative**:

- **N-A1 — Governance artifacts are Rust source.** All `.nir` files are ordinary Rust modules. rustc is the first gate: a policy that does not parse and type-check cannot ship.
- **N-A2 — Zero runtime policy files.** No YAML/JSON policy is read at runtime. Policy lives in compiled-in records and signed snapshot pins. Runtime config = endpoints and credentials, held by drivers only.
- **N-A3 — Plans are unforgeable; drivers are the only executors.** Modules cannot construct a plan (no public constructor); drivers cannot originate one (they only lower guard-issued plan IR).
- **N-A4 — Vendor dependencies are compile-banned outside driver crates.** A workspace lint makes `rdkafka` (or any vendor crate) an error in `policy/*`.
- **N-A5 — Verify is a pure function of compiled metadata.** `cargo nirdosha verify` needs no running system; it is CI-runnable and deterministic.
- **N-A6 — Source compatibility across the toolchain migration.** Identical sources compile under plain `cargo build` today and under `cargo nirdosha build` (custom rustc driver) post-migration. The migration adds analysis and codegen; it never requires source edits.

### 7.2 Workspace layout — neutrality as a dependency graph

```
/workspace
├── crates/
│   ├── nirdosha-rt/        # proc-macro crate: RFC 0023 guard macros
│   ├── ports/              # vendor-neutral traits (StreamSource, WindowRuntime,
│   │                       #  ModelRuntime, ListProvider, Matcher, sinks)
│   ├── mic/                # guarded_apply, ModuleCtx, plan types (type-state)
│   ├── policy/rtm/         # ALL .nir files — depends ONLY on nirdosha-rt + ports + mic
│   └── drivers/            # the ONLY crates allowed vendor dependencies
│       ├── kafka/          #   rdkafka → StreamSource/StreamSink
│       ├── pulsar/
│       ├── flink/          #   WindowRuntime (exactly-once, session windows)
│       ├── datafusion/     #   WindowRuntime (embedded, at_least_once)
│       ├── onnx/           #   ModelRuntime (ort)
│       ├── rules/          #   ModelRuntime (DSL interpreter)
│       ├── osanctions/     #   ListProvider
│       └── pg/             #   RelationalDriver for WritePlan/AccessPlan
├── tools/
│   ├── nirdosha-verify/    # cargo subcommand (Gate 2)
│   └── nirdosha-driver/    # rustc driver (Gate 3, post-migration)
```

`cargo deny` enforces N-A4: `policy/*` resolving to a vendor crate is a build failure. Vendor lock-in is no longer a review comment — it is a red build.

### 7.3 The three gates

```
 .nir ──rustc + proc-macros──▶ rlib + metadata (distributed slices)
        │                            │
        │   Gate 1: cargo check/build      type safety, schema↔policy coherence
        │   Gate 2: cargo nirdosha verify  pure function of metadata → ✓/✗, signed report
        │   Gate 3: cargo nirdosha build   rustc driver: codegen, pins, deploy bundle
        ▼
 deploy bundle = bin(module + guard client + drivers) + signed .nirpkg + driver manifests
```

**Gate 1 — rustc + proc-macros (today, stable toolchain).** Each macro does three jobs in one expansion:

```rust
// What guard_policy! expands to (simplified):
pub mod policy_ingest_create_txn {
    // (1) executable scaffolding — the policy exists as data, not comments
    pub const REC: PolicyRecord = PolicyRecord {
        id: "ingest-create-txn", subject: "svc:ingest",
        action: "create", resource: "transaction",
        purpose: Purpose::FraudMonitoring,
        caps: Caps { affected_rows: Some(1), .. },
        obligations: Obligations::FULL_AUDIT,
        policy_src: file!(), line: line!(),
    };
    // (2) registration into the I6 inventory — linkme distributed slice
    #[linkme::distributed_slice(POLICIES)]
    pub static REG: &PolicyRecord = &REC;
    // (3) typed request builder consumed by guarded_apply
    pub fn request(e: &Transaction) -> EvalRequest { /* field view per classification */ }
}
```

Because macros expand *inside* rustc, coherence violations are compile errors with real diagnostics:

```text
error: unknown field `merchnt_id` on entity `transaction`
  --> rtm/20_ingestion.nir:14:38
help: fields are declared by `dataset(entity = "transaction")` in 10_domains.nir;
      see Nirdosha invariant I6 and RFC 0025 §7.7 (pass V2)
```

**Gate 2 — `cargo nirdosha verify`.** A cargo subcommand that reads the distributed slices (policies, entities, ports, models, workflows, manifests — the entire catalog) and runs the passes in §7.7. Emits a signed verification report that the `.nirpkg` bundle embeds.

**Gate 3 — `cargo nirdosha build`.** Post-migration custom rustc driver (same mechanism as clippy/miri — `RUSTC_WRAPPER` or `rustc_driver::RunCompiler` with callbacks). Sources unchanged (N-A6):

```rust
// tools/nirdosha-driver — callbacks only; no source transformation
rustc_driver::RunCompiler::new(&args, &mut Cbs {
    after_expansion: |c| { collect_metadata(c); run_verify_lints(c); Ok(()) },
    after_analysis:  |c| { specialize_plan_executors(c);   // monomorphize per driver
                           embed_snapshot_pin(c);          // min policy version
                           Ok(()) },
})?.run()
```

It emits the `.nirpkg` (signed policy records + catalog + coverage matrix + manifests) and binaries whose plan executors are monomorphized against the linked drivers.

### 7.4 Macro catalog — what each macro checks, emits, and feeds

| Macro (RFC 0023 surface) | rustc checks at expansion | Emits to metadata | Generates | Consumed by |
|---|---|---|---|---|
| `dataset` | field types, tenant binding present | entity schema record | `class_view()`, `idem_key()`, derive `NirdoshaEntity` | guard eval, idempotency store |
| `classify` | level is a known class | classification on type | masked/absent views | mask engine, I17 |
| `roles!` | principal id syntax | role/principal table | `SvcIngest`-style consts | policy subjects |
| `guard_policy!` | fields ∈ entity schema; roles exist; actions ∈ ladder; caps parse | policy record + coverage-matrix entries | `EvalRequest` builder | `guarded_apply`, verify |
| `stream_port!` | bound/published names are catalog topics | port binding record | typed `StreamSource`/`StreamSink` handles | topic drivers |
| `window!` | keys/aggs reference real columns | `WindowDef` IR records | compile-cache key `(window, policy_version)` | WindowRuntime drivers |
| `model_artifact!` | inputs are declared features; outputs typed | model spec + thresholds | provenance struct fields (I4) | ModelRuntime drivers |
| `matcher!` | list ids exist in catalog | matcher config | hit types | ListProvider/Matcher drivers |
| `workflow!` | machine is closed & deterministic | transition table | **type-state status enum** — illegal transitions don't compile | guard `transition_allowed()` |
| `approval_chain!` | quorum refs real roles | chain record | Pending handlers | escalation machinery |
| `mcp_tools!` | tools map to catalog entities | tool registry (→ OpenAPI) | MCP server skeleton, evaluate↔submit_write pairing types | copilot runtime |
| `break_glass!`, `audit_sampling!`, `audit_rules!`, `reference`, `relation`, `invariant`, `purpose` | refs resolve | records | runtime hooks | guard L1 net |

The key property: **the metadata for Gate 2 is a byproduct of Gate 1.** There is no second parser, no drift between "what the policy says" and "what the verifier sees."

### 7.5 Drivers — the enforcement layer, and the realization of store-agnosticism

The guard HLD's store-agnostic design is realized here: the guard issues plan IR; **drivers lower plan IR to vendor calls and are the only code that touches a store, broker, model runtime, or list source.**

```rust
// crates/mic — driver SPI (the only kernel-adjacent surface this RFC adds;
// purely additive to the existing guard client — nothing in the guard changes).
#[async_trait]
pub trait StoreDriver: Send + Sync {
    fn manifest(&self) -> &DriverManifest;               // capabilities (I12)
    async fn prepare(&self, ir: &PlanIr) -> Result<Prepared, PlanError>;
    async fn commit(&self, p: Prepared, e: EntityBytes) -> Result<Receipt, PlanError>;
}

pub struct Receipt { pub store_commit_id: String, pub digest: [u8; 32] }
```

Rules:

- **Two-level cap enforcement.** The plan IR carries caps/masks (`affected_rows`, `max_scan_rows`, `cohort_floor`, tokenization). The guard enforces at the plan layer; the driver enforces physically (SQL `LIMIT`, broker max-frame, ONNX input-shape checks) and **must fail if it cannot honor a capability the plan requires** — plan-time rejection, never silent degradation (I12).
- **Receipts bind writes to audit (I1).** The driver's `Receipt` is embedded in the audit record, so "the write happened" and "the audit row exists" are one atomic fact. If the audit emitter is down, `guarded_apply` never calls `commit` — the conformance kill-switch probe proves it.
- **Drivers are catalog entities.** Install/upgrade/rollback of a driver is a guarded, versioned mutation — the same machinery that guards a transaction guards the toolchain.

### 7.6 Plans are unforgeable (type system, not convention)

```rust
pub struct WritePlan<R> {                 // no public constructor — only the guard mints one
    driver: DriverHandle<R>, ir: PlanIr,  // ir carries caps/masks/obligations
    _m: PhantomData<fn() -> R>,
}
impl<R: Entity> WritePlan<R> {
    pub async fn execute(self, e: R) -> Result<Receipt, PlanError>;  // driver-only path
}
```

`guarded_apply` (§6.3) is therefore not a convention that modules are asked to follow — it is the *only type-correct path*: no plan constructor exists outside the guard, and no execution path exists outside drivers.

### 7.7 Verify passes (Gate 2)

| Pass | Checks | Invariant / source |
|---|---|---|
| V1 inventory totality | every mutating/read path traceable to a policy record | I6 |
| V2 schema↔policy coherence | policy fields ⊆ dataset schema; types match | Gate-1 residue + V2 |
| V3 coverage matrix totality | every (subject, action, resource) triple classified | §1B |
| V4 positive-relation rule | no policy justified by an absent relation | HLD |
| V5 capability cross-check | every capability a policy requires is attested by ≥1 linked driver manifest | I12; e.g. Mode B requires `event_time` |
| V6 MCP sync | generated tools ↔ catalog entities in lockstep | §8.8 |
| V7 SoD structural | cross-stage deny records present (scoring→alert, ingest→read) | §8 module table |
| V8 two-driver | every registered port has ≥2 driver manifests in the build graph | §11 |

### 7.8 Conformance is generated, not hand-written

`cargo nirdosha conformance <module>` reads the same slices and emits the probe set: deny-by-default per registered policy, cap probes with the *policy's own numbers* (a policy saying `affected_rows = 1` generates an over-cap replay test), SoD probes from V7, the audit kill-switch, idempotency replay, fail-closed TTL, hot-reload provenance bump. Policy changes regenerate the tests — the suite can never lag the policy.

### 7.9 Requirement traceability — rustc / macro / driver / runtime

| Requirement (§3) | Mechanism | Enforced at |
|---|---|---|
| G1 uniform MIC | `mic` crate + unforgeable plans (§7.6) | rustc type system |
| G2 least privilege | deny-by-default + per-module principals | guard runtime + V7 |
| G3 vendor neutrality | ports + workspace lint (N-A4) + two-driver CI (V8) | build + CI |
| Capability honesty | manifests vs policy cross-check | V5 + plan time (I12) |
| G4 provenance | `model_artifact!`-generated fields + snapshot pins | macro + runtime |
| G5 fail-closed | receipts bound to audit; kill-switch probe | driver SPI + conformance |
| I1 audit-before-ack | `Receipt` embedded in audit record | driver SPI |
| I6 inventory | distributed slices over all records | proc-macros |
| Schema coherence | expansion-time field/type checks | rustc diagnostics |
| N-A6 migration compatibility | callbacks-only rustc driver | Gate 3 |

---

## 8. Module Specifications

Summary of surfaces before the details:

| Module | Principal | Port | Guard calls | Integration focus |
|---|---|---|---|---|
| ingest | `svc:ingest` | StreamPort | `evaluate` (create/publish) | idempotency, obligations ordering, write-only SoD |
| features | `svc:features` | WindowOp IR | `evaluate` (read topic, write features) | cross-module read mediation, explicit `predicate_use` (I15) |
| scoring | `svc:scoring` | ModelArtifact | `evaluate` (read features), `admin` (model swap) | provenance stamping (I4), admin-as-mutation |
| screening | `svc:screening` | Matcher + ListProvider | `evaluate` (read lists) | full audit per check, bounded caps |
| event-gen | `svc:alerts` | StreamPort | `evaluate` (create alert) | SoD — sole alert writer, evidence-carrying writes |
| case | `svc:case` + roles | — (macros) | `evaluate` (transitions, SAR), `revalidate_at_commit` | approvals, egress, break-glass |
| consumers | `svc:bi`, … | StreamPort + sinks | `evaluate` (aggregate reads) | cohort floors, single-binding writes |
| copilot | `mcp:analyst-copilot` | MCP (generated) | `evaluate` + delegation | I17 defaults, evaluate-before-write |

### 8.1 Ingestion — `svc:ingest`

**Purpose.** Move external rail events into guarded entities. The only identity that creates `transaction` from raw rails.

**Port (vendor-neutral):**

```rust
// rtm/ports/stream.rs — no vendor type may appear in this file.
pub struct TopicRef { pub dataset: SmolStr }   // catalog name, e.g. "txn_events"

#[async_trait]
pub trait StreamSource: Send + Sync {
    async fn subscribe(&self, sub: Subscription)
        -> Result<BoxStream<'static, RawBatch>, PortError>;
    async fn commit(&mut self, offsets: Vec<Offset>) -> Result<(), PortError>;
}

#[async_trait]
pub trait StreamSink: Send + Sync {
    async fn publish(&self, topic: TopicRef, batch: EventBatch)
        -> Result<PublishAck, PortError>;
}
```

**Driver example (Kafka):**

```rust
// drivers/kafka/src/lib.rs — depends on rdkafka and the catalog, NOT on policy.
pub struct KafkaSource { consumer: StreamConsumer }

#[async_trait]
impl StreamSource for KafkaSource {
    async fn subscribe(&self, sub: Subscription)
        -> Result<BoxStream<'static, RawBatch>, PortError> {
        // external topic name + broker config come from the catalog binding
        // (`topic "txn.authorized" { … }`), never from module code.
        let ext = catalog::external_name(&sub.topic)?;
        /* configure rdkafka; return stream */
    }
}
```

**Guard wiring** (`rtm/20_ingestion.nir`, condensed):

```rust
nirdosha_rt::stream_port! {
    port txn_in  { bind "card_network.rails"; semantics = at_least_once; }
    port txn_out { publish "txn.authorized"; }
}

nirdosha_rt::guard_policy! {
    allow "ingest-create-txn" for SvcIngest
    when action == "create" && resource == "transaction"
    purpose(fraud_monitoring)
    field_policy { required(tenant_id, subject_id, amount, currency, status, occurred_at)
                   allowed(channel, merchant_id, card_token, device_id, geo) }
    cap(affected_rows = 1)
    obligate audit(full)
}

nirdosha_rt::guard_policy! {
    deny "ingest-no-readback" for SvcIngest
    when action == "read" && resource == "transaction"
    reason(sod.ingestion_is_write_only)
}
```

**Call site:**

```rust
async fn run(self: Arc<Self>, ctx: ModuleCtx) -> Result<(), RunError> {
    let mut src = self.kafka.clone();
    let mut stream = src.subscribe(sub("txn_in")).await?;
    while let Some(batch) = stream.next().await {
        for evt in batch.decode::<Transaction>()? {            // schema from catalog
            let plan = ctx.guard.write_plan("transaction")?;
            guarded_apply(&ctx, &SVC_INGEST, "create", "transaction", evt, plan).await?;
        }
        src.commit(batch.offsets()).await?;  // AFTER audit obligations flushed (§6.3)
    }
    Ok(())
}
```

**Vendor swap.** Kafka → Pulsar: new driver crate, new manifest, change one binding in `.nir`. Policies, roles, audit, and this loop are untouched (full procedure in §11).

---

### 8.2 Feature Engine — `svc:features`

**Purpose.** Maintain velocity/behavioral aggregates. **Integration focus:** the guard mediates cross-module reads (topics are datasets) and feature use of sensitive columns is explicitly granted (I15).

**Port:**

```rust
pub enum WindowKind { Sliding { every: Duration, period: Duration },
                      Hopping { every: Duration, period: Duration },
                      Session { gap: Duration } }

pub struct WindowDef { pub name: SmolStr, pub keys: Vec<Column>, pub kind: WindowKind,
                       pub aggs: Vec<Agg>, pub watermark: Duration }

pub trait WindowRuntime: Send + Sync {
    fn capabilities(&self) -> WindowCaps;             // e.g. session windows supported?
    fn compile(&self, defs: &[WindowDef], pv: &PolicyVersion)
        -> Result<CompiledWindows, PlanError>;        // cache key: (window, policy_version)
    fn push(&self, events: EventBatch) -> Result<(), PortError>;
    fn get(&self, window: &str, key: &WindowKey) -> Option<FeatureRow>;
}
```

**Driver example (Flink) — capability attestation in action:**

```toml
# drivers/flink/manifest.toml
[capabilities]
delivery          = "exactly_once"   # checkpointed two-phase commit
session_windows   = true
event_time        = true
```

The embedded DataFusion driver declares `delivery = "at_least_once"` and relies on the guard `IdempotencyStore` for exactly-once *effects* — the planner knows the difference and enforces it.

**Guard wiring** (`rtm/30_features.nir`, condensed):

```rust
nirdosha_rt::window! {
    feature velocity_1h(subject_id) =
        sliding(1h, keys = [subject_id], aggs = [count, sum(amount)]);
    feature impossible_travel(subject_id) =
        session(30m, keys = [subject_id], expr = geo_speed(geo) > 900 km/h);
}

nirdosha_rt::guard_policy! {
    allow "features-read" for SvcFeatures
    when action == "read" && resource == "txn_events"      // topic-as-dataset
    destination(feature_pipeline)
    cap(max_scan_rows = 500_000, max_execution = 10s)
    grant predicate_use(merchant_id, geo, amount)          // I15: explicit, auditable
    obligate audit(sampled)
}
```

**Call site:**

```rust
let plan = ctx.guard.read_plan("txn_events", &sub)?;   // same ladder as any table read
while let Some(batch) = plan.execute_stream().await? {
    runtime.push(batch)?;                              // windows update; caps enforced by plan
}
```

**Vendor swap.** IR → Flink job graph or IR → DataFusion plan. Feature *definitions*, caps, and grants travel unchanged; `verify` re-checks the compile cache coherence per policy version.

---

### 8.3 Scoring — `svc:scoring`

**Purpose.** Produce scores with full provenance. **Integration focus:** `model_version` stamped beside `policy_version` on every decision (I4); model activation is an admin mutation; SoD enforced as an explicit deny.

**Port:**

```rust
pub struct SignedArtifact { pub spec: ModelSpec, pub bytes: Bytes,
                            pub sig: Signature, pub sha256: [u8; 32] }

pub trait ModelRuntime: Send + Sync {
    fn capabilities(&self) -> ModelCaps;                       // onnx? batching? gpu?
    async fn load(&self, art: &SignedArtifact) -> Result<Box<dyn Model>, LoadError>;
}

#[async_trait]
pub trait Model: Send + Sync {
    async fn score(&self, f: &FeatureRow) -> ModelOutput;      // score + explanation
}
```

**Driver examples.** `OnnxRuntime` wraps `ort`; `RulesInterpreter` evaluates the DSL; `TritonClient` is a gRPC driver. All three satisfy the identical contract — a rules-only deployment is the same species as an ONNX deployment.

**Guard wiring** (`rtm/40_scoring.nir`, condensed):

```rust
nirdosha_rt::model_artifact! {
    model rt_fraud_v1 {
        format = onnx;
        inputs = [velocity_1h, distinct_payees_7d, amount_dev_30d, impossible_travel];
        outputs = [score: f64, explanation: vec[string]];
        threshold_alert = 0.85;
    }
}

nirdosha_rt::guard_policy! {
    deny "scoring-no-side-effects" for SvcScoring
    when action in ["create", "update"] && resource in ["alert", "case", "transaction"]
    reason(sod.pipeline_stage_isolation)
}
```

**Call sites — provenance and admin:**

```rust
let out = model.score(&features).await;
let trace = ctx.guard.trace()
    .policy_version(ctx.guard.snapshot().version())
    .model("rt_fraud_v1", art.spec.version)      // provenance per I4 — replayable
    .record(out);

// Model swap is a guarded mutation — never a config file edit:
async fn admin(&self, op: AdminOp, g: &GuardClient) -> Result<AdminAck, AdminError> {
    let d = g.evaluate(EvalRequest {
        subject: &SVC_SCORING_ADMIN, action: "update", resource: "model",
        entity: op.entity_view(), /* … */ }).await?;
    match d.effect {
        Effect::Allow => self.activate(op.artifact()).await,   // shadow-mode flag carried
        Effect::Deny { reason } => Err(AdminError::Denied(reason)),
        _ => Err(AdminError::Unsupported),
    }
}
```

Rollback = re-activate the previous signed artifact; the replay backtest delta is recorded in the audit chain.

---

### 8.4 Screening — `svc:screening`

**Purpose.** Screen names against sanctions lists. **Integration focus:** every check is audited `full`; vendor lists enter via a port with vendor-neutral IDs.

**Port:**

```rust
#[async_trait]
pub trait ListProvider: Send + Sync {
    async fn fetch(&self, list: ListId) -> Result<SignedList, FetchError>;  // e.g. "ofac_sdn"
}

pub trait Matcher: Send + Sync {
    fn screen(&self, name: &ScreenedName, lists: &[SignedList], cfg: &MatcherCfg)
        -> Vec<Hit>;
}
```

**Driver example (OpenSanctions):** `OsanctionsProvider` fetches and verifies signed snapshots; a Dow Jones or World-Check driver implements the same trait with the same `ListId`s.

**Guard wiring** (`rtm/40_scoring.nir`):

```rust
nirdosha_rt::matcher! {
    matcher sanctions {
        algorithm = fuzzy_jaro_winkler;
        threshold = 0.92;
        lists = [ofac_sdn, un_consolidated, eu_fsf];   // vendor-neutral ids
    }
}

nirdosha_rt::guard_policy! {
    allow "screening-check" for SvcScreening
    when action == "read" && resource == "sanctions_lists"
    purpose(fraud_monitoring)
    cap(max_scan_rows = 50)          // bounded per name screened
    obligate audit(full)             // every screening decision auditable
}
```

**Call site:** screening runs in-band when a policy references `screening.hit` (e.g., the Mode B wire rule). Each invocation emits a full audit record containing the input name, list versions, and hits.

**Vendor swap.** New `ListProvider` driver; `matcher!` block unchanged; list refresh cadence re-attested via manifest.

---

### 8.5 Event Generation (Alerts) — `svc:alerts`

**Purpose.** Turn scored events into alert entities. **Integration focus:** SoD by construction — this is the *only* identity whose policies mention `create` on `alert`, and its writes must carry evidence.

**Port.** Reuses `StreamSource` (consumes the decision-feed / scored-events topic). No external vendor boundary of its own.

**Guard wiring** (`rtm/50_policies.nir`):

```rust
nirdosha_rt::guard_policy! {
    allow "alert-raise" for SvcAlerts
    when action == "create" && resource == "alert"
    requires field(score) >= model(rt_fraud_v1).threshold_alert
          or field(rule_hits).nonempty()
    field_policy {
        required(tenant_id, txn_id, score, model_version, policy_version)
        forbidden(status)                    // analyst-only field — guard rejects writes to it
    }
    cap(affected_rows = 1)
    obligate audit(full)
    obligate notify(channel("analyst-inbox"))
}
```

**Call site:**

```rust
let plan = ctx.guard.write_plan("alert")?;
guarded_apply(&ctx, &SVC_ALERTS, "create", "alert",
              Alert::from_scored_event(evt)?, plan).await?;
// A scored event without evidence fields fails at the guard, not in business logic.
```

---

### 8.6 Case Management — `svc:case` + Analyst/ComplianceLead

**Purpose.** Alerts become cases; cases become SAR filings. **Integration focus:** escalation-to-approval, commit-time re-validation, and egress controls — all policy, no bespoke approval code.

**No external port** — workflows and approval chains are macros over existing machinery.

**Guard wiring** (`rtm/60_case_management.nir`, condensed):

```rust
nirdosha_rt::workflow! {
    machine CaseStatus {
        open -> investigating -> [confirmed_fraud, false_positive, escalate];
        confirmed_fraud -> sar_filed -> closed;
        false_positive -> closed;
    }
}

nirdosha_rt::approval_chain! {
    chain sar_release {
        quorum(2, of = [ComplianceLead]);
        timeout(deny);                        // approvals expire to deny, never allow
    }
}

nirdosha_rt::guard_policy! {
    allow "case-transition" for Analyst
    when action == "update" && resource == "case"
    requires field(status).transition_allowed()
    field_policy { allowed(status, assigned_to)
                   forbidden(alert_ids) }     // composition immutable
    cap(affected_rows = 1)
    obligate audit(full)
}

nirdosha_rt::guard_policy! {
    allow "sar-export" for ComplianceLead
    when action == "export" && resource == "case"
    requires field(status) == "confirmed_fraud"
    escalate to approval(chain sar_release)
    obligate audit(full)
    obligate notify(channel("regulatory-log"))
}
```

**Call site:**

```rust
// transition
guarded_apply(&ctx, &analyst, "update", "case", updated_case, plan).await?;

// SAR export: the shim stashes a Pending export; release triggers
// revalidate_at_commit (I3) so the export state is re-checked after quorum.
match guarded_apply(&ctx, &lead, "export", "case", sar_bundle, export_plan).await? {
    Outcome::Pending(exp) => schedule_release_review(exp),
    Outcome::Committed(t) => notify_regulatory_log(t),
    /* … */
}
```

**Break-glass** is a macro with mandatory reason, TTL, dual-approve above severity, and reconciliation — integration via the same `guarded_apply` path.

---

### 8.7 Consumer Projections — `svc:bi` and friends

**Purpose.** Downstream projections (warehouse, ledger). **Integration focus:** aggregate-shaped `AccessPlan` reads with cohort floors; writes restricted to the module's **own** binding (single-binding writes, §1C.1).

**Guard wiring** (`rtm/80_consumers.nir`):

```rust
nirdosha_rt::guard_policy! {
    allow "bi-aggregate" for SvcBi
    when action == "aggregate" && resource == "transaction"
    filter time_range(occurred_at, within, retention_window())
    cap(cohort_floor = 10, max_scan_bytes = 5GB, max_execution = 60s)
    mask(subject_id, tokenized)      // k-anonymity via tokenization
    destination(warehouse)
    obligate audit(full)             // aggregates are still audited (I9)
}
```

**Call site:**

```rust
let plan = ctx.guard.read_plan("transaction", &AggregateQuery { /* … */ })?;
let rows = plan.execute().await?;          // cohort_floor + caps + masking applied by the plan
ctx.sink.write(rows).await?;               // sink bound to the module's OWN dataset only
```

**Vendor swap.** The warehouse sink is just another store driver behind `WritePlan`; the policy and the aggregate semantics don't move.

---

### 8.8 Analyst Copilot — `mcp:analyst-copilot`

**Purpose.** LLM-assisted analyst tooling. **Integration focus:** tools are *generated from the catalog* (one source with OpenAPI), delegation is bound and non-transferable, and writes require a matching dry-run.

**No port to write** — `mcp_tools!` generates the server from the registry.

**Guard wiring** (`rtm/70_mcp.nir`):

```rust
nirdosha_rt::mcp_tools! {
    server analyst_copilot {
        identity = "mcp:analyst-copilot";
        tools = [query_records(Transaction, Alert, Case),
                 get_options(AlertStatus, TxnChannel),
                 evaluate,            // dry-run first — mandatory for writes
                 submit_write];       // gated: requires matching evaluate
        delegation { bind user + agent; ttl = 30m; max_tool_calls = 60; rate = 20/min; }
        defaults {
            audit = full;             // I17: agent access is never sampled
            row_cap = 50;
            destination = llm_context; // policy still governs per field/class
        }                              // masked fields are ABSENT, not just masked
    }
}
```

**Call site:**

```rust
let eval = tools.evaluate(ToolCall::Query { entity: "Case", q })?;   // dry-run, audited
// submit_write requires the matching evaluate id — the pairing is enforced by the runtime.
tools.submit_write(eval.id, WriteOp::UpdateCase { status: "investigating" })?;
```

Under the hood, every tool call executes the same `read_plan`/`guarded_apply` machinery as any other module — the copilot gets no side channel because none exists.

---

## 9. Control-Plane Integration

Modules integrate with the control plane the same way they integrate with the guard — via snapshots:

```rust
// in every module's run():
ctx.guard.snapshot().on_change(|new| {
    // atomically swap compiled windows / model thresholds / matcher configs
    // subsequent traces cite new policy_version — no restart, no redeploy
}).await;
```

- **PolicyStore** distributes signed snapshots; modules verify signatures.
- **Catalog** is the single source for bindings, models, lists, workflows, modules, drivers.
- **Admin ops** (offset reset, rules reload, model swap, driver upgrade) are guarded mutations on the module's own entities — maker-checker above thresholds is policy.

Policy change flow: edit `.nir` → `cargo nirdosha verify` → sign → PolicyStore bump → modules hot-reload → provenance fields update → old decisions still replayable under their recorded versions.

---

## 10. End-to-End Trace (one card transaction)

```
t0  card authz request arrives
t1  guard.evaluate(create, transaction)          svc:ingest · card-authorize → Allow
t2  WritePlan(pg_primary) commits · audit(full) hash-chained (I1)
t3  decision trace published to guard.decisions  (topic-dataset, plan-mediated)
t4  feature runtime ingests event                svc:features · features-read
t5  scoring reads features, scores               svc:scoring · rt_fraud_v1 v1.3.0
                                                 + policy 2025.11.4 → score 0.91
t6  event-gen creates alert                      svc:alerts · alert-raise → Allow
                                                 (score ≥ 0.85) · audit(full) · notify inbox
t7  analyst opens case via copilot               mcp:analyst-copilot · delegation bound
                                                 evaluate → submit_write · audit(full)
t8  SAR export                                   ComplianceLead · escalation → quorum(2)
                                                 → revalidate_at_commit (I3) → egress
```

Every step is a plan execution by a distinct principal; every step is replayable from its trace.

---

## 11. Vendor-Neutrality Conformance (compiler-enforced)

**The two-driver rule.** A port is only accepted into the ecosystem if CI runs it against **at least two drivers** (verify pass V8). Neutrality that isn't exercised isn't neutrality.

**What a swap looks like** — Kafka → Pulsar:

```diff
+ drivers/pulsar/                  # new driver crate: ports + manifest, depends on rdkafka nowhere
  drivers/kafka/                   # kept — two-driver rule (V8) requires it stays green
  # rtm/10_domains.nir
- #[nirdosha_rt::dataset(entity = "txn_events", store = "kafka_main", kind = "topic")]
+ #[nirdosha_rt::dataset(entity = "txn_events", store = "pulsar_main", kind = "topic")]
  topic "txn.authorized" { format = "avro", schema = "transaction" }
```

Then: `cargo nirdosha verify` re-runs V5 (does the Pulsar manifest attest what Mode B needs?), driver install is a guarded catalog mutation, sources untouched. **If V5 fails, the swap fails at plan time with a named missing capability** — the rejection is the mechanism working.

Rules-only → ONNX:

```diff
 # rtm/40_scoring.nir
-model rt_fraud_v1 { format = rules; … }
+model rt_fraud_v1 { format = onnx; inputs = [velocity_1h, …]; … }
```

Everything else — policies, roles, workflows, audit, `guarded_apply` call sites — is byte-identical. `verify` re-runs; manifests re-attest; the planner re-checks capabilities.

**What travels unchanged across every swap:** topics-as-datasets, policies, audit chains, provenance, workflows, MCP tool surface.

---

## 12. Performance Budgets

- Sync authorization path (card): guard overhead target **< 5–15 ms p99**; Mode A scoring is async and never on the authorization budget.
- Evaluation is local (policy snapshots); audit batched per partition; counters via KV `INCR`; windows compiled once per `(window, policy_version)` and cached.
- Mode B holds: `Pending` with deny-by-timeout; release re-evaluates (I3).
- Load-shed order: async scoring depth first; **never** audit (I1) or the L1 net (I7).

---

## 13. Failure Modes

| Failure | Behavior |
|---|---|
| Scoring provider down | Circuit breaker → conservative default (hold wires, pass cards with tightened velocity) + alert |
| Stream lag > watermark | Alerts flagged with provenance lag; Mode B unaffected (sync path) |
| Feature engine down | Mode A alerts pause (audited); authorizations continue |
| Guard down | Fail-closed writes; reads per degraded-mode rules; **no alert generation** — invisible monitoring is worse than paused monitoring |
| Model rollback | Previous signed artifact re-activated as guarded mutation; backtest delta recorded |
| Driver capability regression | Plan-time rejection on next plan build; running plans drain, new ones refuse |

---

## 14. Security Considerations

- Per-module SPIFFE identities; no shared accounts; delegation (`mcp:*`) is session-scoped and non-transferable.
- Denied mutations and all sensitive classes are never sampled; agent access (I17) is always full-audit.
- Masked fields are **absent** from LLM context, not merely masked in it.
- Approval timeouts resolve to deny; break-glass requires reason, TTL, dual-approve above severity, and post-hoc reconciliation.
- Modules hold no store credentials — plans carry authority; a compromised module cannot exceed its policies even if its code is fully malicious, because the guard and plans are outside its control path.

---

## 15. Phasing

| Phase | Delivers |
|---|---|
| 2 (RDBMS slice) | MIC; `nirdosha-rt` macros on stable rustc; TopicDriver(kafka) + RelationalDriver(pg); ingestion + decision feed; `verify` passes V1–V4; initial conformance suite |
| 3 | WindowRuntime drivers (datafusion, flink); features + scoring + Mode B holds; V5 capability cross-check |
| 4 | ModelDriver (onnx, rules); ListProvider drivers; model governance (shadow mode, replay backtest), screening, drift; V6–V8 |
| 5 | Case management + MCP copilot; **rustc-driver cut-over (`cargo nirdosha build`)** — sources unchanged per N-A6; full conformance suite mandatory for all modules |

---

## 16. Alternatives Considered

- **Ad-hoc per-module guard wiring** (library calls where each team chooses): rejected — this is the drift this RFC exists to prevent.
- **Network PDP sidecar per record**: rejected on latency (< 5–15 ms p99 with a hop is unrealistic); retained as a possible future MIC extension for polyglot stacks (see open questions).
- **Central enforcement bus**: rejected — breaks local fail-closed semantics and the latency budget.
- **Vendor config flags instead of ports**: rejected — unattested capabilities degrade silently.
- **External policy files (YAML/JSON) loaded at runtime**: rejected — violates N-A2; loses type safety, diagnostics, and the single-source property that Gate 2 depends on.

---

## 17. Open Questions

1. Should a future extension of the MIC define a gRPC PDP profile so non-Rust modules get the same contract?
2. On Pulsar: do transactions + dedup make driver-level exactly-once preferable to the `IdempotencyStore`, and which wins on conflict?
3. How do feature-store retention windows compose with privacy retention policy — one policy grammar or two?
4. Multi-region snapshot propagation: what SLA keeps provenance coherent across regions without breaking audit-chain ordering?
5. Should maker-checker thresholds for admin ops live in the catalog or in policy? (Leaning policy, for replayability.)
6. MCP tool schema evolution across policy versions — re-generate per snapshot or per release?
7. Metadata source at Gate 3: keep linkme slices authoritative, or let the rustc driver re-collect from HIR for cross-crate totality (V1 across the workspace, not per crate)?
8. `PlanIr` versioning: what compatibility window must a driver maintain across plan-IR versions during rolling upgrades?

---

## 18. References

1. RFC 0023 — Nirdosha Guard Macros (`60_data_guard.nir` syntax).
2. *Nirdosha Guard: Store-Agnostic Design* (HLD) — normative for guard behavior, invariants I1–I17.
3. *Case Study — Real-Time Transaction Monitoring on Nirdosha* (`case-study-rtms.md`) — the worked scenario this RFC generalizes.

---

*Summary: policies are Rust, so the compiler enforces them; drivers are the only I/O, so neutrality is a dependency graph; every module integrates through one contract with a worked example per module; and the same sources compile on plain cargo today and on the Nirdosha driver tomorrow — the migration is a toolchain event, not a rewrite.*