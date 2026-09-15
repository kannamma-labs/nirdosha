# RFC 0019: OMNISCOPE — a unified static analysis engine (speculative, nothing built)

## Motivation

Nirdosha's own `PROVED`/`CHECKED`/`MONITORED`/`UNKNOWN` evidence-tier
model already commits to a tiered-precision story (`README.md`'s
tiered-evidence funnel; `docs/ROADMAP.md`'s post-seed `nirdosha check`
— "language-agnostic CHECKED tier: sandboxed behavioral validation of
non-`.nir` artifacts") but has never designed *how* a CHECKED-tier
engine covering arbitrary languages would actually work internally,
beyond "sandbox it and observe." This document proposes an answer:
synthesize eleven well-known static-analysis tools (mypy, Pyright,
ESLint, SonarQube, Semgrep, CodeQL, Snyk, Coverity, Dafny, Prusti,
Kani) into one engine built around a shared IR, a shared fact
database, one fixpoint kernel parameterized over a product lattice, a
lazily-escalating precision ladder, and a CEGAR feedback loop that
compiles expensive counterexamples back into cheap pattern rules.

This is not a `.nir` language proposal — it doesn't touch grammar,
`requires`, `serve`, or anything `typeck.rs` enforces today. It's an
architecture proposal for a separate analysis engine, positioned as a
candidate design for the `nirdosha check` CHECKED tier and, longer
term, as a possible internal re-architecture of `refine.rs`/`smt.rs`/
`contract_check.rs`'s own currently-separate Tier-1 analyses (see
"Rejected alternatives" and "Open questions" below for why neither is
committed to here).

## Design

*(Full technical design preserved verbatim from the original proposal
— summarized here by section, not shortened in substance.)*

1. **The core insight**: eleven tools are the same
   `Parse → Model → Lattice → Fixpoint → Report` machine with
   different knobs — a subtype lattice (mypy/Pyright), tree matching
   (ESLint/Semgrep), taint/powerset lattices over Datalog
   (SonarQube/CodeQL), intervals/octagons with widening (Coverity),
   logic handed to an SMT solver (Dafny/Prusti), bounded unrolling
   (Kani), and a supply-chain constraint graph (Snyk). OMNISCOPE is one
   engine, one lattice library, one database, and one feedback loop
   that escalates precision lazily and feeds every answer back down
   the ladder.
2. **Seven design principles**: one universal IR (SSA-PERM, below); one
   Datalog code database every stage can query; one fixpoint kernel
   parameterized by (lattice, transfer function, widening strategy);
   lazy precision escalation (never pay for SMT when pattern-matching
   answers, never pattern-match when the question needs a proof);
   CEGAR feedback in both directions; proof-carrying autofix patches
   (every fix bounded-model-checked before it's offered); and a
   compute budget ("gas") per query with graceful degradation
   (soundiness, not soundness, by default).
3. **SSA-PERM, the universal IR**: an SSA control-flow graph (MIR/GOTO
   convergence) with bit-precise values (fixed-width ints, IEEE-754
   floats, no unbounded mathematical integers), fractional permission
   annotations per heap access (Prusti-style, `&mut` = π=1 plus a
   magic-wand obligation), empty-by-default requires/ensures/
   invariant/decreases slots (Dafny-style), and heap modeled as
   select/store arrays so memory reasoning reduces to SMT array
   theory. Every front-end (tree-sitter for breadth, compiler-grade
   parsers for depth) emits SSA-PERM; one IR serves every downstream
   analysis across every source language.
4. **Six subsystems**:
   - **S1 — Extraction & code database** (CodeQL/Semgrep/Snyk): a
     semi-naïve Datalog store of `expr`/`cfg_edge`/`calls`/`reaches`/
     `package`/`affects` relations; a CDCL-style version solver
     resolves the dependency graph and asserts its output back as
     facts; Semgrep-style structural patterns compile to Datalog
     conjunctive queries with metavariable columns, so pattern hits
     become queryable facts. Payoff: dependency-reachability +
     taint-reachability + call-reachability collapse into one query
     (`vuln_flow(pkg, vuln, sink) :- dep(...), affects(...), calls*(...),
     reaches(...)`).
   - **S2 — The unified fixpoint kernel** (mypy/SonarQube/Coverity/
     CodeQL): one worklist algorithm over a product lattice
     `L = L_type × L_taint × L_val × L_perm`, joined component-wise,
     with **mixed widening** — widen only the components whose
     ascending chains diverge (intervals), keep the finite ones exact
     (taint, permissions). Runs mypy-, SonarQube-, and Coverity-shaped
     analyses simultaneously on one worklist, sharing CFG traversal,
     summaries, and caching.
   - **S3 — The precision ladder** (the design's core): L0 structural
     patterns (Semgrep/ESLint, O(n)) → L1 type consistency
     (mypy/Pyright, O(n log n)) → L2 monotone dataflow/taint
     (SonarQube/CodeQL, O(n·k)) → L3 abstract interpretation (Coverity,
     poly(n)) → L4 deductive verification-condition generation +
     SMT (Dafny/Prusti, exp(poly)) → L5 bounded model checking
     (Kani, exp(exp)). Each query starts at L0 and climbs only as far
     as needed; a pattern match can prove *presence* but never
     *absence* — an absence claim is reported with its precision rank
     ("definitely safe up to L3" / "no witness found within k=64"),
     never as an unqualified "safe."
   - **S4 — CEGAR feedback, both directions**: upward, a spurious SMT
     counterexample refines the abstract domain (classic CEGAR,
     sourced from SMT models instead of hand-written predicates).
     Downward — the actually novel move — a *real* counterexample is
     anti-unified (Plotkin's least general generalization) into a new
     Semgrep-style pattern with metavariables, tested against the
     corpus, and promoted into the L0 rule base once its precision
     clears a threshold: the system learns lint rules from proofs, so
     later scans need fewer expensive SMT calls for the same bug
     class.
   - **S5 — Proof-carrying patches** (ESLint × Kani): a candidate fix
     is only offered if bounded model checking finds no counterexample
     of length ≤ k against `Σ ∧ ¬regressions` *and* it type-checks at
     L1 *and* a frame-rule check (Prusti's magic-wand machinery)
     confirms it never touches memory outside its own footprint.
     Ranked by edit distance and footprint preservation.
   - **S6 — Budget & incrementality** (Coverity/Snyk): per-query gas
     (rung limits, iteration caps, SMT timeout, BMC k) with graceful
     degradation; change-based re-analysis at function-level summary
     granularity; interprocedural summaries memoized Sharir–Pnueli-
     style and shared across every rung, so L3's interval summary
     accelerates L4's VC generation instead of redoing the work.
5. **The master algorithm** (§5 of the original): per query, climb L0
   through L5 while gas remains, stopping the moment an answer is
   conclusive for the question asked; at L4, a spurious SMT model
   refines the abstract domain (CEGAR up) while a real one both
   reports the counterexample and generalizes into a new L0 rule
   (CEGAR down); separately, every finding's candidate fixes are
   ranked and the first one that both type-checks and survives bounded
   model checking is emitted with its proof certificate.
6. **The math, condensed**: the whole analysis is `lfp` on a product
   lattice (Knaster–Tarski guarantees existence; per-component Galois
   connections make the product sound); the ladder is a partial order
   of proof systems where conclusiveness at a cheaper rung for an
   *absence* claim implies conclusiveness at every more expensive rung
   too, letting the engine skip rungs once satisfied; rule-learning is
   least-general-generalization (Plotkin 1970, decidable for Semgrep's
   first-order pattern fragment, detection-only so it can't introduce
   unsoundness); patch acceptance is a checkable *bounded relative
   correctness* property, not "looks right."
7. **Per-tool attribution table** mapping each of the eleven tools to
   exactly which subsystem/rung it contributes (mypy/Pyright → L1;
   ESLint → L0 UX + S5; SonarQube/Semgrep/CodeQL → L2/L0/S1; Snyk →
   S6 + dependency facts; Coverity → L3 + budget model; Dafny/Prusti →
   L4 front end + IR permission layer; Kani → L5 + patch verifier).
8. **Feasibility & build order**: the components with existing OSS
   implementations (Soufflé-class Datalog, Z3-class SMT, CBMC/Kani-
   class BMC, tree-sitter + Semgrep pattern matching, abstract-
   interpretation libraries like Crab/INTERPROC) are called out as
   *not* the novel part; the genuinely new engineering is (a) the
   product-lattice kernel, (b) cross-rung CEGAR with rule
   generalization, (c) a permission-annotated IR generalized across
   arbitrary source languages. Proposed MVP order: S1 (Datalog +
   tree-sitter) → L0+L2 (patterns + taint) → L1 for one language → L3
   with intervals → L5 via an existing BMC backend → L4 and the
   learning loop last. Named hard parts: L4 quantifier-instantiation
   flakiness (mitigate: AI-seeded triggers), pattern-mining precision
   (mitigate: a promotion threshold plus corpus testing), and
   multi-language permission inference for legacy code (mitigate:
   permissive defaults, escalate on demand).
9. **The claimed payoff, as five properties no single listed tool
   offers alone**: gets faster with use (learned L0 rules reduce
   future SMT calls); answers "is this CVE exploitable *here*" with
   code-level reachability proof rather than version matching alone;
   offers only regression-proof patches; runs type/taint/interval/
   permission analysis as one shared fixpoint instead of four separate
   traversals; and degrades gracefully under Rice's theorem, with
   every answer carrying an explicit precision rank rather than an
   unqualified "safe."

## Effect on the permission model

None. This proposes a new analysis engine, not a `.nir` grammar or
runtime-permission change — nothing about `requires(role/claim: ...)`,
`acquire`, `screen` view/edit gates, or `serve.rs`'s enforcement is
touched by this design as written. If OMNISCOPE were later adopted as
`nirdosha check`'s real backend, *that* integration RFC would need its
own answer to this question (does a CHECKED-tier finding about
non-`.nir` code ever gate anything the way a `.nir` compile error
does?) — not answered here.

## Compatibility

No existing `.nir` program's behavior changes; this document proposes
no change to any shipped surface. Two real integration paths exist
and neither is committed to by this RFC:

- **As `nirdosha check`'s backend** (`docs/ROADMAP.md`'s post-seed
  CHECKED tier) — a new, additive capability for non-`.nir` code,
  parallel to today's PROVED tier, not a replacement for anything.
- **As an internal re-architecture of `refine.rs`/`smt.rs`/
  `contract_check.rs`** — these three already independently implement
  pieces of what S2/S3 describe (`refine.rs`'s interval analysis is
  literally an L3-shaped abstract interpretation; `smt.rs`'s
  condition-narrowing is L2-shaped monotone dataflow; `contract_check.
  rs`'s Hoare-predicate checking is L4). Unifying them behind one
  product-lattice kernel is a real, large, separate engineering
  decision this document does not make — see "Open questions."

## Rejected alternatives

None are named in the source proposal — it presents OMNISCOPE as a
synthesis, not as a choice among competing designs. Recorded here as a
real gap, not filled in with alternatives invented after the fact: a
revision of this RFC should say what a narrower design (e.g. "just
wire CodeQL and Kani together via a shared fact export" instead of a
from-scratch shared IR and kernel) would cost and why it was judged
insufficient, since that comparison is exactly what would make this
RFC's scope defensible against a "why not just compose the existing
tools' CLIs" objection.

## Open questions

- **Is this additive to `nirdosha check`, or a replacement for
  `refine.rs`/`smt.rs`/`contract_check.rs`?** The two integration
  paths above have very different blast radii and neither is decided
  here.
- **SSA-PERM for arbitrary languages** is the load-bearing claim the
  whole design stands on (§8 calls out permission inference for
  "legacy code" as a named hard part, mitigated only by "permissive
  defaults, escalate on demand"). No evidence is offered that
  permission *inference* (as opposed to Prusti's own setting, where a
  human writes `&`/`&mut` and the borrow checker already computed
  aliasing) is tractable across languages without that discipline
  (Python, JS) at anywhere near the precision L3/L4 need to be useful.
- **CEGAR-down (learning L0 rules from SMT counterexamples)** is
  claimed precision-monotone "under corpus testing" — the corpus, its
  size, and what precision threshold is required before a
  machine-derived pattern is trusted on someone else's codebase are
  all unspecified.
- **Who reviews a promoted rule?** S4's downward loop auto-promotes a
  generalized pattern into the live rule base once it clears a
  precision threshold against the corpus — no human-review gate is
  named. A wrong or overfit auto-promoted rule silently degrades every
  future L0 pass across the whole codebase until caught.
- **Budget-driven soundiness** is stated as the default ("soundiness,
  not soundness, by default") — for `.nir`'s own PROVED tier, silent
  unsoundness by default would be a regression from the current
  `PROVED`/`DISPROVED`/`UNKNOWN` three-way discipline. Any integration
  with Nirdosha's own tiers needs its own explicit answer for how a
  gas-exhausted query is reported (this RFC's own L5 line — "no
  witness found within k=64" — is the right shape; it needs to be the
  *mandatory* framing everywhere gas runs out, not an option the
  engine can silently skip).
