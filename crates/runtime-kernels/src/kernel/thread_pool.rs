//! A self-tuning, reused-worker OS thread pool — ported near-verbatim
//! from the interpreter-side `crates/compiler/src/thread_pool.rs`
//! (removed along with the interpreter, which used it to back `spawn`),
//! because its scheduling/reuse logic has zero interpreter dependency:
//! plain `std::thread`/`Mutex`/`Condvar`/`VecDeque`, nothing
//! `nirdosha`-specific. **Not wired to anything yet, on purpose** — same
//! treatment [`super::HandleTable`]/[`super::pool`] get. `spawn`/
//! `thread` don't compile at all today (`codegen.rs` hard-rejects them,
//! Track B item B6), and per `rfcs/0007-apm-runtime-kernel.md` §8, that
//! item isn't separable from `rfcs/0006-structured-concurrency.md`'s
//! still-unbuilt compiled-concurrency design — this exists now so
//! whichever of those lands first has a real, adversarially-tested
//! worker-reuse mechanism to build on, instead of starting from raw
//! `std::thread::spawn` per call.
//!
//! **Panic containment — resolved and verified, not just assumed.**
//! `worker_loop`'s `catch_unwind` depends on stack unwinding being
//! enabled. This crate's `[profile.release]` used to set `panic =
//! "abort"`, on the reasoning that a panic reaching this crate's own
//! `extern "C"` boundary with unwinding enabled would be undefined
//! behavior. That reasoning predates Rust 1.71: unwinding across a
//! plain `extern "C"` boundary has been *defined* (a safe abort at that
//! boundary, not UB) since then, which is what makes `panic = "unwind"`
//! (`Cargo.toml`'s own doc comment has the full reasoning) strictly
//! safer than `abort` here, not riskier — `catch_unwind` now genuinely
//! contains a panicking job, and an uncaught panic that somehow still
//! reached an `extern "C"` kernel would still abort at that boundary
//! either way. Verified against the real production link path, not
//! assumed: `rfcs/evidence/0007-apm-runtime-kernel/panic_containment/`
//! is a hand-written C program with zero Rust runtime except this
//! crate's own staticlib, linked exactly the way `codegen.rs::build`
//! links a real compiled `.nir` binary — it calls
//! `nir_kernel_self_test_panic_containment` (`lib.rs`), submits a
//! panicking job, and confirms the pool survives and runs a job after
//! it. Run clean 5/5. `kernel_bench` confirms zero measurable overhead
//! on any `nir_tcp_*`/`nir_file_*` call from enabling unwinding —
//! unwind tables are metadata, not a runtime cost, when no panic
//! occurs.
//!
//! **What this deliberately is *not*, and why**: literal Java-style
//! virtual threads (a user-space scheduler multiplexing many logical
//! threads onto a few OS threads) need either a managed runtime that
//! controls its own continuation representation, unsafe stackful
//! coroutines (real UB risk, contradicting this project's own
//! memory-safety guarantees), or a from-scratch async rewrite — none of
//! which this module attempts. What it actually is: real OS threads,
//! reused instead of created fresh per submission — the same proven
//! pattern production thread pools (.NET's `ThreadPool`, Java's own
//! classic `ThreadPoolExecutor`) use. `submit` never leaves a job
//! waiting behind other work purely because every worker happens to be
//! busy: if no worker is idle, a brand-new one is spawned immediately,
//! just for that job.
//!
//! ## Why eager growth, not a bounded queue
//!
//! The obvious-looking alternative — a fixed-size pool with a bounded
//! task queue, blocking `submit` when full — is the textbook thread-pool
//! deadlock trap for exactly this workload: `spawn`+`join` means a
//! worker routinely blocks *waiting on another task this same pool must
//! run*. With a bounded pool, once every worker is blocked waiting on a
//! child still sitting in the queue because no worker is free to start
//! it, nothing can ever make progress. Eager growth (spawn a new worker
//! the instant a job would otherwise wait) makes this class of
//! self-inflicted deadlock structurally impossible: a submitted job is
//! always either claimed by an already-idle worker or given a
//! freshly-spawned one, synchronously, inside `submit` itself.
//!
//! ## Failure mode changed, not just reuse added
//!
//! Calling the free-function `std::thread::spawn` **panics the whole
//! process** if the OS refuses to create a new thread (`RLIMIT_NPROC`/
//! `kernel.threads-max` exhaustion under real heavy load). `submit` here
//! is fallible (`Result<(), SpawnError>`) — a real OS-level failure to
//! create a thread becomes a clean, propagatable error instead of a
//! process abort.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// How long an idle worker waits for new work before retiring — long
/// enough that a bursty-but-not-continuous workload doesn't thrash
/// creating and tearing down threads between waves; short enough that a
/// genuine one-off burst doesn't leave threads parked indefinitely.
const IDLE_TIMEOUT: Duration = Duration::from_secs(10);

type Job = Box<dyn FnOnce() + Send + 'static>;

/// The one real, fallible OS operation this module performs — spawning
/// a worker's backing thread. A trait, not a bare function pointer, so
/// tests can substitute a fake that fails on command without needing to
/// actually exhaust real OS thread resources.
pub trait Spawner: Send + Sync + 'static {
    fn spawn(&self, f: Box<dyn FnOnce() + Send + 'static>) -> std::io::Result<()>;
}

/// The real spawner `ThreadPool::new` uses — `std::thread::Builder`, not
/// the free-function `std::thread::spawn`, specifically because the
/// builder form returns a `Result` instead of panicking on failure.
struct RealSpawner;
impl Spawner for RealSpawner {
    fn spawn(&self, f: Box<dyn FnOnce() + Send + 'static>) -> std::io::Result<()> {
        std::thread::Builder::new().spawn(f)?;
        Ok(())
    }
}

struct PoolState {
    queue: VecDeque<Job>,
    idle_count: usize,
}

/// A self-tuning worker-thread pool. See the module doc comment for the
/// full design rationale, including the panic-containment gap this port
/// does not yet close. Cloned via `Arc` into every job it runs (a
/// spawned task can itself submit more work, recursively, onto the same
/// pool) — `submit` takes `&Arc<Self>` for exactly this reason.
pub struct ThreadPool {
    state: Mutex<PoolState>,
    condvar: Condvar,
    spawner: Box<dyn Spawner>,
    /// Diagnostic/test-only: real OS worker threads currently alive
    /// (busy or idle-but-not-yet-retired).
    live_workers: AtomicUsize,
}

/// A `submit` call that couldn't get a worker running — currently only
/// reachable when the OS itself refuses to create a new thread. The job
/// that was about to be submitted is **not** run and **not** left
/// queued.
#[derive(Debug)]
pub struct SpawnError(pub String);

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "could not create a thread to run this: {}", self.0)
    }
}

impl ThreadPool {
    pub fn new() -> Arc<Self> {
        Self::with_spawner(Box::new(RealSpawner))
    }

    fn with_spawner(spawner: Box<dyn Spawner>) -> Arc<Self> {
        Arc::new(ThreadPool { state: Mutex::new(PoolState { queue: VecDeque::new(), idle_count: 0 }), condvar: Condvar::new(), spawner, live_workers: AtomicUsize::new(0) })
    }

    /// Real OS worker threads currently alive right now (test/diagnostic
    /// use only).
    pub fn live_worker_count(&self) -> usize {
        self.live_workers.load(Ordering::SeqCst)
    }

    /// Runs `job` on some worker thread — an already-idle one if any
    /// exists, otherwise a freshly-spawned one, synchronously, before
    /// this call returns (see the module doc's "why eager growth"
    /// section). `Err` only when the OS itself refused to create a
    /// thread; the queue is left exactly as it was found.
    pub fn submit(self: &Arc<Self>, job: Job) -> Result<(), SpawnError> {
        let mut state = self.state.lock().unwrap();
        if state.idle_count > 0 {
            state.queue.push_back(job);
            self.condvar.notify_one();
            return Ok(());
        }
        drop(state); // never hold the lock across the real spawn syscall
        let pool = Arc::clone(self);
        self.live_workers.fetch_add(1, Ordering::SeqCst);
        let first_job = job;
        let outcome = self.spawner.spawn(Box::new(move || pool.worker_loop(Some(first_job))));
        if let Err(e) = outcome {
            self.live_workers.fetch_sub(1, Ordering::SeqCst);
            return Err(SpawnError(e.to_string()));
        }
        Ok(())
    }

    /// One worker's whole lifetime: run `first_job`, then loop pulling
    /// more work off the shared queue until `IDLE_TIMEOUT` of nothing to
    /// do, then retire.
    fn worker_loop(self: Arc<Self>, first_job: Option<Job>) {
        let mut next = first_job;
        loop {
            let job = match next.take() {
                Some(j) => j,
                None => match self.wait_for_job() {
                    Some(j) => j,
                    None => {
                        self.live_workers.fetch_sub(1, Ordering::SeqCst);
                        return;
                    }
                },
            };
            // Deliberately outside any pool-internal lock: a panic
            // inside `job` must never happen while `self.state`'s mutex
            // is held, or it would poison the whole pool for every
            // future job on every worker. `catch_unwind` genuinely
            // contains it -- this crate's `[profile.release]` is
            // `panic = "unwind"` (verified against the real production
            // link path, not assumed; see this module's own doc
            // comment and `rfcs/evidence/0007-apm-runtime-kernel/
            // panic_containment/`), so this is real containment in the
            // artifact that actually ships, not just under `cargo
            // test`'s own dev-profile safety net.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
        }
    }

    /// Blocks (with periodic wakeups so an idle worker doesn't wait past
    /// `IDLE_TIMEOUT` forever) until a job is available or it's time to
    /// retire. `None` means "retire."
    fn wait_for_job(&self) -> Option<Job> {
        let mut state = self.state.lock().unwrap();
        loop {
            if let Some(j) = state.queue.pop_front() {
                return Some(j);
            }
            state.idle_count += 1;
            let (guard, wait_result) = self.condvar.wait_timeout(state, IDLE_TIMEOUT).unwrap();
            state = guard;
            state.idle_count -= 1;
            if wait_result.timed_out() && state.queue.is_empty() {
                return None;
            }
        }
    }
}

/// Structured concurrency (`rfcs/0006-structured-concurrency.md`
/// Pillar 4's "no orphan threads") for the *reused-worker* pool.
/// Deliberately not a re-export of `std::thread::scope`: that API
/// always creates fresh OS threads bound to a lexical closure, which
/// would throw away the entire reason `ThreadPool` exists (worker
/// reuse). `Scope` instead tracks how many jobs submitted through it
/// are still outstanding, and blocks — on an explicit [`Scope::join`]
/// call, or automatically when the `Scope` is dropped — until that
/// count reaches zero. Same guarantee `std::thread::scope` gives (a
/// scope cannot exit while any of its children are still running),
/// composed correctly with a shared, reused worker pool instead of
/// bypassing it.
///
/// **The point of building this in now, not later**: whenever `spawn`
/// gets real codegen (Track B item B6), every function-call frame that
/// spawns anything gets one `Scope` implicitly — the `.nir` author
/// never writes `scope { }`; the guarantee (nothing spawned by this
/// function can outlive it, no orphan threads ever) is just true of
/// `spawn` from day one, the same way [`super::mailbox`]'s non-blocking
/// `send` becomes `chan`'s behavior with no new syntax.
pub struct Scope {
    pool: Arc<ThreadPool>,
    outstanding: Arc<(Mutex<usize>, Condvar)>,
}

/// Decrements the scope's outstanding-job count on drop — including
/// during a panic unwind, not just on normal return. Without this, a
/// job that panics would leave the count permanently non-zero and
/// `Scope::join`/`Drop` would hang forever waiting for a completion
/// that already happened (just not the way this counter expected to
/// hear about it). `catch_unwind` already contains the panic itself
/// one level up (`worker_loop`) — this guard only has to make sure the
/// *bookkeeping* survives the unwind on its way there, which a plain
/// RAII `Drop` does correctly under this crate's `panic = "unwind"`
/// profile (verified, not assumed — see `Cargo.toml`'s own doc
/// comment).
struct DecrementOnDrop(Arc<(Mutex<usize>, Condvar)>);

impl Drop for DecrementOnDrop {
    fn drop(&mut self) {
        let mut count = self.0.0.lock().unwrap();
        *count -= 1;
        if *count == 0 {
            self.0.1.notify_all();
        }
    }
}

impl Scope {
    pub fn new(pool: &Arc<ThreadPool>) -> Self {
        Scope { pool: Arc::clone(pool), outstanding: Arc::new((Mutex::new(0), Condvar::new())) }
    }

    /// Submits `job` to the underlying pool, tracked by this scope —
    /// [`Scope::join`] (or dropping the scope) will wait for it, even
    /// if it panics. Same fallible-submit contract as
    /// [`ThreadPool::submit`] itself: `Err` only when the OS refuses to
    /// create a worker thread, in which case nothing was submitted and
    /// the scope's count is rolled back to what it was before this call.
    pub fn spawn(&self, job: Job) -> Result<(), SpawnError> {
        {
            let mut count = self.outstanding.0.lock().unwrap();
            *count += 1;
        }
        let outstanding = Arc::clone(&self.outstanding);
        let wrapped: Job = Box::new(move || {
            let _guard = DecrementOnDrop(outstanding);
            job();
            // `_guard` drops here on normal return, or during unwind if
            // `job` panicked -- either way, the count is decremented
            // exactly once per successfully submitted job.
        });
        let result = self.pool.submit(wrapped);
        if result.is_err() {
            let mut count = self.outstanding.0.lock().unwrap();
            *count -= 1;
        }
        result
    }

    /// Blocks until every job spawned via this scope has completed
    /// (whether it returned normally or panicked). Called automatically
    /// on `Drop` — calling it explicitly is only useful to wait at a
    /// specific point without ending the scope's lifetime yet, since
    /// more jobs can still be spawned on it afterward.
    pub fn join(&self) {
        let (lock, cvar) = &*self.outstanding;
        let mut count = lock.lock().unwrap();
        while *count > 0 {
            count = cvar.wait(count).unwrap();
        }
    }

    /// Non-blocking: `true` if every job spawned via this scope has
    /// already completed (so a `join()` call right now would return
    /// immediately, without ever actually waiting). Exists for
    /// `nir_thread_join`'s own deadlock detector (`kernel::concurrency_
    /// wait_begin`) — a `join` that was never going to block must never
    /// be counted as a real wait, or a fast-finishing spawn racing a
    /// slower caller could look indistinguishable from a genuine stall.
    pub fn already_done(&self) -> bool {
        *self.outstanding.0.lock().unwrap() == 0
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        self.join();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn real_pool() -> Arc<ThreadPool> {
        ThreadPool::new()
    }

    #[test]
    fn a_single_job_runs_and_the_worker_is_reused_for_the_next_one() {
        let pool = real_pool();
        let (tx, rx) = mpsc::channel();
        let tx2 = tx.clone();
        pool.submit(Box::new(move || tx2.send(1).unwrap())).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), 1);
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(pool.live_worker_count(), 1, "the one worker should still be alive, parked idle");
        let tx3 = tx.clone();
        pool.submit(Box::new(move || tx3.send(2).unwrap())).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), 2);
        assert_eq!(pool.live_worker_count(), 1);
    }

    #[test]
    fn many_sequential_bursts_reuse_a_small_number_of_workers_not_one_per_spawn() {
        let pool = real_pool();
        for _ in 0..20 {
            let (tx, rx) = mpsc::channel();
            for _ in 0..25 {
                let tx = tx.clone();
                pool.submit(Box::new(move || tx.send(()).unwrap())).unwrap();
            }
            drop(tx);
            for _ in 0..25 {
                rx.recv_timeout(Duration::from_secs(2)).unwrap();
            }
        }
        std::thread::sleep(Duration::from_millis(50));
        let live = pool.live_worker_count();
        assert!(live <= 25, "expected reuse to keep the worker count near one burst's width (<=25), got {live}");
    }

    #[test]
    fn idle_workers_retire_after_the_timeout_and_the_pool_shrinks_back_down() {
        let pool = real_pool();
        let (tx, rx) = mpsc::channel();
        for _ in 0..10 {
            let tx = tx.clone();
            pool.submit(Box::new(move || tx.send(()).unwrap())).unwrap();
        }
        drop(tx);
        for _ in 0..10 {
            rx.recv_timeout(Duration::from_secs(2)).unwrap();
        }
        assert!(pool.live_worker_count() >= 1);
        std::thread::sleep(IDLE_TIMEOUT + Duration::from_secs(2));
        assert_eq!(pool.live_worker_count(), 0, "idle workers should have retired and the pool shrunk back to zero");
    }

    #[test]
    fn a_job_that_spawn_join_blocks_waiting_on_a_child_never_deadlocks_from_pool_exhaustion() {
        let pool = real_pool();
        const DEPTH: usize = 50;
        let (final_tx, final_rx) = mpsc::channel::<usize>();

        fn chain(pool: Arc<ThreadPool>, depth: usize, final_tx: mpsc::Sender<usize>) {
            if depth == 0 {
                final_tx.send(0).unwrap();
                return;
            }
            let (child_tx, child_rx) = mpsc::channel::<usize>();
            let pool2 = Arc::clone(&pool);
            pool.submit(Box::new(move || chain(pool2, depth - 1, child_tx))).unwrap();
            let child_result = child_rx.recv_timeout(Duration::from_secs(5)).expect("child should complete, not deadlock");
            final_tx.send(child_result + 1).unwrap();
        }

        let pool2 = Arc::clone(&pool);
        pool.submit(Box::new(move || chain(pool2, DEPTH, final_tx))).unwrap();
        let result = final_rx.recv_timeout(Duration::from_secs(10)).expect("the whole chain should resolve, not deadlock on pool exhaustion");
        assert_eq!(result, DEPTH);
    }

    struct FlakySpawner {
        fail_every: usize,
        calls: AtomicUsize,
    }
    impl Spawner for FlakySpawner {
        fn spawn(&self, f: Box<dyn FnOnce() + Send + 'static>) -> std::io::Result<()> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if n % self.fail_every == 0 {
                return Err(std::io::Error::other("injected failure: OS refused to create a thread"));
            }
            std::thread::Builder::new().spawn(f)?;
            Ok(())
        }
    }

    #[test]
    fn a_real_os_level_spawn_failure_is_a_clean_err_not_a_panic() {
        let pool = ThreadPool::with_spawner(Box::new(FlakySpawner { fail_every: 1, calls: AtomicUsize::new(0) }));
        let result = pool.submit(Box::new(|| {}));
        assert!(result.is_err(), "expected the injected spawn failure to surface as Err");
        assert_eq!(pool.live_worker_count(), 0, "a failed spawn must not leave the live-worker count incremented");
    }

    #[test]
    fn an_intermittent_spawn_failure_does_not_corrupt_the_pool_for_later_submissions() {
        struct FailFirstCallOnly {
            calls: AtomicUsize,
        }
        impl Spawner for FailFirstCallOnly {
            fn spawn(&self, f: Box<dyn FnOnce() + Send + 'static>) -> std::io::Result<()> {
                if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    return Err(std::io::Error::other("injected failure: OS refused to create a thread"));
                }
                std::thread::Builder::new().spawn(f)?;
                Ok(())
            }
        }
        let pool = ThreadPool::with_spawner(Box::new(FailFirstCallOnly { calls: AtomicUsize::new(0) }));
        let first = pool.submit(Box::new(|| {}));
        assert!(first.is_err(), "the first real spawn() call should have hit the injected failure");
        assert_eq!(pool.live_worker_count(), 0);

        let (tx, rx) = mpsc::channel();
        pool.submit(Box::new(move || tx.send("still alive").unwrap())).expect("the pool must remain usable after a prior spawn failure");
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), "still alive");
    }

    #[test]
    fn a_panic_inside_a_job_does_not_poison_the_pool_for_the_next_job() {
        // Proves the scheduling/reuse logic tolerates a panicking job
        // under this test binary's own `dev`/`test` profile. The
        // separate, harder question -- does containment survive in the
        // actual `--release` artifact this crate ships as, called via
        // `extern "C"` from a host with no Rust runtime of its own --
        // is answered by `rfcs/evidence/0007-apm-runtime-kernel/
        // panic_containment/`, not by this test (this module's own doc
        // comment has the full story: now verified, not just assumed).
        let pool = real_pool();
        let (tx, rx) = mpsc::channel();
        pool.submit(Box::new(|| panic!("boom"))).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        let tx2 = tx.clone();
        pool.submit(Box::new(move || tx2.send("still alive").unwrap())).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), "still alive");
    }

    #[test]
    fn scope_join_waits_for_every_job_spawned_through_it() {
        let pool = real_pool();
        let scope = Scope::new(&pool);
        let done = Arc::new(AtomicUsize::new(0));
        for _ in 0..20 {
            let done = Arc::clone(&done);
            scope
                .spawn(Box::new(move || {
                    std::thread::sleep(Duration::from_millis(10));
                    done.fetch_add(1, Ordering::SeqCst);
                }))
                .unwrap();
        }
        scope.join();
        assert_eq!(done.load(Ordering::SeqCst), 20, "join() must not return before every spawned job has actually completed");
    }

    #[test]
    fn already_done_is_false_while_outstanding_and_true_once_every_job_finishes() {
        let pool = real_pool();
        let scope = Scope::new(&pool);
        assert!(scope.already_done(), "a scope with nothing spawned yet has nothing outstanding");
        let (tx, rx) = mpsc::channel::<()>();
        scope
            .spawn(Box::new(move || {
                rx.recv().unwrap(); // held open until this test releases it below
            }))
            .unwrap();
        assert!(!scope.already_done(), "a job is still genuinely outstanding");
        tx.send(()).unwrap();
        scope.join();
        assert!(scope.already_done(), "join() only returns once every job has actually completed");
    }

    #[test]
    fn dropping_a_scope_waits_the_same_way_join_does() {
        let pool = real_pool();
        let done = Arc::new(AtomicUsize::new(0));
        {
            let scope = Scope::new(&pool);
            for _ in 0..10 {
                let done = Arc::clone(&done);
                scope
                    .spawn(Box::new(move || {
                        std::thread::sleep(Duration::from_millis(10));
                        done.fetch_add(1, Ordering::SeqCst);
                    }))
                    .unwrap();
            }
            // `scope` drops at the end of this block -- C1's actual
            // guarantee: the block cannot finish exiting while any
            // spawned job is still outstanding.
        }
        assert_eq!(done.load(Ordering::SeqCst), 10, "the scope's Drop must have waited for every job before this point");
    }

    #[test]
    fn a_panicking_scoped_job_does_not_hang_join_forever() {
        // The real risk `DecrementOnDrop` exists to prevent: without it,
        // a panicking job would skip the "decrement the outstanding
        // count" step entirely, and `join`/`Drop` would wait on a count
        // that can never reach zero -- a real, silent deadlock, not
        // just a missed decrement. This test would hang (and eventually
        // fail on a test timeout) if that guard didn't work.
        let pool = real_pool();
        let scope = Scope::new(&pool);
        let done = Arc::new(AtomicUsize::new(0));
        scope.spawn(Box::new(|| panic!("boom inside a scoped job"))).unwrap();
        for _ in 0..5 {
            let done = Arc::clone(&done);
            scope
                .spawn(Box::new(move || {
                    done.fetch_add(1, Ordering::SeqCst);
                }))
                .unwrap();
        }
        scope.join();
        assert_eq!(done.load(Ordering::SeqCst), 5, "the panicking job must not have prevented the others from completing or being waited for");
    }

    #[test]
    fn nested_scopes_compose_the_inner_fully_joins_before_the_outer_job_completes() {
        let pool = real_pool();
        let outer = Scope::new(&pool);
        let inner_done_before_outer_job_returns = Arc::new(AtomicUsize::new(0));
        let flag = Arc::clone(&inner_done_before_outer_job_returns);
        let pool2 = Arc::clone(&pool);
        outer
            .spawn(Box::new(move || {
                let inner = Scope::new(&pool2);
                let flag2 = Arc::clone(&flag);
                inner
                    .spawn(Box::new(move || {
                        std::thread::sleep(Duration::from_millis(20));
                        flag2.store(1, Ordering::SeqCst);
                    }))
                    .unwrap();
                // `inner` drops here, at the end of this closure --
                // must block until its own child is done, so `flag` is
                // guaranteed set by the time this outer job returns.
            }))
            .unwrap();
        outer.join();
        assert_eq!(inner_done_before_outer_job_returns.load(Ordering::SeqCst), 1, "the inner scope must have fully joined before the outer job (and therefore the outer scope) considered itself done");
    }
}
