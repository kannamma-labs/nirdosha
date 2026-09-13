# Enhance-phase UX: three design options

Scope: the "enhance" step of `nirdosha hi`'s Build mode — attaching
attributes (`requires(role: ...)`, `requires(claim: ...)`, `effect(...)`,
`nfr(...)`, `validate { ... }`, or raw `custom` text) to a `CodeUnit`
candidate, between Prompt mode (candidate creation) and Generate mode
(compile). Grounded entirely in the current implementation; every claim
below cites `file:line`. No implementation code — this is interaction
design only, for a human to choose between.

## What the current enhance phase actually does

The attribute editor is a fixed template list of six annotation shapes
plus a free-text escape hatch (`crates/compiler/src/hi_graph.html:289-298`),
reachable two ways: the click panel's form
(`hi_graph.html:316-415`) or the console's `:attach <node-id> <attr>`
(`hi_graph.html:858-871`). Both paths post to `/api/attach`
(`hi_api.rs:285-293`), which calls
`hi_graph::attach_attribute` — and that function's own doc comment
records the load-bearing fact: "Deliberately not parsed/validated
against the language's real annotation grammar here... Generate mode's
real compile step is what actually proves an attribute is legal, by
*rejecting* a generated program that misuses one"
(`hi_graph.rs:779-786`). The HTML's own comment on the template list
says the same thing from the UI side: "nothing here parses or validates
the built string against the actual `.nir` grammar"
(`hi_graph.html:279-288`).

Concretely: clicking Attach (or typing `:attach`) always succeeds
immediately — `attach_attribute` only fails if the node id doesn't
exist (`hi_graph.rs:679-686`) — and the only feedback is an inert log
line, `'attached to ' + node.id + ': ' + attr` (`hi_graph.html:371`,
`hi_graph.html:867`). Nothing runs a compiler, a linter, or any check at
attach time. The unit's real fate is decided later, at `:generate`
(`hi_graph.html:904-915`, `hi_api.rs:328-364`), which bundles *every*
confirmed unit into one combined-file compile with a bounded,
4-attempt self-repair loop (`MAX_SELF_REPAIR_ATTEMPTS`,
`hi_llm.rs:663`) — described in the console's own copy as something
that "can take a while."

Critically, not all six attribute shapes get the same treatment once
generation runs. RFC 0016's contract-coverage gate
(`hi_llm.rs:723-849`) makes `validate contract: ...` demands
publish-blocking: if a confirmed `fn` unit demands a contract and the
generated code drops it or the contract doesn't Z3-prove, that's a
`CoverageFailureClass::ContractDropped`/`ContractViolated`/etc. failure
that consumes self-repair budget and, if unresolved, fails `:generate`
and re-fails `:publish`'s own re-check (`hi_api.rs:391-399`). But
`requires(role: ...)` / `requires(claim: ...)` have **no equivalent
gate** — grep of every `CoverageFailureClass` variant
(`hi_llm.rs:742-760`) turns up nothing role- or claim-shaped. The only
place role/claim handling shows up at all is a *reactive*
`self_repair_hint` arm that teaches the model the right
`VerifiedIdentity`/`check_role`/`acquire` builtins — but only after a
compile attempt has already produced a diagnostic from misusing them
(inventing a `User` struct, say) (`hi_llm.rs:1258-1262`). A function
that typechecks fine but simply never calls `check_role` at all — the
"looks gated, isn't" failure — triggers no diagnostic, burns no
self-repair budget, and blocks nothing. This is the seam the three
options below all address: the enhance phase currently gives a fintech
role-gating requirement a false sense of the same rigor a `validate
contract` gets, with the checkpoint that actually distinguishes them
sitting far downstream (or not existing at all).

**Fintech scenario used in every walkthrough below:** a user prompts
`hi` with "a payments app with wire transfers between accounts,"
`:confirm`s the resulting `fn transfer_funds(from, to, amount_cents)`,
and wants only an `admin`-role identity able to call it before
generating.

---

## Option A — Inline Legality Lint

**Core interaction model.** The attach flow stays exactly as it is
today — same form, same `:attach` command, same instant response — but
the response itself gets richer. `/api/attach`'s success path
(`hi_api.rs:289-292`) returns a *classification* of the attribute
alongside the plain OK: syntactically well-formed vs. malformed against
a known template shape, and — the new information — whether this
attribute *kind* is ever checked automatically downstream (`validate
contract` demands are; `requires(role/claim)`, `effect`, `nfr`, and
`custom` text are not, per the gate's own scope,
`hi_llm.rs:742-760`). The UI renders each attribute in the node panel's
existing `<ul class="attr-list">` (`hi_graph.html:322-330`) as a chip:
green "provable" for a contract demand, amber "unverified — not
auto-checked" for everything else, red "malformed" for text that fails
a lightweight grammar sniff. No new screen, no new step — the enhance
phase looks identical, just better-labeled.

**Walkthrough.** The user selects `fn:transfer_funds` in the panel,
picks `requires(role: ...)` from the template dropdown
(`hi_graph.html:333-339`, `ATTR_TEMPLATES.requires_role`,
`hi_graph.html:290`), types `admin`, clicks Attach. The attribute list
immediately shows `requires(role: admin)` tagged amber: "unverified —
Generate mode will accept this even if the code never checks it; review
the generated `transfer_funds` yourself." The user also attaches
`validate contract must_check_role: ...` on the same unit (if the
language surface supports stating that demand in attribute prose,
mirroring `hi_llm.rs:781-800`'s `demanded_contract` convention) and
sees it tagged green: "provable — `:generate` will refuse to lock this
unit until Z3 proves it." The distinction is visible before the user
ever runs `:generate`, at the moment they're deciding what to type.

**Tradeoffs.** Good at: zero added friction, cheap to build (a
string-classification function plus a CSS chip, no new gate, no change
to `:generate`'s semantics), and it never blocks or nags — it only
informs. Bad at: it cannot actually catch the bug. A role-gating
requirement can be perfectly well-formed text and still be silently
dropped by the generated code; this option only ever says "we won't
check this for you," never "this one *is* wrong." A user in a hurry can
read the amber chip once, stop reading it by the fifth attribute, and
ship exactly the same false sense of safety the current UI gives today
— it just documents the gap instead of closing it. Implementation cost:
low. Nagging risk: essentially none (it's a label, not an interruption)
— but that low-friction property is also its ceiling: nothing about
this design pressures the user to act differently.

---

## Option B — Role/Claim Coverage Gate

**Core interaction model.** Extend RFC 0016's existing coverage-gate
pattern (`hi_llm.rs:723-849`) — currently scoped to `validate contract`
demands only — to `requires(role: ...)` / `requires(claim: ...)`
demands too, with a new `CoverageFailureClass` variant alongside
`ContractDropped`/`ContractViolated`/etc. (`hi_llm.rs:742-760`). The
check is structural, not a full Z3 proof (there's no predicate to
prove, just a call that must exist): does the generated `fn` for a unit
carrying `requires(role: X)` actually call `check_role(identity, "X")`
(or route through an `acquire` on such a check) somewhere on every
path? If not, `:generate`'s self-repair loop
(`hi_api.rs:328-364`, `hi_llm.rs:1626-1785`) treats it exactly like a
dropped contract: a diagnostic, a repair-budget charge, a targeted
`self_repair_hint` reusing the role/identity teaching that already
exists for a *different* trigger (`hi_llm.rs:1258-1262`), and — if the
budget runs out — a `:generate`/`:publish` failure the user cannot miss
the way they can miss an amber chip. The enhance phase's attach form
itself is unchanged; the new rigor shows up entirely downstream, at the
same checkpoints `validate contract` already uses.

**Walkthrough.** The user attaches `requires(role: admin)` to
`fn:transfer_funds` — identical click, identical instant "attached"
response as today. They `:confirm` and run `:generate`. The model's
first draft writes `transfer_funds` with no role check at all (a real,
observed failure mode: the annotation is prompt decoration the model
can simply not act on, the same "decorative enum" class RFC 0014's own
edge-wiring fix was written to close, `hi_graph.rs:887-896`). The new
gate flags it: `role coverage failure: fn transfer_funds demands
requires(role: admin) but its body never calls check_role`. The console
log (`hi_graph.html:908`, dumping `body.log`) shows this as one more
self-repair round; the model's next draft adds the `check_role`/
`acquire` call, coverage passes, and `:generate` reports `locked 1
unit(s)`. If the model can't fix it within budget, `:generate` fails
outright with the same diagnostic — the user knows, before ever
running the app, that the role gate is missing, rather than discovering
it in a security review after `:publish`.

**Tradeoffs.** Good at: catching exactly the failure this whole
document exists because of — the case where the attribute is well-typed
prose that the generated code quietly ignores — using the same
enforcement mechanism (a publish-blocking gate) the codebase already
trusts for contracts. It composes with the existing self-repair loop
for free once the check exists. Bad at: it's a structural check, not a
semantic one — it can confirm `check_role(identity, "admin")` was
*called*, not that the call sits on every code path that actually needs
it, or that "admin" wasn't misspelled as a different string the rest of
the program never issues; a determined-enough (or merely careless)
draft can satisfy the letter of the gate while still leaving a bypass.
It also touches more surface than Option A: a new `CoverageFailureClass`
variant, a new structural walker over the generated AST (parallel to,
but distinct from, the contract walker), new `self_repair_hint` wiring,
and new test coverage matching the existing
`self_repair_hints_cover_every_field_failure_class`-style discipline
(`hi_llm.rs:2249`) — real design and engineering cost, not a UI tweak.
Nagging risk: low for a correct draft (it never surfaces if the model
gets it right first try, same as contract coverage today) but a
role/claim demand that's genuinely hard for the model to satisfy could
now burn self-repair attempts and fail `:generate` runs that would
previously have silently "succeeded" — a behavior change some users
will read as `hi` getting stricter, not safer.

---

## Option C — Enhance Review Checkpoint

**Core interaction model.** A new, explicit gate between `:confirm` and
`:generate` — structurally parallel to the confirm step Build mode
already has (`hi_graph.rs:724-733`, "confirmation is an explicit,
dedicated action, never a side effect of anything else in this list")
but scoped to attributes rather than driving text. Before `:generate`
runs, the console (or a dedicated panel) shows one summary screen per
run: every confirmed unit's attached attributes, grouped by the same
provable/unverified split Option A introduces, with an explicit
acknowledgment control — "3 attributes will be proof-checked; 2
(`requires(role: admin)`, `nfr(latency_ms: 200)`) will not be
automatically verified — I understand, generate anyway." `:generate`
itself doesn't proceed until this is dismissed once per graph state (a
new attach, edit, or waive since the last review re-arms it, mirroring
how `edit_driving_text` already unlocks a previously-locked unit on any
edit, `hi_graph.rs:773-777`).

**Walkthrough.** The user attaches `requires(role: admin)` to
`fn:transfer_funds`, then types `:generate`. Instead of jumping
straight into "generating from every confirmed candidate…"
(`hi_graph.html:905`), the console shows the review screen: `transfer_
funds — requires(role: admin) [unverified — hi cannot confirm the
generated code enforces this; verify by hand after generate]`. The user
reads it, decides that's an acceptable risk for now (or goes back and
also attaches a `validate contract` demand that *is* checkable, per
Option B's language surface, before proceeding), and clicks "generate
anyway." Generation proceeds exactly as it does today from that point.
On the *next* `:generate` call, if nothing about the unit's attributes
changed, no review screen reappears — only a change re-arms it.

**Tradeoffs.** Good at: forcing informed consent at the one moment
(right before the slow, model-driven `:generate` call) the user is
already stopping to think, without requiring any new compiler-level
verification machinery — it's a UI/state-machine addition on top of
data the system already has (`node.confirmed`/`node.attributes`,
already read at `hi_graph.html:442-449`). It degrades gracefully: even
if Option B is never built, this still tells the user honestly which
attributes are and aren't covered, which the current UI never states at
all. Bad at: it adds a real interaction cost to every `:generate` call
that touches an unverified attribute, and — unlike Option A's passive
chip — an ignored acknowledgment click carries no less false confidence
than today's silent attach; a user who reflexively clicks through
"generate anyway" learns nothing more than the current flow teaches
them, just with one extra keystroke. It's also the option most exposed
to nagging fatigue: a project with several role-gated functions will
re-show this screen on every attribute edit that touches any of them,
and the "re-arm on any change" rule needed to keep it honest is also
what makes it reappear often on an actively-iterated graph.
Implementation cost sits between A and B: no new compiler check, but a
new UI mode/state machine, a new persistence question (per-graph-state
dismissal), and a new console/panel flow to design and test.

---

## Comparison

| | A: Inline Legality Lint | B: Role/Claim Coverage Gate | C: Enhance Review Checkpoint |
|---|---|---|---|
| Where it acts | At attach time (label only) | At generate/publish time (blocking) | Between confirm and generate (checkpoint) |
| Catches the "looks gated, isn't" bug | No — only discloses the gap | Yes, structurally (call exists) | No — surfaces the gap, doesn't verify it |
| Changes `:generate`/`:publish` behavior | No | Yes — can fail runs that pass today | No |
| New UI surface | A chip on existing attribute list | None (reuses existing log/error surfaces) | A new review screen/step |
| Implementation cost | Low | High (new AST walker, new failure class, new tests) | Medium (new state machine, no new compiler logic) |
| Added friction per use | None | None on success; failure on a bad draft | One click, until attributes next change |
| Nagging risk | Low (easy to stop reading) | Low (silent on a correct draft) | Medium–high (re-arms on every edit) |
| False confidence if user disengages | Same as today | Reduced (gate still runs regardless) | Same as today |

## Open question

Should `hi` verify that a stated non-functional/security requirement
(role gating, claims, latency, etc.) was actually honored by generated
code — extending the same publish-blocking rigor `validate contract`
already gets (Option B) — or is it Nirdosha's stance that only formally
provable requirements ever get that treatment, with everything else
staying the human's own review responsibility, just made honestly
visible (Options A/C)? That's a scope decision about what `hi` promises,
not a UI detail, and it decides which of these three is even the right
shape to build.
