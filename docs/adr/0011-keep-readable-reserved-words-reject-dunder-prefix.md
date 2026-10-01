# 0011: Keep reserved words readable — reject the `__`-prefix collision-elimination proposal

Date: 2026-09-10
Status: accepted

## Context

A `hi` `:generate` run gave up after burning all 3 bounded self-repair
attempts on `parse error ... expected identifier, found State`: the
model had named a struct field `state`, `state` is a reserved word
(`workflow` state machines), and the pre-fix diagnostic printed
`Tok`'s Rust variant name instead of the source text, so the model
couldn't map "State" to the word it had actually written (root cause
and fix in `token.rs`'s `Display for Tok` doc comment).

That failure prompted a design question: should every reserved word be
spelled with a `__` prefix (`__state`, `__fn`, `__open`, ...) so a
natural identifier can never collide with one again? The idea was
studied against the language's design goals and measured where the repo
allowed measurement:

- **Field data**: exactly 1 observed keyword collision across the three
  root-caused `:generate` failures in repo history (missing `fn main()`
  — a prompt conflict; missing `%` — a language gap; the `state`
  collision — a diagnostic gap). One data point, not a rate — but the
  collision class is also the only one of the three already fully fixed.
- **Post-fix cost per collision**: ~1 bounded retry. The diagnostic now
  names the exact word, `hi_llm.rs::self_repair_hint` says to rename it,
  and system-prompt rule 17 lists the words that actually collide with
  app vocabulary. The loop tolerates 3 attempts.
- **Three collision families exist**, and `__`-prefixing only touches
  the first: keyword-as-identifier (45 words); reserved type names as
  identifiers (20, `struct Text { str: i64 }`); and flat-namespace name
  collisions (`enum ReportType { SAR }` vs prelude `CurrencyCode`, user
  fns vs compiler-generated `__workflow_*` — `paste-anywhere-prompt.md`
  rule 1 documents the real `SAR` case). Families 2 and 3 are untouched
  by any keyword respelling.
- **"Zero" is not reachable even in family 1**: deliberate collisions
  persist by construction, and models *invent* `__`-prefixed
  identifiers on their own (Python's dunder convention — the same
  convention this repo's own `__workflow_overdue`-style user-callable
  builtins teach in every workflow example). The failure class would
  shrink and migrate into the `__` namespace, not vanish.
- **Row 7 (`docs/goal.md`)**: its own catch says LLM code quality "is
  dominated by training-corpus volume — no grammar design fixes a
  brand-new language starting at zero corpus." Nirdosha's whole strategy
  is to sit near the priors (Rust/Go-shaped keywords, snake_case) and
  patch enumerable deviations (the prompt's 17 rules).
  `__if`/`__let`/`__struct` have zero corpus prior, and the prior pulls
  toward the un-prefixed spelling — trading a rare, 1-retry failure for
  a frequent, 1-retry one (spelling drift on every generate) plus a
  permanent pass@1 regression, row 7's actual metric. Row 6 (learning
  curve) regresses too (`__if cond { __return x }`).
- **Precedent points the other way**: C/C++ reserve `__` for
  *identifiers*, not keyword spellings; Rust added `r#match` as an
  escape hatch while keeping keywords readable; Scala 3/Swift moved to
  contextual keywords. Nirdosha already follows that pattern in its body
  keywords (`data`, `field`, `action`, `expose`, `pre`, `post`, ...).
- **Migration scale** if it were adopted: 104 `.nir` files / 8,420
  lines / 4,153 keyword tokens + 1,757 type-name tokens, the
  lexer/parser/codegen tables, `docs/GRAMMAR.md`/`LANGUAGE.md`/
  `WORKFLOW.md`, and all 8 `agent-skills/nirdosha` files — one of which
  is the `:generate` system prompt via `include_str!`.

## Decision

**Rejected: no `__` prefix on reserved words. Keywords stay readable.**
Collision incidence is managed at the layers that don't tax every
generation:

- **Shipped (2026-09-10)**: actionable diagnostics (`Display for Tok`,
  all 15 parser "found ..." sites) + the `self_repair_hint` rename arm —
  a collision now costs ~1 bounded retry, not a give-up.
- **Shipped (2026-09-10)**: reserved-words rule in the `:generate`
  system prompt and all agent-skill copies, naming the collision-prone
  subset (`state`, `open`, `serve`, `screen`, ...) and the synonym
  advice.
- **Future lever C, if measurements justify it**: a deterministic
  candidate-name guard in `populate_candidates`/`add_candidate` —
  reject or suggest a rename for candidate names hitting the 65
  reserved words at `:prompt` time, before human confirmation, where a
  rename is free instead of a burned LLM round trip. ~20 lines, no
  grammar change.
- **Future lever D, for the declaration-introducer words only**:
  contextualize them (`state` first — `parse_workflow_decl` already
  matches `data` contextually by identifier text in the identical slot),
  which makes those English words free identifiers without touching
  core syntax. Note this reverses `token.rs`'s corpus-justified
  reserved-vs-contextual decisions for that subset, and is *infeasible*
  for expression-position keywords (`open`, `connect`, `send`, ...) —
  their call-shape/type-inference ambiguity is why they're reserved.
- **In the drawer**: a Rust-style `r#word` escape hatch.

**Trigger for revisiting**: persist diagnostic-class counts from the
`:generate` retry stream (the `on_log` callback already sees them); if
keyword-collision incidence across ~50 runs is high enough to matter,
take lever C, then D — not a prefix.

## Consequences

- Collisions with the 65 reserved names remain possible; each costs
  about one self-repair round trip and produces an unambiguous
  diagnostic. This is accepted as cheaper than taxing every generation
  for a rare case.
- The flat-namespace collision families (prelude enum variants,
  builtins, `__workflow_*` generated names) remain mitigated only by
  prompt rules today — if they start failing generate runs, they get
  their own study, not a prefix.
- New keywords added later (e.g. a future `guard`, RFC 0015) will be
  breaking additions like today's are, rather than non-breaking
  `__`-namespaced ones. That trade is knowingly accepted while the
  language is pre-stability; a namespace rule ("identifiers beginning
  `__` are reserved for the implementation") can be adopted later
  without ever having respelled the keywords.
- The honest downside of this decision: the words the grammar *does*
  steal from app vocabulary (`state` especially) stay stolen until
  lever D is taken, and every `struct Player { state: ... }` a model
  writes will pay one retry — the same retry that used to be a give-up.