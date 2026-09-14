# The isolation checker, on the exact race killer_demo already proved

`examples/killer_demo/RESULTS.md` proved something Nirdosha doesn't
prevent today: a naive, unsynchronized `db` transfer, raced through
Nirdosha's own `spawn`/`thread`, corrupts a ledger exactly like the
Python twin does. `isolation_check.rs` (`crates/runtime-kernels/src/
kernel/isolation_check.rs`) is this project's answer to that specific
gap — a black-box transaction-isolation anomaly *detector*, built on
the same technique Jepsen's Elle uses (a Direct Serialization Graph
over observed reads/writes, anomalies reported as cycles in it), scoped
to whatever runs inside a `transact { ... }` site. Until this directory
existed, that detector had unit tests and one synthetic FFI-level test
— no demo showing it catch a real corrupting run of a real compiled
program, the way every other guarantee this project claims already
does (`examples/attack_demo/`, `examples/killer_demo/`,
`examples/fintech-canon/`).

This is that demo — Phase 3 item 1 of
[`docs/research/2026-09-pending-verification-differentiation-work.md`](../../docs/research/2026-09-pending-verification-differentiation-work.md).
**What actually came out is not just "yes, it catches it"** (it does —
see below), but a second, unplanned finding of real, independent value:
running the checker at `killer_demo`'s own advertised scale doesn't
just cost some overhead, it makes the program hang. That's disclosed
and measured here too, not hidden.

## The two halves of the result

1. **The checker catches the race, live, through the real compiled
   binary's own async escalation path** — not a Rust unit test, not a
   synthetic history fed to `Checker` directly. `race_probe_transact_
   checked.nir` is `killer_demo`'s own naive transfer, wrapped in a real
   `transact { ... }` block (see that file's own top comment for why
   the wrapping is load-bearing, not cosmetic: the checker only sees
   `db` ops while a `transact` site is active, and that window only
   opens for the `commit`/`compensate` slot, not `network`/`verify`).
   `run.sh` builds it, points `NIRDOSHA_OBSERVABILITY_URL` at a real
   local listener (`listener.py`), runs it, and reports what the
   listener actually received on the wire.
2. **The checker itself doesn't scale to the workload it exists to
   catch.** `find_cycles`'s DFS-based simple-cycle enumeration, run
   against the dense conflict graph this exact race produces (every
   racing transfer on the same two accounts conflicts with nearly every
   other one), blows up well before `killer_demo`'s own 8×250 scale —
   see RESULTS.md for the measured cliff. This demo runs at 8×3 (24
   transfers), comfortably under it, not 8×250, and says so in both
   this file and `race_probe_transact_checked.nir`'s own comments.

Full numbers, three real runs, and the scaling measurements that pin
down the cliff: **[`RESULTS.md`](./RESULTS.md)**.

## Reproduce it yourself

```sh
# from the repo root
./examples/isolation_demo/run.sh
```

Needs `clang`/LLVM on `PATH` (same as any `nirdosha build`) and
`python3`. It builds the binary, starts `listener.py` on
`127.0.0.1:8073`, runs the probe against a fresh ledger and a fresh
transact durability log (`NIRDOSHA_TRANSACT_LOG_PATH`, so a prior run's
backlog never leaks into this one), and prints both the ledger drift
and the count/content of anomaly escalations the listener actually
received.

## What this does and doesn't prove

- **Does prove**: `isolation_check.rs`'s wiring is real end to end — a
  genuine concurrent lost-update, produced by the real compiled binary
  racing on a real SQLite file, is detected and escalated over a real
  HTTP POST to a real listener, using nothing but the `NIRDOSHA_
  OBSERVABILITY_URL` mechanism every other kernel escalation
  (`nfr.rs`) already uses. This is real, not simulated — every number
  in RESULTS.md is from an actual run.
- **Does prove**: it detects the anomaly from the *operation history*,
  not by inferring from the final ledger total — RESULTS.md's run 2
  shows a real run where the checker escalated hundreds of real
  anomalies even though that particular run's net ledger drift happened
  to land back on zero.
- **Does not prove**: the checker is usable at production scale today.
  It measurably isn't, past a low double-digit count of concurrent
  transacts contending on the same resource — see RESULTS.md's scaling
  section for the actual cliff and what's really going on
  (`find_cycles`'s own worst-case behavior, not a vague "it's slow").
- **Does not prove**: `evidence_tier` claims anything beyond
  `"monitored"` (`isolation_check.rs`'s own doc comment) — a detected
  cycle is conclusive evidence something already went wrong; the
  absence of one is never a soundness guarantee.
