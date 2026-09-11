---
description: Machine-check whether the current .nir implementation still satisfies every spec.md requirement, via nirdosha certify's real verdict.
---

# Nirdosha converge

Answer one question honestly, for every requirement in this feature's
`spec.md`: **is there a Nirdosha contract for it, and does that
contract currently hold?** This never reports a requirement as
satisfied on the strength of reading the code and judging it looks
right — every claim in the report must come from a real
`nirdosha certify`/`nirdosha verify` run against the actual, current
`.nir` source, not from inspection alone.

## Prerequisites

Same as `/speckit.nirdosha-speckit.contracts`: `spec.md` exists,
a `.nir` implementation exists, `nirdosha` is reachable.

## Steps

1. **List every requirement in `spec.md`** (`FR-...`/acceptance
   criteria), same selection rule as `contracts.md`: keep the ones that
   are checkable properties of one function's behavior, note which
   `.nir` function each targets.

2. **For each requirement, find its `validate <fn_name> { ... }` block**
   in the current source, if one exists. Read the actual `pre`/`post`
   predicates — do not assume they still say what they said when
   written; the implementation may have changed since.

3. **Judge whether the *predicate itself* still states the requirement**
   (not yet whether it holds — that's step 4). A contract that compiles
   and is `PROVED` but checks the wrong thing is a false convergence
   signal, the specific failure mode this command exists to catch.
   Flag any mismatch here explicitly, in the report, even before
   running anything.

4. **Run `nirdosha certify <file.nir>`** (once per file covering this
   feature's functions — not once per requirement; batch by file). Read
   the real JSON: `verdict_summary.verdict`, `evidence_tier`, and for
   each obligation, its own status.

5. **Build the convergence table**, one row per `spec.md` requirement:

   | Requirement | Target fn | Contract exists? | Predicate matches spec? | Verdict | Evidence tier |
   |---|---|---|---|---|---|

   A requirement with no `validate` block at all is `NO CONTRACT`, not
   a blank — that is itself the finding, not an omission from the
   report. A requirement whose contract's predicate doesn't match what
   `spec.md` actually says is `MISMATCH`, regardless of what its
   verdict says (a `PROVED` verdict on a mismatched predicate proves
   the wrong thing).

6. **Summarize convergence as a fraction**: `<N PROVED and matching> /
   <total checkable requirements>`. Never round this up, and never
   describe partial convergence as "done" — an agent or a human reading
   this report needs the honest fraction to decide what to do next, not
   a reassuring rollup.

## What counts as full convergence

Every checkable requirement has a `validate` block, that block's
predicate genuinely states the requirement (step 3), and
`nirdosha certify` reports `PROVED` for it. `UNKNOWN` obligations and
requirements this command's own selection rule excluded (non-functional
requirements, UI-only requirements) are named in the report as their
own category, not folded into either "converged" or "not converged" —
see `/speckit.nirdosha-speckit.contracts`' own notes on what `validate`
can't express.
