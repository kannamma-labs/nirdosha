//! Live FFI wiring for transaction-isolation anomaly detection over
//! `transact`'s own `db` operations -- Nirdosha's answer to the
//! `killer_demo` gap (`docs/research/2026-09-competitive-verification-
//! and-signing-landscape.md` §6): "no data races" today covers
//! `chan`/`spawn` only, and the same lost-update corruption
//! `killer_demo` shows in Python is equally possible through `db`
//! inside a `transact` block, with no guarantee at all otherwise.
//!
//! **The actual detection algorithm (the Direct Serialization Graph,
//! Adya's WW/WR/RW edges, cycle detection) lives in
//! `nirdosha-isolation-core`** (`crates/isolation-core`), extracted out
//! of this module 2026-09-14 so `crates/compiler`'s tooling (`nirdosha
//! check-isolation`, certificate isolation-violation attachment) can
//! depend on the identical, pure detector without linking this crate's
//! own heavy native deps (postgres, native-tls, rusqlite) it needs for
//! everything else. This module is now specifically the *live* half:
//! hooking `db.rs`'s two funnel points (`nir_db_execute`/
//! `nir_db_query`), attributing each call to the `transact` site
//! active on its own thread, holding the one process-wide `Checker`,
//! and firing the async escalation.
//!
//! **Why this is a *detector*, not a *prover*, and why that's the
//! honest answer, not a lesser one.** None of the field's static
//! deductive verifiers (Dafny, Prusti, Creusot, Kani) reason about an
//! external, mutable SQL store any differently than Nirdosha does
//! today -- they verify in-memory state. The field's own practical
//! answer for transaction serializability is dynamic, a-posteriori
//! checking of an observed operation history, not a priori proof:
//! Jepsen's Elle (Kingsbury & Alvaro, VLDB'21) infers a Direct
//! Serialization Graph from observed reads/writes and reports
//! anomalies as *cycles* in it; a 2026 follow-on
//! ("Making Transaction Isolation Checking Practical") makes the same
//! technique faster, not different in kind. This module is that
//! technique, scoped to what Nirdosha's own runtime already has for
//! free: `transact`'s compiler-enforced protocol
//! (`crates/runtime-kernels/src/kernel/transact.rs`) already tracks a
//! `txn_id` per transact site; `db.rs`'s `nir_db_execute`/
//! `nir_db_query` are the two funnel points every compiled `db` call
//! goes through, no matter how deep in the call graph. Hooking those
//! two points is enough to observe every `db` operation a `transact`
//! site's `network`/`verify`/`commit` body performs, with zero changes
//! to `codegen.rs`'s own IR emission.
//!
//! **Real, disclosed simplifications (this module's own honesty
//! obligation, matching every other "v1 scope" in this codebase):**
//! - **Resource identity is coarse: normalized SQL text + bind-value
//!   JSON, not a parsed row/column set** -- see `isolation-core::
//!   resource_key`'s own doc comment for the full reasoning.
//! - **Only `db` operations that happen while a `transact` site is
//!   active are tracked at all** (`current_txn()` below) -- a bare
//!   `db` call outside any `transact` block is invisible to this
//!   checker, which is the correct scope: `transact` is the boundary
//!   this project already asks the model to route durable side effects
//!   through, and it's the only place a `txn_id` exists to attribute
//!   an operation to.
//! - **Adya's three core dependency edges only (WW/WR/RW) — no
//!   process-order or real-time-order edges.** This detects
//!   serializability violations (including the lost-update pattern
//!   `killer_demo` demonstrates), not the stronger strict-
//!   serializability property session/real-time edges would add.
//!   Real, disclosed follow-up work, not a silent gap.
//! - **`evidence_tier: "monitored"`, never `"proved"`, if this ever
//!   reaches a certificate** (mirroring `NfrCommitment`'s own
//!   discipline in `mcp_tools.rs`) -- a detected cycle is conclusive
//!   evidence of an anomaly that already happened; the *absence* of a
//!   detected cycle is never a soundness guarantee that one can't
//!   happen, the same honest asymmetry `evidence_tier`'s `PROVED`/
//!   `DISPROVED`/`UNKNOWN` three-way split already models for Z3.
//! - **No windowing yet: the shared `Checker`'s history is never
//!   rotated on size alone.** `record_and_check` below calls `check()`
//!   on every single tracked `db` op -- `nirdosha-isolation-core::
//!   Checker::check`'s own doc comment already discloses this is meant
//!   for "a bounded checking window." Two real, disclosed fixes landed
//!   2026-09-14, found and measured building `examples/isolation_demo/`
//!   (its own RESULTS.md has the numbers for both): `find_cycles`'s
//!   rewrite fixed the *algorithmic* blowup that used to hit at
//!   ~24-28 concurrent transacts on one resource; `MAX_TRACKED_OPS`
//!   below (`clear_shared()`, which already existed, now actually
//!   gets called) bounds the checker's history so `check()`'s own
//!   still-real `O(ops²)`-ish per-call cost never grows past a fixed
//!   ceiling no matter how long the process runs. **Real, disclosed
//!   cost of that bound**: a WR/RW dependency whose two ops land in
//!   different windows (the older one already cleared by the time the
//!   newer one is recorded) is invisible to this checker -- a false
//!   negative, never a false positive. `evidence_tier: "monitored"`
//!   already carries exactly this asymmetry for the *whole* checker,
//!   not just this one cause of it (this module's own top doc comment).

/// How many ops the shared checker keeps before `record_and_check`
/// rotates it out from under itself (`Checker::clear`). Picked from a
/// real measurement, not guessed: `examples/isolation_demo/RESULTS.md`
/// found `check()` comfortably fast (low seconds) through several
/// hundred tracked ops and measurably slow well before a few thousand;
/// 300 leaves real margin on the fast side while still spanning many
/// times over the handful of ops one `transact` site's own `commit`
/// slot performs (`killer_demo`'s own shape: ~5 ops per transfer), so
/// a genuine same-window conflict between concurrently racing transacts
/// stays visible in the overwhelmingly common case.
const MAX_TRACKED_OPS: usize = 300;

use std::cell::RefCell;
use std::sync::{Mutex, OnceLock};

pub use nirdosha_isolation_core::{Anomaly, Checker, OpKind, resource_key};

/// The process-wide checker every `db` call inside an active
/// `transact` site feeds (`db.rs`'s `nir_db_execute`/`nir_db_query`)
/// and `nirdosha check-isolation`/APM escalation reads from.
fn shared() -> &'static Mutex<Checker> {
    static CHECKER: OnceLock<Mutex<Checker>> = OnceLock::new();
    CHECKER.get_or_init(|| Mutex::new(Checker::new()))
}

thread_local! {
    /// The `txn_id` of the `transact` site currently active on this
    /// thread, if any -- set by `transact.rs`'s `nir_transact_begin`
    /// and cleared by its `nir_transact_mark_committed`/
    /// `nir_transact_mark_compensated`. A thread-local, not a global,
    /// because a compiled program can run more than one `transact`
    /// concurrently on different threads (`spawn`); each thread's own
    /// notion of "which transact is active right now" must stay
    /// independent.
    static CURRENT_TXN: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Called by `transact.rs` when a `transact` site begins.
pub(crate) fn set_current_txn(txn_id: Option<String>) {
    CURRENT_TXN.with(|c| *c.borrow_mut() = txn_id);
}

/// Called by `db.rs`'s `nir_db_execute`/`nir_db_query` -- `None` means
/// this `db` call is outside any `transact` site and is not tracked at
/// all (this module's own doc comment on why that's the right scope).
pub(crate) fn current_txn() -> Option<String> {
    CURRENT_TXN.with(|c| c.borrow().clone())
}

/// Records one `db` operation against the shared checker, then runs a
/// check and fires an async escalation (mirroring `nfr.rs`'s own
/// `NIRDOSHA_OBSERVABILITY_URL` posture exactly) if it just completed a
/// cycle. Never on the calling thread, and never allowed to affect the
/// `db` call's own return value -- an anomaly detector must not become
/// a new way for `db` operations to fail. A no-op if `txn` is `None`
/// (nothing to attribute the op to).
pub(crate) fn record_and_check(txn: Option<String>, resource: String, kind: OpKind) {
    let Some(txn) = txn else { return };
    let anomalies = {
        let mut checker = shared().lock().unwrap_or_else(|e| e.into_inner());
        checker.record(txn, resource, kind);
        let anomalies = checker.check();
        // Rotated *after* this op's own `check()` already ran against
        // it -- an op is always checked against its own window at
        // least once before it can be cleared, so this never misses
        // the anomaly the op that triggers rotation is itself part of.
        if checker.ops().len() >= MAX_TRACKED_OPS {
            checker.clear();
        }
        anomalies
    };
    for anomaly in anomalies {
        escalate(&anomaly);
    }
}

/// Test/tooling entry point: a snapshot of every anomaly the shared
/// checker currently reports, without touching the escalation path.
pub fn snapshot_anomalies() -> Vec<Anomaly> {
    shared().lock().unwrap_or_else(|e| e.into_inner()).check()
}

/// Test/tooling entry point: clears the shared checker's recorded
/// history (not its escalation state, which is stateless per call).
pub fn clear_shared() {
    shared().lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// Test/tooling entry point: how many ops the shared checker currently
/// holds -- mainly so `MAX_TRACKED_OPS` rotation is directly observable
/// (`record_and_check`'s own tests below), not something a caller has
/// to infer indirectly.
pub fn tracked_ops_count() -> usize {
    shared().lock().unwrap_or_else(|e| e.into_inner()).ops().len()
}

/// Fires one escalation, asynchronously, reusing `nfr.rs`'s own
/// `NIRDOSHA_OBSERVABILITY_URL` target and raw HTTP POST (this crate
/// should have exactly one hand-written HTTP client, not one per
/// escalating subsystem). A plain `std::thread::spawn`, not `nfr.rs`'s
/// pooled `ThreadPool`: an isolation anomaly is a rare event compared
/// to a per-call NFR check, so a pooled worker's extra machinery isn't
/// worth sharing state with `nfr.rs` over -- a real, disclosed
/// simplification, not an oversight.
fn escalate(anomaly: &Anomaly) {
    let Some((host, port, path)) = super::nfr::observability_target() else { return };
    let host = host.clone();
    let port = *port;
    let path = path.clone();
    let cycle = anomaly.cycle.clone();
    std::thread::spawn(move || {
        let cycle_json: String = cycle.iter().map(|t| format!("\"{}\"", t.replace('\\', "\\\\").replace('"', "\\\""))).collect::<Vec<_>>().join(",");
        let body = format!(r#"{{"kind":"isolation_anomaly","cycle":[{cycle_json}],"timestamp_ms":{}}}"#, unix_time_ms());
        super::nfr::post_json_fire_and_forget(&host, port, &path, &body);
    });
}

fn unix_time_ms() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_txn_thread_local_round_trips() {
        assert_eq!(current_txn(), None);
        set_current_txn(Some("t1".to_string()));
        assert_eq!(current_txn(), Some("t1".to_string()));
        set_current_txn(None);
        assert_eq!(current_txn(), None);
    }

    #[test]
    fn record_and_check_is_a_noop_with_no_active_txn() {
        clear_shared();
        record_and_check(None, "balance:acct1".to_string(), OpKind::Write);
        assert!(snapshot_anomalies().is_empty());
    }

    /// The `killer_demo` shape, driven through this module's own live
    /// entry point (`record_and_check`/`snapshot_anomalies`), not
    /// `isolation-core`'s lower-level `Checker` API directly -- pins
    /// that the thin FFI-facing wrapper around the shared crate still
    /// behaves correctly, not just the algorithm it delegates to.
    #[test]
    fn record_and_check_detects_the_killer_demo_lost_update_pattern_through_this_modules_own_entry_point() {
        clear_shared();
        record_and_check(Some("a".to_string()), "balance:acct1".to_string(), OpKind::Read);
        record_and_check(Some("b".to_string()), "balance:acct1".to_string(), OpKind::Read);
        record_and_check(Some("a".to_string()), "balance:acct1".to_string(), OpKind::Write);
        record_and_check(Some("b".to_string()), "balance:acct1".to_string(), OpKind::Write);
        let anomalies = snapshot_anomalies();
        assert!(!anomalies.is_empty(), "the lost-update pattern must be reported as an anomaly");
        let involved: std::collections::HashSet<&str> = anomalies.iter().flat_map(|a| a.cycle.iter().map(String::as_str)).collect();
        assert!(involved.contains("a") && involved.contains("b"), "both transacts must appear in the reported cycle: {anomalies:?}");
    }

    /// `record_and_check` must actually rotate the shared checker once
    /// `MAX_TRACKED_OPS` is reached -- the real fix this session's
    /// `examples/isolation_demo/RESULTS.md` scaling section names,
    /// pinned here so a future regression that silently drops the
    /// rotation call fails a fast unit test instead of only ever
    /// showing up as an unexplained slowdown running the real demo.
    #[test]
    fn record_and_check_rotates_the_shared_history_once_max_tracked_ops_is_reached() {
        clear_shared();
        for i in 0..(MAX_TRACKED_OPS * 2) {
            record_and_check(Some(format!("txn{i}")), format!("resource{i}"), OpKind::Write);
            assert!(tracked_ops_count() <= MAX_TRACKED_OPS, "tracked op count {} exceeded MAX_TRACKED_OPS={MAX_TRACKED_OPS} at i={i}", tracked_ops_count());
        }
    }

    /// The rotation above must never cost a same-window anomaly its own
    /// detection -- two ops close enough together to both land in the
    /// same window (the overwhelmingly common real case: a lost-update
    /// pair is typically a handful of ops apart, not hundreds) must
    /// still be caught, exactly as if no rotation existed at all.
    #[test]
    fn rotation_does_not_cost_a_same_window_anomaly_its_own_detection() {
        clear_shared();
        record_and_check(Some("a".to_string()), "balance:acct1".to_string(), OpKind::Read);
        record_and_check(Some("b".to_string()), "balance:acct1".to_string(), OpKind::Read);
        record_and_check(Some("a".to_string()), "balance:acct1".to_string(), OpKind::Write);
        record_and_check(Some("b".to_string()), "balance:acct1".to_string(), OpKind::Write);
        let anomalies = snapshot_anomalies();
        assert!(!anomalies.is_empty(), "a same-window anomaly must still be caught after the windowing fix");
    }
}
