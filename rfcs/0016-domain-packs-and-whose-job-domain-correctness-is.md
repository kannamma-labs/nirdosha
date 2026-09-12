# RFC 0016: Domain packs — pre-baked, non-waivable invariants, and whose job domain correctness actually is

> **Status: design capture of a conversation (2026-09-11), prompted by
> a real observed failure; zero code shipped for this RFC yet.** The
> gap below is not hypothetical — it was demonstrated on 2026-09-11:
> `hi`'s generate, run against a confirmed 7-unit banking graph
> (`~/temp2`, first attempt, compiled clean, published, ran the full
> banking day), shipped a program with **zero `validate` contracts**.
> `nirdosha verify` on that output reports `verdict: PROVED`,
> `contracts proved: 0`. PROVED 0/0 is not safety; it is silence.
> The machinery this RFC proposes around that gap does not exist, and
> the pieces it composes from (`contract_check.rs`'s Z3 proofs,
> `hi_graph.rs`'s confirm/lock model, `hi_llm.rs`'s repair loop) all
> exist today and are cited by their real names below. Treat this as
> a proposal to react to and cut down.

## Motivation

The question that opened this RFC, asked by the user who watched the
PROVED 0/0 run land:

> I am trying to understand whose job it is to ensure that if it's
> banking software, the fundamental rules of any banking system hold
> good. Shouldn't there be a domain expert engaged to ensure all the
> fundamental things related to the banking domain are pre-baked into
> the node, and that could not be changed no matter what happens?

The honest answer today is: **nobody's job.** The current pipeline
distributes domain knowledge across three places, none of which
guarantees it:

1. **The user's prompt** — the only place banking rules appear at all
   in the observed run ("whole cents", "balance can never go negative").
   A convention, freely droppable by the next sentence.
2. **`hi_llm.rs`'s `units_prompt`** — confirmed units carry their
   `CandidateUnit` attributes into the generate prompt, including
   "validate contract …" attributes proposed during decompose
   (`hi_llm::populate_candidates`). These are rendered as things to
   honor. In the observed run the model dropped every one of them and
   nothing failed: `hi_llm::generate_program`'s only gate,
   `typecheck_and_build_check`, checks parse → typecheck → ownership
   → codegen (`codegen::build`). A program that typechecks but
   proves nothing about its money math passes.
3. **`contract_check.rs`** — the Z3-backed engine
   (`check_fn_contract`, `run_program_validates`,
   `check_program_contracts`) that *can* prove
   "balance_cents - debit + credit >= 0 for all inputs". It proves
   exactly the `ast::ValidateDecl`s present in the source, and is
   silent about the ones that aren't. Proof of stated contracts;
   no opinion about missing ones.

So the seam is precise and demonstrable: **decompose proposes
contract attributes, generate can drop them, publish doesn't notice.**
Everything else in the pipeline already has teeth — the transact
protocol's ordering rules (precheck/network/verify/commit/compensate,
`ast::TransactSlot` + `typeck.rs`), role gates (`requires(role:)`),
ownership/affine tracking, deny-by-default `serve` exposure — all are
compiler-enforced, and all hold *no matter what the model writes*.
Domain invariants are the one class of correctness that still depends
on the model's goodwill.

Two more observations sharpen the problem:

- **The prompt is knowledge, not enforcement.** The
  `agent-skills/nirdosha/paste-anywhere-prompt.md` system prompt
  (baked into `hi_llm.rs` as `NIR_SYSTEM_PROMPT`) already teaches the
  recipes. The 2026-09-11 field failures showed rules are *ignored
  under generation pressure* even when present — the repair loop's
  `self_repair_hint` arms exist precisely because of this. Any
  domain-guarantee design that ends at "we told the model" is not a
  guarantee.
- **The graph already has the two properties a fix needs**: units are
  *confirmed* (`hi_graph::confirm_node`) — `generate_program`
  includes every confirmed unit; and units can be *locked*
  (`hi_graph::lock_units_after_sync`) — the post-publish lock hi
  already applies to its own generated units. What's missing is a
  third: *non-waivable* — `hi_graph::waive_node` /
  `hi_graph::delete_node` will remove anything.

## Design

### The three-role split

The question "whose job is domain correctness?" has one answer per
role, and they compose:

1. **The domain expert bakes once.** Not per-generation review — a
   banking expert who watches every LLM output is a vigilance model,
   and vigilance is exactly what "no matter what happens" forbids.
   The expert's job is a one-time translation: knowing which few
   sentences about banking are load-bearing ("money is whole cents",
   "debits equal credits", "balances are non-negative", "spending
   above a limit is impossible by construction"), and writing them
   as `validate` contracts the compiler can prove.
2. **The graph holds them as pre-baked nodes.** A *domain pack* is a
   pre-authored, per-domain set of nodes injected into the graph
   *before* the user's prompt is read — confirmed, locked, and
   non-waivable. hi already treats confirmed units as mandatory
   generate input (`hi_graph::confirmed_units` feeds
   `generate_program`); a pack simply arrives pre-confirmed.
3. **The proof enforces forever.** The "no matter what happens" is
   Z3, not the expert: `contract_check::run_program_validates` /
   `verify_code` (`mcp_tools.rs`) prove the baked contracts against
   whatever the model produces — or the program doesn't publish.
   A reviewer's opinion can be argued with; a proof cannot.

### Solution ladder

Six solutions considered, in ascending strength. 1–2 are the status
quo; 3 is rejected; 4 is the recommended first slice; 5 is the full
answer this RFC proposes; 6 is the long-form endgame.

**1. Do nothing (prompt-only).** Demonstrated insufficient by the run
that prompted this RFC. Rejected.

**2. More prompt-side rules.** Teaching `paste-anywhere-prompt.md`
"always write validate contracts" raises the drop rate, but the
2026-09-11 field failures (a rule ignored under generation pressure,
fixed only by repair-time hint injection) show prompt-resident
knowledge is not enforcement. Keep as knowledge, never as guarantee.

**3. LLM domain-reviewer agent post-generate.** A reviewer persona
that checks drafts against a domain checklist. Soft: an LLM judging
an LLM is opinion, not proof, and fails the user's "no matter what
happens" test — the reviewer can be bored, rushed, or wrong the same
way the generator can. Rejected as *enforcement*; possibly useful
later as a *pack-drafting* aid for the human domain expert. Same
category as the "domain expert who reviews every generation"
model: vigilance, not law.

**4. The coverage gate (the minimal seam fix).** One new stage in the
generate loop: after `typecheck_and_build_check` passes, require that
every confirmed unit whose attributes demand a `validate` contract
actually has one in the draft, and that it proves. Concretely:

- new `hi_llm::contract_coverage_check(source, units)` — parses the
  draft (`ast::Program`), walks `fn.units`' demanded attributes
  against the `ast::ValidateDecl`s actually present
  (`fn.validates`), and runs `contract_check::run_program_validates`
  on those present;
- wired into `generate_program`'s loop alongside
  `typecheck_and_build_check`, with its failures fed to the existing
  repair conversation — a new `self_repair_hint` arm
  ("unit `post_ledger_entry_cents` demanded the validate contract
  `balance_nonnegative` — you dropped it; write it, and it must
  prove, not merely parse") and a machine-readable block per the
  2026-09-11 diagnostics format;
- the give-up error names the dropped contracts.

This alone closes the demonstrated seam: the model can no longer ship
a banking program with zero contracts *when the units demand them*.
It does not help when decompose never proposed the contracts — which
is why it is the first slice, not the answer.

**5. Domain packs (the proposed answer).** A pack is authored once
per domain, versioned alongside the repo, and injected into the
graph at prompt time:

- **Pack shape:** a list of *invariant nodes* — confirmed
  `CandidateUnit`s (kind: invariant/contract attribute carriers, or
  actual proven code units — see Open questions) with their driving
  text and `validate`-contract attributes — plus `RELATES_TO` edges
  (`hi_graph::add_relation`) to the domain's core units so the
  existing wiring contract (`hi_llm.rs`'s
  "How the components relate" section, fed by
  `hi_graph::confirmed_edges`) binds each invariant to the code that
  must obey it.
- **Injection:** a `hi_graph::load_domain_pack(conn, pack)` /
  `hi_api.rs` hook that runs before `populate_candidates`, writing
  each invariant via the existing `add_candidate` + `confirm_node` +
  `attach_attribute` + `lock_units_after_sync` path. Pack nodes are
  marked non-waivable.
- **Non-waivable flag:** a new boolean on packed nodes;
  `hi_graph::waive_node` and `hi_graph::delete_node` refuse
  ("packed domain invariant: not subject to waive — this is the
  banking domain's law, see domain pack v…"). `:confirm all` in the
  console can't remove them either — confirm is already idempotent;
  refusal lives on the waive/delete/unwaive side only.
- **Enforcement:** the coverage gate from solution 4 is what makes
  the pack bite — pack invariants arrive as demanded attributes, the
  gate fails any draft that drops them, Z3 fails any draft whose
  code violates them, and `certify_code` (`mcp_tools.rs`) attests
  the result.

What the pack does *not* need: a domain-expert LLM in the loop after
baking; new proof machinery; language changes.

**6. Certified proven-primitive libraries (endgame, deferred).** The
strongest form: the domain expert authors the ledger core *itself* —
`transfer`, `post_entry`, `authorize` as a certified `.nir` library
with embedded `ValidateDecl`s, proven once
(`mcp_tools::verify_code` + `certify_code`) — and generated programs
must *compose calls to* those primitives rather than re-derive
money math at all. The LLM stops inventing banking and starts
wiring it. This matches how real banking software behaves (you don't
re-invent double-entry; you call a proven ledger), and reuses
`hi_graph`'s existing lock model: pack code units arrive locked,
`units_prompt` presents them as available-and-mandatory, and the
wiring contract's orphan rule already demands real call sites.
Deferred: it needs a linking/story for multi-file programs the
compiler doesn't have yet (single-file today), and it changes what
"generate a program" means. Design note for that day: RFC 0014's
"v1 scope cut" (whole-program generation, file-granularity locking)
is the seam to revisit.

### What is already domain-baked (honest inventory)

To be clear about what the compiler already enforces without any
pack — these hold no matter what the model writes:

- the transact protocol's shape and ordering (`ast::TransactSlot`,
  `typeck.rs`: precheck/network/verify/commit/compensate/log, no
  partial commits, compensation mandatory);
- authorization: `requires(role:)` gates, `VerifiedIdentity`
  auto-injection by the serve route wrapper, deny-by-default
  exposure (`ExposedMutatingFnMissingRequires`);
- resource safety: ownership/affine tracking, no double-spend of a
  moved value by construction;
- whole-cents *representation*: `i64`/`dec128` exist as types — but
  the *choice* of whole cents is prompt convention, and no type
  distinguishes a money `i64` from a count `i64`. See Rejected
  alternatives.

## Effect on the permission model

Packs narrow, never widen. A non-waivable node is a *deletion* of
authority — the console user (and the model, and the operator) loses
the ability to waive the domain's law, which is the point. `:waive`
keeps working for everything the user's own prompt proposed;
`hi_api.rs`'s `/api/waive` and `/api/unwaive` routes return the
refusal for packed nodes. Nothing new becomes mutable.

## Compatibility

- **Graph schema:** `.nir/hi.db` gains a column (packed/non-waivable)
  on the nodes table — `hi_graph::open`'s schema path, additive.
- **Generate loop:** one new check stage + one new
  `self_repair_hint` arm; `MAX_SELF_REPAIR_ATTEMPTS` (4) unchanged —
  dropped-contract failures count against the same budget, so a
  model that keeps dropping pack contracts still exits.
- **Language:** none. Packs speak `validate` contracts and graph
  nodes; the DSL itself is untouched.
- **MCP:** `verify_code`/`certify_code` already read whatever
  `ast::Program` they're given — pack-injected or not — so the
  standalone `nirdosha mcp` surface is unaffected (and benefits: a
  packed program certifies its domain laws by default).
- **Existing graphs:** a pack injected into a graph that already
  holds confirmed units merges additively — invariants arrive as
  new confirmed units with `RELATES_TO` edges to existing ones; no
  existing unit changes.

## Rejected alternatives

- **A distinct `Money` type in the language** (so "cents" can't be
  added to "count"): representation, not law — it would enforce
  units but not `balance >= 0` or debits-equal-credits, and it costs
  a `typeck.rs`-deep change. Possibly worth it someday for
  arithmetic-safety ergonomics; not the mechanism for domain
  guarantees. Deferred.
- **Hardcoding banking checks into the compiler:** wrong layer and
  wrong domain boundary — the compiler must stay domain-agnostic;
  baking one domain's law into it makes every non-banking program
  pay for it and makes the next domain a compiler change.
- **LLM domain reviewer per generation:** rejected above —
  vigilance is not enforcement; also adds a per-run cost and a new
  failure mode (the reviewer's own ignorance) that the proof path
  doesn't have.
- **Grammar/sampling constraints (GBNF) as the fix:** syntactic
  containment can force `validate` *blocks* to exist (tokens), but
  cannot force them to *mean* the domain's law or to prove — that
  is typecheck/Z3 territory, not LL(1) grammar territory.

## Open questions

- **Pack authorship and format:** who writes the banking pack, in
  what format (markdown? a future `.nir` library file? JSON next to
  the graph?), and where does it version? The repo's own
  `agent-skills/nirdosha/` directory is the current precedent for
  human-maintained knowledge files — but packs must be
  machine-injectable, so they're likely a new format, possibly
  reviewed in the same "human-authored, never generated" spirit as
  `paste-anywhere-prompt.md`.
- **Invariant nodes vs. code units:** does a pack's
  "balance_nonnegative" invariant live as an attribute-carrying
  invariant node tied by `RELATES_TO` edge to the fn that must prove
  it (solution 5 as proposed), or as actual locked code (solution
  6)? The attribute form is shippable now; the code form is
  stronger but needs the linking story. Possibly both: attribute
  packs first, primitive packs when multi-file lands.
- **Who arbitrates conflicts:** two packs, or a pack plus a user
  prompt, demanding mutually inconsistent contracts (pack says
  "non-negative", prompt asks for an overdraft fee path). The proof
  engine already answers this — an unprovable draft fails — but the
  *UX* of "your prompt contradicts the banking pack" wants a
  dedicated diagnostic class, not a bare UNSAT.
- **Coverage-gate strictness:** demand a contract per *unit that
  asked for one*, or per *program*? Per-unit is the honest form and
  the one this RFC assumes; per-program would let the model
  satisfy the letter (one contract somewhere) while violating the
  spirit.
- **Does `:waive` stay honest?** Removing waive for packed nodes
  removes the existing escape hatch for genuinely-conflicting
  pack invariants. The refusal message pointing at the pack's
  version and author is the design here, but the operator-level
  override question (a human who *must* ship with a violated
  invariant, e.g. a regulatory change mid-flight) is genuinely
  open — options: a `--no-domain-pack` escape on `nirdosha build`
  (loud, logged) or nothing at all.
- **Relationship to RFC 0014's generative pipeline:** the coverage
  gate changes what "a successful generate" means in 0014's flow;
  0014's "Open questions" already anticipated stricter per-unit
  locking. This RFC should be reconciled with 0014's scope-cut
  notes when either moves.