# RFC 0026 — The Nirdosha Metadata Plane: Governed Lineage Over the Guard Kernel

```
RFC:           0026
Title:         The Nirdosha Metadata Plane — Governed Lineage Over the Guard Kernel
Status:        Draft
Depends:       RFC 0023 (data guard) — normative for: guard macros & registry
               emission (§1B), invariants I1–I17 (§11), evaluation context &
               destination/purpose (§3.1), decision cache & guard-down degraded
               mode (§3.2), FederatedPlan & BudgetToken (§1C.1, §6),
               capability attestation (§7), delegated topology (§9.4), break-glass
               (§14), agent defaults I17 (§1C.3)
               RFC 0024 (hi + guard-mcp) — normative for: agent session
               delegation, proposal/accept flow, dual audit streams
               RFC 0025 (RTM ecosystem) — normative for: MIC, driver SPI
               (§7.5), compilation gates (§7.3), verify passes V1–V8 (§7.7),
               phasing
Supersedes:    nothing
Related:       case-study-rtms.md (worked scenario)
```

---

## 1. Abstract

RFC 0025 established two "byproduct" properties: verification metadata is a
byproduct of compilation, and conformance tests are a byproduct of policy edits.
This RFC establishes the third and largest: **lineage is a byproduct of
enforcement.**

Because every read and mutation in the v2 guard kernel is plan-mediated, the
system already knows — at enforcement time, with cryptographic receipts — what
flowed, under what policy, for what purpose, through which driver. That
knowledge is currently scattered across audit records and decision traces.
This RFC makes it a first-class, queryable, governed graph: the **Metadata
Plane**.

The plane has two halves:

- the **declared graph** — what *should* flow, compiled for free from
  `guard_policy!`, `dataset!`, `model_artifact!`, `window!`, `matcher!`,
  `data_contract!`, and related macro metadata;
- the **observed graph** — what *did* flow, projected from kernel-collected
  lineage observations bound into the audit chain.

Their **delta is itself the governance signal**: dormant policies,
undeclared flows, delegated-boundary anomalies, and break-glass events are all
computable joins between compiled intent and runtime fact.

The role this plane fills is the durable metadata layer that every mature data
platform converges on — the layer whose value compounds with age because engines
churn and meaning must persist. Industry systems fill this role through
instrumentation pointed at the platform from outside; they therefore produce
*claimed* lineage. Nirdosha owns the enforcement point and the compiler, and
therefore produces **attested** lineage: receipt-backed, policy-versioned,
purpose-bound, tamper-evident by inheritance from the audit chain, and
replayable.

Vendor neutrality holds here as everywhere: the metadata-plane *role* is
normative; external catalogs and lineage exchange formats appear only as
illustrative federation targets behind a port. The GraphStore is a two-driver
port like every other vendor boundary in the ecosystem.

---

## 2. Motivation

RFC 0025 prevented three failure classes at the data layer: guard bypass,
integration drift, and vendor lock. At the metadata layer, the same classes
remain open:

1. **Provenance is scattered.** Audit records (RFC 0025 §6.3), decision traces
   carrying `policy_version` + `model_version` (I4, RFC 0023 §11), and the
   end-to-end flow of RFC 0025 §10 each hold a fragment. None is queryable as a
   whole.
2. **Intent and fact are unjoined.** The catalog knows what *should* flow; the
   audit chain knows what *did* flow. Nothing computes the difference — so a
   dormant policy or a write that bypassed every declared surface is silent.
3. **History is engine-shaped.** When Kafka becomes Pulsar and model v1 becomes
   v3, the *meaning* of past flows must not churn with them. Meaning needs a
   home that changes at the speed of the business, not the speed of technology.

Every requirement above is already satisfied by machinery that exists. The gap
is formalization: one crate, one SPI method, two invariants, one macro surface.

---

## 3. Goals and Non-Goals

**Goals**

- MG1. Lineage is **automatic and complete-through-the-guard**: every plan
  execution yields an observation, with zero instrumentation effort by module
  authors and **no dependence on driver cooperation** for the kernel-side
  record.
- MG2. Lineage is **audit-grade**: receipt-backed, referenced to the audit chain,
  tamper-evident by inheritance, replayable per I4.
- MG3. The graph is a **governed query surface**: lineage queries are ordinary
  guarded actions with purposes, caps, classification propagation, and full
  audit.
- MG4. The **declared/observed delta is a first-class governance signal** —
  verify pass V10 — including dormant-surface, undeclared-flow, and
  delegated-boundary detection.
- MG5. **Designed for export, dependent on nothing**: the observation schema is
  federation-ready from the first emission; no external metadata product is a
  dependency.
- MG6. **Zero hot-path cost**: emission rides the existing audit batch; the
  guard overhead budgets of RFC 0023 §13 and RFC 0025 §12 are untouched.
- MG7. **Policy isolation**: lineage never feeds back into policy evaluation or
  the decision cache (RFC 0023 §3.1–3.2).

**Non-Goals**

- Replacing organization-wide catalogs or lineage platforms — Nirdosha is
  authoritative *in-perimeter* and federates outward (§14).
- Per-record graph lineage for `audit(sampled)` flows — aggregate-only edges,
  by design.
- A general graph query language — named views plus bounded traversal (§9).
- Lineage for flows entirely outside the guard perimeter — stated as a limit
  (§13); delegated-engine consumption is attested only at issuance (§7.5).

---

## 4. Terminology

| Term | Meaning |
|---|---|
| **Metadata Plane** | The governed lineage system specified here: declared graph + observed graph + their delta. |
| **Declared graph** | Graph of intended flows from compiled macro metadata (Gate-1 byproduct) plus guarded catalog mutations. |
| **Observed graph** | Graph of actual flows, projected from lineage observations bound into the audit chain. |
| **Lineage observation** | An audit record of kind `lineage`, emitted by the kernel per plan execution, referencing trace ID and receipt digest. |
| **Type-edge** | A graph edge representing a *class* of flows (node pair + transformation), carrying statistics — not one edge per event. |
| **Authority class** | `KernelExecution` \| `KernelIssuance` \| `BreakGlass` \| `ExternalClaimed` (§6.3). |
| **Dormant** | Declared surface with no observed flow in the evaluation window. |
| **Undeclared** | Observed flow with no corresponding declared surface — the bypass signal. |
| **Projection** | The observed graph as a deterministic function of the reconciled audit chain. |
| **GraphStore** | The swappable storage port behind the observed graph (two-driver rule applies). |
| **LineageSink** | The federation export port (illustrative mapping in Appendix A). |

**Naming note.** In the v2 runtime, access-control policies are authored with
`nirdosha_rt::guard_policy!` (an alias for `nirdosha_guard_macros::policy!`).
A separate `nirdosha_rt::policy!` macro exists for compliance-level policy
records. This RFC uses `guard_policy!` throughout to avoid ambiguity.

---

## 5. Design Principles

- **MP-1 — One ledger.** The reconciled audit chain is the sole source of truth.
  The observed graph is a projection: reconstructible, therefore a cache,
  therefore swappable.
- **MP-2 — Off the hot path.** Observations are batched with audit flushes under
  the existing I1 ordering. No new load-shed class: lineage shares the audit
  batch's never-shed status (RFC 0025 §12).
- **MP-3 — The kernel collects; modules cannot author.** A compromised module
  cannot lie about lineage any more than it can bypass policy. Drivers report
  row-level facts through the SPI; the kernel merges them with evaluation
  context it alone holds.
- **MP-4 — Declared vs. observed; the delta is the signal.** Compiled intent and
  runtime fact are separate planes, joined continuously. Silence is never an
  acceptable state.
- **MP-5 — Type-edges, not event-edges.** The graph holds classes of flows with
  statistics; individual events live in the audit chain, referenced by trace ID.
- **MP-6 — Queries are governed surfaces.** Reading the graph is a guarded
  action like any other, with classification propagation and destination
  rules.
- **MP-7 — Trust is explicit.** Every edge carries an authority class; trust
  levels are never silently merged.
- **MP-8 — Design for export, don't depend on import.** Federation-ready schema
  on day one; external formats are targets, never sources of authority.
- **MP-9 — Lineage never feeds back.** Observations are audit-side artifacts.
  They are **not** inputs to `EvaluationContext` (RFC 0023 §3.1), cannot alter
  decisions, and cannot enter the decision-cache key (RFC 0023 §3.2).
  Conformance asserts decision equivalence with the collector disabled.

---

## 6. Architecture

### 6.0 The three load-bearing decisions

> **MD-1 — Projection over the audit chain.** A lineage observation *is* an
> audit record in the module's existing hash chain (RFC 0025 §6.3 envelope;
> local chains reconciled centrally per RFC 0023 §3.2). Tamper-evidence is
> inherited, not reimplemented. Dropping the GraphStore and replaying the
> reconciled chain reproduces the observed graph exactly. The graph may be
> tiered, upgraded, or swapped without touching the permanent record.

> **MD-2 — Type-edges off the hot path, tiered retention.** At RTM rates
> (~10³ txn/s ≈ 8.6×10⁷ executions/day) per-event edges are untenable. Edges are
> **type-edges with statistics** (count, first/last seen, exemplar trace IDs).
> Per-event facts remain queryable through the audit chain by trace ID.
> Retention tiers: full receipt payloads hot for N days; edge aggregates for
> the platform's lifetime; audit chain immutable forever.

> **MD-3 — Declared and observed are separate planes.** The catalog answers
> *what should flow, under what authority*; the observed graph answers *what
> did flow, with what proof*. The delta between them is computed continuously
> and is the input to verify pass V10 (§8). The declared graph reflects
> **accepted, compiled state only**: policy revisions proposed in an agent
> session (RFC 0024 §5.1) are not declared surfaces until promoted via
> `graph_accept` — proposals are visible as separate proposal nodes so that
> `policy_simulation!` (§10.3) can evaluate them without promoting them.

### 6.1 Node model

Every node is a **catalog entity** (RFC 0025 P6) with a stable ID that survives
vendor swaps:

| Node class | Examples | Source |
|---|---|---|
| Data | dataset, topic, feature, model artifact, list, reference data | `#[dataset]`, topic bindings, `window!`, `model_artifact!`, `matcher!` |
| Policy | policy record, role, workflow, approval chain, invariant, purpose | `guard_policy!`, `roles!`, `workflow!`, `approval_chain!`, `#[invariant]`, `#[purpose]` |
| Runtime | module, driver, policy snapshot, model artifact instance, domain pack, policy proposal | manifests + guarded install mutations |

```rust
pub enum NodeKind { Data, Policy, Runtime }

pub struct NodeId {
    pub kind: NodeKind,
    pub catalog_id: String,
    pub version: Option<String>,
}

/// Tokenized reference to a row-level key. RESTRICTED values never enter the
/// graph; only a one-way keyed reference (e.g., HMAC-SHA256 over the key
/// material) is stored (§13).
pub struct KeyRef { pub token: String }

pub struct EntityRef {
    pub node: NodeId,
    pub keys: Option<Vec<KeyRef>>,
}
```

### 6.2 Edge model

```rust
pub enum EdgeType {
    // data-flow
    Read, DerivedFrom, MergedFrom, ScoredBy, ScreenedBy, AlertedBy, ExportedTo,
    // governance
    GovernedBy, EvaluatedUnder, ApprovedBy, DelegatedTo,
    // lifecycle
    VersionOf, DeployedOn, InstalledAs, Consumes,
}

pub enum Authority {
    KernelExecution,   // inline plan execution, receipt-backed
    KernelIssuance,    // delegated credential minted with exact scope (RFC 0023 §9.4)
    BreakGlass,        // attested emergency flow; reconciliation mandatory
    ExternalClaimed,   // inbound federation or external-engine self-report; unverified
}

pub enum FlowCompleteness { Full, Watermark }

pub struct DriverRef { pub port: String, pub vendor: String, pub version: String }

pub enum TransformId {
    Policy(String),
    Model(String),
    Window(String),
    Matcher(String),
    MergeSpec,
    Invariant(String),
    BreakGlass { reason: String },
}

pub struct EdgeStats {
    pub count: u64,
    pub first_seen: String,          // RFC 3339
    pub last_seen: String,
    pub exemplar_trace_ids: Vec<String>, // bounded
    pub sampled: bool,
}

pub struct LineageEdge {
    pub edge_type: EdgeType,
    pub src: NodeId,
    pub dst: NodeId,
    pub authority: Authority,
    pub policy_version: PolicyVersion,
    pub purpose: Purpose,
    pub destination: Destination,
    pub driver: DriverRef,
    pub transformation: TransformId,
    pub completeness: FlowCompleteness,
    pub stats: EdgeStats,
    pub receipt_digests: Vec<[u8; 32]>,
}
```

Rules:

- **Type-edges carry statistics, not events.** One edge
  `svc:features → velocity_1h ← txn_events` carries `count`, `last_seen`,
  exemplars.
- **Authority classes never merge silently** (MP-7). `KernelExecution` is
  receipt-backed plan execution. `KernelIssuance` attests that a scoped
  credential was minted and registered (RFC 0023 §9.4.6) — it does **not**
  attest what the engine later did; engine self-reports join as
  `ExternalClaimed` edges linked to the issuance edge. `BreakGlass` edges carry
  a mandatory reconciliation task. `ExternalClaimed` edges are quarantined in
  attestation-requiring queries.
- **Sampled flows are honest.** Edges from `audit(sampled)` traffic carry
  `sampled: true` and aggregate-only statistics; V10 treats their coverage as
  statistical, never per-record.
- **Policy-version stability.** The edge key includes `policy_version`; a new
  policy version creates a new edge, so historical and current flows are not
  silently merged.
- **Merge identity is explicit.** `EdgeKey` is exactly
  `(edge_type, src, dst, policy_version, purpose, authority)`. Purpose and
  authority are key dimensions, never mergeable metadata: queries can filter
  by purpose, and MP-7 prevents trust levels from being combined. Metadata
  attached to one key is keep-first and therefore immutable under a
  chain-ordered projection; per-observation completeness and degradation stay
  available through chain replay.

### 6.3 Projection and reconstruction

```text
 plan execution ──kernel──▶ AuditRecord { kind: lineage, observation } (hash-chained)
        │                            │
        │                            └──▶ collector batches ▶ GraphStore (type-edges)
        │                                         (two-driver port)
        └── trace_id + receipt_digest ◀── always resolvable back through the chain

 compliance-grade checks (V10, blast radius)  = replay the reconciled chain directly
 exploratory queries (§9)                     = read the projection (eventually consistent)
```

### 6.4 Crate placement (workspace delta)

```text
crates/
├── nirdosha-lineage/            # kernel-adjacent, vendor-free
│   ├── LineageObservation, LineageEdge, NodeId, TransformId
│   ├── collector + projection API
│   ├── declared-graph join
│   └── GraphStore port trait
├── nirdosha-lineage-macros/     # proc-macros for lineage_query!, data_contract!,
│                                # migration_plan!, policy_simulation!
├── nirdosha-lineage-store-embedded/   # GraphStore driver 1 (e.g., RocksDB/SQLite)
├── nirdosha-lineage-store-remote/     # GraphStore driver 2 (remote graph service)
└── nirdosha-lineage-sink-*/     # Phase 4 federation sinks (Appendix A)
```

`nirdosha-lineage` depends only on `nirdosha-guard-core` and
`nirdosha-audit` in Phase 1; the federation dependency is added with the
declared-graph join. It is subject to the same workspace lint as `policy/*`
(no vendor crates; RFC 0025 N-A4). The embedded and remote crates are the two
GraphStore drivers required by V8; they are separate from the projection
cache and do not become a second source of truth.

**Why `nirdosha-lineage-macros` is a separate crate.** Lineage is a
separate plane from guard policy. Folding these macros into
`nirdosha-guard-macros` would couple an orthogonal metadata system to the
policy front-end, making both surfaces harder to version and test. Keeping
them separate preserves the boundary: `nirdosha-guard-macros` owns policy
syntax; `nirdosha-lineage-macros` owns lineage/composition syntax. Both are
re-exported from `nirdosha-rt`.

### 6.5 What vendor swaps look like in the graph

**Kafka → Pulsar.** Node `txn_events` is unchanged. Node
`driver:stream:kafka@0.12` is superseded by `driver:stream:pulsar@0.4` via
`VersionOf`/`DeployedOn` edges. Historical `Read` edges keep their original
driver references — history remains meaningful under both. Nothing is
rewritten.

**Model v1 → v3.** Nodes `rt_fraud_v1@1.3.0` and `@3.0.0` join via `VersionOf`;
`ScoredBy` edges cite the exact artifact version, so "which alerts came from
v1.3.0?" is a graph query forever.

---

## 7. Emission — driver SPI extension and kernel collection

### 7.1 SPI extension (additive to RFC 0025 §7.5)

```rust
// Additive to guard-core. Drivers pre-tokenize keys; raw RESTRICTED values
// never enter the metadata plane.
pub struct LineageEntity {
  pub entity: EntityId,
  pub keys: Vec<String>,
}

pub struct LineageFacts {
  pub sources: Vec<LineageEntity>,
  pub sink_keys: Vec<String>,
}
```

The Phase-2 MIC adds `StoreDriver::lineage()` and maps its evaluation context
and snapshot version into the collector's explicit context parameters. The
Phase-1 lineage crate does not depend on the context's field layout. Node
resolution is collector-owned; drivers report only row-level sources and
optional sink-key refinement. There is no `LineageObservations` bridge type,
async SPI, or driver-owned sink identity.

Scope rules:

- **I18 is kernel-unconditional.** Even if a driver returns nothing, the kernel
  emits a coarse observation from evaluation context + plan IR (source/sink
  datasets, subject, action, purpose, destination, policy version, receipt
  digest). `lineage()` enriches; it is never the sole source. "Every execution
  yields an observation" therefore does not depend on driver cooperation.
- **Manifests declare `lineage_support = none | datasets | row_keys`.** Policies
  may require `row_keys` attribution for designated resources (e.g., RESTRICTED
  writes); a driver lacking it is plan-rejected for those flows only (V5
  machinery, RFC 0025 §7.7).
- The requirement applies to **every plan-executing driver** regardless of port
  kind — topic, window, model, list, and store drivers are all facades over
  plan execution.

### 7.2 Kernel-side collection (MP-3, MP-9)

`guarded_apply` and plan execution (RFC 0025 §6.3–6.4) call the driver's
`lineage()`, merge the result with evaluation context, and append an audit
record of kind `lineage` to the module's hash chain — batched with the audit
flush under existing I1 ordering. Modules have no API to write lineage
directly. Conformance adds one probe: **decision equivalence with the
collector disabled** (MP-9; no observation ever influences a decision or the
cache key).

The audit record shape is the existing RFC 0025 envelope plus `kind: "lineage"`
and a `LineageObservation` payload:

```json
{
  "trace_id": "01JD8WQ7…",
  "ts": "2025-11-27T10:15:32.114Z",
  "module": "ingest@1.4.2",
  "kind": "lineage",
  "policy_versions": ["2025.11.4"],
  "prev_hash": "9f2c…",
  "hash": "1b7e…",
  "observation": { /* LineageObservation serialized */ }
}
```

### 7.3 Hot-path budget

`lineage()` is pure and allocation-light; collection adds no synchronous work
to the authorization path; batches flush with audit. No new load-shed class:
lineage records share the audit batch, which is never shed (RFC 0025 §12). The
RFC 0023 §13 budget (<1 ms cached reads at the guard boundary) is unchanged.

### 7.4 Sampled reads

Flows governed by `audit(sampled)` produce aggregate-only observations
carrying the sampling policy version. We do not promise per-record lineage
where the audit policy deliberately samples — honesty over completeness
theater.

### 7.5 Special flows

| Flow | Substrate (RFC 0023) | Lineage semantics |
|---|---|---|
| **Delegated topology** | §9.4; I13; revocation registry §9.4.6 | Minting is a guarded mutation → `KernelIssuance` edge (credential scope, TTL, engine principal). Engine self-reports, if any, arrive as `ExternalClaimed` edges joined to the issuance edge. Consumption is **never** displayed as execution-attested. |
| **Federated reads** | §1C.1, §6 (`FederatedPlan`, `BudgetToken`), I16 | One observation per sub-plan (per `BindingId`, from each driver) plus one merge-layer edge (`MergedFrom`, transformation = `MergeSpec`). `completeness` set by the budget outcome: `Full` or `Watermark` (mid-stream abort); denials produce no edge. |
| **Guard-down degraded reads** | §3.2 | Observations ride the same local hash-chained writer as degraded-read audits, flagged `degraded`; reconciled on recovery. V10 flags unreconciled outage windows as *pending*, not missing. |
| **Denials & holds** | §6 `Decision::Pending`; I2, I3 | Audit-only — no lineage edge, because no data flowed. A held write that is later released produces its edge at commit (with commit-time re-evaluation, I3). Completed approvals produce `ApprovedBy` edges. |
| **Break-glass** | §14 | `BreakGlass` authority; post-hoc review task is the existing auto-created one; V10 holds the finding open until reconciliation closes. |
| **Agent delegation** | §1C.3; RFC 0024 §2 | Delegation-token mint/burn are guarded mutations → `DelegatedTo` edges (user→agent, purpose, TTL). This gives RFC 0024 §7's dual-stream correlation a queryable substrate. |
| **Reference-data joins** | §1C.2 | Both hops (codes + labels) are guarded reads → ordinary observations; no side channel exists to omit. |

### 7.6 New guard invariant (proposed amendment to RFC 0023 §11)

> **I18 — Lineage reconstructibility.** Every plan execution yields at least
> one lineage observation, emitted by the kernel even without driver
> enrichment. Observations are immutable audit records referencing the
> execution's trace ID and receipt digest. Only the kernel's collector may
> append them. The observed graph is a projection: replaying the reconciled
> audit chain reproduces it exactly. No component may write to the observed
> graph by any other path. Observations are never inputs to policy evaluation
> (MP-9).

---

## 8. The delta as governance signal — verify pass V10

> **V9 is reserved** for the baseline model-pack proposal (separate RFC,
> pending). The lineage pass is **V10**, continuing the pass set of RFC 0025
> §7.7. This section is a **proposed amendment** to RFC 0025; RFC 0026 cannot
> be marked Final until RFC 0025 incorporates V10.

> **V10 — Lineage completeness.** Runs in two modes, both deterministic:
> (a) **CI/nightly replay** over a signed audit-chain export — verify remains
> a pure function of (compiled metadata, chain segment), extending RFC 0025
> N-A5; (b) a **streaming checker** in the projection service for timely
> undeclared-flow alerts. Compliance-grade conclusions are always drawn in
> mode (a).
>
> 1. every catalog node is reachable in the observed graph **or** flagged
>    `absent(reason)`;
> 2. every declared policy surface has ≥1 observed flow within the window
>    **or** is flagged **dormant**;
> 3. every observed flow resolves to a declared catalog surface **or** is
>    flagged **undeclared**;
> 4. every `BreakGlass` edge has an open reconciliation task until closed;
> 5. every `KernelIssuance` edge has matching observed consumption **or** is
>    flagged (feeding the revocation reaper, RFC 0023 §9.4.6); consumption
>    with no issuance parent is **undeclared** (the delegated-boundary bypass
>    detector);
> 6. unreconciled degraded-mode windows are flagged *pending*, never silent.

| Signal | Meaning | Raised as |
|---|---|---|
| **Dormant** | Declared but never observed — dead policy or broken pipeline | Catalog annotation |
| **Undeclared** | Observed but never declared — bypass detector | Security event + annotation; never silent |
| **Issued-unconsumed / Unissued-consumption** | Delegated-boundary anomalies (§7.5) | Security event; feeds reaper |
| **BreakGlass** | Attested emergency flow | Mandatory reconciliation task |
| **ExternalClaimed** | Federation inbound or engine self-report | Quarantined from attestation-requiring queries |

The undeclared detector completes a chain that started with type-level plan
unforgeability (RFC 0025 §7.6): inside the ecosystem, bypass is a compile
error; at the perimeter, bypass is a flagged anomaly.

---

## 9. Querying the plane — `lineage_query!`

### 9.1 The query surface is a policy surface (MP-6)

"Downstream of this MSISDN" can reveal an investigation. Queries compile
(Gate 1, type-checked against the catalog; unknown nodes/views are compile
errors with `help:` spans) into plans executed **through the guard**; the
graph propagates classification: masked fields are *absent* from `llm_context`
results (RFC 0023 §1C.3, RFC 0025 §8.8).

```rust
nirdosha_rt::guard_policy! {
    allow "lineage-explore" for Analyst
    when action == "lineage_query" && resource == "downstream_of"
    filter tenant_scope()
    cap(max_depth = 5, max_nodes = 1_000, max_execution = 5s)
    destination(llm_context) denied_above(CONFIDENTIAL)
    obligate audit(full)             // lineage reads are never sampled
}
```

Tool exposure follows the single-source rule: lineage views are generated
into the MCP registry like any other tool (RFC 0023 §1C.3; RFC 0024 §6 signed
build-time descriptions), keeping verify pass V6 in lockstep.

### 9.2 Macro grammar and expansion

The illustrative yield-list grammar from the earlier draft is superseded by
the typed view form below: `view name(params) -> Row { field: Type [filter] }`.

```rust
nirdosha_rt::lineage_query! {
    view downstream_of(start: NodeId, depth: u32) -> DownstreamRow {
        node: NodeId,
        edge_type: EdgeType [filter],
        policy_version: PolicyVersion [filter],
        authority: Authority [filter],
    };

    view upstream_of(start: NodeId, depth: u32) -> UpstreamRow {
        node: NodeId,
        edge_type: EdgeType [filter],
        purpose: Purpose [filter],
        destination: Destination [filter],
    };

    view provenance_of(trace_id: TraceId) -> ProvenanceRow {
        step: u32,
        src: NodeId,
        edge_type: EdgeType,
        dst: NodeId,
        policy_version: PolicyVersion,
        receipt_digest: [u8; 32],
    };
}
```

Each `view` expands, at macro-expansion time, to:

1. A public `Row` struct (`DownstreamRow`, `UpstreamRow`, `ProvenanceRow`)
   with the declared fields. Fields marked `[filter]` additionally generate a
   `Default` value for omitted filters.
2. A public query-builder struct named `<Name>Query` with a private
   `filters: Vec<FilterExpr>` field and the declared parameters bound at
   construction.
3. A public constructor function `<name>(...) -> <Name>Query`.
4. A `.with_<field>(value) -> Self` method for every `[filter]` field that
   appends a typed `FilterExpr` to the builder's filters.
5. An `execute(self, ctx: &GuardCtx) -> Result<Vec<Row>, GuardError>`
   method that evaluates a guarded `lineage_query` action with
   `resource = "<name>"`, the bound `start`/`depth`/`trace_id` as query
   shape, and the assembled filters, then maps the resulting observations
   into `Row` values.

The expansion is deterministic and contains no string formatting. Unknown
node IDs or edge types are compile errors: the macro resolves catalog IDs
against the distributed registry at expansion time.

#### Generated-code sketch

```rust
// Expanded from `view downstream_of(start: NodeId, depth: u32) -> DownstreamRow { ... }`
#[derive(Debug, Clone)]
pub struct DownstreamRow {
    pub node: NodeId,
    pub edge_type: EdgeType,
    pub policy_version: PolicyVersion,
    pub authority: Authority,
}

pub struct DownstreamOfQuery {
    start: NodeId,
    depth: u32,
    filters: Vec<nirdosha_guard_core::FilterExpr>,
}

pub fn downstream_of(start: NodeId, depth: u32) -> DownstreamOfQuery {
    DownstreamOfQuery { start, depth, filters: vec![] }
}

impl DownstreamOfQuery {
    pub fn with_edge_type(mut self, value: EdgeType) -> Self {
        self.filters.push(/* typed filter */);
        self
    }
    pub fn with_policy_version(mut self, value: PolicyVersion) -> Self {
        self.filters.push(/* typed filter */);
        self
    }
    pub fn with_authority(mut self, value: Authority) -> Self {
        self.filters.push(/* typed filter */);
        self
    }

    pub async fn execute(
        self,
        ctx: &GuardCtx,
    ) -> Result<Vec<DownstreamRow>, GuardError> {
        let plan = ctx.guard.lineage_query_plan(
            "downstream_of",
            &LineageQueryShape {
                start: self.start,
                depth: self.depth,
                filters: self.filters,
            },
        )?;
        let observations = plan.execute().await?;
        Ok(observations.into_iter().map(|o| DownstreamRow {
            node: o.node,
            edge_type: o.edge_type,
            policy_version: o.policy_version,
            authority: o.authority,
        }).collect())
    }
}
```

The `GuardCtx::lineage_query_plan` surface is added to the existing guard
client; it is the only way to obtain a `LineageQueryPlan`, preserving the
same unforgeability property as `read_plan` / `write_plan` (RFC 0025 N-A3).

### 9.3 Worked queries

```rust
// Every flow that used rt_fraud_v1@1.3.0 under policy 2025.11.4 → alerts:
let flows = lineage_query::downstream_of(
        NodeId::model("rt_fraud_v1".into(), Some("1.3.0".into())), 6)
    .with_policy_version("2025.11.4".into())
    .execute(&ctx)?;

// Prove no svc:features output reached llm_context without explicit grant:
let to_llm = lineage_query::downstream_of(
        NodeId::data("feature".into(), None), 6)
    .with_destination(Destination::LlmContext)
    .execute(&ctx)?;
if !to_llm.is_empty() {
    return Err("features reached llm_context".into());
}

// "How did alert A-12891 happen?" — regulator-facing, receipt-backed:
let chain = lineage_query::provenance_of("01JD8WQ7…".into()).execute(&ctx)?;
// chain[0]: txn_events --Read--> velocity_1h --ScoredBy--> rt_fraud_v1@1.3.0
//             (policy 2025.11.4, purpose fraud_monitoring, receipt 9f2c…)

// Correlate an agent session across both audit streams:
let from_proposal = lineage_query::downstream_of(
        NodeId::runtime("proposal".into(), Some("policy-rev-812".into())), 3)
    .execute(&ctx)?;
let from_session = lineage_query::upstream_of(
        NodeId::runtime("delegation".into(), Some("S-4471".into())), 3)
    .execute(&ctx)?;
let both = LineageQuery::union(from_proposal, from_session)?;
```

---

## 10. Composition macros

### 10.1 `data_contract!` — packaging intent, not a subsystem

Every clause maps onto existing machinery:

```rust
nirdosha_rt::data_contract! {
    contract txn_v2 {
        entity = "transaction";
        owner = "payments-platform";
        schema_version = "2.1";
        quality { invariant(amount_positive), invariant(currency_iso) };
        retention = 7y;
        downstream_consumers = [
            node(runtime, "svc:bi"),
            node(runtime, "regulatory_archive"),
            node(data, "rt_fraud_v1", "1.3.0"),
        ];
    }
}
```

Expansion:

1. A `DataContract` record registered in the catalog distributed slices.
2. `Consumes` edges from each declared consumer to the entity — V10 checks
   them.
3. The quality invariants are references to existing `#[invariant]`
   registrations; missing registrations are a compile error.
4. `retention` maps to a policy record / catalog retention field.

### 10.2 `migration_plan!` — blast radius at evolution time

V2 (schema↔policy coherence, RFC 0025 §7.7) runs at compile time over the
workspace; `migration_plan!` computes the same coherence **at evolution
time, over the graph** — possible because edges carry `transformation` IDs
pointing at the specific windows, models, and policies that touch a field:

```rust
nirdosha_rt::migration_plan! {
    migrate field("transaction", "geo") {
        replace_with = field("transaction", "geo_region");
        mask = region_only;
        compatibility = backward;
        blast_radius from lineage;
        requires approval(chain = "schema_release");
        obligate audit(full);
    }
}
```

Expansion:

1. A guarded mutation request (`action = "migrate"`, `resource = "transaction"`).
2. A lineage-derived blast-radius report: upstream and downstream flows that
   read or write the field.
3. A reference to an `approval_chain!` definition.

### 10.3 `policy_simulation!` — dry-run at scale

I4 replay + a candidate policy snapshot + decision deltas. The same replay
service powers the baseline model-pack promotion gate (V9, reserved).

```rust
nirdosha_rt::policy_simulation! {
    simulation sim_2025_12 {
        window = 7d;
        candidate_policy = "2025.12.1-candidate";
        baseline_policy = current;
        measure { alert_volume, denied_flows, escalation_load, case_load };
        obligate audit(full);
    }
}
```

Expansion:

1. A `SimulationRun` catalog artifact.
2. A replay job over the window using historical observations and the
   candidate policy snapshot.
3. No promotion: simulation results are read-only and subject to V10.

### 10.4 Macro expansion summary

| Macro | Expansion outputs | Verifies |
|---|---|---|
| `lineage_query!` | `Row` structs, query builders, guarded `execute` | view resource ∈ catalog, unknown nodes = compile error |
| `data_contract!` | `DataContract` record, `Consumes` edges, invariant refs | invariants exist, consumers exist |
| `migration_plan!` | guarded migration request + blast-radius query | approval chain exists, field exists |
| `policy_simulation!` | `SimulationRun` record + replay descriptor | candidate policy exists, window valid |

The expansion of all four macros is deterministic, contains no runtime
string formatting, and is checked at macro-expansion time against the
catalog distributed slices.

---

## 11. Failure modes of metadata planes — design pressure, answered

| Failure mode | Root cause | Mechanism here |
|---|---|---|
| Instrumentation drift | Emission optional and external | Emission in the driver SPI + kernel; modules cannot opt out; I18 unconditional |
| Staleness | Observed graph is the only half that exists | Declared/observed delta *is* the signal (V10); compliance checks replay the chain |
| Cardinality collapse | One edge per event at stream rates | MD-2 type-edges; per-event facts stay in the chain |
| Second source of truth | Metadata store evolves independently | MD-1 projection; GraphStore is a swappable cache |
| Silent trust mixing | Claimed and verified edges look alike | MP-7 authority classes; issuance ≠ consumption (§7.5); `ExternalClaimed` quarantined |
| Silent incompleteness | Nothing notices missing coverage | V10 totality; I18; break-glass and degraded-mode reconciliation |
| Ungoverned metadata access | The map becomes a side channel | MP-6: guarded, classified, fully audited queries |

---

## 12. Performance Budgets

- **Hot path:** zero added synchronous work; `lineage()` is pure; batches flush
  with audit under I1. Budgets inherited unchanged: RFC 0023 §13 (<1 ms cached
  reads at the guard boundary, masking and audit enqueue included) and RFC 0025
  §12 (5–15 ms p99 authorization envelope). No new load-shed class — lineage
  shares the never-shed audit batch.
- **Cardinality:** ~8.6×10⁷ executions/day collapse to O(edge types × node
  pairs) — thousands of type-edges for a mid-size RTM deployment.
- **GraphStore sizing** scales with node/edge-type cardinality and retention
  tiers, independent of event volume.
- **Reconstruction SLA:** bounded by reconciled-chain replay throughput; full
  rebuild must complete within the operator's declared recovery window.
  Compliance-grade checks (V10 mode (a), blast radius, `provenance_of`) replay
  the chain directly.

---

## 13. Security Considerations

- **The graph is sensitive.** Classifications propagate to query results;
  cross-tenant views aggregate; `destination(llm_context)` rules apply (RFC
  0023 §3.1); masked fields are absent, not masked-in-place. Lineage queries are
  full-audit, never sampled.
- **References, not payloads.** RESTRICTED values never enter the GraphStore —
  only tokenized key references (§6.1).
- **Kernel trust.** Modules cannot forge lineage; drivers report only row-level
  facts; the collector is kernel code; only the collector appends (I18).
- **Policy isolation.** Observations never enter `EvaluationContext` or the
  decision cache (MP-9); the conformance equivalence probe is mandatory.
- **Delegated honesty.** Issuance is attested; consumption under an issued
  credential is not, and is never displayed as if it were (§7.5) — the
  strongest honest statement the I13 topology permits. Upgrading delegated
  consumption to `KernelExecution` would require a new trust model (signed
  engine receipts, verification contract, key rotation), which is out of
  scope for this RFC. Any such model must be specified in a separate RFC and
  cannot be assumed here.
- **Perimeter honesty.** The graph is authoritative *for flows through the
  guard*. Spreadsheets, external SaaS, and unguarded legacy pipelines are
  invisible or `ExternalClaimed`. The boundary is stated, not imagined away.
- **Break-glass** edges are immutable and reconcile-or-escalate; an unclosed
  reconciliation is itself a V10 finding (RFC 0023 §14 machinery).

---

## 14. Federation — export by design, import with quarantine

The observation schema is federation-ready from the first emission (MP-8);
the mapping is a pure function of observations (Appendix A, illustrative).

- **`LineageSink` port** — two-driver rule (V8) applies. Installing a sink is a
  guarded catalog mutation.
- **Inbound** edges are `ExternalClaimed`: stored, joinable, quarantined from
  attestation-requiring queries. The plane never presents claimed lineage as
  attested.
- **Posture:** Nirdosha is the *authoritative in-perimeter* source; the
  organization-wide metadata layer (whatever fills the role) is a *consumer*.

---

## 15. Phasing — with explicit substrate gates

| Phase | Delivers | Requires (substrate) |
|---|---|---|
| **1** | `LineageObservation` schema; additive `lineage()` SPI default; `AuditRecord` `kind: "lineage"`; kernel collector; concrete `lineage_query!` expansion; I18 + MP-9 conformance probe | RFC 0023 checklist: registry emission, I1/I4/I6; RFC 0025 Phase 0/1: MIC, audit chain, driver SPI. |
| 2 | Projection service; GraphStore port (embedded + remote drivers); V10 mode (a) replay | RFC 0025 Phase 2/3. |
| 3 | `LineageSink` federation drivers; inbound `ExternalClaimed` edges; V10 streaming checker | RFC 0025 Phase 4; attestation machinery (RFC 0023 §7). |
| 4 | `data_contract!`; `migration_plan!`; `policy_simulation!` (shared replay with model-pack gates) | RFC 0025 Phase 5; replay service. |

---

## 16. Alternatives Considered

- **Adopt an external metadata plane as the system of record and feed it.**
  Rejected as system of record: it cannot see the enforcement point, cannot
  carry receipts, has no compiler, and reintroduces the drift class this RFC
  prevents. Retained as federation target (§14).
- **SDK/collector instrumentation.** Rejected: optional, drift-prone,
  claim-only, no declared graph.
- **Per-module self-reporting of lineage.** Rejected: a compromised module must
  not be the source of its own exoneration (MP-3).
- **Let delegated engines self-attest execution.** Rejected: would present
  claimed consumption as attested. Kept: issuance-attested edges + quarantined
  self-reports. Upgrading self-reports to `KernelExecution` requires a future
  trust model with a separate RFC; it is not a patch on this design.
- **Per-event graph edges.** Rejected on cardinality (MD-2).
- **Graph as an independent store of truth.** Rejected (MD-1).
- **A general graph query language.** Rejected: ungovernable expressiveness;
  named views + bounded traversal + the guard cover the need.
- **Lineage observations as policy inputs** (e.g., "deny flows with incomplete
  lineage"). Rejected: violates MP-9 and the RFC 0023 §3.1 context contract;
  enforcement of completeness belongs to V10, not evaluation.

---

## 17. Effect on the permission model

This RFC adds a new guarded action class (`action = "lineage_query"`, plus
`migrate` and `simulate` via the composition macros). It does not broaden what
any existing subject can do: lineage reads obey the same deny-by-default,
cap, classification, and destination rules as data reads. The graph itself is
a sensitive resource, so reading it is never sampled and never default-allow.

The only new runtime authority is the kernel collector's exclusive right to
append lineage audit records. That authority is internal to the guard; modules
gain no new user-visible permission.

---

## 18. Compatibility

Purely additive to the v2 dialect and to RFC 0025's driver SPI:

- Existing modules compile unchanged; the new `lineage()` method has a default
  implementation returning coarse-only attribution.
- Existing policies are unchanged; `guard_policy!` blocks gain no implicit
  lineage semantics.
- Existing audit chains remain valid; the new `kind: "lineage"` records are a
  backward-compatible addition to the RFC 0025 envelope.

Normative dependencies on RFC 0023 and RFC 0025 are stated as **proposed
amendments**: I18 to RFC 0023 §11 and V10 to RFC 0025 §7.7. RFC 0026 stays in
Draft until those amendments are accepted or the RFC is revised to remove the
dependency.

---

## 19. Open Questions

The following are genuinely open but not blocking for Draft:

1. Should `lineage_query!` support parameterized aggregation windows (hourly,
   daily) or is per-policy-version edge separation sufficient?
2. Cross-tenant lineage views for platform operators: aggregation floors;
   does `cohort_floor` semantics extend to edges?
3. Receipt payload tiers expire; chain digests do not. Supported workflow
   when a `provenance_of` replay hits an expired tier?
4. Minimum driver set for `LineageSink` under V8: is the embedded debug sink
   enough, or must two production emitters exist?
5. Reconstruction SLA as a catalog-attested GraphStore capability?
6. Should `ExternalClaimed` edges require a guarded `link_external_lineage`
   action before they are joined to kernel-attested nodes? (Leaning yes.)
7. Should tenant become an `EdgeKey` dimension for cross-tenant projections,
  or remain a query/reconciliation constraint?
8. How should watermark/degraded observations contribute to `EdgeStats`
  beyond the current sampled flag and chain-replay visibility?
9. What canonical external-ingress node represents a zero-source flow while
  preserving the distinction between unknown source and genuine ingress?

Load-bearing questions resolved in this version:

- Edge statistics are keyed by
  `(edge_type, src, dst, policy_version, purpose, authority)`; a new policy
  version, purpose, or authority starts a new edge.
- Dormant aging thresholds live in policy records, making them replayable.
- `KernelIssuance` edges persist as historical records with a `revoked_at`
  annotation; they do not auto-expire.

---

## 20. References

1. RFC 0023 — *Nirdosha Guard — Store-Agnostic Access Control* (macros §1B,
   context §3, FederatedPlan §1C.1/§6, attestation §7, delegated §9.4,
   invariants I1–I17 §11, break-glass §14, agents §1C.3) and its
   implementation checklist.
2. RFC 0024 — *`nirdosha-hi` and `nirdosha-guard-mcp` integration*
   (delegation §2, proposal/accept §5, dual audit streams §7).
3. RFC 0025 — *Nirdosha RTM Ecosystem* (MIC §6, compilation gates §7,
   V-passes §7.7, phasing §16).
4. Baseline model packs proposal — reserves verify pass V9; shares the replay
   engine (§10.3).
5. *Case Study — Real-Time Transaction Monitoring on Nirdosha*
   (`rfcs/case-study-rtms.md`).

---

## Appendix A — Illustrative federation mapping (non-normative)

The `LineageSink` port is normative; this mapping illustrates that the
observation schema exports cleanly to an open lineage exchange format.
Named formats and catalogs here have exactly the status Kafka has in RFC
0025: examples behind a port, never dependencies.

| Observation field | Exchange representation |
|---|---|
| plan execution | run event (job = module, run = trace ID) |
| `sink` / `sources` | output / input dataset facets |
| `transformation` | processing-run facet |
| `policy_version`, `purpose` | custom facets `nirdosha:policy_version`, `nirdosha:purpose` |
| `receipt_digests`, trace ID | custom facet `nirdosha:attestation` (marks kernel-attested) |
| `authority` | custom facet `nirdosha:authority`; `ExternalClaimed` edges are **not emitted** |
| node catalog IDs | dataset/job entity names, namespaced `nirdosha.<pack>.<entity>` |

Inbound edges are accepted via a federation driver, typed
`ExternalClaimed`, and never promoted to kernel-attested except by a guarded
reconciliation flow (§17 Q6).

---

*Summary: enforcement already produces the facts; compilation already produces
the intent. This RFC joins them — one audit-record kind, one additive SPI
method, one projection, two proposed invariants (I18, V10), and four macros —
every element carried by the existing rustc + macros + driver machinery.*
