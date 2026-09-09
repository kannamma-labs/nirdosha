//! Direct, non-`--ignored` tests for `contract_check::check_fn_contract`
//! against small, hand-written `.nir` snippets -- unlike
//! `extracted_typed_v1_verification.rs`/`extracted_typed_v2_verification.rs`,
//! this file needs no local-only fixture, so `cargo test` always runs it.

use std::collections::HashMap;

use nirdosha::ast::Program;
use nirdosha::contract_check::{check_fn_contract, ContractCheckResult};
use nirdosha::ownership::check_ownership;
use nirdosha::parser::Parser;
use nirdosha::token::Lexer;
use nirdosha::typeck::typecheck_optional_main;

fn build_program(src: &str) -> Program {
    let toks = Lexer::new(src).tokenize().expect("lex should succeed");
    let program = Parser::new(toks).parse_program().expect("parse should succeed");
    typecheck_optional_main(&program).unwrap_or_else(|e| panic!("typecheck should succeed: {e:?}"));
    check_ownership(&program).expect("ownership check should succeed");
    program
}

const DOUBLE_NIR: &str = r#"
    fn double(x: i32) -> i32 {
        return x * 2
    }
"#;

#[test]
fn satisfiable_pre_still_proves_a_true_post() {
    let program = build_program(DOUBLE_NIR);
    let result = check_fn_contract(&program, "double", &["x > 0".to_string()], &["result > 0".to_string()], &HashMap::new());
    assert_eq!(result, ContractCheckResult::Proved, "unexpected result: {result:?}");
}

#[test]
fn contradictory_pre_logic_is_vacuous_not_proved() {
    let program = build_program(DOUBLE_NIR);
    // No `x` satisfies both halves at once -- every `post_logic` below
    // would otherwise "pass" for a reason that has nothing to do with
    // `double` actually being correct.
    let result = check_fn_contract(
        &program,
        "double",
        &["x > 10".to_string(), "x < 5".to_string()],
        &["result > 0".to_string()],
        &HashMap::new(),
    );
    assert_eq!(result, ContractCheckResult::VacuousPrecondition, "unexpected result: {result:?}");
}

#[test]
fn contradictory_pre_logic_is_flagged_even_when_the_post_is_false() {
    let program = build_program(DOUBLE_NIR);
    // Same unsatisfiable precondition, but this time paired with a
    // `post_logic` that's obviously false for `double` (it never
    // returns a negative number for a positive `x`) -- if this test
    // reported `Proved`, the vacuous-precondition detection would be a
    // no-op in exactly the case it exists to catch: this must be
    // `VacuousPrecondition`, never a false `Proved`.
    let result = check_fn_contract(
        &program,
        "double",
        &["x > 10".to_string(), "x < 5".to_string()],
        &["result < 0".to_string()],
        &HashMap::new(),
    );
    assert_eq!(result, ContractCheckResult::VacuousPrecondition, "unexpected result: {result:?}");
}

#[test]
fn empty_pre_logic_is_never_vacuous() {
    let program = build_program(DOUBLE_NIR);
    // No `pre_logic` at all means "the function's full declared-type
    // domain" -- always non-empty, so this must behave exactly as
    // before this check existed: a real counterexample, not a vacuous
    // precondition.
    let result = check_fn_contract(&program, "double", &[], &["result < 0".to_string()], &HashMap::new());
    match result {
        ContractCheckResult::Counterexample { .. } => {}
        other => panic!("expected a Counterexample, got {other:?}"),
    }
}
