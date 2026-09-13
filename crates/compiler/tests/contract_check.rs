//! Direct, non-`--ignored` tests for `contract_check::check_fn_contract`
//! against small, hand-written `.nir` snippets -- unlike
//! `extracted_typed_v1_verification.rs`/`extracted_typed_v2_verification.rs`,
//! this file needs no local-only fixture, so `cargo test` always runs it.

use std::collections::{HashMap, HashSet};

use nirdosha::ast::Program;
use nirdosha::contract_check::{check_fn_contract, check_mandatory_primitive_call_sites, ContractCheckResult};
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

// RFC 0016 Phase 3: `check_mandatory_primitive_call_sites` -- `transfer`
// stands in for a pack's own certified primitive (its `validate` block
// is exactly what `hi_plugin::prepend_pack_primitives` would prepend
// alongside the primitive's own real body). `pay_invoice_unsafe` calls
// it with no guard at all; `pay_invoice_safe` calls it only after
// establishing the exact precondition `transfer` demands.
const PRIMITIVE_AND_CALLERS_NIR: &str = r#"
    fn transfer(amount: i64) -> i64 {
        return amount
    }

    validate transfer {
        pre: amount > 0
        post: result == amount
    }

    fn pay_invoice_unsafe(amount: i64) -> i64 {
        return transfer(amount)
    }

    fn pay_invoice_safe(amount: i64) -> i64 {
        return if amount > 0 { transfer(amount) } else { 0 }
    }
"#;

#[test]
fn a_call_site_with_no_guard_at_all_is_a_real_precondition_violation() {
    let program = build_program(PRIMITIVE_AND_CALLERS_NIR);
    let mut mandatory = HashSet::new();
    mandatory.insert("transfer".to_string());
    let outcomes = check_mandatory_primitive_call_sites(&program, &mandatory);
    let unsafe_outcome = outcomes.iter().find(|o| o.fn_name == "pay_invoice_unsafe").expect("pay_invoice_unsafe must be in the sweep");
    match &unsafe_outcome.result {
        ContractCheckResult::Counterexample { violated_predicate, .. } => {
            assert!(violated_predicate.contains("transfer"), "got: {violated_predicate}");
        }
        other => panic!("expected a Counterexample (an unguarded call can pass amount <= 0), got {other:?}"),
    }
}

#[test]
fn a_call_site_guarded_by_the_callees_own_precondition_proves() {
    let program = build_program(PRIMITIVE_AND_CALLERS_NIR);
    let mut mandatory = HashSet::new();
    mandatory.insert("transfer".to_string());
    let outcomes = check_mandatory_primitive_call_sites(&program, &mandatory);
    let safe_outcome = outcomes.iter().find(|o| o.fn_name == "pay_invoice_safe").expect("pay_invoice_safe must be in the sweep");
    assert_eq!(safe_outcome.result, ContractCheckResult::Proved, "the `if amount > 0` guard already establishes transfer's own precondition on this path: {:?}", safe_outcome.result);
}

#[test]
fn a_callee_not_named_in_mandatory_fns_is_never_gated() {
    let program = build_program(PRIMITIVE_AND_CALLERS_NIR);
    // `transfer` is deliberately absent from this set -- the same
    // unguarded call site that fails the check above must be untouched
    // when nothing declares `transfer` mandatory.
    let mut mandatory = HashSet::new();
    mandatory.insert("some_other_primitive_entirely".to_string());
    let outcomes = check_mandatory_primitive_call_sites(&program, &mandatory);
    let unsafe_outcome = outcomes.iter().find(|o| o.fn_name == "pay_invoice_unsafe").expect("pay_invoice_unsafe must be in the sweep");
    assert_eq!(unsafe_outcome.result, ContractCheckResult::Proved, "a callee not declared mandatory must never be gated: {:?}", unsafe_outcome.result);
}

#[test]
fn an_empty_mandatory_fns_set_sweeps_nothing() {
    let program = build_program(PRIMITIVE_AND_CALLERS_NIR);
    assert!(check_mandatory_primitive_call_sites(&program, &HashSet::new()).is_empty());
}
