# RFC 0014: The generative build console — prompt → build → generate → publish, over an interactive Realm graph

> **Status: speculative design capture, nothing built.** This RFC
> transcribes a design conversation (2026-09-10) that deliberately
> worked "alternate reality" first — lay out the whole shape of the
> idea before checking any of it against what's actually feasible —
> and is written down at that stage on purpose, not after a feasibility
> pass. Treat this as a proposal to react to and cut down, not a
> committed roadmap. Section "Open questions" below is unusually long
> and unusually honest for that reason: several pieces here are
> genuinely unresolved research problems, not just unscheduled work,
> and this RFC says so explicitly rather than presenting them as solved
> details.

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
prompt --(LLM populates the Realm graph)--> build --(materialize .nir)--> generate --(deploy)--> publish
```

Entered automatically on `nirdosha hi` startup, after RFC 0013's
existing auto-scaffold (`hi::open_realm_or_warn`) — this RFC doesn't
change that step, it adds what happens after the console is up.

### 1. Prompt mode

The default mode on entry. A length/complexity ceiling gates every
prompt: over it, reject outright ("too huge to comprehend"), no
generation attempted at all — **exact ceiling and how it's measured
(tokens? bytes? a structural complexity heuristic?) is an open
question below, not specified here.**

Under the ceiling, the prompt goes to the LLM — but unlike RFC 0012's
`generate_and_build` (which treats the model's output as one opaque
`.nir` source blob, built or not), **the model's job here is to
populate the Realm graph directly**: candidate `CodeUnit`s and their
relationships, not a single source file. On completion, the mode
flips automatically, prompt → build.

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
- **Attach gates — acceptance-type or exit-type**, a category
  distinction adopted as stated: both are conceptually gates on a
  CodeUnit, and neither needs compiler backing to exist as one.
  Acceptance criteria can, sometimes, reduce to a `validate post:`
  (per RFC 0013's Tier-1 gating discussion); exit criteria has no
  compiled-path target at all today, and — per this conversation's own
  framing — that's fine, same status acceptance criteria already had
  before this RFC.
- **Semantic search over the graph** — a query highlights every
  CodeUnit conceptually associated with it (not literal keyword
  matching) as a visual state of the same 3D view, not a separate
  results screen.

Edits/attachments the user makes directly in build mode (as opposed to
the LLM's own prompt-mode population) are appended onto the graph as
audit-trail entries, distinct from LLM-originated content.

### 3. Generate mode

Materializes the actual `.nir` source files from everything
accumulated across build mode — text, attached attributes, attached
gates, per CodeUnit. As each CodeUnit's generation finishes, its node
in the 3D view gets a **lock** symbol, visually distinguishing
finalized units from ones still in flight.

### 4. Publish mode

Gated on generate completing. Reads a `deployment_provider=""` config
value and deploys the generated binary in the shape that provider
expects — Render named as one example, presumably others follow the
same pattern through some provider abstraction.

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
— a new rendering technology (§ below), not just new console verbs —
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

This section carries more real, unresolved weight than usual for this
repo's RFCs, on purpose — see the status note at the top.

1. **Prompt-length ceiling** — what triggers "too huge to comprehend,"
   and how is it measured (tokens/bytes/a complexity heuristic)? Not
   specified.
2. ~~"3D graph in a terminal" feasibility.~~ **Resolved** — see
   "Build mode"'s own "Rendering surface" note above: `wry`+`tao`
   embedding `three.js`/`3d-force-graph`, served locally via
   `tiny_http`, mirroring `ui_gen.rs`/`codegen::build_serve`'s existing
   pattern. What's still genuinely open under that decision: the local
   JSON API's own shape (what does `.nir/realm.db` need to expose for
   the graph to render/update live), and how `hi_tui.rs`'s existing
   `ratatui` console and this new webview-window coexist in the same
   process/session — does entering build mode replace the terminal UI,
   run alongside it, or hand off entirely? Not designed.
3. **Two different generation strategies need reconciling.** RFC
   0012's `generate_and_build` treats model output as one opaque
   `.nir` blob with a whole-program self-repair loop; this RFC's
   prompt mode has the LLM populate the graph directly, per-unit.
   Whether/how the self-repair discipline applies per-CodeUnit instead
   of per-program isn't designed.
4. **Invisible per-unit code regeneration** (build mode, editing a
   CodeUnit's text) needs the same bounded-attempt,
   diagnostic-feedback discipline RFC 0012 already has for
   whole-program generation, scaled to one unit at a time — not
   designed.
5. **Attribute-legality-per-kind** (§2 above) is a real, checkable
   constraint — RFC 0013 already established the specific version of
   this for NFR/Hoare (checking a target's real signature before
   proposing `error_rate_max`/a Tier-1 `validate`). Extending that
   check to the full `requires`/`effect`/screen-masking surface for
   the UI to gate on is real, unbuilt work.
6. **Gate semantics** — acceptance vs. exit criteria as two categories
   is adopted as stated, but what operationally distinguishes them
   (when is a gate "acceptance" vs. "exit"?) was left undefined in the
   conversation this RFC transcribes, and stays undefined here.
7. **`deployment_provider=""`** — no design for a provider
   abstraction, how many providers, or where this config actually
   lives (`nirdosha.toml` is a guess, not confirmed against real
   project-config convention).
8. **Semantic search needs a real embedding/vector layer.** RFC 0013
   deliberately scoped vector search as optional and deferred,
   FTS5-only for v1 — "an optional accelerator, never an architectural
   prerequisite." This RFC's search requirement reopens that as a hard
   requirement rather than an optional one, a real escalation past
   0013's stated v1 posture, not just an add-on.
9. **Completion gating** — whether *every* CodeUnit must lock before
   publish becomes available, or a partial/subset publish is allowed,
   isn't specified.
10. **This RFC inherits every open problem already logged against the
    underlying `CodeUnit`/Realm model** — its granularity and
    name-based-identity drawbacks compound once a whole 3D graph is
    built directly on that same node model (a rename-orphaned link is
    now a visual, not just a flagged-text, problem); and if prompt
    mode is ever handed something PRD-sized rather than a short
    request, it inherits the unsolved document-structure-discovery
    problem too. Not re-derived here — see the project memory
    (`nirdosha_realm_v2_design_backlog`) this RFC's own follow-up work
    should be read alongside.
