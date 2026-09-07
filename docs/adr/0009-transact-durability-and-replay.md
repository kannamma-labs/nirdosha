# 0009: `transact` durability log and crash replay

Date: 2026-09-08
Status: accepted

## Context

`transact { precheck?/network/verify/commit/compensate?/log? }` already
compiled and ran for real (Layer 1: `codegen::emit_transact`) — real
control flow, no durability. `docs/TRANSACT.md`'s own design calls for
more: a durable log surviving a crash, bounded retry-with-backoff on
`commit`/`compensate` when their return type supports it, and replay at
process restart so a crash between `commit` succeeding and the log
recording that fact doesn't silently lose the transaction's own
bookkeeping.

Two things this phase discovered while implementing it, not assumed
going in:

- **Retry-on-trap (the original interpreter-era design) is
  architecturally blocked by the compiled trap model** — every trap in
  this backend is a hard `@abort()`, with no unwind/catch mechanism a
  generated retry loop could wrap around. Real trap-recovery would need
  per-attempt OS-process isolation (`sandbox`, ROADMAP's B6, itself
  still `[OPEN, DESCOPED FROM v1]`), not a compiler-only fix. This phase
  narrows retry to **retry-on-`Result::Err`**: `commit`/`compensate`
  only get bounded retry when their declared return type is
  `Result(_, _)` (already unconstrained per `typeck.rs`); any other
  return type runs once, exactly as Layer 1 already did.
- **Args reconstruction for replay only ever needs to cover the
  `commit_pending`/`compensate_pending` states** — this phase's replay
  never resumes mid-`network`/`verify` (a crash there just means the
  whole `transact` never happened; nothing was durably promised yet).
  So replay doesn't need the fuller `network`/`txn_id`/`opaque`
  per-argument symbolic classification `docs/TRANSACT.md`'s original,
  more general design described — it durably stores `commit`'s or
  `compensate`'s own already-computed argument *values* (the exact
  `NirBindValue` array `emit_db_binds` already builds for `db_execute`,
  reused verbatim) at the moment each is marked pending. Simpler, and
  sufficient for what this phase actually replays.

## Decision

**A private durability-log connection, not `kernel::db`'s pool.**
`kernel::transact`'s own `rusqlite::Connection` (`journal_mode=WAL`,
`synchronous=FULL` — a real fsync per write, the actual point of the
log), guarded by the log's own mutex, opened once at process start
(`nir_transact_log_init`, called from generated `main`'s prologue) and
held for the process's whole lifetime. Its path is
`NIRDOSHA_TRANSACT_LOG_PATH`, defaulting to a cwd-relative
`nirdosha_transact_log.sqlite` — configurable because cwd is ephemeral
in a container, and because two independent programs (or two test
binaries) sharing a cwd must not silently share a log.

**`kernel::instance_lock`, ported from dead code, wired for real.**
`crates/compiler/src/instance_lock.rs` was written for the deleted
interpreter's `nirdosha serve` and never had a real caller — dead code
in a crate that isn't even linked into compiled output. Moved to
`crates/runtime-kernels/src/kernel/instance_lock.rs` (same mechanism:
`PRAGMA locking_mode=EXCLUSIVE` on a `<log path>.lock` sidecar file, no
`busy_timeout` — a startup lock conflict fails immediately and loudly).
`nir_transact_log_init` is its first real caller; `emit_c_main` aborts
the whole program if log init fails (fail fast, matching this file's
existing trap philosophy) rather than running live `transact` calls with
no durability guarantee behind them.

**Bounded retry-with-backoff is one shared codegen helper
(`Codegen::emit_call_with_retry`), used by both the live path and
replay.** Fixed compile-time budget
(`TRANSACT_RETRY_MAX_ATTEMPTS = 3`, doubling backoff from
`TRANSACT_RETRY_BASE_BACKOFF_MS = 100` via `nir_sleep_ms`) — not
configurable, because the safety property doesn't depend on how many
live attempts happen before a row is left `*_pending` for replay; retry
here is a latency optimization for the common transient-failure case,
not the durability mechanism itself. A non-`Result` return calls once,
unconditionally "successful" (matching Layer 1's original, unconstrained
discard).

**The live `commit`/`compensate` path**: `nir_transact_begin` writes a
`pending` row (only reached *after* `precheck`, so a precheck-rejected
`transact` never gets a durable row at all — nothing for replay to ever
act on). Right before attempting `commit` (or `compensate`), its
already-marshaled args are durably written via
`nir_transact_mark_commit_pending`/`mark_compensate_pending` — durable
*before* the first live attempt, so a crash at any point from here
through the retry loop leaves enough for replay to finish the job. On a
retry-loop success, `nir_transact_mark_committed`/`mark_compensated`
marks the row terminal; on exhaustion, the row is left `*_pending` on
purpose, recoverable by the next process's own `nir_transact_replay_all`
rather than lost. **The `transact` expression's own live `bool` result
is unchanged by any of this** — it still reports which branch `verify`
chose, exactly as Layer 1 already did, not whether `commit` is yet
confirmed durable.

**Crash replay is compiler-synthesized code, not interpreter dispatch.**
One replay trampoline is generated per `transact` call site
(`Codegen::emit_transact_replay_trampoline`, the same
`self.out`/`self.trampolines`-swap mechanism `spawn`'s own per-call-site
trampolines already use), matching `kernel::transact::ReplayFn`'s real
ABI: `extern "C" fn(*const u8, i64, i32) -> i32`. It decodes its row's
JSON-encoded args (`nir_transact_decode_args`) into one shared,
worst-case-sized `NirBindValue` buffer, dispatches to `commit` or
`compensate` per its `is_commit` flag, through the exact same
`emit_call_with_retry` logic the live path uses, and reports success/
failure — `nir_transact_replay_all` (kernel side) is what marks the row
`committed`/`compensated` on success, or leaves it pending (logged, not
silently dropped) on failure.

**Registration happens from generated `main`'s own prologue, not
`@llvm.global_ctors`.** `emit_c_main` loops over `Codegen::transact_sites`
(populated once per `emit_transact` call, in program codegen order) and
calls `nir_transact_register_replay_site` for each, strictly before its
own call to `nir_transact_replay_all` — simpler than a global-constructor
table, and sufficient today since `main` is the only entry point that
exists (compiled `serve`, ROADMAP B8, is still ahead of this phase).

**Site dispatch is by a bare `site_id` (position in program order) —
no build-version fingerprint guard, disclosed rather than hidden.** A
rolling deploy that changes the *set* of `transact` sites between the
process that wrote a pending row and the process that replays it could
dispatch to the wrong site. The fingerprint hardening
`docs/TRANSACT.md`'s original design called for (hash the compile-time
`site_id → (callee, arg-shape)` table, quarantine a row whose stored
fingerprint doesn't match) is real, separate follow-up work — named
here, not built in this phase. An unregistered `site_id` at replay time
(the one case this phase does handle) is marked `stuck`, never guessed
at.

## Consequences

Verified end to end against real compiled binaries
(`crates/compiler/tests/codegen.rs`):
`transact_commit_retries_with_backoff_and_leaves_the_row_pending_on_exhaustion`
proves the live retry loop makes exactly `TRANSACT_RETRY_MAX_ATTEMPTS`
real calls and leaves the row `commit_pending` on exhaustion (a real
SQLite side-table tracks the actual attempt count, not a mock);
`transact_replay_finishes_a_commit_pending_row_left_by_a_prior_process`
runs the *same compiled binary twice as two separate OS processes*
against the same durability log and the same attempt counter — the
second process's own `main` prologue replays the first process's
still-pending row before its own `main` body ever runs, and the row's
own retry budget (independent of the first process's exhausted one)
succeeds on replay's second attempt. The existing
`transact_commits_and_compensates_for_real_matching_the_checked_in_example`/
`transact_precheck_false_skips_every_other_slot`/
`transact_verify_false_with_no_compensate_slot_yields_false` all still
pass unmodified in their program-level behavior, updated only to give
each test its own `NIRDOSHA_TRANSACT_LOG_PATH` (a real collision,
caught by running this suite locally: two such tests sharing the
default log path under `cargo test`'s ordinary parallelism made the
second one's `nir_transact_log_init` fail `instance_lock::acquire` and
hard-abort).

**A local-SQLite log is unsafe under this repo's own
`deploy/kustomize/overlays/postgres-multi-replica/`** — two replicas
sharing one log file over a volume corrupts it (`instance_lock` only
protects one process on one file, not a fleet), and a pod reschedule
without a persistent volume silently loses pending rows. A
Postgres-backed durability log for fleet-wide use is real, separate
follow-up work, not part of this phase — do not point that overlay at a
binary compiled with a live `transact` block until it lands.

**Bind-after-replay ordering, not yet load-bearing.** `emit_c_main`
calls `nir_transact_replay_all()` unconditionally before `nir_main()`
runs — correct today because nothing serves a request before `main`'s
own body does. Compiled `serve` (ROADMAP B8, still ahead of this phase)
will need to bind its listening socket before replay runs but withhold
`accept()` until replay finishes, per the plan's own k8s-probe reasoning
— a real, disclosed follow-up for that phase, not solved here.
