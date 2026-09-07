//! `rfcs/0009-ui-catalog-extensibility.md` Phase B's real reference UI
//! component -- proof that the compile-time widget-extension mechanism
//! (`nirdosha::ui_plugin::NativeUiComponent`, `typeck::
//! typecheck_optional_main_with_ui_components`, `ui_gen::
//! generate_with_ui_components`) works against a genuinely separate
//! crate, not just inline Rust in a test file the way
//! `crates/compiler/tests/ui_plugin.rs`'s own hand-assembled component
//! already proved the mechanism itself. See [`src/render.js`] for the
//! actual widget: a `layout { sparkline { source: <fn> field: "..." }
//! }` leaf, fetching its own data via `node.source` and drawing a small
//! inline-SVG trend line.
//!
//! **Deliberately zero dependency on `nirdosha`** (mirrors
//! `plugin-example-native-shout`/`plugin-example-native-kv`'s own
//! posture toward the compiled-builtin ABI): this crate hands over
//! plain data, not a Rust type living in the compiler crate. A consumer
//! builds its own `ui_plugin::NativeUiComponent { name: NAME.into(),
//! render_js: RENDER_JS, render_fn: RENDER_FN }`.
//!
//! **No Cargo-driven auto-discovery yet** (rfcs/0009's own "Open
//! questions," shared with rfcs/0008 Phase 3): `crates/compiler/tests/
//! ui_plugin_examples.rs` depends on this crate directly and reads
//! these constants at test time, standing in for what a future
//! `nirdosha build`/`emit-ui` discovery pass would do automatically —
//! the same "hand-assemble it in Rust source for now" posture
//! `crates/compiler/tests/native_plugin_codegen.rs` already has toward
//! `NativePluginBuiltin` ahead of its own still-open Phase 3.

/// The bare identifier a `.nir` program writes as a layout leaf:
/// `sparkline { source: <fn> field: "..." }`.
pub const NAME: &str = "sparkline";

/// The exact top-level function name [`RENDER_JS`] declares.
pub const RENDER_FN: &str = "render_sparkline";

/// The JS spliced verbatim into the emitted `<script>` block.
pub const RENDER_JS: &str = include_str!("render.js");
