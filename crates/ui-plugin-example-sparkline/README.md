# nirdosha-ui-plugin-example-sparkline

The reference for `rfcs/0009-ui-catalog-extensibility.md` Phase B's
compile-time UI-catalog extension: a `layout { ... }` widget kind past
the std closed set (`"divider" | "card" | "timeline"`), contributed by
a genuinely separate crate rather than inline Rust in a test file.

## Try it

This crate has no build step of its own to run — it's a plain library
of constants (`NAME`/`RENDER_FN`/`RENDER_JS`), read directly by whoever
assembles a `nirdosha::ui_plugin::NativeUiComponent`:

```rust
use nirdosha::ui_plugin::NativeUiComponent;

let sparkline = NativeUiComponent {
    name: nirdosha_ui_plugin_example_sparkline::NAME.to_string(),
    render_js: nirdosha_ui_plugin_example_sparkline::RENDER_JS.to_string(),
    render_fn: nirdosha_ui_plugin_example_sparkline::RENDER_FN.to_string(),
};
```

(`render_js`/`render_fn` are owned `String`, not `&'static str` — a
*discovered* component's JS is read from disk at `emit-ui` runtime, so
the field can't stay a compile-time constant; this crate's own
`&'static str` constants still convert via `.to_string()`.)

`crates/compiler/tests/ui_plugin_examples.rs` does exactly this, then
runs the real `typecheck_optional_main_with_ui_components`/
`generate_with_ui_components` pipeline against it —
`cargo test -p nirdosha --test ui_plugin_examples`.

## What it demonstrates

A `.nir` program writes the widget as an ordinary layout leaf — no new
syntax, the same bare-identifier shape `divider {}`/`card { ... }`/
`timeline { ... }` already have:

```nir
screen SalesReport {
    layout {
        sparkline { source: recent_sales_totals field: "amount" }
    }
}
```

[`src/render.js`](./src/render.js)'s `render_sparkline(node)` reads
`node.source` (a zero-arg-from-the-client fn name, the same `source`
key the std `timeline` widget already uses) and `node.entries.field`
(a generic passthrough of every other key the `.nir` program declared —
`ui_gen.rs::widget_entry_json`, new in this same RFC), fetches the data
via the page's own `callFn`, and draws a small inline SVG trend line —
zero external charting library, the same "self-contained" stance every
other renderer in `ui_gen_template.html` already holds.

## Cargo-driven discovery is real for this crate

`[package.metadata.nirdosha] kind = "nir-ui-component"` in this crate's
own `Cargo.toml` is exactly what `ui_plugin::discover_components`
(`nirdosha emit-ui --manifest-path <Cargo.toml>`, or auto-detected next
to the input `.nir` file) greps an app author's own dependency graph
for via a real `cargo metadata` call — no hand-assembled Rust slice
needed. `crates/compiler/tests/ui_plugin_discovery.rs` spawns the real
compiled `nirdosha` binary against a scratch project whose own
`Cargo.toml` path-depends on this crate and proves auto-detection, the
explicit flag, and the untouched no-manifest default all work.

This closes rfcs/0009's own "Open questions" item **for UI components
specifically** — it does not close `rfcs/0008` Phase 3 (Cargo-driven
discovery for *native builtins*), a genuinely different, harder problem
since a native builtin crosses a real ABI boundary and a UI component
doesn't (see `ui_plugin.rs`'s own doc comment for the full reasoning).
The hand-assembled path shown above (`crates/compiler/tests/
ui_plugin_examples.rs`) still works too, and remains useful for
anything short of a real linked dependency.
