//! Compile-time UI-catalog extension (rfcs/0009-ui-catalog-
//! extensibility.md Phase B): a Rust crate contributes one additional
//! `layout { ... }` widget kind -- past the std closed set
//! (`"divider" | "card" | "timeline"`, `typeck.rs::check_screen_layout`)
//! -- by handing `typeck::typecheck_optional_main_with_ui_components`/
//! `ui_gen::generate_with_ui_components` a `&[NativeUiComponent]` slice.
//!
//! **Two ways to build that slice.** Hand-assemble it in Rust source
//! (`crates/compiler/tests/ui_plugin.rs`'s own proof, or an app's own
//! `build.rs`/CLI wrapper) — the same posture
//! `crates/compiler/tests/native_plugin_codegen.rs` already has toward
//! `NativePluginBuiltin` ahead of rfcs/0008's own still-open Phase 3.
//! Or call [`discover_components`], which reads it automatically from a
//! project's own `Cargo.toml` via `cargo metadata` — a real, working
//! answer to rfcs/0009's own "Open questions" for UI components
//! specifically (not for native builtins/rfcs/0008 Phase 3, which needs
//! a genuinely different, harder mechanism — see that function's own
//! doc comment for exactly why the two don't share one implementation
//! despite sharing one design intent).
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
//! **Trust boundary**: `render_js` is developer-authored JS, spliced
//! into the emitted single-file template verbatim at `emit-ui` build
//! time -- never generated, never model- or request-supplied. Whether
//! hand-assembled or discovered, it only ever comes from a crate the
//! app author chose to depend on (an ordinary `Cargo.toml` entry,
//! reviewed the same way any other dependency is) -- same trust level
//! `rfcs/0009` §"Effect on the permission model" already claims.

use std::path::Path;

/// One plugin-contributed `layout { ... }` widget kind.
#[derive(Debug, Clone)]
pub struct NativeUiComponent {
    /// The bare identifier a `.nir` program writes as a layout leaf,
    /// e.g. `sparkline { field: "amount" }` -- must not collide with a
    /// std kind (`"divider"`/`"card"`/`"timeline"`); neither
    /// `typecheck_optional_main_with_ui_components` nor
    /// [`discover_components`] detects a collision between two
    /// *linked* components sharing a name on its own -- see
    /// `discover_components`'s own doc comment for the one place that
    /// check does happen.
    pub name: String,
    /// The JS spliced verbatim into the emitted `<script>` block -- must
    /// declare a plain top-level function under the exact name
    /// `render_fn` below names, e.g. `function render_sparkline(node) {
    /// ... return /* a DOM node */; }`. `node` is the same `{type:
    /// "widget", kind, source, title, entries}` shape `ui_gen.rs::
    /// layout_json`'s `LayoutNode::Widget` arm already emits for every
    /// widget -- `node.entries.<key>` reaches whatever config keys the
    /// `.nir` program's own `layout { <name> { key: value } }` block
    /// declared. Owned, not `&'static str`: a discovered component's JS
    /// is read from a file at `nirdosha emit-ui` runtime, so it can
    /// never be a compile-time constant the way a hand-assembled
    /// component's `include_str!` literal already is (`String` accepts
    /// both — a `&'static str` still converts via `.to_string()`).
    pub render_js: String,
    /// The exact function name `render_js` declares -- kept separate
    /// from `name` (rather than derived from it) so a component's JS
    /// identifier isn't forced to match its (possibly non-identifier-
    /// shaped) `.nir`-facing name.
    pub render_fn: String,
}

/// Cargo-metadata-driven discovery for UI components (rfcs/0009's own
/// "Open questions" — the piece deliberately left unbuilt when Phase B
/// first shipped). Runs `cargo metadata --format-version 1
/// --manifest-path <manifest_path>` against the *app author's own*
/// project manifest (not `nirdosha`'s own `Cargo.toml`), which resolves
/// that project's full dependency graph — every crate it depends on,
/// including transitively — and returns every package whose own
/// `Cargo.toml` carries:
///
/// ```toml
/// [package.metadata.nirdosha]
/// kind = "nir-ui-component"
/// name = "sparkline"                # -> NativeUiComponent.name
/// render_fn = "render_sparkline"     # -> NativeUiComponent.render_fn
/// render_js_path = "src/render.js"   # relative to that crate's own Cargo.toml
/// ```
///
/// (`crates/ui-plugin-example-sparkline/Cargo.toml` is the real,
/// working example this format is drawn from.)
///
/// **Why this doesn't need `rfcs/0008` Phase 2/3's harder machinery.**
/// A native builtin crosses a real ABI boundary — its Rust function
/// signature has to be captured (Phase 2's proposed authoring macro) and
/// its compiled `.a` bytes have to be built and linked into the target
/// binary (Phase 3's `cargo build --release -p <crate>` + reading the
/// artifact). A UI component crosses no ABI boundary at all: its entire
/// "artifact" is three plain strings, one of which is JS *source text*
/// sitting in the crate's own tree — nothing to compile, nothing to
/// link. `cargo metadata` alone (which every Cargo installation already
/// ships, no new dependency) is enough to resolve the dependency graph
/// and find each declaring crate's own directory; reading its
/// `render_js_path` file is a plain `std::fs::read_to_string`. This is
/// genuinely a different, easier problem than rfcs/0008 Phase 3's own —
/// solving it here doesn't solve that one, and isn't a substitute for
/// it.
///
/// Returns `Ok(vec![])` for a project with no linked UI-component
/// crate at all (the common case) — never an error just for finding
/// nothing. Returns `Err` for a real problem: `cargo metadata` failing
/// to run at all, a package declaring `kind = "nir-ui-component"` but
/// missing one of `name`/`render_fn`/`render_js_path`, a declared
/// `render_js_path` that doesn't exist, or two linked components
/// declaring the same `name` (checked here, since this is the one place
/// that actually enumerates every linked component at once — a
/// hand-assembled slice has no equivalent check, same as any other
/// caller-owns-uniqueness Rust `Vec` construction).
pub fn discover_components(manifest_path: &Path) -> Result<Vec<NativeUiComponent>, String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let output = std::process::Command::new(&cargo)
        .args(["metadata", "--format-version", "1", "--manifest-path"])
        .arg(manifest_path)
        .output()
        .map_err(|e| format!("failed to invoke `{cargo} metadata` for UI-component discovery: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "`{cargo} metadata --manifest-path {}` failed:\n{}",
            manifest_path.display(),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let doc: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("failed to parse `cargo metadata` output as JSON: {e}"))?;
    let packages = doc
        .get("packages")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "`cargo metadata` output is missing its `packages` array".to_string())?;

    let mut components = Vec::new();
    let mut seen_names: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for pkg in packages {
        let pkg_name = pkg.get("name").and_then(|v| v.as_str()).unwrap_or("<unnamed package>");
        let Some(nirdosha_meta) = pkg.get("metadata").and_then(|m| m.get("nirdosha")).and_then(|v| v.as_object()) else {
            continue;
        };
        if nirdosha_meta.get("kind").and_then(|v| v.as_str()) != Some("nir-ui-component") {
            continue;
        }
        let get_str = |key: &str| -> Result<&str, String> {
            nirdosha_meta
                .get(key)
                .and_then(|v| v.as_str())
                .ok_or_else(|| format!("package `{pkg_name}` declares kind = \"nir-ui-component\" but has no metadata.nirdosha.{key}"))
        };
        let name = get_str("name")?.to_string();
        let render_fn = get_str("render_fn")?.to_string();
        let render_js_path = get_str("render_js_path")?;

        let manifest = pkg
            .get("manifest_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("package `{pkg_name}` is missing its own manifest_path in `cargo metadata` output"))?;
        let crate_dir = Path::new(manifest)
            .parent()
            .ok_or_else(|| format!("package `{pkg_name}`'s manifest_path `{manifest}` has no parent directory"))?;
        let js_path = crate_dir.join(render_js_path);
        let render_js = std::fs::read_to_string(&js_path).map_err(|e| {
            format!("package `{pkg_name}` declares render_js_path = \"{render_js_path}\" but reading {} failed: {e}", js_path.display())
        })?;

        if let Some(existing_pkg) = seen_names.insert(name.clone(), pkg_name.to_string()) {
            return Err(format!(
                "both `{existing_pkg}` and `{pkg_name}` declare metadata.nirdosha.name = \"{name}\" -- UI-component names must be unique across every linked crate"
            ));
        }
        components.push(NativeUiComponent { name, render_js, render_fn });
    }
    Ok(components)
}
