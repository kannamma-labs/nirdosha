# Nirdosha Guard — Updated Implementation Plan v2

This revision bakes the review findings and performance design into the plan itself. Three things changed structurally:

1. **Phase 0 now covers decision semantics** (composition algebra, obligation classes, commit-time re-evaluation) — these were the load-bearing gaps.
2. **Performance primitives are Phase 0/1, not retrofits** — partitioned audit chains and KV counters are designed in before scale arrives.
3. **Every phase closes with explicit invariants** — testable statements that CI enforces, so drift is caught mechanically.

---

## Hard invariants (hold across all phases, CI-enforced)

These are non-negotiable from the first commit:

- **I1 — Audit-before-commit:** no state mutation is durably visible without its audit record in the same partition chain. Enforced by: decision record carries `audit_seq`; storage layer refuses commit without a populated `audit_seq`.
- **I2 — Deny-composition:** `deny` from any layer overrides all `allow`/`escalate` except an active, resource-scoped, time-boxed break-glass grant. Documented in one module (`decision/composition.rs`), tested exhaustively.
- **I3 — Commit-time re-evaluation:** any mutation that passed through `escalate`/`approval` is re-evaluated at commit against current state. Approval is a *certificate over a diff*, not a license to write.
- **I4 — Policy snapshot on every decision:** `policy_version` + context snapshots (risk score, screening result, geo) recorded in the decision trace. Reproducibility: `replay(decision_id)` must re-derive the outcome.
- **I5 — Fail-closed default:** every domain is fail-closed unless a written, expiring exception exists in the bypass registry. Fail-open is data, not config drift.
- **I6 — Complete mutation inventory:** `cargo nirdosha verify` fails the build if any state-changing path (route, job, consumer, migration, seed) lacks either a guard attribute or an explicit `#[guard_exempt(reason, ticket, expires)]`.

---

## Phase 0 — Decision core + partitioned audit spine (was: gate skeleton)

Goal: the semantics are correct before anything builds on them.

**Decision algebra (new — this is the heart of v2):**
- `GuardDecision = Allow { obligations } | Deny { reasons[] } | Escalate { approval_chain, revalidate_at_commit } | Pending { handle, expires_at }`
- **Obligation classes:** `Blocking` (durable audit — must succeed before commit; failure = deny), `Ordered` (run post-commit in defined order, e.g., notify then index), `BestEffort` (async, never affects outcome).
- **Composition rules (I2):** explicit precedence table — deny > break-glass-allow > escalate > allow; obligations merged with dedup by `kind+target`; conflicting ordered obligations on same target → deny with `obligation.conflict`.
- **Escalation depth:** max 1 step-up re-evaluation; second escalation → `Deny { reason: escalation.loop }`. No unbounded loops.
- **Decision trace schema:** designed for N decisions per request from day one (bulk semantics), each with `policy_version`, context snapshot, layer timings.

**Partitioned audit chain (replaces single-chain design):**
- Chains partitioned by `(tenant, service)`; each has a monotonic `seq`, hash `h_n = H(h_{n-1}, record_n)`.
- Periodic **checkpoint root**: Merkle root over all partition heads every N seconds/minutes, anchored to an external timestamp source. Tamper-evidence preserved without global ordering.
- **Batched writes:** records buffered per partition (window 10–25ms or 64 records), hashed and appended as a batch; async fsync with WAL-tail replay on crash (bounded durability window, documented per domain).
- Audit store access is itself guarded: reading audit records is a guarded action, audited.

**Guard service skeleton:**
- `guard::evaluate(subject, action, resource, context) -> GuardDecision`
- `#[nirdosha_guard(action, resource)]` macro; `mutate!` with `dry_run: true` path returning full trace.
- Router wiring: `expose!` auto-tags mutating routes.
- **Decision cache:** keyed on `(subject, action, resource_version, context_bucket, policy_version)`, TTL ≤30s. Disabled by default for financial actions until Phase 2 sign-off.

**Closes:** decision composition ✦ escalation loops ✦ audit-before-commit ✦ audit partitioning ✦ reproducibility ✦ dry-run.

---

## Phase 1 — Identity, auth, complete mutation inventory

Goal: know *who*, and prove *nothing bypasses the gate*.

- Multi-IdP registry (IdPRegistry config + loader), OIDC discovery, SAML adapter, JWT validation.
- mTLS/SPIFFE identity verifier for service-to-service.
- **Session revocation store with cache:** local session cache, TTL 30–60s, revocation propagates within TTL; `CredentialFreshness` and `StepUpMfa` obligations wired into the escalation path (not ad-hoc checks).
- `rbac_admin!` screens; roles/claims compiled into policy.
- **Mutation inventory (I6):** static analysis pass listing every `INSERT/UPDATE/DELETE` path — routes, `scheduler!` jobs, queue consumers, migrations, seeds. Every entry either has `#[nirdosha_guard]` or `#[guard_exempt]` with ticket + expiry. **Data-layer backstop:** DB roles deny write access to the application user except through guard-approved connections — so the gate is enforcement, and the DB is the second line, not the only line.
- Migrations go through the guard as `action = "schema.migrate"` with maker-checker above a severity threshold.

**Closes:** multi-IdP ✦ OIDC/SAML ✦ mTLS ✦ session revocation ✦ non-HTTP mutation paths ✦ RBAC admin.

---

## Phase 2 — State, financial controls, KV-backed counters

Goal: domain rules with the performance design built in.

- `#[immutable]` + compensating-entry convention for ledger/financial records (direct mutation = hard deny at state layer).
- `#[workflow_state_machine]` with CI **reachability check** (no dead states, no forbidden transitions reachable).
- **Type-inferred classification (review fix):** fields typed `Money`, `CardNumber`, `Iban`, `Email`, `Phone` auto-inherit classification from the type; `#[classify]` is for overrides only; CI fails on unclassified sensitive-typed fields.
- **LimitService on KV counters:** atomic `INCR`+TTL for velocity limits; local in-memory counters with periodic flush for low-stakes limits; strict DB counters only for money-movement caps. Per-tenant fairness limits at the gate entrance.
- **IdempotencyStore:** key scoped to `(tenant, subject, action)`; duplicate key + different payload → `Deny { idempotency.payload_mismatch }`; concurrent same-key → second request parks on `Pending` until first resolves.
- `approval_chain!` with quorum + delegation (delegation revocable, chains depth-limited); **commit-time re-evaluation (I3)** implemented here and tested: approve at version N, mutate to N+2 before commit → re-run cheap layers, diff-check approved fields.
- `scheduler!` for future-dated mutations (validated at schedule time, re-evaluated at fire time).
- `notification_inbox!`.

**Closes:** immutable ledger ✦ state machines ✦ sealed/frozen records ✦ thresholds/velocity ✦ dual control ✦ idempotency ✦ scheduled mutations ✦ approval chains ✦ stale-approval hole.

---

## Phase 3 — Context, risk, environment

Goal: dynamic context without becoming the latency tail.

- RequestContext extractor: IP/geo (cached provider responses), device posture, environment separation (prod credentials rejected on non-prod data and vice versa — checked at identity layer).
- **External call discipline (performance contract):** every external dependency (risk score, sanctions/AML, geo) behind a uniform adapter with: hard timeout (p99 budget), circuit breaker, cached last-good fallback, conservative default + alert on breaker-open.
- Impossible-travel detector.
- Risk-score-driven **policy tier selection:** low risk → fast path; elevated → full pipeline with step-up.
- Load-shedding policy: under overload, layer 5 (risk/context) degrades to cached fallback first; layers 0–2 and audit never shed.
- SoD DSL with **CI satisfiability check** (no rule sets that make required flows impossible).

**Closes:** rate limiting ✦ geo-fencing ✦ device posture ✦ env separation ✦ risk gating ✦ watchlist deltas ✦ SoD conflicts.

---

## Phase 4 — Compliance, policy ops, governance

Goal: the gate governs itself.

- Classification-driven compliance defaults (PCI/GDPR/SOX/KYC domains).
- Consent store for PII mutations; retention enforcement (block delete → route to purge workflow).
- **PolicyStore:** versioned policies, compiled-artifact signing, `verify-artifact` offline check, `--audit` re-checks policy source hashes.
- **Shadow mode + backtesting:** new policies run log-only *and* replayed against the last 30–90 days of recorded decisions (`replay(decision_id)` from I4 makes this possible) — deploy with a deny/escalate delta report, not blind.
- **Bypass/break-glass registry:** resource-scoped, time-boxed, mandatory reason + ticket, auto-expiry, every use generates a post-hoc review task.
- **Policy test framework:** unit tests per policy + scenario suite (compliance matrix from the original doc becomes executable tests).
- Observability: deny rate, override rate, latency per layer, audit-lag, checkpoint-root verification job (periodic re-hash of partition chains).
- FIPS profile wiring; artifact evidence for gate binary + policy bundle.

**Closes:** PCI/GDPR/SOX/KYC ✦ policy versioning ✦ shadow mode ✦ explainability ✦ signed trust chain ✦ observability.

---

## Phase 5 — External surface & UX

- OpenAPI generation from `#[nirdosha_guard]` metadata (includes the dry-run endpoint — make it public early for integrators).
- ARIA in generated screens; `search_screen!` over guarded resources; import/export jobs (needs blob type).
- **Import flow uses bulk semantics from Phase 0:** per-item decisions in one trace, partial-failure report, compensating saga for committed batches.

## Phase 6 — Native compiler / product lane

Unchanged: conditional service embedding, embedded UI assets, high-perf HTTP path, OTLP export, mobile profiles sharing guard policies.

---

## What to build first (concrete order, first 4–6 weeks)

1. `decision/` module: types, composition table (I2), obligation classes, trace schema.
2. `audit/` module: partitioned chain, batching, checkpoint root, WAL recovery.
3. `mutate!` macro + `#[nirdosha_guard]` + dry-run.
4. Commit-time re-evaluation hook (even before approvals exist — the seam matters).
5. Mutation inventory check in `cargo nirdosha verify` (I6).
6. Session cache + revocation (identity at coarse level is enough to make audit entries meaningful).
7. KV counter adapter behind the `LimitService` trait (SQLite-local first, Redis-compatible interface).

## Test matrix additions (from review)

- **Property tests:** composition algebra — random decision sets must never produce both allow and deny.
- **Crash tests:** kill the process mid-batch; chain must recover via WAL tail with no lost committed mutation.
- **Concurrency tests:** 2k concurrent mutations on one tenant and one hot account — assert no counter drift, no audit gaps, no duplicate application.
- **Stale-approval test:** approve → mutate → commit must re-evaluate (I3).
- **Bypass audit test:** every `#[guard_exempt]` entry expires or fails CI.

The big shift from v1: **semantics and the audit partition design land before any policy layer**, and every phase ends with CI-enforced invariants rather than "screens built." Want me to detail the `decision/` module — the actual Rust types for `GuardDecision`, obligations, and the composition table — as the next concrete artifact?