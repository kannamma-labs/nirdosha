# 0005: Postgres `db_connect` — pooling design and TLS-by-default

Date: 2026-09-07
Status: accepted

## Context

`db_connect`/`db_query`/`db_execute` already compiled for real for
SQLite (`docs/ROADMAP.md` B2), but connection-per-call — no pooling —
and with no Postgres backend at all. Adding Postgres needed two real
decisions `docs/ROADMAP.md`'s own B2 note had already flagged as open:
how a compiled binary using Postgres links its TLS dependency (`postgres`/
`postgres-native-tls` aren't statically bundled the way `rusqlite`'s
SQLite is), and whether/how pooling gets designed in from the start
rather than retrofitted once real connection-reuse/staleness problems
show up in production.

## Decision

**Pooling**: `crates/runtime-kernels/src/kernel/db.rs` routes both
backends through the existing, already-built `kernel::pool::PoolRegistry<M>`
(one registry per backend manager type), keyed by the exact connection
string. `:memory:` is the one deliberate exception — never pooled, since
a `:memory:` SQLite database is private to the connection that created
it; pooling it would silently hand two unrelated callers either the
same database or two different empty ones. Both managers
(`SqliteManager`, `PostgresManager`) implement `r2d2::ManageConnection::is_valid`
as a real `SELECT 1` round-trip, never an `is_closed()`-only check —
after a server-side restart, a client socket typically has no idea the
peer is gone until it actually tries to use the connection, and
`is_closed()` alone would false-negative straight through that window.
r2d2 itself (confirmed by reading its source, not assumed) already
calls `is_valid` exactly once, on every checkout of an idle connection,
and transparently drops-and-retries on `Err` before `Pool::get` ever
returns — the actual "APM rehydrates a stale connection before the
caller sees it" mechanism is r2d2's own, already-proven design; this
phase's job was making `is_valid` real. A new `DomainCounters::stale_rehydrated`
field (`kernel/mod.rs`), incremented from inside `is_valid`'s error
path — the one and only call site for that method — makes this
telemetry exact, not approximate, in the flight-recorder dump.

**TLS**: verify-by-default for any connection string whose host isn't
`localhost`/`127.0.0.1`/`::1`/a Unix socket, and only when the caller
hasn't already chosen an `sslmode` themselves (an explicit
`sslmode=disable` in a caller's own connection string is honored, never
silently overridden). Implemented via `postgres::Config::ssl_mode(SslMode::Require)`
plus a `native_tls::TlsConnector` (via `postgres-native-tls`) with no
`danger_accept_invalid_certs`/`danger_accept_invalid_hostnames` call
anywhere in this crate — real certificate and hostname verification
stays on. `native-tls`/vendored `openssl` were already dependencies
here (the `http`/`https` kernels' own decision) — Postgres TLS reuses
that same TLS stack rather than introducing a second one.

## Consequences

Real Postgres support, real pooling, real rehydration telemetry — a
`db_connect("postgres://...")` in a compiled binary now behaves like a
production-grade connection, not a bare "open a socket and hope."

**Honest costs, not hidden**: the synchronous `postgres` crate pulls in
`tokio` transitively (it wraps `tokio-postgres` internally, running a
private runtime under the hood) — a real departure from this crate's
otherwise-consistent "no async runtime" posture elsewhere
(`kernel::pool`/`kernel::mailbox`'s own doc comments), though every
`nir_db_*` kernel stays a plain synchronous `extern "C"` function; no
caller of this module needs to know or care. `pool.max_size` must stay
`≥` `Domain::Db`'s own admission ceiling (enforced by
`db_pool_config()` clamping upward, not by convention) — admission
itself never blocks (`kernel::acquire` is a lock-free CAS check), but
`Pool::get()` does, so a smaller pool would silently become the real,
invisible bottleneck underneath a healthy-looking admission dashboard.

**Not yet verified**: this ADR's Windows path is assumed to go through
`native-tls`'s SChannel backend the same way the `http`/`https` kernels'
own TLS decision does, with no OpenSSL involved on that platform — this
has not yet been confirmed against a real `build-windows` CI run for
the Postgres path specifically. Real, opt-in-only, `NIRDOSHA_TEST_POSTGRES_URL`-gated
test coverage exists (`crates/runtime-kernels/src/lib.rs`'s
`db_kernel_tests`, `crates/runtime-kernels/src/kernel/db.rs`'s own
`tests` module) — run locally against `docker-compose.dev.yml`'s real
Postgres, never required by CI.
