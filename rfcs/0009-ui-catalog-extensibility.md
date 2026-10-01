# RFC 0009: Grammar-of-graphics charts and compile-time UI-plugin components — escaping the closed 4-chart/7-control catalog without opening a runtime hole

> **Status.** Phase 0, Phase A, and Phase B — including Cargo-metadata
> auto-discovery for UI components specifically (not rfcs/0008 Phase
> 3's native-builtin discovery, a genuinely different, harder problem —
> see the discovery bullet below) — are built and verified, on
> `phase0/ui-catalog-extensibility`, not a proposal — same "update only
> this box, keep the original design capture below as written" style
> `rfcs/0008`'s own Status box uses.
>
> - **Phase 0**: `crates/compiler/catalog/std/0.1.json` documents the
>   real manifest wire shape (layout/controls/action/charts/theme
>   tokens), and `nirdosha emit-catalog [-o out.json]` (`main.rs`) prints
>   it. `crates/compiler/tests/emit_catalog.rs` (3 tests) is the proof.
> - **Phase A**: shipped **additively**, not as the "old four keywords
>   become sugar for the new one" desugaring this document originally
>   proposed below (a smaller, lower-risk cut — see the callout after
>   §Phase A). `render: "chart"` is a fifth `visual { ... }` kind,
>   configured by `mark: "bar"|"line"|"area"|"point"|"arc"|"rule"` plus
>   `encode <channel> { field: "..." type: "..." aggregate: "..." }`
>   blocks (`x`/`y`/`color`/`size`/`theta`) — folded into the existing
>   flat `Vec<KvEntry>` via the same prefix convention `paginate { }`
>   already uses (`"encode.<channel>.<key>"`), **not** the
>   `BTreeMap<String, EncodingChannel>` Rust struct sketched below.
>   `field` is checked to be a string only — never cross-checked against
>   a real column, since the backing fn returns opaque `json` with no
>   struct to resolve against (this document's own §Design anticipated
>   resolving against "the bound struct's real fields," which turned out
>   not to apply to the `visual`/`dashboard`/`panel` path at all). One
>   generalized `renderGraphicsChart` in `ui_gen_template.html` replaces
>   four hand-built functions' worth of *new* surface area (the four old
>   ones are untouched). **Also wired into workspace `panel { ... }`**,
>   a follow-up landed after this box's first version — same grammar,
>   same typeck (`check_encode_entry` generalized from a `visual`-only
>   helper to a shared one), same client renderer, just threaded through
>   `Panel`/`PanelRender` instead of `Metric`/`MetricRender`. 19 typeck
>   tests (`tests/visual_dsl.rs`) + 3 manifest tests (`tests/emit_ui.rs`)
>   + the real shipped renderer extracted and run headlessly in Chrome
>   against sample data for all six marks (screenshotted).
> - **Phase B**: shipped narrower than sketched below — a
>   `ui_plugin::NativeUiComponent { name, render_js, render_fn }`
>   contributes one additional `layout { ... }` **widget kind** (the one
>   extension point `ast::LayoutNode::Widget`'s own doc comment already
>   reserved for this), not a full catalog-schema entry with typed
>   `props`/`slots`/a `gbnf_fragment` the way the original sketch's
>   `NativeUiComponent` struct below has them.
>   `typeck::Checker.extra_widget_kinds` widens `check_screen_layout`'s
>   closed set; `ui_gen::generate_with_ui_components` splices `render_js`
>   into the emitted `<script>` and registers it into a new
>   `WIDGET_RENDERERS` map, read by `renderLayoutNode`'s widget-dispatch
>   fallback. `layout_json`'s `Widget` arm also gained a generic
>   `entries` object (every kv-entry, not just the `source`/`title` the
>   three std kinds special-case) so a component's own config keys reach
>   its `render_js`. Proven three ways, each a real follow-up: hand-
>   assembled inline (`crates/compiler/tests/ui_plugin.rs`); against a
>   real, separate, zero-`nirdosha`-dependency crate consumed via an
>   ordinary `[dev-dependencies]` edge (`crates/ui-plugin-example-
>   sparkline` + `tests/ui_plugin_examples.rs` — closing this RFC's own
>   "proof obligation" line literally, the way `plugin-example-native-
>   shout`/`-native-kv` already do for `rfcs/0008`); and **discovered
>   automatically** — see the next bullet. The reference crate's own
>   widget (`sparkline { source: <fn> field: "..." }`) is a genuinely
>   useful one, not a stub — it fetches its own data via `node.source`
>   (the `timeline` widget's own convention) and draws a small
>   inline-SVG trend line.
> - **Cargo-metadata auto-discovery — done for UI components, still open
>   for native builtins.** `ui_plugin::discover_components(manifest_path)`
>   runs a real `cargo metadata --format-version 1` against an app
>   author's own `Cargo.toml`, resolves the full dependency graph, and
>   picks out every package declaring `[package.metadata.nirdosha] kind
>   = "nir-ui-component"` (+ `name`/`render_fn`/`render_js_path`,
>   `render_js` read from that file). `nirdosha emit-ui` gained
>   `--manifest-path <Cargo.toml>`, or auto-detects one sitting next to
>   the input `.nir` file — absent either, zero behavior change, nothing
>   even shells out. This closes rfcs/0009's own "Open questions" item
>   **for UI components specifically** — it does **not** close
>   `rfcs/0008` Phase 3 (native-builtin discovery), because the two are
>   genuinely different problems, not the same problem solved twice: a
>   UI component crosses no ABI boundary (its whole "artifact" is three
>   plain strings, one a path to JS source sitting in the crate's own
>   tree — nothing to compile, nothing to link, no need for rfcs/0008
>   Phase 2's proposed authoring macro), while a native builtin's real
>   compiled `.a` bytes still have to be built and linked the harder way
>   Phase 3 was always going to need. This document's own §Design
>   originally assumed one shared discovery pass across both; that
>   assumption didn't survive contact with how much simpler the
>   UI-component case turned out to be — see `ui_plugin.rs`'s own doc
>   comment for the full "why not shared" reasoning. 3 new end-to-end
>   tests (`tests/ui_plugin_discovery.rs`) spawn the real compiled
>   `nirdosha` binary against a scratch project whose own `Cargo.toml`
>   path-depends on the reference crate — no Rust code written for the
>   occasion, no compiler-side dev-dependency — proving auto-detection,
>   the explicit flag, and the untouched no-manifest default all work.
> - **Still open, named not hidden**: Cargo-driven discovery for native
>   builtins (`rfcs/0008` Phase 2/3 — its own, harder problem, not
>   solved here); `NativeUiComponent` widening past a widget kind to a
>   full props/slots/GBNF-carrying catalog entry; GBNF export for either
>   phase (`crates/grammar_export`'s `nirdosha.gbnf` covers the core
>   `.nir` language only — no UI-specific grammar exists yet to extend);
>   `render_js` review-policy documentation (a discovered component's JS
>   is read from whatever crate the app author's `Cargo.toml` names —
>   exactly as reviewed as any other dependency, which is to say: only
>   as reviewed as the app author actually reviews their dependencies,
>   same as always). `cargo test -p nirdosha` stayed green (36 test
>   binaries) throughout every step above — no existing behavior
>   changed. (Unrelated, pre-existing, not investigated here: `cargo
>   build --workspace` fails on `crates/grammar_check` with a
>   shift-reduce conflict in its hand-maintained LALR(1) grammar —
>   confirmed unrelated to anything in this RFC, since nothing here
>   touches the core `.nir` grammar/tokenizer at all; every build/test
>   claim above is `-p nirdosha`-scoped specifically to avoid it.)
>
> The rest of this document is the original design capture, kept as
> written below; this box is the only part updated after the fact.
>
> **A second, independent track was appended 2026-09-16: see "Track C —
> the v2 Rust dialect's own UI engine" at the end of this document.**
> Everything above (Phase 0/A/B, the catalog, `ui_gen.rs`, GBNF) targets
> the native `.nil` compiler (`crates/compiler`). Track C targets the
> separate v2 Rust-dialect pipeline (`nirdosha-rt`/`nirdosha-macros`/
> `cargo-nirdosha`) built out this session, and shares no code, no
> catalog, and no trust model with Phase 0/A/B — it's recorded in this
> same document because it answers the same underlying question
> ("how does this ecosystem's UI vocabulary grow without a fork or a
> hidden runtime hole") for a genuinely different compiler, not because
> the two tracks compose.

> **Provenance note.** This RFC responds to an external draft spec
> ("NUIM" — a generative-UI manifest format synthesizing OpenUI Lang,
> Vercel's json-render, and Google's A2UI) proposed for adoption into
> `emit-ui`. That review concluded NUIM's live half — a flat
> `elements`-map manifest, JSON-Pointer bindings, JSONL patch streaming,
> server-round-tripped queries/mutations/RBAC enforcement — depends on a
> compiled `serve` that barely exists on this branch (GET-only, no
> mutations, no RBAC enforcement; `serve.rs` itself was deleted with the
> interpreter and Track B8 is `[PARTIAL]` per `docs/ROADMAP.md`), and
> that adopting it wholesale is a distinct product bet (agent-authored,
> chat-streamed UI) layered on top of a still-unfinished core migration.
> This RFC deliberately extracts the one piece of that review that
> stands on its own and needs none of that infrastructure: today's
> catalog is a *closed, hardcoded* vocabulary (7 form controls, 4 chart
> types, fixed MD3 tokens — `crates/compiler/UI_DSL_TODO.md`), and a
> real product gap exists independent of NUIM — an app author cannot
> get a chart shape `ui_gen.rs` didn't anticipate, or a component type
> nirdosha's own maintainers didn't hand-write, without forking the
> compiler. Both tracks below plug into the *current* nested
> `Screen`/`Panel`/`FieldSpec` manifest (`ui_gen.rs`) and the current
> `screen`/`dashboard` DSL — nothing here requires the flat-map rewrite
> or live protocol NUIM would need, and both remain valid if NUIM (or
> something like it) is adopted later.

## Motivation

`ui_gen.rs`'s catalog is closed by construction: `MetricRender`/
`PanelRender`-style enums enumerate exactly the chart/control shapes
nirdosha's own maintainers have hand-written a renderer for
(`bar_chart`, `graph`, `heatmap`, `timeline`; 7 form controls;
`crates/compiler/UI_DSL_TODO.md`'s own tracker). This closed vocabulary
is deliberate and is the right default — it's what makes the catalog
GBNF-enumerable and the renderer auditable — but it currently has no
extension mechanism at all. Two concrete gaps follow from that:

1. **Expressiveness is capped at four fixed chart shapes.** A scatter
   plot, a funnel, a gauge, a calendar heatmap, or any chart that isn't
   one of the four hand-built renderers in `ui_gen_template.html`
   requires a nirdosha core change, not an app-level one.
2. **No app or third party can add a genuinely new component** (a
   domain-specific widget, a richer media type) without forking
   `ui_gen.rs`/`ui_gen_template.html` directly — there is no equivalent,
   for UI components, of what `rfcs/0008` just built for native
   builtins.

Both gaps have the same shape as the problem `rfcs/0008` already solved
for `NativePluginBuiltin`: "the closed vocabulary is right, but closed-
forever-at-the-language-level is not." `rfcs/0008`'s answer —
Cargo-metadata-driven, `build.rs`-emitted manifests, resolved and
statically linked at `nirdosha build` time, nothing loaded or
interpreted at the target program's runtime — generalizes directly to
UI components and is reused here rather than re-designed from zero.

## Design

Two independent, additive tracks. Track A needs no new infrastructure
(it's a new element type inside the existing manifest); Track B reuses
`rfcs/0008`'s discovery mechanism, generalized to a second kind of
Cargo-metadata-tagged crate.

### Phase 0 — Make "the catalog" a real artifact (prerequisite for both tracks)

Today the catalog exists only as Rust match arms across `ui_gen.rs`
and `ui_gen_template.html`; neither typeck nor `crates/grammar_export`
can inspect it as data. Before either track lands:

- Extract the current component/prop vocabulary into
  `catalog/std/0.1.json` (JSON Schema): the 7 form controls, the 4
  chart types (kept as sugar — see Phase A), layout primitives,
  MD3 tokens. This is documentation of what already ships, not a
  behavior change.
- Add `nirdosha emit-catalog`, a sibling to the existing
  `emit-grammar`/`emit-prompt` commands (`crates/compiler/src/main.rs`),
  that dumps the *resolved* catalog — std merged with whatever Phase B
  plugin components are linked into this build — as JSON. This is the
  single artifact both typeck's catalog-resolution pass and
  `crates/grammar_export`'s GBNF merge (Phase B, below) read from.

### Phase A — Grammar-of-graphics `chart` element

Replace `bar_chart`/`graph`/`heatmap`/`timeline` as four independently
hand-built renderers with one `chart` element whose spec is a bounded
mark × encoding composition (the same idea Vega-Lite/Observable Plot
use, cut down to a closed, GBNF-enumerable subset):

```rust
// ui_gen.rs — new, alongside the existing MetricRender/PanelRender enums
enum ChartMark { Bar, Line, Area, Point, Arc, Rule }

enum EncodingType { Quantitative, Nominal, Ordinal, Temporal }
enum Aggregate { Sum, Avg, Count, Min, Max }

struct EncodingChannel {
    field: String,           // must resolve against the bound struct's real fields
    kind: EncodingType,
    aggregate: Option<Aggregate>,
}

struct ChartSpec {
    mark: ChartMark,
    encoding: BTreeMap<String, EncodingChannel>, // keys: "x","y","color","size","theta"
}
```

- **Typeck**: `field` on each `EncodingChannel` resolves against the
  bound struct's declared fields — the exact same struct/field-
  resolution pass `screen`/`dashboard` targets already use
  (`typeck.rs`), not new machinery.
- **Renderer**: one generalized SVG-emitting function in
  `ui_gen_template.html` replaces the four existing hand-built ones,
  sharing one scale/axis computation across marks — net *less* new JS
  than the four separate renderers already are, while covering a much
  larger combinatorial space of chart shapes.
- **GBNF**: the chart production in `nirdosha.gbnf` (or a future
  `nuim.gbnf`, if NUIM is later adopted) enumerates the `mark`/
  `encoding` vocabulary in place of four fixed keywords — still fully
  closed, just parameterized instead of enumerated per-shape.
- **Compatibility shim**: `bar_chart`/`graph`/`heatmap`/`timeline`
  remain valid DSL keywords that desugar to a fixed `{mark, encoding}`
  pair at parse time — no existing `.nir` program's `screen`/
  `dashboard` block changes behavior.

> **What actually shipped differs here** (see the Status box): the
> "field resolves against the bound struct's real fields" assumption
> above doesn't hold for `dashboard { visual ... }` — its backing fn
> returns opaque `json`, not a typed struct, so `field` is checked to
> be a string and nothing more. And rather than *replacing* the four
> fixed keywords with a desugaring, `"chart"` shipped as a fifth,
> additive `render` value — smaller diff, zero risk to the existing
> four, at the cost of two parallel-looking mechanisms living side by
> side instead of one. Revisit unifying them if that duplication
> becomes a real maintenance cost.

### Phase B — Compile-time UI-plugin components (generalizes `rfcs/0008`)

A plugin crate contributes one or more components via a new manifest
shape, `NativeUiComponent`, parallel to `rfcs/0008`'s
`NativePluginBuiltin`:

```rust
struct NativeUiComponent {
    name: String,                    // catalog-visible component type
    props: Vec<(String, Ty)>,        // checked by typeck exactly like a struct's fields
    slots: Vec<String>,              // named child regions, if any
    gbnf_fragment: String,           // merged into the exported grammar, namespaced
    render_js: &'static str,         // spliced into ui_gen_template.html at build time
}
```

- **Discovery/authoring convention**: identical to `rfcs/0008` Phase
  2/3 — a plugin crate's own `Cargo.toml` carries
  `[package.metadata.nirdosha] kind = "nir-ui-component"`; its
  `build.rs` emits a TOML manifest (name/props/gbnf fragment) next to
  its compiled artifact, exactly as `rfcs/0008` Phase 2 already
  specifies for native builtins. `render_js` ships as a plain
  `include_str!`'d `.js` file in the crate — developer-authored,
  reviewed, static text, never generated or model-supplied.
- **`nirdosha build`/`emit-ui`**: discovers crates tagged
  `nir-ui-component` the same way `rfcs/0008` Phase 3 discovers
  `nir-native` builtin crates, reads each manifest, merges `props` into
  the resolved catalog (Phase 0's `emit-catalog` artifact), splices
  `render_js` into the single-file template, and merges
  `gbnf_fragment` into the exported grammar under a
  `component_<crate>_<name>` namespace to avoid rule collisions across
  plugins.
- **Typeck/allowlist**: an element referencing an unresolved `type`
  gets the same `UNKNOWN_COMPONENT` rejection std components get today
  — plugin components are first-class catalog entries, not a side
  channel with weaker checking.
- **Proof obligation**: land one real reference component crate (e.g.
  a gauge or sparkline) built, discovered, rendered into a real
  `emit-ui` HTML output, and screenshot-verified in a browser — the
  same bar `rfcs/0008` Phase 4 sets for `mysql` before calling the
  mechanism proven rather than merely designed.
- **Dependency on `rfcs/0008` Phase 3**: `rfcs/0008` itself is only
  Phase-1-complete (the native builtin ABI); Cargo-driven discovery
  (its own Phase 3) is still open. Phase B's discovery step should be
  built *as* (or immediately alongside) `rfcs/0008` Phase 3, sharing
  one `nirdosha build`-time crate-discovery pass that dispatches on
  `kind = "nir-native"` vs `kind = "nir-ui-component"`, rather than
  standing up a second, parallel discovery mechanism.

> **What actually shipped differs here too** (see the Status box):
> `NativeUiComponent` is `{ name, render_js, render_fn }` only — no
> `props`/`slots`/`gbnf_fragment` fields, and it contributes a `layout`
> **widget kind**, not a standalone catalog element type with its own
> props schema. No Cargo discovery landed (per the "dependency on
> `rfcs/0008` Phase 3" note above, correctly anticipated as blocking —
> that phase still doesn't exist), so "proof obligation" was met via a
> hand-assembled Rust slice + a real headless-Chrome render instead of a
> real `Cargo.toml`-tagged crate discovered by `nirdosha build`.

## Effect on the permission model

Neither track touches `requires(role/claim: ...)`, `acquire`, or a
`screen`'s view/edit gates directly — a `chart` element's `rbac` field
(if/when the catalog gains one) and a plugin component's props are
gated exactly like any other element's, via the same
`field_gates_for_fn`/`field_gates_for_struct` machinery
`crates/compiler/UI_DSL_TODO.md`'s RBAC section already documents.

The one genuine addition to the trust boundary is Phase B's
`render_js` splicing: it is developer-authored, build-time JS, linked
into the compiled artifact with the same trust level as any other
Cargo dependency the app author chose to add — never something an
agent, a served request, or a chat turn can introduce or modify. This
is the same posture `rfcs/0008`'s own "Effect on the permission model"
section takes toward native builtins ("an ordinary function call in
`sigs`... gated identically"): extension happens once, at
`cargo build` time, by a human reviewing a dependency, never at
serving time.

**Why this is stronger than the three competing generative-UI specs'
own catalog model, not merely equivalent to it:** OpenUI/json-render's
catalog is a Zod object the app's own runtime process can edit or
extend at any time; A2UI's is chosen via a live "capability
negotiation" handshake at session start. Both put the extension
decision inside the same process serving the agent, protected only by
app-author discipline. Here, the catalog — std plus every linked
plugin component — is fully resolved, typechecked, and GBNF-compiled
before the binary is ever run, into `catalog/*.json` (Phase 0) with a
first-class provenance record (crate name + version) for every entry.
There is no code path in this design — no admin API, no hot-reload, no
session negotiation — that adds or changes a component definition
after `nirdosha build` finishes. An auditor can answer "why does this
app's UI vocabulary include a gauge chart" by reading one versioned,
reviewed Cargo dependency, not by inspecting live server state.

## Compatibility

- **Phase 0**: pure documentation extraction (`catalog/std/0.1.json`)
  plus one new, additive CLI command (`emit-catalog`). No existing
  `.nir` program's compiled or rendered output changes.
- **Phase A**: `bar_chart`/`graph`/`heatmap`/`timeline` keep parsing
  and typechecking exactly as before, desugaring internally to
  `ChartSpec` — no existing `screen`/`dashboard` block's behavior
  changes. `chart(mark: ..., encoding: {...})` is new syntax at a spot
  nothing valid previously occupied.
- **Phase B**: fully additive, same posture `rfcs/0008`'s own
  Compatibility section claims for the builtin ABI — no plugin crate
  ships today, so no existing build's output changes; a project must
  explicitly add a `nir-ui-component`-tagged dependency to gain any new
  component type.

## Rejected alternatives

**A model-authored `custom_svg`/`iframe`/raw-markup escape hatch**,
instead of either track. Rejected because it is the one move that
actually breaks `ui_gen_template.html`'s existing "no HTML, no script,
no `eval`, declarative data only" invariant — the same invariant NUIM's
own §6 names as the property none of OpenUI/json-render/A2UI can
fully guarantee. Richer *primitives*, developer-curated and reviewed
before they ever reach a binary an agent talks to, get most of the
same UX gain without reopening that hole.

**Vendoring a full external charting library (Observable Plot,
Vega-Lite's own renderer) and exposing its entire spec surface**
instead of a hand-cut mark/encoding subset. Rejected for now as
solving a bigger problem than the evidence justifies, the same
judgment `rfcs/0008`'s "Open questions" makes about a generic
serialization-based FFI ABI: a bounded mark/encoding subset covers the
overwhelming majority of real dashboard/report charts nirdosha's
compliance-CRUD niche needs, without taking on an external library's
full expression language (which is generally Turing-complete or close
to it, and not GBNF-enumerable) or its dependency footprint inside a
"zero-dependency client" template.

**A bespoke nirdosha UI-component registry (a hosted service, its own
publish CLI)** instead of reusing Cargo/crates.io via
`[package.metadata.nirdosha]`. Rejected for the identical reason
`rfcs/0008`'s own "Rejected alternatives" section already gives for
builtins: hosting a registry is the expensive option for a
solo-maintainer project (`docs/ECOSYSTEM.md` §G5), and Cargo already
solves resolution, semver, and lockfiles — the only missing piece is
the metadata convention this RFC (and `rfcs/0008` Phase 3) define.

## Open questions

- **GBNF fragment merging at scale.** `crates/grammar_export`'s
  fidelity cross-check (the llama.cpp validator plus its own
  independent matcher, `rfcs/0008`'s own Status box notes this
  pipeline exists for the core grammar) needs to run against the
  *merged* grammar — std plus whatever plugin components a given build
  links — not just std in isolation. Namespacing rule names
  (`component_<crate>_<name>`) avoids collisions but hasn't been tried
  against more than one plugin at once. **Still fully open** — no
  UI-specific GBNF exists yet at all (Status box).
- **Should Phase B's discovery pass be a new code path or a literal
  extension of `rfcs/0008` Phase 3?** **Resolved, differently than this
  document originally assumed**: a new, separate code path
  (`ui_plugin::discover_components`), not an extension of Phase 3 —
  see the Status box's discovery bullet for why the two turned out to
  be genuinely different problems (no ABI boundary, no build/link step)
  rather than one problem worth solving once. `rfcs/0008` Phase 3
  remains its own, still-unbuilt, harder problem.
- **`render_js` review policy.** Nothing here mandates *how* an app
  author reviews a UI-plugin crate before depending on it beyond
  "it's an ordinary Cargo dependency" — worth a documented convention
  (e.g. requiring plugin crates to be workspace-vendored, or an
  explicit allowlist in the app's own `Cargo.toml`) before this ships,
  so "add a UI plugin" stays visibly a human decision, never something
  an agent can reach for unsupervised. **Still open.**
- **Interaction with NUIM, if adopted later.** This RFC assumes
  `ChartSpec` and `NativeUiComponent` entries slot into whatever
  catalog format eventually exists — including a NUIM-style flat
  `elements` map — as just more entries. That assumption is untested
  since NUIM Phase 0+ hasn't been decided on; if NUIM's catalog schema
  ends up incompatible in some way this document doesn't anticipate,
  this RFC's Phase 0 artifact (`catalog/std/0.1.json`) is the thing
  that would need reconciling, not Phase A/B's underlying mechanisms.
- **Widening `NativeUiComponent` past a widget kind** (typed `props`, a
  full element/chart/control type, a `gbnf_fragment`, crate-name+
  version provenance in `catalog/std/0.1.json`) — the shipped shape
  (Status box) proves the splice-and-dispatch mechanism works but is
  narrower than this document's original sketch; whether the wider
  shape is worth building is a separate decision from whether the
  mechanism itself is sound.

---

## Track C — the v2 Rust dialect's own UI engine

**Status: Phase 0 (dashboards) and the CRUD/List/Detail/Form/Delete
archetypes are built and proven** — `nirdosha_rt::dashboard!`,
`nirdosha_rt::crud_screens!`, `crates/nirdosha-rt/src/{dashboard,
screens}.rs`, worked example `examples/nirdosha-v2-corpus/src/
57_ui_engine_demo.nir` (CRUD + categorical actions + dashboard, all
attached to one real datasource, verified live via curl, a headless-
browser screenshot pass, and a real-socket golden test). `Router` also
gained a top nav bar (`with_nav`) and cookie-session login/logout
(`with_login`) — the Auth-screens row below is now partial, not
unstarted. Two real bugs were found live and fixed with regression
tests: `Router::serve` panicking the whole process on a client that
disconnected mid-response, and a literal-suffix route (`/products/new`)
being silently swallowed by the `/products/{id}` wildcard because of
registration order. Phases 2–5 (query-param filtering, per-widget
visibility, real-time push, client-side interactivity) remain as
originally designed below, not yet built. Everything here targets
`nirdosha-rt`/`nirdosha-macros`/`cargo-nirdosha` — the Rust dialect
whose role/policy/categorical-action/HTTP work is recorded in RFC 0020,
not `crates/compiler`'s native `.nil` grammar. Read alongside RFC 0020,
which this track depends on directly (`Router`, `RoleProof<R>`/
`Auth::prove`, the Road 1/Road 2 framing).

### Why a separate track, not a shared catalog

Phase 0/A/B's whole design centers on a **closed, GBNF-enumerable
vocabulary** resolved and typechecked at `nirdosha build` time against
a bound struct, because the native compiler's contract with an LLM
caller is "every valid completion is a valid program." The v2 dialect
has no GBNF-constrained generation story and no separate compiler
front-end to resolve a catalog against — its whole design principle,
established over this session and stated plainly in
`docs/nirdosha-rt-dialect.md`, is **no bespoke parser, ever**: every
guarantee must be real `rustc` — the type system, ordinary proc-macro
expansion, or `const`-evaluation — never a closed-vocabulary DSL
checked by a separate tool. So Track C does not reuse `ChartSpec`,
`NativeUiComponent`, `catalog/std/0.1.json`, or any GBNF fragment —
those are load-bearing for a fundamentally different trust model. What
*does* generalize is the underlying shape: dashboards are already
`stat_fn`/`chart_fn` references in this dialect's own corpus
(`nirdosha:dashboard` doc comments in `enterprise_app.nir`,
`40_dashboard.nir`), and Track A's mark/encode vocabulary
(`bar`/`line`/`area`/`point`/`arc`/`rule`) is worth borrowing as a
*convention*, not as shared code.

### Where dashboards sit in the larger UI-screen map

Dashboards are one of ten recurring screen archetypes in web
applications, each with its own dominant interaction pattern — worth
naming up front so "the UI engine" has a real destination, not just a
dashboard-shaped hole:

| Archetype | Primary verb | Interaction density | v2 dialect status |
|---|---|---|---|
| CRUD: List/Index | scan & select | high (filter, sort, search, paginate, batch actions) | **built** — `crud_screens!`'s `list_html`; no filter/sort/paginate yet (Phase 2) |
| CRUD: Detail/Show | read & tweak | medium (tabs, history, related records) | **built** — `crud_screens!`'s `detail_html`; no tabs/history |
| CRUD: Create/Edit Form | input & validate | medium-high (inline errors, autosave, steppers) | **built** — `crud_screens!`'s `form_html`, inline field-scoped errors on a failed submit; no autosave/steppers |
| CRUD: Delete/destructive | confirm & undo | low, high-stakes | **built** — `crud_screens!`'s typed "type DELETE to confirm" page; no undo-toast |
| Dashboard/Overview | monitor & drill | medium (date range, drill-down, cross-filter) | **built (Phase 0)** — `dashboard!`, `Metric`/`Chart` widgets; no drill-down yet (Phase 2) |
| Auth screens | credential & verify | low-medium, security-sensitive | **partial** — `Router::with_login` (cookie session, login form, logout) + `with_nav`'s Login/Logout button; no MFA/SSO/password-strength/CAPTCHA |
| Workflow/Wizard | progress linearly | low per step, gated | not started; `nirdosha:workflow` doc comments exist in the corpus (inert) but Track C would need its own, non-GBNF mechanism |
| Search & Discovery | find & narrow | high (autocomplete, facets) | not started |
| Board/Canvas | manipulate spatially | very high (drag, zoom, connect) | not started, likely out of scope for a "no JS pipeline" dialect (see Open Questions) |
| Settings/Configuration | configure safely | low-medium (danger zones, guards) | not started — a natural next slice: `crud_screens!` against a singleton datasource |
| Communication | react & respond | high, often real-time | **built** - `communication_feed!` with timer refresh or opt-in bounded long polling; concurrent Router workers (#66); SSE/WebSocket still pending |
| System state (404/empty/loading) | recover | low | **partial** — real 404 (`Response::not_found`), 403, and empty-list states exist; no styled error pages or loading skeletons |
| Report/Export | select & schedule | medium (format, columns, preview) | not started |
| Audit trail | review & attest | medium (who, when, before/after) | not started — natural generalization of `workflow_history` |
| Approval chain | decide & escalate | high (quorum, delegation, SLA) | not started — extends `workflow!` with quorum + delegation |
| Notification inbox | read & dismiss | medium (categories, read/unread) | not started — `on_entry` can already insert rows; no bell-icon convention |
| RBAC admin | assign & review | low-medium (roles, claims, mappings) | not started — needs claim primitive first |
| Scheduled job / scheduler | schedule & monitor | medium (cron, retries, log) | not started — no cron primitive; external scheduler contract |
| Search & faceted discovery | find & narrow | high (autocomplete, saved queries) | not started |
| Import / export job | upload & validate / select & download | medium-high | not started — blocked on file/blob type |

Read-heavy archetypes (List, Dashboard, Search) optimize for scanning
and filtering; write-heavy ones (Form, Wizard) optimize for validation
and preventing mistakes; spatial ones (Board/Canvas) optimize for
direct manipulation and are the ones most likely to need a client-side
JS layer this dialect doesn't have yet (see Phase 5). This table is a
map, not a commitment — it exists so "add screen type X" has an
obvious place to slot in later, not so every row gets built.

### Dashboards, phased

| Phase | Adds | New infra needed | Risk |
|---|---|---|---|
| 0 | `Metric` + `Chart` widgets, `refresh_seconds`, JSON+HTML dual output | None — reuses `Router` entirely | Low |
| 1 | `Trend` + `Table` widgets | SVG sparkline primitive, JSON→table column convention | Low |
| 2 | Query-param filtering (interactive drill-down) | Query-string parsing in `Request`, new macro syntax | Medium |
| 3 | Per-widget role visibility within one dashboard | Extends existing `get_gated::<R>`/masking pattern | Low-Medium |
| 4 | True real-time push (WebSocket/SSE) | A WS layer — zero WS support exists today | High |
| 5 | Client-side interactivity (cross-highlight, pivot, no reload) | This dialect's first JS artifact, ever | High, architectural |
| — | Self-service/embedded dashboard builder | A different product (a builder UI) | Out of scope |

**Phase 0 — foundation.** `nirdosha_rt::dashboard!`, same shape as
`categorical_actions!` (RFC 0020): takes widget declarations that
reference real Rust functions by name, so a wrong name or signature is
an ordinary `rustc` error, not a runtime surprise or a `cargo-nirdosha`
finding.

```rust
nirdosha_rt::dashboard! {
    title: "Sales Overview",
    refresh_seconds: 300,
    widgets {
        Metric { label: "Revenue MTD", fn: stat_revenue_mtd, target: 1_000_000, alert_below: true },
        Chart  { label: "By Region", fn: chart_revenue_by_region, mark: bar },
    }
}
```

Registers `GET /dashboard.json` (structured data, an OpenAPI entry for
free via the existing `Router`) and `GET /dashboard` (server-rendered
HTML: `Metric` as a colored number against its `target`, `Chart` as
inline SVG). `refresh_seconds` becomes a `<meta http-equiv="refresh">`
tag — covers both the "periodic" (strategic, weekly+) and
"near-real-time" (operational, minutes) update-frequency tiers
honestly, with zero JS. The macro also auto-emits the equivalent
`/// nirdosha:dashboard {...}` doc comment — Road 2, non-authoritative
documentation of what Road 1 (the generated routes) actually does,
same trick `#[contract(...)]` already plays for its own doc encoding.

Verification: a new worked example (e.g. `57_dashboard_ui.nir`), a
real-socket golden test against both routes (same convention as
`56_product_crud_api.nir`'s `product_crud_api_serves_real_requests`),
and property tests (`dashboard.json` lists every registered widget
exactly once; the SVG renderer never panics on arbitrary label/value
data — `proptest`, same posture as RFC 0020's router tests).

**Phase 1 — widget completeness.** `Trend` (a number plus an inline
sparkline, `fn() -> Vec<(String, f64)>`) and `Table` (`fn() -> Json`
rendered as an HTML table). Real decision to make explicitly rather
than leave implicit: column inference from the first JSON object's
keys, or an explicit `columns: [...]` list in the macro call. Given
this dialect's `#[serde(deny_unknown_fields)]`-everywhere posture
elsewhere (RFC 0020's `Contract`/`Crud`/`Policy` models all reject
rather than guess), explicit columns is the better default.

**Phase 2 — server-side interactivity.** `GET /dashboard.json?range=7d`
passes `range` through to a backing function
(`chart_revenue_by_range(range: &str)`). Needs `Request` to actually
parse its query string (today `path_without_query` in
`crates/nirdosha-rt/src/web.rs` discards it) and a new, optional
`filter:` key in the macro syntax. OpenAPI generation gains a
`parameters` entry per filterable widget for free once this lands.
Covers the "analytical" (explore, drill down) purpose without any
client-side code.

**Phase 3 — per-widget visibility.** A whole dashboard can already be
gated (`Router::get_gated::<R>`, reused directly) — the new piece is
one *ungated* dashboard where some widgets are still role-restricted
(e.g. an exact-revenue `Metric` visible only to `Admin`, a rounded one
visible to everyone). Mechanism: the same `Option<&RoleProof<R>>`
masking pattern RFC 0020 already proved for `cost_cents`, applied
per-widget instead of per-field.

**Phase 4 — true real-time (WebSocket/SSE).** The honest
seconds-latency "operational" tier (live ops centers, trading, IoT).
Nothing in `nirdosha-rt` speaks WebSocket or SSE today; this is a new
protocol layer on raw `TcpStream`, not a config flag — the same
"declare the gap, don't fake it" posture RFC 0020 takes toward
at-rest/in-transit encryption. Deserves its own RFC before being built,
given the blast radius (a persistent-connection primitive) is bigger
than dashboards alone, and would also be the mechanism Communication
screens (chat, live notifications) eventually need.

**Phase 5 — client-side interactivity.** Cross-highlighting, pivoting,
drill-down without a full page reload — and Board/Canvas screens
(drag-and-drop, zoom/pan) if this dialect ever takes those on. This is
the one that changes what kind of project the v2 dialect is, not just
what a dashboard does: every guarantee built this session has been
"real `rustc`, checked at compile time," and a JS runtime is a
different trust boundary entirely (no bundler exists today; even a
hand-written vanilla-JS asset is a new kind of artifact). Deserves a
real design conversation of its own before any code lands — not a
sub-bullet of a dashboard feature.

**Out of scope, explicitly:** a self-service dashboard *builder* (a
different product — a UI for building UIs — not an engine feature).

### Effect on the permission model (Track C)

No new permission primitive, same posture as Phase A/B's own section
takes for the native catalog. `Metric`/`Chart`/etc. gating is exactly
`Router::get_gated::<R>`/`Auth::prove::<R>()`, already proven
unforgeable; per-widget visibility (Phase 3) is exactly
`Option<&RoleProof<R>>` masking, already proven for field-level data in
RFC 0020. The one new *documentation* surface — the auto-emitted
`nirdosha:dashboard` doc comment — is Road 2 by construction: derived
from the same macro invocation as the real gates, never an independent
input, never consulted for authorization.

### Rejected alternatives (Track C)

**Reusing Phase 0/A/B's catalog/GBNF mechanism for the v2 dialect.**
Rejected: that machinery's entire value proposition is bounding what
an LLM can generate inside a closed, enumerable grammar resolved by a
separate compiler front-end. The v2 dialect has neither a GBNF story
nor a separate front-end to resolve against — grafting the catalog on
would mean building a parallel, unused enforcement path, not reusing
one.

**A `nirdosha:dashboard`-style doc-comment JSON schema, checked by an
opt-in `cargo-nirdosha` scanner extension**, mirroring what Track A/B
do for the native compiler. Rejected for the same reason RFC 0020
rejected this shape for `nirdosha:entity`/`nirdosha:policy`: doc
comments are inert to `rustc`, so the guarantee would only hold for
whoever remembers to run the scanner — a real regression from every
other mechanism in this dialect, all of which hold under plain
`cargo build` alone.

### Open questions (Track C)

- Whether Board/Canvas screens are in scope for this dialect at all, or
  permanently out of reach without Phase 5's JS layer — spatial
  direct-manipulation UIs are hard to imagine as server-rendered HTML
  no matter how the rest of Phase 5 shakes out.
- Whether Workflow/Wizard screens should generalize
  `categorical_actions!` (a wizard step is arguably a categorical
  "current step" field with role/validation-gated transitions) or need
  their own mechanism — not yet explored.
- Whether `Table`'s column convention (explicit `columns:` list,
  recommended above) should also support a typed row struct instead of
  raw `Json`, once a real use case asks for it.
- Which of the new archetypes (audit trail, approval chain,
  notification inbox, RBAC admin, scheduler, search, import/export)
  belong as standalone macros versus extensions of `crud_screens!`/
  `dashboard!` — most are macro-only once the underlying table/view
  convention is decided.
- Same "who reviews this" question Phase B's own Open Questions raises
  for `render_js`, deferred to Phase 5: once any client-side JS exists
  in this dialect, what review convention keeps "add a UI script" a
  visible human decision rather than something slipped in unreviewed.
