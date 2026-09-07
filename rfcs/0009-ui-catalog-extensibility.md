# RFC 0009: Grammar-of-graphics charts and compile-time UI-plugin components — escaping the closed 4-chart/7-control catalog without opening a runtime hole

> **Status.** Phase 0, Phase A, and Phase B's *mechanism* (not its
> Cargo-auto-discovery ambition) are built and verified, on
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
>   `NativeUiComponent` struct below has them. No Cargo-metadata
>   discovery, no `nirdosha build` auto-linking — a component slice is
>   still hand-assembled in Rust source (either inline,
>   `crates/compiler/tests/ui_plugin.rs`, or via an ordinary
>   `[dev-dependencies]` edge to a real separate crate,
>   `crates/ui-plugin-example-sparkline` + `tests/
>   ui_plugin_examples.rs` — a follow-up closing this RFC's own "proof
>   obligation" line literally, the way `plugin-example-native-shout`/
>   `-native-kv` already do for `rfcs/0008`), the same posture
>   `rfcs/0008`'s own `native_plugin_codegen.rs` has toward
>   `NativePluginBuiltin` ahead of its still-open Phase 3.
>   `typeck::Checker.extra_widget_kinds` widens `check_screen_layout`'s
>   closed set; `ui_gen::generate_with_ui_components` splices `render_js`
>   into the emitted `<script>` and registers it into a new
>   `WIDGET_RENDERERS` map, read by `renderLayoutNode`'s widget-dispatch
>   fallback. `layout_json`'s `Widget` arm also gained a generic
>   `entries` object (every kv-entry, not just the `source`/`title` the
>   three std kinds special-case) so a component's own config keys reach
>   its `render_js`. The reference crate's own widget
>   (`sparkline { source: <fn> field: "..." }`) is a genuinely useful
>   one, not a stub — it fetches its own data via `node.source` (the
>   `timeline` widget's own convention) and draws a small inline-SVG
>   trend line. 4 + 3 new tests (`tests/ui_plugin.rs`,
>   `tests/ui_plugin_examples.rs`) + both the hand-written and the real
>   crate's own `render.js` extracted and run headlessly in Chrome
>   (screenshotted).
> - **Still open, named not hidden**: Cargo-driven discovery for both
>   the chart grammar's own future extensions and UI components (shared
>   with `rfcs/0008` Phase 3, per this document's own §Open questions);
>   `NativeUiComponent` widening past a widget kind to a full
>   props/slots/GBNF-carrying catalog entry; GBNF export for either
>   phase (`crates/grammar_export`'s `nirdosha.gbnf` covers the core
>   `.nir` language only — no UI-specific grammar exists yet to extend);
>   `render_js` review-policy documentation. `cargo test -p nirdosha`
>   stayed green (35 test binaries) throughout every step above — no
>   existing behavior changed. (Unrelated, pre-existing, not investigated
>   here: `cargo build --workspace` fails on `crates/grammar_check` with
>   a shift-reduce conflict in its hand-maintained LALR(1) grammar —
>   confirmed unrelated to anything in this RFC, since nothing here
>   touches the core `.nir` grammar/tokenizer at all; every build/test
>   claim above is `-p nirdosha`-scoped specifically to avoid it.)
>
> The rest of this document is the original design capture, kept as
> written below; this box is the only part updated after the fact.

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
  extension of `rfcs/0008` Phase 3?** This document assumes the
  latter (one crate-discovery pass, two `kind` tags) but `rfcs/0008`
  Phase 3 itself isn't built yet — the two should land together or in
  direct sequence, not be designed twice independently. **Still open.**
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
