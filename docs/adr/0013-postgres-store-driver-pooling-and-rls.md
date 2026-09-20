# 0013: Postgres `StoreDriver` — duplicated pooling/TLS, RLS via session variables, resource-keyed tenant handoff

Date: 2026-09-20
Status: accepted

## Context

`rfcs/0026-metadata-plane-checklist.md` carried an honest, still-open line:
"Live vendor capability attestation and production store drivers." Only
`nirdosha_guard_mic::MemStoreDriver` (in-memory) implemented the
`StoreDriver` trait (`crates/nirdosha-guard-mic/src/lib.rs`) that
`GuardClient::guarded_apply` calls to `prepare`/`commit` a mutation after a
policy decision. `crates/nirdosha-guard-core/src/drivers/rdbms.rs`'s
`RdbmsEmitter` already compiled `FilterExpr` into SQL text and RLS session
settings, but nothing executed that SQL against a real database — no
production `StoreDriver` existed.

A related, unplanned finding surfaced while scoping this: `RdbmsEmitter::emit_where`
only handled `Eq`/`In`/`TenantEq`/`And`/`Or`/`Not`. Every other `FilterExpr`
variant (`Compare`, `TimeRange`, `Pattern`) fell through a wildcard arm to
`"1=1"` — a silently unfiltered scan, not an error. Fixed as part of this
same change (see `crates/nirdosha-guard-core/src/drivers/rdbms.rs`'s test
module for coverage of each case, including `LIKE`-metacharacter escaping
for `Pattern`), since no driver built on top of the emitter could otherwise
report an honest `CapabilityManifest`.

## Decision

**New crate `crates/nirdosha-guard-store-postgres`, always-on workspace
member**, following the established `nirdosha-<domain>-<subject>-<vendor>`
naming and one-crate-per-driver pattern already used by
`nirdosha-lineage-store-embedded`/`-remote` and
`nirdosha-lineage-sink-jsonl`/`-http`.

**Pooling/TLS is duplicated, not imported.** The real pooled/TLS Postgres
pattern already exists in `crates/runtime-kernels/src/kernel/db.rs`
(`docs/adr/0005-postgres-pooling-and-tls.md`), but `runtime-kernels` is a
separate Cargo workspace root by the workspace root `Cargo.toml`'s own
`exclude = ["crates/runtime-kernels", "crates/compiled-serve"]` — a
root-workspace crate path-depending on it would hit Cargo's "multiple
workspace roots" error, the same constraint `isolation-core`'s own comment
in that file already documents for the reverse direction. `PostgresManager`
in the new crate mirrors ADR-0005's design exactly rather than diverging
from it: a hand-rolled `r2d2::ManageConnection` (not `r2d2_postgres`, for
the same explicit-`is_valid`-control reasoning ADR-0005 gave), a real
`SELECT 1` liveness check on every pool checkout, and `SslMode::Require` by
default for any non-local host unless the caller's own connection string
already set `sslmode=` — no `danger_accept_invalid_certs`/`danger_accept_invalid_hostnames`
call anywhere. Dependency versions match what's already pinned in
`crates/compiler/Cargo.toml`/`crates/runtime-kernels/Cargo.toml`
(`postgres = "0.19"`, `postgres-native-tls = "0.5"`, `native-tls = "0.2"`,
`r2d2 = "0.8"`).

**RLS via session variables, no generated DDL per tenant** (the checklist's
own stated preference, `rfcs/0023-data-guard-checklist.md`). One table
(`guard_entities`), RLS enabled and forced, one static policy created
idempotently (`CREATE POLICY` has no `IF NOT EXISTS` form, unlike `CREATE
TABLE` — the idempotent form is a `pg_policies` catalog check inside a `DO`
block). `commit()` issues `SELECT set_config('app.tenant', $1, true)` inside
the same transaction as the write — not `SET LOCAL app.tenant = $1`, which
isn't valid SQL (`SET` doesn't accept bind parameters); `set_config`'s
third argument (`is_local`) is the parameterized equivalent, scoped to the
transaction. Verified against a real Postgres container
(`docker-compose.dev.yml`) with a dedicated non-superuser `guard_probe` role
granted `SELECT` only: querying with the wrong `app.tenant` returns zero
rows, the right one returns the row — proving the policy predicate, not
just its DDL existing (a superuser or table-owner connection would bypass
RLS regardless of `FORCE ROW LEVEL SECURITY`, so the test deliberately
doesn't rely on the default `postgres`-role test connection for this
assertion).

**What this driver models, deliberately not more.** `StoreDriver::prepare`
returns `Prepared { resource, policy_version }` — the trait carries no
row-level structure or dataset field. This driver persists exactly that
shape (one row per `resource`, an opaque payload blob), matching
`MemStoreDriver`'s own semantics, rather than inventing a richer schema the
trait doesn't actually carry.

**Resource-keyed tenant handoff between `prepare` and `commit` — a real,
narrow, documented limitation, not hidden.** `prepare()` extracts the
tenant from the residual `FilterExpr` (a `TenantEq` node, possibly nested
under `And`/`Or`/`Not`) and stashes it in a `Mutex<HashMap<String, String>>`
keyed by `resource`, since `Prepared` has no field for driver-private state
and widening it would ripple into every `StoreDriver` implementor
(`MemStoreDriver`, the e2e test) for the sake of one driver. This is safe
within a single `guarded_apply` call (synchronous, `prepare` immediately
followed by `commit`, no intervening await point) but would misbehave if
the *same* driver instance were shared across threads and two callers raced
on an identical `resource` key with different tenants inside that window.
Not fixed here — narrow enough, and disproportionate to solve by widening a
type every driver shares, for one driver's internal bookkeeping.

**Fail-closed on missing tenant scope.** `prepare()` rejects
(`PlanError::Rejected("missing tenant scope")`) any write plan whose
residual filter contains no `TenantEq`, rather than committing ungoverned.
Consistent with the guard's own deny-by-default posture, and covered by an
opt-in integration test (`prepare_rejects_a_write_plan_with_no_tenant_scope`).

## Consequences

**What this makes possible.** A real `cargo test -p
nirdosha-guard-store-postgres -- --ignored` against
`docker-compose.dev.yml` proves: a guarded write actually lands in
Postgres, RLS actually isolates tenants (not just "a policy object exists"),
and `GuardClient::verify_audit()` still passes against a real driver, not
only `MemStoreDriver`. `rfcs/0026-metadata-plane-checklist.md`'s "production
store drivers" line and `rfcs/0023-data-guard-checklist.md`'s "Versioned
capability manifests emitted by driver crates" / "Postgres RLS via session
variables" lines are now honestly checkable, not aspirational.

**What this does *not* make possible, stated so it's never misread later.**
This is the write path only. Read-path (`AccessPlan`/`FederatedPlan`)
execution against Postgres has no trait/contract anywhere in the codebase —
`RdbmsEmitter`/`DdlAst` compile IR to SQL text but nothing executes it. That
needs its own RFC amendment (a new trait, not an implementation against an
existing one) and is explicitly not solved by this ADR.

Live vendor **capability attestation** — `DriverAttestation::{CanaryRows,
DifferentialTests, QueryPlanInspection}` (`nirdosha-guard-core`) — stays an
unused, unimplemented enum. Nothing today verifies that this driver's
`CapabilityManifest` claims are actually true beyond the unit/integration
tests written alongside it; a future V5-style verify pass would need to
run canary rows or differential tests against the manifest's claims, not
just trust them. `guard-verify`'s existing V5 pass checks something
different (`lineage_support` non-empty) and should not be read as covering
this.

**A quality fix that rode along, not scope creep**: the `RdbmsEmitter`
fix (`Compare`/`TimeRange`/`Pattern` translation, `RelationIn` now panics
loudly instead of silently emitting `"1=1"`) benefits every future driver
built on this emitter, not just this one — found while tracing what a real
driver's manifest could honestly claim, not searched for independently.
