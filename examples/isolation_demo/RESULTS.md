# The isolation checker really catches it — and really doesn't scale

Every number below is from an actual run of the actual code in this
directory, on this machine, today (2026-09-14) — none are estimated or
simulated. See `README.md` for what this demo is and `run.sh` for
exactly how each run was driven.

## Part 1: the checker catches the race, for real, three times

`race_probe_transact_checked.nir` is `examples/killer_demo/race_probe.
nir`'s own naive transfer (same read/sleep/write shape, same 2ms sleep,
same two-account ledger), wrapped in a real `transact { ... }` block so
`isolation_check.rs` actually sees the operations (see that file's own
top comment for why the wrapping has to be exactly where it is). 8
real OS threads, **3 transfers each — 24 total**, not `race_probe.nir`'s
own 250; Part 2 below is the measured reason why.

| | run 1 | run 2 | run 3 |
|---|---:|---:|---:|
| Final ledger total (expected 2,000,000) | 2,000,004 | 2,000,000 | 2,000,004 |
| Ledger drift | **+4** | **0** | **+4** |
| Anomaly escalations the listener received | **407** | **436** | **274** |

**Every run escalated real anomalies over a real HTTP POST to a real
local listener** (`listener.py`, driven by `run.sh`, using the exact
`NIRDOSHA_OBSERVABILITY_URL` mechanism `nfr.rs` already uses for NFR
escalation — `isolation_check.rs`'s `escalate()` is the second, not
first, caller of `nfr.rs`'s shared `post_json_fire_and_forget`). A
sample of what actually arrived (run 1, first two lines of 407):

```json
{"kind":"isolation_anomaly","cycle":["txn-1dac01-18d5401a3aac6f28-4","txn-1dac01-18d5401a3ab0c3d9-6","txn-1dac01-18d5401a3aac6f28-4"],"timestamp_ms":1789406908353}
{"kind":"isolation_anomaly","cycle":["txn-1dac01-18d5401a3aac1739-3","txn-1dac01-18d5401a3ab0c3d9-6","txn-1dac01-18d5401a3aac1739-3"],"timestamp_ms":1789406908360}
```

Real `txn_id`s (`nir_transact_gen_txn_id`'s own output, not test
fixtures), reported in real cycles — some as small as the textbook
two-txn lost-update pair, some, later in a run, growing to 15+ distinct
txns as more concurrent transfers pile onto the same two accounts'
conflict graph (see Part 2 for exactly why that matters).

**Run 2 is the more interesting result of the three, not the least
interesting.** Its ledger drift is 0 — this particular run's races
happened to cancel out numerically, the same variance `killer_demo`'s
own RESULTS.md documents (`+4, -89, -108` across its three runs). A
checker that only looked at the final total would have called run 2
clean. This one didn't: it escalated 436 real anomalies, because it
detects the anomaly from the *operation history* (Adya's WW/WR/RW
edges over the actual observed read/write order), not from whether the
arithmetic happens to come out even. That's the entire point of a
Jepsen-Elle-style detector over a black-box store, working as designed.

**407-436 escalations for 24 transfers is not a bug in the demo — it's
the checker re-reporting the same underlying, still-unresolved
corruption on every subsequent conflicting op**, since `record_and_
check` calls `check()` (a full graph rebuild) after *every* `db`
operation, over a history that's never rotated mid-run. A production
integration would want to de-duplicate or rate-limit escalations of the
same cycle; this demo intentionally reports the raw, undeduplicated
count, because the point here is "does the wiring really fire," not
"is the escalation volume production-tunable" (a smaller, disclosed
follow-on, not part of this demo's own claim).

## Part 2: it doesn't scale to the workload it exists to catch

The natural first attempt at this demo used `killer_demo`'s own literal
parameters — 8 threads, 250 transfers each. **It doesn't finish.** Not
"it's slow" — it was killed after 2m34s having printed nothing at all,
and every attempt at a full reproduction since (fresh ledger, fresh
`NIRDOSHA_TRANSACT_LOG_PATH` so no prior run's backlog could be the
cause) reproduces some version of the same thing. Ruled out along the
way, each with a real, isolated measurement:

- **Not the transact durability log.** A concurrent `transact` workload
  with a *trivial, no-`db`* `commit` slot (8 threads × 5 iterations
  each, same `nir_transact_begin`/`mark_committed` durability writes)
  finishes in **0.015s**.
- **Not the pre-existing durability-log backlog** in this checkout's
  shared `nirdosha_transact_log.sqlite` (186 old rows from unrelated
  past runs, found while investigating). Re-run with a fresh
  `NIRDOSHA_TRANSACT_LOG_PATH` — same hang.
- **Not thread count specifically.** A 2-thread version of the same
  workload hangs too, at almost exactly the same *total transfer
  count* as the 8-thread version, not at the same per-thread count —
  see the sweep below. What both sweeps agree on is the *total number
  of concurrent transacts contending on the same two accounts*.

**It is `isolation_check.rs`'s own `find_cycles`** — a DFS-based
simple-cycle enumeration (`dfs_find_cycle`, no memoization beyond
"already reported as part of an earlier cycle") run, via `record_and_
check`, on *every single `db` operation*, over the *entire, never-
rotated history* of the run. This exact workload — many concurrent
transfers racing on the same two accounts — produces about the densest
conflict graph the detector can be handed: every write to `account|1`
or `account|2` creates WW/WR/RW edges to nearly every other concurrent
transact's own ops on that same resource. `check()`'s own doc comment
already discloses `O(ops²)` per call "fine for a bounded checking
window, not meant to run over an unbounded, unrotated history" — what
this demo adds is the actual measurement of what that means in
practice, on exactly the adversarial workload this checker exists for.

### The measured cliff

2-thread sweep (same workload, thread count held at 2, only
per-thread iteration count `N` varies — total transfers `= 2N`):

| N (per thread) | Total transfers | Wall clock | Result |
|---:|---:|---:|---|
| 2 | 4 | 0.11s | completes |
| 4 | 8 | 0.21s | completes |
| 8 | 16 | 0.46s | completes |
| 10 | 20 | 1.26s | completes |
| 12 | 24 | 2.75s | completes |
| 13 | 26 | >30s | **times out** |
| 14 | 28 | >30s | **times out** |
| 16 | 32 | >30s | **times out** |
| 32 | 64 | >30s | **times out** |

8-thread sweep (this demo's own real thread count, total transfers `=
8N`):

| N (per thread) | Total transfers | Wall clock | Result |
|---:|---:|---:|---|
| 3 | 24 | 1.52s | completes |
| 4 | 32 | >30s | **times out** |
| 5 | 40 | >60s | **times out** |

Both sweeps agree: somewhere around **24-28 total concurrent
transfers** on the same two accounts, `find_cycles` stops finishing in
any practical time. Growth from N=10 (1.26s) to N=12 (2.75s) on the
2-thread sweep is already faster than linear in the added 4 transfers;
crossing from N=12 (2.75s, completes) to N=13 (>30s, doesn't) in one
step is consistent with the genuinely exponential worst case a naive
DFS-based simple-cycle enumeration has on a dense digraph — this demo
doesn't prove the exact asymptotic curve, but the cliff itself is real
and reproduced across two independent sweeps.

**This demo therefore runs at 8×3 (24 transfers), not `killer_demo`'s
own 8×250** — comfortably under the cliff (1.52s in the standalone
sweep above; the three full `run.sh` runs in Part 1 took a few seconds
each including build + listener startup). Reproducing this demo at
`killer_demo`'s literal scale is not possible today without either
windowing/rotating the checker's history (`clear_shared()` on a timer
or op-count threshold — not currently wired into the live `db.rs`
path) or replacing `find_cycles`'s algorithm with one that doesn't
degrade like this on dense graphs.

## What this does and doesn't prove

- **Does prove**: `isolation_check.rs`'s live wiring — instrumentation,
  thread-local `txn_id` attribution, cycle detection, async escalation
  over a real HTTP POST — is real and correct at a scale it can
  actually run at. Three full runs, zero synthetic data.
- **Does prove**: it detects from operation order, not final-value
  inference (run 2).
- **Does prove, as a second and unplanned finding**: the checker's own
  documented "not meant for an unbounded, unrotated history" caveat
  understates the real risk. It's not "somewhat slower over time" —
  it's a hang, reachable well before the scale the project's own
  flagship concurrency demo (`killer_demo`) already runs at today
  *without* the checker attached. A `transact`-using program that
  enables this checker (which is to say, any `transact`-using program,
  today — there's no opt-out) and then faces genuine concurrent
  contention on a hot resource is at real risk of this, not a
  contrived one.
- **Does not prove**: anything about the checker's accuracy on
  workloads that *don't* create a dense single-resource conflict graph
  — a more realistic production workload (many resources, only
  occasional contention on any one of them) was not tested here and
  would very plausibly not hit this cliff at all. This demo is
  deliberately the adversarial case, not the typical one.

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
`NIRDOSHA_OBSERVABILITY_URL` pointed at it. The scaling sweeps in Part
2 used the same binary/build with only the `spawn worker(...)`
iteration counts edited, each run given a fresh
`NIRDOSHA_TRANSACT_LOG_PATH` and a generous `timeout` (30s or 60s as
noted); a `timeout`-killed run is reported as "times out" with the
bound it was killed at, not as a real completion time.
