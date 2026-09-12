//! RFC 0016 Phase 0: deterministic solver fuel and the `EngineLimit`
//! classification. A separate test *binary* on purpose — the fuel
//! override this file uses is process-global, and running it in
//! `contract_check.rs`'s own binary would let these tests race every
//! concurrently-running contract test in that process (a solver
//! created mid-override would get the tiny test fuel and fail
//! spuriously). One process per concern: here, and only here, the
//! fuel is manipulated.
//!
//! The load-bearing property under test is RFC 0016's fail-closed
//! semantics: fuel exhaustion must surface as `EngineLimit` — its own
//! classification, never a silent `Proved` ("unknown treated as
//! unsat" is exactly the hole the RFC closes), never a panic (the
//! equivalence path used to `get_model().expect(...)` on an `unknown`
//! answer), and never a violation of the code being checked.

use std::collections::HashMap;

use nirdosha::ast::Program;
use nirdosha::contract_check::{check_equivalence, check_fn_contract, set_proof_fuel_rlimit, ContractCheckResult, EquivalenceResult};
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

    fn triple(x: i32) -> i32 {
        return x * 3
    }
"#;

/// The whole fuel lifecycle in one test function, serialized on
/// purpose: override → check the classification → restore the
/// default → check the same obligation proves normally. Keeping it
/// in one fn means the override window is closed before any other
/// test in this binary creates a solver.
#[test]
fn fuel_exhaustion_is_engine_limit_and_default_fuel_still_proves() {
    let program = build_program(DOUBLE_NIR);

    // 1. Starve the solver: rlimit=1 is small enough that even
    //    trivially-linear obligations cannot be decided.
    set_proof_fuel_rlimit(1);
    let starved = check_fn_contract(&program, "double", &[], &["result > x".to_string()], &HashMap::new());
    match starved {
        ContractCheckResult::EngineLimit { obligation, fuel } => {
            assert_eq!(fuel, 1, "the fuel actually in force must be reported");
            assert!(!obligation.is_empty(), "the obligation names what couldn't be decided");
        }
        other => panic!("expected EngineLimit under a starved rlimit, got {other:?} -- \
                         fuel exhaustion must never masquerade as Proved, a Counterexample, or a violation"),
    }

    // 2. Equivalence under the same starvation: a *genuinely different*
    //    pair, so the divergence search needs a real model, which the
    //    starved engine cannot produce. This used to panic
    //    (get_model().expect on an `unknown` answer) before the
    //    EngineLimit arm existed. (Equivalent pairs like `x*2` vs
    //    `x+x` decide by simplification even starved -- soundly, so
    //    they must NOT be forced to EngineLimit; the starved test
    //    needs a search, not a simplification.)
    let starved_eq = check_equivalence(&program, "double", "triple");
    assert_eq!(starved_eq, EquivalenceResult::EngineLimit, "equivalence must classify fuel exhaustion, not panic on it");

    // 3. Restore the default and prove the same obligations normally:
    //    the starved results were the engine's limit, not the code's
    //    truth -- `double` really does return `> x` for all `x` (in
    //    i32 bounds, `x*2 > x` iff `x > -2^30`, and... no: `x = -1`
    //    gives `-2 > -1`, false. So the obligation is NOT a theorem
    //    and the default-fuel answer below must be the real
    //    counterexample, which is exactly the point: starving the
    //    solver gave no answer at all, the default fuel gives the
    //    true one.)
    set_proof_fuel_rlimit(0);
    let fed = check_fn_contract(&program, "double", &[], &["result > x".to_string()], &HashMap::new());
    match fed {
        ContractCheckResult::Counterexample { .. } => {}
        other => panic!("expected a real Counterexample under the default fuel (x = -1 breaks `result > x`), got {other:?}"),
    }
    let fed_eq = check_equivalence(&program, "double", "triple");
    match fed_eq {
        EquivalenceResult::Different { .. } => {}
        other => panic!("expected a real Different counterexample under the default fuel (x = 1), got {other:?}"),
    }

    // 4. And a genuinely-true obligation proves under the default.
    let proved = check_fn_contract(&program, "double", &[], &["result == x + x".to_string()], &HashMap::new());
    assert_eq!(proved, ContractCheckResult::Proved, "the default fuel must still prove ordinary contracts");
}

#[test]
fn vacuity_check_under_starved_fuel_is_engine_limit_not_satisfiable() {
    let program = build_program(DOUBLE_NIR);
    set_proof_fuel_rlimit(1);
    // A satisfiable precondition with a starved engine: the vacuity
    // check's `solver.check()` cannot decide SAT, and "unknown" must
    // NOT be read as "satisfiable, proceed" -- that would push the
    // obligation downstream to burn the already-empty budget and
    // misreport the whole contract.
    let result = check_fn_contract(&program, "double", &["x > 0".to_string()], &["result > 0".to_string()], &HashMap::new());
    set_proof_fuel_rlimit(0);
    match result {
        ContractCheckResult::EngineLimit { .. } => {}
        other => panic!("expected EngineLimit from the starved vacuity check, got {other:?}"),
    }
}