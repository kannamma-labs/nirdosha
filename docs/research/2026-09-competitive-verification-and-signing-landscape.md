# Comparative landscape: verification, borrow-checking, and signing (2026-09)

> Phase 1 of the "catch up to parity, then differentiate" plan (see the
> conversation that produced this doc — no RFC number yet, this feeds
> RFC 0016 Phase 3/5b and the killer_demo `db`-serializability gap).
> Scope and honesty note up front: this is a literature/web-research
> pass, not a hands-on read of any of these tools' source trees. Every
> claim below is attributed to a source; none of it is a number we
> measured ourselves, and none of it should be repeated as a Nirdosha
> result until we've built and measured our own version. Standing on
> published algorithms is normal engineering (Dafny itself stands on
> ESC/Java and Boogie); claiming someone else's number as ours would
> not be.

## 1. Inter-procedural verification-condition generation — Dafny/Boogie

RFC 0016 Phase 3 names this the largest unbuilt item: today's coverage
gate proves single-function contracts; a certified primitive's
precondition at a *call site* needs caller facts to flow across
function boundaries, which the engine doesn't do yet.

**How Dafny/Boogie actually do it**, per [Dafny's own program-verifier
literature](https://www.emergentmind.com/topics/verification-aware-programming-language-dafny)
and the original [Dafny
paper](https://www.microsoft.com/en-us/research/wp-content/uploads/2008/12/dafny_krml203.pdf):
Dafny compiles to Boogie, an intermediate verification language whose
job is exactly VC generation over a weakest-precondition calculus.
Critically, **each method is checked against its contract in
isolation — callee bodies are never inlined.** A call site doesn't
re-derive the callee's proof; it only has to discharge the callee's
stated precondition using facts available at the call site, then gets
to assume the callee's postcondition. This is made sound by explicit
**frame conditions** (`reads`/`modifies` clauses) that bound what a
function is allowed to depend on or mutate — without them, "assume the
postcondition, don't look inside" would be unsound the moment two
functions touch overlapping mutable state.

**What's portable to Nirdosha:** the shape, not the implementation.
`contract_check.rs` already has the single-function proof machinery
(Z3 directly, no Boogie IR layer needed for our scope); what's missing
is (a) a call-site obligation generator that, at every call to a
certified primitive, asserts the precondition as a fresh proof
obligation discharged from caller-local facts, and (b) explicit frame
declarations on primitives so the obligation generator knows what a
call can and can't have invalidated. RFC 0016 Phase 3 already scoped
this down to caller-local facts only (no cross-function summaries, no
loop invariants) — which is a *smaller* problem than what Dafny
solves, not a port of Dafny's own summary machinery. That's the
honest framing: we're not porting Dafny's engine, we're porting the
frame-condition *concept* into a much narrower obligation generator.

**What's not portable:** Dafny/Boogie's full modular-summary system
(a method's own proof can stand in as a fact for any caller,
recursively) is real engineering weight — ghost state, termination
metrics, a full IVL. Out of scope for what RFC 0016 Phase 3 actually
needs.

## 2. Rust-specific verification tools — Prusti, Creusot, Kani

Directly relevant because Nirdosha's codegen path is Rust-adjacent,
and because these are the closest published prior art for "verify a
Rust-shaped language with contracts."
[Surveying the Rust Verification Landscape](https://arxiv.org/pdf/2410.01981)
(Le Blanc, HATRA'24) is the single best source here — a direct,
recent survey.

- **Prusti** — deductive verification built on the Viper
  infrastructure; specifies mutable-borrow effects with *pledges*.
- **Creusot** — translates MIR into Why3's WhyML, uses *prophecy
  variables* to reason about mutable borrows, and per the survey
  supports a wider range of borrowing patterns than Prusti (e.g.
  reborrowing in a loop, which Prusti doesn't).
- **Kani** — bounded model checking, not deductive verification: it
  explores program states up to a bound rather than proving a
  universally-quantified property the way Z3-backed deductive
  verification does. Different soundness story (bounded vs.
  unbounded), different cost profile.
- All three, per the survey, currently define their own contract
  language, with a real, disclosed risk of divergence between them —
  the Rust project's own [lang-team
  issue #181](https://github.com/rust-lang/lang-team/issues/181)
  tracks exactly this as unsettled.

**What's portable:** confirmation that Nirdosha's own choice (a
purpose-built contract syntax + one proof backend, not three
competing verifiers with drifting contract languages) avoids a real,
acknowledged pain point in this space. Also: Creusot's prophecy-variable
technique for mutable borrows is the concrete prior art to study
*before* extending Nirdosha's ownership model past affine `box`
tracking, if/when `&mut` liveness work starts — it's a published,
working answer to "how do you specify a function's effect on a
borrowed value it hands back," which is exactly what our
`primitive_exclusivity` (RFC 0016) and any future `&mut` work will
need.

**What's not portable:** none of these three tools handle external,
mutable stores (a real SQL database) any differently than Nirdosha
does today — they verify in-memory Rust state. This confirms §6 below
is a genuinely separate problem, not something "adopt Creusot's
technique" solves.

## 3. Borrow-checker liveness — Polonius

The most directly relevant prior art for closing "no `&mut`-style
liveness/exclusivity enforcement beyond affine `box`" (README's own
disclosed gap). As of 2026-08-04, [Polonius Alpha is enabled on Rust
nightly](https://blog.rust-lang.org/2026/08/04/enabling-polonius-alpha-on-nightly/),
heading toward stabilization.

**The actual algorithm**, per [Niko Matsakis's original
formulation](https://smallcultfollowing.com/babysteps/blog/2018/04/27/an-alias-based-formulation-of-the-borrow-checker/)
(still the base algorithm the crate implements, per the [compiler-team
working-group page](https://rust-lang.github.io/compiler-team/working-groups/polonius/)):
it's a **Datalog fixpoint computation** over four relations —

- `borrow_region(R, L, P)` — region `R` corresponds to loan `L` at point `P`, created at each borrow expression.
- `subset(R1, R2, P)` — subtyping-derived region containment, propagated forward through the CFG.
- `requires(R, L, P)` — region `R` requires loan `L`'s terms to hold at `P`, derived by composing `subset` with `borrow_region`.
- `loan_live_at(L, P)` — a loan is live if any live region requires it.

An error is `invalidates(P, L) ∧ loan_live_at(L, P)`. The structural
difference from NLL: NLL's regions are program-point *spans* (lexical
reasoning); Polonius's regions are sets of *loans* that can be
**killed on reassignment**, which is what lets it accept patterns NLL
rejects (the canonical case: the "get-or-insert into a HashMap" idiom).

**What's portable:** the relation shapes themselves are a genuinely
reusable design — a Datalog-style fixpoint over `subset`/`requires`/
`loan_live_at` is a much smaller, more tractable thing to scope down
and reimplement for exactly the subset Nirdosha's ownership model
needs (real `&`/`&mut` liveness, not a full NLL/Polonius reimplementation)
than starting from Rust's own borrow-checker source, which carries two
decades of unrelated Rust-specific baggage.

**What's not portable, and worth saying plainly:** Polonius alpha
itself, per the Rust blog post, shows a real, disclosed 2-3x
compile-time regression on borrow-heavy crates versus NLL — a genuine
cost/precision tradeoff, not a free upgrade. Any Nirdosha work in this
space should budget for and disclose the same kind of tradeoff rather
than assume liveness tracking is free.

## 4. Financial-domain formal verification — Imandra

The closest published example of "formal verification aimed
specifically at fintech," per [Imandra's own
docs](https://www.imandra.ai/) and the
[Aesthetic Integration launch coverage](https://www.finextra.com/pressarticle/66137/aesthetic-integration-launches-formal-verification-platform-for-smart-contracts/retail).
Its Reasoning Engine separates four concerns: simulation (execute a
model), verification (prove/disprove a safety property), counterexample
generation, and (per docs) live multi-user collaboration on models —
targeting exchange matching logic and FIX connectivity specifically.

**What's portable:** the four-way split (simulate / verify /
counterexample / collaborate) is a useful lens on where Nirdosha's own
trust pipeline already has gaps — we have verify + counterexample
(Z3's `DISPROVED` path already gives a concrete counterexample per the
README) but nothing resembling Imandra's simulation mode (run the
model against a scenario without a full compile/deploy cycle) or its
collaborative-model story. Not urgent, but a real gap Imandra's
architecture makes visible.

**What's not portable / key difference:** Imandra verifies models
written in its own ML-family language (IML) by domain experts working
directly with the prover — it is not solving "an LLM writes ordinary
application code and a compiler proves properties about it
automatically," which is Nirdosha's actual problem. Imandra is
stronger proof tooling aimed at a narrower, expert-operated audience;
not a competitor on the "LLM-writable" axis at all.

## 5. Signing/trust infrastructure — Sigstore

Directly maps onto RFC 0016 Phase 5b, which is blocked on "who runs
the registry." Per [Sigstore's own
docs](https://docs.sigstore.dev/cosign/signing/overview/) and the
[Rekor overview](https://docs.sigstore.dev/logging/overview/):

- **Fulcio** — a CA issuing *short-lived* certificates binding an
  ephemeral key to a verified OIDC identity (a GitHub/Google/etc.
  login), not a long-lived personal key a user has to protect for years.
- **Cosign** — signs the artifact digest with that ephemeral key, then
  discards the private key in memory when the process exits.
- **Rekor** — an append-only, publicly verifiable Merkle-tree
  transparency log recording every signing event. **The load-bearing
  design insight: trust shifts from "has this long-lived key ever been
  compromised" to "was this signing event recorded in the log at a
  specific time, under a verified identity."** That's a fundamentally
  different (and much more auditable) trust model than
  `nirdosha keygen`'s current raw-PKCS8-keypair-on-disk model.

**What this solves for RFC 0016 5b, directly:** the "registry
governance" problem 0016 calls blocking isn't really "who hosts a
database of pack IDs" — it's "who is the trust root that says a given
public key really belongs to the domain expert who sealed this pack."
Sigstore already answers exactly that question, generically, at
Internet scale, and our own `GOVERNANCE.md` *already* commits to
"cosign-signed and SBOM'd, all via GitHub OIDC" for our own releases —
so adopting the same architecture for pack signing isn't a new
dependency to justify, it's reusing a trust model this project has
already accepted for itself. A pack author's identity becomes "this
GitHub/organizational identity signed this pack_id digest, recorded in
a public transparency log at time T" instead of "here is a bare public
key, trust it because someone pinned it" — which is a real, concrete
unblock for 5b that doesn't require Nirdosha to build or operate its
own registry/CA at all.

**What's not free:** Sigstore's default trust roots are a public,
Internet-facing CA/log (the public good instance) — a regulated
deployment (the FAPI/banking case 0016 cares about most) will want a
private Fulcio/Rekor instance or an alternative trust root, which is
itself an operational commitment, not a zero-cost adoption.

## 6. The real gap: `db`-mediated serializability (killer_demo)

This is the one none of the above tools solve, and it's the most
important finding of this research pass: **static/deductive
verification of general transaction serializability against an
external, mutable store is not where this field's state of the art
actually lives.** None of Dafny, Prusti, Creusot, or Kani reason about
an external SQL store any differently than Nirdosha does today — they
verify in-memory state. The field's actual practical answer is
**dynamic, observational checking of the transaction log after the
fact**, not static proof before it:

- [**Elle**](https://github.com/jepsen-io/elle) (VLDB'21, Jepsen) —
  a black-box isolation checker: infers a Direct Serialization Graph
  from *observed* transaction operations (reads/writes, process order,
  real-time order) and finds isolation anomalies as **cycles in that
  graph**. It doesn't prove anything before the fact; it certifies (or
  refutes) serializability of a history that already happened, fast
  enough for real production-scale histories.
- A 2026 follow-on paper, ["Making Transaction Isolation Checking
  Practical"](https://arxiv.org/pdf/2604.20587), improves Elle-class
  checking further via structural pruning of the search space —
  confirming this is still the active frontier, and that the field's
  own consensus is **dynamic checking over collected histories, not
  static analysis**, is the practical path.

**This is the finding that reframes the killer_demo gap, and directly
validates the APM-kernel differentiation idea from the earlier
strategy discussion.** Nirdosha already has almost everything Elle
needs, for free, on the runtime side: `transact`'s compiler-enforced
precheck/network/verify/commit/compensate protocol and its real
crash-durable log (per the README, already built). What's missing is
turning that log into a *dependency graph* and running an Elle-style
cycle detector over it — inside the APM kernel, which already owns
the "observe everything the compiled binary does" seam. Concretely:

1. Every `transact` block's compiled log entry already records what it
   read/wrote (or can, with a small, additive extension) — that's
   Elle's raw input.
2. The APM kernel (already the thing watching runtime behavior for
   `nfr(...)`) builds the dependency graph and checks for cycles,
   either continuously or on demand.
3. A detected cycle is exactly the killer_demo failure mode, caught
   automatically at runtime instead of silently corrupting the ledger
   — and it's evidence that can feed back into the certificate
   (`nfr_commitments`'s sibling: an `isolation_violations` field,
   `evidence_tier: "monitored"`, same discipline) and into
   `hint_cache` the same way a self-repair lesson does today.

This is not a static guarantee — it will never read `"proved"` in the
certificate, and that's the honest, correct label for it (Elle's own
technique is explicitly a posteriori, not a priori). But it closes a
real gap none of the static-verification competitors close either,
using an asset (the APM kernel + `transact`'s log) none of them have.
It is the clearest concrete instance of the "domain translation"
framing from the strategy discussion: `transact`'s durability-log
domain talking to a new isolation-checking domain, surfaced through
the certificate domain that already exists.

## Priority synthesis for Phase 2+

1. **`transact`-log → Elle-style cycle checker in the APM kernel** —
   closes the single most reputation-relevant gap (killer_demo), uses
   an asset competitors don't have, and is honestly scoped (dynamic,
   `"monitored"`, never claims `"proved"`).
2. **Sigstore-pattern signing for RFC 0016 5b** — unblocks work that's
   been sitting blocked on a governance question this already solves;
   reuses a trust model `GOVERNANCE.md` already committed to.
3. **Caller-local call-site obligations (Dafny/Boogie frame-condition
   pattern)** — RFC 0016 Phase 3, now with a concrete, scoped-down
   design pattern to build from instead of starting blank.
4. **Polonius-style `subset`/`requires`/`loan_live_at` relations,
   scoped to Nirdosha's `&`/`&mut`** — real, but the lowest priority
   of the four: it's the least reputationally urgent gap (no public
   demo currently exposes it the way killer_demo exposes #1), and
   Polonius's own disclosed 2-3x compile-time cost is a warning to
   budget for, not rush into.

## Sources

- [Enabling the next iteration of the borrow checker on nightly — Rust Blog, 2026-08-04](https://blog.rust-lang.org/2026/08/04/enabling-polonius-alpha-on-nightly/)
- [Polonius working group — rust-lang compiler-team](https://rust-lang.github.io/compiler-team/working-groups/polonius/)
- [An alias-based formulation of the borrow checker — Niko Matsakis](https://smallcultfollowing.com/babysteps/blog/2018/04/27/an-alias-based-formulation-of-the-borrow-checker/)
- [Dafny: Verification-Aware Programming — overview](https://www.emergentmind.com/topics/verification-aware-programming-language-dafny)
- [Dafny: An Automatic Program Verifier for Functional Correctness — original paper](https://www.microsoft.com/en-us/research/wp-content/uploads/2008/12/dafny_krml203.pdf)
- [Surveying the Rust Verification Landscape — Le Blanc, HATRA'24 (arXiv:2410.01981)](https://arxiv.org/pdf/2410.01981)
- [Contracts and Automated Reasoning for Rust — rust-lang/lang-team issue #181](https://github.com/rust-lang/lang-team/issues/181)
- [Imandra — Home of Reasoning as a Service](https://www.imandra.ai/)
- [Aesthetic Integration launches formal verification platform for smart contracts — Finextra](https://www.finextra.com/pressarticle/66137/aesthetic-integration-launches-formal-verification-platform-for-smart-contracts/retail)
- [Sigstore / Cosign signing overview](https://docs.sigstore.dev/cosign/signing/overview/)
- [Rekor transparency log overview](https://docs.sigstore.dev/logging/overview/)
- [Elle: Inferring Isolation Anomalies from Experimental Observations — VLDB'21 (jepsen-io/elle)](https://github.com/jepsen-io/elle)
- [Making Transaction Isolation Checking Practical — arXiv:2604.20587 (2026)](https://arxiv.org/pdf/2604.20587)
