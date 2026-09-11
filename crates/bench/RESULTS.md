# Benchmark harness v1 — results

`nirdosha-master-plan.md` Part 3 Sprint 2's "Benchmark harness v1." This
is a real run, not simulated numbers: three tasks, generated for real
against `gemini-2.5-flash` (Google's OpenAI-compatible endpoint,
`GOOGLE_API_KEY` in this environment), self-repaired for real against
this compiler's own diagnostics (`nirdosha::hi_llm::generate_from_task_prompt`,
the exact loop `nirdosha hi`'s Generate mode uses), and scored for real
by `nirdosha certify`'s own JSON verdict. Raw artifacts —
`results/summary.json` and the generated `.nir` source for every task
that compiled — are committed alongside this file.

## What this is *not* (disclosed up front, not buried)

The master plan's own comparison matrix for this item is: *"Nirdosha vs
TypeScript vs Rust vs plain-LLM vs LLM+XGrammar vs LLM+Imandra ... on
injection / type confusion / overflow / deadlock classes; includes
AlgoVeri/Vericoding tasks → direct published comparison vs Kōdo's 20/20
claim."* This run covers none of the cross-language or cross-tool
columns, and doesn't reproduce Kōdo's benchmark:

- **TypeScript/Rust baselines** need a parallel generate-then-
  statically-analyze pipeline in each language, scored for the same
  failure classes — a second harness, not built here.
- **LLM+XGrammar/LLM+Imandra** need those third-party tools installed
  and wired up; neither is present in this repo or this environment.
- **AlgoVeri/Vericoding** needs Kōdo's own benchmark corpus
  (arXiv 2602.09464), not available locally.

What *is* real: Nirdosha's own generate → self-repair → verify loop,
measured against a live model, for three tasks.

## Why three tasks, and why these three

The master plan names four failure classes: injection, type confusion,
overflow, deadlock. Designing a *fair, solvable* Nirdosha task for each
turned up a real finding, not assumed going in:

- **`type confusion`, taken literally as "mix two integer widths"
  (e.g. an `i8` parameter combined with an `i64` one), is unsolvable in
  today's Nirdosha.** There is no int-to-int or int-to-float conversion
  builtin at all — `ast.rs`'s builtin list has `dec_from_i64` for the
  `Decimal` money type and nothing general-purpose. A task requiring
  that combination has no correct answer to self-repair toward; every
  attempt would fail identically, testing nothing. Reframed instead as
  "does the model reach for unnecessary float semantics where integer
  arithmetic suffices" (`average_no_float_confusion`, below) — solvable
  purely by the model choosing consistent types, no missing builtin in
  the way.
- **`deadlock` is left out of v1 entirely.** Scoring it needs a
  concurrent task with an automatable race/deadlock detector, a
  materially bigger lift than the other three within this pass.

That leaves three tasks: one per remaining class.

## Results (one run, `gemini-2.5-flash`, 2026-09)

| Task | Class | Outcome | Attempts | `nirdosha certify` verdict | `evidence_tier` |
|---|---|---|---|---|---|
| `injection_safe_lookup` | injection | **gave up** | 3/3 (never compiled) | — | — |
| `overflow_checked_multiply` | overflow | self-repair rescued | 2/3 | **PROVED** (`contracts_proved: 1`) | `proved` |
| `average_no_float_confusion` | type confusion | self-repair rescued | 3/3 | UNKNOWN (`contracts_unsupported: 1`) | `unknown` |

`pass@1: 0/3`, `self-repair-rescued: 2/3`, `gave up: 1/3`,
`certified PROVED: 1/3`. Small-N, one run, illustrative — not a
statistically robust claim about model capability (see "What this does
and doesn't prove" below).

### `overflow_checked_multiply` — the real proof

The task asked for `order_total_cents(unit_price_cents, quantity)` with
`validate order_total_cents { pre: unit_price_cents >= 0 && quantity >= 1, post: result >= unit_price_cents }`.
Attempt 1 failed to compile; the self-repair loop fixed it on attempt 2.
The final program (`results/overflow_checked_multiply.nir`) got a real
`PROVED` verdict with `contracts_proved: 1` — Z3 formally proved the
postcondition holds for every `i64` input satisfying the precondition,
not a vacuous pass (an earlier draft of this task had no precondition
at all, and Z3 correctly *disproved* it by finding `quantity = 0` as a
counterexample — a real finding about task design, not a compiler bug,
kept here rather than quietly edited away).

### `average_no_float_confusion` — a real, disclosed Tier-1 gap

The task asked for `average_score(total_points, num_students)` with
`validate average_score { pre: total_points >= 0 && num_students >= 1, post: result <= total_points }`.
The model never introduced a float anywhere (the actual property this
task was designed to probe) — every attempt used plain `i64` division.
It still took 3 attempts to compile, and the final program's verdict is
**`UNKNOWN`**, not `PROVED`: `contract_check.rs`'s Tier 1 can't model a
predicate built on an integer-division result at all today
(`contracts_unsupported: 1`), regardless of the precondition. This is a
real, disclosed compiler gap surfaced by this run, not a task-design
mistake — Tier 1's own documented scope is overflow/division-by-zero/
array-bounds obligations *within a function body*, not arbitrary
predicates a `validate` block states *about* a division's result.

### `injection_safe_lookup` — a real, disclosed self-repair-loop gap

The task asked for a `db`-backed lookup keyed by an `i64` id, returning
`Result(Text, ErrorCode)`. The model never attempted a concatenated,
injectable query at any point across all 3 attempts — Nirdosha's own
`str`-has-no-concatenation rule (AGENTS.md rule 3) makes that
inexpressible, so this class's central guarantee held throughout,
`gave_up` notwithstanding. All 3 attempts failed on unrelated syntax:
the final diagnostic was `expected an expression, found `{`` — the
match-arm-block mistake AGENTS.md rule 9 documents as "the single most
common mistake an LLM makes writing Nirdosha," most likely inside the
`Result`/error-handling composition this task's shape invites. The
existing `self_repair_hint` (`hi_llm.rs`) doesn't yet special-case this
diagnostic with a worked db/`Result` example — a real, concrete
follow-up this run surfaced, not a task flaw: the task's own guarantee
(no injectable query) was never at risk; the loop's rescue rate for
*this specific idiom, in this specific composition*, was the thing
that fell short.

## What this does and doesn't prove

- **Does not prove**: a general pass@1/self-repair-rate figure for
  Nirdosha, or a claim that Nirdosha beats any other language/tool on
  these classes — 3 tasks, one model, one run each is a demonstration
  that the harness and the scoring pipeline are real and wired up
  correctly, not a statistically powered result.
- **Does prove**: the harness genuinely drives a live model, genuinely
  self-repairs against this compiler's real diagnostics, and genuinely
  scores outcomes off `nirdosha certify`'s own JSON — including
  reporting `UNKNOWN` and `gave_up` honestly rather than rounding either
  up to a pass. The overflow task's `PROVED`/`contracts_proved: 1` is a
  real Z3 proof of a real property, reproducible with the committed
  source and `nirdosha certify`.
- **Separately real, not superseded by this run**: the underlying
  by-construction guarantees this harness's task design leans on (no
  string concatenation, so an injectable query is inexpressible; Tier-1
  Z3 proof for overflow/division/bounds within a function body) are
  documented and tested independently in `docs/LANGUAGE.md` and
  `crates/compiler/tests/`, not established for the first time by this
  benchmark.

## Reproduce it

```sh
export NIRDOSHA_LLM_PROVIDER_KEY=<a Google AI Studio API key>
export NIRDOSHA_LLM_PROVIDER_MODEL=gemini-2.5-flash
export NIRDOSHA_LLM_PROVIDER_BASE=https://generativelanguage.googleapis.com/v1beta/openai
cargo run -p nirdosha-bench
```

(`crates/bench/run_gemini.sh` wraps exactly this, reading
`GOOGLE_API_KEY` from your environment.) Any OpenAI-compatible endpoint
works — set `NIRDOSHA_LLM_PROVIDER_KEY`/`_MODEL`/`_BASE` accordingly, or
just `OPENAI_API_KEY` for a real OpenAI key with the default model
(`hi_llm.rs::resolve_activation`).
