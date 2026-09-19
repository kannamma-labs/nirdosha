# Nirdosha Guard — Store-Agnostic Access Control

## High-Level Design Document (v3)

Status: revision addressing review rounds 1–3 + expressiveness pass.  
**Implementation status:** see [`0023-data-guard-checklist.md`](./0023-data-guard-checklist.md) for concrete, verified progress. This RFC targets the **v2 Rust dialect** (`nirdosha-rt` / `cargo-nirdosha` / `nirdosha-driver`); it does **not** modify the deprecated native `.nir` compiler (`crates/compiler`).

Scope: extends the Centralized Mutation Gate (v2 implementation plan) to cover
reads, writes, enumerations, and analytics across arbitrary storage systems.
(Relies on invariants I1–I6 and the decision algebra from that plan; both are
assumed linked here. I1 = audit-before-commit, I2 = deny-composition,
I3 = commit-time re-evaluation, I4 = policy snapshot/replay, I5 = fail-closed,
I6 = complete mutation inventory.)

Changelog vs v1: write-side plans (§2), policy context & cache contract (§3),
relation expressiveness design (§4), capability attestation (§7), QueryShape
leak controls (§8), delegated enforcement contract (§9), failure/revocation
semantics (§13), break-glass (§15). All blocking comments from review round 1
are resolved inline and indexed in §18.

---

## 1. Vision and principles

The guard decides; the store enforces. One decision surface, many native
enforcements.

1. **Decide once, enforce natively.** Policy evaluation happens in one
store-agnostic engine, emitting portable plans. Each driver compiles into
the strongest mechanism the store supports.
2. **Push down or give up gracefully — never leak.** Drivers report
capabilities; unexpressible filters escalate the ladder or deny.
3. **Mandatory safety net — inline topology only.** Every *inline* driver
applies a final in-process record check (L1) before data crosses the guard
boundary. The delegated topology (§9) has no L1 net by construction; it is
admitted only under the compensating controls of §9.4, and this limitation
is stated wherever the topology is offered (I13).
4. **No string-level query rewriting.** All filters compile from a typed,
parameterized AST. DDL generation (§9.3) uses a separate DDL AST with
strict identifier/value quoting — injection impossible by construction.
5. **The guard never proxies bulk data.** Heavy analytics receive scoped,
short-lived, policy-bound credentials, never a proxy.

---

## 1A. Adopted primitives & policy front-end boundary (review round 2)

Principle 6: **stand on audited primitives; the guard's value is the glue and
the invariants, not another policy engine.**

The product is the driver contract, I7 (driver must or deny — not "the app
does it"), I9 aggregate-leak control, I13 + scoped-credential topology.
Everything below is adopted OSS; `nirdosha-guard-core` owns only the IR,
the invariants, and the glue.

**Policy front-end — Cedar.** Policies are authored in
[Cedar](https://github.com/cedar-policy/cedar) (production Rust
implementation, formal analyzer, schema validation, permit/deny testing).
Design rule: **Cedar is one front-end, not the core.** A `PolicyFrontend`
trait maps (policy set, schema, context) → decision + residual; the
`AccessPlan`/`FilterExpr` IR remains the stable contract.

Hard rule — the **lowerable subset**: only a subset of Cedar lowers to
pushdown data filters (the same lesson OPA's SQL translation and Immuta
learned). Compilation pipeline:

```javascript
Cedar policy + schema + context
  → partial evaluation (residual policy)
  → LOWERABLE-SUBSET CHECK
       in subset  → lower to FilterExpr → driver ladder
       outside    → Tier 3: Deny { policy.not_lowerable } | Materialize
                     (never "app filters it later" — that is I7's whole point)
```

The subset is versioned and CI-tested. What we inherit from Cedar: the
analyzer (dissolves several policy-CI work items into configuration),
schema/entity model (synced from our dataset registry), and permit/deny
regression tests. What Cedar does not do (and we keep): purpose/consent/
destination context (carried as context entities/attributes), cohort floors
and caps (plan layer), and anything store-shaped.

**The pipeline, stated explicitly** (resolves the Cedar-vs-`policy!`
boundary):

```javascript
policy! { … }            Nirdosha canonical surface; TOTAL over the
     │                   lowerable subset (§1B). Every policy! block
     ▼                   lowers 1:1 to Cedar.
Cedar policy set         Hand-written Cedar is accepted but treated as
     │                   `raw_cedar` (linted, §1B) unless it matches a
     │                   policy!-generated artifact byte-for-byte.
     ▼
Partial evaluation       Cedar engine + evaluation context (§3.1).
     ▼
Residual policy
     ▼
LOWERABLE-SUBSET CHECK   Runs at policy-compile time for static residuals;
     │                   at plan-compile time for context-dependent ones.
     │ in subset          ▼
     ▼              FilterExpr / AccessPlan / WritePlan  → driver ladder (§7)
 outside subset    → Tier 3: Deny { policy.not_lowerable } | Materialize
```

So: `policy!` is canonical sugar over Cedar, not a separate language; the
lowerable-subset check runs at two well-defined points; and nothing reaches
the IR except through it.

**SQL layer — sqlparser-rs as verifier, not emitter.** Our typed layer emits
dialect SQL with bound parameters (I8 by construction). sqlparser-rs parses
the emitted SQL back; the parsed AST must round-trip structurally against
the intended plan — generation bugs become CI failures. Rationale: sqlparser's
AST holds literals, not bound parameters, so emitting directly from it would
weaken I8; and its BigQuery/ClickHouse coverage is partial, so columnar
emitters remain ours regardless.

**Per-dialect conformance requirement:** round-trip verification *with bound
parameters* must be proven per dialect before that dialect is admitted —
some dialect parsers normalize placeholders differently, and a dialect that
cannot round-trip its parameter nodes is not certified. The conformance
table (dialect × round-trip × parameter handling) is a CI artifact;
uncertified dialects emit via a literal-free path with an explicit
identifier allowlist and are surfaced as `expr_support`-limited.

**Arrow/Parquet — DataFusion machinery.** The driver is a policy→DataFusion
bridge: `FilterExpr` → physical expressions → `PruningPredicate` (L3,
row-group pruning) and `RowFilter` (L2, scan-time). No pruning engine of
our own.

**Relations — OpenFGA / SpiceDB as resolver sources.** The `RelationResolver`
trait (§4) admits FGA stores as providers: `listObjects` → bounded value set
→ Tier 1 `In`-list; checkpoint tokens map to `source_epoch`; bulk APIs
(`CheckBulkPermission`, `listObjects` batching) implement `resolve_many`.
We do not build a graph engine. Tier 2 materialization (§4) remains for hot,
unbounded relations where even bounded list resolution is too slow at plan
time — FGA sits behind the same freshness contract as any resolver.

**Prior art to mirror (study before Phase 2/4):**

- `cerbos/query-plan-adapters` — their answer to "filter unsupported" is
"the app does it"; ours is "the driver must, or deny" (I7). That delta is
the product. Read to know exactly what we are rejecting and why.
- Immuta's published work on SQL k-anonymity — adopt their
cohort-floor→`HAVING` translation for §8.1 rather than deriving our own.
- `trinodb/trino#1480` (row-filter/column-mask thread) — a field guide to
the exact leak modes I9/I15 target; convert into a conformance checklist
and run it against our invariant list before Phase 4.

---

## 1B. Syntax expressiveness contract (new)

Goal: everything in this document is expressible in Nirdosha syntax — and
anything the syntax accepts is guaranteed to transit the IR, the invariants,
and the lowering pipeline. No concept requires dropping to raw code; no raw
code can bypass the gate.

Framing: **the IR is the contract; syntax is its total preimage, minus
invariant-violating programs.** "Expressible" is deliberately not permissive:
a negated relation (§4), a masked field in a predicate without
`predicate_use` (I15), a create without a tenant binding (§2) are
*unexpressible by design* — compile errors, not runtime denials.

### Mechanisms

1. **Exhaustive coverage matrix (compile-time enforced).**
`nirdosha-guard-core` defines the IR enums (`FilterExpr`, `Cap`,
`Obligation`, `Decision`, `WritePlan` fields, topology knobs). A
`coverage_matrix!` test in the macros crate performs an *exhaustive,
wildcard-free match* over every IR enum. Adding an IR variant without a
matrix entry fails to compile; each matrix entry must name (a) its
canonical syntactic form and (b) a compiled example test. The IR cannot
grow silently faster than the syntax.
2. **Bidirectional doc conformance.**
Every worked example in this HLD lives as a compiled Rust file under
`crates/nirdosha-rt/tests/` (v2 macro smoke tests) and later as a full
v2 example crate under `examples/rt-*`. CI compiles all of them. Two
directions of enforcement: a new HLD concept without a compiling example
fails the build; a syntax change that breaks an example fails the build.
The doc and the DSL can never drift apart unnoticed.
3. **Registry emission.**
Every macro (`policy!`, `#[dataset]`, `#[relation]`, `#[classify]`,
`enumerate!`, `approval_chain!`, `break_glass!`, …) emits a descriptor
into a distributed registry (linkme-style). `cargo nirdosha verify`
aggregates the registry and cross-checks: schema/policy coherence, field
existence, positive-relation rule, classification consistency, dataset
coverage, and matrix totality.
4. **Escape valves with lints, not holes.**
`raw_cedar { … }` and `raw_filter { … }` exist for the genuinely novel
case. They still transit partial evaluation, the lowerable-subset check,
and all invariants — raw means "not sugar-coated", never "unguarded".
`raw_sql` for query shapes is **denied by default** in production
profiles; enabling it requires a `#[guard_exempt(reason, ticket,
expires)]`-style justification that CI enforces. A lint (`nirdosha::raw`)
fires on every use and is reported on the compliance dashboard.
5. **Diagnostics as documentation.**
Proc-macro errors carry spans and `help:` notes pointing at the canonical
form ("masked field `salary` used in filter — grant `predicate_use` or
remove from WHERE"). rustc's error surface becomes the DSL's tutorial.

### Construct → syntax coverage (canonical forms)

| HLD construct | Nirdosha syntax |
| --- | --- |
| Read plan / policy | `policy! { allow "name" when action == "read" && resource == "customer" filter … }` |
| Write plan | `policy! { allow "limit-adjust" when action == "update" … requires … ensures … field(amount).allowed() rows ≤ 1 }` |
| `FilterExpr` nodes | typed combinators: `==`, `in`, `cmp`, `and/or/not`, `time_range(field, from, to)`, `tenant()`, `pattern(path, glob/…)` |
| `RelationIn` (§4) | `customer.branch_id in relation(branches_under, subject)` — negation rejected at compile time |
| Masks / classification | type-inferred (`Money`, `CardNumber`); `#[classify(level)]` override; `mask(salary, "***")` in policy |
| `predicate_use` (I15) | `grant predicate_use(salary) to role("analyst")` |
| Caps (§8.2) | `cap(max_scan_rows = 1_000_000)`, `cap(cohort_floor = 10)`, `cap(row_cap = 100)`, `cap(max_execution = 5s)` |
| `Escalate { to: … }` (§2) | `escalate to approval(chain("payout-approvers"))`, `escalate to step_up(mfa)`, `escalate to materialize(job("…"))` |
| Obligations | `obligate audit(full)`, `obligate notify(channel("security"))` |
| Approvals | `approval_chain! { quorum(2) delegate(depth ≤ 2) }` |
| Dataset binding (§5) | `#[dataset(entity = "transaction", store = "parquet_lake")] glob "s3://…"` |
| Relation declaration (§4) | `#[relation(source = "org_registry", cardinality = 500, ttl = 60s)] fn branches_under(User) -> Set<BranchId>` |
| Materialization (Tier 2) | `#[materialize(relation = visible_closure)]` — write-through codegen |
| Enumeration / dropdown | `enumerate! { options from customer filter … cap(row_cap = 50) }` — opaque cursor enforced |
| Break-glass (§14) | `break_glass! { scope(resource("customer"), field("tax_id")) ttl(1h) dual_approve above(severity(2)) }` |
| Delegated topology (§9.4) | store adapter config: `store hive { enforce delegated, mint ddl_ast, audit full }` — topology + compensating controls are config, not code |
| Capability manifest (§7) | emitted by driver crates; `attest canary_rows, differential_tests` |
| Cohort floor → HAVING (§8.1) | `cap(cohort_floor = 10)` on grouped queries; lowering per Immuta translation |
| Guard-exempt paths (I6) | `#[guard_exempt(reason, ticket, expires)]` — CI-enforced expiry |
| Reference dataset (§1C.2) | `#[reference(field, store, missing = FailClosed\\|MaskedPlaceholder\\|Drop)]` — Tier 2 stability |
| Mask transforms (§17) | `#[classify(level)] struct CardNumber { value: str }; impl Mask for CardNumber; mask(card_number, partial_last4)` — type-class driven, closed registry |
| Audit sampling (§17) | `audit_sampling! { classification: CONFIDENTIAL -> rate 1.0; ... }` — explicit table, denials always full |
| Purpose taxonomy (§17) | `enum Purpose { Operations, Billing, Support, ... }; #[purpose(code = "...")]` — closed core + tenant-extensible registry |
| Write conditions / invariants (§17) | Phase 0: `requires field(status).in([...])`, `ensures field(amount) <= field(limit)`; Phase 2: `#[invariant(name = ..., schema = ...)]` pure registry |
| Raw escape valves | `raw_cedar { … }`, `raw_filter { … }` (linted); `raw_sql` (deny-by-default) |

### Stability tiers

- **Tier 1 (stable, semver-guaranteed):** `policy!`, `#[dataset]`,
`#[nirdosha_guard]`, `#[classify]`, `mutate!`.
- **Tier 2 (evolving with deprecation window):** caps, obligations,
`enumerate!`, approval chains, break-glass.
- **Internal (unstable):** driver capability manifests, registry descriptors.

Tier is part of the coverage matrix; a Tier 1 construct can only change via
an explicit deprecation cycle, enforced by the doc-conformance suite.

### Phase 0 addition

The Cedar spike (§1A) now has an expressiveness acceptance criterion: the
three spike policies must compile from `policy!` syntax — not from
hand-written Cedar — and the deliberately-out-of-subset policy must fail at
compile time with a `policy.not_lowerable` diagnostic. Proving the DSL covers
the lowerable subset is the spike's exit gate.

---

## 1C. Federated catalog, governed reference data, and agent access (new)

Three related questions, one answer: the guard sits in front of *every* data
consumer — federated queries, option/label resolution, and AI agents — and
each consumer is just another subject of the same decision surface.

### 1C.1 Entity catalog and the unified view

The dataset registry (§5) already binds entity → N physical bindings. To
answer "which store holds what, and serve it as one view," bindings gain
**routing metadata**: freshness/lag watermark, latency class, cost tier, and
query-shape affinity (which access patterns this store serves — point reads,
scans, aggregations, time ranges).

Federated read path:

```javascript
request(entity, QueryShape)
 → catalog selects eligible bindings (policy may also restrict WHICH stores
   a subject may read — store choice is itself a policy decision)
 → ONE logical AccessPlan expanded into a FederatedPlan (§6):
     per-binding sub-plans compiled with §9 strategies + a MergeSpec
     (union vs join, dedup keys, provenance flags) + a BudgetToken
 → parallel execution
 → merge at the Arrow/record layer
 → L1 net + masks applied at MERGE (uniformly, post-union)
 → global caps enforced via the budget coordinator (below)
 → response carries provenance: which store served which rows/partitions
```

Rules:

- **I16 — merge-layer re-filtering:** pushdown inside each store is not
sufficient for cross-store coherence; the union is re-filtered and re-masked
at the merge layer before anything leaves. A store that lied (I12) is caught
here even if its pushdown lied.
- **Cross-store aggregation safety (specified):** aggregating across bindings
(e.g., `SUM(amount)` where the entity lives in Postgres *and* Parquet) is
safe only under one of: (a) **declared disjoint partitions** — dataset
metadata asserts the bindings hold disjoint row sets for the queried
shape; (b) a **declared dedup key** — MergeSpec dedups on a unique key
before aggregating (adds cost, bounded by caps); or (c) **deny**.
"Overlapping replicas" without a dedup key is a plan-time rejection, not a
runtime surprise. Federation is a read-path feature; **writes are
single-binding** — the catalog declares one primary write binding per
entity, and multi-binding writes are out of scope for this design version.
- **Global caps coordinator (specified):** the guard issues a per-request
`BudgetToken` (scan rows/bytes, time, result bytes). Each sub-plan checks
out budget before executing and as it streams; exhaustion cancels siblings
via cancellation tokens and aborts mid-stream (store-native cancel where
supported, else client-side abort). The coordinator is in-process for the
inline topology; **coordinator failure = fail-closed deny of the request**
(a request whose spend cannot be tracked does not run). Mid-stream aborts
surface as an explicit partial-result watermark or a deny, per entity
config — never a silently truncated answer presented as complete.
- **No silent partial answers for correctness-critical unions** (listings,
totals the UI depends on): if an authoritative binding is unavailable, deny
or return an explicit degraded-result watermark — never a quiet shortfall.
**"Authoritative" is declared, not implied:** dataset metadata marks which
bindings are authoritative for which query shapes (e.g., the Parquet lake is
authoritative for historical aggregates; Postgres for current state);
policy may further restrict. A query shape with no declared authoritative
binding is denied.
- **Freshness contract:** every response carries per-binding lag watermarks;
fintech consumers can enforce "reject data staler than X."

### 1C.2 Reference data: codes in one store, names in another

The enum indirection (codes stored with records; display names elsewhere) is
modeled as a **reference dataset**:

```rust
#[reference(customer.status, store = "refdata")]   // code -> label binding
dataset refdata { table customer_status_labels { code, label, class } }
```

The `get options` flow is the existing enumeration path (§8.4, `enumerate!`)
with one extra hop — and *both hops are guarded*:

1. **Enumerate policy** evaluated for the field (subject, purpose, caps).
2. **Codes** fetched from the primary store, scope-pushed as usual.
3. **Label resolution is a guarded read on the reference dataset** — it
transits the full ladder (L4…L1) with its own policy; the reference store
cannot be used as an unguarded side-channel around the gate.
4. **Masks apply to labels too.** Category names can be as sensitive as the
records (fraud flags, medical categories): label-level classification,
label masking, and "drop this code *with* its label" are all policy-
expressible (§1B).
5. **What goes out:** only (code→label) pairs the subject may see; invisible
options are absent from the response entirely (no existence oracle, §8.4).
External consumers receive surrogate opaque IDs (I10), never raw internal
codes.
6. **Caching:** labels cached per `source_epoch` (reference data changes
rarely); TTL bound per the §4 freshness contract. Missing labels (code
without a row in refdata) are policy-configured: fail closed, masked
placeholder, or drop.

Joining records to labels on ordinary read paths goes through the same
governed resolution — there is no free JOIN around the guard. **Join routing
is explicit:** when the reference dataset lives in the *same store* as the
primary dataset, the driver compiles a **guarded JOIN** (the reference read's
scope becomes join conditions — full pushdown, no merge-layer cost). When it
lives in a *different store*, the join happens at the **merge layer**, which
is only viable with cached label sets (bounded per §4 cardinality rules) —
otherwise the plan is denied or routed to materialization. Merge-layer joins
at high volume are a plan-time denial, not a runtime perf cliff.

Two clarifications: "drop this code with its label" affects **only the
options/label view** — the record's own field value is governed independently
by field masks, not by enumeration visibility. And "missing labels" is a
declared policy enum, not free text:
`#[reference(..., missing = FailClosed | MaskedPlaceholder | Drop)]`.

### 1C.3 MCP server for LLM agents

Yes — an MCP server, as a **first-class guarded client**. It adds no path
around policy (same architectural pattern as the Cedar front-end, §1A): every
tool is a thin wrapper over the same `evaluate` surface.

- **Tools are generated from the registry** (single source shared with
OpenAPI): `list_entities`, `describe_entity` (schema + classification,
redacted), `get_options`, `query_records`, `evaluate` (dry-run),
`submit_write`. "Which API should the LLM call" is answered by the same
catalog the UI uses — never hand-written tool docs that drift.
- **Agent subjects:** delegation token — user-on-behalf, short-lived,
purpose-bound, **bound to both identities and non-transferable**
(audience-restricted, session-scoped; a token presented by a different
agent or user is rejected). The agent identity itself is authenticated.
Effective scope = intersection(user scopes, agent grants, declared purpose).
- **Agent policy defaults (I17):** full audit with **no sampling**; tighter
default row caps; **per-agent rate caps and per-agent cost caps**;
**loop/retry limits** (maximum tool calls per delegation session, with an
overall token/time budget); no export tool unless explicitly granted.
- **`submit_write` is default-off** for agents, and where enabled it is
gated: **a write requires a prior `evaluate` in the same delegation
session whose plan hash matches the submitted plan exactly** —
evaluate-then-act is mandatory, not encouraged. The matching evaluate is
recorded and cited in the write's audit entry.
- **Destination control (the biggest §1C control):** `destination` is a
policy dimension (§3.1). Default posture: destination = `llm-context` is
denied for classification ≥ CONFIDENTIAL and for sensitive-purpose reads
unless a policy explicitly grants it; `export-file` follows the export
path (§2) with full audit. An LLM is not entitled to everything the user
may see.
- **Tool descriptions are an injection surface too:** they are **static,
build-time artifacts, reviewed and signed with the policy bundle** — never
derived from user-controlled or runtime data, never containing live record
values. A compromised registry can poison tools, so descriptions carry the
same artifact verification as policies (`verify-artifact`).
- **Prompt-injection containment:** tool results are tagged data, not
instructions; the MCP layer never exposes credentials, policy internals, or
other subjects' traces; masked fields are *absent* from context — the model
literally cannot see what it may not see, which is the strongest possible
form of output control for an LLM consumer.

### Expressiveness additions (§1B coverage matrix)

New canonical forms: `#[reference(field, store)]`; catalog routing metadata in
`#[dataset]`; `store allowlist/denylist` in policy (which stores may serve a
subject); MCP tool generation config. Added to the coverage matrix with
compiled examples — the federated get-options flow (1C.1 + 1C.2 end to end)
becomes one of the HLD's worked examples in `crates/nirdosha-rt/tests/` or a
future `examples/rt-*` v2 crate.

---

## 2. Write Access Plans (new)

`AccessPlan` is read-shaped by definition. The write side gets its own plan
type; the decision surface is shared, the outputs differ.

```rust
pub struct WritePlan {
    pub decision: Decision,
    pub action:   WriteAction,          // Create | Update | Delete | Migrate | Export
    pub row_scope: Option<FilterExpr>,  // WHICH rows this write may touch
    pub preconditions:  Vec<Condition>,// must hold on pre-image (store-enforced)
    pub postconditions: Vec<Condition>,// must hold on post-image (store-enforced)
    pub field_policy: Vec<FieldPolicy>, // per-field: required | allowed | forbidden | default
    pub affected_row_cap: u64,          // hard cap on rows touched; overflow = abort
    pub obligations: Vec<Obligation>,   // blocking audit BEFORE commit (I1), etc.
    pub policy_version: PolicyVersion,
}

pub enum Condition { Expr(FilterExpr), Custom(InvariantId) }
// Condition is extensible by design: #[invariant] registers pure,
// schema-typed invariant functions into a CI-validated registry;
// policies reference them by InvariantId; no stringly extension.
pub enum FieldPolicy { Required(FieldPath), Allowed(FieldPath),
                       Forbidden(FieldPath), Default(FieldPath, Value) }
// Unlisted fields are FORBIDDEN. The write-side default is deny:
// a field not explicitly Allowed (or given a Default) cannot be written.
// Enforced by the macros at compile time where the schema is known.
```

Per-action semantics:

- **Create** — `field_policy` enforces required/allowed/forbidden fields and
tenant binding (tenant is always `Required`; a create without a tenant
binding is a build-time policy error). No `row_scope`. Classification of the
new record computed and recorded in the audit entry.
- **Update** — `row_scope` is enforced **at the store** (WHERE clause of the
UPDATE), not only at L1: the compiled write carries
`UPDATE ... WHERE <row_scope> AND <id>` and the driver asserts
`affected_rows ≤ affected_row_cap`. Pre-image is captured before the write
(blocking audit obligation, I1). `preconditions` evaluate on the pre-image,
`postconditions` on the post-image; either failing aborts the transaction.
- **Delete** — `row_scope` store-enforced; explicit tombstone policy per
entity: soft-delete (status flag via Update plan) or hard-delete. Retention
rules may convert a requested hard delete into `Deny { retention }` or route
to purge workflow.
- **Migrate** — schema migrations are guarded mutations: `action="migrate"`,
maker-checker above a severity threshold, full audit, dry-run mode
mandatory before apply.
- **Export** — disambiguated: export is an **egress action**, not a verb on
both sides. It composes a `ReadPlan` (scope, masks, caps) *plus* egress
obligations: full audit (no sampling), cohort floors, row/byte caps, and
`Escalate { approval }` above configured thresholds. Export is the most
audited action in the system.

`Escalate` is fully disambiguated (was ambiguous in v1): its payload names the
target — `Escalate { to: Approval(chain) }`, `Escalate { to: StepUp(mfa) }`,
`Escalate { to: Materialization(job) }`, or `Escalate { to: StrongerDriver }`.
A decision with an unnamed escalation target is invalid.

**Atomicity contract for update/delete (specified, was underspecified):**

All of the following happen in **one database transaction**:

1. row lock + pre-image read (`SELECT … FOR UPDATE`, or the store's
equivalent) — the `row_scope` predicate is part of this read;
2. `preconditions` evaluated on the pre-image;
3. the write itself, with `row_scope` in its WHERE and
`affected_rows ≤ affected_row_cap` asserted;
4. post-image read (`RETURNING` where supported);
5. `postconditions` evaluated on the post-image.

Isolation: REPEATABLE READ minimum; SERIALIZABLE where the store supports it
and the predicate is complex (hierarchy scopes, relation-lowered sets).
A concurrent change between steps 1 and 3 aborts the transaction (lock or
serialization failure) → retry per configured policy, never apply-then-check.
Ordering with I1: the blocking audit obligation (pre-image + decision) is
durable **before** commit — written in the same transaction where the audit
store is the same database, or via WAL-ahead where it is not. There is no
window in which the mutation is visible but unaudited.

---

## 3. Policy Evaluation Context & Cache Contract (new)

### 3.1 Evaluation context — request-level only, complete by construction

The policy engine sees exactly this (anything else cannot influence policy,
which is what makes caching provable):

```javascript
subject:      id, roles[], claims[], clearance (max classification)
tenant:       tenant_id (mandatory, never optional)
resource:     entity, dataset — the STATIC binding only
destination:  where the result goes: browser | api-client | llm-context |
              export-file | webhook — a first-class policy dimension
              (§1C.3): sending data into an LLM context is a different risk
              than returning it to the user's own browser, and
              GDPR/PCI-style obligations often turn on it
environment:  env (prod/staging), ip/geo/device posture, session freshness
time:         evaluation timestamp (truncated to time_bucket)
query:        QueryShape (§8) — verbs, aggregate presence, subject dimension
purpose:      declared purpose code (e.g., "support", "billing", "analytics")
              — consent/lawful-basis checks consume this
```

**Row-level attributes are not context.** "record owner", "branch", "status"
are per-record facts; for set queries there is no single record at decision
time. They are expressed exclusively as `FilterExpr` field references
(§6) — i.e., as *compiled row predicates*, which is where they belong: the
plan is per-request, the predicate is per-row, and both are cacheable on
their own terms. Putting row attributes in the context would make the
decision cache key per-record (uncacheable) or meaningless (unsafe).

### 3.2 Cache contract

Two cache layers, both keyed conservatively:

- **Decision cache** — key:
`(subject_id, roles_hash, tenant, entity, dataset, action, env,
destination, time_bucket, query_shape_hash, purpose,
policy_version)`. All components are request-level (§3.1); nothing
row-level enters the key.
Hard rules: (a) a cached decision is never shared across tenants unless the
policy compiler proves tenant-independence (cross-tenant sharing is off by
default; the proof is a Phase 4 deliverable, until then keys always contain
tenant); (b) any policy change invalidates immediately — `policy_version`
is part of the key, so mismatch = miss by construction; (c) no cached
decisions for writes, exports, or sensitive-class reads.
- **Fragment cache** — preferred for hot paths: cache *compiled filter
fragments parameterized by context* (e.g., `branch_id = :subject_branch`
with a parameter binding step), so per-request work is parameter binding,
not recompilation.

Guard-down exception (I5) — deliberately a deviation, so it is specified
as strictly as an invariant:

- **Default off, per-domain opt-in:** each domain (entity class) must
explicitly enable degraded reads; the global default is fail-closed.
- **"Non-sensitive" is an explicit allowlist:** classification ≤ a configured
threshold per domain (e.g., PUBLIC/INTERNAL only), AND no destination =
`llm-context` or `export-file`, AND purpose not in the sensitive-purpose
list. "Non-sensitive" is never inferred.
- Policy version pinned; TTL ≤ 60s; **separate kill switch** independent of
the main guard (a guard outage must not also disable the switch that ends
degraded mode).
- **No writes, no exports, no delegation minting, no enumerate** — degraded
mode serves plain scoped reads only.
- **Degraded-mode audit:** every degraded read is recorded locally
(append-only, hash-chained on the writer) and **reconciled into the central
audit chain when the guard recovers** — recovery without reconciliation
fails the health check and alerts.

---

## 4. Filter Expressiveness: Relation Lowering (new)

Design decision: **relations never reach drivers.** One new AST node, erased
at plan-compile time:

```rust
pub enum FilterExpr {
    // Eq, In, And, Or, Not, Compare, TimeRange, Pattern (see §6), TenantEq
    RelationIn { field: FieldPath, relation: RelationExpr },
}
// compile pipeline:  policy --[resolve]--> In { field, values } --[driver]--> native
```

Resolution strategies, chosen per (relation, dataset, cardinality):

- **Tier 0 — native:** predicate on stored columns; no resolution.
- **Tier 1 — decision-time resolution:** relation resolves to a bounded value
set, lowered to `In`. Bounded by `relation.max_cardinality` (declared in
schema); overflow falls to Tier 2 or `Deny { relation.unpushable }`.
- **Tier 2 — materialized column:** hot/unbounded relations maintained as
stored data (`org_path`, `visible_closure`), written through the guard, so
queries become Tier 0. CI guards write-path divergence.
- **Tier 3 — deny/escalate:** unresolvable, unbounded, or negated relation
with no materialization → deny or approved materialization job. Never
silent full-scan filtering.

Rules enforced at policy-validation time:

- **Positive relations only:** negated relations (`Not(RelationIn …)`) are
rejected unless they lower to a Tier 2 materialized column. A negated
snapshot leaks as relationships change (time-of-check leak); this is
static-checkable.
- **Resolution determinism:** relations are pure functions of
(subject, source_epoch); same inputs → same set. Required for replay (I4).
- **Freshness contract:** every resolution is tagged
`(source, source_epoch, resolved_at, ttl)`, embedded in the compiled plan
and the decision trace. Expired plans re-resolve, never execute stale.
- **L1 re-check is defined per tier** (v3: the v2 wording — "L1 re-checks
the original RelationExpr" — implied a per-record graph traversal at L1,
which is neither performant nor always possible):
- **Tier 0:** L1 applies the compiled predicate only; nothing extra.
- **Tier 1:** L1 re-checks membership against a **fresh re-resolution** of
the relation (cached per `(subject, relation, source_epoch)`, never a
per-record traversal — one resolution per batch, not per row).
- **Tier 2:** the materialized column is exact; L1 verifies the column
value only (constant-time per row).
- **Tier 3:** denied at plan time. **L1 is never the enforcement backstop
for a relation the plan layer already rejected** — no graph traversal
ever runs at L1, and no unbounded resolution is attempted there.
For writes, commit-time re-evaluation (I3) re-resolves at the same tier.
- **Batch API:** `resolve_many(subjects, relation)` for bulk flows.
- **Providers:** resolvers are pluggable; OpenFGA/SpiceDB are the primary
ReBAC sources (see §1A). The freshness contract applies identically.
- **Relation graph validated in CI:** cycles and depth limits are errors at
definition time (same machinery as state-machine/SoD checks).

---

## 5. Logical resource model

Unchanged from v1 (Entity → Dataset → Rows → Fields; actions; scope
dimensions), with three fixes:

- `export` appears once, as an egress action (§2).
- `TenantEq` is a **security primitive, not syntax sugar**: it is the one node
drivers must implement natively and attest (§7), guaranteeing tenant
isolation is always pushdown-enforced rather than incidental to `Eq`.
(A store failing to attest native `TenantEq` cannot host multi-tenant data.)
- `PathGlob` generalized to `Pattern { field, matcher }` where `matcher` is a
closed set (Glob | Prefix | Exact) — no file-specific types in core IR.
File drivers interpret Glob/Prefix; stores without path semantics reject
`Pattern` at capability negotiation.

---

## 6. Decision core

`nirdosha-guard-core` remains storage-agnostic. Read side:

```rust
pub struct AccessPlan {                 // read plan (single binding)
    pub decision: Decision,             // Allow | Deny | Escalate{to} | Pending
    pub filter:   Option<FilterExpr>,   // may contain RelationIn (lowered pre-driver)
    pub masks:    Vec<FieldMask>,
    pub caps:     Vec<Cap>,             // §8: row/scan/byte/time/cohort caps
    pub obligations: Vec<Obligation>,
    pub policy_version: PolicyVersion,
}

pub struct FederatedPlan {              // multi-binding read (§1C.1) — the IR
    pub decision: Decision,             // the IR §1C.1 references exists here
    pub sub_plans: Vec<(BindingId, AccessPlan)>,
    pub merge: MergeSpec,               // union|join, dedup keys, provenance on
    pub budget: BudgetToken,            // global caps coordinator hook (§1C.1)
}

// Decision::Pending — defined (was undefined):
//   Pending { handle, expires_at, cause: DuplicateInFlight | AwaitingObligation }
// A duplicate in-flight request parks on the first request's outcome
// (idempotency interlock, §2); an awaiting-obligation pending resolves when
// the obligation (e.g., step-up) completes or expires. Pending always
// carries an expiry and resolves to a terminal decision; it is never a
// resting state.
```

All values parameterized; all compilation via typed ASTs. Invariants I1–I6
from the mutation-gate plan apply unchanged; I7–I12 from v1 retained as
written (safety net, no injection, aggregate control, enumeration control,
delegation audit, capability lies) and extended:

- **I14 — store-enforced write scope:** `WritePlan.row_scope` must be pushed
into the store operation (WHERE on UPDATE/DELETE); L1 alone is insufficient
for writes. Drivers that cannot express the scope natively reject the write.
- **I15 — masked-field predicate control:** a field present in `masks` for
this subject may not appear in `WHERE/ORDER BY/GROUP BY` of the executed
query unless the policy explicitly grants `predicate_use`. See §8.2.

---

## 7. Capability model, attestation, and ladder

Ladder unchanged (L4 full pushdown → L3 pruning → L2 scan-time vectorized →
L1 post-read → L0 deny). Capability *reporting* is hardened per review:

- **Versioned capability manifests:** each driver ships a manifest (schema
version, supported `FilterExpr` nodes, masking points, aggregate
semantics). Runtime behavior is checked against the manifest, not trusted.
- **Attestation beyond error messages** (I12 strengthened): (a) canary rows —
synthetic rows no real subject may see, inserted per partition; any canary
in a result is a lie; (b) differential tests — identical queries through
guard vs. direct store access compared in CI *and* periodically in
production (sampled); (c) query-plan inspection where the store exposes
plans (EXPLAIN), verifying predicates actually appear.
- **Dynamic downgrade semantics:** on a detected lie, the driver downgrades
its effective capability, invalidates its compiled-plan cache entries,
fails in-flight requests that relied on the lied capability (they re-plan
honestly), and alerts. Circuit-breaking follows repeated lies.
- `TenantEq` attestation required for multi-tenant datasets (§5).

---

## 8. QueryShape & Leak Controls (new)

### 8.1 QueryShape contract

`QueryShape` describes the query before compilation: verbs (select/aggregate/
count/exists), aggregate function presence, grouping keys, subject-dimension
presence, ordering, pagination mode. It feeds policy (context, §3.1) and
determines aggregate safety:

- **Inline/DataFusion execution:** the guard controls plan shape — filter is
forced *before* aggregation in the logical plan. Aggregates are safe at any
pushdown level because the guard owns the plan.
- **Store-side aggregation:** allowed only when the filter is fully pushed
(L4) *or* the store natively supports row-filter-then-aggregate with
attestation. Otherwise `Deny { aggregate.unpushable }` (I9).
- **Cohort floors need an identifiable subject dimension in the base:**
the requirement is not "the query groups over subjects" — it is that the
base rows carry a subject ID (directly or via the dataset schema). For any
aggregate query over such a base, the driver injects
`HAVING count(distinct subject_id) >= floor` **per group** (e.g., a query
grouped by department still carries the floor, evaluated over the subjects
within each department) when the store supports it; if the floor is
required and not expressible, deny. A query whose base has no subject
dimension cannot satisfy a mandated floor and is denied.

### 8.2 Cost caps (row caps are insufficient)

`Cap` variants: `RowCap`, `MaxScanRows`, `MaxScanBytes`, `MaxExecutionTime`,
`MaxResultBytes`, `CohortFloor`. `COUNT(*)`/`SUM` without scan caps are
denial-of-wallet vectors; scan caps are pushed into store LIMITs / partition
budgets / query timeouts where possible, else enforced by aborting the stream.

### 8.3 Predicate side channels

A masked field usable in `WHERE`/`ORDER BY`/`GROUP BY` leaks via inference
(binary search on ordering, group cardinalities). The same applies to
**JOIN predicates, HAVING, and window functions** — a masked field inside a
join condition or a `ROW_NUMBER() OVER (ORDER BY salary)` leaks exactly as a
visible ORDER BY does. Policy gains `predicate_use` grants per
(subject-class, field); the SQL/Arrow compilers reject plans where a
masked-but-not-granted field appears in any non-projection clause —
filter, join, grouping, having, ordering, or window specification (I15).
Projection itself only reads allowed columns where the format supports it
(columnar) — masked columns are not decoded at all.

### 8.4 Enumeration, pagination, ordering, existence

- **Opaque cursors only** (encrypted, guard-signed, carrying the plan's
scope hash); offset pagination rejected for guarded resources (offset +
filtering leaks counts).
- **Total counts** only when policy grants `count_allowed`; otherwise the
response omits totals (or returns a capped estimate).
- **Stable, policy-controlled ordering** — ordering by a masked field
requires `predicate_use` (§8.2) or is replaced by a neutral key.
- **Existence oracles:** uniform response shape for "no rows" vs "no access"
at the API boundary (I12's leak channel); internally distinguished, logged,
externally indistinguishable.
- Enumeration endpoints keep `I10` controls (rate caps, row caps, surrogate
external IDs).

---

## 9. Enforcement topologies

### 9.1 Inline

Unchanged: evaluate → compile → native pushdown → L2/L1 residual → masking at
projection → response. For columnar, projection happens before decode of
masked columns. Audit: sampled for low-sensitivity reads (sampling decision
recorded), full for sensitive fields, exports, denials.

### 9.2 Masking placement

Masks apply at the last point before data leaves the process. Inline: on
record batches / typed records, vectorized; masks compiled per
(entity, policy) and cached. Delegated: masks must be expressible in the
credential's DDL (§9.3) or the request is denied — there is no "mask later."

### 9.3 DDL generation without injection (I8 extended)

- **Postgres RLS: prefer no generated DDL.** Pre-created policies that read
session variables — the minted role path sets
`set_config('app.tenant', …, false)` / `app.subject` on the pooled
connection; policies reference `current_setting(...)`. Interpolation-free.
- **Where DDL must be generated** (Hive row filters, column masks, temp
views): a dedicated DDL AST with strict identifier quoting (allowlist) and
literal binding; values never concatenated. Generation is pure code, unit-
tested, fuzzed in CI.
- **Lifecycle:** generated objects are name-spaced (`ng_<cred_id>`),
idempotent, created with `IF NOT EXISTS`, dropped at TTL expiry and at
revocation; a reaper job reconciles leaked objects; name collisions are
impossible by construction (cred_id uniqueness) and asserted.
- **Connection-pool interaction:** scoped sessions use a dedicated pool slot
with the role bound at connection time; the guard verifies the effective
role (`SELECT current_user`) before releasing the connection.

### 9.4 Delegated Enforcement Contract

Admits engines without L1 under these compensating controls (all mandatory):

1. **Least-privilege, exact scope:** the credential encodes exactly the
compiled plan (row filter, column mask, cohort floor, caps). Broader-than-
plan credentials are a generation bug — the minting path asserts scope
equality against the plan (tested in CI with adversarial plans).
2. **If the engine ignores the credential/scope:** the credential itself is
the boundary (engine principal has no other grants); capability lies are
caught by attestation (§7) and downgrade revokes minting for that engine.
3. **No listing:** credentials never grant enumeration rights beyond the
approved scope (S3: no `ListBucket` outside the approved prefix).
4. **TTL + revocation:** TTL ≤ query window; revocation propagates
(connection kill for RLS sessions; invalidation for tokens); in-flight
queries killable where the engine supports it, else TTL bounded.
5. **Full audit, no sampling** for every delegated access (plan, scope,
version, expiry recorded in the hash chain).
6. **Revocation registry + reaper:** all minted credentials registered;
guard-down or policy-change events bulk-revoke.

**S3/object storage correction (v1 was wrong):** per-object signed URLs are
not prefix-scoped access. Use STS `AssumeRole` with a session policy scoped to
the approved prefix (read-only, no listing beyond scope), or a scoped
credential; signed URLs only for single approved objects.

### 9.5 Per-store strategy

| Store | Topology | Strategy |
| --- | --- | --- |
| RDBMS | Inline + delegated | Typed AST → parameterized WHERE; writes carry store-enforced row_scope; RLS via session variables for external tools |
| Columnar (CH/BQ/Redshift) | Inline | Same AST, dialect backends; cohort floor → HAVING; scan caps → LIMIT/timeouts |
| Parquet/Arrow/DataFusion | Inline | Pattern→file set; FilterExpr→PruningPredicate; residual→Arrow RowFilter; masks on batches; masked columns undecoded |
| Files (fs/S3) | Inline + STS | Path predicates; manifests/sidecar classification; decode-time record filter; S3 via scoped STS (§9.4.6) |
| Hive/Spark | Delegated | Generated row-filter/column-mask DDL (DDL AST); temp principal; full audit |
| Time-series | Inline | Time/tag pushdown; retention = time scopes; min-step floors vs bucket averaging |
| Lakehouse | Delegated | Partition-pruned snapshot reference / scoped credential; compaction & deletes are guarded mutations |

---

## 10. Macros and crates

Unchanged layout (core / macros / sql / arrow / file / timeseries / hive /
audit), **plus two new crates**: `nirdosha-guard-federation` (catalog
routing, `FederatedPlan` execution, budget coordinator) and
`nirdosha-guard-mcp` (registry-generated tools, delegation, agent
defaults). Details:

- `#[relation(...)]` — declares relations with source, cardinality bound, TTL
(§4); CI validates the relation graph.
- `#[materialize(relation = ...)]` — Tier 2 columns maintained write-through;
CI guards divergence.
- `policy!` validated against registered schemas: field existence,
classification coherence, positive-relation rule, predicate_use grants.
- `enumerate!` / `dropdown!` — scope-injected enumeration with built-in caps
and rate limits; opaque cursors only.
- `derive(GuardedRecord)` — generates `apply_access_plan` (masks) and masked
projections; the store-agnostic masking executor entry point.
- `cargo nirdosha verify` extensions: read-path inventory, dataset binding
coverage, cross-store classification consistency (same logical field → same
class or an explicit divergence record — review gap, now a CI check),
DDL-generation fuzz corpus run.

---

## 11. Invariants (consolidated)

I1 audit-before-commit · I2 deny-composition · I3 commit-time re-evaluation ·
I4 policy snapshot/replay · I5 fail-closed · I6 complete mutation inventory ·
I7 inline L1 safety net · I8 no injection (incl. DDL AST) · I9 aggregate leak
control · I10 enumeration control · I11 delegation fully audited · I12
capability attestation & dynamic downgrade · **I13 delegated topology has no
L1 net and is admitted only under §9.4 **· **I14 write row_scope store-enforced**
· **I15 masked fields excluded from all non-projection clauses (filter, join,
grouping, having, ordering, window) unless granted** · **I16 merge-layer
re-filtering and re-masking of federated unions** · **I17 agent defaults:
full audit, non-transferable dual-bound delegation, per-agent rate/cost
caps, loop limits, destination control, mandatory evaluate-then-act for
writes**.

---

## 12. Failure modes & revocation semantics

| Failure | Behavior |
| --- | --- |
| Guard down | Fail-closed (I5). Cached fragments for non-sensitive reads ≤60s only with per-domain opt-in, pinned policy version, kill switch (§3.2). |
| Policy change | Immediate invalidation by key construction (§3.2); in-flight executions finish under old version, recorded as such. |
| Store down | Fail-closed; reads deny, writes deny. |
| Pushdown unsupported | Escalate ladder with caps; aggregate-shaped query without full pushdown → deny (I9). |
| Masking unavailable in delegated engine | Deny (§9.2). |
| Credential minting failure | Deny; never fall back to broad credentials. |
| Credential revocation | Propagation bound defined per engine class (RLS session-kill immediate; tokens ≤ TTL); in-flight kill where supported. Bulk-revoke on guard-down/policy-change events. |
| Capability lie | Downgrade + in-flight replan + alert + circuit-break on repeat (§7). |
| Break-glass (§15) | Scoped, time-boxed, dual-approved above threshold, full audit, post-hoc review task auto-created. |

---

## 13. Performance model

- Decision cache and fragment cache per §3.2; compiled-plan cache per
(store, plan-hash); relation resolution cache per
(subject, relation, source_epoch) with TTL.
- Target guard overhead on cached reads **<1 ms measured end-to-end at the
guard boundary, including masking and audit-write enqueue** (v1's target
excluded these; now inclusive).
- Masking vectorized on Arrow batches; masked columns not decoded (columnar);
mask maps compiled per (entity, policy) and reused.
- Read audit sampled for low-sensitivity (decision recorded), full for
sensitive/exports/denials/delegated.
- Guard never in the byte path for analytics (delegated credentials).

---

## 14. Break-glass & emergency access (new)

- Explicit resource-scoped grants, never a global switch; time-boxed (default
≤ 1h), dual approval above a severity threshold, mandatory reason + ticket.
- Every use: full audit (no sampling), auto-created post-hoc review task,
alerting to security channel, counted on the compliance dashboard.
- Break-glass grants are themselves guarded mutations (they transit the gate:
maker-checker applies). No out-of-band path — **which means that during a
guard outage there is no emergency access** (fail-closed, I5). This is a
deliberate, stated property: availability of the guard is the availability
of emergency access.

Because a total outage with zero recourse is its own risk, the out-of-band
path is designed rather than implied: **dual-control sealed procedure** —
two authorized humans jointly execute the minimum necessary action
directly at the store using a pre-registered emergency principal (store
credentials sealed offline, split-knowledge). Every out-of-band action is
**reconciled on recovery**: submitted to the guard's reconciliation
inbox, fully audited, with mandatory post-hoc review, and detected via
the §7 attestation machinery (canary rows / differential tests reveal
un-reconciled out-of-band writes). An emergency-principal action without a
matching reconciliation entry within N hours raises a page-level alert.
Emergency access is possible; invisible emergency access is not.

---

## 15. OpenAPI dry-run

Dry-run endpoints return decision + plan shape + redacted trace: policy
internals, resolved value sets, and other subjects' data are never exposed.
A dry-run response is itself a guarded, rate-limited, audited action — it is
a policy oracle otherwise (§8.4 existence rules apply to its responses).

---

## 16. Phasing (updated)

The four pre-Phase-0 sections required by review are now §2 (write plans),
§3 (context & cache), §9.4 (delegated contract), §8 (QueryShape & leaks) —
all designed, so Phase 0 may proceed.

- **Phase 0:** decision core incl. `WritePlan`/`AccessPlan`/`FederatedPlan`
types, relation lowering, capability manifests + canary attestation
framework. (Scope reduced per review: the DDL AST moves to Phase 4 — no
generated DDL is needed before the delegated topology.)
- **Phase 1:** `#[dataset]`/read-path inventory, schema descriptors,
session-variable RLS path (no generated DDL for Postgres), **catalog
introspection API** (feeds UI/OpenAPI/MCP from one source — pulled forward
per review).
- **Phase 2:** masking executor + `derive(GuardedRecord)`; `enumerate!` with
opaque cursors; **RDBMS inline vertical slice — one read path AND one write
path (review recommendation)**: proves plan→AST→pushdown→masking→L1→audit
on reads and row_scope→pre/post-image→caps→audit on writes.
- **Phase 3:** Arrow/Parquet driver; file driver; relation resolvers + L1
re-check; cost caps.
- **Phase 4:** delegated topology (RLS roles for external tools, Hive DDL
generation, S3 STS); differential attestation in prod; policy backtesting
extended to read plans.
- **Phase 5:** TSDB driver, lakehouse credentials, OpenAPI dry-run
(redacted), **`nirdosha-guard-mcp`** alongside the OpenAPI surface, with
the §1C.3 agent defaults enforced from the first tool.
- **Phase 6:** native compiler embedding.

## 17. Open questions (resolved)

The four questions left open in earlier drafts are now decided. The
answers below are chosen to fit Nirdosha's constraints: no free `str` in
user function signatures, no runtime string formatting, typed and
compile-time-enforced preference, and reuse of existing mechanisms
(`requires`, `validate`, `effect`, and the macro registry).

### 17.1 Masking transforms: type-class driven, phased by capability

**Decision:** static masks are the default; format-preserving transforms
follow; vault-backed tokenization is deferred until true reversibility is
required.

Rationale: Nirdosha cannot build strings at runtime, so masking cannot be a
free `"***"` template. Instead, each classified type implements a typed
`Mask` trait / type-class; policies name transforms from a **closed
registry** (`full`, `partial_last4`, `hash`, `drop`). The literal mask
value comes from source, satisfying the language's `str` rules.

Canonical form (see `crates/nirdosha-rt/tests/guard_policy_macro.rs` and
`crates/nirdosha-rt/tests/guard_attributes.rs` for the v2 surface; full
`examples/hld/*.nir` files have been removed because they targeted the
deprecated native `.nir` compiler):

```rust
#[classify(level = "CONFIDENTIAL")]
struct CardNumber { value: String }

#[mask_transform(name = "partial_last4")]
fn partial_last4(c: CardNumber) -> CardNumber { ... }

nirdosha_rt::guard_policy! {
    allow "support-view-card" when action == "read" && resource == "payment_card"
    mask(card_number, partial_last4)
    mask(cvv, full)
}
```

**Phasing:**
- MVP / Slice A: default static mask per classification.
- Slice B–C: format-preserving registry entries.
- Slice D+: vault-backed tokenization, only where de-tokenization is
  required; gated by a `#[vault(...)]` dependency declaration.

### 17.2 Read-audit sampling: explicit classification table

**Decision:** sampling rates are a static, reviewed configuration artifact;
denials, exports, delegated access, break-glass, and sensitive-field reads
are always 100% audited.

Rationale: adaptive sampling is a side channel and a debugging burden. A
static table is deterministic, cache-friendly, and itself auditable. The
table must be **total over all declared classifications**; a missing entry
is a compile error.

Canonical form (see `crates/nirdosha-rt/tests/guard_policy_macro.rs` for
the v2 macro surface; the old `examples/hld/audit_sampling.nir` has been
removed):

```rust
nirdosha_rt::audit_sampling! {
    classification: PUBLIC       -> rate 0.0;
    classification: INTERNAL     -> rate 0.01;
    classification: CONFIDENTIAL -> rate 1.0;
    classification: RESTRICTED  -> rate 1.0;
}

nirdosha_rt::audit_rules! {
    always_full: [deny, export, delegated, break_glass, sensitive_field_read];
}
```

The security owner owns the table; `cargo nirdosha verify` checks
completeness and emits the table into the compliance dashboard.

### 17.3 Purpose-code taxonomy: closed enum + tenant-extensible registry

**Decision:** purpose is a typed enum with a small closed core; tenants may
register additional codes, each with a lawful basis and review date.

Rationale: `str` cannot be a policy dimension without breaking cacheability
and inviting injection. Enum variants hash cleanly and participate in the
decision cache key. Tenant extensions prevent core bloat while keeping the
set closed and reviewable.

Canonical form (see `crates/nirdosha-rt/tests/guard_attributes.rs` for
the v2 macro surface; the old `examples/hld/purpose_taxonomy.nir` has been
removed):

```rust
#[purpose(code = "clinical_trial_23", basis = "consent", review = "2025-06-01")]
enum LocalPurpose { ClinicalTrial23 }

nirdosha_rt::guard_policy! {
    allow "support-lookup" when action == "read" && resource == "customer"
        && purpose == Purpose::Support
}
```

Every `policy!` block must reference a registered `Purpose` variant. The
`purpose` dimension also drives destination control: `purpose ==
Purpose::Agent && destination == Destination::LlmContext` is a separate
policy path with its own caps and obligations.

### 17.4 `Condition` / invariant DSL: `FilterExpr` first, then a pure registry

**Decision:** Phase 0 supports only `FilterExpr` conditions. Phase 2 adds
a small `#[invariant]` registry of pure, schema-typed functions that reuse
the existing `validate` contract machinery.

Rationale: a general-purpose invariant language would become a second policy
engine and recreate the injection surface the guard is trying to close.
`FilterExpr` already covers the common cases ("old status in {A, B}", "new
balance ≤ credit_limit"). For anything richer, a pure Rust/Nirdosha
function registered at compile time is safer and more expressive than a
mini-language.

Canonical form (see `crates/nirdosha-rt/tests/guard_attributes.rs` for
the `#[invariant]` v2 attribute surface; the old
`examples/hld/invariant_conditions.nir` has been removed):

```rust
#[invariant(name = "no_overdraft", schema = "account")]
fn no_overdraft(old: Account, new: Account) -> bool {
    return new.balance >= 0
}

nirdosha_rt::guard_policy! {
    allow "balance-update" when action == "update" && resource == "account"
    requires field(status).in(["active"])
    requires invariant(no_overdraft)
}
```

Rules for registry invariants:
1. **Pure** — no I/O, no handles, no mutation.
2. **Schema-typed** — arguments must be declared entities/structs in the
   dataset registry.
3. **Registered at compile time** — no dynamic loading.
4. **Run inside the same transaction** as the write (§2 atomicity
   contract), against the pre-image and/or post-image.

A stringly condition DSL is explicitly out of scope.

---

## 18. Review-round-1 resolution index

| # | Comment | Resolution |
| --- | --- | --- |
| B1 | Write-side semantics missing | §2 `WritePlan` (pre/postconditions, field policy, store-enforced row scope, I14) |
| B2 | Cache key unsafe | §3.2 two-layer cache, conservative keys, tenant rules |
| B3 | Delegated contradicts safety net | §1 principle 3 scoped to inline; §9.4 compensating controls; I13 |
| B4 | RLS/DDL injection | §9.3 session-variable RLS preferred; DDL AST + lifecycle |
| B5 | S3 prefix URLs wrong | §9.4.6 STS AssumeRole with session policy |
| B6 | Aggregate imprecision | §8.1 QueryShape contract, who-executes rule, subject injection, scan caps |
| B7 | Capability attestation | §7 manifests, canaries, differential tests, downgrade semantics |
| B8 | Failure/revocation | §12 tightened guard-down, invalidation, revocation propagation |
| G1 | Eval context undefined | §3.1 |
| G2 | Predicate side channels | §8.2, I15 |
| G3 | Pagination/ordering/count leaks | §8.4 opaque cursors, count gating, uniform responses |
| G4 | Cost caps | §8.2 five cap variants |
| G5 | Cross-store consistency | §10 CI check |
| G6 | Masking placement | §9.2 |
| G7 | Break-glass | §14 |
| G8 | Dry-run oracle | §15 |
| E1–E7 | Editorial | export disambiguated (§2); Escalate targets (§2); TenantEq rationale (§5); I1–I6 summarized in header; <1 ms now inclusive (§13); PathGlob→Pattern (§5) |

Review round 2 (build-on-primitives):

| # | Comment | Verdict / Resolution |
| --- | --- | --- |
| R2-1 | Embed Cedar as policy front-end | Adopt, with boundary: `PolicyFrontend` trait; lowerable-subset rule; outside-subset → Tier 3, never "app filters" (§1A) |
| R2-2 | sqlparser-rs for SQL backbone | Adopt as round-trip verifier; typed emitter stays ours (I8: sqlparser AST holds literals, not bound params) (§1A) |
| R2-3 | DataFusion PruningPredicate/RowFilter | Adopt wholesale; driver = policy→DataFusion bridge (§1A) |
| R2-4 | OpenFGA/SpiceDB for ReBAC | Adopt as `RelationResolver` providers; checkpoint tokens → `source_epoch`; no graph engine (§1A, §4) |
| R2-5 | "The gap is the glue" reframe | Adopted as Principle 6 and the product positioning (§1A) |
| R2-6 | Study cerbos adapters / Immuta k-anonymity / Trino#1480 | Adopted as mandatory pre-Phase-2/4 reading with concrete takeaways each (§1A) |
| R3-1 | Ensure all of this is expressible in Nirdosha syntax | §1B: IR-totality via exhaustive coverage matrix; bidirectional doc conformance; registry verification; linted escape valves; stability tiers; expressiveness is the Cedar spike's exit gate |
| R4-1 | Store discovery + unified multi-store view | §1C.1: catalog routing metadata, federated planner, merge-layer enforcement (I16), freshness watermarks, no silent partial unions |
| R4-2 | Enum codes in one store, names in another, governed options | §1C.2: `#[reference]` datasets; label resolution is a guarded read; masks apply to labels; drop-with-label; surrogate external IDs |
| R4-3 | MCP server so LLMs know which API to call | §1C.3: MCP as first-class guarded client; tools generated from registry; delegation tokens; full audit (I17); prompt-injection containment; masked fields absent from model context |
| R5-1 | Row-level attrs must leave the eval context | §3.1 split request-level context vs row-level (FilterExpr only); cache key corrected (§3.2) |
| R5-2 | L1 re-check of RelationExpr implies per-record graph traversal | §4: per-tier L1 definition; Tier 3 denies at plan time; no traversal at L1 |
| R5-3 | Cedar vs `policy!` boundary unclear | §1A: explicit pipeline; `policy!` = canonical sugar over Cedar; subset check at two defined points |
| R5-4 | Guard-down exception still a backdoor | §3.2: default-off per-domain opt-in, explicit non-sensitive allowlist, separate kill switch, degraded-mode audit with mandatory reconciliation |
| R5-5 | FieldPolicy default ambiguous | §2: unlisted fields FORBIDDEN — write-side default deny |
| R5-6 | Write pre/post-image atomicity underspecified | §2: single-transaction contract (lock→pre-image→write→post-image), isolation levels, no apply-then-check |
| R5-7 | Cohort floor wording too narrow | §8.1: subject dimension in the base; per-group floor injection; deny if unexpressible |
| R5-8 | Break-glass during guard down undefined | §14: stated fail-closed property + dual-control sealed out-of-band path with mandatory reconciliation |
| R5-9 | Pending undefined; Phase 0 too broad; Condition a placeholder; side channels miss JOIN/HAVING/window; sqlparser params unverified | §6 (Pending defined), §16 (DDL AST→Phase 4), §2 (`#[invariant]` registry), §8.3 (JOIN/HAVING/window added), §1A (per-dialect parameter round-trip conformance) |
| R5-10 | §1C gaps: federated IR, cross-store aggregation, caps coordinator, join routing, destination dimension, agent binding, tool-description injection, I16/I17 placement, MCP crates/phasing, `#[reference]` tier, "authoritative", write federation | §6 (`FederatedPlan`), §1C.1 (disjoint/dedup/deny; fail-closed budget coordinator; declared authority; single-binding writes), §1C.2 (store-locality join routing; clarifications; missing-label enum), §3.1+§1C.3 (destination; non-transferable tokens; mandatory evaluate-then-act; signed static tool descriptions), §11 (I16/I17), §10/§16 (`-federation`, `-mcp` crates; catalog→Phase 1; MCP→Phase 5), §1B (`#[reference]` tiered) |
| R6-1 | Masking transforms undecided | §17.1: type-class driven, closed registry, phased static → format-preserving → tokenization; v2 smoke test in `crates/nirdosha-rt/tests/guard_policy_macro.rs` |
| R6-2 | Audit sampling table undecided | §17.2: explicit per-classification table, always-full rules, CI totality; v2 smoke test in `crates/nirdosha-rt/tests/guard_policy_macro.rs` |
| R6-3 | Purpose taxonomy undefined | §17.3: closed enum core + tenant-extensible `#[purpose]` registry; v2 smoke test in `crates/nirdosha-rt/tests/guard_attributes.rs` |
| R6-4 | Condition/invariant DSL unresolved | §17.4: Phase 0 `FilterExpr`, Phase 2 `#[invariant]` pure registry; reuses `validate`; no stringly DSL; v2 smoke test in `crates/nirdosha-rt/tests/guard_attributes.rs` |