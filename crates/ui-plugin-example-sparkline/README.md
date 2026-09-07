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
    render_js: nirdosha_ui_plugin_example_sparkline::RENDER_JS,
    render_fn: nirdosha_ui_plugin_example_sparkline::RENDER_FN,
};
```

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

## No Cargo-driven discovery yet

`[package.metadata.nirdosha] kind = "nir-ui-component"` in this crate's
own `Cargo.toml` is there for a future `nirdosha build`/`emit-ui`
auto-discovery pass to grep for — that pass isn't built (rfcs/0009's
own "Open questions" names it as shared, still-open work with
`rfcs/0008` Phase 3's identical discovery problem for native builtins).
Until it lands, whoever wants this widget depends on this crate
directly and hand-assembles the `NativeUiComponent`, exactly as shown
above — a real, disclosed narrowing, not a broken promise.
