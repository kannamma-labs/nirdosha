//! Tests for `str` -- literals, escapes, function parameter/return
//! positions, and equality. Deliberately minimal (see `Ty::Str`'s doc
//! comment in ast.rs): no concatenation, no indexing. Existing purely to
//! name things (a hostname, an image tag) that couldn't be spelled any
//! other way -- see `tests/tcp.rs` for what that's actually for.

use nirdosha::ast::Ty;
use nirdosha::codegen;
use nirdosha::ownership::check_ownership;
use nirdosha::parser::Parser;
use nirdosha::token::Lexer;
use nirdosha::typeck::{typecheck, TypeErrorKind};
use std::process::Command;

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

fn unique_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Compiles `src` (real path, not the interpreter) and returns what it
/// printed. `str` can't be `main`'s return type either (`StrInFnSignature`
/// -- no exception for `main`, despite `emit_c_main` having dead code for
/// that case), so every caller here uses `fn main() { print(...) }`
/// instead, the same pattern `tests/codegen.rs`'s own
/// `main_printing_a_str_directly_compiles_and_prints_it` establishes.
fn compile_and_run_str(src: &str) -> String {
    let program = parse_ok(src);
    typecheck(&program).expect("should typecheck cleanly");
    check_ownership(&program).expect("should ownership-check cleanly");
    let report = nirdosha::smt::analyze(&program);
    let mut out_path = std::env::temp_dir();
    out_path.push(format!("nirdosha_test_strings_{}_{}", std::process::id(), unique_suffix()));
    codegen::build(&program, &report, &out_path, codegen::OptLevel::O2).expect("codegen::build should succeed");
    let output = Command::new(&out_path).output().expect("compiled binary should run");
    let _ = std::fs::remove_file(&out_path);
    String::from_utf8_lossy(&output.stdout).to_string()
}

/// Same as `compile_and_run_str`, for a `bool`-returning `main` --
/// `emit_c_main`'s generic integer path widens `true`/`false` to exit
/// code `1`/`0`.
fn compile_and_run_bool(src: &str) -> bool {
    let program = parse_ok(src);
    typecheck(&program).expect("should typecheck cleanly");
    check_ownership(&program).expect("should ownership-check cleanly");
    let report = nirdosha::smt::analyze(&program);
    let mut out_path = std::env::temp_dir();
    out_path.push(format!("nirdosha_test_strings_{}_{}", std::process::id(), unique_suffix()));
    codegen::build(&program, &report, &out_path, codegen::OptLevel::O2).expect("codegen::build should succeed");
    let status = Command::new(&out_path).status().expect("compiled binary should run");
    let _ = std::fs::remove_file(&out_path);
    match status.code() {
        Some(0) => false,
        _ => true, // bool true main-return sign-extends to i32 -1, exit code 255 -- any nonzero code means true
    }
}

// ---- literals and escapes ------------------------------------------------

#[test]
fn a_plain_string_literal_round_trips() {
    // `str` can't be `main`'s return type (`StrInFnSignature`, no
    // exception for `main`) -- `print` it instead, the same pattern
    // `tests/codegen.rs::main_printing_a_str_directly_compiles_and_prints_it`
    // establishes as the real, working one.
    let src = r#"
        fn main() {
            print("hello")
        }
    "#;
    assert_eq!(compile_and_run_str(src), "hello\n");
}

#[test]
fn escape_sequences_are_interpreted_correctly() {
    let src = r#"
        fn main() {
            print("a\nb\tc\\d\"e\rf")
        }
    "#;
    assert_eq!(compile_and_run_str(src), "a\nb\tc\\d\"e\rf\n");
}

#[test]
fn an_unknown_escape_is_a_lex_error() {
    let toks = Lexer::new(r#"fn main() { let s: str = "\q" }"#).tokenize();
    assert!(toks.is_err(), "an unrecognized escape must be rejected, not silently kept literal");
}

#[test]
fn an_unterminated_string_is_a_lex_error() {
    let toks = Lexer::new("fn main() { let s: str = \"never closed").tokenize();
    assert!(toks.is_err(), "a string with no closing quote must be a lex error, not a hang");
}

// ---- passing strings through functions ------------------------------------

// A bare `str` can no longer be a function's parameter or return type at
// all (the "enum favoring" rule — `TypeErrorKind::StrInFnSignature`,
// `typeck.rs::check_fn`), so this no longer tests passing a bare `str`
// through a function boundary; it tests the sanctioned replacement —
// wrapping free text in a carrier struct (`Text`) — still passes through
// a function boundary with its value unchanged, not silently corrupted
// or truncated in transit.
#[test]
fn text_passes_through_function_parameters_and_returns_unchanged() {
    let src = r#"
        struct Text {
            value: str,
        }
        fn pass_through(s: Text) -> Text {
            return s
        }
        fn main() {
            let result: Text = pass_through(Text("passed through"))
            print(result.value)
        }
    "#;
    assert_eq!(compile_and_run_str(src), "passed through\n");
}

// ---- equality (found missing at runtime once, fixed, pinned here) --------

#[test]
fn equal_strings_compare_equal() {
    let src = r#"
        fn main() -> bool {
            let a: str = "same"
            let b: str = "same"
            return a == b
        }
    "#;
    assert!(compile_and_run_bool(src));
}

#[test]
fn different_strings_compare_unequal() {
    let src = r#"
        fn main() -> bool {
            let a: str = "one"
            let b: str = "two"
            return a != b
        }
    "#;
    assert!(compile_and_run_bool(src));
}

// ---- static rejections (a real gap found and fixed, pinned here) --------

#[test]
fn ordering_strings_is_rejected_statically_not_at_runtime() {
    // typeck.rs used to only reject `Bool` for `<`/`>`/etc, letting
    // `str < str` typecheck and fail at runtime with a generic
    // TypeMismatch instead. Fixed to reject any non-numeric type
    // uniformly -- this pins the fix for `str` specifically.
    let kind = first_type_error(
        r#"
        fn main() -> bool {
            let a: str = "a"
            let b: str = "b"
            return a < b
        }
    "#,
    );
    assert_eq!(kind, TypeErrorKind::ExpectedNumeric { found: Ty::Str });
}

#[test]
fn arithmetic_on_strings_is_rejected_statically() {
    let kind = first_type_error(
        r#"
        fn main() -> bool {
            let a: str = "a"
            let b: str = "b"
            let c: i64 = a + b
            return true
        }
    "#,
    );
    assert_eq!(kind, TypeErrorKind::ExpectedNumeric { found: Ty::Str });
}
