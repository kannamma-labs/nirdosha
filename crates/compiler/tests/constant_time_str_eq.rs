//! Tests for `constant_time_str_eq` — a red-team finding, fixed: `.nir`
//! code comparing two secret-derived strings had no way to do it other
//! than `==`, a short-circuiting comparison that's a timing side channel
//! on whatever secret comparison the program was trying to do.
//!
//! Runs through the real compiled path (`codegen::build` + execute), not
//! the interpreter — `constant_time_str_eq` is one of the builtins that
//! already compiles today (`nir_str_eq` in `crates/runtime-kernels`).

use std::process::Command;

use nirdosha::codegen;
use nirdosha::ownership::check_ownership;
use nirdosha::parser::Parser;
use nirdosha::token::Lexer;
use nirdosha::typeck::typecheck;

fn unique_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn compile_and_run_bool(src: &str) -> bool {
    let toks = Lexer::new(src).tokenize().expect("lex should succeed");
    let program = Parser::new(toks).parse_program().expect("parse should succeed");
    typecheck(&program).expect("should typecheck cleanly");
    check_ownership(&program).expect("should ownership-check cleanly");
    let report = nirdosha::smt::analyze(&program);
    let mut out_path = std::env::temp_dir();
    out_path.push(format!("nirdosha_test_streq_{}_{}", std::process::id(), unique_suffix()));
    codegen::build(&program, &report, &out_path, codegen::OptLevel::O2).expect("codegen::build should succeed");
    let status = Command::new(&out_path).status().expect("compiled binary should run");
    let _ = std::fs::remove_file(&out_path);
    match status.code() {
        Some(0) => false,
        _ => true, // bool true main-return sign-extends to i32 -1, exit code 255 -- any nonzero code means true
    }
}

#[test]
fn equal_strings_compare_equal() {
    let src = r#"
        fn main() -> bool {
            return constant_time_str_eq("same-token-value", "same-token-value")
        }
    "#;
    assert!(compile_and_run_bool(src));
}

#[test]
fn different_strings_compare_unequal() {
    let src = r#"
        fn main() -> bool {
            return constant_time_str_eq("token-a", "token-b")
        }
    "#;
    assert!(!compile_and_run_bool(src));
}

#[test]
fn different_length_strings_compare_unequal() {
    let src = r#"
        fn main() -> bool {
            return constant_time_str_eq("short", "much-longer-string")
        }
    "#;
    assert!(!compile_and_run_bool(src));
}

#[test]
fn empty_strings_compare_equal() {
    let src = r#"
        fn main() -> bool {
            return constant_time_str_eq("", "")
        }
    "#;
    assert!(compile_and_run_bool(src));
}
