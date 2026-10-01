# RFC 0023 — Implementation Checklist

This file tracks concrete implementation progress against `rfcs/0023-data-guard.md`. Items are checked only once they are **built and verified** in the repo, not when they are merely designed or stubbed.

Status legend (same discipline as `docs/ROADMAP.md`):
- `[DONE]` — verified (compiles/tests pass, or exercised end-to-end).
- `[PARTIAL]` — real, verified progress; the remaining gap is named.
- `[OPEN]` — scoped, not started.
- `[BLOCKED: X]` — cannot start until X lands.

Last updated: 2026-09-18 — reverted legacy `.nir` compiler changes; guard surface now wired into the v2 `nirdosha-rt` macro re-exports with passing tests.

---

## 1. Foundation: `nirdosha-guard-core` IR crate

- `[DONE]` Crate skeleton (`crates/nirdosha-guard-core`) with `lib.rs`.
- `[DONE]` Core decision types: `Decision`, `Pending`, `Escalate` + `EscalateTarget`.
- `[DONE]` Read plan types: `AccessPlan`, `FederatedPlan`, `MergeSpec`, `BudgetToken`.
- `[DONE]` Write plan types: `WritePlan`, `WriteAction`, `Condition`, `FieldPolicy`.
- `[DONE]` Filter IR: `FilterExpr` (Eq, In, And, Or, Not, Compare, TimeRange, Pattern, TenantEq, RelationIn).
- `[DONE]` Policy surface types: `Cap`, `Obligation`, `FieldMask`, `PolicyVersion`, `QueryShape`.
- `[DONE]` Capability manifest types: `CapabilityManifest`, `DriverAttestation`, `FilterNodeSupport`.
- `[DONE]` Relation resolver trait (`RelationResolver`) + `ResolutionTier`/`Freshness`.
- `[DONE]` Evaluation context (`EvaluationContext`) — request-level only, §3.1.
- `[DONE]` `PolicyFrontend` trait boundary (Cedar front-end slot).
- `[DONE]` Unit tests for IR construction / JSON round-trip.

## 2. Compiler / language surface

- `[DONE]` `policy!` parsed as a v2 Rust macro; compiles in `nirdosha-rt` tests.
- `[DONE]` `#[dataset]`, `#[relation]`, `#[classify]`, `#[materialize]`, `#[reference]`, `#[mask_transform]`, `#[invariant]`, `#[purpose]` parsed as v2 Rust attribute macros; compiles in `nirdosha-rt` tests.
- `[DONE]` `audit_sampling!`, `audit_rules!`, `enumerate!`, `break_glass!`, `approval_chain!` available as v2 Rust macros through `nirdosha_rt`.
- `[ ]` `field_policy { ... }`, `requires`, `ensures`, `mask(...)`, `cap(...)`, `escalate to ...`, `obligate ...` lowered to IR inside `policy!`.
- `[DONE]` HLD examples rewritten as v2 Rust programs: `examples/nirdosha-v2-corpus/src/60_data_guard.nir` with `cargo check`/`cargo run`/`cargo test` verified.
- `[ ]` Full `AccessPlan`/`WritePlan` lowering from `guard_policy!` blocks; currently macros are no-ops.
- `[ ]` Typeck / ownership integration: at minimum no false-positive errors on guard items.

## 3. Write plans (§2)

- `[ ]` `WritePlan` can be constructed from a `policy!` block.
- `[ ]` Per-action semantics: Create (tenant required), Update (row_scope + affected_row_cap), Delete (soft/hard tombstone policy), Migrate (dry-run + maker-checker), Export (read + egress obligations).
- `[ ]` Pre/postcondition lowering to `FilterExpr`.
- `[ ]` Atomicity contract documented in crate API (pre-image read → lock → write → post-image read).
- `[ ]` Blocking audit obligation hook (`audit-before-commit`, I1).

## 4. Policy evaluation context & cache (§3)

- `[ ]` `EvaluationContext` is the *only* input to policy evaluation.
- `[ ]` Decision cache key excludes row-level attributes.
- `[ ]` Fragment cache for parameterized filter fragments.
- `[ ]` Guard-down exception: opt-in per domain, TTL ≤ 60s, separate kill switch, reconciliation.

## 5. Relation lowering (§4)

- `[ ]` `RelationIn` node in `FilterExpr`.
- `[ ]` Resolution tiers: Tier 0 native, Tier 1 bounded `In`, Tier 2 materialized column, Tier 3 deny/escalate.
- `[ ]` Positive-relation rule enforced (no negated relation unless Tier 2 materialized).
- `[ ]` OpenFGA/SpiceDB `RelationResolver` trait slot.
- `[ ]` Batch API `resolve_many`.
- `[ ]` Freshness contract (`source_epoch`, TTL) embedded in plan + trace.

## 6. Capability model & ladder (§7)

- `[PARTIAL]` Versioned capability manifests emitted by driver crates — `MemStoreDriver` and the new `PostgresStoreDriver` (`crates/nirdosha-guard-store-postgres`) both emit a real `CapabilityManifest` with `schema_version`; nothing yet cross-checks a manifest's claims against reality (that's the attestation-framework line below, still open).
- `[ ]` Capability ladder: L4 pushdown → L3 pruning → L2 scan-time → L1 post-read → L0 deny.
- `[ ]` Attestation framework: canary rows, differential tests, query-plan inspection.
- `[ ]` Dynamic downgrade semantics on detected lie.
- `[ ]` `TenantEq` attestation required for multi-tenant datasets.

## 7. QueryShape & leak controls (§8)

- `[ ]` `QueryShape` type (verbs, aggregate, grouping, subject dimension, ordering, pagination).
- `[ ]` Aggregate leak control (I9): pushdown-first, otherwise deny.
- `[ ]` Cohort floor → `HAVING` translation.
- `[ ]` Cost caps: `RowCap`, `MaxScanRows`, `MaxScanBytes`, `MaxExecutionTime`, `MaxResultBytes`, `CohortFloor`.
- `[ ]` Masked-field predicate control (I15): filter/join/grouping/having/ordering/window.
- `[ ]` Opaque cursor pagination; offset rejected.
- `[ ]` Uniform "no rows" vs "no access" response shape.
- `[ ]` Enumeration rate caps + surrogate external IDs (I10).

## 8. Enforcement topologies (§9)

- `[ ]` Inline topology: evaluate → compile → native pushdown → L2/L1 residual → masks at projection.
- `[ ]` Delegated topology contract (§9.4): least-privilege exact scope, TTL + revocation, no listing, full audit, reaper.
- `[DONE]` Postgres RLS via session variables (preferred; no generated DDL) — `crates/nirdosha-guard-store-postgres`, verified against a real Postgres container with a non-superuser probe role (`docs/adr/0013-postgres-store-driver-pooling-and-rls.md`).
- `[ ]` DDL AST for stores that require generated row-filter/column-mask DDL.
- `[ ]` S3 STS `AssumeRole` with session-policy scope (not signed URLs).

## 9. Drivers (RDBMS, Arrow/Parquet, Files, Hive/Spark, TSDB, Lakehouse)

- `[PARTIAL]` RDBMS driver: typed AST → parameterized WHERE (`RdbmsEmitter` now covers `Eq/In/Compare/And/Or/Not/TimeRange/Pattern/TenantEq`, `crates/nirdosha-guard-core/src/drivers/rdbms.rs`) and a real Postgres `StoreDriver` exists (`crates/nirdosha-guard-store-postgres`); still missing: row_scope-bounded UPDATE/DELETE against arbitrary existing rows (today's driver is a keyed upsert by `resource`, not a WHERE-scoped bulk mutation) and read-path execution.
- `[ ]` Arrow/Parquet driver: `FilterExpr` → `PruningPredicate` + `RowFilter`; masks on batches.
- `[ ]` File driver: path predicates + scoped STS.
- `[ ]` Hive/Spark driver: delegated DDL generation.
- `[ ]` TSDB driver: time/tag pushdown.
- `[ ]` Lakehouse driver: partition-pruned snapshot / scoped credential.

## 10. Federation, reference data, MCP (§1C)

- `[PARTIAL]` `nirdosha-guard-federation` crate: skeleton with catalog router, federated planner, budget coordinator; execution engine is scaffolding.
- `[ ]` Merge-layer re-filter + re-mask (I16).
- `[ ]` Cross-store aggregation safety: disjoint / dedup-key / deny.
- `[ ]` `#[reference]` governed reference-data resolution.
- `[PARTIAL]` `nirdosha-guard-mcp` crate: skeleton with tool descriptions, delegation token shape, default-off `submit_write`; full registry generation and token minting are scaffolding.
- `[ ]` MCP tool descriptions signed build-time artifacts.

## 11. Break-glass & emergency access (§14)

- `[ ]` `break_glass!` macro: scoped, time-boxed, dual-approved, reason + ticket.
- `[ ]` Full audit, auto review task, alert, compliance dashboard count.
- `[ ]` Guard-down = no break-glass; dual-control sealed out-of-band procedure with reconciliation.

## 12. Macros & registry

- `[PARTIAL]` `nirdosha-guard-macros` crate: skeleton proc macros for `policy!`, `#[dataset]`, `#[relation]`, `#[classify]`, `#[materialize]`, `#[reference]`, `#[mask_transform]`, `#[invariant]`, `#[purpose]`, `audit_sampling!`, `audit_rules!`, `enumerate!`, `break_glass!`, `approval_chain!`; no actual IR lowering yet.
- `[ ]` Registry emission (`linkme`-style) for cross-item verification.
- `[ ]` `cargo nirdosha verify` extensions: policy/schema coherence, coverage matrix totality.
- `[ ]` Exhaustive coverage matrix linking IR variants to canonical syntax.

## 13. Invariants

- `[ ]` I1 audit-before-commit.
- `[ ]` I2 deny-composition.
- `[ ]` I3 commit-time re-evaluation.
- `[ ]` I4 policy snapshot/replay.
- `[ ]` I5 fail-closed.
- `[ ]` I6 complete mutation inventory.
- `[ ]` I7 inline L1 safety net.
- `[ ]` I8 no injection (incl. DDL AST).
- `[ ]` I9 aggregate leak control.
- `[ ]` I10 enumeration control.
- `[ ]` I11 delegation fully audited.
- `[ ]` I12 capability attestation & dynamic downgrade.
- `[ ]` I13 delegated topology admitted only under §9.4.
- `[ ]` I14 write row_scope store-enforced.
- `[ ]` I15 masked fields excluded from non-projection clauses unless granted.
- `[ ]` I16 merge-layer re-filtering/re-masking of federated unions.
- `[ ]` I17 agent defaults.

## 14. Examples & docs

- `[ ]` End-to-end federated get-options HLD example (v2 Rust program).
- `[DONE]` v2 macro smoke tests: `guard_policy_macro.rs`, `guard_attributes.rs` in `crates/nirdosha-rt/tests/`.
- `[DONE]` Crate READMEs for `nirdosha-guard-core`, `-federation`, `-mcp`, `-macros`.
- `[ ]` RFC text updated to reference implemented v2 artifacts.
