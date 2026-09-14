# The isolation checker, on the exact race killer_demo already proved

`examples/killer_demo/RESULTS.md` proved something Nirdosha doesn't
prevent today: a naive, unsynchronized `db` transfer, raced through
Nirdosha's own `spawn`/`thread`, corrupts a ledger exactly like the
Python twin does. `isolation_check.rs` (`crates/runtime-kernels/src/
kernel/isolation_check.rs`, the live half; the actual detection
algorithm now lives in `crates/isolation-core`) is this project's
answer to that specific gap — a black-box transaction-isolation anomaly
*detector*, built on the same technique Jepsen's Elle uses (a Direct
Serialization Graph over observed reads/writes, anomalies reported as
cycles in it), scoped to whatever runs inside a `transact { ... }`
site. Until this directory existed, that detector had unit tests and
one synthetic FFI-level test — no demo showing it catch a real
corrupting run of a real compiled program, the way every other
guarantee this project claims already does (`examples/attack_demo/`,
`examples/killer_demo/`, `examples/fintech-canon/`).

This is that demo — Phase 3 item 1 of
[`docs/research/2026-09-pending-verification-differentiation-work.md`](../../docs/research/2026-09-pending-verification-differentiation-work.md).
**What actually came out is not just "yes, it catches it"** (it does —
see below), but a second, unplanned finding of real, independent value:
running the checker at `killer_demo`'s own advertised scale didn't just
cost some overhead, it made the program hang -- for two real, separate
reasons, both found, root-caused, and fixed the same session (one more
the next day). That's disclosed and measured here too, not hidden.

## The two halves of the result

1. **The checker catches the race, live, through the real compiled
   binary's own async escalation path, at `killer_demo`'s own full
   8×250 (2,000-transfer) scale** — not a Rust unit test, not a
   synthetic history fed to `Checker` directly, and not a scaled-down
   stand-in either. `race_probe_transact_checked.nir` is `killer_demo`'s
   own naive transfer, wrapped in a real `transact { ... }` block (see
   that file's own top comment for why the wrapping is load-bearing,
   not cosmetic: the checker only sees `db` ops while a `transact` site
   is active, and that window only opens for the `commit`/`compensate`
   slot, not `network`/`verify`). `run.sh` builds it, points
   `NIRDOSHA_OBSERVABILITY_URL` at a real local listener
   (`listener.py`), runs it, and reports what the listener actually
   received on the wire.
2. **The checker itself didn't scale to the workload it exists to
   catch — two real bugs, both fixed.** `find_cycles`'s DFS-based
   simple-cycle enumeration blew up exponentially on the dense conflict
   graph this exact race produces (rewritten as plain graph
   reachability, now strictly `O(V+E)`); separately, the checker's
   history was never rotated, so its still-real `O(ops²)`-ish per-call
   cost grew without bound over a long run (fixed with `MAX_TRACKED_
   OPS`-based windowing). With only the first fix, the full 2,000-
   transfer scale still didn't finish in 15 minutes; with both, it
   finishes in ~23 seconds. RESULTS.md has every measurement for both
   bugs, before and after.

Full numbers, three real runs at full scale, and the complete before/
after story for both fixes: **[`RESULTS.md`](./RESULTS.md)**.

## Reproduce it yourself

```sh
# from the repo root
./examples/isolation_demo/run.sh
```

Needs `clang`/LLVM on `PATH` (same as any `nirdosha build`) and
`python3`. It builds the binary, starts `listener.py` on
`127.0.0.1:8073`, runs the probe (8 threads × 250 transfers, ~23s)
against a fresh ledger and a fresh transact durability log
(`NIRDOSHA_TRANSACT_LOG_PATH`, so a prior run's backlog never leaks
into this one), and prints both the ledger drift and the count/content
of anomaly escalations the listener actually received.

## What this does and doesn't prove

- **Does prove**: `isolation_check.rs`'s wiring is real end to end, at
  real scale — a genuine concurrent lost-update, produced by the real
  compiled binary racing on a real SQLite file across 2,000 concurrent
  transfers, is detected and escalated over a real HTTP POST to a real
  listener, using nothing but the `NIRDOSHA_OBSERVABILITY_URL`
  mechanism every other kernel escalation (`nfr.rs`) already uses. This
  is real, not simulated — every number in RESULTS.md is from an actual
  run.
- **Does prove**: it detects the anomaly from the *operation history*,
  not by inferring from the final ledger total.
- **Does prove**: both scaling fixes are real and durable, not just
  claimed once — `crates/isolation-core`'s own unit tests
  (`dense_conflict_graph_does_not_blow_up`) and `isolation_check.rs`'s
  (`record_and_check_rotates_the_shared_history_once_max_tracked_ops_
  is_reached`, `rotation_does_not_cost_a_same_window_anomaly_its_own_
  detection`) pin both at the test level, so a regression fails CI, not
  only ever shows up as an unexplained slowdown running this demo again.
- **Does not prove**: the checker is unboundedly scalable at any real
  contention level. `MAX_TRACKED_OPS`'s windowing trades recall (a
  cross-window anomaly is invisible) for a bounded cost — a real,
  disclosed tradeoff, not a claim of a perfect fix. See RESULTS.md's
  own "what this does and doesn't prove" for the full scope.
- **Does not prove**: `evidence_tier` claims anything beyond
  `"monitored"` (`isolation_check.rs`'s own doc comment) — a detected
  cycle is conclusive evidence something already went wrong; the
  absence of one is never a soundness guarantee.
