//! The real, end-to-end proof for rfcs/0009-ui-catalog-extensibility.md
//! Phase B: a hand-assembled `ui_plugin::NativeUiComponent` (the same
//! "no Cargo auto-discovery yet, hand-assemble the slice in Rust
//! source" posture `tests/native_plugin_codegen.rs` already holds for
//! `NativePluginBuiltin` ahead of rfcs/0008 Phase 3) actually widens
//! what a `layout { ... }` block can name, and its `render_js` actually
//! reaches the emitted client -- not just "the struct compiles."

use nirdosha::ownership::check_ownership;
use nirdosha::parser::Parser;
use nirdosha::token::Lexer;
use nirdosha::typeck::{typecheck_optional_main, typecheck_optional_main_with_ui_components, TypeErrorKind};
use nirdosha::ui_gen::{generate, generate_with_ui_components};
use nirdosha::ui_plugin::NativeUiComponent;

const SRC: &str = r#"
    struct Trend {
        id: i64,
        amount: i64,
    }
    fn list_trend() -> Result(json, i64) requires(public) {
        return match json_parse("[]") { Ok(v) => Ok(v), Err(e) => Err(0), }
    }

    screen Trend {
        layout {
            column {
                sparkline { field: "amount" }
            }
        }
    }

    fn main() {}
"#;

fn sparkline_component() -> NativeUiComponent {
    NativeUiComponent {
        name: "sparkline".to_string(),
        render_js: "function render_sparkline(node) { \
            const d = document.createElement('div'); \
            d.className = 'sparkline-stub'; \
            d.textContent = 'sparkline:' + (node.entries && node.entries.field); \
            return d; \
        }",
        render_fn: "render_sparkline",
    }
}

fn parse(src: &str) -> nirdosha::ast::Program {
    let toks = Lexer::new(src).tokenize().expect("lex should succeed");
    Parser::new(toks).parse_program().expect("parse should succeed")
}

#[test]
fn an_unlinked_plugin_widget_kind_is_rejected_the_same_as_any_unknown_kind() {
    let program = parse(SRC);
    let errors = typecheck_optional_main(&program).expect_err("no component linked -- `sparkline` isn't a real kind yet");
    assert!(
        errors
            .iter()
            .any(|e| matches!(&e.kind, TypeErrorKind::UnknownRenderValue { key, render, .. } if key == "kind" && render == "sparkline")),
        "expected an UnknownRenderValue{{kind: \"sparkline\"}} error, got: {errors:?}"
    );
}

#[test]
fn linking_the_component_makes_its_name_a_legal_widget_kind() {
    let program = parse(SRC);
    let components = [sparkline_component()];
    typecheck_optional_main_with_ui_components(&program, &components)
        .expect("`sparkline` should typecheck once the component is linked");
    check_ownership(&program).expect("ownership check should succeed");
}

#[test]
fn generate_with_ui_components_splices_the_render_js_and_registers_it() {
    let program = parse(SRC);
    let components = [sparkline_component()];
    typecheck_optional_main_with_ui_components(&program, &components).expect("typecheck");
    let registry = nirdosha::ast::TypeRegistry::build(&program);
    let effects = nirdosha::effects::infer_effects(&program, &registry);

    let html = generate_with_ui_components(&program, &effects, None, false, false, false, None, &components);
    assert!(html.contains("function render_sparkline(node)"), "component render_js should be spliced in verbatim");
    assert!(
        html.contains(r#"WIDGET_RENDERERS["sparkline"] = render_sparkline;"#),
        "component should be registered into WIDGET_RENDERERS by name"
    );
    // The widget leaf itself reaches the manifest with its own config
    // entries intact -- `entries.field`, not just `source`/`title`.
    assert!(html.contains(r#""kind":"sparkline""#));
    assert!(html.contains(r#""entries":{"field":"amount"}"#));
}

#[test]
fn plain_generate_links_no_components_and_stays_byte_for_byte_the_same_shape() {
    // `generate` (no components) must not silently pick up the plugin --
    // proves `generate`/`generate_with_ui_components` are genuinely
    // separate opt-in entry points, not one defaulting into the other.
    let program = parse(SRC);
    let components = [sparkline_component()];
    // typeck still needs the component linked for this fixture to
    // typecheck at all (its `layout` names `sparkline`) -- this test is
    // only about `generate`'s own output, not re-proving typeck's gate.
    typecheck_optional_main_with_ui_components(&program, &components).expect("typecheck");
    let registry = nirdosha::ast::TypeRegistry::build(&program);
    let effects = nirdosha::effects::infer_effects(&program, &registry);

    let html = generate(&program, &effects, None, false, false, false, None);
    assert!(!html.contains("render_sparkline"));
    assert!(!html.contains("WIDGET_RENDERERS[\"sparkline\"]"));
    assert!(html.contains("const WIDGET_RENDERERS = {};"));
}
