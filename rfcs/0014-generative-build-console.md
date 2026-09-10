# RFC 0014: The generative build console — prompt → build → generate → publish, over an interactive Realm graph

> **Status: speculative design capture, nothing built.** This RFC
> transcribes a design conversation (2026-09-10) that deliberately
> worked "alternate reality" first — lay out the whole shape of the
> idea before checking any of it against what's actually feasible —
> and is written down at that stage on purpose, not after a feasibility
> pass. Treat this as a proposal to react to and cut down, not a
> committed roadmap. Section "Open questions" below carries more real,
> unresolved weight than usual for this repo's RFCs, on purpose — most
> of the load-bearing questions are answered directly in Design, where
> the answer actually belongs; what's left there is what's genuinely
> still undecided.
>
> **Scope.** This RFC covers every way a human sees or interacts with
> the Realm graph via `hi`, not only the 3D webview below. `hi`'s
> `:ask`/`:impact` console verbs (`hi.rs`'s `Command::Ask`/
> `Command::Impact`) are this RFC's already-shipped baseline UI — the
> same `realm::ask`/`realm::impact` query functions the graph exposes,
> rendered as flat text rather than a graph. RFC 0013 owns the graph
> itself and the programmatic, CI-facing `nirdosha realm ...` CLI;
> this RFC owns everything downstream of that, from the plain console
> verbs already running today to the speculative 3D view described
> below.

## Motivation

RFC 0012 gave `nirdosha hi` NL-to-`.nir` generation with a bounded
compiler-feedback self-repair loop — but it's a black box: one prompt
in, one pass/fail build out, no visibility into *what* got created
along the way, and no way to correct structure short of regenerating
from scratch. RFC 0013 gave `hi` a local knowledge graph
(`.nir/realm.db`) with bidirectional code↔requirement traceability —
but the only way to see it is `:ask`/`:impact` printing a flat text
list into the console transcript.

Neither closes the actual gap: **a user handing `hi` a substantial
prompt has no way to watch the structure it implies take shape, catch
a misunderstanding before code exists, attach the language's richer
annotations (`requires`, `nfr`, `validate`, screen field masking)
without dropping into hand-written `.nir`, or get from "generated" to
"running somewhere"** without leaving the console entirely. This RFC
sketches a four-mode workflow — **prompt → build → generate →
publish** — with an interactive graph view of the Realm `CodeUnit`
graph as the build-mode surface, aimed at closing that gap end to end.

## Design

### State machine overview

```
prompt --(populate the Realm graph)--> build --(materialize .nir)--> generate --(deploy)--> publish
```

Entered automatically on `nirdosha hi` startup, after RFC 0013's
existing auto-scaffold (`hi::open_realm_or_warn`) — this RFC doesn't
change that step, it adds what happens after the console is up.

### What's authoritative, when

Every other decision in this RFC depends on answering one question
first, so it goes first: **at each point in the state machine, is the
Realm graph or the `.nir` source the thing the user should trust?**

- **Prompt → build:** the graph is authoritative. No `.nir` file
  exists yet; everything the user is correcting — structure,
  attributes, gates — lives only in `.nir/realm.db`.
- **Build → generate:** the graph stays authoritative going in —
  generate mode's job is to make the `.nir` files match the graph, not
  the reverse. A unit whose generated source doesn't actually satisfy
  what the graph says it should (its text, its attached `validate`/
  `nfr`, its gates) does not silently materialize anyway: it fails to
  lock, the same way a failed self-repair attempt in RFC 0012 doesn't
  silently ship broken code. The node stays unlocked and visible as
  such; build mode's own editing loop is where the user reconciles it
  — edit the text, re-attempt generation.
- **Generate → publish:** once a unit locks (see "Generate mode"'s own
  precise definition below), the *compiled* `.nir` becomes
  authoritative for that unit — it's the thing being deployed, not the
  graph node that produced it. A locked unit's graph node and its
  `.nir` are meant to agree by construction; if the user edits the
  graph again after locking, RFC 0013's own mechanism already answers
  this without new design: the edit flags the relevant edges/nodes
  `possibly_stale`, generate mode re-runs for that unit, and it
  re-locks once it agrees again.

This is deliberately the same answer RFC 0013 already gives for
human-authored code drifting from an approved requirement, applied to
LLM-authored structure drifting from generated code: **flag, never
silently overwrite, on either side.** The graph never claims a truth
the compiled code contradicts, and the compiled code never silently
overrides what the graph says was approved.

**Build order this implies:** authority semantics (above) → prompt
mode's funnel → build mode → generate mode → publish. Each stage's
behavior is defined in terms of the one before it — there's no safe
way to build generate mode's lock semantics, say, before prompt mode's
funnel exists to say what a "unit" even is with any real confidence.

### 1. Prompt mode

The default mode on entry. This is deliberately not "send the raw
prompt to the LLM and hope": **populating a graph from prose is
Problem B from this project's own document-structure-
discovery work, just at prompt scale instead of PRD scale** (the
`nirdosha_realm_v2_design_backlog` project memory has the full
formulation) — so it gets the same cheap → expensive funnel every
other Realm feature already uses, not a single unbounded LLM call:

1. **Deterministic candidate extraction first (Tier 0, no LLM).**
   Regex/schema-guided probes over the prompt text for the same kind
   of signal structure-discovery already identified as cheap and
   reliable — noun phrases that look like `struct` candidates, verb
   phrases that look like `fn` candidates, explicit numbers/units that
   look like `nfr` thresholds. This can't produce a full graph on its
   own, but it bounds and grounds what the LLM is asked to do next.
2. **LLM population for the residue (Tier 2).** The model is handed
   the prompt *plus* the Tier-0 candidates already found, and asked to
   complete and relate them into `CodeUnit` candidates and edges — not
   to invent the whole structure cold. Every LLM-originated node/edge
   carries the same provenance discipline RFC 0013 already requires
   for an inferred link: tagged as such, never silently promoted to
   the same status as a human-confirmed one.
3. On completion, mode flips automatically, prompt → build, where the
   user's own correction pass (designed below) is the backstop for
   whatever the funnel still got wrong.

**The rejection ceiling gates the wrong thing if it's measured in
size.** A short, dense prompt describing forty interacting entities is
harder to populate correctly than a long, uniform one — the same
"heterogeneity, not length, determines difficulty" lesson this
project's own structure-discovery formulation already reached for
whole documents. The ceiling should trigger on what Tier 0's own probe
pass finds (candidate-entity count, relationship density), not raw
token/byte count. **Still open:** the concrete thresholds — see Open
Questions; this needs a benchmark, not a guess.

### 2. Build mode — the interactive Realm graph

The central new surface: a live, navigable **3D graph** over the
`CodeUnit`s the prompt-mode pass just populated — nodes are code
units, edges are their relationships. Purpose: let the user catch a
structural misunderstanding *before any `.nir` file exists*, when
correcting it is still cheap.

**Rendering surface — resolved, not open.** `hi` (the `nirdosha`
binary) spawns a native window via `tao`, with a `wry`-embedded
webview inside it — one process, one window, one app the user
perceives as `hi` itself, not a browser. `wry` uses whichever
rendering engine the OS already has installed (WebKit on macOS/Linux,
WebView2 on Windows) — no bundled Chromium, no separate browser
process the user ever sees or has to have running. That webview loads
a page served by a small local HTTP server (`tiny_http`, already a
workspace dependency — `crates/compiler/Cargo.toml`), which serves a
generated single-page app rendering the graph via `three.js`'s
`3d-force-graph` library against a small local JSON API backed
directly by `.nir/realm.db`. This mirrors a pattern this codebase
already uses, not a new one: `ui_gen.rs`/`codegen::build_serve`
already generate and locally serve a self-contained HTML/JS app for a
*compiled `.nir` program's own* UI (`nirdosha build --serve`); this is
the same technique, aimed at `hi`'s own console surface instead of a
compiled program's generated screens.

**Binding constraint, decided now rather than left implicit.** The
local server binds `127.0.0.1` only, on an OS-assigned ephemeral port,
with no CORS headers — never reachable from outside the machine, never
a fixed or guessable port. Easy to get right at this stage, exactly
the kind of thing that gets forgotten later because it feels like
plumbing rather than a real decision.

**Concurrency — still genuinely open, not resolved by the schema being
shared.** "One graph, two UIs" (below) settles that there's a single
source of truth; it doesn't settle what happens when the `ratatui`/
plain console, the webview's local API, and a `nirdosha realm` CLI
invocation all hold a connection to `.nir/realm.db` at once. SQLite's
WAL mode (available in the bundled build already in use) gets
concurrent readers and a single writer without blocking each other,
but doesn't answer the real question: who wins when the user edits a
node's text in the webview at the same moment a `realm sync` is
running from a shell. Not designed — see Open Questions.

**Session continuity across the `ratatui` ↔ webview swap.** Entering
build mode closes the terminal UI — but *which surface wins* was never
the whole question; what survives the swap is the part that matters
for the user. The console's transcript, scrollback, and mode memory
belong to the running `hi` process, not to `hi_tui.rs`'s own render
loop — kept in memory across the swap, not re-derived, so a `:ask`
result from two minutes before the transition is still there when
control returns to the base console. **Closing the webview window
mid-excursion is treated as backing out** — the same exit edge as
finishing publish: control returns to the base console immediately;
any CodeUnit that had already locked (see "Generate mode"'s own
definition below) stays locked and persisted exactly as it was;
anything still in flight is simply left un-generated, never corrupted
and never silently resumed — re-entering build mode later shows the
graph exactly as last locked, and the user picks up from there.

Per-node interactions:

- **Click a node → surface the text that produced it**, not its
  code — the requirement/prompt fragment behind it, exposing
  provenance. The code layer is intentionally invisible here.
- **Edit that text inline → a new version of the CodeUnit's driving
  text.** The underlying `.nir` is re-derived as a consequence, but
  invisibly — the user only ever interacts with the text layer in
  build mode, never the generated code directly.
- **Attach attributes — only the ones legal for that node's own
  kind**, drawn from the language's real annotation grammar, not a
  fixed universal panel:
  - `fn`: `requires(role/claim: ...)`, `effect(...)`,
    `nfr(latency_ms:/error_rate_max:/throughput_min_per_sec:/concurrency_max:)`,
    `validate { pre: post: }`.
  - a `struct` field: `requires(role/claim: ...)` field masking (what
    this conversation called "rolesview" — masks a field's data from a
    viewer lacking the required role/claim).
  - a `screen` field: `view`/`edit` gating by role/claim.
- **Attach gates — acceptance-type or exit-type, with a real
  operational difference, not just a label.** **Acceptance** means
  verifiable against a real target: it reduces to a `validate post:`
  when the target CodeUnit is Tier-1-provable (per RFC 0013's own
  gating discussion), or otherwise stays a documented, unenforced
  intent — the same status RFC 0013 already gives acceptance criteria
  with no compiled target. **Exit** means human-acknowledged only:
  there is no compiled-path target for it at all, by design, and
  attaching one never implies the compiler will check it. Shipping the
  distinction without this definition would invite a user to attach an
  "exit" gate and expect enforcement nothing in the compiler ever
  promised — the definition is load-bearing, not decorative.
- **Semantic search over the graph** — a query highlights every
  CodeUnit conceptually associated with it (not literal keyword
  matching) as a visual state of the same 3D view, not a separate
  results screen.

Edits/attachments the user makes directly in build mode (as opposed to
the LLM's own prompt-mode population) are appended onto the graph as
audit-trail entries, distinct from LLM-originated content.

**"13 as the base" means the data model, not just the entry point.**
RFC 0014 introduces no new or parallel data structure. The webview's
local JSON API is a pure read/write projection over the exact same
`.nir/realm.db` schema RFC 0013 already defines — the same `nodes`/
`edges`/`provenance` tables, the same `CodeUnit`/`Requirement` identity
and `content_hash`/`possibly_stale` semantics. The `ratatui` console
and the webview are two UIs over one graph, not one graph each — a
`:impact` run from the base console and a click in the 3D view reflect
identical underlying state, always.

### 3. Generate mode

Materializes the actual `.nir` source files from everything
accumulated across build mode — text, attached attributes, attached
gates, per CodeUnit — using the same bounded, diagnostic-feedback
self-repair discipline RFC 0012's `generate_and_build` already has for
whole-program generation, scaled to one unit at a time. (The concrete
per-unit retry/composition behavior — what happens when a unit that
compiles alone doesn't compile in composition with its neighbors — is
still open; see Open Questions.)

**Lock, defined precisely: a CodeUnit locks when its generated `.nir`
both exists and builds successfully** — typechecks and compiles, not
merely "text was materialized." A unit that fails to build after its
bounded retry budget stays unlocked, visibly, in the 3D view — the
same "flag, never silently overwrite" rule "What's authoritative,
when" states above, applied at this step specifically. This also
settles publish's own completion gating (below): publish only ever
considers locked units ready; a unit left unlocked is excluded from
that publish, not a blocker for the rest.

### 4. Publish mode

Gated on generate completing — meaning every CodeUnit the user wants
published has locked, not that every CodeUnit in the graph has (a unit
deliberately left unlocked, or out of scope for this publish, doesn't
block the rest — see "Generate mode"'s own lock definition). Reads a
`deployment_provider=""` config value and deploys the generated binary
in the shape that provider expects — Render named as one example,
presumably others follow the same pattern through some provider
abstraction (still undesigned — see Open Questions).

**Publish is best-effort; failure lands the user back in build mode,
never mid-air.** A provider unreachable, a partial/failed deploy, or a
rejected push all report the failure and return control to build mode
with nothing silently lost — the graph and every already-locked unit's
`.nir` are untouched by a failed publish attempt, so retrying costs
nothing already done. Rollback (undoing a *successful* but unwanted
deploy) is a provider-specific concern this RFC doesn't take a
position on.

## Effect on the permission model

None beyond what RFC 0012 (host-tooling, untouched) and RFC 0013
(reads `requires`/screen annotations as graph data, never
reinterprets enforcement) already established. Build mode's attribute
editor *authors* `requires(role/claim: ...)`/screen-masking
annotations through a UI instead of by hand, but the compiled
program's actual enforcement — `serve.rs`, codegen'd role/claim
checks — is exactly what it already is; this RFC changes how those
annotations get written, not what they do once compiled.

## Compatibility

Additive over RFC 0012's existing console and RFC 0013's existing
schema/CLI — the plain-text `Command::Request`/`:ask`/`:impact` path
is untouched; prompt/build/generate/publish is a new, parallel
interaction surface over the same LLM client, Realm graph, and
compiler pipeline. One honest caveat: unlike 0013's CLI-verb additions
(genuinely small diffs), this RFC implies a materially larger surface
— a new rendering technology (§ above), not just new console verbs —
so "additive" here means "doesn't change existing behavior," not
"cheap to build alongside it."

## Rejected alternatives

- **Folding this into the existing flat-transcript console instead of
  a dedicated graph surface.** Considered and rejected: the
  visibility/correction/attribute-attachment/deploy pipeline described
  here has no natural home in a scrolling text transcript — it needs a
  spatial, clickable surface the existing plain and `ratatui` front
  ends can't provide.
- **Opening the system's default browser at a local URL, instead of an
  embedded webview.** Genuinely simpler (zero new dependency — just
  shell out to `open`/`xdg-open`/`start`), and was the first cut
  considered. Rejected: it breaks the illusion of one integrated app —
  the user sees a browser tab with a URL bar, not `hi`. `wry`/`tao`
  costs one real dependency pair but keeps the window, the chrome, and
  the perceived identity of the app as `hi`'s own, not a side effect
  of it.
- **A fully native Rust 3D surface (`egui`+`wgpu`, or `bevy`), with no
  embedded web-rendering engine anywhere in the process.** Not
  rejected outright — a legitimate alternative if "zero web tech in
  the binary" is ever a hard constraint (licensing, security posture)
  — but not the default recommendation: it forgoes `three.js`'s
  `3d-force-graph` library, which already solves force-directed
  layout, click-picking, and camera controls; the native path means
  building that interaction model from scratch on raw `wgpu`, real,
  open-ended engineering `wry`/`tao` avoids for the cost of one
  embedded, OS-native webview the user never actually perceives as a
  browser.

## Open questions

Each item names what would actually resolve it — a design session, an
implementation spike, a benchmark — not just that it's open.

1. ~~What's authoritative, when.~~ **Resolved** — see the Design
   section by that name, placed directly after the state machine
   overview. This is the load-bearing question every other mode's
   behavior depends on, answered once rather than left implicit across
   "two generation strategies to reconcile" and "invisible per-unit
   regeneration" separately.
2. ~~Prompt mode bypassing the cheap → expensive funnel.~~
   **Resolved** — see "1. Prompt mode"'s own design: deterministic
   candidate extraction first, LLM only for the residue, the same
   funnel every other Realm feature already uses. **Still open under
   that:** the concrete rejection-ceiling thresholds (candidate-entity
   count, relationship density) — *resolves via a benchmark*, not a
   design session; there's no principled number without real prompts
   to measure against.
3. ~~"3D graph in a terminal" feasibility / `ratatui` coexistence /
   which mode starts where.~~ **Resolved** — `wry`+`tao` embedding
   `three.js`/`3d-force-graph` over a local `tiny_http` server,
   replacing (not running alongside) `ratatui`, with RFC 0013's
   console as the base entry point, and session state (transcript,
   scrollback) surviving the swap — see "Build mode"'s "Session
   continuity" note, including the mid-excursion window-close exit
   edge. **Still open:** the local JSON API's full query/mutation
   shape beyond what's already in use (`realm::ask`/`realm::impact`),
   and live-update propagation while the graph is open — *resolves via
   an implementation spike*, this is faster to prototype than to keep
   speculating about.
4. **API concurrency** — three potential writers (the base console,
   the webview's local API, `nirdosha realm` CLI) on one
   `.nir/realm.db`. WAL mode (see "Build mode"'s own concurrency note)
   gets non-blocking concurrent reads; conflicting concurrent writes
   to the same node still need an actual policy (last-writer-wins?
   optimistic concurrency with a version check and a surfaced
   conflict?) — *resolves via a design session*, a real decision with
   real tradeoffs, not something an implementation spike alone
   settles.
5. **Attribute-legality-per-kind** (per-CodeUnit-kind checking of
   which `requires`/`effect`/`nfr`/`validate`/screen-masking
   attributes are legal to attach) is a real, checkable constraint —
   RFC 0013 already established the specific version of this for
   NFR/Hoare. Extending that check to the full attribute surface for
   the UI to gate on is real, unbuilt work — *resolves via
   implementation*, the design (check the target's real AST) is
   already settled by RFC 0013's own precedent.
6. ~~Gate semantics.~~ **Resolved** — see "Build mode"'s gates bullet:
   acceptance means verifiable (a `validate post:` when
   Tier-1-provable, otherwise documented-only, same status RFC 0013
   already gives unenforceable acceptance criteria); exit means
   human-acknowledged only, with no compiled-path target, ever, by
   design — not merely "not yet built."
7. **`deployment_provider=""`** — no design for a provider
   abstraction, how many providers, or where this config actually
   lives (`nirdosha.toml` is a guess, not confirmed against real
   project-config convention) — *resolves via a design session*, a
   real decision about the abstraction's shape is needed before any
   provider-specific code is worth writing.
8. **Semantic search is a config option, not an architecture fork.**
   It doesn't have to reopen RFC 0013's deliberately-deferred
   vector-search question as a hard requirement: FTS5/`bm25`
   lexical candidates (already built) handle the majority case; a
   pluggable embedding backend covers the semantic residue as the same
   kind of optional accelerator RFC 0013 already scoped vector search
   to be, never an architectural prerequisite. **Still open:** which
   embedding backend (if any) actually ships, and when the residue is
   worth the cost — *resolves via a design session*, but a much
   smaller one than "do we own an embedding layer at all."
9. ~~Completion/publish gating.~~ **Resolved** — see "Generate mode"'s
   lock definition and "Publish mode"'s own text: publish only
   considers locked (built-successfully) units ready; an unlocked one
   is excluded from that publish, not a blocker for the rest. Pinning
   down the lock symbol's precise meaning was the prerequisite this
   question actually depended on.
10. **This RFC inherits `CodeUnit`'s known drawbacks, and an
    interactive editing surface makes them sharper than a flagged-text
    problem — worth naming concretely, not just pointing at the
    memory that first logged them:**
    - **Name collision becomes a data-corruption vector, not a display
      bug.** Two files each declaring `fn transfer_funds` collide on
      the identical `code:fn:transfer_funds` node today (RFC 0013's
      own open gap). In a flat `:impact` list, that's a wrong line of
      text. In build mode, it's the user editing "the" node's driving
      text and the write landing against whichever file's declaration
      the id happened to resolve to last — silently editing the wrong
      file's function, not just displaying it wrong.
    - **Whole-declaration granularity invites a gate that lies about
      its own precision.** A 400-line `fn` is one `CodeUnit`, one 3D
      node. Attaching a `validate post:` to that node reads as "this
      function is contract-checked" — but Tier-1 provability (RFC
      0013's own scope limit) almost certainly fails the moment that
      function is 400 lines with real I/O in it, so the gate the UI
      just made easy to attach is exactly the kind that silently
      degrades to documentation-only, per RFC 0013's own disclosed
      ceiling — and an interactive UI, by making attachment easy,
      makes it easier to believe the gate did something it didn't.

    Neither is new — both were named against the flat-text console
    already — but both get worse, not the same, once the same model
    backs a clickable, editable graph. *Resolves via the same fixes
    already catalogued for the underlying model* (rename/move
    detection for the first, sub-function granularity work for the
    second — both real redesigns, not quick fixes, per the
    `nirdosha_realm_v2_design_backlog` project memory) — this RFC
    doesn't re-derive those fixes, it names why they matter more here
    than they did before.
