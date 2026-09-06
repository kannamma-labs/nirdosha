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

fn counters_for(domain: Domain) -> &'static DomainCounters {
    match domain {
        Domain::Tcp => &TCP,
        Domain::File => &FILE,
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
    for (name, domain) in [("tcp", Domain::Tcp), ("file", Domain::File)] {
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

#[cfg(test)]
mod tests {
    use super::*;

    // Each test picks a domain the others don't touch, so they can run
    // concurrently (`cargo test`'s default) without one test's grants
    // skewing another's -- these are real `static` counters, process-
    // wide, not reset between tests.

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
}
