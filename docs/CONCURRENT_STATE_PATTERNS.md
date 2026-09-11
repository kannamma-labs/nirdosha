# Concurrent `db` state: the decision sequence

This is Phase B3 of `rfcs/0015-keyed-guard-external-state.md`'s
"Implementation plan" — guidance for a `.nir` author whose program has
multiple `spawn`ed threads touching the same rows, written down once so
it doesn't have to be re-derived per call site. `docs/LANGUAGE.md` §7
is explicit that `chan`/`spawn`'s own race-freedom guarantee does not
extend to `db`: nothing in the language stops a program from writing a
naive, unsynchronized concurrent read-then-write against a database, and
it corrupts exactly the way the same naive code corrupts in any other
language (`examples/killer_demo/RESULTS.md`, `RESULTS_BOOKING.md`).
This page is the answer to "so what do I actually do about it," in
order, cheapest first.

RFC 0015 spent nine review rounds trying to justify a new keyed-lock
language primitive (`guard(tag, key) { ... }`) for exactly this problem,
and — the useful outcome of that process — found that every example it
tried, including a deliberately-hard multi-row case, is answered by one
of the three steps below, measured, not argued. Read that RFC's
Motivation and "Implementation plan" sections for the full history; this
page is the short, forward-looking version for someone who just wants to
know what to write.

## 1. A single conditional DML statement, first

**If the check and the write can be expressed as one `INSERT`/`UPDATE`/
`DELETE` statement, do that. This closes more cases than it looks like
it should**, because a `WHERE` clause (or the `SELECT` an `INSERT ...
SELECT` reads from) can carry a subquery reaching however many other
rows or tables the invariant needs — "single statement" is not the same
constraint as "single row."

- **Single-row compare-and-swap**: `UPDATE account SET balance_cents =
  balance_cents - ? WHERE id = ? AND balance_cents >= ?` — the `WHERE`
  clause *is* the check, evaluated and applied atomically by the
  database. `examples/killer_demo/race_probe_atomic.nir`, verified 3/3
  against `race_probe.nir`'s naive twin corrupting 3/3
  (`RESULTS.md`).
- **Multi-row existence check**: `INSERT INTO booking (room_id, start,
  end) SELECT ?, ?, ? WHERE NOT EXISTS (SELECT 1 FROM booking WHERE
  room_id = ? AND <overlap predicate>)` — the existence check and the
  insert are one statement; two concurrent attempts for an overlapping
  range cannot both succeed, because whichever commits first is visible
  to the second's `NOT EXISTS` subquery. `examples/killer_demo/
  race_probe_booking.nir`, verified 3/3 against a naive
  check-then-insert control racing 3/3 (`RESULTS_BOOKING.md`).

This works on SQLite and Postgres alike, needs no schema change, and is
available today with nothing beyond ordinary `db_execute` calls
(`docs/LANGUAGE.md` §"db": `sql` is passed through verbatim to the
backend, with no restriction on subqueries within one literal).

**When this doesn't fit**: the invariant genuinely can't be reduced to a
predicate a database engine can evaluate against its own current state —
most commonly because the decision depends on something the database
doesn't hold (a call to an external system whose result must be
incorporated and can't be re-run for a second attempt). Move to step 2
or 3; if neither fits either, see "When none of this fits," below.

## 2. A schema-level constraint, where the backend has one

**On Postgres, prefer a real constraint over any lock if the invariant
can be expressed as one.** `EXCLUDE USING gist` enforces a range-overlap
invariant (the room-booking case above, or any "no two rows may overlap
on this predicate" shape) at the schema level — no application code at
all, and correct even against a caller who never checks anything:

```sql
CREATE EXTENSION IF NOT EXISTS btree_gist;
ALTER TABLE booking ADD CONSTRAINT booking_no_overlap
  EXCLUDE USING gist (room_id WITH =, during WITH &&);
```

`.nir`'s `db_execute` can issue this DDL against a real Postgres server
— it's a plain literal string, the same as any `CREATE TABLE` this
repository's other examples already send.
`examples/killer_demo/race_probe_booking_pg_exclude.nir`: the same
8-thread contention workload as step 1's booking example, but
`try_book_inner` does *no* overlap check of its own — the constraint is
the only thing preventing a double-booking. Verified 3/3, zero
overlaps, zero application logic (`RESULTS_BOOKING.md`).

**SQLite has no equivalent feature.** This step is Postgres-only; on
SQLite, step 1 (or, if that doesn't fit, step 3, in principle) is what's
available.

## 3. A Postgres advisory lock — reachable, but not yet safe under contention

`SELECT pg_advisory_xact_lock($1, $2)`, called through `db_query` on a
`db` connection handle that then issues the rest of its work and a
`COMMIT` on that same handle, is syntactically reachable from `.nir`
today, and the lock genuinely releases at commit — confirmed with a
sequential, single-threaded 5-cycle smoke test, 5/5 clean.

**Do not use this yet.** Running the same lock under real concurrent
contention deadlocks the whole process's `db` layer, not just the two
threads involved — a real, confirmed kernel bug, not a property of
advisory locks: `crates/runtime-kernels/src/kernel/mod.rs`'s
`HandleTable<T>::with` holds one process-wide mutex for the entire
duration of the blocking call it wraps, so a thread genuinely waiting on
a Postgres-side lock holds that mutex for as long as it waits —
including against the unrelated thread that would release the lock it's
waiting on. Reproduced with both `pg_advisory_xact_lock` and a plain
`SELECT ... FOR UPDATE`, so this affects any two concurrent `.nir` `db`
operations with a real server-side wait between them, not just advisory
locks. Full root-cause writeup: `examples/killer_demo/
RESULTS_BOOKING.md`; tracked as its own fix, independent of RFC 0015
(see that RFC's Open Questions):
[kannamma-labs/nirdosha#37](https://github.com/kannamma-labs/nirdosha/issues/37).

Once `HandleTable::with` no longer holds its table-wide lock across a
blocking call, this step becomes a real option for an invariant that
needs cross-process coordination (a second instance of the same
service, not just a second thread) — the one case steps 1 and 2 can't
reach, since both are scoped to what a single statement or constraint
inside one database can see.

## When none of this fits

A critical section that genuinely requires a non-SQL, non-idempotent
external call between the check and the write — the result of that call
must be incorporated into the decision, and calling it twice for the
same resource would be wrong, not just wasteful — is not answered by any
of the three steps above. RFC 0015's `guard(tag, key) { ... }` design
was motivated by exactly this shape, but no worked example of it
survived review: every example tried reduced to step 1 or step 2. If a
real one turns up, RFC 0015's "Implementation plan" names the gate it
has to pass before a language feature gets built for it — read that
section before reaching for a hand-rolled lock of your own; the
research and the reference design already exist, kept on file, not
discarded.
