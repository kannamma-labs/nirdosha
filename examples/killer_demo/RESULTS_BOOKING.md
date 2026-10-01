# Room-booking collapse — RFC 0015 Phase A1/A2/A3, run for real

This is Phase A of `rfcs/0015-keyed-guard-external-state.md`'s
"Implementation plan" section, run to completion: does the SQL-level
collapse claimed in that RFC's Motivation section actually hold under
real concurrent `spawn` load, the same way `race_probe_atomic.nir`/
`RESULTS.md` empirically verify the balance-transfer collapse instead of
just arguing for it in prose — for the SQLite single-statement rewrite
(A1), a real Postgres exclusion constraint (A2), and a real Postgres
advisory lock (A3)?

Every number below is from an actual run of the actual code in this
directory, or an actual `psql`/Rust-source check — none are estimated or
simulated.

## A1 — SQLite: `INSERT ... SELECT ... WHERE NOT EXISTS`

## The two probes

- [`race_probe_booking.nir`](./race_probe_booking.nir) — the collapse:
  a single `INSERT INTO booking (...) SELECT ... WHERE NOT EXISTS
  (SELECT 1 FROM booking WHERE room_id = ? AND NOT (end_ts <= ? OR
  start_ts >= ?))`. The overlap check and the insert are one SQL
  statement, no gap between them.
- [`race_probe_booking_naive.nir`](./race_probe_booking_naive.nir) —
  the control: a separate overlap-check `SELECT`, a `sleep_ms(2)` gap
  (modeling a real round trip), then an unconditional `INSERT` — the
  shape the RFC's Motivation section describes in prose before
  introducing the collapse. This file exists only to prove the harness
  actually detects a race when one exists, the same role
  `race_probe.nir` plays opposite `race_probe_atomic.nir`.

Both run the identical workload: 8 real OS threads (`spawn`), each
racing every other thread for the same 250 disjoint 10-unit time slots
on room `1` (`[0,10), [10,20), ..., [2490,2500)`), attempted in the same
order by every thread so each slot is genuinely contested by up to 8
concurrent attempts, not tried once each. If no double-booking ever
happens, exactly one attempt per slot succeeds: 250 successful bookings
total, 250 rows in the table, and zero overlapping pairs among them.

## Result: the collapse holds, 3/3. The naive control races, 3/3.

| | `race_probe_booking.nir` (collapsed) | `race_probe_booking_naive.nir` (control) |
|---|---:|---:|
| Runs | 3 | 3 |
| Successful bookings (expect 250) | `250, 250, 250` | `597, 709, 569` |
| Final row count (expect 250) | `250, 250, 250` | `597, 709, 569` |
| Overlapping pairs (expect 0) | `0, 0, 0` | `664, 974, 603` |
| Runs with any double-booking | **0 / 3** | **3 / 3** |
| Wall-clock time | ~3.4–3.6 s | (not separately timed — not the point of this probe) |

**The collapsed version never double-booked, in any of 3 runs, across
2,000 concurrent attempts (8 threads × 250 slots) per run.** The naive
control double- and triple-booked most slots in every run — its
"successful bookings" count is 2–3x the 250 slots that actually exist,
and the overlap-pair count confirms those aren't legitimate distinct
bookings, they're the same slot won by more than one thread. This is
the textbook check-then-write race RFC 0015's original (superseded)
claim about the room-booking example was written to be immune to, and
it reproduces reliably here exactly as the naive control was built to
demonstrate — confirming the test harness would have caught it in the
collapsed version too, if it were there.

## What this does and doesn't prove

- **Does prove**: on SQLite, for this invariant, under real concurrent
  `spawn` load, `INSERT ... SELECT ... WHERE NOT EXISTS (...)` is
  atomic with respect to the check it performs — two concurrent
  executions for an overlapping range cannot both succeed. RFC 0015's
  Motivation section claim is not merely argued, it is measured.
- **Does not prove**: that this is the *only* correct SQL-level fix, or
  that every check-then-write invariant collapses this way — only that
  this one, for this schema, on this backend, does. RFC 0015's broader
  rule ("if the check-then-write invariant can be expressed as one
  conditional DML statement, `guard` is not necessary") rests on the
  general expressiveness of SQL subqueries, not on this one measurement
  alone — this run is the concrete instance of it that RFC 0015's own
  "Implementation plan" called for before treating the rule as settled.
- **Per RFC 0015's decision rule**: A1 succeeding means Phase C (the
  `guard` language feature itself) does not proceed on the strength of
  the room-booking example — regardless of how Phase A2/A3 (the
  Postgres-specific exclusion-constraint and advisory-lock checks) turn
  out.

## Methodology

```sh
nirdosha build examples/killer_demo/race_probe_booking.nir -o /tmp/race_probe_booking
nirdosha build examples/killer_demo/race_probe_booking_naive.nir -o /tmp/race_probe_booking_naive
rm -f examples/killer_demo/race_probe_booking.db examples/killer_demo/race_probe_booking_naive.db
/tmp/race_probe_booking
/tmp/race_probe_booking_naive
```

Each run resets its own `booking` table, spawns 8 threads racing for
the same 250 slots, joins all of them, then reads back the row count
and an explicit overlap-pair count. Run three times per probe; every
number reported above is from one of those six runs, unedited.

## A2 — Postgres: `EXCLUDE USING gist`, no application-level check at all

[`race_probe_booking_pg_exclude.nir`](./race_probe_booking_pg_exclude.nir)
against a real local Postgres (`docker-compose.dev.yml`, this repo's own
dev convention: `docker compose -f docker-compose.dev.yml up -d`).
Schema:

```sql
CREATE EXTENSION IF NOT EXISTS btree_gist;
CREATE TABLE booking_pg_exclude (id SERIAL PRIMARY KEY, room_id INTEGER, during INT4RANGE);
ALTER TABLE booking_pg_exclude ADD CONSTRAINT booking_pg_exclude_no_overlap
  EXCLUDE USING gist (room_id WITH =, during WITH &&);
```

`try_book_inner` does **no overlap check of its own** — it just issues
`INSERT INTO booking_pg_exclude (room_id, during) VALUES (?, int4range(?, ?, '[)'))`
and lets Postgres accept or reject. Same workload as A1: 8 threads,
250 contested slots on room `1`.

| | Run 1 | Run 2 | Run 3 |
|---|---:|---:|---:|
| Successful bookings (expect 250) | 250 | 250 | 250 |
| Final row count (expect 250) | 250 | 250 | 250 |
| Overlapping pairs (expect 0) | 0 | 0 | 0 |

**3/3, zero overlaps, with zero application-level logic.** `.nir`'s
`db_execute` can issue `ALTER TABLE ... ADD CONSTRAINT ... EXCLUDE USING
gist` against a real Postgres server (the DDL is a plain literal string,
same as every `CREATE TABLE` this repo's other examples already send),
and the constraint alone — no `guard`, no lock, no hand-written check —
is a complete, stronger answer than the SQLite collapse: the invariant
lives in the schema, not in any call site's code, so it holds even
against a caller who forgets to check anything. **A2 confirms the
Postgres-specific schema-level answer RFC 0015 named but never ran.**

## A3 — Postgres: `pg_advisory_xact_lock`, and a real kernel bug found along the way

**Reachability: confirmed.** `SELECT pg_advisory_xact_lock(1, ?)` is
callable through `db_query` on a `db` connection handle, and the lock it
takes is genuinely released at that connection's `COMMIT` — verified
with a single-threaded, sequential 5-cycle smoke test (`BEGIN` /
`pg_advisory_xact_lock` / a query / `COMMIT`, five times over, reusing
the connection pool each time): 5/5 cycles completed cleanly, no error,
no stall.

**Concurrent correctness: blocked by a real, reproducible kernel bug —
not a defect in the advisory-lock approach itself.** Running the same
8-thread/250-slot workload as A1/A2, wrapped in `BEGIN` /
`pg_advisory_xact_lock(1, room_id)` / overlap-check / `INSERT` /
`COMMIT` on one held connection
([`race_probe_booking_advisory.nir`](./race_probe_booking_advisory.nir)),
deadlocks as soon as two threads genuinely contend for the same lock —
reproduced consistently across multiple attempts, isolated down to a
**2-thread, 3-iteration** case. `pg_stat_activity` at the point of the
hang shows the *winning* thread's backend `idle in transaction`, its
last query already `pg_advisory_xact_lock(...)` having returned — it
simply never issues its next statement — while the *losing* thread's
backend sits `active`, correctly waiting on that same lock. A follow-up
test replacing the advisory lock with an ordinary `SELECT ... FOR UPDATE`
row lock reproduces the identical stall, which rules out anything
specific to advisory locks: **any two `.nir` threads where one is
genuinely blocked server-side waiting on a lock the other holds can
deadlock the whole process**, including threads with no relation to
each other.

**Root cause, found by reading the source, not guessed at**:
`crates/runtime-kernels/src/kernel/mod.rs`'s `HandleTable<T>::with`
(the generic handle-dispatch primitive `rfcs/0007-apm-runtime-kernel.md`
describes as shared, unwired infrastructure "for the next resource
domain... to use instead of inventing its own table"):

```rust
pub fn with<R>(&self, id: i64, f: impl FnOnce(&mut T) -> R) -> Option<R> {
    self.handles.lock().unwrap().get_mut(&id).map(f)
}
```

`self.handles` is `Mutex<HashMap<i64, T>>` — one process-wide mutex,
shared by **every** live handle of that type. `db_table()`
(`crates/runtime-kernels/src/lib.rs:2194`) is one `&'static
HandleTable<DbConn>` for **every** `db` connection in the process,
SQLite and Postgres alike, across every thread. `nir_db_execute`/
`nir_db_query` run the actual blocking network call (`pg.execute(...)`/
`pg.query(...)`) **inside** the closure passed to `.with(...)` — so the
global table mutex is held for the entire duration of every blocking
database call, not just for the handle lookup. Two threads whose
Postgres-side operations have any ordering dependency (one waiting on a
lock the other holds) deadlock immediately: the blocked thread holds the
global mutex for as long as it's blocked, so the thread that would
release the lock it's waiting on can't even acquire the mutex to issue
its own next statement. This is not particular to advisory locks or to
this probe — it is a property of `HandleTable::with`'s shape plus how
`nir_db_execute`/`nir_db_query` use it, and it would affect any two
concurrent `.nir` `db` operations with a real server-side wait between
them (a lock, a long query blocking another connection, etc.), on either
backend, today.

**Relevance to RFC 0015 specifically**: "Implementation plan" Phase C
(not currently authorized — see that RFC) proposed building `guard`'s
own keyed lease entries on this exact `HandleTable<T>`. Built naively on
`.with`'s current shape, a `guard` acquire that blocks waiting for
another thread to release — the normal, expected case under contention
— would hold `HandleTable`'s shared dispatch mutex for the whole wait,
stalling every other handle of that type (every other `db` connection,
if lease entries shared `db_table`'s own table; every other lease, if
they got a dedicated table of the same shape) process-wide. RFC 0015's
own lease-table design (`GuardShard`, sharded, with a per-key `Condvar`
and no blocking call ever made while holding the shard mutex) does not
have this defect — it was designed independently and happens to avoid
exactly this trap — but reusing `HandleTable<T>` verbatim, as Phase C
suggested, would reintroduce it. Worth fixing (`HandleTable::with`
should drop the table lock before running `f`, or `f` should never
itself block) before `HandleTable` is reused for anything with a
blocking, contended acquire path — independent of whether RFC 0015's
Phase C ever proceeds. Filed:
[kannamma-labs/nirdosha#37](https://github.com/kannamma-labs/nirdosha/issues/37).

**This does not change A3's or A1's standing** in RFC 0015's decision
rule: A1 alone already settles that Phase C does not proceed, and
nothing here contradicts the reachability of `pg_advisory_xact_lock`
found by the sequential test above. What A3 could not do is complete its
own concurrent-correctness check, because the mechanism used to run the
check hit an unrelated, pre-existing bug first.

## Methodology (A2/A3)

```sh
docker compose -f docker-compose.dev.yml up -d
export NIRDOSHA_TEST_POSTGRES_URL=postgres://nirdosha:nirdosha@localhost:5432/nirdosha_dev

nirdosha build examples/killer_demo/race_probe_booking_pg_exclude.nir -o /tmp/race_probe_booking_pg_exclude
/tmp/race_probe_booking_pg_exclude   # A2 — run 3x

nirdosha build examples/killer_demo/race_probe_booking_advisory.nir -o /tmp/race_probe_booking_advisory
/tmp/race_probe_booking_advisory     # A3 — hangs under real contention; see above
```

The A3 sequential smoke test and the 2-thread/`SELECT ... FOR UPDATE`
isolation test that pinned down the root cause were ad hoc diagnostic
`.nir` files (not checked in — their only purpose was narrowing down
the failure, and `HandleTable::with`'s source above is the actual
evidence for the root-cause claim, not the diagnostic files themselves).
