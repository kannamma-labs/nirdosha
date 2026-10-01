---
description: Draft or update Nirdosha `validate` contracts from spec.md's testable requirements, and confirm each one compiles.
---

# Nirdosha contracts

Turn this feature's `spec.md` requirements into real, checkable Nirdosha
`validate` blocks — never invented from scratch, never guessed at: each
contract must trace back to one specific requirement line, and every
contract you write must actually be run through `nirdosha verify`
before you report it as done.

## Prerequisites

- The current feature's `spec.md` exists and has at least one
  functional requirement (`FR-...`) or acceptance criterion.
- A `.nir` implementation for this feature exists, or is being written
  in the same session — this command drafts contracts *for* an
  implementation, it does not invent the implementation itself.
- `nirdosha` is on `PATH` (or point at it via `NIRDOSHA_BIN` if your
  environment sets that convention — check first rather than assuming).

## Steps

1. **Read `spec.md`.** List every requirement that is a genuine,
   checkable property of a function's *behavior* — an input/output
   relationship, an invariant, a bound. Skip requirements that are
   about UI copy, non-functional/performance targets `validate` can't
   express (those belong in an `nfr(...)` annotation instead, not
   here), or anything that isn't really about one function's contract.
   For each one you keep, note its requirement ID (`FR-003`, etc.) and
   which `.nir` function it constrains.

2. **For each kept requirement, look at the target function's current
   signature and body.** Decide the exact `pre`/`post` predicate that
   states the requirement precisely — in terms of the function's own
   parameter names and `result` (`validate`'s own bound name for the
   return value). If the requirement genuinely can't be expressed as a
   predicate over parameters/`result` (e.g. it depends on an external
   side effect `validate` has no way to observe), say so explicitly in
   your report instead of forcing a fake contract — an honest "cannot
   express as a validate predicate, needs a different check" is a
   correct, useful answer.

3. **Write or update the `validate` block** for that function:

   ```nirdosha
   validate <fn_name> {
       pre: <precondition, if any — omit the line if there is none>
       post: <postcondition stating the requirement>
   }
   ```

   If a `validate` block for this function already exists, merge into
   it — a function has exactly one `validate <fn_name> { ... }` block,
   never two. Combine multiple conditions on one `pre`/`post` line with
   `&&`, not repeated `pre:`/`post:` keys.

4. **Run `nirdosha verify <file.nir>` immediately after writing the
   contract** — not at the end of the whole batch. Read the JSON
   verdict:
   - `"PROVED"` — real, done, move to the next requirement.
   - `"DISPROVED"` — the *contract* or the *implementation* is wrong;
     the verdict's `contracts.obligations[].detail` names the exact
     counterexample binding. Decide which side is at fault (usually:
     the requirement had an implicit precondition you didn't state —
     e.g. "quantity is at least 1" — add it to `pre:`, or the
     implementation genuinely has the bug the requirement describes,
     in which case fix the implementation, not the contract). Never
     loosen a `post:` just to make it pass — that defeats the entire
     point of writing it.
   - `"UNKNOWN"` — Z3's Tier 1 couldn't model this specific predicate
     shape (a known, disclosed compiler boundary — non-integer
     parameters, or a predicate built on a division result, are
     current examples; see the nirdosha repo's
     `docs/PUBLIC_ROADMAP.md`). Report it as `UNKNOWN`, honestly — do
     not present it as `PROVED`, and do not silently drop the
     contract just because Z3 can't help here; the contract is still
     documentation of the requirement even when it can't be
     formally discharged yet.
   - A parse/type error — fix the contract's own syntax and retry
     before doing anything else.

5. **Report back a small table**: requirement ID → target function →
   verdict (`PROVED`/`DISPROVED`/`UNKNOWN`) → one-line note. Do not
   report a requirement as "done" unless its row says `PROVED`.

## What this command does not do

It does not invent test cases, does not run the compiled binary, and
does not touch anything outside `validate` blocks and the specific
function bodies a `DISPROVED` result required fixing. For a full
requirement-by-requirement coverage report against the *current* state
of the code (not just what you just wrote), use
`/speckit.nirdosha-speckit.converge` instead.
