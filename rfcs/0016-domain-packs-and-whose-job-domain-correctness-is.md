# RFC 0016: Sealed domain plugins — pre-baked, non-waivable, cryptographically attested invariants (and compliance profiles)

> **Status: unified design capture (2026-09-11 → 2026-09-12), prompted by
> a real observed failure; zero code shipped for this RFC yet.** The
> gap below is not hypothetical — it was demonstrated on 2026-09-11:
> `hi`'s generate, run against a confirmed 7-unit banking graph
> (`~/temp2`, first attempt, compiled clean, published, ran the full
> banking day), shipped a program with **zero `validate` contracts**.
> `nirdosha verify` on that output reports `verdict: PROVED`,
> `contracts proved: 0`. PROVED 0/0 is not safety; it is silence.
> The machinery this RFC composes (`contract_check.rs`'s Z3 proofs,
> `hi_graph.rs`'s confirm/lock model, `hi_llm.rs`'s repair loop) all
> exists today and is cited by real name below.
>
> **Reviewed 2026-09-12: accepted as a bridge, not the final
> guarantee.** Ship solution 4 immediately (it closes the demonstrated
> seam with zero crypto). Ship **5a** — non-waivable plugin nodes with
> zero crypto — as the shippable delivery mechanism; the full sealing
> (**5b**) is spec-ready but blocked on registry reality.
> For banking, do **not** defer solution 6 — inject certified
> primitives as a mandatory prelude even before multi-file linking
> lands. Everything the review flagged — fail-closed proof semantics,
> non-vacuity, break-glass, registry trust anchors, standard
> canonicalization — is now designed in below, not left open.
> Third pass (2026-09-12): red-teamed as an adversary; findings and
> mitigations are consolidated in "Threat model" below. Two sentences
> govern the implementation branch: **the pack ID is the law, and the
> signature must cover it. The model wires the ledger; it does not
> write it.**
>
> Fourth pass (2026-09-12): spec bugs fixed — solver fuel replaces
> wall-clock timeouts, engine-limit failures are distinguished from
> code violations in the repair loop, the governing set moves to the
> attestation sidecar (zero DSL change), standalone-verify's limits
> are stated honestly, and solutions 5a/5b are split so the
> demonstrable fix is not coupled to an organization that does not
> yet exist.

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

A third observation set the final shape of this RFC. If domain law is
non-waivable, **the law itself must be provably authentic** — otherwise
the pack becomes the new single point of failure, and "no matter what
happens" is only as strong as an unauthenticated JSON file. Hence the
pack is not a file; it is a **sealed, signed, content-addressed
document** whose ID is embedded in every artifact it governs.

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
as `validate` contracts the compiler can prove — then **sealing**
that translation under their signing key.
2. **The plugin holds them as pre-baked, non-waivable nodes.** A
*domain plugin* is a pre-authored, per-domain sealed document
injected into the graph *before* the user's prompt is read —
confirmed, locked, non-waivable, and carrying a verifiable
identity. hi already treats confirmed units as mandatory generate
input (`hi_graph::confirmed_units` feeds `generate_program`); a
plugin simply arrives pre-confirmed.
3. **The proof enforces forever.** The "no matter what happens" is
Z3 plus signature verification, not the expert:
`contract_check::run_program_validates` / `verify_code`
(`mcp_tools.rs`) prove the baked contracts against whatever the
model produces — or the program doesn't publish. The plugin loader
verifies the seal before any graph write — or the plugin doesn't
load. A reviewer's opinion can be argued with; a proof cannot; a
signature cannot be faked.

### Solution ladder

Seven solutions considered, in ascending strength. 1–2 are the status
quo; 3 is rejected; 4 is the minimal seam fix and a strict subsystem of
5; **5 (sealed domain plugins) is this RFC's delivery mechanism — a
bridge, not the guarantee — split into 5a (non-waivable plugin
nodes, zero crypto, shippable now) and 5b (the sealing
infrastructure, spec-ready, blocked on registry reality)**; 6 is
the guarantee for code-level correctness and is *not* deferred for
regulated domains; 7 extends the same machinery to compliance.
**4+5a ship now** — 5b must not gate anything shippable; 5+6 is
the best offer, and the RFC says so in its own text.

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
later as a *plugin-drafting* aid for the human domain expert. Same
category as the "domain expert who reviews every generation" model:
vigilance, not law.

**4. The coverage gate (the minimal seam fix).** One new stage in the
generate loop: after `typecheck_and_build_check` passes, require that
every confirmed unit whose attributes demand a `validate` contract
actually has one in the draft, and that it proves. Concretely:

- new `hi_llm::contract_coverage_check(source, units)` — parses the
draft (`ast::Program`), walks `fn.units`' demanded attributes
against the `ast::ValidateDecl`s actually present
(`fn.validates`), and runs `contract_check::run_program_validates`
on those present. Two modes, by `domain_class`:
- **Injected (regulated, default):** the plugin's `ValidateDecl`s
are inserted into the program AST at generate time, exactly like
the solution-6 prelude — the model's code is checked *against*
the sealed law; it never authors it. This closes the
weaker-contract hole: with demanded attributes alone the model
can satisfy a name with `validate balance_nonnegative { true }`,
which is non-vacuous (preconditions satisfiable) and trivially
provable. Non-vacuity checks reachability; they say nothing
about strength. Injection is the only fail-closed answer for
regulated domains.
- **Demanded (non-regulated long tail):** attribute form only
here. Plugin-demanded names must be NFKC-normalized ASCII and
matched canonically — no homoglyph names (`balance_nonnegativе`
with a Cyrillic `е`) on either side of the match.
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
is why it is a subsystem of 5, not the answer.

**5. Sealed domain plugins (this RFC's proposal), in two slices.**
A plugin is authored once per domain and injected into the graph at
prompt time. The review's sequencing correction is adopted — 5 splits
into:

- **5a — non-waivable plugin nodes, zero crypto (shippable now).**
The graph mechanics the whole design rests on — pre-confirmed
invariant units, `RELATES_TO` edges, the non-waivable flag,
refuse-on-waive/delete, prelude injection, the coverage gate —
ship without any of the sealing machinery. Integrity of the pack
bytes rides on where they live: a pack committed to the repo is
content-addressed by git already; the loader pins the expected
pack hash in the graph at install time, so a modified working-tree
pack fails the pin on load. Name the pin for what it is: **trust
on first use** — its authority is first-install plus git history,
not provenance. Anyone auditing a 5a graph later must not read the
pin as evidence of where the pack came from, only that it has not
changed since someone installed it. This closes the demonstrated 0/0 seam
this week. What 5a honestly does *not* provide: proof of pack
authorship (anyone with repo write access can author a pack) and
standalone verification against tampering outside git.
- **5b — the sealing infrastructure (spec-ready, blocked on
registry reality).** Everything in "The sealed plugin format"
below: Ed25519, JCS, certificates, transparency logs, trust
anchors, revocation, key compromise. For `domain_class:
regulated` the registry anchor is mandatory — and Open Questions
is honest that the registry's owner is a governance problem with
no answer yet. **5b therefore must not gate 5a or 4**: the
motivating domain's correctness fix is coupled to an organization
that does not exist, and this split says so in the text.

The short form of 5 as a whole:

- **Plugin shape:** a canonicalized document — manifest, invariant
nodes (confirmed `CandidateUnit`s carrying `validate`-contract
attributes), `RELATES_TO` edges to the domain's core units, and
optionally locked code units — plus signature, certificate, and
trust metadata.
- **Injection:** a `hi_graph::load_domain_plugin(conn, plugin)` /
`hi_api.rs` hook that runs before `populate_candidates`, writing
each invariant via the existing `add_candidate` + `confirm_node` +
`attach_attribute` + `lock_units_after_sync` path. Plugin nodes are
marked non-waivable.
- **Non-waivable flag:** a new boolean on plugin nodes;
`hi_graph::waive_node` and `hi_graph::delete_node` refuse
("sealed domain invariant: not subject to waive — see plugin
`<pack-id>`"). `:confirm all` in the console can't remove them
either — confirm is already idempotent; refusal lives on the
waive/delete/unwaive side only.
- **Enforcement:** the coverage gate from solution 4 is what makes
the plugin bite — plugin invariants arrive as demanded attributes,
the gate fails any draft that drops them, Z3 fails any draft whose
code violates them, and `certify_code` (`mcp_tools.rs`) attests
the result with the plugin IDs embedded.

What the plugin does *not* need: a domain-expert LLM in the loop
after baking; new proof machinery; language changes.

**6. Certified proven-primitive libraries (the guarantee — not
deferred for regulated domains).** The strongest form, and the honest
answer to "is 5 the best we can offer for banking?": **no.** Sealed
invariants as attributes still let the model *write the ledger* — Z3
can prove `balance >= 0` against the draft, but it cannot prove the
model implemented the *right* ledger; it can invent banking badly and
satisfy a weak contract. The fix: the domain expert authors the ledger
core *itself* — `transfer`, `post_entry`, `authorize` — as certified
code with embedded `ValidateDecl`s, proven once
(`mcp_tools::verify_code` + `certify_code`), and generated programs
must *compose calls to* those primitives rather than re-derive money
math at all. The LLM stops inventing banking and starts wiring it.
This matches how real banking software behaves (you don't re-invent
double-entry; you call a proven ledger), and reuses `hi_graph`'s
existing lock model: primitive units arrive locked, `units_prompt`
presents them as available-and-mandatory, and the wiring contract's
orphan rule already demands real call sites.

**The multi-file objection does not block banking.** The compiler is
single-file today, but a mandatory *prelude* needs no linking story:
`generate_program` prepends the certified primitive block to the
draft before typecheck (the primitives become ordinary in-scope
functions), and the coverage gate requires call sites for each
primitive the plugin marks mandatory. Primitive *units* are locked
and non-waivable exactly like invariant nodes. Two enforcement rules
make the prelude a boundary rather than advice:

- **Call-site precondition obligations.** Certified primitives carry
preconditions (`transfer(from, to, amount)` requires
`amount > 0`, `balance(from) >= amount`). The coverage gate
generates a proof obligation at every call site and proves it
through the same fail-closed, non-vacuous Z3 path. This is the
difference between "calls the ledger" and "uses the ledger
correctly" — a call with unconstrained `amount` is a gate failure
with its own `self_repair_hint` arm.
- **`primitive_exclusivity`.** For every plugin-declared protected
type/field (`Account`, `Ledger`, `balance_cents`), any read,
write, or mutation outside certified primitive units fails the
generate gate. Primitive names live in a reserved namespace that
generated code cannot redeclare or shadow; if the language lacks
field privacy, the locked primitive units own the fields and
generated code touches them only through primitive calls. If the
DSL ever grows macros/comptime codegen, exclusivity applies to
*expanded* code, not surface syntax — stated now, before the
feature exists.

These are the two largest implementation items in this RFC, and
the first is larger than it looks. Call-site precondition
obligations are **inter-procedural verification-condition
generation** — a new capability class, not "plumbing reusing the
existing engine." The engine today proves single-function contracts;
`balance(from) >= amount` at a `transfer` call site requires caller
facts to flow across function boundaries (caller state, loop
invariants, the very shape of the v4 app's money functions calling
each other). Budget it as the largest engineering item in the
document. Exclusivity is a `typeck.rs`-deep change — the same class
rejected for the `Money` type, justified here because it enforces
law, not representation.

**The residual surface, stated honestly.** "The model wires the
ledger; it does not write it" is true only of the ledger.
`transfer`/`post_entry`/`authorize` cover money movement — but
approval chains, SLA math, fee tiers, notification rules are *not*
ledger primitives. The model still writes those, un-governed, unless
the pack injects contracts over them too. Attribute-only coverage
of that surface reintroduces the strength hole the injection mode
exists to close; the honest position is that a banking pack must
inject contracts over every load-bearing behavior it claims to
govern, and the pack's sealed text should say which behaviors it
deliberately leaves to the application — plugin completeness is a
claim the pack makes about itself, visible in its bytes. The full multi-file
linking story (RFC 0014's "v1 scope cut") remains future work — for
library *distribution* — but must not be the reason banking keeps
letting the model write `balance_cents -= debit` by hand.

An attribute-only plugin (solution 5 without 6) shrinks the
demonstrated seam but leaves **plugin completeness** as an assumed
property: if the pack never stated a load-bearing rule, the model can
still violate it. Certified primitives shrink that surface by
construction — the money math is no longer the model's to write — and
are why 5 is a bridge and 6 is the guarantee.

**7. Compliance profiles (same machinery, applied to regulation).**
A sealed plugin can carry not just provable invariants but a
machine-checkable statement of what "compliant" means for the domain
— FAPI for fintech, HIPAA-flavored rules for health — enforced
across the same stages. See "Compliance profiles" below. FAPI is
the motivating case, not a special case, and no FAPI-specific code
ever enters `nirdosha` core.

## The sealed plugin format

The plugin is a directory of files, canonicalized and hashed as a
unit:

```javascript
banking.plugin/
  pack.manifest.json      # metadata, invariants, edges, profiles, policies
  pack.invariants.nir     # optional proven code units (solution-6 form)
  PACK.sig                # Ed25519 signature over the pack_id digest itself
                          # (domain separation included — one hash, one signed message)
  PACK.cert               # certificate / attestation of the signing key
  PACK.log                # optional transparency-log inclusion proof
```

**Content addressing — the pack ID is the law.** The plugin's
identity is `pack_id = "sha256:" + SHA-256(domsep || canonical_bytes)`
over the manifest + invariants, and `PACK.sig` signs **this exact
digest** — one hash, one signed message, domain separation included.
Two different hashes (one for the ID, one for the signature) would
let a signature minted for one context be replayed in another,
defeating the domain separation entirely. This is the root of the
seal and lands before any other code. Canonicalization is **JCS (RFC 8785)**
for all JSON, with a domain-separation prefix (`"nir-plugin/v1\0"`)
hashed before the payload and length-prefixed file framing for the
non-JSON members. Hand-rolled "sorted keys, UTF-8" canonicalization
is exactly where content-addressing schemes die — a standard (JCS)
plus explicit domain separation removes the ambiguity class entirely;
an implementation that canonicalizes differently produces a different
ID and fails verification loudly at load, never silently. JCS covers
JSON only; the `.nir` members get their own stated canonicalization:
UTF-8 without BOM, LF line endings, comments stripped, insignificant
whitespace normalized per the grammar's pretty-printer. A source
file is not JSON; the rules must be written down, not assumed. Any
byte change — an edited invariant, a bumped profile version, a new
edge — yields a new pack ID. The ID is therefore not a lookup key
into a trusted database; it *is* the sealed document. This is what
makes "verifiable over time" possible: validity is evaluated at
signing/load time and recorded, never re-judged retroactively against
a mutated document.

**Signing and certificates.** The manifest carries `sig_alg` and
`pack_format_version` (both inside the signed bytes). Ed25519 is the
only defined `sig_alg` today, but the field exists for a 10-year
artifact: banking artifacts must survive the post-quantum transition,
and agility plus a registry policy bit is how root anchors migrate
(e.g. to hash-based signatures) without re-baking the format.
Loaders reject deprecated `pack_format_version`s outright — the
version lives inside the signature, so downgrade is a verification
failure, not a compatibility mode. The domain expert holds an
Ed25519 key. `PACK.cert` is one of two forms, both supported:

- **Registry-issued** — **MANDATORY for regulated domains** (the
manifest declares its `domain_class`; `load_domain_plugin` enforces
the match). Allowed values are pinned in the format spec:
`regulated` (registry anchor mandatory, transparency log
mandatory, dual-control mandatory) and `long_tail` (self-signed
permitted, all 5b machinery optional per policy bit). The
registry-issued form is a short-lived X.509 or COSE/CWT certificate
from a domain-authority registry binding the public key to an attested
identity ("payments domain authority — banking working group"),
chained to a configured trust anchor, meeting a specified baseline:
allowed algorithms and minimum key sizes, maximum validity period,
a defined revocation mechanism (status list preferred over
CRL/OCSP for offline-friendliness), and certificate-transparency
logging of the registry's issuances — "chain to a configured trust
anchor" alone is not regulator-ready. The anchor set is pinned by
the operator out-of-band, never model-reachable, and changes to it
carry the same dual-control as break-glass; the active anchor set
is part of the attestation context so verifiers know which
registry was trusted. Content addressing and
Ed25519 are necessary, not sufficient — for banking, *who runs the
registry* is the actual guarantee, and a self-signed "banking"
pack is exactly the attack this design exists to prevent. The
format supporting self-signed is pragmatism for the long tail, not
an escape hatch for regulated domains; a self-signed cert on a
manifest declaring `domain_class: regulated` fails step 5 of the
verification pipeline.
- **Self-signed attestation** (non-regulated long tail only): the
public key itself, plus human-readable identity claims, pinned in
the local trust store on first install with an explicit user
action, and attested as self-signed in every certificate it ever
produces — so no downstream consumer mistakes it for
registry-vouched law.

**Manifest fields.** Beyond identity and signatures, the manifest
declares `domain_class` (see certificates above) and
`jurisdictions`: a **set**, not a single value — a cross-border
system legitimately runs under EU + US + UK law simultaneously. The
loader's check is one-directional: a deployment that declares
jurisdiction J must have a plugin covering J present; plugins
covering other jurisdictions are not an error. "Banking" was never
one domain; the manifest says which banking.

**Verification pipeline** (`hi_graph::load_domain_plugin`, before any
graph write — a failed check refuses the load, and nothing is
written):

1. Canonicalize and hash → pack ID.
2. Verify `PACK.sig` against the manifest-declared public key.
3. Verify `PACK.cert`: chain, expiry, and revocation status against
the local CRL / status list.
4. Verify `PACK.log`: a transparency-log inclusion proof against a
trusted, append-only, publicly verifiable log head (CT-style), and
check the recorded timestamp — answering "was this pack valid at
time T" forever. Mandatory for `domain_class: regulated`; a local
file or self-attested timestamp is not trusted time for banking —
the log head, not the local clock, anchors every as-of claim in
the system. Optional policy bit for the long tail.
5. Check local trust store: is this key (or its registry anchor)
trusted for this domain?

Loader hardening, all fail-closed: plugin units are **parsed and
typechecked, never executed** at load — a pack is data until the
graph says otherwise. Loads are bounded: max pack bytes, max
invariants, max edges, max profile entries, per-load time and memory
— a malformed pack fails, it does not consume the machine.
`nirdosha plugin install` takes a `--dry-run` that performs the full
load plus runs
the pack's own contracts against a stub program, so a pack that
would brick generation (fail-closed cuts both ways) is caught before
deploy. Archive-form installs, if ever supported, are checked
against zip-slip/path-traversal and decompression bombs.

**Proof-engine semantics are fail-closed, and non-vacuous.**
`solution 4`'s gate inherits the proof engine's failure modes, so the
engine's contract is part of this RFC:

- **Unknown or timeout is FAIL, not "maybe."** Z3 `unknown`, solver
timeout, and resource exhaustion are indistinguishable from a
violated contract for guarantee purposes — the coverage gate and
`run_program_validates` treat all three as a failed proof, feed
the failure to the repair loop, and count it against
`MAX_SELF_REPAIR_ATTEMPTS`. "We couldn't check it" must never
publish.
- **Non-vacuity is checked.** A contract whose preconditions are
unsatisfiable is trivially true and proves nothing — PROVED with
zero reachable states is the vacuous sibling of PROVED 0/0.
`contract_check` therefore also reports satisfiability of each
contract's precondition (a model of the pre-state exists), and a
vacuous proof is reported as such and treated as a gate failure
with its own `self_repair_hint` arm ("your contract cannot fire —
the preconditions are unsatisfiable; fix the code or the
contract").
- **Violated and engine-limit are different failure classes, and
the repair loop must tell them apart.** `VIOLATED` is the model's
fault: the contract is provable and the code breaks it — the hint
says fix the code, and counts against `MAX_SELF_REPAIR_ATTEMPTS`.
`ENGINE_LIMIT` (`unknown`/fuel exhaustion on *nonlinear* money
math — fee percentages, tiered limits) is not a code bug: no
edit the model makes can "fix" it, and four repair attempts
against it produce a correct-but-ungeneratable program with no
lever. The gate must instead escalate: the hint arm says
"simplify the arithmetic into provable form (linearize the fee,
split the tier)", `ENGINE_LIMIT` does not consume the violation
budget, and after one failed simplification the run escalates to
the operator with the proof obligation attached. Blocking is
right; repair pedagogy must match the failure class — that was
the whole lesson of the 2026-09-11 self-repair work.
- **Resource limits are deterministic — fuel, not wall-clock.**
Limits are fixed toolchain constants expressed as *solver fuel*
(deterministic conflict/step/decision caps), never wall-clock
timeouts: same source, same fuel, same verdict on any machine. A
wall-clock cap would make "proved" machine-dependent — PROVE on a
fast box, TIMEOUT on a slow one — and the attestation
non-reproducible. A fuel-exhausted proof reports `ENGINE_LIMIT`,
deterministically, and is handled by the engine-limit path below.
Memory caps are likewise expressed against a deterministic budget
where the solver permits; where they cannot be, the cap is part of
the attestation context so verifiers reproduce the exact
conditions.
- **The proof model is the runtime model.** Z3 proves against the
language's model of `i64`/`dec128` — so the language spec must pin
overflow behavior (trap, never wrap, for money arithmetic) and the
`dec128` rounding mode, and the proof model must be *identical* to
the execution semantics. If codegen wraps where the proof assumed
mathematical integers, every arithmetic guarantee in this RFC is
void at exactly the boundary cases that matter. Solver version and
language-spec version are recorded in the attestation alongside the
resource limits.

**The ID in the artifact — no DSL change.** The DSL has no `#[…]`
attribute syntax (the parser has no such token; the language uses
`requires(…)` suffixes), and adding one contradicts this RFC's own
compatibility claim. So the governing set is not emitted into source
at all: `certify_code` already emits an attestation sidecar, and the
governing plugin IDs are recorded there, alongside the artifact's
content hash — the toolchain writes it, the model cannot reach it,
which deletes the forgery surface a source-level attribute would
need a policing paragraph for. `nirdosha verify` on any artifact
reports `governing plugins: banking@sha256:9f2c… — signature OK, cert
OK, not revoked`. Anyone holding a published program can see exactly
which sealed law it obeyed, and can fetch that exact document by
content hash and re-verify every claim.

**Break-glass (specified, not open).** A wrong plugin made
non-waivable is unfixable in the field, and real emergencies exist
(a regulatory change mid-flight, a recalled primitive). So:

- Escape is `nirdosha build --break-glass=<pack-id> --reason=<text>`
— **operator-level only** (never the model, never the console's
normal flow), requiring an authenticated operator identity.
- **Loud:** the build refuses unless `--reason` is non-empty and
emits the override at the top of every diagnostic and log surface.
- **Logged:** the override is appended to a hash-chained operator
log (each entry commits to the previous), with operator identity,
timestamp, pack ID, and reason. A hash chain alone is rewritable
wholesale by an attacker with machine control, so the chain head
is anchored off-box — pushed to the transparency log under 5b, or
exported and committed alongside the repo under 5a. Detectable
tampering requires the anchor to leave the machine.
- **Dependency stated:** break-glass presumes an *operator-identity
system the toolchain does not have today* (`VerifiedIdentity` is
for served applications, not for `nirdosha build`). Dual-control
and "operator-authenticated" are requirements on hi itself —
either built as a real component or performed out-of-band (two
humans, external log) and recorded as such. This is a tracked
dependency of 5a/5b, not a manifest bit that conjures it.
- **Never attestable:** `certify_code` refuses outright to issue
any attestation — compliance or otherwise — for a program built
under break-glass, on every re-run, forever. `nirdosha verify` on
a break-glass artifact reports the override prominently at the top
of its output. The artifact runs; it can never claim. This is the
load-bearing property: break-glass preserves *operability* of
production systems while preserving the *meaning* of the seal —
and note it protects the seal, not the bank, since running
unattestable code is itself a harm in regulated domains. Hence
dual-control is **mandatory** for `domain_class: regulated`
(`break_glass_requires_dual: true` is implied by the domain
class; the manifest bit can only tighten it), and the operator
log is expected to be monitored for anomaly alerting. For the
avoidance of doubt, break-glass on a plugin disables that
plugin's enforcement entirely — including its prelude and
`primitive_exclusivity` — and `verify` reports the missing
governing set accordingly; a half-disabled plugin would be the
worst of both worlds.

**Plugin lifecycle** (`nirdosha plugin install / verify / revoke`):

- Packs are never edited in place. New version = new bytes = new ID =
new cert. The graph may hold multiple versions; the loader refuses
to install two plugins whose invariants conflict unless the
operator explicitly displaces the older one (which itself is a
graph event, recorded).
- Revocation retires a pack's authority going forward; artifacts
already certified under it remain *timestamp-valid* — the
transparency log shows when the claim was valid and when it
stopped. Retroactive invalidation of history is not possible, by
construction. **Key compromise is a different event**: the log
records a "key compromised at T" marker under dual control, and
the verifier treats artifacts signed after T as suspect while
pre-T artifacts stay valid — compromise must not erase history,
and revocation must not launder forgeries.
- Trust-store changes are the operator's explicit act, never the
model's.

**Generation audit and the governing-set snapshot.**
`generate_program` snapshots the governing pack IDs at
generate-start, and the snapshot is recorded in a hash-chained audit
entry: operator, timestamp, pack IDs, model version, artifact hash —
and **hash-commitments to the prompts, never verbatim storage**,
because prompts contain secrets and customer PII, and an immutable
hash chain that stores them is a compliance incident waiting for its
first subpoena. This closes the TOCTOU window: an install or revoke
mid-flight cannot change which law governs an in-progress
generation. The toolchain's own reproducible-build hash is part of
the attestation context — "proved is reproducible" is meaningless
against divergent compiler binaries.

**Verifier hardening — and its honest limit.** `nirdosha verify`
treats `.nir/hi.db` as untrusted input: graph state (including the
non-waivable bit) is never taken as evidence. But standalone
verification is only as strong as its anchors, and in the 5a world
those anchors are thin: the source carries no governing set (it
moved to the sidecar), the local pack store is attacker-writable,
and the sidecar is only trustworthy once 5b signs it. So 5a
standalone `verify` offers **convenience, not proof** — its real
integrity rides on the packs being git-committed (git is the
content-addressing) and on the operator reading diffs. Flipping a
bit in SQLite must not mint a verification, and neither must
flipping bits in the pack store: `verify` re-hashes pack bytes
against their declared IDs and fails closed on mismatch. The
upgrade path is explicit: 5b's signed attestation binds governing
set, artifact, and proofs under a named key, and *that* is when
standalone verify becomes evidence. Without network, revocation cannot
be checked, so `verify` reports "valid as of \<last-known status
list timestamp\>" — never a bare "signature OK" — and all as-of
claims anchor to the transparency-log head, not the local clock.
Because the governing set lives only in the toolchain-written
sidecar, there is no source-level value for an attacker to forge —
`verify` checks the sidecar against the governing set actually
proven, and under 5b checks the sidecar's signature.

**Conflict arbitration, answered by construction.** Two plugins, or
a plugin plus a user prompt, demanding mutually inconsistent
contracts is not a new problem under this design — the proof engine
already answers it, since an unprovable draft fails. What the plugin
adds is *attribution*: the failing contract names the plugin ID that
demanded it, so "your prompt contradicts the banking plugin" is a
diagnostic with a cryptographic source, not a bare UNSAT.

## Compliance profiles — FAPI by default for fintech

A sealed plugin may carry **compliance profiles**: ordered, sealed,
versioned lists of requirements in exactly four generic kinds the
toolchain knows how to check. "FAPI" is never a keyword in
`nirdosha` core — the toolchain learns four requirement kinds, and
the fintech plugin carries the knowledge, covered by the same
signature and pack ID as everything else.

| Requirement kind | Mechanism | Example (FAPI 2.0 Security Profile, Final Feb 2025) |
| --- | --- | --- |
| `validate_contract` | Coverage gate + Z3 (`contract_check::run_program_validates`) | session-scoped invariants the profile cares about |
| `static_rule` | New AST lint stage beside `contract_coverage_check` | every exposed mutating `fn` carries `requires(role:)`; every `serve` route has an explicit exposure decision (deny-by-default already compiler-enforced — the profile turns compiler behavior into an *attested claim*); no plaintext-secret parameters |
| `wiring_requirement` | Mandatory attributes injected by the plugin, rendered into generated config | sender-constrained tokens only (DPoP per RFC 9449 or mTLS cert-bound per RFC 8705 — never naked bearer); PAR (RFC 9126) before authorize; PKCE S256; `iss` checked on the authorization response (RFC 9207); PAR `request_uri` lifetime < 600s |
| `external_conformance` | Attestation slot in the certificate chain, not a compile-time check | OIDF FAPI 2.0 conformance-suite results, content-hashed and attached at deploy time |

The four kinds are the whole core-language surface. A `health`
plugin carries a HIPAA profile over the same four kinds; a
`payments` plugin carries PCI-DSS-flavored rules. The toolchain never
learns what FAPI *is*.

A `wiring_requirement` is enforceable only if the toolchain has a
corresponding emitter. Plugins declaring unsupported wiring
requirements **fail at load**, not at generate — otherwise the model
burns its repair budget against a toolchain deficiency in an
unexitable loop. (A pack-validation pass at load also enforces
taxonomy discipline: a `validate_contract` entry must reference a
proof obligation, not a runtime behavior — the honest-scoping prose
is made a load-time check, so no plugin author can put a runtime
property in the proved column.)

**Honest scoping** — the plugin says this out loud in its sealed
text, and the toolchain enforces the distinction:

- **Provable now (compile time):** authorization gating on every
exposed mutating function (already enforced via
`ExposedMutatingFnMissingRequires`), deny-by-default exposure,
transact ordering, ownership safety. The profile *attests* these
held for this artifact — turning compiler behavior into audit
evidence.
- **Enforceable at generate time (wiring):** the serve/auth
middleware config the profile mandates, because hi *emits* that
layer — mandatory wiring attributes whose absence fails the
coverage gate.
- **NOT provable by us:** authorization-server runtime behavior
(clock-skew windows, code-replay rejection at the AS, TLS
termination), and OIDF certification as a legal/trademark act.
These are `external_conformance` requirements: the deployer must
attach the OIDF conformance-suite report before `certify_code`
issues the compliance claim. The attachment must be verifiable
**against the issuer** — OIDF's published conformance results or
registry lookup, a trusted scanner's signed report — not merely
content-hashed: anyone can hash a fake report, and content-hash
alone repeats the unsigned-plugin mistake at the compliance layer.
The plugin gates the claim; the external suite grounds it.

**Versioning:** profiles pin spec versions — FAPI 2.0 Security
Profile final (2025-02) as baseline, message-signing final (2025-09)
as an optional sub-profile for payment-initiation routes where
non-repudiation matters. Profiles compose by listing both.

**Revocation interplay:** if an erratum or a withdrawn profile
version stales an old plugin's compliance claim, plugin revocation
retires the claim with it — and the transparency log shows *when*
the claim was valid and when it stopped.

### What certification emits

`certify_code`'s attestation, fully determined by sealed inputs:

```javascript
packs:       banking@sha256:9f2c…, fapi-2.0@sha256:71aa…
proofs:      contracts proved 14/14 · static rules 8/8 · wiring 6/6
compliance:  fapi-2.0-security-profile (final, 2025-02)
             external: conformance report sha256:3bd1… attached 2026-09-12
             external: TLS config attestation sha256:e08a… attached 2026-09-12
```

Every line is re-verifiable by anyone holding the artifact: the
profile text is inside the sealed plugin, the proofs are
reproducible, the external attachments are content-addressed.
"FAPI-compliant by default" becomes a property of the artifact, not
a checkbox in a wiki — the same move this RFC makes for domain law,
applied to regulatory posture.

**Point-in-time, not perpetual.** An artifact's compliance claim is
certified against pack P, version V, at time T. That is "was
compliant with the law as sealed" — it is **not** "is compliant with
current law." Regulations change; ongoing compliance requires
re-attestation against the current pack. `nirdosha verify` always
reports the claim's as-of anchor (pack IDs, profile versions,
log-head time) and never an unqualified "compliant." Artifacts and
their governing packs are designed to be retained and re-verifiable
offline for exactly this reason.

*Technical attestation is not legal compliance*: it does not
substitute for regulatory approval or OIDF certification, and the
API name will eventually say `attest_code`, reserving "certification"
for the external legal act.

## What is already domain-baked (honest inventory)

To be clear about what the compiler already enforces without any
plugin — these hold no matter what the model writes:

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

Plugins narrow, never widen. A non-waivable node is a *deletion* of
authority — the console user (and the model, and the operator) loses
the ability to waive the domain's law, which is the point. `:waive`
keeps working for everything the user's own prompt proposed;
`hi_api.rs`'s `/api/waive` and `/api/unwaive` routes return the
refusal for plugin nodes. Trust-store and plugin-lifecycle operations
(`install`/`revoke`) are operator-level acts outside the model's
reach. Nothing new becomes mutable.

## Compatibility

- **Graph schema:** `.nir/hi.db` gains a column (plugin origin /
non-waivable) on the nodes table — `hi_graph::open`'s schema path,
additive.
- **Generate loop:** two new check stages (`contract_coverage_check`,
static-rule lint) + new `self_repair_hint` arms; for plugins with
certified primitives, one prelude prepend before typecheck
(solution 6) + inter-procedural call-site proof obligations for
flagged primitives — budgeted as the largest engineering item in
this RFC (see solution 6), not as lint;
`MAX_SELF_REPAIR_ATTEMPTS` (4) unchanged — dropped-contract
failures count against the same budget, so a model that keeps
dropping plugin contracts still exits.
- **Language:** none. The governing set lives in the attestation
sidecar, not in source. The DSL itself is untouched — verified
against the parser, which has no attribute token.
- **MCP:** `verify_code`/`certify_code` already read whatever
`ast::Program` they're given — plugin-governed or not — so the
standalone `nirdosha mcp` surface is unaffected (and benefits: a
plugin-governed program certifies its domain laws and compliance
posture by default).
- **Existing graphs:** a plugin injected into a graph that already
holds confirmed units merges additively — invariants arrive as new
confirmed units with `RELATES_TO` edges to existing ones; no
existing unit changes.

## Threat model (red team, 2026-09-12)

The findings below are the adversarial pass over this design —
assuming every honest participant does their job, where does an
attacker push? Mitigations already specified inline are cited, not
repeated.

- **T1. Prompt injection via *signed* plugin content.** Signed ≠
benign: a rogue or compromised signer's "driving text" flows into
`units_prompt` rendered as instructions. Mitigation: plugin prose
is structurally treated as *data* — delimited in the prompt,
never rendered as directives — and for `domain_class: regulated`,
manifests minimize free text to identifiers and structured
fields. (This is a prompt-construction invariant in
`hi_llm::units_prompt`, reviewed like the other invariants.)
Residual, stated honestly: the mitigation is fully coherent only
for injected/regulated mode, where the model never needs plugin
prose. In demanded (long-tail) mode the model must *read* the
demand text well enough to author a satisfying contract — that
prose is inherently instruction-like, and only the coverage gate,
not the prompt construction, stands between a demand and a
gamed paraphrase of it. The long tail accepts that residual;
regulated domains do not have to.
- **T2. Provenance-copy bypass.** The seal binds *provenance*, not
content: an operator can paste a plugin's contract text into an
ordinary user prompt as waivable units, and the coverage gate
sees only demands it can waive. Mitigation: verbatim or
near-verbatim copies are detectable via hash-match against plugin
content in non-plugin units, with a warning at confirm time —
but a paraphrased copy, the realistic attack, will not match.
Documented as a convention, not a proof: unattributed copies are
policy violations, and only the sealed path attests.
- **T3. Local-state tampering.** Covered inline: `.nir/hi.db` is
untrusted input (verifier hardening); the pack store is re-hashed
against declared IDs; the operator log's chain head is anchored
off-box. Residual: an attacker with machine control and repo
write access in the 5a world can substitute a *committed* pack —
the pin only binds what's pinned. 5b's signatures are the answer;
5a says so.
- **T4. Time and network.** Covered inline: offline revocation
reports "valid as of", as-of anchors to the log head, trusted
time is the transparency log for regulated domains. Residual: a
verifier with a stale log head verifies against stale law — the
report must state the head it used.
- **T5. Rogue-but-signed plugin as availability attack.** Fail-closed
means a bad pack bricks generation by design. Mitigation inline:
`install --dry-run` runs the pack's contracts against a stub
program; staging before deploy is the operational expectation.
- **T6. Proof-layer gaming.** Covered inline: injection (regulated)
removes contract authorship from the model; non-vacuity catches
unsatisfiable preconditions; fuel-based limits make ENGINE_LIMIT
deterministic; the violated/engine-limit split stops the model
being blamed for solver limits. Residual: functional behavior the
pack does not state is ungoverned — see "residual surface" under
solution 6.
- **T7. Downgrade and algorithm drift.** Covered inline:
`pack_format_version` inside the signature, deprecated versions
rejected, `sig_alg` agility for the post-quantum transition.
- **T8. Machine-control compromise of the toolchain itself.**
Out of scope for any toolchain-local guarantee: a modified
compiler can emit anything. Mitigation is the reproducible-build
hash in the attestation context — verifiers rebuild or compare —
plus the off-box anchors (log, git) that make history's
modification detectable. Stated so nobody mistakes the attestation
for protection against a compromised build host.

## Rejected alternatives

- **A distinct `Money` type in the language** (so "cents" can't be
added to "count"): representation, not law — it would enforce
units but not `balance >= 0` or debits-equal-credits, and it costs
a `typeck.rs`-deep change. Possibly worth it someday for
arithmetic-safety ergonomics; not the mechanism for domain
guarantees. Deferred.
- **Hardcoding banking or FAPI checks into the compiler:** wrong
layer and wrong domain boundary — the compiler must stay
domain-agnostic; baking one domain's law (or one regulation's
profile) into it makes every other program pay for it and makes
the next domain a compiler change.
- **Unsigned plugins / pack registry as trusted database:** without
content addressing and signatures, pack identity is a lookup key
into something attackers can edit. The pack ID *is* the document,
or the whole "no matter what happens" story is optional.
- **LLM domain reviewer per generation:** rejected above —
vigilance is not enforcement; also adds a per-run cost and a new
failure mode (the reviewer's own ignorance) that the proof path
doesn't have.
- **Grammar/sampling constraints (GBNF) as the fix:** syntactic
containment can force `validate` *blocks* to exist (tokens), but
cannot force them to *mean* the domain's law or to prove — that
is typecheck/Z3 territory, not LL(1) grammar territory.

## Open questions

- **Transparency log policy:** worth it for banking-grade plugins;
overkill for the long tail. Make it a per-plugin policy bit in the
manifest.
- **Invariant nodes vs. code units:** does a plugin's
"balance_nonnegative" invariant live as an attribute-carrying
invariant node tied by `RELATES_TO` edge to the fn that must prove
it (the long-tail form), or as actual locked code
(solution 6)? For regulated domains the answer is now decided:
prelude-based primitives, injected contracts, and exclusivity —
no deferral. Multi-file matters only for library *distribution*
of primitives, not for their enforcement.
- **Coverage-gate strictness:** demand a contract per *unit that
asked for one*, or per *program*? Per-unit is the honest form and
the one this RFC assumes; per-program would let the model satisfy
the letter (one contract somewhere) while violating the spirit.
- **Who operates the registries:** the format now *requires* a
registry trust anchor for regulated domains, but which
organizations vouch for "banking" in which jurisdictions is a
social/governance problem the crypto cannot answer. Ship with a
configurable anchor set; expect the governance answer to arrive
by convention, not by code.
- **External-conformance freshness:** how long an attached OIDF
conformance report stays valid before `certify_code` demands a new
one (spec version drift, profile version bumps) — policy bit per
profile, default TBD.
- **Relationship to RFC 0014's generative pipeline:** the coverage
gate changes what "a successful generate" means in 0014's flow;
0014's "Open questions" already anticipated stricter per-unit
locking. This RFC should be reconciled with 0014's scope-cut notes
when either moves.