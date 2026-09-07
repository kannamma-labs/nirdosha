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
//! builds its own `ui_plugin::NativeUiComponent { name: NAME.to_string(),
//! render_js: RENDER_JS.to_string(), render_fn: RENDER_FN.to_string() }`
//! (`render_js`/`render_fn` are owned `String`, not `&'static str` —
//! a *discovered* component's JS is read from disk at `emit-ui` runtime,
//! so the field can't stay a compile-time constant; `RENDER_JS`/
//! `RENDER_FN` here still convert via `.to_string()`).
//!
//! **Cargo-driven auto-discovery exists for this crate specifically**
//! (`ui_plugin::discover_components`, `nirdosha emit-ui
//! --manifest-path`/auto-detected `Cargo.toml`): this crate's own
//! `Cargo.toml` carries `[package.metadata.nirdosha] kind =
//! "nir-ui-component"`, which is exactly what that discovery pass greps
//! an app author's dependency graph for — see that `Cargo.toml`'s own
//! comment and `crates/compiler/tests/ui_plugin_discovery.rs` for the
//! real, no-hand-assembly, no-dev-dependency end-to-end proof.
//! `crates/compiler/tests/ui_plugin_examples.rs` additionally depends on
//! this crate directly via an ordinary `[dev-dependencies]` edge and
//! reads these constants to hand-assemble a `NativeUiComponent` itself —
//! a second, independent proof that the crate's exported constants are
//! usable on their own, not a stand-in for discovery not existing.
//! (Still open, and shared with `rfcs/0008` Phase 3: Cargo-driven
//! discovery for native *builtins*, a genuinely different, harder
//! problem — see `ui_plugin.rs`'s own doc comment for why.)

/// The bare identifier a `.nir` program writes as a layout leaf:
/// `sparkline { source: <fn> field: "..." }`.
pub const NAME: &str = "sparkline";

/// The exact top-level function name [`RENDER_JS`] declares.
pub const RENDER_FN: &str = "render_sparkline";

/// The JS spliced verbatim into the emitted `<script>` block.
pub const RENDER_JS: &str = include_str!("render.js");
