//! Tests for `serve { expose ... }` and the exposure model's
//! deny-by-default rule (`rfcs/0010-landing-and-serve-exposure.md`).
//! Existence/shape checking at the typeck layer only — actually
//! dispatching these routes over real HTTP is `compiled-serve`'s job
//! (ROADMAP B8), a separate, later phase.

use nirdosha::parser::Parser;
use nirdosha::token::Lexer;
use nirdosha::typeck::{exposed_public_read_warnings, typecheck, TypeErrorKind, TypeWarningKind};

fn parse_ok(src: &str) -> nirdosha::ast::Program {
    let toks = Lexer::new(src).tokenize().expect("lex should succeed");
    Parser::new(toks).parse_program().expect("parse should succeed")
}

fn first_type_error(src: &str) -> TypeErrorKind {
    let program = parse_ok(src);
    match typecheck(&program) {
        Ok(()) => panic!("expected a type error, but the program type-checked cleanly"),
        Err(errors) => errors.into_iter().next().unwrap().kind,
    }
}

#[test]
fn a_program_with_no_screen_or_serve_block_typechecks_exactly_as_before() {
    let src = "fn main() {}";
    let program = parse_ok(src);
    assert!(program.serve_config.is_none());
    typecheck(&program).expect("should typecheck cleanly with neither a screen nor a serve block");
}

#[test]
fn a_mutating_fn_bound_to_a_screen_create_with_no_requires_is_rejected() {
    let src = r#"
        struct Product { id: i64, name: str }
        fn list_product() -> i64 { return 0 }
        fn create_product(name: str) -> i64 { return 0 }
        screen Product {
            title: "Catalog"
            list: list_product
            create: create_product
        }
        fn main() {}
    "#;
    let kind = first_type_error(src);
    assert_eq!(kind, TypeErrorKind::ExposedMutatingFnMissingRequires { fn_name: "create_product".to_string() });
}

#[test]
fn requires_public_on_a_screen_bound_mutating_fn_is_accepted_as_an_explicit_choice() {
    let src = r#"
        struct Product { id: i64, name: str }
        fn list_product() -> i64 { return 0 }
        fn create_product(id: i64) -> i64 requires(public) { return 0 }
        screen Product {
            title: "Catalog"
            list: list_product
            create: create_product
        }
        fn main() {}
    "#;
    let program = parse_ok(src);
    typecheck(&program).expect("requires(public) is an explicit decision -- should be accepted");
}

#[test]
fn a_gated_screen_bound_mutating_fn_is_accepted() {
    let src = r#"
        struct Product { id: i64, name: str }
        fn list_product() -> i64 { return 0 }
        fn create_product(id: i64) -> i64 requires(role: "admin") { return 0 }
        screen Product {
            title: "Catalog"
            list: list_product
            create: create_product
        }
        fn main() {}
    "#;
    let program = parse_ok(src);
    typecheck(&program).expect("requires(role: \"admin\") should satisfy deny-by-default");
}

#[test]
fn a_mutating_fn_named_convention_but_never_bound_to_any_screen_is_not_flagged() {
    // The whole point of the *implicit* exposure set being narrower than
    // Row 12's pure naming-convention inference: `update_widget` exists,
    // matches the convention, but is never referenced by any `screen`/
    // `dashboard`/`serve { expose ... }` -- so it's not in the exposure
    // set at all, and the deny-by-default rule never applies to it.
    let src = r#"
        fn update_widget(id: i64) -> i64 { return id }
        fn main() {}
    "#;
    let program = parse_ok(src);
    typecheck(&program).expect("an unbound convention-named fn must not be treated as exposed");
}

#[test]
fn a_list_fn_bound_to_a_screen_with_no_requires_is_not_flagged_by_the_hard_rule() {
    // `list_` is read-only -- the hard deny-by-default error only covers
    // `create_`/`update_`/`delete_`. A public read is a *confidentiality*
    // concern (the separate warning below), not a hard error.
    let src = r#"
        struct Product { id: i64, name: str }
        fn list_product() -> i64 { return 0 }
        screen Product {
            title: "Catalog"
            list: list_product
        }
        fn main() {}
    "#;
    let program = parse_ok(src);
    typecheck(&program).expect("an exposed read-only route with no requires(...) is allowed (soft warning only)");
}

#[test]
fn the_confidentiality_warning_fires_once_per_program_not_once_per_function() {
    let src = r#"
        struct Product { id: i64, name: str }
        struct Order { id: i64, total: i64 }
        fn list_product() -> i64 { return 0 }
        fn stat_product_count() -> i64 { return 0 }
        screen Product {
            title: "Catalog"
            list: list_product
        }
        dashboard {
            tile "Count" -> stat_product_count
        }
        fn main() {}
    "#;
    let program = parse_ok(src);
    typecheck(&program).expect("should typecheck cleanly -- reads with no requires(...) are a warning, not an error");
    let warnings = exposed_public_read_warnings(&program);
    assert_eq!(warnings.len(), 1, "exactly one summarized warning, not one per exposed read");
    match &warnings[0].kind {
        TypeWarningKind::ExposedPublicReadsSummary { count } => assert_eq!(*count, 2),
        other => panic!("expected ExposedPublicReadsSummary, got {other:?}"),
    }
}

#[test]
fn requires_public_on_an_exposed_read_silences_the_confidentiality_warning() {
    let src = r#"
        struct Product { id: i64, name: str }
        fn list_product() -> i64 requires(public) { return 0 }
        screen Product {
            title: "Catalog"
            list: list_product
        }
        fn main() {}
    "#;
    let program = parse_ok(src);
    let warnings = exposed_public_read_warnings(&program);
    assert!(warnings.is_empty(), "an explicit requires(public) should silence the confidentiality warning");
}

#[test]
fn serve_expose_names_a_real_extra_fn_and_typechecks_cleanly() {
    let src = r#"
        fn utility_report() -> i64 requires(public) { return 0 }
        serve {
            expose utility_report
        }
        fn main() {}
    "#;
    let program = parse_ok(src);
    assert_eq!(program.serve_config.as_ref().unwrap().expose.len(), 1);
    typecheck(&program).expect("should typecheck cleanly");
}

#[test]
fn serve_expose_naming_an_undeclared_fn_is_rejected() {
    let src = r#"
        serve {
            expose no_such_fn
        }
        fn main() {}
    "#;
    let kind = first_type_error(src);
    assert_eq!(kind, TypeErrorKind::UnknownExposedFn { fn_name: "no_such_fn".to_string() });
}

#[test]
fn an_explicitly_exposed_mutating_fn_with_no_requires_is_rejected_too() {
    // Deny-by-default covers *both* halves of the exposure set, not just
    // the implicit screen-bound one.
    let src = r#"
        fn delete_everything() -> i64 { return 0 }
        serve {
            expose delete_everything
        }
        fn main() {}
    "#;
    let kind = first_type_error(src);
    assert_eq!(kind, TypeErrorKind::ExposedMutatingFnMissingRequires { fn_name: "delete_everything".to_string() });
}

#[test]
fn only_one_serve_block_is_allowed_per_program() {
    let src = r#"
        fn a() -> i64 requires(public) { return 0 }
        fn b() -> i64 requires(public) { return 0 }
        serve { expose a }
        serve { expose b }
        fn main() {}
    "#;
    let toks = Lexer::new(src).tokenize().expect("lex should succeed");
    let err = Parser::new(toks).parse_program().expect_err("a second serve block should be a parse error");
    assert!(err.message.contains("only one `serve"), "unexpected message: {}", err.message);
}

// ---- `ExposedFnRoleViewParamUnverifiable` -- red-team finding, 2026-09-11 ----
//
// Before this rule existed, a `RoleView`/`ClaimView`-typed parameter on
// an exposed fn with no matching `requires` fell through to
// `codegen.rs`'s generic JSON-arg decode path at the compiled `serve`
// boundary -- letting a client construct an arbitrary, self-asserted
// `RoleView` via the request body (`[{"role":"admin"}]`), bypassing
// field-level masking for a caller whose real identity never proved it.
// See `crates/compiler/tests/codegen.rs::compiled_serve_never_lets_a_client_supplied_role_view_bypass_field_masking`
// for the full HTTP-level reproduction of that exploit against the real
// compiled binary; these are the narrower typeck-layer tests for the
// rule that now refuses to compile the unsafe shape at all.

#[test]
fn an_exposed_fn_with_a_role_view_param_and_matching_requires_is_accepted() {
    let src = r#"
        struct Employee { name: str, salary: f64 requires(role: "admin") }
        fn list_employees(caller: RoleView) -> Employee requires(role: "hr_staff") {
            return Employee("Ada", 150000.0)
        }
        serve {
            expose list_employees
        }
        fn main() {}
    "#;
    let program = parse_ok(src);
    typecheck(&program).expect("a RoleView param anchored by a matching requires(role: ...) should typecheck");
}

#[test]
fn an_exposed_fn_with_a_role_view_param_and_no_requires_at_all_is_rejected() {
    let src = r#"
        struct Employee { name: str, salary: f64 requires(role: "admin") }
        fn list_employees(caller: RoleView) -> Employee {
            return Employee("Ada", 150000.0)
        }
        serve {
            expose list_employees
        }
        fn main() {}
    "#;
    let kind = first_type_error(src);
    assert_eq!(kind, TypeErrorKind::ExposedFnRoleViewParamUnverifiable { fn_name: "list_employees".to_string(), param_name: "caller".to_string(), is_claim_view: false });
}

#[test]
fn an_exposed_fn_with_a_role_view_param_but_a_claim_requires_is_rejected() {
    // The parameter *type* and the `requires` *kind* must match -- a
    // `requires(claim: ...)` doesn't anchor a `RoleView` parameter, even
    // though both ultimately check something about the identity.
    let src = r#"
        struct Employee { name: str, salary: f64 requires(role: "admin") }
        fn list_employees(caller: RoleView) -> Employee requires(claim: "department", "hr") {
            return Employee("Ada", 150000.0)
        }
        serve {
            expose list_employees
        }
        fn main() {}
    "#;
    let kind = first_type_error(src);
    assert_eq!(kind, TypeErrorKind::ExposedFnRoleViewParamUnverifiable { fn_name: "list_employees".to_string(), param_name: "caller".to_string(), is_claim_view: false });
}

#[test]
fn an_exposed_fn_with_a_claim_view_param_and_matching_requires_is_accepted() {
    let src = r#"
        struct Employee { name: str, salary: f64 requires(role: "admin") }
        fn list_employees(caller: ClaimView) -> Employee requires(claim: "department", "hr") {
            return Employee("Ada", 150000.0)
        }
        serve {
            expose list_employees
        }
        fn main() {}
    "#;
    let program = parse_ok(src);
    typecheck(&program).expect("a ClaimView param anchored by a matching requires(claim: ...) should typecheck");
}

#[test]
fn an_exposed_fn_with_a_claim_view_param_and_no_requires_at_all_is_rejected() {
    let src = r#"
        struct Employee { name: str, salary: f64 requires(role: "admin") }
        fn list_employees(caller: ClaimView) -> Employee {
            return Employee("Ada", 150000.0)
        }
        serve {
            expose list_employees
        }
        fn main() {}
    "#;
    let kind = first_type_error(src);
    assert_eq!(kind, TypeErrorKind::ExposedFnRoleViewParamUnverifiable { fn_name: "list_employees".to_string(), param_name: "caller".to_string(), is_claim_view: true });
}

#[test]
fn a_role_view_param_on_a_fn_never_exposed_to_serve_is_not_flagged() {
    // The whole rule is about the compiled-`serve` HTTP boundary
    // specifically -- an ordinary, never-exposed `.nir` function taking
    // a `RoleView` (the normal, safe, `acquire`-gated pattern) is
    // unaffected regardless of whether it declares a matching
    // `requires` itself.
    let src = r#"
        struct Employee { name: str, salary: f64 requires(role: "admin") }
        fn get_employee(caller: RoleView) -> Employee {
            return Employee("Ada", 150000.0)
        }
        fn main() {}
    "#;
    let program = parse_ok(src);
    typecheck(&program).expect("a RoleView param on a never-exposed fn must not be flagged");
}

#[test]
fn serve_expose_accepts_a_trailing_comma() {
    let src = r#"
        fn a() -> i64 requires(public) { return 0 }
        fn b() -> i64 requires(public) { return 0 }
        serve {
            expose a, b,
        }
        fn main() {}
    "#;
    let program = parse_ok(src);
    assert_eq!(program.serve_config.as_ref().unwrap().expose.len(), 2);
    typecheck(&program).expect("should typecheck cleanly");
}
