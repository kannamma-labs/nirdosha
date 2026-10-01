# `hi` UX redesign: options

Scope: everything downstream of typing a prompt into `hi` — the four-mode
console (RFC 0014: prompt → build → generate → publish), the attribute-
attach mechanism for roles/NFRs/screens (`hi_enhance_phase_ux.md`), and
the domain-pack/certification architecture (RFC 0016) — reconsidered
against one constraint: **basic use needs zero new vocabulary, and
advanced use (a new screen, a role, an NFR, a compliance standard)
should feel like the same product, not a mode switch into jargon.**

Grounded entirely in what's real today (cited by file:line) and what's
already *designed* but unbuilt (RFC 0016's domain-pack/compliance
machinery — cited as design, not code). No new compiler work is invented
here that RFC 0016 doesn't already cover; this document is UX only,
answering "how does a person reach the machinery that already exists,
or is already speced, without learning its internals."

---

## Part 0 — Why it feels hard today

**The mode boundary is real, not cosmetic.** `nirdosha hi` is a state
machine — prompt → build → generate → publish
(`rfcs/0014-generative-build-console.md:130-138`) — and today the UI
makes the user *be* that state machine: type `:confirm`, `:attach`,
`:waive`, `:generate`, `:publish` by name in a bottom console
(`hi_graph.html:858-871`), or use the one point-and-click surface that
exists, a 320px node panel with a fixed six-template attribute dropdown
(`hi_graph.html:289-298`). There is no point-and-click confirm/generate
flow beyond that panel, no edge editing, no semantic search — the RFC
says so itself (`0014-generative-build-console.md:66-73`).

**Advanced features are the same six templates, always.**
`requires(role: ...)`, `requires(claim: ...)`, `effect(...)`, three
`nfr(...)` shapes, `validate { ... }`, or raw text
(`hi_graph.html:289-298`) — one dropdown, no visual difference between
"this is enforced" and "this is decoration" until `hi_enhance_phase_
ux.md` closes that gap. A new *screen* isn't attached at all — it's
implied by a struct name following a naming convention
(`get_<snake>`/`list_<snake>`, `ui_gen.rs:1294-1361`) the user never
sees stated anywhere in the UI.

**Certification already has a real, general design — RFC 0016.** The
important thing to internalize before reading Parts 1-4: *the backend
concept for "add a new standard" is not a gap*. RFC 0016 defines
exactly four generic requirement kinds — `validate_contract`,
`static_rule`, `wiring_requirement`, `external_conformance`
(`0016-domain-packs...md:668-678`) — and states outright that a new
compliance regime "nirdosha doesn't already know about (not FAPI/
HIPAA/PCI)... is free if expressible purely in [those] four... kinds
— a new sealed pack carrying it is pure data, no toolchain change"
(same file, "Open questions", last item). Medical, aerospace, IoT are
the same move as FAPI: a domain expert authors a pack; nirdosha never
learns the regulation's name. **What's missing is entirely UX**: no
surface shows a pack, lets a non-expert pick one, or explains what an
attestation means. Status honesty: RFC 0016 is "zero code shipped...
sealed crypto"; only the banking-invariant layer (5a, no signatures
yet) is real (`0016...md:3-19`, `Open questions` last-but-one item).
The options in Part 3 are designed against the RFC's shape so they
don't need rework when 5b (signing) lands — they're UI for a real
spec, not vaporware.

---

## Part 1 — The base experience: options for zero-learning-curve basics

Four genuinely different shapes. Each answers "what does a first-time
user do, with no vocabulary," and "what happens to prompt/build/
generate/publish underneath."

### Option 1 — Conversational thread (modes become narration, not UI)

**Model.** One continuous chat, like this session. No tabs, no console,
no `:verb`. The user describes what they want in prose; `hi` replies in
prose, narrating state transitions instead of naming them: *"I've
sketched an Account, a PaymentRequest, and three screens from that —
want to see them before I build?"* Under the hood this is still
`populate_candidates` → confirm → `generate_program` → publish
(`hi_llm.rs`), but the four mode names never appear as UI labels. The
graph becomes an optional "show me the structure" view one click away
— for the curious, not the entry point.

**How basics stay simple.** There's nothing to learn beyond "type what
you want, answer follow-up questions." Confirm/waive become inline
yes/no chips on each proposed piece, not a separate mode.

**How advanced features fit.** Same channel: "make the payment screen
admin-only" is just another message; the AI translates intent to
`requires(role: admin)` and shows the translation before applying it
(ties directly into Part 2 and Part 4).

**Tradeoffs.** Closest to zero learning curve; reuses a UX pattern
everyone already knows. Weak at: the graph's actual value —
*seeing structure and relationships at a glance* — is what RFC 0014
was built to add over RFC 0012's blind one-shot generation
(`0014...md:107-116`); demoting it to an optional tab risks regressing
exactly the problem 0014 solved. Also the hardest to keep honest about
*what's provable vs. decoration* (Part 2's core problem) in pure prose
— chips/labels do that better than sentences a user skims.

### Option 2 — Guided stepper (wizard with a visible spine)

**Model.** A persistent horizontal step tracker — **Describe → Review →
Build → Run** — always visible, current step highlighted, done steps
clickable to revisit. Each step is one focused screen: Describe is a
text box; Review is the *current* node panel's content reflowed as a
checklist ("12 pieces found — tap any to rename, remove, or add a
rule"); Build is a progress view with live log lines in plain language
("checking your rules… fixing an issue… done"); Run is the working app
plus a "make a change" button that jumps back to Review.

**How basics stay simple.** The stepper itself teaches the shape
("there are four things that happen") without requiring the four
*names* prompt/build/generate/publish to ever appear — label them by
what a user does, not what the compiler does.

**How advanced features fit.** Review is where role/NFR/screen editing
lives (Part 2's contextual micro-forms attach here); nothing new opens
elsewhere.

**Tradeoffs.** Very learnable — a stepper is a familiar pattern from
setup wizards everywhere — and keeps the graph's structural view alive
as a real step, not a demoted tab. Weaker at power-user speed: going
back and forth between Review and Build repeatedly (the realistic
iterate-until-right loop) means re-walking the spine each time, which
can feel slower than a single persistent canvas once someone knows
what they're doing.

### Option 3 — Live-preview-first (edit the app, not the model)

**Model.** Skip showing structure at all, initially. First response to
a prompt is the *running app itself* — real screens, sample data — full
width. A user's "advanced" edits happen **by directly manipulating the
preview**: right-click a field → "hide from employees" writes
`requires(role:)` behind the scenes; a "+ New Screen" button in the
preview's own nav bar starts a screen from a live blank page, not from
declaring a struct first. The graph, and the vocabulary in it, becomes
a strictly optional "under the hood" inspector.

**How basics stay simple.** Nothing to learn: the product looks and
acts like the thing it's building, always. This most directly answers
"I don't want people to learn anything new to operate basic features."

**How advanced features fit.** Best-in-class *if* every advanced
feature has a visual analogue in the running app (a role gate hides a
button; an NFR usually doesn't visibly change anything, which is this
option's real weak spot — see below).

**Tradeoffs.** Strongest "what you see is what you get" feel, and the
one option that makes a generated screen's reality checkable at a
glance (does the button actually disappear for a non-admin preview
identity — directly testing Part 3's "looks gated, isn't" bug by
construction, since you're looking at the *running* preview, not a
declared intent). Weakest at anything with no visual shape: an NFR
(`nfr(latency_ms: 200)`), a `validate` contract, a compliance profile —
none of these render as anything you can click on in a live preview,
so this option still needs a side panel for exactly the things Part 2/3
care about most. Also the most implementation-heavy: it requires the
served app and the editing surface to be the same view, which the
current architecture (a separate generated binary, `hi_api.rs::
handle_publish`) doesn't do today.

### Option 4 — Unified canvas, modes dissolved into it (closest to "the current idea")

**Model.** Keep the graph canvas — the part the user said they like —
but remove mode as a *place you go*. One surface, always: nodes appear
as you describe things (today's Prompt mode), get a soft "reviewing"
ring you clear with a click (today's confirm), and the canvas
recompiles continuously in the background the moment enough is
confirmed (today's Generate), the way a modern IDE type-checks as you
type rather than on a separate "build" button press. "Publish" becomes
a single always-present button in the corner — enabled once there's
something valid to publish, disabled with a one-line reason
otherwise — not a fourth destination. Console commands (`:confirm`,
`:attach`, ...) stay as an accelerator for people who want to type, but
every one of them has a direct-manipulation equivalent: click a node to
confirm it, drag a template chip onto a node to attach it, and so on.

**How basics stay simple.** A user who never opens the console and
never learns a `:verb` can still do everything — describe, watch pieces
appear, click to accept, watch it build, hit the one publish button.
The vocabulary that exists today only in `:command` form
(`hi_graph.html:858-871`) becomes discoverable through the panel/canvas
itself, the same "menu teaches the shortcut" relationship most editors
use.

**How advanced features fit.** Directly hosts Part 2's contextual
micro-forms (hover a node → small "+role" / "+NFR" / "+screen"
affordances) and Part 3's compliance-pack halo (a visible boundary
around plugin-governed nodes, distinct from user nodes) without adding
a fifth surface.

**Tradeoffs.** Preserves the most of what's already built
(`hi_graph.rs`'s confirm/lock/waive model, the live graph view) — lowest
migration cost of the four, and the option that best satisfies "I like
the current idea" literally. The real design work is continuous-
recompile UX: today's Generate is a bounded, visible, sometimes-slow
self-repair loop against an LLM (`MAX_SELF_REPAIR_ATTEMPTS`,
`hi_llm.rs`) — making that feel like ambient background compilation
without ever silently eating an API-cost-and-latency operation the
user didn't ask for is a real interaction-design problem, not a
skin change (see Part 5's sequencing note).

### Comparison

| | 1: Conversational | 2: Guided stepper | 3: Live-preview-first | 4: Unified canvas |
|---|---|---|---|---|
| Learning curve, basics | Lowest | Low | Lowest | Low–medium (console optional) |
| Preserves graph's structural value | Weak (demoted) | Yes (a real step) | Weak (demoted) | Strongest |
| Best fit for NFR/contract/compliance (no visual shape) | Weak in-channel | Good (Review step) | Weakest | Good (side panel) |
| Migration cost from today | Medium | Medium | Highest | Lowest |
| Power-user speed once learned | Medium | Lower (re-walks spine) | Medium | Highest |

---

## Part 2 — Making roles, NFRs, and screens intuitive

These options assume Option 4 (or 2) as the host surface but are mostly
portable to any of Part 1's choices — they're about *how one attribute
gets attached*, not where the canvas lives.

1. **Plain-language micro-forms, jargon hidden behind "view code."**
   Replace the six-template dropdown (`hi_graph.html:290-298`) with
   task-named affordances: "Who can use this?" (→ role/claim),
   "Any performance or reliability requirement?" (→ nfr), "Add a rule
   that must always hold" (→ validate). Each opens a tiny, single-
   purpose form in the user's words ("Only people with the role:
   `[ admin ▾ ]` can do this") that compiles to `requires(role: admin)`
   behind a toggle-able "view generated attribute" line — so a curious
   or advanced user can see the real syntax without a beginner ever
   needing to.
2. **Natural-language attachment, AI-translated.** A single free-text
   field per node — "type a rule in plain English" — parsed by the
   same LLM path already in the pipeline into the right attribute
   shape, shown back for confirmation before it's attached ("I read
   that as: `requires(role: finance_director)` — attach?"). Lowest
   friction, but needs a translation-confidence UI: what happens when
   the model isn't sure, or over-fits a vague sentence to the wrong
   template. Pairs naturally with Part 4's AI layer — it *is* an
   enhance-phase interaction, not a separate mechanism.
3. **Visual role/permission matrix.** For role-shaped rules
   specifically (the highest-value case, per the fintech incident that
   started this thread): a spreadsheet-style grid, screens/actions down
   the rows, roles across the columns, checkboxes at the intersections.
   Ticking a box writes `requires(role:)`; the grid also *reads back*
   current state at a glance — "which roles can see the Accounts
   screen" is one row, not a click-through-every-node exercise the
   current panel-per-node model makes tedious. Doesn't generalize to
   NFRs/contracts (they're not role×screen shaped) — pair with #1 for
   those.
4. **Contextual "+" affordances directly on the canvas.** Hovering any
   node (Option 4's canvas) reveals small inline buttons — "+screen",
   "+role", "+NFR" — right where the thing they add will visually
   attach, instead of requiring a click-to-select-then-scroll-the-side-
   panel round trip. Screens specifically: "+screen" on a struct is the
   first UI acknowledgment that a screen even *is* a thing you add
   (today it's an invisible naming-convention side effect,
   `ui_gen.rs:1294-1361`) — worth building even if nothing else in this
   document ships, since it turns a silent compiler convention into a
   real, discoverable action.
5. **"Explain this" — the inverse operation.** Click any generated
   piece and see, in plain language, everything that governs it right
   now: which roles can reach it, which NFRs apply, which contracts are
   proved vs. merely stated. This is `hi_enhance_phase_ux.md`'s Option
   A (chip labels) generalized from "one attribute list" to "click
   anything, understand its current guarantees" — legibility of
   *existing* state is a precondition for adding more state
   intelligently, and it's the cheapest of these five to build (mostly
   read-only rendering over data the compiler already produces).

---

## Part 3 — Certification and new standards (FAPI, medical, aerospace, IoT), as a UX problem

Reminder from Part 0: the backend generalizes today, on paper
(`0016...md`, four requirement kinds). Everything below is front-of-
house design for that real spec.

1. **A pack gallery/marketplace, browsed like a dependency list.**
   A screen (reachable from anywhere, not nested in a mode) listing
   installable domain packs — Banking, FAPI 2.0, a HIPAA-flavored
   health profile, a DO-178C-style aerospace baseline, an IoT device-
   security baseline — each a card in plain language: *what it locks
   down* ("every money-moving function needs a role check, proven, not
   just written"), *what it can't guarantee* (RFC 0016's own "honest
   scoping" concept, `0016...md:690-713`, surfaced as UI text instead
   of RFC prose), and a trust indicator (who signed it, self-signed vs.
   registry-issued — `domain_class`, same file). Installing a pack is
   presented the way adding a dependency is in any modern tool — a
   search, a one-click add, a visible entry in a "what's governing this
   project" list.
2. **Proactive suggestion at prompt time (the AI layer, Part 4
   overlap).** The enhance phase reads the free-text prompt for domain
   language — "bank," "patient," "flight," "device firmware" — and
   offers the matching pack *before* generation starts: *"This sounds
   like it handles payments — attach the FAPI 2.0 pack? It'll require
   proof that every money-moving action checks who's allowed to call
   it."* Declining is one click; nothing is forced. This is the answer
   to "a user shouldn't need to know packs exist" — the product
   suggests the vocabulary instead of requiring it up front.
3. **A visible "law" boundary on the canvas, not a silent lock.**
   Plugin-sourced nodes are non-waivable by design
   (`0016...md:253-258`) — that has to be *visible*, or it reads as a
   bug ("why won't this delete?") rather than a guarantee. A distinct
   visual treatment (a soft halo/border color, a small lock glyph)
   marks anything a pack owns, and clicking it explains in one sentence
   why it can't be removed here, naming the pack. This turns RFC 0016's
   "narrows, never widens" permission model (`0016...md:779-786`) into
   something a user *feels* is protecting them, not obstructing them.
4. **A Certification panel, read in plain language.** Parallel to the
   canvas (a tab, not a mode): a live checklist per installed pack —
   "14/14 rules proved," "6/6 required security settings wired," "1
   item needs your input: attach your lab's external test report" — one
   line per requirement *kind* from RFC 0016's four
   (`0016...md:668-678`), never the kind's internal name. A single
   "Get certificate" action produces `certify_code`'s attestation
   (`0016...md:725-741`), rendered as a shareable one-pager: packs
   governing this build, what's proved, what's attested externally,
   and the point-in-time caveat ("valid as of this build — re-check
   after any change," `0016...md:744-752`) stated in one sentence, not
   left implicit the way a raw JSON attestation would.
5. **A drafting wizard for a *new* standard, not just installing an
   existing one.** For "we need something like FAPI but for our
   internal loyalty-points system" — a guided flow that walks a domain
   expert through RFC 0016's four generic kinds one at a time in plain
   language ("Is there a rule that must always be mathematically true?
   → contract. Is there a structural rule about which functions need
   what? → static rule. Does this need specific security wiring on
   every request? → wiring requirement. Is there an outside audit or
   report involved? → external conformance.") and emits a **draft**
   pack for the expert to review and sign — the wizard drafts, a human
   still seals it (preserves RFC 0016's "domain expert bakes once,
   signs" model, `0016...md:116-124`; the AI never gets signing
   authority). This is the UX answer to "how do we add medical/
   aerospace/IoT standards" without waiting on nirdosha's own team to
   hand-build each one.

---

## Part 4 — AI-based enhancement, woven through, not bolted on

Revisits the three enhance-phase options already scoped
(`hi_enhance_phase_ux.md`) and places them inside the base experience,
plus new AI surface area the "no learning curve" goal opens up.

- **Enhance-phase options A/B/C, relocated.** Option A's chips become
  Part 2's "Explain this" (#5) — same idea, generalized past just the
  attribute list. Option B's coverage gate is orthogonal to all of
  this — it's a compiler-side guarantee, not a UI choice, and composes
  with any base option. Option C's review checkpoint fits most
  naturally into Option 2 (Guided stepper)'s Review step or Option 4's
  publish-gate reason text — a natural checkpoint already exists there
  in both, so it's less "a new screen" than "content on a screen that
  was already going to exist."
- **"Ask hi," promoted, not buried.** `/api/ask` already exists
  (`hi_api.rs`, RFC 0014's scope line) but lives only in the console's
  `:ask` verb. Surface it as a persistent, always-visible question box
  — "what would break if I remove the balance check?", "is this
  FAPI-ready yet?", "who can currently see account balances?" — the
  single most direct way to make an advanced concept (compliance
  status, blast radius of a change) answerable in the user's own words
  instead of requiring them to read a graph or a diagnostic.
- **Proactive gap-filling**, i.e. the original "enhance phase" ask from
  earlier this session — suggesting missing NFRs/roles/workflows —
  belongs inside whichever base surface is chosen as *more of the same
  interaction*, not a special mode: in Option 1/3, it's another message
  in the thread; in Option 2, it's extra rows appearing in the Review
  step, tagged "suggested" vs. "you asked for this"; in Option 4, it's
  new nodes on the canvas with a distinct "proposed by hi" visual state
  before a user accepts them into the normal confirmed set.
- **Plain-language change summaries.** After every generate/publish, a
  one-paragraph, no-jargon summary of what changed and why it's safe
  ("Added the finance-director approval screen. Nothing else changed.
  All 6 money-math rules still hold.") — replaces reading a console log
  or a diff for the common case, escalating to the real log only when
  asked.
- **Where AI does *not* get authority.** Consistent with RFC 0016's own
  stance (the model wires the ledger, never writes it;
  `0016...md:27-29`) and Part 3 item 5 above: AI drafts, suggests, and
  translates, everywhere in this document — it never silently attaches
  a role, seals a compliance pack, or force-generates without a
  human's visible acceptance step. That boundary is what keeps "no
  learning curve" from becoming "no control."

---

## Part 5 — A synthesis, and what's cheap vs. expensive

Given "I like the current idea" — **Option 4 (unified canvas)** as the
backbone reads as the best fit: it keeps the most already-built value
(the live graph, `hi_graph.rs`'s confirm/lock model), and every other
part of this document attaches to it without inventing a fifth
surface — Part 2's contextual affordances live on its nodes, Part 3's
pack gallery and certification panel are two more tabs beside it, Part
4's AI layer speaks through the same canvas instead of a separate chat
window.

Rough sequencing, cheapest/lowest-risk first (each stands alone; none
requires the others to ship first):

1. **Part 2 #5, "Explain this."** Pure read-side rendering over data
   the compiler already emits. No new compiler work.
2. **Part 2 #4, contextual "+screen"/"+role"/"+NFR."** UI-only; writes
   through the exact same `/api/attach` route that exists today
   (`hi_api.rs:285-293`).
3. **Part 3 #3, the visible "law" boundary.** UI-only *today*, against
   the real (if unsigned) 5a plugin data (`banking-v0`'s
   `pack.json`) — the non-waivable bit already exists in the graph
   schema per RFC 0016's compatibility section.
4. **Part 4, "Ask hi" promoted + plain-language summaries.** Wraps
   `/api/ask` and the existing generate log; no new compiler surface.
5. **Part 2 #1/#2/#3 (micro-forms, NL attachment, role matrix)** and
   **Part 3 #1/#2/#4 (pack gallery, proactive suggestion, certification
   panel).** Real UI builds, still against today's real routes and
   RFC 0016's real (5a) data — but bigger scope than 1-4.
6. **Option 4's "continuous recompile" feel, and Part 3 #5 (standard-
   drafting wizard).** The two genuinely hard interaction-design
   problems in this document — background compilation that never
   surprises the user with cost/latency, and a wizard whose output is
   trustworthy enough for a domain expert to actually sign. Budget
   these separately; don't let them block 1-5.
7. **Anything depending on RFC 0016's 5b (real signatures,
   transparency log, external registries).** Out of this document's
   control — the UX for it (trust badges, revocation status) is
   designed above so it's ready the day 5b ships, but the pack-gallery
   trust indicators show "unsigned / trust-on-first-use" honestly
   until then, per the RFC's own disclosure rule
   (`0016...md:224-230`).

## Open question this document doesn't resolve

Which of the four Part 1 base experiences to actually commit to is a
product bet, not an engineering one — it decides the shape of every
later screen. This document recommends Option 4 on stated-preference
grounds ("I like the current idea") and lowest migration cost, not
because the others are wrong; Option 3 (live-preview-first) is worth a
second look specifically if the near-term goal shifts from "power users
who confirm/attach" toward "non-technical stakeholders who just want to
see and poke at the app" — a different audience than this document
otherwise assumes.
