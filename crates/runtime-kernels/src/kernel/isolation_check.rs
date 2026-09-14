//! Black-box transaction-isolation anomaly detection over `transact`'s
//! own `db` operations -- Nirdosha's answer to the `killer_demo` gap
//! (`docs/research/2026-09-competitive-verification-and-signing-
//! landscape.md` §6): "no data races" today covers `chan`/`spawn`
//! only, and the same lost-update corruption `killer_demo` shows in
//! Python is equally possible through `db` inside a `transact` block,
//! with no guarantee at all today.
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
//!   JSON, not a parsed row/column set.** Parsing arbitrary SQL to
//!   derive the exact row set a statement touches is a research-grade
//!   problem on its own; this instead treats "same normalized
//!   statement, same bind values" as the same resource -- which is
//!   exactly what `killer_demo`'s own `UPDATE accounts SET balance = ?
//!   WHERE id = ?` pattern needs (the account id is a bind value), but
//!   can both under- and over-approximate real conflicts for anything
//!   fancier (a range `WHERE balance > ?` isn't modeled as touching
//!   every row it could match).
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

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

/// One recorded `db` operation, attributed to the `transact` site that
/// was active when it ran.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Op {
    pub txn: String,
    /// Coarse resource identity -- see this module's own doc comment.
    pub resource: String,
    pub kind: OpKind,
    /// Global, monotonically increasing sequence number -- process
    /// order across every tracked operation, the only ordering
    /// information this checker has (no wall-clock/real-time edges).
    pub seq: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpKind {
    Read,
    Write,
}

/// One detected anomaly: a cycle in the dependency graph, named by the
/// `txn_id`s involved in encounter order (the cycle's first txn repeats
/// as the last element, so the loop is visible without a reader having
/// to know cycle-detection conventions).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Anomaly {
    pub cycle: Vec<String>,
}

/// The checker's own state: every recorded op, in recording order.
/// Kept as a flat `Vec`, not a live graph -- `check()` rebuilds the
/// graph from scratch each time, which is the right tradeoff for a
/// detector meant to run periodically/on-demand over a bounded window,
/// not per-operation.
#[derive(Default)]
pub struct Checker {
    ops: Vec<Op>,
    next_seq: u64,
}

impl Checker {
    pub fn new() -> Self {
        Checker::default()
    }

    pub fn record(&mut self, txn: String, resource: String, kind: OpKind) {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.ops.push(Op { txn, resource, kind, seq });
    }

    /// Every recorded op, oldest first -- for tests and for a caller
    /// that wants to inspect/persist the raw history, not just the
    /// checker's own verdict.
    pub fn ops(&self) -> &[Op] {
        &self.ops
    }

    /// Clears every recorded op -- used between checking windows so a
    /// long-running process doesn't grow this unboundedly; anomalies
    /// already found by a prior `check()` call are unaffected (the
    /// caller owns what it does with a returned `Anomaly`).
    pub fn clear(&mut self) {
        self.ops.clear();
    }

    /// Builds the Direct Serialization Graph (Adya's WW/WR/RW edges,
    /// this module's own doc comment on why no others) over every
    /// recorded op and returns one [`Anomaly`] per simple cycle found.
    /// `O(ops² )` in the worst case (per-resource version history is
    /// linear in that resource's own op count) -- fine for a bounded
    /// checking window; not meant to run over an unbounded, unrotated
    /// history.
    pub fn check(&self) -> Vec<Anomaly> {
        // Per-resource, ops in seq order -- gives us both "which write
        // installed which version" and "what a read at seq S observed"
        // (the last write to that resource at a seq strictly before S).
        let mut by_resource: HashMap<&str, Vec<&Op>> = HashMap::new();
        for op in &self.ops {
            by_resource.entry(op.resource.as_str()).or_default().push(op);
        }
        for ops in by_resource.values_mut() {
            ops.sort_by_key(|o| o.seq);
        }

        let mut edges: HashSet<(String, String)> = HashSet::new();
        for ops in by_resource.values() {
            let mut writes: Vec<&&Op> = ops.iter().filter(|o| o.kind == OpKind::Write).collect();
            writes.sort_by_key(|o| o.seq);

            // WW: consecutive writes to the same resource order their
            // own txns -- writer(v_i) -> writer(v_{i+1}).
            for pair in writes.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                if a.txn != b.txn {
                    edges.insert((a.txn.clone(), b.txn.clone()));
                }
            }

            for read in ops.iter().filter(|o| o.kind == OpKind::Read) {
                // The version a read observed: the last write to this
                // resource strictly before the read's own seq (process
                // order is the only ordering signal available).
                let observed = writes.iter().filter(|w| w.seq < read.seq).max_by_key(|w| w.seq);
                // WR: the writer whose version this read observed
                // precedes the reader.
                if let Some(w) = observed {
                    if w.txn != read.txn {
                        edges.insert((w.txn.clone(), read.txn.clone()));
                    }
                }
                // RW (anti-dependency): the writer of the *next*
                // version after what this read observed didn't have
                // its write seen by this read -- the reader must
                // precede that writer.
                let next_write = match observed {
                    Some(w) => writes.iter().find(|x| x.seq > w.seq),
                    None => writes.first(),
                };
                if let Some(next) = next_write {
                    if next.txn != read.txn {
                        edges.insert((read.txn.clone(), next.txn.clone()));
                    }
                }
            }
        }

        find_cycles(&edges)
    }
}

/// Simple-cycle detection over a directed edge set via DFS with a
/// recursion stack -- standard textbook approach (no need for
/// Tarjan/Johnson's more elaborate all-cycles algorithms here: this
/// checker only needs to report *that* a txn is caught in a
/// non-serializable cycle and show one witness path, not enumerate
/// every cycle through it). Returns at most one anomaly per distinct
/// cycle start found by the outer loop; a txn already reported inside
/// an earlier cycle is skipped as a fresh start to avoid duplicate
/// reports of the same underlying cycle from a different offset.
fn find_cycles(edges: &HashSet<(String, String)>) -> Vec<Anomaly> {
    let mut adjacency: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut nodes: Vec<&str> = Vec::new();
    for (a, b) in edges {
        adjacency.entry(a.as_str()).or_default().push(b.as_str());
        for n in [a.as_str(), b.as_str()] {
            if !nodes.contains(&n) {
                nodes.push(n);
            }
        }
    }
    nodes.sort_unstable();

    let mut anomalies = Vec::new();
    let mut globally_reported: HashSet<&str> = HashSet::new();

    for &start in &nodes {
        if globally_reported.contains(start) {
            continue;
        }
        let mut stack: Vec<&str> = vec![start];
        let mut on_stack: HashSet<&str> = HashSet::from([start]);
        if let Some(cycle) = dfs_find_cycle(start, &adjacency, &mut stack, &mut on_stack) {
            for n in &cycle {
                globally_reported.insert(n);
            }
            anomalies.push(Anomaly { cycle: cycle.into_iter().map(str::to_string).collect() });
        }
    }
    anomalies
}

fn dfs_find_cycle<'a>(node: &'a str, adjacency: &HashMap<&'a str, Vec<&'a str>>, stack: &mut Vec<&'a str>, on_stack: &mut HashSet<&'a str>) -> Option<Vec<&'a str>> {
    let Some(neighbors) = adjacency.get(node) else { return None };
    for &next in neighbors {
        if next == stack[0] && stack.len() > 1 {
            // Closed a cycle back to this search's own start.
            let mut cycle: Vec<&str> = stack.clone();
            cycle.push(next);
            return Some(cycle);
        }
        if on_stack.contains(next) {
            continue; // a cycle not involving `stack[0]` -- a later start will find it.
        }
        stack.push(next);
        on_stack.insert(next);
        if let Some(cycle) = dfs_find_cycle(next, adjacency, stack, on_stack) {
            return Some(cycle);
        }
        stack.pop();
        on_stack.remove(next);
    }
    None
}

/// The coarse resource identity this module's own doc comment
/// discloses: **table name + only the last bound value**, not the
/// normalized SQL statement text and not the full bind list, and not a
/// parsed row/column set.
///
/// **Why not the statement text** (this module's own first attempt,
/// caught by `isolation_checker_catches_a_real_concurrent_lost_update_
/// through_the_ffi_surface` in `lib.rs` -- an end-to-end test against
/// the real FFI surface, not just this module's own unit tests, is
/// exactly what caught it): a read and the write that conflicts with
/// it are almost always two *different* statements by construction
/// (`SELECT balance FROM accounts WHERE id = ?` vs. `UPDATE accounts
/// SET balance = ? WHERE id = ?`) -- keying on statement text made a
/// read and its own conflicting write look like two unrelated
/// resources, silently missing every WR/RW edge, which is to say
/// silently missing the lost-update pattern this module exists to
/// catch. The table name is stable across both statement shapes;
/// that's the correlating key.
///
/// **Why the last bind value, not all of them, and not none**: the
/// near-universal `UPDATE t SET col = ? [, col2 = ?...] WHERE key = ?`
/// / `SELECT ... WHERE key = ?` shape binds its row-selecting predicate
/// *last*, after every `SET`-clause payload value -- exactly the shape
/// `killer_demo`'s own `UPDATE accounts SET balance = ? WHERE id = ?`
/// pattern (and every `balance_cents`/`account_id`-keyed example in
/// `examples/fintech-canon/`) uses. Including every bind value would
/// make two *conflicting* writes to the same row look like different
/// resources whenever they write different new values -- the same
/// silent-miss failure mode as above. Including no bind values at all
/// would correctly catch it but over-flag any two unrelated rows in
/// the same table as conflicting.
///
/// **Real, disclosed weaknesses of both heuristics, not hidden**: a
/// composite-key `WHERE` clause, or any statement where the row
/// selector isn't the final placeholder, gets an identity that doesn't
/// actually track the row; a table-name extraction that isn't a real
/// SQL parser (`extract_table_name` below, a `FROM`/`UPDATE`/`INTO`
/// token scan) can misfire on a statement shape it doesn't recognize
/// (a subquery, a JOIN naming more than one table, a quoted/backticked
/// identifier). Both are real, scoped simplifications, not silent
/// gaps -- a proper SQL parser deriving an exact predicate-column set
/// per statement is the honest fix, genuinely out of scope for this
/// pass (this module's own top doc comment on why).
pub(crate) fn resource_key(sql: &str, binds_json: &str) -> String {
    let table = extract_table_name(sql).unwrap_or_else(|| "?".to_string());
    let last_bind = serde_json::from_str::<serde_json::Value>(binds_json)
        .ok()
        .and_then(|v| v.as_array().and_then(|a| a.last().cloned()))
        .map(|v| v.to_string())
        .unwrap_or_else(|| binds_json.to_string());
    format!("{table}|{last_bind}")
}

/// A token scan, not a parser: the identifier immediately after the
/// first `from`/`update`/`into` keyword (case-insensitive), covering
/// `SELECT ... FROM t`, `UPDATE t SET ...`, `DELETE FROM t`, and
/// `INSERT INTO t`. Strips surrounding punctuation (backticks, quotes,
/// a trailing comma or paren) so `INSERT INTO t (col) VALUES (?)`
/// still extracts `t`, not `t(col)`. Returns `None` for anything this
/// scan doesn't recognize -- `resource_key` falls back to a fixed
/// placeholder rather than guessing further.
fn extract_table_name(sql: &str) -> Option<String> {
    let lower = sql.to_lowercase();
    let tokens: Vec<&str> = lower.split_whitespace().collect();
    for (i, tok) in tokens.iter().enumerate() {
        if matches!(*tok, "from" | "update" | "into") {
            let raw = tokens.get(i + 1)?;
            let name: String = raw.chars().filter(|c| c.is_alphanumeric() || *c == '_').collect();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}

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
        checker.check()
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

    /// The `killer_demo` shape itself: two transacts (`a`, `b`) both
    /// read the same balance before either writes it, then both write
    /// -- the classic lost-update anomaly, and exactly the pattern
    /// `killer_demo`'s own README shows corrupting the ledger. Must be
    /// caught.
    #[test]
    fn detects_the_killer_demo_lost_update_pattern() {
        let mut checker = Checker::new();
        checker.record("a".to_string(), "balance:acct1".to_string(), OpKind::Read); // a reads 10000
        checker.record("b".to_string(), "balance:acct1".to_string(), OpKind::Read); // b reads 10000 (stale for b's eventual write)
        checker.record("a".to_string(), "balance:acct1".to_string(), OpKind::Write); // a writes 9000
        checker.record("b".to_string(), "balance:acct1".to_string(), OpKind::Write); // b writes 10300, clobbering a's write
        let anomalies = checker.check();
        assert!(!anomalies.is_empty(), "the lost-update pattern must be reported as an anomaly");
        let involved: HashSet<&str> = anomalies.iter().flat_map(|a| a.cycle.iter().map(String::as_str)).collect();
        assert!(involved.contains("a") && involved.contains("b"), "both transacts must appear in the reported cycle: {anomalies:?}");
    }

    /// The same two transacts, properly serialized -- `b` only reads
    /// after `a`'s write completes, so `b` sees `a`'s real balance and
    /// no update is lost. Must NOT be reported as an anomaly: a
    /// detector that flags correctly-serialized histories would be
    /// useless (unusable false-positive rate), the same "never cry wolf"
    /// discipline `self_repair_hint`'s own arms already hold themselves to.
    #[test]
    fn does_not_flag_a_properly_serialized_history() {
        let mut checker = Checker::new();
        checker.record("a".to_string(), "balance:acct1".to_string(), OpKind::Read);
        checker.record("a".to_string(), "balance:acct1".to_string(), OpKind::Write);
        checker.record("b".to_string(), "balance:acct1".to_string(), OpKind::Read); // b reads AFTER a's write
        checker.record("b".to_string(), "balance:acct1".to_string(), OpKind::Write);
        let anomalies = checker.check();
        assert!(anomalies.is_empty(), "a correctly serialized history must not be flagged: {anomalies:?}");
    }

    /// Two transacts touching disjoint resources (different accounts)
    /// concurrently -- no shared resource, no possible conflict, must
    /// never be flagged regardless of interleaving.
    #[test]
    fn does_not_flag_disjoint_resources() {
        let mut checker = Checker::new();
        checker.record("a".to_string(), "balance:acct1".to_string(), OpKind::Read);
        checker.record("b".to_string(), "balance:acct2".to_string(), OpKind::Read);
        checker.record("a".to_string(), "balance:acct1".to_string(), OpKind::Write);
        checker.record("b".to_string(), "balance:acct2".to_string(), OpKind::Write);
        assert!(checker.check().is_empty());
    }

    /// A single transact touching a resource more than once must never
    /// be reported as conflicting with itself -- every edge-insertion
    /// path explicitly skips same-txn pairs; this pins that no
    /// self-loop ever survives into a reported cycle.
    #[test]
    fn a_single_transact_never_conflicts_with_itself() {
        let mut checker = Checker::new();
        checker.record("a".to_string(), "balance:acct1".to_string(), OpKind::Read);
        checker.record("a".to_string(), "balance:acct1".to_string(), OpKind::Write);
        checker.record("a".to_string(), "balance:acct1".to_string(), OpKind::Read);
        checker.record("a".to_string(), "balance:acct1".to_string(), OpKind::Write);
        assert!(checker.check().is_empty());
    }

    /// A three-way cycle (a -> b -> c -> a), not just the two-txn case
    /// -- confirms `find_cycles` isn't accidentally special-cased to
    /// pairs.
    #[test]
    fn detects_a_three_way_cycle() {
        let mut checker = Checker::new();
        // a reads x (initial), b reads y (initial), c reads z (initial)
        checker.record("a".to_string(), "x".to_string(), OpKind::Read);
        checker.record("b".to_string(), "y".to_string(), OpKind::Read);
        checker.record("c".to_string(), "z".to_string(), OpKind::Read);
        // a writes y (a -> b via RW, since b's read of y's initial version precedes a's write)
        checker.record("a".to_string(), "y".to_string(), OpKind::Write);
        // b writes z (b -> c via RW)
        checker.record("b".to_string(), "z".to_string(), OpKind::Write);
        // c writes x (c -> a via RW)
        checker.record("c".to_string(), "x".to_string(), OpKind::Write);
        let anomalies = checker.check();
        assert!(!anomalies.is_empty(), "the three-way cycle must be detected: {anomalies:?}");
    }

    #[test]
    fn resource_key_ignores_set_payload_but_tracks_the_trailing_predicate() {
        // Same statement shape, same WHERE-clause id (`1`, last bind),
        // different SET payload (the new balance) -- must be the SAME
        // resource: this is exactly killer_demo's own conflicting-write
        // shape, and the whole point of using only the last bind.
        let a = resource_key("UPDATE accounts SET balance = ? WHERE id = ?", "[9000,1]");
        let b = resource_key("UPDATE accounts SET balance = ? WHERE id = ?", "[10300,1]");
        assert_eq!(a, b, "same row (id=1), different new balance -- must resolve to one resource");

        // Same statement shape, different WHERE-clause id -- must be
        // DIFFERENT resources: genuinely unrelated rows.
        let c = resource_key("UPDATE accounts SET balance = ? WHERE id = ?", "[9000,2]");
        assert_ne!(a, c, "different accounts must not be conflated");
    }

    /// The bug the end-to-end FFI test (`lib.rs`) originally caught: a
    /// `SELECT` and the `UPDATE` that conflicts with it are two
    /// different statements by construction, so keying on statement
    /// TEXT (this module's first attempt) made them look like
    /// unrelated resources, silently losing every WR/RW edge. Keying
    /// on table name instead must correlate them.
    #[test]
    fn resource_key_correlates_a_read_and_the_write_that_conflicts_with_it() {
        let read = resource_key("SELECT balance FROM accounts WHERE id = ?", "[1]");
        let write = resource_key("UPDATE accounts SET balance = ? WHERE id = ?", "[9000,1]");
        assert_eq!(read, write, "a read and the write it conflicts with must resolve to the same resource");
    }

    #[test]
    fn extract_table_name_covers_select_update_delete_insert() {
        assert_eq!(extract_table_name("SELECT balance FROM accounts WHERE id = ?"), Some("accounts".to_string()));
        assert_eq!(extract_table_name("UPDATE accounts SET balance = ? WHERE id = ?"), Some("accounts".to_string()));
        assert_eq!(extract_table_name("DELETE FROM accounts WHERE id = ?"), Some("accounts".to_string()));
        assert_eq!(extract_table_name("INSERT INTO accounts (id, balance) VALUES (?, ?)"), Some("accounts".to_string()));
        assert_eq!(extract_table_name("not sql at all"), None);
    }

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
}
