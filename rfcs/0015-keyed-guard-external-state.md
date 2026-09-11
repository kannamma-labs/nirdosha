# RFC 0015: `guard(keys) { ... }` — keyed mutual exclusion for external state

**Status note — nine review rounds. This is a summary of outcomes, not
a blow-by-blow history; see git history for that if it's ever needed.**
(An earlier revision of this note said "eight," corrected down to "six"
because the four bullets below only accounted for six — then three more
rounds actually happened, each recorded below, and the count follows
them rather than getting fixed at whatever number was true the day it
was last edited. The count belongs to "what this revision does not
claim: completeness," below, as much as anything else it lists — it is
current as of this revision, not a promise it stays nine in the next
one.) Every round found at least one genuinely load-bearing
bug, not rewording. In rough order of how fundamental they were (the
first four bullets); rounds seven through nine, each a single finding
rather than a category, are recorded by name below the bullets, not
folded into them:

- **The key model itself was wrong twice.** First, a per-call-site id
  in the key meant two different functions guarding "the same" resource
  from two call sites never actually excluded each other — the
  primitive's central promise, false. Fixed by dropping the call-site
  id. Second, the namespace tag was itself independently acquirable,
  meaning every call for *any* instance of a resource type contended on
  the shared tag string — meant to namespace, not to lock. Fixed:
  `guard(tag, k1, k2, ...)` requires the tag as a real, typechecked
  first argument (`GuardMissingNamespaceTag`, a compile error, not a
  lint) and acquires `N` compound `(tag, key_i)` identities, never the
  tag alone. See "API" and "`GuardKeyValue` and `GuardKey`."
- **No motivating example has survived as structurally required, and
  this revision stops claiming one has.** Balance transfer, payment
  capture, and a low-stock alert each collapsed into
  `race_probe_atomic.nir`'s own single-statement conditional-`UPDATE`
  pattern — the check and the write live in one row's columns, a
  compare-and-swap problem, not a `guard` problem. The room-booking
  example (an overlap check spanning an unbounded set of *other* rows)
  was believed to be structurally immune to that specific collapse; this
  round of review found the wider collapse it was never tested against —
  `INSERT ... SELECT ... WHERE NOT EXISTS (...)` is one statement too,
  and SQLite serializes it the same way, no lock required. A second
  attempt (a two-warehouse inventory transfer, testing a two-row
  invariant) collapses the same way, into one `UPDATE` with subqueries.
  The Motivation section now states the honest conclusion: on the
  backends `.nir` targets today, `guard`'s value in every example this
  document has tried is ergonomic and structural (one reviewed primitive
  plus static detection, versus N hand-rolled locking schemes), not
  existential necessity — and names the one shape (a required, non-SQL,
  non-dedupable external call between check and write) that would
  actually need `guard`, without claiming to have a worked example of
  it.
- **Concurrency correctness gaps, each real, each fixed**: an
  unspecified enter/exit FFI ABI (resolved: key values captured once
  into compiler temporaries, never re-evaluated); no rollback on
  partial multi-key acquisition failure (specified: reverse-order
  release under the same protocol as a normal exit); a
  deadlock-freedom claim stated unconditionally that's actually false
  whenever a lease is already held from an enclosing `guard`
  (rescoped, not oversold, to the case it actually covers); reentrancy
  keyed on `ThreadId` alone, unsound given this runtime's *reused*
  worker-thread pool (fixed: a global execution-generation counter with
  proper save/restore around nested units, not a per-thread counter);
  an `RandomState`/hashing story that would either break shard
  determinism or reopen hash-flooding depending on which mistake was
  made (fixed: one random-but-fixed-per-process seed, shared and
  reused, `hashbrown` named as the real dependency this requires); a
  budget-accounting rule that contradicted itself across two
  paragraphs (fixed: one rule, reservation-then-reconcile); a
  stuck-holder sweep that would false-positive on a waiter who just won
  a long queue (fixed: the hold clock resets on a genuinely new hold,
  not on reentrancy, and the sweep skips holderless entries); and an
  RAII panic-safety mitigation that could not work given the design's
  own two-call ABI (withdrawn, replaced with a checked argument for why
  the case doesn't arise).
- **Documentation-vs-body drift, fixed where found**: an error type
  that collapsed three distinguishable failure modes into
  string-differentiated copies of one variant, restated as "fixed" once
  already before actually being fixed; a Testing section that claimed
  to solve a collision case its own mechanism doesn't reach; an Open
  Questions bullet contradicting a mitigation the body had already
  withdrawn.

**What this revision still does not claim: completeness.** A real,
named tail remains open — no per-domain-tag partitioning of the lease
budget, near-miss domain tags (two call sites choosing different tag
strings for what should be one resource) with no lint able to catch it,
`O(n)` waiter removal under sustained timeouts, whether
`transact`-wrapping-`guard` should get its own lint symmetric with
`NestedGuard`, a shard count with no benchmark behind it, and more,
itemized in "Open questions" below. **A seventh round found the widest
version yet of the collapse this document keeps re-deriving — see
"The example this RFC uses," above — and, on the strength of that
finding, added "Implementation plan," below: a verification gate this
document did not previously have, because until this round nothing in
this document had actually admitted that its own motivating examples
might not need building.** **An eighth round then ran that gate's
load-bearing experiment (A1) for real** instead of leaving it as a
documented-but-untested plan: `examples/killer_demo/race_probe_booking.nir`
and its naive control, 3/3 runs each, confirmed the collapse and
confirmed the harness would have caught it if the collapse didn't hold
(`examples/killer_demo/RESULTS_BOOKING.md`). Per the gate's own decision
rule, **Phase C — the `guard` language feature itself — does not
proceed on the evidence in this document**, and "Implementation plan"
was restructured accordingly: moved ahead of "Design" (which now reads
as a kept, not currently-actioned, reference spec) instead of trailing
it, so a reader meets the outcome before the mechanism. **A ninth round
ran A2 and A3 too**, against a real local Postgres — A2 (a Postgres
`EXCLUDE USING gist` constraint) passed 3/3 with zero application-level
code, a second and stronger confirmation that the outcome above is
right. A3 (`pg_advisory_xact_lock`) confirmed the mechanism is reachable
and correct when uncontended, but running its concurrent-correctness
check surfaced a real, unrelated, confirmed kernel bug instead of an
answer: `HandleTable::with` (`crates/runtime-kernels/src/kernel/mod.rs`)
holds one process-wide mutex across the blocking Postgres call it wraps,
so two threads with any server-side ordering dependency between them —
not specific to advisory locks, reproduced with a plain row lock too —
deadlock the whole process's `db` layer. Named in "Open questions" as
its own item, independent of this RFC, and as a now-documented
precondition on Phase C's proposal to reuse `HandleTable` for `guard`'s
own lease entries. **Nine rounds of real findings so far — this note
does not predict there won't be a tenth.** The
document's own standard hasn't changed: name what's still wrong,
don't claim the list is closed just because it's long.

## Motivation

### The example this RFC does *not* rest on, and why

`examples/killer_demo/race_probe.nir` corrupts a SQLite ledger under
`spawn` concurrency — a naive read-then-write race. The obvious next
thought is "so we need a lock." But `examples/killer_demo/
race_probe_atomic.nir` already fixes that *exact* bug with one
rewritten SQL statement: `UPDATE account SET balance_cents =
balance_cents - ? WHERE id = ? AND balance_cents >= ?`. Verified 3/3
runs, zero drift, no new language feature. A single-statement,
single-table, arithmetic invariant is a solved problem — SQLite's own
per-statement atomicity handles it, in any language, today. **A `guard`
construct motivated by this example would be solving a problem that
doesn't need it**, which is the first and most serious defect the prior
draft had.

### The example this RFC uses — and the honest limit of what it proves

Both prior examples here failed the same way, and it's worth naming the
general shape of the failure before trying again: the balance-transfer
example collapsed into one atomic `UPDATE ... WHERE balance >= ?`. The
low-stock-alert example — reviewed, and shown to have exactly the same
flaw — collapses just as completely into one atomic `UPDATE sku SET
low_stock_alert_sent = true WHERE id = ? AND low_stock_alert_sent =
false AND stock_level < reorder_threshold`, checking the affected-row
count to decide who "won" the right to send the alert. The payment
example collapsed the same way, behind a deterministic idempotency key.
**Any critical section whose entire invariant lives in one row's columns
is a compare-and-swap problem, full stop.** A prior revision of this
section stated the rule at exactly that scope — *"if the check and the
write can be expressed as a single `UPDATE ... WHERE <predicate on the
current row>`, `guard` is not necessary"* — and used a meeting-room
booking example (an overlap check against an unbounded number of *other*
rows, not the row being written) to claim structural immunity to it.
That claim does not survive this round of review, for the reason below,
and the rule itself has to be restated wider as a result.

A meeting-room booking system: reserve room `room_id` for `[start,
end)`, but only if no *existing* booking for that room overlaps the
requested range —

```
read every existing booking for room_id where NOT (end <= existing.start
  OR start >= existing.end)  -- an overlap check against an unbounded,
  variable number of other rows, not one row's own columns
if any overlap found: return Err(RoomUnavailable)
insert a new booking row (room_id, start, end, ...)
```

The claim made for this example was that no rewording of
`UPDATE ... WHERE <predicate on the current row>` reaches it, because
the check is an existence query over *other* rows, not a predicate on
the row being written — true as stated, and irrelevant, because it is
not the only single-statement shape SQL has. This does:

```sql
INSERT INTO booking (room_id, start, "end")
SELECT :room_id, :start, :end
WHERE NOT EXISTS (
  SELECT 1 FROM booking
   WHERE room_id = :room_id
     AND NOT ("end" <= :start OR start >= :end)
)
```

On SQLite — the backend this repository's own examples target — a
single statement executes atomically and writers serialize: two
concurrent executions of this `INSERT ... SELECT ... WHERE NOT EXISTS`
against the same room cannot both succeed. Whichever commits first makes
its row visible to the second's `NOT EXISTS` subquery, which then
correctly evaluates false and inserts nothing. No double-booking, no
`guard`, no lock — and **this is no longer just argued, it's measured**:
`examples/killer_demo/race_probe_booking.nir` runs exactly this
statement under real concurrent `spawn` load — 8 threads racing for the
same 250 disjoint time slots on one room, up to 8 concurrent attempts
per slot — and `examples/killer_demo/race_probe_booking_naive.nir` runs
the separate-SELECT-then-INSERT control from the pseudocode above,
proving the harness actually catches a race when one exists. Three runs
each: the collapsed version produced exactly 250 bookings, 250 rows, 0
overlapping pairs, every time; the naive control produced 569–709
bookings and 603–974 overlapping pairs, every time — a real,
reproducible double-booking race, confirming the harness works, and
confirming the collapse closes it. Full numbers in
`examples/killer_demo/RESULTS_BOOKING.md`. This is Phase A1 from
"Implementation plan," below, run to completion — not a remaining open
question. The room-booking example is not
structurally immune to single-statement collapse; it only looked that
way because the prior revision tested it against the narrower rule
(single-row `UPDATE ... WHERE`) instead of the general one.

**The general rule is broader than "single-row predicate," and it is
this: if the check-then-write invariant can be expressed as one
conditional DML statement — `UPDATE ... WHERE`, `DELETE ... WHERE`, or
`INSERT ... SELECT ... WHERE`, with or without subqueries reaching other
rows or other tables — `guard` is not necessary.** SQL's subquery
support means "one statement" is a much larger class than "one row's own
columns": a `WHERE` clause (or the `SELECT` an `INSERT` reads from) can
encode an existence check, an aggregate, or a join against arbitrary
other rows, all evaluated against one atomic snapshot of the database as
of that statement's execution. Nothing in `.nir`'s `db_execute` (`docs/
LANGUAGE.md` §"db, SQLite + Postgres": `sql: str` is passed through
verbatim to the backend, up to 8 bind values, no restriction on
subqueries or joins within that one literal) narrows this — a `.nir`
program can write any single SQL statement its target backend accepts,
including the one above. This is the position on the SQL surface an
earlier revision left unstated (see "Open questions" in a prior round);
it is now stated: single-statement DML in `.nir`, on either backend this
repository targets, is as expressive as the backend's own SQL dialect
allows, subqueries included.

**A second attempt, to test the rule against a case with two rows on
*both* sides of the invariant, not just one:** transfer `qty` units of
some SKU from warehouse `from_id` to warehouse `to_id`, only if the
source would not go negative and the destination would not exceed its
own capacity — a decision that depends on both rows together, not either
one's columns alone. This also collapses into one statement:

```sql
UPDATE warehouse_stock SET qty = CASE id
    WHEN :from THEN qty - :amount
    WHEN :to   THEN qty + :amount
  END
WHERE id IN (:from, :to)
  AND (SELECT qty FROM warehouse_stock WHERE id = :from) >= :amount
  AND (SELECT qty FROM warehouse_stock WHERE id = :to) + :amount
      <= (SELECT capacity FROM warehouse_stock WHERE id = :to)
```

Same shape, same result: one statement, evaluated atomically against one
snapshot, no `guard` required. Two rows instead of an unbounded set
doesn't change anything — subqueries reach however many rows the
predicate needs.

**The honest conclusion, stated as plainly as the three prior collapses
were**: no example this document has tried survives as *structurally
required* to use `guard`, on the backends `.nir` actually targets today,
because a single SQL statement with subqueries can express essentially
any pure-data check-then-write invariant, however many rows or tables it
spans. The room-booking collapse is now measured, not just argued (see
above); the two-warehouse collapse just above it is still an argued
claim, not yet run the way the room-booking one was — named here rather
than left blended into a paragraph that reads as if both carried the
same weight of evidence. The one shape that *would* survive — a critical section where the
decision between check and write depends on something a `WHERE` clause
cannot reach, because it isn't data the database holds (a call to an
external system — a rate lookup, a capacity planner, a fraud check —
whose result must be incorporated and which cannot be run twice for the
same resource without being wrong) — is exactly the shape the payment
example gestured at and this document backed away from, because the
specific version of it tried here reduced to a dedupable idempotency
key instead. This document does not currently have a worked example of
that harder shape, and does not claim one. **What `guard` actually
offers, on the evidence in this document, is ergonomic and structural
value — one reviewed, tested primitive in place of N call sites each
hand-rolling their own locking scheme, plus the static detection in
"Static detection" below catching the "forgot to guard a write path"
class of bug that a hand-rolled scheme has no equivalent defense
against — not a claim that any example here is unsolvable without it.**
A real database-level alternative is also worth naming honestly:
PostgreSQL's `EXCLUDE USING gist` constraint enforces the room-booking
invariant at the schema level with no application-level lock at all —
**and is reachable from `.nir` today, now measured, not merely argued**:
"Implementation plan" Phase A2, below, runs `db_execute` against a real
Postgres server with `ALTER TABLE ... ADD CONSTRAINT ... EXCLUDE USING
gist (room_id WITH =, during WITH &&)` and a `try_book_inner` that does
*no* overlap check of its own at all — 3/3 runs, 250/250/0, the
constraint alone. An earlier revision of this paragraph said this
alternative "isn't reachable from `.nir` today" and gave that as the
reason it didn't change the conclusion above; that claim was asserted,
not verified, and A2 shows it was wrong — `db_execute` passes any
literal SQL text through, DDL included, with no special case that would
have blocked this. SQLite has no equivalent feature, so the alternative
is Postgres-specific, but on Postgres it is not just reachable, it's a
*stronger* answer than either the SQLite collapse or `guard`: the
invariant lives in the schema, not in any call site's code, so it holds
even against a caller who never checks anything. This doesn't change
the conclusion above — it strengthens it.

The room-booking example is still used throughout the rest of this RFC,
not because it is structurally required, but because it is the clearest
available illustration of the pattern this document is actually about:
a critical section spanning more than one statement, where the
correctness argument depends on holding exclusivity across all of them.
Every mechanic below (multi-key acquisition, reentrancy, the nested-`guard`
deadlock case) is demonstrated against it on that basis, not on a claim
that no cheaper fix exists.

## Implementation plan: a verification gate, exercised — and the result

**Everything from "What this gives you" through "Rejected alternatives,"
below, is a real, reviewed engineering design. It is kept for reference
and for the case Phase A2/A3 (below) or future work reopen — it is not,
as of this gate's result, this document's recommended next step.** The
Motivation section above concludes that no motivating example tried in
this document survives as structurally required — every one collapses
into a single conditional DML statement on the backends `.nir` targets
today, and the one shape that would genuinely need `guard` (a required,
non-SQL, non-dedupable external call between check and write) has no
worked example anywhere in this document. That conclusion changes what
"implementing RFC 0015" should mean: build the 256-shard lease table,
the two new dependencies (`parking_lot`, `hashbrown`), and the new
exit-path codegen pass only after checking, cheaply, whether the problem
they solve is still open once the database-layer alternatives are
actually run — not argued about in prose, the way every collapse in this
document so far has been. This section states that gate, and — for A1 —
its result.

### Phase A — verification (go/no-go gate; no shipped features)

- **A1 (load-bearing) — done, and it passed.** Run the
  `INSERT ... SELECT ... WHERE NOT EXISTS` collapse from "The example
  this RFC uses" against SQLite under real concurrent `spawn` load, the
  same way `race_probe_atomic.nir` already verifies the balance-transfer
  collapse: N concurrent attempts to book overlapping ranges on one
  room, 3/3 runs, assert exactly one booking survives.
  `examples/killer_demo/race_probe_booking.nir` (the collapse) and
  `examples/killer_demo/race_probe_booking_naive.nir` (a check-then-
  insert control, proving the harness catches a race when one exists)
  are both this experiment and Phase B's B1 deliverable (below) — one
  pair of artifacts, not built twice. **Result, 3/3 runs each**: the
  collapsed version produced exactly 250 bookings / 250 rows / 0
  overlapping pairs every run; the naive control produced 569–709
  bookings and 603–974 overlapping pairs every run — a real,
  reproducible double-booking race in the control, and none in the
  collapse. Full numbers: `examples/killer_demo/RESULTS_BOOKING.md`.
  **A1 confirms the collapse.**
- **A2 — done, and it passed, fully.** Check whether `.nir`'s
  `db_execute` can issue `ALTER TABLE ... ADD CONSTRAINT ... EXCLUDE
  USING gist` against Postgres — a plain DDL string, no different in
  kind from the `CREATE TABLE` calls `docs/LANGUAGE.md` already
  documents `db_execute` accepting. `examples/killer_demo/
  race_probe_booking_pg_exclude.nir`, against a real local Postgres
  (`docker-compose.dev.yml`, this repo's own dev convention): the same
  8-thread/250-slot workload as A1, but `try_book_inner` does **no
  overlap check of its own at all** — it just inserts and lets the
  constraint accept or reject. **Result, 3/3 runs: exactly 250
  bookings / 250 rows / 0 overlapping pairs, every run, with zero
  application-level logic.** Full numbers:
  `examples/killer_demo/RESULTS_BOOKING.md`. Schema-level enforcement of
  the room-booking invariant is a real, working, documented
  Postgres-specific answer, stronger than anything `guard` provides.
- **A3 — reachability confirmed; concurrent-correctness check blocked
  by a real, unrelated kernel bug found while running it.** Check
  whether `SELECT pg_advisory_xact_lock($1, $2)` is callable through
  `db_query` on a `db` connection handle, and whether the lock it takes
  is actually released at that connection's `COMMIT` (Postgres's own
  documented behavior for the `_xact_` variant, not assumed). **This
  RFC's own original framing of A3 — "inside a `transact` block" — was
  wrong**: `transact` (`docs/TRANSACT.md`) is a six-slot durable/
  compensating-effect construct, not a plain SQL-transaction wrapper;
  what actually matters, and what's true, is that one `db` handle holds
  one physical connection across multiple calls
  (`kernel::pool::PoolRegistry`), so `BEGIN` / `pg_advisory_xact_lock` /
  ... / `COMMIT` as four ordinary calls on the same handle is correct,
  no `transact` involved. **Sequential reachability: confirmed** — a
  single-threaded, 5-cycle smoke test (reusing the connection pool each
  time) completed cleanly, 5/5, lock genuinely taken and released each
  time. **Concurrent correctness: not established** —
  `examples/killer_demo/race_probe_booking_advisory.nir`, the same
  8-thread/250-slot workload as A1/A2, deadlocks as soon as two threads
  genuinely contend for the lock. Root-caused, not just observed:
  `crates/runtime-kernels/src/kernel/mod.rs`'s `HandleTable<T>::with`
  holds one process-wide mutex (shared by *every* `db` handle) for the
  full duration of the blocking network call it wraps, so a thread
  blocked waiting on a Postgres-side lock holds that mutex for as long
  as it waits — including against the unrelated thread that would
  release the lock it's waiting on. Confirmed general (not
  advisory-lock-specific) with a `SELECT ... FOR UPDATE` control that
  deadlocks the same way. Full root-cause writeup, and why this is
  independently worth fixing before `HandleTable` is reused for
  anything with a blocking, contended acquire path (which is exactly
  what Phase C, below, proposed for `guard`'s own lease table):
  `examples/killer_demo/RESULTS_BOOKING.md`. **Does not change the
  decision rule's outcome below** — A1 alone already settles it; the
  SQL-level mechanism (`pg_advisory_xact_lock`) is reachable and correct
  when uncontended, and what's unverified is `.nir`'s current ability to
  run it correctly under contention, which is a kernel bug, not a
  property of advisory locks.

**Decision rule, stated precisely so this gate cannot be read as
optional or as requiring unanimous failure**: A1 alone is the test of
this document's own central claim ("The example this RFC uses"). If A1
confirms the collapse — no lock, no lease table, no new keyword, one
already-existing SQL feature — **Phase C does not proceed**, regardless
of how A2 or A3 turn out; a Postgres-specific escape hatch failing to
exist does not reopen a case that plain, backend-agnostic DML already
closed. **A1 has confirmed the collapse (above). Phase C does not
proceed on the evidence in this document.** A2 and A3 have both now run
too — A2 fully confirms a second, independent, and stronger answer
(schema-level enforcement, zero application code); A3 confirms the
SQL-level mechanism is reachable and correct when uncontended but
surfaced a real kernel bug (`HandleTable::with` holding a process-wide
lock across a blocking call — see A3, above) that blocks verifying it
under contention. Neither changes the outcome — A1 alone was, and
remains, load-bearing for this decision.

### Phase B — documentation (regardless of Phase A's outcome)

- **B1 — done.** `examples/killer_demo/race_probe_booking.nir`,
  `race_probe_booking_naive.nir` (A1), `race_probe_booking_pg_exclude.nir`
  (A2), and `race_probe_booking_advisory.nir` (A3) all ship as worked
  evidence: A1 and A2 as the worked proofs of a real collapse each, A3
  as the worked artifact for a mechanism that's reachable but not yet
  safe to recommend (see A3, above). `examples/killer_demo/
  RESULTS_BOOKING.md` carries the full numbers and the A3 root-cause
  writeup.
- **B2 — done.** `race_probe.nir`'s header comment now carries an
  `expected_race` marker and points at the fixes that actually exist
  (`race_probe_atomic.nir` for the single-row case,
  `race_probe_booking.nir`/`race_probe_booking_pg_exclude.nir` for the
  multi-row case), not at `guard`. `docs/LANGUAGE.md` §7's race-freedom
  paragraph now has the sentence Phase A earned: `db`-state races are
  addressable today with ordinary SQL discipline, not waiting on a new
  primitive — same three examples cited, plus a pointer to RFC 0015's
  Motivation section for why the primitive wasn't built.
- **B3 — done.** `docs/CONCURRENT_STATE_PATTERNS.md`: the decision
  sequence this document's own Motivation section argues for, as
  guidance for future `.nir` authors — conditional DML first (cheapest,
  works today, proven by A1), schema-level constraints where reachable
  on Postgres (proven by A2) ahead of advisory locks (mechanism proven
  reachable by A3, but explicitly flagged **not yet safe to use** under
  contention pending the `HandleTable::with` fix A3 surfaced), `guard`
  only for a critical section that genuinely cannot be expressed as any
  of those — and no such case is demonstrated in this document yet.

### Phase C — conditional build (not proceeding, per the decision rule above)

**Not currently authorized.** If Phase A ever leaves a real case
standing — it does not, on A1's result — build only what "The key model
itself was wrong twice" and "Concurrency correctness gaps" (Status note,
above) established is genuinely novel — no counting-domain retrofit
answers "which one, by whom" (see "Extending the existing counting-
domain model," below) — and unify everything else with `rfcs/0007`'s
resource-control kernel rather than duplicating it:

- Keyed lease entries, `(tag, key) → HolderState`, as specified in "The
  lease table," below — on `rfcs/0007`'s existing `HandleTable<T>` (its
  own write-up: "for the next resource domain... to use instead of
  inventing its own table"), not a second table. **Blocked on a
  prerequisite fix, found by A3 above, not assumed**:
  `HandleTable::with` currently holds its one process-wide mutex for the
  full duration of the closure it runs, including any blocking call
  inside it — reusing it verbatim for `guard`'s lease entries would mean
  a `guard` acquire that blocks waiting on another thread (the normal
  contended case, not an edge case) holds that same mutex for the whole
  wait, stalling every other handle sharing the table. `HandleTable` is
  not reusable for this until `with` (or a caller's use of it) stops
  holding the table lock across a call that can block — a real
  precondition on this bullet, not a detail to sort out during
  implementation.
- Generation-scoped reentrancy exactly as "Reentrancy," below,
  specifies — the `execute_compiled_unit` choke point belongs in
  `rfcs/0007`'s `kernel::thread_pool` dispatch, the same reused-worker
  pool that makes `ThreadId`-alone unsound in the first place, not a
  parallel entry-point hook maintained separately from it.
- Multi-key canonical-order acquisition and rollback, and the
  block-scope exit-path codegen pass, exactly as "Multi-key acquisition
  order" and "Exit paths," below, specify — the exit-path pass
  generalizes `rfcs/0006`'s existing `Scope` auto-join machinery
  (`codegen.rs`'s `emit_affine_free`) rather than inventing sibling
  scope-exit infrastructure next to it.
- The typecheck surface from "API" and "`GuardError`," below
  (`GuardMissingNamespaceTag`, `GuardKeyMustBeIntOrStr`), plus a named
  `TypeErrorKind` for the zero-key case this document's grammar allows
  but never names (see "Open questions," at the end).

### Phase D — unification with `rfcs/0007` (only with Phase C; not proceeding)

- Budget accounting ("The lease table," below) folded into `rfcs/0007`'s
  existing per-domain ceiling machinery — same `10_000` default every
  one of its 7 built-in domains already uses, not a new number invented
  for this one.
- Recorder integration for the stuck-holder sweep ("Holders are not
  time-bounded," below) into `rfcs/0007`'s `kernel::recorder`, **with
  the telemetry-redaction decision (Security, below) made first**, not
  discovered after keys are already flowing into it.
- The stall detector `rfcs/0007` already ships
  (`concurrency_wait_begin`/`_end`) extended to `guard` waiters, per
  "Extending the existing `join`/`recv` deadlock detector," below —
  the pattern that RFC already establishes for new blocking primitives,
  not a bespoke detector for this one.
- `AdmissionDenied` as a distinct surfaced error kind, shared with
  `rfcs/0007`'s own open item of the same name, rather than `guard`
  inventing its own parallel error-taxonomy fix.
- The `nirdosha verify` lints in "Static detection," below
  (`UnguardedTableAccess`, `NestedGuard`) built against `rfcs/0007`'s
  compiler manifest pass once it exists, since that pass is the natural
  place to know which tags a function touches — not a second,
  `guard`-specific AST walk.
- The shard-count benchmark "Open questions" already calls for, run
  before fixing 256, not after.

### Non-goals for this plan

- No cross-process/distributed lock beyond what A3 might find already
  reachable through Postgres — the answer for anything A3 doesn't cover
  is documented patterns (idempotency keys, outbox/saga), not a runtime
  feature this RFC adds.
- No deadlock-freedom proof beyond the bounded timeout plus Phase D's
  detector extension — the accepted posture per "Multi-key acquisition
  order," below, not a blocking requirement on Phase C, itself not
  currently authorized.
- No flattening of the nested `Result` shape ("The nested-Result shape,"
  below) — unchanged, awaiting general propagation sugar this RFC does
  not add.

**Reading the rest of this document**: everything from here through
"Rejected alternatives" is the design Phase C would build if the gate
above ever reopens — read it as a spec kept on file, not as this
document's current call to action.

## What this gives you, stated precisely, before anything else

- **In-process only.** `guard` coordinates threads inside one compiled
  binary. A second instance of the same service, a cron job, or a human
  at a `psql` prompt touching the same `order_id` is completely outside
  its reach. This is not a caveat buried in "Rejected alternatives" —
  it is the actual shape of the guarantee.
- **Advisory, not sound.** Nothing about the type system, ownership
  checker, or grammar stops a `db_execute` anywhere else in the same
  program from writing to `order` without ever calling `guard`. Two
  cooperating call sites are mutually exclusive; a third, unguarded one
  is invisible to both. `guard` gives you a real primitive to build a
  correct program with — it does not make an incorrect program
  impossible to write, the way `box`/`chan` do for memory. See "Static
  detection" below for the one piece of compile-time help this RFC
  does provide, and its own honestly-stated limits.
- **A discipline guarantee, at a better price than hand-rolled SQL
  discipline** — real value (one primitive instead of every call site
  reinventing its own locking scheme), but not a categorically different
  claim than "write correct code," and this RFC does not claim otherwise
  anywhere below.

## Design

### API: multi-key, not single-key nesting

```
guard(<tag-expr>, <key-expr>, <key-expr>, ...) {
    <block>
}
```

**The first argument is structurally required to be a `str` literal —
a compile error, not a convention, and not merely a lint.** An earlier
revision made this a warning (`TypeWarning::GuardKeyNotNamespaced`) a
caller could ignore; this revision makes it a `TypeErrorKind`
(`GuardMissingNamespaceTag`) because the tag is not just a namespacing
nicety — it's load-bearing for what `guard` actually locks, per the
next paragraph. Every remaining argument (one or more) is a key
expression, each `i64` or `str` (`TypeErrorKind::GuardKeyMustBeIntOrStr`,
matching `db_execute`/`db_query`'s existing bind-value type
restriction).

**What gets acquired is `N` separate compound keys, each `(tag,
key_i)` — never the tag alone.** An earlier revision of this RFC's key
model acquired the tag itself as an independently-lockable entry
alongside the other keys, sorted into the same canonical order as
everything else — which meant *every* `guard("room", ...)` call, for
*any* room, contended on the literal string `"room"` before ever
reaching a specific `room_id`, serializing every room-booking operation
in the whole program against every other one. That's the same bug this
RFC's key model already fixed once at the call-site-id layer (see
"`GuardKeyValue` and `GuardKey`," below), reappearing one layer up. The
fix is the same shape: the tag pairs with *each* key individually to
form the actual lockable identity, so `guard("room", room_id)` acquires
exactly one compound key, `("room", room_id)` — never `"room"` on its
own — and two different rooms' bookings never contend with each other
at all.

The `N` compound keys from one call are acquired **as one atomic set**:
internally sorted into one canonical order (by the `key_i` component —
`Str` before `Int`, then by value — the tag is fixed across all of them
within one call, so it never participates in the ordering) and
deduplicated before acquisition, so:

- `guard("room", room_id) { ... }` — the room-booking example above:
  one compound key.
- `guard("account", from_id, to_id) { ... }` — a lock-based version of
  the (abandoned) balance-transfer example, if someone still wants one:
  two compound keys, `("account", from_id)` and `("account", to_id)`,
  acquired as one call, not two nested ones, which is what makes it
  safe.
- `guard("account", 1, 1) { ... }` (a transfer to itself) — the two
  `Int(1)` keys deduplicate to one compound key, `("account", 1)`.
  **Not** a deadlock, by construction, not by a timeout bailing you out.
- `guard("account", a, b) { ... }` and, concurrently,
  `guard("account", b, a) { ... }` — acquire in the same canonical
  order regardless of argument order. **Not** a lock-order deadlock,
  because there is no "order the caller chose" for two keys acquired in
  the same call — there is only the one canonical order this construct
  always uses.

This directly replaces an earlier draft's nested `guard(from_id) {
guard(to_id) { ... } }` pattern, which is exactly what produced both
the self-deadlock and the classic two-thread lock-order deadlock an
earlier review caught. Nesting a `guard` inside another `guard`'s block
is still syntactically legal (see "Reentrancy" below for the one case
this must handle safely regardless), but is not the pattern this RFC's
own example uses, and `nirdosha verify` warns on it (see "Static
detection").

`guard` is an expression, evaluating to `Result(T, GuardError)` where
`T` is the block's own result type:

```nir
// Distinct variants, not one DbError(str) with a different message
// per case -- a caller that can only tell GuardTimeout from
// KernelNotInitialized by string-matching an error message has no real
// way to tell them apart at all (this is the exact conflation an
// earlier revision's own prose claimed was fixed by using separate
// match arms while the arms themselves still all constructed
// DbError(...); the fix is the enum shape, not the match).
enum ErrorCode {
    DbError(str),
    GuardBusy,         // GuardTimeout -- contended, retrying soon may help
    GuardSaturated,    // LeaseTableExhausted -- capacity, not misconfiguration;
                        // retrying later (after waiters time out and free
                        // budget) may help, unlike GuardUninitialized below
    GuardUninitialized, // KernelNotInitialized -- a real misconfiguration;
                        // retrying won't fix it, distinct from GuardSaturated
                        // on purpose, not merged into one "unavailable" case
}

fn book_room(room_id: i64, start: i64, end: i64) -> Result(Text, ErrorCode) {
    // guard(...) { block } is itself the expression -- its value is
    // Result(T, GuardError) where T is whatever the block's own last
    // expression produces. Here T is already Result(Text, ErrorCode)
    // (the function's own return type), so `attempt`'s real type is
    // Result(Result(Text, ErrorCode), GuardError) -- a nested Result,
    // not flattened. This is a real, disclosed ergonomic cost, not an
    // oversight -- see "The nested-Result shape" below for why this
    // RFC does not attempt to flatten it automatically.
    let attempt: Result(Result(Text, ErrorCode), GuardError) = guard("room", room_id) {
        // the read-check-insert body: scans every existing booking for
        // this room for an overlap with [start, end), returns
        // Err(RoomUnavailable) if any is found, otherwise inserts the
        // new booking row -- ends in a Result(Text, ErrorCode) value,
        // the block's own last expression. No caller outside this
        // lease can run its own overlap scan against a still-incomplete
        // view of this room's bookings while this one holds it.
        book_room_body(room_id, start, end)
    }
    return match attempt {
        Err(GuardTimeout) => Err(GuardBusy),
        Err(LeaseTableExhausted) => Err(GuardSaturated),
        Err(KernelNotInitialized) => Err(GuardUninitialized),
        Ok(inner) => inner,
    }
}
```

Two things this fixes versus an earlier draft's example, both real
bugs: it now shows the actual `guard(...) { block }` syntax (an earlier
example matched on `guard(order_id)` with no block at all — not valid
under this RFC's own grammar), and `GuardTimeout` maps to a distinct
`ErrorCode` **variant** (`GuardBusy`), not merely a distinct `match`
arm that still constructs the same `DbError(str)` constructor as
`LeaseTableExhausted`/`KernelNotInitialized` with a different string —
a still-earlier revision made exactly that mistake while claiming it
fixed the conflation, because separate `match` arms produce distinguishable
*behavior* only if they also produce distinguishable *types*; two arms
both building `DbError(...)` collapse back into one type a caller can
only tell apart by string-matching a message, which is the bug, not the
fix. `GuardBusy`, `GuardSaturated`, and `GuardUninitialized` here are
real, structurally different `ErrorCode` variants a caller's own
`match` can branch on
without touching a string.

### The nested-Result shape — a real cost, not flattened

`guard`'s own `Result(T, GuardError)` wrapping a block whose own type is
already a `Result` produces a doubly-wrapped type, as above. This RFC
does not propose flattening it, because doing so generically needs
either a `?`-style propagation operator or a generic
"convert-`GuardError`-into-my-own-error-type" mechanism, and `.nir` has
neither today (every existing `Result`-returning function in this
language is unwrapped by explicit `match`, with no sugar for it — this
RFC is not the place to add one unprompted). The double-`match` pattern
above is the correct, if unlovely, way to write this until/unless a
separate RFC adds propagation sugar to the language generally.

### `GuardError` — the complete enum, all three variants used consistently from here on

```nir
enum GuardError {
    GuardTimeout,         // a waiter's bounded wait elapsed before its turn
    LeaseTableExhausted,  // NIRDOSHA_GUARD_MAX_LEASES reached; see "The lease table"
    KernelNotInitialized, // the runtime kernel hasn't booted (unit tests not going
                           // through the normal compiled-binary entry point, mainly)
}
```

Every `match` on a `guard(...)` call's result elsewhere in this
document handles all three arms — the example below included, corrected
from an earlier revision of this RFC that matched only two and would
not have typechecked as written.

### `GuardKeyValue` and `GuardKey` — one definition, with the actual scoping problem it has to solve stated up front

**The problem an earlier draft of this RFC got wrong**: including a
per-`guard(...)`-call-site id in the key, believing it was needed so an
unrelated `guard(1)` at one call site wouldn't collide with a
different `guard(1)` elsewhere. That reasoning is backwards. Two
*different* functions legitimately need to guard the *same* logical
resource — `capture_payment` and a `refund_order` function both need to
exclude each other over the same `order_id`, from two different call
sites. A call-site id in the key makes that **impossible**: it gives
every call site its own private lease table entry for what should be
one shared entry, so two functions guarding "the same" order never
actually exclude each other at all. This is the design this revision
fixes — not a wording problem, a wrong data structure.

```rust
// crates/compiler/src/main.rs / codegen.rs already define NirBindValue
// (see crates/runtime-kernels/src/kernel/db.rs's `PgBindValue` conversion
// from it) as the tagged repr db_execute/db_query bind values already
// cross the codegen/kernel boundary in. GuardKeyValue reuses that exact
// tag scheme instead of inventing a new one, restricted to the two tags
// this RFC accepts:
enum GuardKeyValue {
    Int(i64),
    Str(Arc<str>),
}

// The lease table's actual per-entry key: the call's required `str`
// tag, paired with exactly one of its remaining key values -- never
// the tag alone (see "API," above, for why an earlier revision of this
// design letting the tag be independently acquirable was a real bug,
// not a style choice). Two calls to guard("room", 7) anywhere in the
// program, from any function, resolve to the same GuardKey and share
// the exact same table entry; guard("room", 7, 12) resolves to two
// GuardKeys, ("room", 7) and ("room", 12), each its own table entry.
// "Multi-key acquisition order" (below) acquires each of a call's
// compound keys one at a time, in canonical order, as N separate
// table entries -- never stored as a multi-element group.
type GuardKey = (GuardKeyValue, GuardKeyValue); // (tag, key) -- tag is always Str, checked at typeck, not re-encoded as a separate invariant here
```

**Namespacing is no longer a convention the type system can't see —
"API," above, already made the tag a required, typechecked first
argument** (`TypeErrorKind::GuardMissingNamespaceTag`), not a `str` key
indistinguishable from any other by its type. `guard(1)` — no tag at
all — is now a compile error, not a footgun a caller could ship and
only discover via a warning. What the type system still cannot check:
whether two *different* call sites chose the *same* tag string for what
should be the same resource, or accidentally different ones
(`"order"` vs. `"orders"`) for what should collide — that residual risk
is real and is exactly what `nirdosha verify`'s `UnguardedTableAccess`-
style heuristics (see "Static detection," below) are the closest
available defense against, not a claim that it's closed.

### The lease table: sharded, not one global mutex

The prior draft's single `Mutex<HashMap<...>>` was called out
correctly (category D) as a process-wide serialization point and an
unspecified-cost hot path. This revision uses a fixed number of shards
(`GUARD_SHARD_COUNT`, a compile-time constant — 256, matching common
sharded-lock-table sizing, not derived from any measurement this RFC
has actually run; see "Open questions"), each independently locked:

`Mutex` here is `parking_lot::Mutex`, not `std::sync::Mutex` —
deliberately non-poisoning: a `std::sync::Mutex` poisons on any panic
while held, after which every future lock attempt itself panics, which
for a process-wide-shared lease table would turn one bad panic into
every future `guard` call failing, everywhere, for the rest of the
process's life. `parking_lot` doesn't have this failure mode. **This
adds `parking_lot` as a new dependency of `runtime-kernels`** — it is
not already one; this RFC is not free of new dependency cost, stated
plainly rather than implied to be zero-cost by omission.

```rust
struct GuardShard {
    // hashbrown::HashTable, not std::HashMap -- see below, "the hash
    // shard_for computes is reused here" is the reason. Entries are
    // (GuardKey, Box<HolderState>), not HolderState stored by value --
    // see the note directly below for why the box is load-bearing, not
    // defensive style.
    table: Mutex<hashbrown::HashTable<(GuardKey, Box<HolderState>)>>,
    // no shard-wide Condvar -- see below for why that was wrong.
}

// Always stored behind Box<...> in the table, never by value: hashbrown's
// HashTable is open-addressed and moves entries within its backing
// allocation on rehash. parking_lot::Condvar is not documented as safe
// to relocate while a thread is parked on it -- a waiter's registration
// is against the Condvar's own address, and a rehash mid-wait would
// move that address out from under a parked thread, either losing the
// wakeup or (worse) leaving it pointed at freed/reused memory. Boxing
// HolderState pins its address across any rehash; only the (GuardKey,
// Box<...>) pointer pair in the table's own storage relocates, never
// the pointee the Condvar's waiters are registered against.
struct HolderState {
    holder_thread: ThreadId,
    generation: u64,           // pairs with holder_thread; see "Reentrancy"
    hold_count: u32,
    acquired_at: Instant,      // for the stuck-holder warning; see "Holders are not time-bounded"
    waiters: VecDeque<WaiterTicket>,
    condvar: Condvar,          // per-KEY, not per-shard -- see below
}

struct WaiterTicket {
    id: u64,   // monotonic, assigned under the shard's own mutex
    // No reservation handle here, and none in HolderState either --
    // the lease-table budget (see "The lease table," below) is tracked
    // purely as a scalar AtomicU64 count, not as a per-occupancy
    // identity threaded through these structs. "Owned by that specific
    // occupancy," used below, describes which event increments and
    // which event decrements the counter -- an accounting relationship,
    // not a stored back-reference -- and no field is needed to express
    // it.
}

// HashMap::new() isn't const, so this isn't a plain `static` array
// literal the way the sketch previously implied -- each shard's
// HashMap is initialized lazily (`std::sync::OnceLock` per shard, or
// one `OnceLock<[GuardShard; N]>` for the whole table) on first use,
// the same pattern `runtime-kernels/src/kernel/mod.rs`'s own
// `domain::register_builtin_domains` already uses for its 7 `OnceLock`
// cells (see "New kernel primitive" -- this reuses that established
// pattern, not a new one).
static SHARDS: OnceLock<[GuardShard; GUARD_SHARD_COUNT]> = OnceLock::new();

// Fixed FOR THE PROCESS'S LIFETIME, not fixed-as-in-hardcoded -- an
// earlier revision of this comment said "constant seeds" and that was
// a real, caught mistake, not just imprecise wording: a hardcoded
// constant seed is known to anyone who can read this source (or the
// compiled binary), which lets an attacker who controls `Str` key
// content (built from user input, however indirectly) precompute
// colliding keys offline -- exactly the hash-flooding attack SipHash's
// normal per-instance random seed exists to prevent, reopened by
// "fixed" being misread as "hardcoded." The actual requirement is
// weaker and both goals ARE simultaneously satisfiable: one
// `std::hash::RandomState::new()` (a real random seed, chosen once,
// unknown to any external attacker) constructed a single time at
// process startup and held in a `OnceLock` next to `SHARDS` -- random
// for flooding resistance, constant *within this process's run* for
// shard-selection determinism (the only property `shard_for` actually
// needs: the same key hashes to the same shard on every call *in this
// process*, not across different processes or restarts, which nothing
// in this design requires). One instance, one seed, used for both
// `shard_for`'s own shard-selection modulo and the hash passed into
// each shard's internal `hashbrown::HashTable` lookup (see below) --
// there is no second, inner hasher for `hashbrown::HashTable` to have
// its own seed diverge from, since that type takes an explicit,
// caller-supplied hash on every operation rather than owning a
// `BuildHasher` itself (see the paragraph after `shard_for`, below,
// for why that's exactly the shape this design needs).
static GLOBAL_HASHER: OnceLock<RandomState> = OnceLock::new();

fn shard_for(key: &GuardKey) -> &GuardShard {
    let hasher = GLOBAL_HASHER.get_or_init(RandomState::new);
    // `% GUARD_SHARD_COUNT as u64` before the `as usize`, not after --
    // `hash_one` returns u64, and reducing mod the shard count while
    // still u64 keeps this correct on a 32-bit target, where casting
    // the full u64 hash to usize first would truncate silently before
    // the modulo ever ran.
    let shard_index = (hasher.hash_one(key) % GUARD_SHARD_COUNT as u64) as usize;
    &SHARDS.get_or_init(init_shards)[shard_index]
}
```

The hash `shard_for` computes to pick a shard is deliberately **reused**
for the map lookup inside that shard, not recomputed. `std::HashMap`
cannot do this — its hash-and-key-separately API
(`raw_entry`/`hash_raw_entry`) has never stabilized — so each
`GuardShard`'s `mutex: Mutex<HashMap<GuardKey, HolderState>>` is
actually `Mutex<hashbrown::HashTable<(GuardKey, Box<HolderState>)>>`
(boxed for `Condvar` address stability across rehash — see the note on
`HolderState`, above):
**`hashbrown` is a second new direct dependency this RFC adds**, named
explicitly here the same way `parking_lot` is named in "The lease
table," above, not left implicit. `hashbrown::HashTable`'s own API
(`find`/`insert`/`entry`) takes an explicit, caller-supplied hash for
every operation rather than owning a `BuildHasher` internally, which is
exactly the shape needed: `shard_for`'s `GLOBAL_HASHER.hash_one(key)`
is computed once and passed into both the shard-selection modulo *and*
the table lookup inside that shard, with no second hasher to
accidentally fall out of sync with the first (the risk a plain
`HashMap::new()`, defaulting to its own freshly-seeded `RandomState`,
would have reintroduced silently).

**The condvar moved from one-per-shard to one-per-key, and this is a
real fix, not a style choice.** A shard-wide `Condvar` means releasing
any key in that shard wakes every waiter on every *other* key that
happens to hash to the same shard — a shard holding waiters across
hundreds of unrelated keys turns one release into a wakeup storm far
larger than "every waiter on the released key," which is what the
fairness cost was supposed to be. A `Condvar` owned by each key's own
`HolderState` (still protected by the shard's `Mutex` for the state it
guards, `parking_lot::Condvar` doesn't require its own separate lock)
means `notify_all()` only ever wakes waiters actually contending that
key. `GUARD_SHARD_COUNT` shards still bound how many *mutexes* exist
(the throughput concern sharding was for); condvars scale with live
keys, which is the axis the fairness/wakeup cost actually lives on.

`hash(key)` runs **before** taking any shard's mutex — the review's
"key hashing under the lock" point is correct and this is the fix. A
`Str` key's `Arc<str>` is already reference-counted at the language
level (`docs/LANGUAGE.md` §2's own description of `str`), so hashing it
does not allocate; hashing does not clone the string either, since
`Hash` for `Arc<str>` hashes through the pointee without copying it.
A multi-key `guard(k1, k2, ...)` call still needs somewhere to hold the
evaluated key values while sorting them into canonical order before
splitting them into individual single-`GuardKey` acquisitions (per
"Multi-key acquisition order") — that's a `Vec<GuardKeyValue>`,
call-local and never stored in the table itself (the table's own
`GuardKey` is singular, per its corrected definition above), but it
**is** one heap allocation per `guard(...)` call regardless. An earlier
revision's "no allocation beyond the `HashMap` entry itself" claim for
the uncontended path was wrong on its face; corrected here: the
uncontended path is one `Vec` allocation (for sorting the call's key
list) plus one `HashMap` entry per key actually inserted, not
zero-plus-one.

**Budget accounting (`NIRDOSHA_GUARD_MAX_LEASES`) — one rule, stated
once, after two earlier revisions stated two different rules in the
same paragraph without noticing they conflicted.** The unit the budget
counts is neither "table entry" nor "waiter" alone — it's **one slot
per `(caller, key)` occupancy, whether that occupancy is currently
holding or currently queued as a waiter**, reentrancy excepted (below).
Concretely:

- **Reserve, then reconcile — and reentrancy is checked before a full
  budget is allowed to fail the call.** Before taking any shard lock,
  attempt one slot reservation via the global `AtomicU64` (CAS loop: if
  `current < MAX`, bump it; if the CAS loses the race, retry the read).
  If that reservation succeeds, proceed to the shard lock as below. **If
  it fails (`current == MAX`), this is not treated as a final answer —
  the shard lock is still taken**, and only *after* the real case is
  known (per "Acquisition and release protocol," below) does the call
  actually fail: a reentrant hit (case (b)) releases nothing, costs
  nothing, and proceeds to success regardless of the reservation outcome;
  only a genuinely new hold (case (a)) or new waiter (case (c)) that hit
  a failed reservation returns `Err(LeaseTableExhausted)`. **This is a
  real correctness requirement, not a nicety**: "Reentrancy," below,
  states that reentrancy is unconditional — a helper called from inside
  a `guard` block must always be able to reenter without deadlock or
  failure, regardless of what else is happening in the process. An
  earlier version of this section reserved unconditionally and returned
  `Err` before the shard lock ever revealed the case, which meant a
  helper's reentrant `guard` call on a key the outer block already held
  could fail with `LeaseTableExhausted` merely because *unrelated*
  waiters on other keys had saturated the global budget — falsifying the
  unconditional-reentrancy guarantee under exactly the load conditions
  that guarantee exists for. The cost of the fix is one shard lock taken
  on the budget-saturated path even when the call will ultimately fail —
  acceptable, because saturation is already the exceptional case, and a
  reentrant call must never pay for a lock it doesn't need to fail.
- Once the shard lock is held and the real case is known: if it turns
  out to be **reentrant** (case (b) — the calling thread already holds
  this exact key), any reservation this attempt did succeed in taking
  was unnecessary — release it immediately, back to the global counter,
  before proceeding. Reentrancy costs zero slots, full stop; this is the
  one case the reservation doesn't end up paying for, and the one case
  that proceeds to success even when the reservation attempt above
  failed outright.
- If it's a **new hold** (case (a)) or a **new waiter** (case (c)/step
  2), the reservation stands, and is owned by that specific occupancy —
  not by the table entry as a whole. A key with one holder and 9,999
  queued waiters is 10,000 outstanding reservations, not one — which is
  what makes "Security," below, correctly say a retry storm parked on
  one hot key can exhaust the global budget for the whole process: each
  parked waiter really does cost a real slot, because each is a real
  parked OS thread and queue node, a real resource, not a free entry in
  someone else's table row.
- **Released** when its specific occupancy ends: a holder's reservation
  releases when `hold_count` returns to zero (not on every reentrant
  decrement — reentrancy never held a separate reservation to begin
  with); a waiter's reservation releases the moment its ticket is
  removed, whether by winning (converts to a holder's reservation — no
  double-charge, no new reservation taken) or by timing out (released
  outright) or by "Multi-key acquisition order"'s partial-acquisition
  rollback (each rolled-back key's reservation releases with it).

A multi-key `guard("account", k1, k2, k3)`, uncontended, reserves and
keeps 3 slots — one per compound key, matching "up to 3 slots" from an
earlier revision, now for a reason that doesn't contradict the waiter
accounting an earlier paragraph also claimed.

**This protocol has a real, if transient, over-counting property, stated
here rather than left implicit.** Every acquire reserves a slot
optimistically, *before* the shard lock reveals whether the case is
reentrant, and a reentrant acquire only releases its reservation
*after* that reservation succeeded. Under concurrent load with enough
in-flight reentrant acquires, the global counter can briefly read higher
than the true number of live, distinct occupancies — enough, in
principle, to cause a fresh (non-reentrant) acquire to see
`current == MAX` and take the budget-saturated shard-lock path (above)
even though the true occupancy count is under budget; the shard-lock
check will then correctly identify that acquire as a genuinely new hold
and fail it with `LeaseTableExhausted`. This is a liveness hiccup, not a
correctness one — the caller's retry sees the reconciled count and
succeeds — but the claim two paragraphs up, that the reservation "tracks
distinct held-or-waited-on entries," is exactly true only in steady
state; under transient load it is a slight overcount, never an
undercount. Stated explicitly so an operator seeing `LeaseTableExhausted`
above the count they expected has the explanation, rather than
suspecting a leak.

Entries are removed from a shard's `HashMap` the moment their last
holder releases and no waiter remains — steady-state table size is
bounded by *live contention*, not by the historical count of distinct
keys ever guarded. `NIRDOSHA_GUARD_MAX_LEASES` defaults to 10,000, the
same default every existing kernel domain ceiling uses.

### Acquisition and release protocol — specified to close the two condvar bugs the review found

For one already-sorted set of `GuardKey`s (multi-key acquisition
acquires each in canonical order, one at a time, in the protocol below
— see "Multi-key acquisition order" for why one-at-a-time-in-order is
still deadlock-free even though it isn't a single atomic step across
shards), against **one deadline computed once, at the start of the
whole `guard(...)` call** (`now + NIRDOSHA_GUARD_TIMEOUT_MS`) —
**not** once per key. A multi-key call with several contended keys
spends the *same* remaining time budget across all of them, so a call
guarding *N* keys cannot pin earlier-acquired keys for up to
`N × NIRDOSHA_GUARD_TIMEOUT_MS` by stacking a fresh timeout at every
step; total acquisition time for the whole call is bounded by the one
timeout, exactly as "never blocks forever" implies it should be:

1. Under the target shard's mutex, exactly one of three cases applies
   — **stated as three, not two, because the middle one is easy to
   fall through into the wrong branch by accident**: (a) the key has no
   entry at all, or (b) the entry exists and its `(holder_thread,
   generation)` matches the caller's own (reentrancy — see below): both
   (a) and (b) create/update the entry with the current thread as
   holder, increment `hold_count`, release the mutex, proceed
   immediately — the uncontended, common-case path, one hashmap
   lookup/insert under a lock already held, no syscall. (c) the entry
   exists, is currently unheld (its last holder released), but its
   `waiters` queue is **non-empty** — this state is real and transient,
   reachable in the window between a release's `notify_all` and the
   front waiter actually reacquiring the shard mutex to claim its win —
   and it does **not** qualify for the immediate-success path above,
   even though the key is technically unheld at that instant: a fresh
   arrival here still enqueues (step 2, below), landing behind whoever
   is already queued. Treating "unheld" alone as sufficient for
   immediate acquisition — the natural way to misread case (a) as
   covering this too — silently reintroduces the barging/starvation bug
   the ticket queue exists to prevent.
2. Otherwise (held by a different thread, or unheld-with-waiters per
   case (c) above): push a new `WaiterTicket` (monotonically increasing
   `id`, assigned while still holding the shard mutex — this is what
   gives FIFO ordering) onto the entry's `waiters` queue, then loop
   against the **call's own single deadline** from above (not a fresh
   one per key):
   - `condvar.wait_timeout(shard_guard, remaining_time)`.
   - On every wake (spurious or real — `Condvar::wait_timeout`'s own
     contract requires this loop; a single check-and-proceed is
     incorrect regardless of `guard`'s own design, and this revision
     says so explicitly rather than leaving it implied by "use a
     condvar"): still holding the shard mutex, check whether this
     ticket is now at the front of the entry's `waiters` queue *and*
     the entry is unheld (or held by this same thread — reentrancy).
     If so: pop the ticket, become holder, proceed.
   - If not yet the ticket's turn and the deadline has passed: **while
     still holding the shard mutex**, remove this ticket from the
     `waiters` queue, then return `Err(GuardTimeout)`. Removal and the
     timeout decision happen under the same lock acquisition, which is
     what closes the timeout-then-grant race the review named: a
     release that runs `notify_all` and hands the entry to "whoever is
     at the front of the queue" cannot do so to a ticket that is either
     still in the queue (in which case it legitimately wins and the
     timing race resolves in the waiter's favor, correctly) or already
     removed (in which case release only ever sees the remaining
     tickets) — there is no window where both sides believe a different
     outcome happened, because both the timeout-check and the
     release-handoff are the same critical section (one shard mutex),
     never two separate operations that could interleave.
3. Release (normal exit, or an early `return` propagating out of the
   block — see "Exit paths," below, for the full enumeration): under
   the shard mutex, decrement `hold_count`; if it reaches zero, remove
   this thread as holder. If the `waiters` queue is non-empty,
   `condvar.notify_all()` (not `notify_one`) — **every** waiter wakes
   and rechecks the front-of-queue condition under the lock; only the
   one actually at the front proceeds, the rest re-block. This costs a
   thundering-herd wakeup under heavy contention (a real, disclosed
   cost — `notify_one` would be cheaper) in exchange for FIFO
   correctness: `notify_one` provides no way to guarantee the *specific*
   waiter that should go next is the one woken, which reopens the
   barging/starvation problem — a late arrival "jumping" a queued waiter
   because it happened to notice the key was free first — that the FIFO
   ticket queue exists to prevent. `guard` trades wakeup efficiency for
   the fairness guarantee deliberately, not by oversight.
   If the map entry has neither a holder nor any waiters left, it is
   removed from the shard's `HashMap` (bounded table size, above).

**FIFO ordering, and what it costs**: a fresh caller arriving after nine
others are already waiting is queued behind all nine, always — it
cannot "jump the queue" the way an implementation using only "check if
free, else block on one shared condvar" can (the prior draft's
implicit design, and the source of the review's barging finding). The
cost is that a burst of unrelated callers all contending the same key
serializes strictly in arrival order even if a later one could have
been served faster; this RFC accepts that cost as the honest price of
not silently starving anyone.

### Reentrancy — closes the self-transfer deadlock as a second, independent safety net

Step 1 above already treats "the current thread already holds this
exact key" as an immediate, uncontended success (`hold_count`
increments). This means even a hand-written nested `guard(order_id) {
... guard(order_id) { ... } ... }` on the same thread does not
deadlock — reentrancy is unconditional, not opt-in, precisely because
the multi-key API (which *is* opt-in, by how the caller writes the
call) is not the only way a single key could recur: a helper function
called from inside a `guard` block might itself call `guard` on the
same key without the outer caller's knowledge. Making that case safe by
default, rather than a documented footgun, is deliberate.

Reentrancy is scoped to `hold_count` on the *same thread* only — a
different thread requesting the same key while the first thread holds
it still queues normally, per the protocol above.

**`ThreadId` alone is the wrong identity for this check, given how
threads actually work in this runtime, and this is a real correctness
bug in the design as stated above, not a hypothetical.**
`thread_pool.rs`'s own module doc: `spawn` runs on a "self-tuning,
**reused-worker** OS thread pool" — an OS thread is not a
one-job-per-lifetime resource here, it services many unrelated `spawn`
calls over its life. If a lease is ever leaked without release (the
codegen-exit-path-miss case "Exit paths" defends against with a
holder-identity check, or any other bug) and the leaking thread later
returns to the pool and picks up a **different, unrelated** job that
happens to call `guard(...)` on the **same key**, step 1's reentrancy
check — `holder_thread == current_thread` — succeeds, because it *is*
the same OS thread, even though it is a completely unrelated logical
execution. This silently converts a leaked lease from the availability
bug "Holders are not time-bounded" frames it as ("every other thread
eventually gets `GuardTimeout`... until the process is restarted") into
a **silent correctness bug**: two logically unrelated critical sections
both believe they hold exclusive access, because reentrancy handed the
second one in for free.

**The fix, corrected once already since first written — a per-thread
counter is the wrong shape.** An earlier version of this fix put
`generation` on `thread_pool.rs`'s worker loop alone, incrementing once
per dispatched job. Two problems with that, both real: `.nir` code
doesn't only run on pool-dispatched worker threads — the main thread's
own `fn main()`, and (per `docs/PUBLIC_ROADMAP.md`'s compiled `serve`
and `mq` entries) a `serve_http` request handler or an `mq` consumer
callback are each a distinct way compiled code starts running, and a
per-thread-pool-only counter leaves `generation` undefined for all of
them. And even scoped to the pool alone, a *per-thread* counter resets
to 0 for every newly created worker thread — since OS `ThreadId`s are
reused after a thread dies, a stale lease held by `(TID, generation 0)`
from a dead thread collides with a brand-new thread that reused that
`TID` and whose own first job is also `generation 0`, reopening exactly
the silent-inheritance hole this fix exists to close, just through a
narrower window.

**The actual fix**: one process-wide, monotonically increasing
`AtomicU64` (`GLOBAL_EXECUTION_GENERATION` or similar), and **one
choke point every entry into compiled code must route through** — not
N hand-placed increment sites an implementor could forget one of.
Concretely, an internal (not `.nir`-visible) RAII type:

```rust
// Constructed at the start of the *one* function every compiled-code
// entry point calls through -- thread_pool.rs's job dispatch, fn
// main()'s own entry, a serve_http request handler, an mq consumer
// callback, and any future entry point this runtime grows all call
// execute_compiled_unit(...) to run compiled code, never the compiled
// entry point directly. This is what makes "every entry point
// increments generation" an invariant of the call graph, not a
// convention every new call site has to separately remember.
struct ExecutionUnitGuard {
    previous: Option<u64>, // whatever GENERATION_TLS held before this unit began
}

impl ExecutionUnitGuard {
    fn enter() -> Self {
        let previous = GENERATION_TLS.with(|g| g.get());
        let new_gen = GLOBAL_EXECUTION_GENERATION.fetch_add(1, Ordering::Relaxed);
        GENERATION_TLS.with(|g| g.set(Some(new_gen)));
        ExecutionUnitGuard { previous }
    }
}

impl Drop for ExecutionUnitGuard {
    fn drop(&mut self) {
        // restore, not clear -- see "nested units" below for why this
        // has to be a stack, not a set-and-reset-to-sentinel.
        GENERATION_TLS.with(|g| g.set(self.previous));
    }
}

fn execute_compiled_unit<T>(f: impl FnOnce() -> T) -> T {
    let _unit = ExecutionUnitGuard::enter();
    f()
}
```

Holder identity is `(ThreadId, generation)` where `generation` is
`GENERATION_TLS`'s value for the duration of the current unit — read
once per `guard` call, not cached elsewhere, since the thread-local
itself is now the single source of truth.

**What a `guard` call does when `GENERATION_TLS` holds `None`, stated
explicitly rather than left for an implementor to invent**: this only
happens when compiled code runs without ever passing through
`execute_compiled_unit` — a miswired entry point that bypasses the
choke point, a test harness invoking a compiled function directly
instead of through the normal entry, or a future kernel callback that
re-enters compiled code on its own. This is a kernel-level defect, the
same class of problem `KernelNotInitialized` already exists to report —
not a state `guard` silently tolerates. A `guard` call with no
generation in TLS returns `Err(KernelNotInitialized)` before taking any
shard lock, the same as a call made before the kernel has booted. The
alternative of inventing a fallback generation value (say, treating
`None` as its own distinct generation) is deliberately rejected: it
would make an unrouted entry point silently composable with the rest of
this design instead of loudly wrong, reopening exactly the kind of
silent-inheritance hole "The actual fix," above, exists to close.

**Nested units, save-and-restore, not overwrite-and-clear**: an earlier
version of this fix set the thread-local at unit start and implied
clearing it at unit end, which breaks the moment one unit of execution
synchronously invokes another on the same thread — a `thread_pool.rs`
job that itself dispatches an `mq` consumer callback synchronously, say.
A plain overwrite would make the *inner* unit's generation visible to
the *outer* unit's remaining `guard` calls after the inner one returns,
causing the outer unit's own, legitimately-held leases to spuriously
fail their own reentrancy check (a liveness bug — it fails safe, into a
timeout, never into silent inheritance, but it's real). `ExecutionUnitGuard`
above is a save/restore stack instead: `enter()` remembers whatever
`GENERATION_TLS` held before (the outer unit's own generation, or `None`
at the true top level), and `Drop` restores exactly that value, so
nested units compose correctly regardless of depth, and the outer
unit's `guard` calls see their own generation again the instant the
inner unit returns.

This closes every version of the problem the per-thread design had: a
value from a global monotonic source is never reused, on any thread,
ever, so "undefined on a non-pool thread," "restarts at 0 and collides
after thread death," and "clobbered by a synchronously nested unit" are
all unreachable. The reentrancy check becomes "same thread *and* same
generation," so a stale leaked lease from a prior execution has a
different generation than the current one, fails the reentrancy check,
and correctly falls into the normal contended path (queues, eventually
times out) instead of being silently inherited. This turns a possible
silent correctness
bug back into the availability bug the rest of this RFC already
discusses and accepts — worse observable behavior (a timeout instead of
quiet success) is the correct direction to fail in here, and is why
this fix is required, not optional polish.

`nir_kernel_guard_exit`'s own holder-verification check (see "Exit
paths," below) checks `(ThreadId, generation)` together against
`HolderState`, not `ThreadId` alone — the same identity, checked
consistently at both the point that grants reentrancy and the point
that verifies release, not just the one that grants it.

### Multi-key acquisition order and why one-at-a-time-per-shard is still deadlock-free

Keys in one `guard(k1, k2, ...)` call may hash to different shards, so
"acquire all keys as one atomic step" cannot mean "hold one lock across
all of them" the way a single-shard design could. Instead: sort the
full key set into one canonical order (by `GuardKeyValue`'s own tag —
`Str` before `Int`, say — then by value, `Str` compared lexically and
`Int` numerically; any total order works, as long as it is the *same*
total order for every caller, everywhere, which is why this is fixed by
this RFC and not left as a per-call-site choice), then acquire each key
via the single-key protocol above, strictly in that order, blocking as
needed at each step.

**Partial-acquisition failure is rolled back, explicitly — this is a
hard requirement of this design, not an implied detail.** If acquiring
key *i* of *N* fails (timeout, or `LeaseTableExhausted`), every key
`1..i-1` already acquired in this call is released immediately, in
**reverse** canonical order, before `guard` returns `Err` to the
caller — under each key's own shard mutex, exactly the same release
path "Acquisition and release protocol" describes for a normal exit
(decrement `hold_count`; if it reaches zero, `condvar.notify_all()` on
that key's own waiters — the same wake discipline everywhere in this
design, not a second one just for rollback — then remove the entry),
and the same `LeaseTableExhausted` budget
accounting is unwound for each one released. **The block never runs**
on a partial-acquisition failure — there is no codegen exit call to
rely on, which is exactly why this rollback has to be the acquisition
path's own responsibility, not something "exit paths" (below) can
cover. Without this, a single contended acquisition attempt would
permanently strand every key that happened to acquire before it —
this is corrected from an earlier revision of this RFC, which specified
acquisition without ever specifying its failure path.

**Given that rollback, is the canonical-order argument actually
deadlock-free?** Only for the case it's scoped to: **two callers, each
starting a flat, multi-key `guard(...)` call from a state where neither
already holds a lease acquired by an *enclosing* `guard` block.** Under
that scoping, whichever caller reaches the first (canonically smallest)
contested key first can always complete: the other caller, sorting the
same key set into the same order, cannot have acquired anything past
that same point without contending for it first — and if it does
contend and lose, its own timeout-then-rollback (above) releases
whatever it *did* acquire, which is what guarantees the winner isn't
stuck waiting on a partial holder that will never finish. This is the
standard "acquire in a fixed global order" argument, with rollback
substituting for "the loser never holds anything to begin with."

**This scoping is not the general case, and the general case is not
deadlock-free.** A thread already holding a lease from an *enclosing*
`guard` block, which then makes a second, nested `guard` call, is not
covered by the argument above — its "first key" isn't first at all, it
already holds something the sort order didn't account for. Concretely:
Thread 1 holds `A` (an outer `guard("acct", A)`), then makes a flat
call `guard("acct", A, B)` — reentrant on `A`, then waits on `B`.
Thread 2 holds `B` (an outer `guard("acct", B)`), then makes a flat
call `guard("acct", B, A)` — canonical order still puts `A` before `B`
regardless of the argument order, so Thread 2 tries `A` first, held by
Thread 1: waits. Deadlock, resolved only by the per-key timeout plus
rollback, the same as any nested case — **not** prevented by the
canonical-order argument, because that argument assumes no lease is
held before the multi-key call starts, and nesting is exactly the case
where one might be. `nirdosha verify`'s `NestedGuard` warning (see
"Static detection") is the only defense this RFC provides against this
specific shape, and it is a warning, not a rejection — the deadlock
remains possible, timeout-bounded, in any program that nests. The
canonical-order deadlock-freedom claim above is stated only for the
non-nested case for exactly this reason; it is not a claim about
`guard` in general.

### Holders are not time-bounded — stated as a design decision, not an oversight

Only the *waiting* side of this protocol is bounded
(`NIRDOSHA_GUARD_TIMEOUT_MS`). A thread that blocks forever inside a
guarded block — a `recv` that never delivers, a `db_query` against a
connection the pool never returns — holds its lease indefinitely, and
every other thread contending that key eventually gets `GuardTimeout`,
repeatedly, until the process is restarted or whatever the holder is
stuck on resolves.

This RFC does **not** propose forcibly revoking a stuck holder's lease.
Revocation the holder hasn't agreed to is unsound: if the "stuck"
thread eventually does resume (the `recv` was merely slow, not
permanently hung), it would continue executing believing it still holds
exclusive access, while a second thread — granted the forcibly-revoked
lease — believes the same thing at the same time. That is a worse bug
than the one this RFC exists to fix. The correct fix for "an operation
inside a guarded block can hang forever" is that operation having its
own bound — `docs/TRANSACT.md`'s own `timeout` clause is exactly this
for `transact`; a future bounded `db_query` is the equivalent for `db`
directly (not proposed here). `guard`'s contract is "this thread is
guaranteed exclusive access for as long as it holds the lease," which
depends on the guarded operations themselves being bounded elsewhere —
`guard` cannot and does not second-guess them.

What this RFC does add: a stuck-holder is genuinely observable, not
silent — and, unlike an earlier revision of this section, with an
actual mechanism, not just a threshold name. Two corrections to that
mechanism from what an earlier revision specified, both real: (1)
`HolderState.acquired_at` (`Instant`) is **not** "set once when a key
entry is first created" — it is reset every time `hold_count` goes from
`0` to `1`, i.e. every time a *new* continuous hold begins, whether
that's the entry's first-ever holder or a waiter who just won after a
long queue. Without this, a waiter who queues for minutes behind a
legitimate holder, finally wins, and becomes holder would inherit the
*entry's* age, not their own — the very next sweep would report a
holder who has held the lease for zero seconds as stuck, which is
false, not a diagnostic. (Reentrancy — `hold_count` going `1→2` or
higher on the *same* thread that already holds — does **not** reset
`acquired_at`: a long-running reentrant loop holding a key for a long
time is exactly the thing an operator wants a warning about, since it
occupies the resource for that whole duration regardless of whether any
single reentrant call is itself "stuck" — this is the deliberate
answer to whether reentrancy should reset the clock, not an oversight.)
(2) **The sweep only considers entries with a live holder
(`hold_count > 0`)** — an entry in the transient unheld-with-waiters
state (per "Acquisition and release protocol," case (c)) has no holder
at all, and reporting it as "stuck" would be reporting a queue depth
as a stuck holder, a category error the sweep must not make.

The sweep itself — reusing `reaper.rs`'s existing periodic-sweep
pattern (`reaper_interval_secs`, already in the flight recorder banner
today) rather than inventing a second timer subsystem — checks on each
pass: any live-held entry whose `acquired_at` is older than
`NIRDOSHA_GUARD_WARN_MS` (default 10,000 — independent of and larger
than the waiter timeout, since this is a diagnostic threshold, not a
correctness one) gets one flight-recorder report per sweep interval it
remains stuck, the same fail-open way `nfr`'s escalation already works
(`nfr.rs`'s own module doc: "never on the hot path," best-effort,
swallowed on failure). **This is a real periodic cost, not "reusing an
existing pattern" for free**: unlike `reaper.rs`'s own sweep (over
aggregate counters), this sweep must take and release each of the 256
shards' own mutexes once per pass to walk their tables — up to 256 lock
acquisitions per `reaper_interval_secs`, a real, if bounded and
infrequent, cost the Performance section should carry, not just this
one. This is observability, explicitly not a
correctness fix, and the design above should not be read as leaving
"holders never time out" unaddressed; it is addressed by naming the
actual fix's owner (the blocking call inside the block) instead of
papering over it at the wrong layer.

### Holder death: `.nir` code abort()s the whole process, but the underlying kernel doesn't — this needs an honest answer, not a confident one

A holding thread doesn't just stall — could it die (panic/abort) while
holding, leaking the lease into a process that keeps running with a
permanently-wedged key? Checked, not assumed: `codegen.rs` emits an
unconditional `call void @abort()` (confirmed at 12+ call sites) for
every trap a `.nir` program can trigger through defined behavior —
overflow, division by zero, a failed contract, the deadlock detector.
`abort()` is a whole-process `SIGABRT`, not a per-thread event — there
is no way for a `.nir`-triggerable trap to kill only the thread holding
a lease while the rest of the process survives to observe a leaked
entry. For everything a `.nir` **author** can write, this section's
worry doesn't apply, by construction, not by policy.

**What about an internal kernel-side panic** (a genuine Rust bug in
`runtime-kernels`, not something `.nir` source can trigger, but a real
possibility in any nontrivial codebase)?
`runtime-kernels/src/kernel/thread_pool.rs`'s worker loop wraps each
dispatched job in `std::panic::catch_unwind` — checked, not assumed,
against that file's own documentation: Rust's standard behavior for a
panic attempting to unwind across a plain `extern "C"` boundary (which
is what every kernel FFI function this RFC's compiled code calls
through already is, per "Compatibility," below) is to **abort the
process**, not to continue unwinding past it. So a panic originating
inside a Rust-side kernel function called from compiled `.nir` code —
which is the only place a `guard`-held lease could be "in progress" at
the moment of the panic — hits that boundary and aborts, the same
whole-process `SIGABRT` outcome as any `.nir`-triggered trap above, not
a silent per-thread death. `catch_unwind` in `thread_pool.rs` still
serves its purpose (catching a panic that originates and fully unwinds
*within* pure Rust job-dispatch code, before or after the compiled call,
never while a lease from *inside* that call is held) without
contradicting this.

**A previous revision of this section proposed an RAII/`Drop`-based
release as a belt-and-suspenders mitigation for this exact case. That
mitigation cannot work as designed, and is withdrawn, not repeated
here.** `nir_kernel_guard_enter` returns *before* the guarded block's
body runs — there is no Rust stack frame spanning "enter until exit"
for a `Drop` to fire on if something goes wrong in between; `enter` and
`exit` are two independent calls with nothing connecting them on the
Rust call stack, exactly as "Key-expression evaluation," below,
describes. The correct answer to holder death is the process-abort
argument above, not an RAII object this design's own two-call ABI makes
impossible to place.

### Key-expression evaluation, and the enter/exit ABI this settles

Each key expression is evaluated **exactly once**, left to right,
before any lease in the set is acquired. The resulting values are
stored in compiler-synthesized temporaries — ordinary SSA-style
locals `codegen.rs` already generates for any intermediate value, not
a new kind of storage and not a value visible to or nameable by `.nir`
source — kept alive for the duration of the `guard` block. Both
`nir_kernel_guard_enter` and `nir_kernel_guard_exit` are passed these
same stored values, not the original key expressions re-evaluated a
second time.

This is the concrete, decided answer to the enter/exit ABI question:
**neither** re-evaluating the key expressions at exit (which would
violate "evaluated exactly once," could re-run a side effect, and — for
a key expression that is a plain mutable local, since `.nir` `let`
bindings are reassignable — could legitimately observe a *different*
value than entry did, if the block body reassigns it) **nor** exposing
a first-class lease/token value to `.nir` source (which would
contradict this RFC's own "no first-class value a program can hold,
move, or drop incorrectly" decision, "`codegen.rs` surface," below).
Capturing the already-evaluated values once, in storage the *compiler*
manages and the block body cannot observe or mutate, needs neither: it
is the same technique `codegen.rs` already uses for any expression
whose value is needed more than once (a `match` scrutinee, a loop
bound), applied here without exception.

A key expression that itself performs I/O (a `db_query` to compute the
key, for instance) is legal but ill-advised: it runs outside any lease
this `guard` call will hold, so a key computed that way can itself be
stale or racing against whatever produced it, and it adds real latency
to every acquisition attempt including the contended retry path. This
RFC does not forbid it (forbidding "any expression with a side effect"
is not a boundary the language draws anywhere else, and the capture-once
mechanism above makes it *safe*, if still not advisable) but states the
hazard explicitly rather than leaving it to be discovered.

### Exit paths — the complete set, because `.nir` has a small one

`.nir` has no `break`/`continue` (confirmed: no such tokens exist in
the grammar), and — checked, not assumed, the same audit standard as
`break`/`continue` — no closures or lambdas either
(`docs/LANGUAGE.md`'s own §6: "No `for` loops, no closures/lambdas");
first-class functions exist (a named `fn` used as a value) but
introduce no new *lexical* scope a `return` inside a `guard` block
could jump into or out of that this enumeration doesn't already cover —
a first-class function value called from inside a `guard` block is an
ordinary call, its own `return`s are its own function's, not the
`guard` block's. No other non-local control-flow form exists in `.nir`
today. The complete set of ways a `guard(...) { block }` can stop
executing its block is exactly two: **normal fallthrough to the
block's last expression**, and **a `return` inside the block, at any
nesting depth of `match`/`if`/`while` within it, propagating out**.
`codegen.rs` emits the release call (the scoped equivalent of a
`defer`) on both paths.

**This is genuinely new codegen machinery, not a reuse of anything that
exists today.** The prior draft's "emit like a `defer`" language
implied familiar infrastructure; there isn't any — `stop conn`/`join`
today are ordinary function calls an author writes explicitly, with no
compiler-enforced release-on-scope-exit at all (which is exactly why
`race_probe.nir`'s own `transfer_inner` has a `stop conn` on every
return path, written by hand, once per function — precedent for the
discipline this RFC is choosing *not* to repeat for `guard`, on
purpose, because a forgotten manual release here is a stuck-holder bug
with no bound, per the section above). Building this means `codegen.rs`
gaining a real block-scope-exit hook that runs on every `return` lowered
inside a `guard` block's lexical scope, not just at the block's own
syntactic end — a new pass, sized similarly to how `ownership.rs`
already has to walk every control-flow path to track moves, not a
one-line addition. If `.nir` ever gains `break`/`continue` or any other
non-local exit, this pass is exactly the one that must be revisited
before that lands — noted here so it isn't rediscovered as a fresh bug
later.

`nir_kernel_guard_exit` additionally checks, under the same shard
mutex it releases under, that the calling `(ThreadId, generation)` pair
actually matches the `holder_thread`/`generation` it's about to
decrement — the same two-part identity "Reentrancy" specifies for
granting reentrancy, checked here too so the two places that reason
about holder identity can't drift apart from each other. A mismatch is
a defensive check against a `codegen.rs` bug (an exit path this pass
missed, releasing a lease this thread never actually held) surfacing as
silent corruption of an unrelated holder's `hold_count` instead of a
loud, immediate, diagnosable failure. This guards against a compiler
bug, not a malicious `.nir` program — a `.nir` program cannot itself
construct an arbitrary call to `nir_kernel_guard_exit`; only
`codegen.rs`-emitted calls exist.

**What this check does not catch, stated plainly rather than left to
look solved**: a *missed* exit path (`codegen.rs`'s pass simply
forgetting to emit a release call on some path) produces no call to
verify at all — there is nothing for this check to reject, because
nothing calls it. That failure mode is silent at the point it happens;
it only becomes observable later, indirectly, through the stuck-holder
sweep ("Holders are not time-bounded," above) once the orphaned entry's
`acquired_at` ages past `NIRDOSHA_GUARD_WARN_MS` — a real detection
path, just a delayed and indirect one, not a substitute for the
exit-path pass itself being correct.

### Spawned work does not inherit the lease

`guard` is lexical: the lease is held for exactly as long as the
calling thread is executing inside the block. `spawn` returns
immediately with a `thread` handle — it does not block — so:

```nir
guard("room", room_id) {
    let t: thread i64 = spawn some_other_write(room_id)
    // guard's block ends here; the lease releases now, NOT when `t`
    // eventually finishes. `some_other_write`, running on its own
    // thread, executes with NO lease held at all.
    0
}
```

is a real footgun this RFC does not attempt to prevent structurally
(doing so would mean either forbidding `spawn` inside a `guard` block
outright, or making `guard` block-and-wait for every thread it
transitively spawned, neither of which this RFC proposes) — stated
here explicitly so it's a documented hazard, not a surprise: **`guard`
protects the calling thread's own synchronous execution of the block,
nothing spawned from within it.** A future `nirdosha verify` lint
flagging `spawn` lexically inside a `guard` block is a plausible
addition (folded into "Static detection," below, as a candidate, not
committed to in this revision).

### Static detection — real, and honestly scoped

`.nir` string literals cannot be concatenated (`docs/LANGUAGE.md` §2:
"Literal + escapes... only — no concatenation, slicing, or indexing").
This means the full SQL text passed to every `db_execute`/`db_query`
call is a compile-time-visible literal, with no dynamic-string escape
hatch to worry about — a real static-analysis advantage most languages
don't have for this exact check. `nirdosha verify`/`build` gains a new,
non-fatal warning pass (the same "surface a previously-silent case"
precedent `typeck::ungated_fn_warnings` already sets):

- Collect the set of table names that appear (via a naive
  `INSERT/UPDATE/DELETE/SELECT ... FROM/INTO <name>`-shaped scan of the
  literal SQL text, not a real SQL parser — stated as a heuristic, not
  a sound proof: quoted identifiers, unusual whitespace, or a
  sufficiently unusual statement shape can evade it) written to inside
  any `guard(...)` block, anywhere in the program.
- Any `db_execute`/`db_query` call whose own literal SQL text mentions
  one of those table names, but which is not itself lexically inside
  any `guard(...)` block, produces `TypeWarning::UnguardedTableAccess`
  — "table `booking` is written inside a `guard(...)` block at
  <span>, but this call touches it outside any guard."
- ~~`TypeWarning::GuardKeyNotNamespaced`~~ — superseded, not a lint
  anymore: "API," above, made the namespace tag a required first
  argument, checked as a real `TypeErrorKind`
  (`GuardMissingNamespaceTag`) at the same pass that already rejects a
  non-`i64`/`str` key. What a lint still can't catch — two call sites
  choosing different tag strings for what should be one resource — is
  a residual risk with no full fix in this document; see "Open
  questions."
- Separately, any `guard(k1, k2, ...) { ... guard(k3, ...) { ... } ...
  }` — a `guard` block lexically containing another `guard` — produces
  `TypeWarning::NestedGuard`, pointing at "Multi-key acquisition order"
  above as the recommended alternative.

Neither warning is a hard error (a heuristic false positive should
never block a build the way a real type error does), and neither claims
soundness. This is explicitly the one piece of static help this RFC
provides — named, scoped, and limited, rather than the prior draft's
silence on `verify` integration entirely. `examples/killer_demo/
race_probe.nir` is deliberately left unguarded (see "Compatibility")
specifically so it is available as the worked example of what this
warning is for, **if this pass is ever built — Phase C, not currently
authorized (see "Implementation plan," above)** — not as an endorsed
pattern left to be copy-pasted uncommented.

## Interaction with `transact`

Independent primitives, one required ordering convention: **`guard`
wraps `transact`, never the reverse.** `transact`'s own retry loop
(`docs/TRANSACT.md`'s "Runtime protocol") re-runs its block's body on a
retryable failure; if `transact` wrapped `guard`, each retry would
re-acquire the lease from scratch, competing with every other waiter
again on every retry — multiplying contention under exactly the
conditions (transient failure, retry storms) where it's worst. With
`guard` on the outside, one lease acquisition covers every retry
attempt `transact` makes inside it, which is also the correct semantic
match for the room-booking example: the whole "check for an overlap,
maybe insert" sequence — retries included — is the thing that must not
run twice concurrently for the same room, not just one attempt of it.
More generally, for any multi-statement critical section with a
retryable inner operation, the property that must hold is "the whole
retry loop, not one pass through it, is the unit that needs exclusive
access" — which is exactly what `guard` wrapping `transact` guarantees
and the reverse ordering would not.

`validate`'s `pre`/`post` contracts (`docs/LANGUAGE.md` §16) run as part
of ordinary function-call codegen, unaffected by whether the call
happens to be lexically inside a `guard` block — a contract failure
inside a lease is released the same as any other exit path (per "Exit
paths," above) before the failure propagates, so a `validate` failure
never itself becomes a stuck holder.

## Testing

**What this section's mechanism actually fixes, stated precisely
because an earlier revision overstated it**: two *unrelated test
binaries* (or two separate `cargo test` process invocations) guarding
the same literal key, e.g. `guard("room", 1)`, would otherwise collide
across process boundaries and flake.

**How the salt actually applies, respecified against the compound-key
model** — an earlier revision of this section described
`NIRDOSHA_GUARD_TEST_NAMESPACE` as "prepended as one extra `Str` key to
every `guard(...)` call's key set before sorting," which was written for
a flat sorted-key-list design this RFC no longer has. Under the compound
`(tag, key_i)` model, there is no flat key set to prepend to, and taking
that description literally would mean acquiring an *additional* compound
key, `(tag, salt)` — identical for every call sharing that tag within
the process, which is exactly the shared-tag global-serializer bug "API,"
above, already fixed once, reintroduced through the test-only path. That
is not what this mechanism does. Instead: when
`NIRDOSHA_GUARD_TEST_NAMESPACE` is set, it namespaces the **tag**
component itself, at lookup time, after typechecking — every `GuardKey`
this process ever constructs uses an effective tag of `salt ⊕ tag`
(concatenation with a separator no `.nir` string literal can produce, or
equivalently a combined hash of the two — the exact encoding is an
implementation choice, not specified further here, as long as it is
injective in `(salt, tag)`) rather than the literal tag argument alone.
`guard("room", room_id)` in a process with salt `S` and `guard("room",
room_id)` in a process with salt `T` resolve to different `GuardKey`s;
within one process, every call still resolves the same way relative to
every other call in that process, so two different tags in the same
process still don't collide with each other, and the same tag from two
different call sites in the same process still does — namespacing is
by-process, not a new per-call dimension. This happens entirely at the
kernel/lookup layer: the literal-tag-required-first-argument typecheck
("API," above) sees and validates the caller's own literal tag, unaware
of and unaffected by whatever salting the runtime applies underneath it
— a fixed per-process value, set once by whatever test harness invokes
the compiled test binary.

**What it does not fix**: two *parallel tests running as threads within
the same process* (the common case for a Rust/`.nir` test binary using
a thread-per-test runner) share one `NIRDOSHA_GUARD_TEST_NAMESPACE`
value — the salt is per-process, not per-test — so two such tests both
guarding `"order", 1` still collide with each other exactly as before.
An earlier revision of this section opened by naming this exact
within-process case as a problem this mechanism solves; it doesn't. A
real per-test salt would need the test harness itself to pick a fresh
value per test (a thread-local override, not a process-wide env var),
which this RFC does not design — closing the within-process case is
left as a real, named gap for whoever writes the test harness this
mechanism assumes exists, not solved by `NIRDOSHA_GUARD_TEST_NAMESPACE`
alone.

Neither case is about making deadlock tests fast: a real timeout is
still a real wait, and a test that wants to observe `GuardTimeout`
should set `NIRDOSHA_GUARD_TIMEOUT_MS` low for that run, not rely on a
shorter built-in test-mode default this RFC doesn't propose.

## Security

**`guard` is not an authorization check, and does not become one by
being called with a caller-supplied id.** `guard(from_id)` acquires a
lease on whatever `from_id` value it's given — it says nothing about
whether the calling code was allowed to act on that id in the first
place. A function whose *own* authorization is missing or wrong is
exactly as exploitable with `guard` in it as without: `guard` can
only ever serialize concurrent access to a resource a caller already
had (or lacked) permission to touch, per "Effect on the permission
model," below. Authorize the key before guarding it, not instead of.

Every `GuardShard`'s `hashbrown::HashTable` is looked up using the hash
`GLOBAL_HASHER` (`std::hash::RandomState`, "The lease table," above)
computes — the standard library's own default hasher family (SipHash),
not a faster non-cryptographic hash — deliberately: `str` keys can
originate from data a caller controls (a `guard(...)` call built from
user-supplied input, however indirectly), and a faster hash chosen
purely for throughput would reopen a hash-flooding DoS this default
already resists. (There is no separate hasher living inside the
`HashTable` itself to make this claim about — see "The lease table" for
why `GLOBAL_HASHER` is the only hasher in this design, used for both
shard selection and the table lookup inside it.) Revisit only alongside
a real measured bottleneck, not preemptively.

- **DoS via contention** (a caller holding a hot key and sleeping) and
  **key-squatting** (a caller guessing another call site's key
  convention and colliding with it) are real, and this RFC does not
  solve them — they are a consequence of `guard` being a shared,
  in-process primitive available to any code running in the same
  binary. This is not a new trust boundary `guard` introduces: any code
  with the ability to call `guard` already has the ability to spin a
  thread and burn CPU, open every `file`/`tcp` handle up to its
  admission ceiling, or otherwise degrade the process — the same trust
  boundary RFC 0004 (native plugin sandboxing) already governs for
  native (Kind A) plugins. `guard` adds one more thing available inside
  that boundary; it does not weaken the boundary itself.
- **Timing side channel**: in a multi-tenant, single-process deployment
  where different tenants' data shares guarded keys (or keys derived
  predictably from tenant-visible ids), `GuardTimeout` vs. immediate
  success reveals whether another tenant is concurrently touching the
  same resource, and roughly how long their operation took. Disclosed,
  not mitigated — a deployment with this threat model should not derive
  guard keys from cross-tenant-visible identifiers without accepting
  this.
- **Telemetry privacy**: the flight-recorder entries this RFC adds (the
  stuck-holder warning, above) name the key. A key built from an
  account id, user id, or similar is now in whatever log/observability
  sink the flight recorder writes to. This needs an explicit decision —
  redact by default and require an opt-in to log real key values, or
  log a hash — **before** implementation, not discovered after; left as
  an open question below rather than decided unilaterally here.
- **No audit trail.** This RFC gives no record of "which thread held
  which lease for how long," beyond the stuck-holder warning above. A
  post-mortem on a timeout storm has the flight recorder's aggregate
  counts (matching the existing per-domain `held`/`grants`/`denials`
  banner shape) and nothing more granular. A real per-acquisition audit
  log is a larger feature, not proposed here.

## Performance

- Sharded (256-way, fixed) rather than one global lock — see "The lease
  table," above.
- Hashing happens before any lock is taken.
- No number is claimed for "cheap." This RFC has not benchmarked an
  implementation, and says so rather than asserting an unverified
  target — see "Open questions."
- Convoy effects under heavy contention on one specific key are not
  eliminated by sharding (sharding spreads *unrelated* keys across
  locks; a genuinely hot single key still serializes everyone who wants
  it, through that one shard's queue, by design — that is what mutual
  exclusion means, not a bug in this design).
- Thread-per-waiter: each blocked caller is a parked OS thread for the
  duration of its wait, same as `join`/`recv` already are
  (`docs/LANGUAGE.md` §7) — not a new cost model, but worth stating so
  "thousands of contended waiters" is understood as "thousands of
  parked threads," the same trade `spawn`'s own reused-worker pool
  already makes elsewhere in this runtime.
- Latency under sustained overload degrades to repeated
  `NIRDOSHA_GUARD_TIMEOUT_MS`-long stalls followed by errors — a real
  metastability risk under load spikes the review named correctly.
  Backoff/jitter on the *caller's* retry is the recommended mitigation
  (the same posture `transact`'s own retry guidance takes), not
  something `guard` itself enforces — `guard` returns a caught, typed
  error; what a caller does with `Err(GuardTimeout)` (immediate retry,
  vs. backoff, vs. giving up) is the caller's decision, same as any
  other `Result`.

## Effect on the permission model

None. `guard` has no interaction with `requires(role/claim:...)`,
`RoleView`/`ClaimView`, `acquire`, or `screen`/`dashboard` view gating —
it is a concurrency-control primitive, not an identity/authorization
one.

## Compatibility

`guard` is a new reserved keyword. No file in this repository uses
`guard` as an identifier today (checked). External `.nir` code that
does would break — this is disclosed as a real, if likely narrow,
compatibility cost, not claimed as provably impossible to hit.

Every existing kernel FFI symbol (`nfr`, `transact`, the 7 built-in
domains) is statically linked into each compiled binary at `nirdosha
build` time, not dynamically resolved against a separately-versioned
runtime — the new `nir_kernel_guard_enter`/`nir_kernel_guard_exit`
symbols this RFC adds follow the same model. There is no "binary
compiled against an older runtime" version-skew hazard the way there
would be for a dynamically-loaded library: a binary either was compiled
with a `nirdosha` that has these symbols, or it wasn't and `guard`
wasn't available to write in the first place. This differs from the
prior draft's unaddressed concern by removing the premise, not by
adding handling for a case that doesn't apply to this build model.

`examples/killer_demo/race_probe.nir` is **not** rewritten to use
`guard` by this RFC — nor, as of "Implementation plan," above, by
anything else this RFC currently recommends building. **B2, updated for
the no-`guard`-yet world Phase A actually found**: its own header
comment should still be updated (`expected_race`, pointing at this RFC)
to name the fixes that actually exist today rather than a Phase-C
feature that isn't proceeding — `race_probe_atomic.nir` for the
single-statement/single-row case, and `race_probe_booking.nir`/
`race_probe_booking_pg_exclude.nir` (A1/A2, both real, both measured)
for the multi-statement/multi-row case this document originally reached
for `guard` to solve. `guard`/`race_probe_guarded.nir` remains a
plausible *future* addition only if Phase C ever reopens (see
"Implementation plan"), not the thing B2's header update should point at
today. Leaving `race_probe.nir` silently as-is, still compiling clean
under `nirdosha verify` with no warning pointing anywhere, is the
adoption hazard an earlier review named; this paragraph is the fix for
that — carried out with whatever `nirdosha verify` static detection
exists at the time (today, none; "Static detection," above, is itself
Phase-C-gated), not assumed to require `guard` shipping first.

`docs/LANGUAGE.md` §7's race-freedom paragraph (already amended once
this session to disclose the `db` boundary honestly) needs one further
sentence **now**, not "once `guard` ships" — Phase A already gives it
real content: `db`-state races are *addressable today*, with ordinary
SQL discipline (a single conditional DML statement, or a schema-level
constraint where the backend supports one — A1/A2, both measured, both
real) — not, as an earlier revision of this sentence assumed, only once
a new language primitive lands. This doc edit is part of Phase B3
(above), not contingent on Phase C; it is named here so it isn't the
thing that quietly doesn't happen.

## Rejected alternatives

- **Compile-time-only enforcement** (blocking at column, not row,
  granularity): rejected — serializes every caller touching a table
  against every other one regardless of which row, destroying real
  concurrency for unrelated keys.
- **Extending the existing counting-domain model**
  (`DomainId`/`BUILTIN_DOMAINS`): rejected — those answer "how many are
  held," never "which one, by whom," which is the entire question
  `guard` exists to answer. A purpose-built keyed table is a better fit
  than retrofitting identity onto a counter.
- **Fully automatic locking keyed off `db_execute`/`db_query`'s own
  bind values, no explicit `guard` syntax**: rejected — requires
  parsing arbitrary SQL to find which bind value is the row identity
  (fragile, open-ended), and cannot distinguish "read for display" from
  "read as the first half of a read-modify-write" the way an explicit
  block boundary does unambiguously.
- **An annotated row key on `db_execute` itself**
  (`db_execute(..., row_key: from_id)`) instead of a separate `guard`
  construct: considered, and rejected for conflating two different
  concerns — `guard`'s block boundary is what lets it cover a critical
  section spanning *more than one statement and more than one
  subsystem* (the payment example's `http` call in the middle), which
  an annotation on a single `db_execute` call cannot express by
  construction. This alternative is strictly weaker for the motivating
  case this revision uses, not just differently shaped.
- **A distributed/cross-process lock**
  (`SELECT ... FOR UPDATE`-backed, or similar): out of scope for this
  RFC, not rejected as a bad idea — a real cross-process version is a
  separate, larger design (`db`-engine-backed row locking), and this
  RFC's in-process scope is stated as a limit, not an implicit claim of
  more.
- **Forcibly revoking a stuck holder's lease past a hold-time bound**:
  rejected as unsound — see "Holders are not time-bounded," above; a
  revoked-but-still-running holder and its replacement can both believe
  they hold exclusive access simultaneously, which is a worse bug than
  the unbounded stall it would replace.
- **Extending the existing `join`/`recv` deadlock detector to also
  watch `guard` waiters**: not rejected outright — genuinely useful
  future work — but not required for this RFC to be soundly
  implementable, because `guard`'s own bounded timeout already turns
  every deadlock into a caught, typed error rather than a silent hang,
  which is the property a detector would otherwise exist to guarantee.
  A detector would turn a 5-second-per-attempt stall into a
  millisecond-scale abort — a real operational improvement, not a
  correctness one — and is left as an "Open questions" item rather than
  blocking this design on a second, independently-scoped subsystem.

## Open questions

- **`HandleTable::with` holds its table-wide mutex across a blocking
  call — a real, confirmed kernel bug, out of this RFC's scope to fix,
  but affecting more than `guard`.** Found while running A3 (above):
  `crates/runtime-kernels/src/kernel/mod.rs`'s `HandleTable<T>::with`
  runs its closure while still holding `self.handles`'s lock, and
  `nir_db_execute`/`nir_db_query` run their actual (blocking) Postgres
  call inside that closure — so any two `.nir` threads whose Postgres
  operations have a server-side ordering dependency (one waiting on a
  lock the other holds) deadlock the whole process's `db` layer, not
  just those two threads, confirmed with both `pg_advisory_xact_lock`
  and a plain `SELECT ... FOR UPDATE`. This is a pre-existing bug this
  RFC didn't introduce and isn't the right place to fix, but it blocks
  A3's concurrent-correctness check (see A3, above) and blocks reusing
  `HandleTable` for `guard`'s own lease entries (see Phase C's first
  bullet, above) until fixed. Filed as its own issue, independent of
  whether this RFC's Phase C ever proceeds:
  [kannamma-labs/nirdosha#37](https://github.com/kannamma-labs/nirdosha/issues/37).
- **Shard count (256) is unmeasured.** No benchmark backs this number.
  Needs a real contention benchmark (workload, thread count, key
  cardinality all specified) before or shortly after implementation —
  this RFC does not claim the number is right, only that sharding-at-all
  is the right shape.
- **Telemetry redaction for logged keys** (Security, above) — needs a
  decision before implementation.
- **Lock-ordering / deadlock-avoidance beyond the bounded timeout for
  the nested-`guard` case** the multi-key form doesn't cover — real
  future work, not this RFC.
- **Extending the `join`/`recv` deadlock detector to `guard` waiters** —
  see "Rejected alternatives"; a real improvement, deliberately not
  required for v1.
- **`race_probe_guarded.nir`** — a new example demonstrating `guard`
  itself — is gated on Phase C the same way the rest of "Design" is:
  not written as part of this revision, and not scheduled, because
  Phase C is not currently authorized (see "Implementation plan,"
  above). The multi-statement invariant this document actually
  motivates already has real, measured, checked-in examples that don't
  need `guard` — `race_probe_booking.nir` (A1) and
  `race_probe_booking_pg_exclude.nir` (A2) — which is a different
  status than "not yet written," worth not conflating with this bullet.
- **A real cross-process/`db`-engine-backed version** — out of scope
  here; a natural RFC of its own if the in-process guarantee turns out
  to be insufficient for a real deployment's needs.
- ~~Whether an internal kernel panic can actually unwind past the
  `.nir`↔Rust FFI boundary while a lease is held~~ — resolved in this
  revision, not still open: "Holder death," above, gives the checked
  argument (Rust aborts a panic attempting to unwind across a plain
  `extern "C"` boundary rather than continuing past it, matching
  `thread_pool.rs`'s own documented behavior) and withdraws the earlier
  RAII mitigation this bullet used to reference as unproven — kept here,
  struck through, only so a future reader who remembers the RAII version
  doesn't wonder where the question went.
- **The namespace tag being a hard compile-time requirement, not a
  warning, raises the stakes on a case the prior warning-based design
  could leave to judgment**: a resource type with only ever one call
  site gets no benefit from a tag (there's nothing else to collide
  with) but still pays the syntax cost, as a build error, not a
  dismissible warning. Whether that's the right tradeoff, or whether a
  single-key `guard(key)` (no tag) should be allowed when `nirdosha
  verify` can *prove* no other `guard(...)` call site in the program
  ever uses a bare `Int`/`Str` key of the same shape (a real, checkable
  analysis, just not one this revision specifies), is left open.
- **A `spawn`-inside-`guard` lint** (see "Spawned work does not inherit
  the lease") — plausible, not committed to in this revision.
- **Items raised across three review passes that have never been
  designed, fixed, or previously listed here — named explicitly now so
  they stop silently disappearing between revisions**: a per-call-site
  configurable timeout (`guard(key, timeout_ms: N) { ... }`) instead of
  one process-wide `NIRDOSHA_GUARD_TIMEOUT_MS`; a non-blocking
  `try_guard` for a "busy, do something else" pattern with no wait at
  all; an explicit fence stating `guard` blocks the calling OS thread
  and is not meant for a future async/await execution model, should
  `.nir` ever grow one; whether `guard()` with zero keys is a type error
  (the grammar says "one or more," but no `TypeErrorKind` for the
  zero-key case is named anywhere in this document); whether
  `NIRDOSHA_GUARD_TEST_NAMESPACE` colliding with a caller's own literal
  `Str` key (both happen to be the same string) silently merges two
  namespaces that were meant to stay separate; whether `transact`
  lexically wrapping `guard` (the discouraged reverse of "Interaction
  with `transact`"'s required ordering) should get its own
  `nirdosha verify` lint, symmetric with `NestedGuard`'s lexical check,
  rather than being convention-only; whether a thread that repeatedly
  re-enters the same key via reentrancy (a long-lived outer `guard`
  whose body loops, calling a helper that itself guards the same key
  every iteration) can starve queued waiters for the outer block's
  entire duration, and whether that's acceptable; how a multi-statement
  literal SQL string (`"UPDATE a ...; UPDATE b ..."`, if `.nir`'s
  `db_execute` even accepts one — not confirmed either way in this
  document) interacts with the table-name scan "Static detection"
  describes; whether many `guard` waiters parked on the same
  `thread_pool.rs` worker pool the *holder*'s own continuation (or
  whatever unblocks the holder) is also queued on can produce a
  cross-primitive liveness problem distinct from the lock-ordering
  deadlock already discussed; and a staging/rollout plan for landing
  the keyword, the runtime subsystem, and the three lint passes without
  an intermediate state where compiled code references symbols an
  older-built `nirdosha` binary doesn't have. None of these are solved
  by this revision — they are named so the next review's time goes
  toward genuinely new findings, not re-discovering ones already known
  and previously left unrecorded.
