# The isolation checker really catches it — and, after two real fixes, really scales too

Every number below is from an actual run of the actual code in this
directory, on this machine — none are estimated or simulated. See
`README.md` for what this demo is and `run.sh` for exactly how each run
was driven. Dated in two parts: 2026-09-14 (the demo built, the
scaling bug found and root-caused, both fixes designed and landed) and
2026-09-15 (this file rewritten once the demo could finally run at its
intended full scale).

## Part 1: the checker catches the race, for real, three times, at full scale

`race_probe_transact_checked.nir` is `examples/killer_demo/race_probe.
nir`'s own naive transfer (same read/sleep/write shape, same 2ms sleep,
same two-account ledger), wrapped in a real `transact { ... }` block so
`isolation_check.rs` actually sees the operations (see that file's own
top comment for why the wrapping has to be exactly where it is). **8
real OS threads, 250 transfers each — 2,000 total, matching
`race_probe.nir`'s own scale exactly** (an earlier draft of this file
ran at 8×3 instead; see Part 2 for why, and why that's no longer
necessary).

| | run 1 | run 2 | run 3 |
|---|---:|---:|---:|
| Final ledger total (expected 2,000,000) | 2,000,188 | 2,000,218 | 2,000,166 |
| Ledger drift | **+188** | **+218** | **+166** |
| Anomaly escalations the listener received | **24,242** | **23,863** | **24,176** |
| Wall clock (the compiled probe binary alone) | ~23s | ~23s | ~23s |

**Every run escalated real anomalies over a real HTTP POST to a real
local listener** (`listener.py`, driven by `run.sh`, using the exact
`NIRDOSHA_OBSERVABILITY_URL` mechanism `nfr.rs` already uses for NFR
escalation — `isolation_check.rs`'s `escalate()` is the second, not
first, caller of `nfr.rs`'s shared `post_json_fire_and_forget`). A
sample of what actually arrived (real `txn_id`s — `nir_transact_gen_
txn_id`'s own output, not test fixtures — reported in real cycles, some
as small as the textbook two-txn lost-update pair, some growing to 8-9
distinct txns as more concurrent transfers pile onto the same two
accounts' conflict graph):

```json
{"kind":"isolation_anomaly","cycle":["txn-267ed7-18d543ab657738f8-7c7","txn-267ed7-18d543ab6694cfbc-7c9","txn-267ed7-18d543ab657738f8-7c7"],"timestamp_ms":1789410830574}
{"kind":"isolation_anomaly","cycle":["txn-267ed7-18d543ab6088e820-7c3","txn-267ed7-18d543ab615f5be5-7c5","txn-267ed7-18d543ab61594e01-7c4","txn-267ed7-18d543ab622f15af-7c6","txn-267ed7-18d543ab6088e820-7c3"],"timestamp_ms":1789410830574}
```

**24,000+ escalations for 2,000 transfers is not a bug in the demo —
it's the checker re-reporting the same underlying, still-unresolved
corruption on every subsequent conflicting op**, since `record_and_
check` calls `check()` (a graph rebuild) after *every* `db` operation.
A production integration would want to de-duplicate or rate-limit
escalations of the same cycle; this demo intentionally reports the raw,
undeduplicated count, because the point here is "does the wiring really
fire, at real scale," not "is the escalation volume production-tunable"
(a smaller, disclosed follow-on, not part of this demo's own claim —
see the pending-work doc's disclosed-gaps list).

The checker detects from the *operation history*, not by inferring from
the final ledger total — this held in the earlier 8×3-scale runs too
(see this file's git history): a run whose net drift happened to land
on zero still escalated hundreds of real anomalies, because Adya's
WW/WR/RW edges are computed from observed read/write order, not from
whether the arithmetic happens to come out even.

## Part 2: it didn't scale, at all — two real, sequential fixes, before it did

The first attempt at this demo used `killer_demo`'s own literal
parameters from the start. **It didn't finish.** Not "it's slow" — the
first real run was killed after 2m34s having printed nothing at all.
Two separate, real bugs were responsible, found and fixed in sequence,
each confirmed with its own before/after measurement rather than
assumed fixed.

### Bug 1 (fixed): `find_cycles` was exponential on a dense conflict graph

Ruled out first, each with its own isolated measurement, before landing
on the real cause: not the transact durability log (a no-`db` transact
workload at the same thread count finished in 0.015s), not a
pre-existing durability-log backlog in this checkout (reproduced with a
fresh `NIRDOSHA_TRANSACT_LOG_PATH` too), not thread count specifically
(a 2-thread version hit the same cliff at the same *total transfer
count*, not the same per-thread count).

**The real cause**: `isolation_check.rs`'s `find_cycles` was a DFS-based
simple-cycle search that pruned only nodes already on the current
recursion stack, with no memoization of "no cycle back to start found
from here" — provably fine for a sparse graph, but this exact
workload (many concurrent transfers racing the same two accounts)
produces close to the densest conflict graph the detector can be
handed, and the search re-explored the same node's entire subtree from
every sibling branch that reached it. Measured cliff before the fix,
two independent sweeps (2-thread and 8-thread, only iteration count
varying):

| N per thread | Total transfers | Wall clock | Result |
|---:|---:|---:|---|
| 2-thread, N=12 | 24 | 2.75s | completes |
| 2-thread, N=13 | 26 | >30s | **times out** |
| 8-thread, N=3 | 24 | 1.52s | completes |
| 8-thread, N=4 | 32 | >30s | **times out** |

**The fix** (`crates/isolation-core/src/lib.rs`, extracted from
`isolation_check.rs` the same session — see below): rewrote cycle
detection as plain graph *reachability* instead of simple-path search.
"Does a cycle through `start` exist" is exactly "is `start` reachable
from one of its own successors" — reachability is monotonic (a node's
"can't reach start" verdict is a fixed fact about the graph, never
dependent on which other nodes are mid-exploration), so a plain
visited-once DFS is both correct and strictly `O(V+E)` per start node.
Confirmed with the same 2-thread sweep, extended much further:

| N per thread | Total transfers | Wall clock | Result |
|---:|---:|---:|---|
| 12 | 24 | 0.60s | completes |
| 13 | 26 | 0.66s | completes |
| 64 | 128 | 6.36s | completes |
| 128 | 256 | 9.84s | completes |
| 250 | 500 | 27.0s | completes |

The exponential wall was gone — but growth from 128→256 total (9.8s)
to 250→500 total (27s) is still clearly worse than linear (`check()`'s
own doc comment already discloses `O(ops²)` per call, called on every
op, over a never-rotated history — the algorithmic blowup was fixed,
the *lack of windowing* wasn't). Running the actual full 8×250 scale
(2,000 transfers) with only this first fix applied was tried for real,
not skipped: killed after 900s (15 minutes) still running, having
burned over 11 minutes of CPU with no sign of finishing on its own.

### Bug 2 (fixed): no windowing — history was never rotated

**The fix**: `MAX_TRACKED_OPS = 300` (`isolation_check.rs`) — once the
shared checker's history reaches this many ops, `record_and_check`
rotates it (`Checker::clear()`, which already existed as a public
function but nothing called it automatically before this). Bounds
`check()`'s own still-real `O(ops²)`-ish per-call cost to a fixed
ceiling regardless of how long the process runs, at the real, disclosed
cost of missing a WR/RW dependency whose two ops happen to land in
different windows — a false negative, never a false positive, the same
asymmetry `evidence_tier: "monitored"` already discloses for the
checker as a whole. A unit test
(`record_and_check_rotates_the_shared_history_once_max_tracked_ops_is_
reached`) pins the rotation itself; another
(`rotation_does_not_cost_a_same_window_anomaly_its_own_detection`)
confirms a same-window anomaly (the overwhelmingly common real case —
`killer_demo`'s own lost-update pair is a handful of ops apart, not
hundreds) is still caught.

**With both fixes**: the full 8×250 (2,000-transfer) scale — the same
workload that didn't finish in 15 minutes with only Bug 1's fix, and
never finished at all before either fix — completes in **~23 seconds**.
Part 1's three real runs above are all at this full scale.

### The extraction that made both fixes shareable: `crates/isolation-core`

Both fixes landed in a new crate, `crates/isolation-core`
(`nirdosha-isolation-core`), extracted out of `runtime-kernels::kernel::
isolation_check` the same session — pure graph logic (`Checker`,
`find_cycles`, `resource_key`), no FFI, no threads, no native deps. Not
a refactor for its own sake: it's what let `crates/compiler` gain a
`nirdosha check-isolation <ops.json>` CLI subcommand and a
`Certificate::isolation_violations` field (RFC 0016 Phase 4's "surface
it in the certificate over time," `mcp_tools.rs`) using the *identical*
detector the live path runs, instead of either reimplementing it or
pulling postgres/native-tls/rusqlite into the compiler tool binary.
Both new surfaces are real and tested (`crates/compiler/tests/
check_isolation_command.rs`, `certify_command.rs`'s `--isolation-log`
tests) — see `docs/research/2026-09-pending-verification-
differentiation-work.md`'s Phase 4 section for what "over time" means
there and its own real limits.

## What this does and doesn't prove

- **Does prove**: `isolation_check.rs`'s live wiring — instrumentation,
  thread-local `txn_id` attribution, cycle detection, async escalation
  over a real HTTP POST — is real and correct, now at the same scale
  `killer_demo` itself already runs at. Three full runs, zero synthetic
  data.
- **Does prove**: it detects from operation order, not final-value
  inference.
- **Does prove**: both scaling fixes are real, not just claimed — every
  before/after table above is a real measurement, and the unit tests
  in `crates/isolation-core` (`dense_conflict_graph_does_not_blow_up`)
  and `isolation_check.rs` (the two rotation tests) pin both fixes at
  the test level, not only "it worked when I tried it."
- **Does not prove**: the checker is unboundedly scalable. 300 ops is a
  measured-comfortable window, not a proof of an optimal one; a
  workload with much higher real contention on one resource than
  `killer_demo`'s own two-account shape could still find a slower edge
  of this design. Windowing trades recall (a cross-window anomaly is
  invisible) for the bound; that trade is real and disclosed, not
  hidden.
- **Does not prove**: `evidence_tier` claims anything beyond
  `"monitored"` (`isolation_check.rs`'s own doc comment) — a detected
  cycle is conclusive evidence something already went wrong; the
  absence of one is never a soundness guarantee.

## Methodology

```sh
# from the repo root
./examples/isolation_demo/run.sh
```

Each of the three Part-1 runs: fresh ledger (`isolation_probe.db`
deleted first), fresh transact durability log
(`NIRDOSHA_TRANSACT_LOG_PATH` pointed at a fresh file each time, so no
prior run's backlog rows are replayed at startup), a real
`listener.py` bound to `127.0.0.1:8073` before the probe binary starts,
`NIRDOSHA_OBSERVABILITY_URL` pointed at it. `listener.py` sets
`SO_REUSEADDR` and swallows `ConnectionResetError` on individual
requests — real, disclosed artifacts of gathering three runs back to
back at 2,000-transfer scale (thousands of fire-and-forget connections
in a tight window), neither affecting the anomaly count logged, both
found for real doing exactly this, not guessed at in advance. The
Part-2 scaling sweeps used the same binary/build with only the `spawn
worker(...)` iteration counts edited, each run given a fresh
`NIRDOSHA_TRANSACT_LOG_PATH` and a generous `timeout` (30s, 60s, or
900s as noted); a `timeout`-killed run is reported as "times out" with
the bound it was killed at, not as a real completion time.
