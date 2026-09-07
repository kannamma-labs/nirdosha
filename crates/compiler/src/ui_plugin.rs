//! Compile-time UI-catalog extension (rfcs/0009-ui-catalog-
//! extensibility.md Phase B): a Rust crate contributes one additional
//! `layout { ... }` widget kind -- past the std closed set
//! (`"divider" | "card" | "timeline"`, `typeck.rs::check_screen_layout`)
//! -- by handing `typeck::typecheck_optional_main_with_ui_components`/
//! `ui_gen::generate_with_ui_components` a `&[NativeUiComponent]` slice.
//!
//! **Deliberately narrow, mirroring `plugin.rs`'s own "prove the
//! mechanism before automating discovery" posture** (rfcs/0008's Status
//! box): this is Phase B's *mechanism* only. Cargo-metadata-driven
//! discovery (a project's own `Cargo.toml` tagging a dependency `kind =
//! "nir-ui-component"`, `nirdosha build`/`emit-ui` finding, building, and
//! linking it automatically) is explicitly **not** built here --
//! rfcs/0008 Phase 3 (the identical discovery problem for native
//! builtins) isn't built either, and rfcs/0009's own "Open questions"
//! section says the two should share one discovery pass rather than
//! each inventing its own. Until that lands, a `NativeUiComponent` slice
//! is hand-assembled in Rust source, the same posture
//! `crates/compiler/tests/native_plugin_codegen.rs` already has toward
//! `NativePluginBuiltin` (rfcs/0008 Phase 1, also proven before its own
//! Phase 3 discovery exists).
//!
//! **What a component is not, yet:** it never contributes an `Element`/
//! `chart`/form-control type, a catalog action, or a GBNF fragment --
//! only a `layout` widget kind, the one existing extension point
//! `ast::LayoutNode::Widget`'s own doc comment already reserved for
//! "every future render-vocabulary widget." Widening past widgets (a
//! full catalog-schema entry with typed `props`, per rfcs/0009's
//! original sketch) is real, disclosed future work, not silently
//! narrowed away.
//!
//! **Trust boundary**: `render_js` is developer-authored JS, supplied
//! only by whoever calls `generate_with_ui_components` in Rust source --
//! never generated, never model- or request-supplied, spliced into the
//! emitted single-file template verbatim at `emit-ui` build time. Same
//! trust level as any other Cargo dependency an app author chose to add
//! (rfcs/0009 §"Effect on the permission model").

/// One plugin-contributed `layout { ... }` widget kind.
#[derive(Debug, Clone)]
pub struct NativeUiComponent {
    /// The bare identifier a `.nir` program writes as a layout leaf,
    /// e.g. `sparkline { field: "amount" }` -- must not collide with a
    /// std kind (`"divider"`/`"card"`/`"timeline"`); `typecheck_
    /// optional_main_with_ui_components` doesn't itself detect a
    /// collision between two *linked* components sharing a name (the
    /// caller assembling the slice owns that), only between a component
    /// and the std set.
    pub name: String,
    /// The JS spliced verbatim into the emitted `<script>` block -- must
    /// declare a plain top-level function under the exact name
    /// `render_fn` below names, e.g. `function render_sparkline(node) {
    /// ... return /* a DOM node */; }`. `node` is the same `{type:
    /// "widget", kind, source, title, entries}` shape `ui_gen.rs::
    /// layout_json`'s `LayoutNode::Widget` arm already emits for every
    /// widget -- `node.entries.<key>` reaches whatever config keys the
    /// `.nir` program's own `layout { <name> { key: value } }` block
    /// declared.
    pub render_js: &'static str,
    /// The exact function name `render_js` declares -- kept separate
    /// from `name` (rather than derived from it) so a component's JS
    /// identifier isn't forced to match its (possibly non-identifier-
    /// shaped) `.nir`-facing name.
    pub render_fn: &'static str,
}
