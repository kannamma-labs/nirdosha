//! The compiled-path resource-control kernel — `rfcs/0007-apm-runtime-
//! kernel.md`'s "local admission plane" (§3), built for real for the
//! first time, deliberately scoped to the smallest useful version of
//! that RFC's mechanism rather than the whole five-plane design at
//! once. Two decisions this file makes, both taken directly from that
//! RFC's own findings and from the fresh, independent performance
//! review that fed into it:
//!
//! 1. **Admission happens once per resource-*creation* call
//!    (`connect`/`listen`/`accept`/`open`), never on `send`/`recv`/
//!    `read`/`write`.** RFC 0007 §4.2 already identifies "an accepted
//!    TCP connection or an opened file" as the compiled path's real
//!    admission boundary (there's no classified-request boundary yet —
//!    compiled `serve` doesn't exist); the independent performance
//!    review sharpened this further: checking on every syscall, not
//!    every resource-acquisition event, was the wrong granularity in
//!    the original design. `kernel_bench` (`rfcs/evidence/0007-apm-
//!    runtime-kernel/kernel_bench/`) measured the *hot-path* calls
//!    (`send`/`recv`/`read`/`write`) at 0.8-1.8µs with zero admission
//!    logic — a per-call check there was the one real risk that
//!    review flagged. Gating only at creation avoids it entirely.
//! 2. **The check itself is one atomic compare-and-swap, no lock, no
//!    allocation, no syscall** — the "O(1) local lease check" §3 calls
//!    for. The same review argued the original ~100ns hot-path budget
//!    was the wrong order of magnitude for a true per-call gate (it
//!    should cost single-digit nanoseconds); by only running at
//!    creation calls, which already cost 2.4-27µs per `kernel_bench`'s
//!    own measurements, this doesn't need to hit that bar at all — but
//!    it's a plain atomic op regardless, cheap enough to not need the
//!    exemption.
//!
//! **Telemetry is a real, working data plane now, not just counters.**
//! [`recorder`] double-buffers every `acquire`/`release`/denial event
//! into pages, flushes a full page to disk asynchronously (via
//! [`thread_pool::ThreadPool`], gzip-compressed) without ever blocking
//! `acquire`/`release` themselves, and flushes whatever's left on exit.
//! Still genuinely smaller than RFC 0007 §6's full design: no OTLP
//! export, no sampling policy, one file rather than a real sink/pipeline
//! — but this is a real double-buffered recorder, not a placeholder for
//! one. See [`recorder`]'s own module doc. No cross-shard wait-for
//! sweep (only two domains exist; nothing to deadlock across yet), no
//! NFR declarations — real future work, left for when a real need shows
//! up, per this session's own "add capability through the kernel as we
//! go" plan.
//!
//! [`HandleTable`] exists now, wired to nothing yet, specifically so the
//! next resource domain this project adds (`json`/`db`/`mq` — see the
//! "how many features could we bring back" discussion this was built
//! from) gets a real opaque-handle mechanism from its very first line of
//! code instead of a fourth independently invented one.
//!
//! **Two more primitives live alongside this one, also unwired**:
//! [`pool`] (generic, backend-agnostic connection *pooling* — reuse, not
//! just admission — ported from the interpreter's own real `pool.rs`,
//! which had zero interpreter dependency) and [`thread_pool`] (an
//! eager-growth, reused-worker OS thread pool — ported from the
//! interpreter's own `thread_pool.rs`, same story, with a real
//! panic-containment question this crate's `panic = "abort"` release
//! profile used to raise — resolved: this crate now builds with `panic
//! = "unwind"` instead, verified against the real production link path,
//! not assumed — see `thread_pool`'s own module doc and
//! `rfcs/evidence/0007-apm-runtime-kernel/panic_containment/`).
//! Admission (this module) answers "are we under the ceiling"; pooling
//! answers "reuse what's already open instead of paying to create a new
//! one" — a future `db`/`mq` domain plausibly wants both at the same
//! call site, and a future `spawn` codegen effort wants the worker pool
//! specifically (not connection pooling — threads aren't connections).
//!
//! **Flight recorder, not a query interface.** A compiled `.nir`
//! program never asks the kernel anything — there is deliberately no
//! builtin a `.nir` author can call to read admission stats mid-run.
//! Instead, [`dump_report`] is invoked exactly once, automatically, by
//! `codegen.rs`'s generated `main` wrapper, right before the process
//! exits — every compiled binary silently records what it did and
//! reports it on the way out, the same way a flight recorder doesn't
//! answer queries mid-flight, it just captures and hands over the tape
//! afterward. This keeps the recording passive and uniform (nothing a
//! `.nir` author writes can suppress or distort it) instead of opt-in
//! and queryable.
//!
//! **Two of RFC 0006's five pillars, wired in for real now, hidden
//! behind keywords that already existed — no new syntax, no new
//! developer-facing API.** [`mailbox`] is Pillar 2 (non-blocking `send`)
//! and Pillar 3 (blocking, multi-consumer `receive`) — `chan`/`send`/
//! `recv` compile to `runtime-kernels/src/lib.rs`'s `nir_chan_new`/
//! `nir_chan_send`/`nir_chan_recv`, which is this module's `mailbox`
//! exactly, not a parallel API a `.nir` author opts into.
//! [`thread_pool::Scope`] is Pillar 4 (structured spawn, no orphan
//! threads) — `spawn`/`join` compile to `nir_thread_spawn`/
//! `nir_thread_join`, each spawn getting its own dedicated one-job
//! `Scope`, auto-joined by `codegen.rs`'s `emit_affine_free` if the
//! spawning function itself never consumes the `thread` handle (see
//! `lib.rs`'s "chan/spawn/join kernels" section for the one real,
//! disclosed gap versus the RFC prototype's own lexical-`Scope`-per-
//! function mechanism, and `codegen.rs`'s `is_word_sized` for the other
//! one: word-sized payloads/arguments/results only, so far — no
//! `str`/`dec128`/struct/enum yet).
//!
//! **Pillars 1 and 5 are honestly still open, and nothing in this crate
//! substitutes for them.** Pillar 1 (`iso`/`froze`/`lend` capability
//! types on `Box<T>`) and Pillar 5 (the lexical-level deadlock-freedom
//! proof via scope levels) are type-system properties the compiler must
//! check statically — they need real work in `ownership.rs` and a
//! level-typing extension to scopes, not a runtime primitive. A kernel
//! module can host the mechanics Pillars 2-4 need at runtime; it cannot
//! retroactively make `.nir`'s type checker prove something it doesn't
//! check yet.

// `stats`/`HandleTable` aren't called from anywhere in `lib.rs` yet --
// deliberately: `stats` is for a future telemetry export point, and
// `HandleTable` is for the next resource domain to be added (see this
// module's own doc comment). Both are proven by this file's own unit
// tests, not dead in any real sense; `#[allow(dead_code)]` just says so
// to the compiler until a real caller exists.
#![allow(dead_code)]

pub mod mailbox;
pub mod pool;
pub mod recorder;
pub mod thread_pool;

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// One resource domain the kernel admits/tracks. Deliberately a closed,
/// explicit enum (not a string or an open trait) — the same "notation
/// with nothing to check" argument `ast::Effect`'s own doc comment
/// makes for why *that* set is closed too: a domain with no kernel
/// logic behind it yet would be a variant nothing exercises. Add one
/// here, and its own two entries in `counters_for`/`Domain::env_var`,
/// exactly when a real resource (a `json` handle table, a `db`
/// connection pool) needs it — not speculatively ahead of that.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Domain {
    Tcp,
    File,
    /// One outstanding `thread` handle (between `spawn` and its
    /// matching `join`) — the same ceiling-on-concurrently-held
    /// resources tcp/file already get, now that `spawn`/`join` are real
    /// (`nir_thread_spawn`/`nir_thread_join`, `lib.rs`'s "chan/spawn/
    /// join kernels" section). `chan` deliberately has no domain here:
    /// unlike a thread handle, a channel handle is never released
    /// (`Ty::Channel` has no `stop` case — `nir_chan_new`'s own doc
    /// comment), so a concurrently-*held* ceiling doesn't fit its
    /// lifecycle the way it fits an affine handle's acquire/release
    /// pair; a total-ever-created cap would be a different, not-yet-
    /// asked-for kind of limit.
    Thread,
}

impl Domain {
    /// Read once per domain, lazily, on first `acquire` — an operator
    /// can raise or lower the ceiling per deployment without a
    /// recompile, the same override-by-env-var convention
    /// `crates/compiler/src/pool.rs`'s `PoolConfig` already
    /// establishes for the interpreter's own (now-removed) DB pooling.
    fn env_var(self) -> &'static str {
        match self {
            Domain::Tcp => "NIRDOSHA_KERNEL_MAX_TCP",
            Domain::File => "NIRDOSHA_KERNEL_MAX_FILE",
            Domain::Thread => "NIRDOSHA_KERNEL_MAX_THREAD",
        }
    }

    /// Deliberately generous, not a production-tuned ceiling — this
    /// exists to prove the admission mechanism is real and load-bearing,
    /// not to actually constrain a well-behaved program today. Tighten
    /// per-deployment via the env var above once there's a real reason
    /// to.
    fn default_max(self) -> i64 {
        10_000
    }
}

struct DomainCounters {
    held: AtomicI64,
    grants: AtomicU64,
    denials: AtomicU64,
    max: OnceLock<i64>,
}

impl DomainCounters {
    const fn new() -> Self {
        DomainCounters { held: AtomicI64::new(0), grants: AtomicU64::new(0), denials: AtomicU64::new(0), max: OnceLock::new() }
    }
}

static TCP: DomainCounters = DomainCounters::new();
static FILE: DomainCounters = DomainCounters::new();
static THREAD: DomainCounters = DomainCounters::new();

fn counters_for(domain: Domain) -> &'static DomainCounters {
    match domain {
        Domain::Tcp => &TCP,
        Domain::File => &FILE,
        Domain::Thread => &THREAD,
    }
}

fn max_for(domain: Domain, counters: &DomainCounters) -> i64 {
    *counters.max.get_or_init(|| {
        std::env::var(domain.env_var()).ok().and_then(|s| s.parse::<i64>().ok()).filter(|&n| n > 0).unwrap_or_else(|| domain.default_max())
    })
}

/// Attempts to admit one more concurrently-held resource in `domain`.
/// `true` if granted (the caller must eventually call [`release`]
/// exactly once); `false` if `domain` is already at its ceiling, in
/// which case the caller should fail the same way any other resource-
/// creation error already does (`nir_tcp_connect`/`nir_file_open`
/// return `-1` uniformly today — this is deliberately not yet a
/// distinct error code; that's real future work, not a gap to route
/// around today, see this module's own doc comment).
pub fn acquire(domain: Domain) -> bool {
    let counters = counters_for(domain);
    let max = max_for(domain, counters);
    let mut current = counters.held.load(Ordering::Relaxed);
    loop {
        if current >= max {
            counters.denials.fetch_add(1, Ordering::Relaxed);
            recorder::record(domain, recorder::EventKind::Denial);
            return false;
        }
        match counters.held.compare_exchange_weak(current, current + 1, Ordering::AcqRel, Ordering::Relaxed) {
            Ok(_) => {
                counters.grants.fetch_add(1, Ordering::Relaxed);
                recorder::record(domain, recorder::EventKind::Grant);
                return true;
            }
            Err(observed) => current = observed,
        }
    }
}

/// Releases one previously-[`acquire`]d resource in `domain`. Must be
/// called exactly once per successful `acquire` — `ownership.rs`'s
/// affine typing already guarantees a `tcp`/`file` handle's own `stop`
/// runs at most once per handle in a well-typed program (this crate's
/// own "the checker is the real gate" convention), so a matched
/// acquire/release pair per handle is a real invariant, not a hope.
pub fn release(domain: Domain) {
    counters_for(domain).held.fetch_sub(1, Ordering::AcqRel);
    recorder::record(domain, recorder::EventKind::Release);
}

/// Raw self-metrics — RFC 0007 §7's "kernel self-metrics are first-class"
/// principle, in its smallest form: no exporter, no aggregation, just
/// the numbers. `(currently_held, total_grants, total_denials)`.
pub fn stats(domain: Domain) -> (i64, u64, u64) {
    let c = counters_for(domain);
    (c.held.load(Ordering::Relaxed), c.grants.load(Ordering::Relaxed), c.denials.load(Ordering::Relaxed))
}

/// The flight recorder's one output: every domain's final counters,
/// formatted as one line each. Called exactly once, automatically, from
/// `codegen.rs`'s generated `main` wrapper (via `nir_kernel_flight_
/// recorder_dump` in `lib.rs`) — never from anything a `.nir` author
/// writes (this module's own doc comment). Deliberately plain text, not
/// JSON — this is the smallest possible version of "the tape gets
/// handed over," not a structured export pipeline (RFC 0007 §6's own
/// rings/aggregator/exporter design is the real future version of this,
/// if a real need for one ever shows up).
///
/// `held` should always print `0` for every domain by the time this
/// runs, in a well-typed program — `ownership.rs`'s affine checking
/// already proves every `tcp`/`file` handle's `stop` runs before
/// `main` returns (nothing currently open at exit is holdable at all,
/// since a live handle would need a live binding, and every binding's
/// scope has already ended). A nonzero `held` here would mean this
/// kernel's own acquire/release bookkeeping has drifted from what
/// `ownership.rs` guarantees — worth treating as a real bug report if
/// it's ever seen, not just a diagnostic curiosity.
pub fn dump_report() -> String {
    let mut out = String::from("nirdosha kernel flight recorder:\n");
    for (name, domain) in [("tcp", Domain::Tcp), ("file", Domain::File), ("thread", Domain::Thread)] {
        let (held, grants, denials) = stats(domain);
        out.push_str(&format!("  {name}: held={held} grants={grants} denials={denials}\n"));
    }
    out
}

/// A generic, process-wide table mapping an opaque, mint-once `i64`
/// handle to a live Rust value `T` — for the next resource domain this
/// project adds whose handle isn't already a raw OS fd (`json`'s parsed
/// document, a `db` connection, an `mq` subscription). Same shape the
/// now-removed `nirdosha-plugin-support::HandleRegistry` used, minus
/// its interpreter-specific error-construction helpers (`Value`/
/// `RuntimeError` don't exist on this side of the ABI boundary — this
/// crate can't depend on the compiler crate at all, this file's own
/// module doc). Not wired to any `nir_*` kernel yet; exists now so the
/// next one that needs it doesn't invent its own table from scratch.
pub struct HandleTable<T> {
    next_id: AtomicI64,
    handles: Mutex<HashMap<i64, T>>,
}

impl<T> Default for HandleTable<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> HandleTable<T> {
    pub fn new() -> Self {
        HandleTable { next_id: AtomicI64::new(1), handles: Mutex::new(HashMap::new()) }
    }

    /// Takes ownership of `value`, mints a fresh id (never `0` —
    /// reserve that as a caller-chosen "no handle"/invalid sentinel),
    /// and returns it as the plain `i64` a `nir_*_open`-style kernel
    /// hands back across the ABI boundary.
    pub fn insert(&self, value: T) -> i64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.handles.lock().unwrap().insert(id, value);
        id
    }

    /// Runs `f` against the live resource for `id` without removing it.
    /// `None` if `id` isn't currently open (already closed, or never
    /// existed) — the caller turns that into `-1`, the same uniform
    /// failure convention every other kernel here already uses.
    pub fn with<R>(&self, id: i64, f: impl FnOnce(&mut T) -> R) -> Option<R> {
        self.handles.lock().unwrap().get_mut(&id).map(f)
    }

    /// Removes and returns the resource for `id` — what a `_stop`
    /// kernel calls; the returned `T` is dropped at the call site.
    /// `None` on a double-close, not a panic.
    pub fn remove(&self, id: i64) -> Option<T> {
        self.handles.lock().unwrap().remove(&id)
    }

    pub fn len(&self) -> usize {
        self.handles.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A cheap, **exact** dynamic backstop for `rfcs/0006-structured-
/// concurrency.md`'s still-unbuilt, still-deferred Pillar 5 (the
/// compile-time, lexical-scope-level deadlock-freedom proof) — the same
/// relationship this whole module has to admission control in general:
/// a runtime mechanism standing in for a static guarantee the type
/// checker doesn't make yet, honestly scoped rather than oversold.
///
/// **The technique**: the same one Go's own runtime uses ("fatal error:
/// all goroutines are asleep - deadlock!"), not a general wait-for-graph
/// cycle detector. Track how many `.nir`-level concurrent participants
/// currently exist (`live` — the main thread, plus one per outstanding
/// `spawn` job) against how many are, right now, blocked in exactly the
/// two operations that can only ever be unblocked by *another* one of
/// those participants (`join`, `recv` — never `tcp`/`file` I/O, which
/// can always still resolve from outside the process). If every live
/// participant is simultaneously in that state, nothing left in the
/// process could ever run the `send`/return that would unblock any of
/// them — a certain, permanent stall, not a heuristic guess.
///
/// **Why this is exact, not probabilistic**, unlike a timeout-based
/// guess: `live`/`blocked` only ever change while holding `STALL`'s own
/// lock, so a thread cannot be "about to do something that would
/// unblock everyone" without that fact already being reflected in
/// `live` before the check runs (a not-yet-submitted `spawn` hasn't
/// incremented `live` yet, so it correctly doesn't count as a possible
/// rescuer; once submitted, it does, before its job could possibly
/// reach a `join`/`recv` of its own).
///
/// **What this deliberately doesn't catch**: a *local* deadlock — two
/// threads cyclically stuck on each other while a third, unrelated
/// thread keeps making progress on something else. Pillar 5's own real
/// proof-by-construction (once built) would catch that too, ahead of
/// time; this only fires once the *whole* program can never move again,
/// same as Go's detector, and for the same reason (correctly telling
/// "some of it is still running" from "literally none of it can ever
/// run again" needs exactly the global count this uses, not a partial
/// one). See `rfcs/0007-apm-runtime-kernel.md` §8 for why this and
/// Pillar 5 are genuinely separate deadlock classes to begin with
/// (resource-acquisition cycles vs. reply-obligation cycles) — this
/// mechanism is a backstop for the *reply-obligation* class only.
struct StallTracker {
    live: usize,
    blocked: usize,
}

static STALL: Mutex<StallTracker> = Mutex::new(StallTracker { live: 1, blocked: 0 });

/// What one currently-blocked thread is actually waiting on — purely
/// diagnostic (`concurrency_wait_begin`'s abort message), never
/// consulted by the deadlock decision itself (`register_wait_and_check_
/// stall` only ever looks at `STALL`'s plain counts). Naming the real
/// stuck handle/call kind is the difference between "all 2 threads are
/// blocked" and "thread A is blocked in `recv` on channel 3, thread B
/// is blocked in `join` on thread 5" — the latter is what actually lets
/// a `.nir` author find the two call sites that made a cycle, without
/// this crate attempting anything like real prevention (this section's
/// own doc comment on why a general wait-for graph doesn't fit `chan`'s
/// no-single-owner semantics).
#[derive(Clone, Copy, Debug)]
pub enum WaitTarget {
    ChanRecv(i64),
    ThreadJoin(i64),
}

impl std::fmt::Display for WaitTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WaitTarget::ChanRecv(handle) => write!(f, "`recv` on chan handle {handle}"),
            WaitTarget::ThreadJoin(handle) => write!(f, "`join` on thread handle {handle}"),
        }
    }
}

/// A separate lock from `STALL`, deliberately — this map is read only
/// for the abort message itself (after `STALL` has already decided a
/// deadlock is real), never as part of the decision, so it doesn't need
/// to share `STALL`'s own ordering guarantees. A brief window where this
/// map and `STALL.blocked` disagree by one entry (registered just before
/// vs. just after the count) only affects the diagnostic's completeness,
/// never whether a deadlock is correctly detected.
fn waiting_registry() -> &'static Mutex<HashMap<std::thread::ThreadId, WaitTarget>> {
    static REGISTRY: OnceLock<Mutex<HashMap<std::thread::ThreadId, WaitTarget>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Call exactly once, synchronously, at the point a `spawn` job is
/// actually handed off to run (`nir_thread_spawn`, after a successful
/// submit — never on a failed one, which never runs at all) — this
/// job is now a real concurrent participant that could, in principle,
/// be the one to unblock someone else.
pub fn concurrency_thread_started() {
    STALL.lock().unwrap().live += 1;
}

/// Call exactly once, synchronously, once `nir_thread_join` has
/// confirmed (via `thread_pool::Scope::already_done`/`join`) that a
/// spawned job's own code has fully finished running — it can no
/// longer unblock anyone.
///
/// **Deliberately not called from inside the spawned job itself, at the
/// instant its own code returns** — that would create a real, if
/// narrow, race: this counter and `thread_pool::Scope`'s own completion
/// state are two separate locks with no ordering relationship between
/// them, so a joiner's non-blocking "is it done yet" check could
/// observe "not yet" a few instructions after this counter had already
/// dropped, undercounting `live` for a job that (from the joiner's own
/// perspective) hasn't finished. Decrementing only once `join` itself
/// has *confirmed* completion ties this counter to the one fact a
/// caller can already prove, closing that gap — at the cost of a
/// finished-but-not-yet-joined job still counting as `live` (a real,
/// narrower detection gap, not a correctness one: it can only make this
/// detector miss a deadlock it otherwise would have caught, never
/// report one that isn't real).
pub fn concurrency_thread_finished() {
    STALL.lock().unwrap().live -= 1;
}

/// The actual decision, factored out from the real `nir_*` entry points
/// (`concurrency_wait_begin`) so it's unit-testable without triggering
/// a real `process::abort()` inside the test binary itself. `true`
/// means this call is the one that made every live participant blocked
/// at once — a certain deadlock, already registered (the caller is
/// still counted as blocked either way; there is no valid "un-register"
/// for a call that's about to abort the process).
fn register_wait_and_check_stall() -> (bool, usize) {
    let mut s = STALL.lock().unwrap();
    s.blocked += 1;
    (s.blocked >= s.live, s.live)
}

/// Call immediately before the real blocking wait inside `nir_thread_
/// join`/`nir_chan_recv`, naming what this specific call is about to
/// wait on (purely for the abort message — see `waiting_registry`'s own
/// doc comment) — see this section's own doc comment for the exact
/// detection guarantee. Aborts immediately on a detected deadlock
/// rather than letting the calling thread actually block forever: the
/// same "trap now, with a real diagnostic, rather than hang silently"
/// contract every other unrecoverable condition in this backend already
/// has (division-by-zero, out-of-bounds, narrow-type overflow — see
/// `codegen.rs`'s `guard_io_ok`/`guard_recv_ok`), extended here to a
/// failure kind only this runtime kernel, not generated LLVM IR, can
/// actually observe.
pub fn concurrency_wait_begin(target: WaitTarget) {
    waiting_registry().lock().unwrap().insert(std::thread::current().id(), target);
    let (deadlocked, live) = register_wait_and_check_stall();
    if deadlocked {
        let mut lines: Vec<String> =
            waiting_registry().lock().unwrap().iter().map(|(tid, target)| format!("  {tid:?} is blocked in {target}")).collect();
        lines.sort();
        eprintln!(
            "nirdosha: deadlock detected -- all {live} concurrently-running thread(s) are \
             blocked, with nothing left in the process that could ever unblock any of them:\n{}",
            lines.join("\n")
        );
        recorder::flush_remaining();
        std::process::abort();
    }
}

/// Call immediately after a `join`/`recv` call actually completes
/// (successfully or not — see `concurrency_wait_begin`'s doc comment,
/// there is no "unblock" path once a deadlock has already been
/// reported, since the process has already aborted by then).
pub fn concurrency_wait_end() {
    waiting_registry().lock().unwrap().remove(&std::thread::current().id());
    STALL.lock().unwrap().blocked -= 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    // Each test picks a domain the others don't touch, so they can run
    // concurrently (`cargo test`'s default) without one test's grants
    // skewing another's -- these are real `static` counters, process-
    // wide, not reset between tests.

    // `STALL`, unlike the per-domain counters above, has no such "each
    // test picks its own" escape hatch -- there's only one of it, for
    // the same reason there's only one real deadlock question for a
    // whole process. The two tests that touch it share this lock so
    // `cargo test`'s default parallelism can't interleave their
    // increments/decrements into a spurious pass or failure.
    static STALL_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn acquire_then_release_returns_to_zero_held() {
        let (held_before, _, _) = stats(Domain::File);
        assert!(acquire(Domain::File));
        let (held_after_acquire, _, _) = stats(Domain::File);
        assert_eq!(held_after_acquire, held_before + 1);
        release(Domain::File);
        let (held_after_release, _, _) = stats(Domain::File);
        assert_eq!(held_after_release, held_before);
    }

    #[test]
    fn denial_past_the_ceiling_does_not_increment_held() {
        // SAFETY: test-only env mutation, single-threaded within this
        // process's env (std::env::set_var is process-global) --
        // chosen a domain (Tcp) no other test in this file touches, and
        // this must run before anything else calls `acquire(Tcp)` for
        // the first time, since the ceiling is resolved once and cached
        // (`max_for`'s `OnceLock`). This is the only test touching Tcp.
        unsafe { std::env::set_var("NIRDOSHA_KERNEL_MAX_TCP", "2") };
        assert!(acquire(Domain::Tcp));
        assert!(acquire(Domain::Tcp));
        let (_, _, denials_before) = stats(Domain::Tcp);
        assert!(!acquire(Domain::Tcp), "third acquire must be denied at a ceiling of 2");
        let (held, _, denials_after) = stats(Domain::Tcp);
        assert_eq!(held, 2, "a denied acquire must not increment held");
        assert_eq!(denials_after, denials_before + 1);
        release(Domain::Tcp);
        release(Domain::Tcp);
    }

    #[test]
    fn handle_table_mint_use_remove_is_a_clean_lifecycle() {
        let table: HandleTable<String> = HandleTable::new();
        let id = table.insert("hello".to_string());
        assert_ne!(id, 0, "0 is reserved as a caller-chosen invalid sentinel");
        assert_eq!(table.with(id, |v| v.clone()), Some("hello".to_string()));
        assert_eq!(table.remove(id), Some("hello".to_string()));
        assert_eq!(table.remove(id), None, "a double-remove must be a clean None, not a panic");
        assert_eq!(table.with(id, |v| v.clone()), None, "use-after-remove must be visible as None too");
    }

    #[test]
    fn handle_table_ids_are_unique() {
        let table: HandleTable<i32> = HandleTable::new();
        let a = table.insert(1);
        let b = table.insert(2);
        assert_ne!(a, b);
        assert_eq!(table.len(), 2);
        table.remove(a);
        assert_eq!(table.len(), 1);
    }

    // `STALL` is one process-wide static, and nothing else in this test
    // binary touches it (only `lib.rs`'s `nir_thread_*`/`nir_chan_recv`
    // do, exercised by the compiler crate's own integration tests, not
    // here) -- still balanced back to its starting point explicitly,
    // the same discipline `recorder.rs`'s own shared-state tests use,
    // so this test's own effect on global state is self-contained.
    // Deliberately only ever calls the pure `register_wait_and_check_
    // stall` -- never `concurrency_wait_begin`, which calls
    // `std::process::abort()` on a real detection and would kill this
    // whole test binary, not just fail one test.
    #[test]
    fn every_live_participant_blocked_at_once_is_detected_exactly_once() {
        let _guard = STALL_TEST_LOCK.lock().unwrap();
        // Two more participants join (three "live" total, including the
        // baseline "main thread" `STALL` starts with).
        concurrency_thread_started();
        concurrency_thread_started();

        // Two of the three block -- not everyone yet, so no detection.
        let (deadlocked_1, live_1) = register_wait_and_check_stall();
        let (deadlocked_2, live_2) = register_wait_and_check_stall();
        assert!(!deadlocked_1, "only 1 of {live_1} live participants is blocked so far");
        assert!(!deadlocked_2, "only 2 of {live_2} live participants is blocked so far");

        // The third (and last) blocks too -- now every live participant
        // is blocked at once, with nothing left that could ever run the
        // send/return that would unblock any of them.
        let (deadlocked_3, live_3) = register_wait_and_check_stall();
        assert!(deadlocked_3, "all {live_3} live participants are now blocked -- this must be reported as a real deadlock");

        // Restore the exact starting state -- three waits registered,
        // three ended; two participants started, two finished.
        concurrency_wait_end();
        concurrency_wait_end();
        concurrency_wait_end();
        concurrency_thread_finished();
        concurrency_thread_finished();

        let s = STALL.lock().unwrap();
        assert_eq!((s.live, s.blocked), (1, 0), "must return to the exact baseline once every registered wait/spawn is matched");
    }

    // A thread that's still running real work (not blocked at all) is
    // never mistaken for a rescuer that's "about to" unblock someone --
    // `live` only ever counts participants that *exist*, `blocked` only
    // ever counts ones that are actually stuck; a live-but-unblocked
    // participant keeps `blocked < live` correctly, with no detection.
    #[test]
    fn a_still_running_participant_prevents_a_false_positive() {
        let _guard = STALL_TEST_LOCK.lock().unwrap();
        concurrency_thread_started(); // two live: main + this one
        let (deadlocked, _) = register_wait_and_check_stall(); // only main blocks
        assert!(!deadlocked, "the second participant is still running, not blocked -- must not be reported as a deadlock");
        concurrency_wait_end();
        concurrency_thread_finished();

        let s = STALL.lock().unwrap();
        assert_eq!((s.live, s.blocked), (1, 0));
    }
}
