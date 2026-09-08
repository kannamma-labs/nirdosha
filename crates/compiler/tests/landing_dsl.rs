//! Tests for `landing { ... }` (`rfcs/0010-landing-and-serve-exposure.md`)
//! — per-role/claim default-screen dispatch. Existence/shape checking
//! only, the same scope `screen_dsl.rs`'s own tests cover for
//! `screen`/`dashboard`: does every rule's target resolve to a real
//! `screen`, is there exactly one `default`, is it last, and is no rule
//! an unreachable duplicate of an earlier one.

use nirdosha::parser::Parser;
use nirdosha::token::Lexer;
use nirdosha::typeck::{typecheck, TypeErrorKind};

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

const SCREENS: &str = r#"
    struct AdminHome { id: i64 }
    struct ClinicianQueue { id: i64 }
    struct CardiologyQueue { id: i64 }
    struct HomeScreen { id: i64 }

    screen AdminHome { title: "Admin" }
    screen ClinicianQueue { title: "Queue" }
    screen CardiologyQueue { title: "Cardiology" }
    screen HomeScreen { title: "Home" }
"#;

#[test]
fn a_program_with_no_landing_block_typechecks_exactly_as_before() {
    let src = format!("{SCREENS}\nfn main() {{}}");
    let program = parse_ok(&src);
    assert!(program.landing.is_none());
    typecheck(&program).expect("should typecheck cleanly with no landing block at all");
}

#[test]
fn well_formed_landing_with_role_claim_and_default_parses_and_typechecks_cleanly() {
    let src = format!(
        r#"{SCREENS}
        landing {{
            role("admin") -> AdminHome
            claim("department", "cardiology") -> CardiologyQueue
            role("clinician") -> ClinicianQueue
            default -> HomeScreen
        }}
        fn main() {{}}
    "#
    );
    let program = parse_ok(&src);
    let landing = program.landing.as_ref().expect("landing block should have parsed");
    assert_eq!(landing.rules.len(), 4);
    typecheck(&program).expect("should typecheck cleanly");
}

#[test]
fn only_one_landing_block_is_allowed_per_program() {
    let src = format!(
        r#"{SCREENS}
        landing {{ default -> HomeScreen }}
        landing {{ default -> AdminHome }}
        fn main() {{}}
    "#
    );
    let toks = Lexer::new(&src).tokenize().expect("lex should succeed");
    let err = Parser::new(toks).parse_program().expect_err("a second landing block should be a parse error");
    assert!(err.message.contains("only one `landing"), "unexpected message: {}", err.message);
}

#[test]
fn a_landing_rule_targeting_an_undeclared_screen_is_rejected() {
    let src = format!(
        r#"{SCREENS}
        landing {{
            role("admin") -> NoSuchScreen
            default -> HomeScreen
        }}
        fn main() {{}}
    "#
    );
    let kind = first_type_error(&src);
    assert_eq!(kind, TypeErrorKind::UnknownLandingTarget { target: "NoSuchScreen".to_string() });
}

#[test]
fn a_landing_block_with_no_default_rule_is_rejected() {
    let src = format!(
        r#"{SCREENS}
        landing {{
            role("admin") -> AdminHome
        }}
        fn main() {{}}
    "#
    );
    let kind = first_type_error(&src);
    assert_eq!(kind, TypeErrorKind::LandingMissingDefault);
}

#[test]
fn a_landing_block_with_two_default_rules_is_rejected() {
    let src = format!(
        r#"{SCREENS}
        landing {{
            default -> HomeScreen
            default -> AdminHome
        }}
        fn main() {{}}
    "#
    );
    let kind = first_type_error(&src);
    assert_eq!(kind, TypeErrorKind::LandingDuplicateDefault);
}

#[test]
fn a_rule_after_default_is_rejected_as_unreachable() {
    let src = format!(
        r#"{SCREENS}
        landing {{
            default -> HomeScreen
            role("admin") -> AdminHome
        }}
        fn main() {{}}
    "#
    );
    let kind = first_type_error(&src);
    assert_eq!(kind, TypeErrorKind::LandingRuleAfterDefault);
}

#[test]
fn a_duplicate_role_rule_is_rejected_as_unreachable() {
    let src = format!(
        r#"{SCREENS}
        landing {{
            role("admin") -> AdminHome
            role("admin") -> HomeScreen
            default -> HomeScreen
        }}
        fn main() {{}}
    "#
    );
    let kind = first_type_error(&src);
    match kind {
        TypeErrorKind::LandingUnreachableRule { .. } => {}
        other => panic!("expected LandingUnreachableRule, got {other:?}"),
    }
}

#[test]
fn a_duplicate_claim_rule_is_rejected_as_unreachable() {
    let src = format!(
        r#"{SCREENS}
        landing {{
            claim("department", "cardiology") -> CardiologyQueue
            claim("department", "cardiology") -> HomeScreen
            default -> HomeScreen
        }}
        fn main() {{}}
    "#
    );
    let kind = first_type_error(&src);
    match kind {
        TypeErrorKind::LandingUnreachableRule { .. } => {}
        other => panic!("expected LandingUnreachableRule, got {other:?}"),
    }
}

#[test]
fn a_role_and_a_claim_rule_are_never_confused_as_duplicates() {
    // Different `Requirement` variants -- `role("admin")` and
    // `claim("admin", "admin")`, say -- must never compare equal just
    // because they happen to share a string.
    let src = format!(
        r#"{SCREENS}
        landing {{
            role("admin") -> AdminHome
            claim("admin", "admin") -> HomeScreen
            default -> HomeScreen
        }}
        fn main() {{}}
    "#
    );
    let program = parse_ok(&src);
    typecheck(&program).expect("role(\"admin\") and claim(\"admin\",\"admin\") must not collide");
}

#[test]
fn landing_inside_a_module_is_rejected_the_same_as_screen_dashboard() {
    let src = format!(
        r#"{SCREENS}
        module "Nested" {{
            landing {{ default -> HomeScreen }}
        }}
        fn main() {{}}
    "#
    );
    let toks = Lexer::new(&src).tokenize().expect("lex should succeed");
    let err = Parser::new(toks).parse_program().expect_err("landing nested in a module should be a parse error");
    assert!(err.message.contains("landing"), "unexpected message: {}", err.message);
}
