//! RFC 0011 §5's reaper — proactive rehydration realized as one job,
//! submitted once, whose body is its own infinite loop:
//! `loop { sweep_all_pools(); thread::sleep(interval) }`, not a second
//! threading primitive alongside [`super::thread_pool::ThreadPool`].
//! Reactive rehydration (`ManageConnection::is_valid` on checkout,
//! already real for every registered pool — `db.rs`/`http.rs`/
//! `pool.rs`'s own `PluginManagedConnection`) already guarantees no
//! caller ever sees a stale connection; this module exists only to
//! shrink the *window* a stale idle connection sits unnoticed, per
//! [[nirdosha_pool_rehydration_requirement]] — never something this
//! RFC's own correctness claim depends on.
//!
//! Runs on its own dedicated [`super::thread_pool::ThreadPool`]
//! singleton (not the one `lib.rs`'s `global_thread_pool` uses for
//! user `spawn`/`join`) — RFC §5 only asks for "using
//! `kernel::thread_pool::ThreadPool` as it actually exists," not that
//! it share the same instance as user-code threads; keeping it
//! dedicated avoids any lifecycle coupling with user thread-count
//! accounting (`domain::thread()`'s own ceiling) for a job that isn't a
//! user resource at all. From `ThreadPool`'s own perspective this
//! permanently occupies one worker for the life of the process — a
//! disclosed, deliberate cost of exactly one OS thread, not a new
//! capability requirement on `thread_pool.rs` itself (that module's own
//! doc comment already states it has none).

use super::pool;
use super::thread_pool::ThreadPool;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Once, OnceLock};
use std::time::Duration;

static REAPER_PANICS: AtomicU64 = AtomicU64::new(0);

/// RFC §5: "a broken reaper is a visible number going up, not a silent
/// absence" — surfaced through [`super::dump_report`].
pub fn reaper_panics() -> u64 {
    REAPER_PANICS.load(Ordering::Relaxed)
}

/// `0` until [`start`] actually resolves and records a real interval —
/// [`super::dump_report`]'s own caller treats `0` as "the reaper hasn't
/// started in this process," never as a resolved zero-second interval
/// (`resolve_interval_secs`'s own floor of 1 makes a *resolved* `0`
/// structurally impossible).
static REAPER_CONFIGURED_INTERVAL_SECS: AtomicU64 = AtomicU64::new(0);

/// Red-team report A9 (`scratch/red-team-report-main-d7fae42.md`):
/// `resolve_interval_secs`'s floor/soft-ceiling downgrades used to be
/// visible only via a stderr `eprintln!`, easy to lose in a systemd
/// journal — an operator querying `dump_report` post-incident had no way
/// to see the reaper was misconfigured. This is the actual, currently
/// in-effect interval (post floor/ceiling resolution), not the raw env
/// var value, so a `0`-vs-floored-`1` or an accepted-above-ceiling value
/// is visible without needing to go find the startup log line.
pub fn reaper_configured_interval_secs() -> u64 {
    REAPER_CONFIGURED_INTERVAL_SECS.load(Ordering::Relaxed)
}

/// Checked once per wake, never joined against today — `ThreadPool` (as
/// read, this module's own doc comment) has no shutdown/join API, so a
/// real compiled Nirdosha binary already has no graceful-shutdown story
/// that would need to signal this; this is forward-compatible plumbing
/// for a future RFC 0006 structured-concurrency shutdown mechanism, not
/// something exercised by any shutdown path today. A bare
/// `thread::sleep(interval)` is uninterruptible for the whole interval,
/// so even a signaled shutdown could take up to `interval` to actually
/// notice — accepted for now, stated rather than hidden (RFC §5's own
/// text).
static SHUTDOWN: AtomicBool = AtomicBool::new(false);

/// Signals the reaper loop to exit at its next wake (see [`SHUTDOWN`]'s
/// own doc comment for what this does and does not guarantee today).
pub fn request_shutdown() {
    SHUTDOWN.store(true, Ordering::Relaxed);
}

/// RFC §5's fairness note: "starting each wake's sweep at a rotating
/// offset into the pool list... fixes [uneven coverage] for free."
static SWEEP_OFFSET: AtomicUsize = AtomicUsize::new(0);

fn reaper_pool() -> &'static Arc<ThreadPool> {
    static POOL: OnceLock<Arc<ThreadPool>> = OnceLock::new();
    POOL.get_or_init(ThreadPool::new)
}

/// `NIRDOSHA_KERNEL_REAPER_INTERVAL_SECS` — parsed exactly the way
/// [`super::pool::PoolConfig::from_env`] already resolves its own env
/// vars (a malformed value degrades to the default field-by-field,
/// never fails the whole program), extended with RFC §5's own floor/
/// ceiling: parse failure or unset → default (30s, silently, same
/// convention as `PoolConfig::from_env`); `0` or negative → the RFC's
/// own explicit "never accepted silently" case, clamped up to the 1s
/// floor with a warning (an accepted-but-unclamped `0` would be
/// `sleep(Duration::ZERO)`, a busy loop pinning one core at 100%
/// forever); above the 3600s (1h) soft ceiling → accepted as configured
/// but warned, since an operator setting an enormous interval is far
/// more likely to have made a typo than to have intended to disable
/// proactive sweeping (reactive, checkout-time rehydration still covers
/// correctness either way, per this module's own doc comment — so this
/// is a warn, not a clamp).
///
/// Pure function of the raw env var string (or its absence), not of
/// `std::env` directly, so unit tests can exercise every branch without
/// racing real env-var mutation across parallel `cargo test` threads.
fn resolve_interval_secs(raw: Option<&str>) -> u64 {
    const DEFAULT_SECS: i64 = 30;
    const FLOOR_SECS: i64 = 1;
    const SOFT_CEILING_SECS: i64 = 3600;

    let parsed = raw.and_then(|v| v.parse::<i64>().ok());
    match parsed {
        None => DEFAULT_SECS as u64,
        Some(n) if n < FLOOR_SECS => {
            eprintln!(
                "nirdosha kernel: NIRDOSHA_KERNEL_REAPER_INTERVAL_SECS={n} is below the {FLOOR_SECS}-second floor -- \
                 using {FLOOR_SECS}s instead (0 or a negative interval would busy-loop the reaper thread)"
            );
            FLOOR_SECS as u64
        }
        Some(n) if n > SOFT_CEILING_SECS => {
            eprintln!(
                "nirdosha kernel: NIRDOSHA_KERNEL_REAPER_INTERVAL_SECS={n} is above the {SOFT_CEILING_SECS}s (1h) soft ceiling -- \
                 accepted as configured, but this likely disables proactive rehydration rather than being an intentional choice \
                 (reactive, checkout-time rehydration still covers correctness either way)"
            );
            n as u64
        }
        Some(n) => n as u64,
    }
}

fn interval() -> Duration {
    Duration::from_secs(resolve_interval_secs(std::env::var("NIRDOSHA_KERNEL_REAPER_INTERVAL_SECS").ok().as_deref()))
}

/// One sweep round: [`super::pool::sweep_all_registered`], wrapped in
/// its own `catch_unwind` — **inside** the reaper's loop body, not
/// relying on [`super::thread_pool::ThreadPool`]'s own `worker_loop`
/// outer `catch_unwind`. RFC §5's own review-caught failure mode: the
/// reaper's job body is `loop { sweep_all_pools(); sleep(...) }`, a
/// panic inside one `sweep_all_pools()` call would otherwise be caught
/// at the *outermost* `catch_unwind`, ending the whole loop — the job
/// "completes" from `ThreadPool`'s perspective, no crash, nothing
/// visibly wrong, and rehydration silently stops forever while every
/// other health signal stays green. This is the fix: catch each sweep
/// individually, increment [`REAPER_PANICS`] on catch, and keep going
/// to the next `sleep`/wake regardless.
///
/// This is a kernel-side-bug containment mechanism (a poisoned lock
/// `.unwrap()`, an indexing error) — **not** a plugin-panic mechanism.
/// A panic crossing a plugin's plain `extern "C"` boundary
/// (`is_valid_fn`, called transitively through
/// `PluginManagedConnection::is_valid` during a sweep) is a *defined
/// abort at that boundary* (`thread_pool.rs`'s own "Panic containment"
/// doc comment), not something any Rust-side `catch_unwind` can
/// intercept — the panic never returns to this frame at all. This
/// `catch_unwind` only ever catches a panic that happens to originate
/// in this crate's own Rust code during a sweep.
fn sweep_once() {
    let offset = SWEEP_OFFSET.fetch_add(1, Ordering::Relaxed);
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| pool::sweep_all_registered(offset))).is_err() {
        REAPER_PANICS.fetch_add(1, Ordering::Relaxed);
    }
}

/// Starts the reaper, idempotently — safe to call more than once (later
/// calls are no-ops), the same `Once`-guarded self-start posture
/// `domain::register_builtin_domains` already established (Phase 2a).
/// `lib.rs`'s `nir_kernel_start_reaper` FFI wrapper is what `codegen.rs`
/// actually calls, once, in every compiled program's `main` preamble
/// (immediately after the domain/plugin-provider registration calls) —
/// this function itself has no dependency on that wiring and is
/// directly callable (and tested) from plain Rust too.
pub fn start() {
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        let interval = interval();
        REAPER_CONFIGURED_INTERVAL_SECS.store(interval.as_secs(), Ordering::Relaxed);
        // `submit`'s `Job` (`thread_pool.rs`) is private to that module
        // -- callers never name it, they just pass a closure that
        // structurally matches it, the same pattern every other
        // `ThreadPool::submit` call site in this crate already uses
        // (`lib.rs`, `nfr.rs`, `recorder.rs`).
        let _ = reaper_pool().submit(Box::new(move || {
            loop {
                if SHUTDOWN.load(Ordering::Relaxed) {
                    return;
                }
                sweep_once();
                std::thread::sleep(interval);
            }
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::pool::{ManageConnection, PoolConfig, PoolRegistry};

    #[test]
    fn resolve_interval_secs_defaults_on_unset_or_unparseable() {
        assert_eq!(resolve_interval_secs(None), 30);
        assert_eq!(resolve_interval_secs(Some("not a number")), 30);
        assert_eq!(resolve_interval_secs(Some("")), 30);
    }

    #[test]
    fn resolve_interval_secs_floors_zero_and_negative_to_one() {
        assert_eq!(resolve_interval_secs(Some("0")), 1);
        assert_eq!(resolve_interval_secs(Some("-5")), 1);
    }

    #[test]
    fn resolve_interval_secs_accepts_a_value_above_the_soft_ceiling_unclamped() {
        // Accepted as configured (just warned), not clamped down -- the
        // RFC's own "warn, not clamp" rule for the ceiling, unlike the
        // floor which does clamp.
        assert_eq!(resolve_interval_secs(Some("99999999")), 99999999);
    }

    #[test]
    fn resolve_interval_secs_passes_through_an_ordinary_value_unchanged() {
        assert_eq!(resolve_interval_secs(Some("45")), 45);
        assert_eq!(resolve_interval_secs(Some("3600")), 3600, "exactly the ceiling must not warn or clamp");
        assert_eq!(resolve_interval_secs(Some("1")), 1, "exactly the floor must not warn or clamp");
    }

    /// A synthetic Rust-side `ManageConnection` whose `is_valid` panics
    /// only while explicitly "armed" (an external `Arc<AtomicBool>` the
    /// test flips, disarming itself on the way out) — deliberately
    /// **not** "panics on its Nth call," because r2d2's own
    /// `try_get_inner` (confirmed by reading `r2d2 0.8.10`'s source
    /// directly, not assumed) calls `is_valid` on a connection it just
    /// created just as readily as on one popped from the idle list —
    /// `get_timeout`'s retry loop calls `add_connection` on an empty
    /// pool, then loops back into `try_get_inner`, which pops that
    /// freshly-added connection and validates it like any other. A
    /// call-count-based "first call panics" design panics during this
    /// test's own setup checkout, before `sweep_once` ever runs — caught
    /// by nothing, since setup code isn't wrapped in `catch_unwind` —
    /// which is exactly the failure this arm/disarm design avoids by
    /// construction rather than by getting r2d2's internal call count
    /// right.
    ///
    /// **Why this is the only correct shape for this test, stated
    /// explicitly per RFC §5's own warning**: a panic inside a plugin's
    /// `extern "C"` function is a defined *abort* at that FFI boundary
    /// (`thread_pool.rs`'s own doc comment) -- no Rust-side
    /// `catch_unwind` can intercept it, and attempting to exercise that
    /// path here would abort the whole `cargo test` process instead of
    /// producing a test failure. This test panics inside a plain Rust
    /// `ManageConnection::is_valid` impl instead, deliberately, which is
    /// exactly the class of bug (a kernel-side bug, not a plugin one)
    /// `sweep_once`'s `catch_unwind` actually exists to contain.
    struct PanicsWhileArmed {
        armed: Arc<AtomicBool>,
    }

    struct DummyConn;

    impl ManageConnection for PanicsWhileArmed {
        type Connection = DummyConn;
        type Error = std::io::Error;

        fn connect(&self) -> Result<Self::Connection, Self::Error> {
            Ok(DummyConn)
        }

        fn is_valid(&self, _conn: &mut Self::Connection) -> Result<(), Self::Error> {
            if self.armed.swap(false, Ordering::SeqCst) {
                panic!("PanicsWhileArmed: deliberate armed-call panic, kernel-side-bug simulation for sweep_once's catch_unwind test");
            }
            Ok(())
        }

        fn has_broken(&self, _conn: &mut Self::Connection) -> bool {
            false
        }
    }

    #[test]
    fn sweep_once_contains_a_panic_inside_one_pools_is_valid_and_keeps_sweeping() {
        let registry: PoolRegistry<PanicsWhileArmed> = PoolRegistry::new();
        let armed = Arc::new(AtomicBool::new(false));
        let mgr = PanicsWhileArmed { armed: Arc::clone(&armed) };
        let cfg = PoolConfig::default();
        let pool = registry.get_or_create("panic_test_key", cfg, || Ok(mgr)).expect("build a one-key pool");
        // Setup checkout, unarmed: whether or not r2d2 happens to call
        // `is_valid` on this freshly-created connection is irrelevant
        // now -- it can only ever return `Ok`. Returning it (`drop`)
        // makes it idle, available for `sweep_once`'s own `try_get`.
        drop(pool.get().expect("setup checkout, unarmed, must always succeed"));

        // Leaked deliberately -- `register_for_reaping` needs `&'static`,
        // and this test-only registry only ever needs to outlive this
        // one test process, the same permanent-leak posture other
        // process-wide `'static` kernel state in this crate already has.
        let leaked: &'static PoolRegistry<PanicsWhileArmed> = Box::leak(Box::new(registry));
        pool::register_for_reaping(leaked);

        let panics_before = reaper_panics();
        armed.store(true, Ordering::SeqCst);
        // `try_get` pops the one idle connection; `test_on_check_out`
        // (on by default) calls `is_valid`, now armed -- `sweep_once`
        // must contain the resulting panic.
        sweep_once();
        assert_eq!(reaper_panics(), panics_before + 1, "a panic inside one pool's is_valid during a sweep must increment reaper_panics");
        assert!(!armed.load(Ordering::SeqCst), "is_valid must actually have run (and disarmed itself) during the sweep, not been skipped");

        // r2d2 drops a connection whose `is_valid` panicked out from
        // under `catch_unwind` (the checkout never completed), so the
        // pool is now empty -- check one back in (unarmed, so this must
        // succeed) so the second sweep below has something to find.
        drop(pool.get().expect("checkout after the panic must still succeed, unarmed"));

        // Second sweep: must run at all (the loop wasn't ended by the
        // first sweep's panic) and must not double-count as a panic
        // (still unarmed).
        sweep_once();
        assert_eq!(reaper_panics(), panics_before + 1, "a second, unarmed sweep must not increment reaper_panics again");
    }

    #[test]
    fn start_is_idempotent_and_does_not_panic_when_called_more_than_once() {
        // Real assertion here is narrow and deliberate: `start` must not
        // panic or block, called twice, from a plain unit test -- it
        // doesn't assert anything about sweep timing, since this
        // process-wide reaper is shared with every other test in this
        // binary and a real interval-based assertion would be flaky by
        // construction.
        start();
        start();
    }
}
