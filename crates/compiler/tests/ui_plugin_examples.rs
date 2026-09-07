//! Proves that `crates/ui-plugin-example-sparkline` — a genuinely
//! separate crate, not inline Rust in `tests/ui_plugin.rs` — actually
//! works as its own README claims, the same "exercise the identical
//! bytes a real user would get from this crate" standard
//! `tests/native_plugin_examples.rs` already holds itself to for
//! `plugin-example-native-shout`/`plugin-example-native-kv`. No linking
//! step is needed here (a UI component crosses no ABI boundary, unlike
//! a native builtin) — this crate's `NAME`/`RENDER_FN`/`RENDER_JS`
//! constants, read via an ordinary `[dev-dependencies]` edge, are
//! exactly what a consumer would read to build a real
//! `ui_plugin::NativeUiComponent`.

use nirdosha::typeck::{typecheck_optional_main, typecheck_optional_main_with_ui_components, TypeErrorKind};
use nirdosha::ui_gen::generate_with_ui_components;
use nirdosha::ui_plugin::NativeUiComponent;
use nirdosha_ui_plugin_example_sparkline as sparkline_crate;

fn component() -> NativeUiComponent {
    NativeUiComponent {
        name: sparkline_crate::NAME.to_string(),
        render_js: sparkline_crate::RENDER_JS.to_string(),
        render_fn: sparkline_crate::RENDER_FN.to_string(),
    }
}

const SRC: &str = r#"
    struct Report {
        id: i64,
    }
    fn recent_sales_totals() -> Result(json, i64) requires(public) {
        return match json_parse("[]") { Ok(v) => Ok(v), Err(e) => Err(0), }
    }
    fn list_report() -> Result(json, i64) requires(public) {
        return match json_parse("[]") { Ok(v) => Ok(v), Err(e) => Err(0), }
    }

    screen Report {
        layout {
            column {
                sparkline { source: recent_sales_totals field: "amount" }
            }
        }
    }

    fn main() {}
"#;

#[test]
fn the_crate_exposes_a_plain_js_function_under_its_own_declared_name() {
    assert_eq!(sparkline_crate::NAME, "sparkline");
    assert_eq!(sparkline_crate::RENDER_FN, "render_sparkline");
    assert!(sparkline_crate::RENDER_JS.contains("function render_sparkline(node)"));
    // `node.source`/`node.entries.field` are the real widget-JSON keys
    // `ui_gen.rs::layout_json`'s `Widget` arm emits (rfcs/0009 Phase B)
    // -- a smoke check that this crate's JS actually reads the shape
    // the real pipeline hands it, not an assumed one.
    assert!(sparkline_crate::RENDER_JS.contains("node.source"));
    assert!(sparkline_crate::RENDER_JS.contains("node.entries"));
}

#[test]
fn unlinked_sparkline_is_rejected_the_same_as_any_unknown_widget_kind() {
    let toks = nirdosha::token::Lexer::new(SRC).tokenize().expect("lex");
    let program = nirdosha::parser::Parser::new(toks).parse_program().expect("parse");
    let errors = typecheck_optional_main(&program).expect_err("sparkline isn't a real kind without the crate linked");
    assert!(errors
        .iter()
        .any(|e| matches!(&e.kind, TypeErrorKind::UnknownRenderValue { key, render, .. } if key == "kind" && render == "sparkline")));
}

#[test]
fn linking_the_real_crate_makes_sparkline_a_legal_widget_and_splices_its_real_js() {
    let toks = nirdosha::token::Lexer::new(SRC).tokenize().expect("lex");
    let program = nirdosha::parser::Parser::new(toks).parse_program().expect("parse");
    let components = [component()];

    typecheck_optional_main_with_ui_components(&program, &components).expect("sparkline should typecheck once linked");
    nirdosha::ownership::check_ownership(&program).expect("ownership check should succeed");

    let registry = nirdosha::ast::TypeRegistry::build(&program);
    let effects = nirdosha::effects::infer_effects(&program, &registry);
    let html = generate_with_ui_components(&program, &effects, None, false, false, false, None, &components);

    assert!(html.contains(sparkline_crate::RENDER_JS), "the crate's real JS text should be spliced in verbatim, not paraphrased");
    assert!(html.contains(r#"WIDGET_RENDERERS["sparkline"] = render_sparkline;"#));
    assert!(html.contains(r#""kind":"sparkline""#));
    assert!(html.contains(r#""source":"recent_sales_totals""#));
    // `entries` is every kv-entry generically (ui_gen.rs::widget_entry_json)
    // -- `source` lands in both `node.source` (checked above, what the
    // std `timeline` widget's own code already reads) and here, a
    // harmless superset, not a second parallel shape.
    assert!(html.contains(r#""entries":{"field":"amount","source":"recent_sales_totals"}"#));
}
