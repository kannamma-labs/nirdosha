//! Pure transaction-isolation anomaly detection -- a Direct
//! Serialization Graph (Adya's WW/WR/RW edges) built from an observed
//! `db` operation history, anomalies reported as cycles in it, the
//! same technique Jepsen's Elle (Kingsbury & Alvaro, VLDB'21) uses.
//! Extracted (2026-09-14) out of `runtime-kernels::kernel::
//! isolation_check` into its own dependency-light crate so it can be
//! shared by two very different consumers without either pulling in
//! the other's dependency tree: `runtime-kernels` (heavy native deps --
//! postgres, native-tls, rusqlite -- for the live, FFI-hooked path
//! every compiled `db` call inside a `transact` site actually runs
//! through) and `crates/compiler` (the `nirdosha` tool binary, which
//! needs this same algorithm for `nirdosha check-isolation` and for
//! attaching observed anomalies to a certificate, but has no business
//! linking a Postgres client to do it). This crate itself has no I/O,
//! no threads, no FFI -- just the graph.
//!
//! **Why this is a *detector*, not a *prover*, and why that's the
//! honest answer, not a lesser one.** None of the field's static
//! deductive verifiers (Dafny, Prusti, Creusot, Kani) reason about an
//! external, mutable SQL store any differently than Nirdosha does
//! today -- they verify in-memory state. The field's own practical
//! answer for transaction serializability is dynamic, a-posteriori
//! checking of an observed operation history, not a priori proof. This
//! module is that technique.
//!
//! **Real, disclosed simplifications:**
//! - **Resource identity is coarse: table name + only the last bound
//!   value, not a parsed row/column set** -- see `resource_key`'s own
//!   doc comment for the full reasoning and the real, disclosed
//!   weaknesses of that heuristic.
//! - **Adya's three core dependency edges only (WW/WR/RW) -- no
//!   process-order or real-time-order edges.** This detects
//!   serializability violations, not the stronger strict-
//!   serializability property session/real-time edges would add.
//! - **`evidence_tier: "monitored"`, never `"proved"`, wherever this
//!   reaches a certificate** (mirroring `mcp_tools::NfrCommitment`'s own
//!   discipline) -- a detected cycle is conclusive evidence of an
//!   anomaly that already happened; the *absence* of a detected cycle
//!   is never a soundness guarantee that one can't happen.

use std::collections::{HashMap, HashSet};

/// One recorded `db` operation, attributed to the `transact` site that
/// was active when it ran.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Op {
    pub txn: String,
    /// Coarse resource identity -- see `resource_key`'s own doc comment.
    pub resource: String,
    pub kind: OpKind,
    /// Global, monotonically increasing sequence number -- process
    /// order across every tracked operation, the only ordering
    /// information this checker has (no wall-clock/real-time edges).
    pub seq: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpKind {
    Read,
    Write,
}

/// One detected anomaly: a cycle in the dependency graph, named by the
/// `txn_id`s involved in encounter order (the cycle's first txn repeats
/// as the last element, so the loop is visible without a reader having
/// to know cycle-detection conventions).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

    /// Loads a pre-recorded history wholesale (`nirdosha check-
    /// isolation <log>`'s own entry point -- a saved log has real
    /// `seq` values already, not ones this checker assigned, so this
    /// bypasses `record`'s own sequencing and takes each `Op` as given;
    /// `next_seq` is advanced past the highest loaded `seq` so any
    /// further `record` calls on the same `Checker` keep ordering
    /// correctly.).
    pub fn load(&mut self, ops: Vec<Op>) {
        self.next_seq = self.next_seq.max(ops.iter().map(|o| o.seq + 1).max().unwrap_or(0));
        self.ops = ops;
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

/// Cycle *existence* detection over a directed edge set, one witness
/// path per distinct cycle found -- this checker only needs to report
/// *that* a txn is caught in a non-serializable cycle and show one
/// witness path, not enumerate every simple cycle through it (no need
/// for Tarjan/Johnson's more elaborate all-cycles algorithms here).
/// Returns at most one anomaly per distinct cycle start found by the
/// outer loop; a txn already reported inside an earlier cycle is
/// skipped as a fresh start to avoid duplicate reports of the same
/// underlying cycle from a different offset.
///
/// **Plain reachability, not an on-stack-pruned simple-path DFS --
/// deliberately, a real, measured fix, not a style choice.** "Does a
/// cycle through `start` exist" is exactly "is `start` reachable from
/// one of its own successors" -- graph reachability, which is
/// monotonic: whether node `X` can reach `start` is a fixed fact about
/// the graph alone, never dependent on which other nodes happen to be
/// mid-exploration on the current call stack. That means a plain
/// "visited once, never re-explored" DFS is both correct *and*
/// strictly `O(V+E)` per `start`. An earlier version instead searched
/// for a *simple path* back to `start` (skipping any node already on
/// the current recursion stack), which does NOT have that reachability
/// property -- a node's "leads nowhere" verdict from one path context
/// isn't valid to reuse from a different one, so nothing could be
/// safely memoized, and the search could re-explore the same node's
/// entire subtree from every sibling branch that reached it. On the
/// dense conflict graph many concurrent transacts racing one resource
/// produce (exactly this checker's own reason to exist), that
/// re-exploration measurably blew up well before any realistic
/// workload -- see `examples/isolation_demo/RESULTS.md`'s "Part 2" for
/// the actual before/after numbers (the cliff this fix removed).
pub fn find_cycles(edges: &HashSet<(String, String)>) -> Vec<Anomaly> {
    let mut adjacency: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut nodes: HashSet<&str> = HashSet::new();
    for (a, b) in edges {
        adjacency.entry(a.as_str()).or_default().push(b.as_str());
        nodes.insert(a.as_str());
        nodes.insert(b.as_str());
    }
    let mut nodes: Vec<&str> = nodes.into_iter().collect();
    nodes.sort_unstable();

    let mut anomalies = Vec::new();
    let mut globally_reported: HashSet<&str> = HashSet::new();

    for &start in &nodes {
        if globally_reported.contains(start) {
            continue;
        }
        if let Some(cycle) = dfs_find_cycle(start, &adjacency) {
            for n in &cycle {
                globally_reported.insert(n);
            }
            anomalies.push(Anomaly { cycle: cycle.into_iter().map(str::to_string).collect() });
        }
    }
    anomalies
}

/// One witness path `start -> ... -> start`, or `None` if `start` isn't
/// part of any cycle -- see `find_cycles`'s own doc comment for why
/// this is plain reachability from each of `start`'s direct successors,
/// sharing one `visited` set across all of them (a node already found
/// not to reach `start` from one successor's search genuinely can't
/// reach it from another's either -- that fact doesn't change).
fn dfs_find_cycle<'a>(start: &'a str, adjacency: &HashMap<&'a str, Vec<&'a str>>) -> Option<Vec<&'a str>> {
    let start_neighbors = adjacency.get(start)?;
    let mut visited: HashSet<&str> = HashSet::new();
    for &first in start_neighbors {
        if first == start || visited.contains(first) {
            continue; // no self-loops in practice (`record`'s own same-txn edge skip); defensive either way
        }
        let mut path: Vec<&str> = vec![start, first];
        if let Some(cycle) = reach_back_to(first, start, adjacency, &mut visited, &mut path) {
            return Some(cycle);
        }
    }
    None
}

/// Plain visited-once DFS reachability from `node` to `target`, `path`
/// accumulating a real simple path as it goes (popped back off on a
/// dead end, matching the original function's own witness-path shape).
fn reach_back_to<'a>(node: &'a str, target: &'a str, adjacency: &HashMap<&'a str, Vec<&'a str>>, visited: &mut HashSet<&'a str>, path: &mut Vec<&'a str>) -> Option<Vec<&'a str>> {
    visited.insert(node);
    let Some(neighbors) = adjacency.get(node) else { return None };
    for &next in neighbors {
        if next == target {
            let mut cycle = path.clone();
            cycle.push(target);
            return Some(cycle);
        }
        if visited.contains(next) {
            continue; // already known (for real, not assumed) not to reach `target` -- see this fn's own doc comment
        }
        path.push(next);
        if let Some(cycle) = reach_back_to(next, target, adjacency, visited, path) {
            return Some(cycle);
        }
        path.pop();
    }
    None
}

/// The coarse resource identity this module's own doc comment
/// discloses: **table name + only the last bound value**, not the
/// normalized SQL statement text and not the full bind list, and not a
/// parsed row/column set.
///
/// **Why not the statement text** (this module's own first attempt,
/// caught by an end-to-end test against the real FFI surface, not just
/// a unit test, in `runtime-kernels`): a read and the write that
/// conflicts with it are almost always two *different* statements by
/// construction (`SELECT balance FROM accounts WHERE id = ?` vs.
/// `UPDATE accounts SET balance = ? WHERE id = ?`) -- keying on
/// statement text made a read and its own conflicting write look like
/// two unrelated resources, silently missing every WR/RW edge, which
/// is to say silently missing the lost-update pattern this module
/// exists to catch. The table name is stable across both statement
/// shapes; that's the correlating key.
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
pub fn resource_key(sql: &str, binds_json: &str) -> String {
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
    /// no update is lost. Must NOT be reported as an anomaly.
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
    /// be reported as conflicting with itself.
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
        checker.record("a".to_string(), "x".to_string(), OpKind::Read);
        checker.record("b".to_string(), "y".to_string(), OpKind::Read);
        checker.record("c".to_string(), "z".to_string(), OpKind::Read);
        checker.record("a".to_string(), "y".to_string(), OpKind::Write);
        checker.record("b".to_string(), "z".to_string(), OpKind::Write);
        checker.record("c".to_string(), "x".to_string(), OpKind::Write);
        let anomalies = checker.check();
        assert!(!anomalies.is_empty(), "the three-way cycle must be detected: {anomalies:?}");
    }

    #[test]
    fn resource_key_ignores_set_payload_but_tracks_the_trailing_predicate() {
        let a = resource_key("UPDATE accounts SET balance = ? WHERE id = ?", "[9000,1]");
        let b = resource_key("UPDATE accounts SET balance = ? WHERE id = ?", "[10300,1]");
        assert_eq!(a, b, "same row (id=1), different new balance -- must resolve to one resource");

        let c = resource_key("UPDATE accounts SET balance = ? WHERE id = ?", "[9000,2]");
        assert_ne!(a, c, "different accounts must not be conflated");
    }

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

    /// The exact adversarial shape `examples/isolation_demo/RESULTS.md`
    /// measured a real hang on before the `find_cycles` rewrite: many
    /// concurrent transacts racing the same two resources, producing a
    /// dense conflict graph. This pins the fix at the unit-test level,
    /// not just "it ran fast when I tried it" -- 40 transacts (80 ops,
    /// alternating two shared resources) must both find the real
    /// anomaly *and* return well inside a generous bound, so a future
    /// change that reintroduces the exponential blowup fails CI instead
    /// of only ever being caught by someone running the full demo.
    #[test]
    fn dense_conflict_graph_does_not_blow_up() {
        let mut checker = Checker::new();
        // Every one of 40 txns reads first (interleaved across two
        // shared resources, matching real concurrent contention), then
        // every one writes -- so every read observed no write yet
        // (`observed = None`), giving each of them an RW edge to the
        // *first* writer of its own resource, while consecutive writes
        // chain WW edges -- together a densely-connected, genuinely
        // cyclic conflict graph (real cycles present, not a synthetic
        // shape with none), the same general shape `examples/
        // isolation_demo/RESULTS.md` measured the real blowup on.
        for i in 0..40 {
            let txn = format!("t{i}");
            let resource = if i % 2 == 0 { "account|1" } else { "account|2" };
            checker.record(txn, resource.to_string(), OpKind::Read);
        }
        for i in 0..40 {
            let txn = format!("t{i}");
            let resource = if i % 2 == 0 { "account|1" } else { "account|2" };
            checker.record(txn, resource.to_string(), OpKind::Write);
        }
        let start = std::time::Instant::now();
        let anomalies = checker.check();
        let elapsed = start.elapsed();
        assert!(elapsed.as_secs() < 5, "dense-graph check() took {elapsed:?} -- the exponential blowup is back");
        assert!(!anomalies.is_empty(), "this workload is a real lost-update pattern and must be flagged");
    }

    #[test]
    fn load_replaces_history_and_advances_next_seq_past_the_loaded_max() {
        let mut checker = Checker::new();
        checker.load(vec![
            Op { txn: "a".to_string(), resource: "r".to_string(), kind: OpKind::Read, seq: 0 },
            Op { txn: "b".to_string(), resource: "r".to_string(), kind: OpKind::Write, seq: 1 },
        ]);
        assert_eq!(checker.ops().len(), 2);
        checker.record("c".to_string(), "r".to_string(), OpKind::Read);
        assert_eq!(checker.ops().last().unwrap().seq, 2, "a record() after load() must continue the sequence, not restart it");
    }
}
