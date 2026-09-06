//! Tests for `sha256_hex` — a plain SHA-256 hex-digest builtin. Added to
//! make a real hash-chained audit log possible from Nirdosha source (a
//! `str` has no `+`/concatenation, `docs/LANGUAGE.md` §2, so the 2-arg
//! form does the combining in Rust instead:
//! `sha256_hex(prev_hash, payload)`).
//!
//! Runs each program through the real compiled path (`codegen::build` +
//! execute), not the interpreter — `sha256_hex` is one of the small set
//! of builtins that already compiles today (`nir_sha256_hex` in
//! `crates/runtime-kernels`). `compile_and_run`/`compile_and_run_bool`
//! mirror `tests/codegen.rs`'s own helpers of the same shape.

use std::process::Command;

use nirdosha::codegen;
use nirdosha::ownership::check_ownership;
use nirdosha::parser::Parser;
use nirdosha::smt::analyze;
use nirdosha::token::Lexer;
use nirdosha::typeck::typecheck;

fn unique_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Compiles a `str`-returning `main` and returns what it printed
/// (`emit_c_main`'s own convention for a `str` result: printf then exit 0
/// — there's no sensible integer exit code for a `str`).
fn compile_and_run_str(src: &str) -> String {
    let toks = Lexer::new(src).tokenize().expect("lex should succeed");
    let program = Parser::new(toks).parse_program().expect("parse should succeed");
    typecheck(&program).expect("should typecheck cleanly");
    check_ownership(&program).expect("should ownership-check cleanly");
    let report = analyze(&program);
    let mut out_path = std::env::temp_dir();
    out_path.push(format!("nirdosha_test_sha256_{}_{}", std::process::id(), unique_suffix()));
    codegen::build(&program, &report, &out_path, codegen::OptLevel::O2).expect("codegen::build should succeed");
    let output = Command::new(&out_path).output().expect("compiled binary should run");
    let _ = std::fs::remove_file(&out_path);
    String::from_utf8_lossy(&output.stdout).to_string()
}

/// Compiles a `bool`-returning `main` and returns its truth value via the
/// process exit code (`emit_c_main`'s generic integer path: `true`/`false`
/// widen to `i32` `1`/`0`).
fn compile_and_run_bool(src: &str) -> bool {
    let toks = Lexer::new(src).tokenize().expect("lex should succeed");
    let program = Parser::new(toks).parse_program().expect("parse should succeed");
    typecheck(&program).expect("should typecheck cleanly");
    check_ownership(&program).expect("should ownership-check cleanly");
    let report = analyze(&program);
    let mut out_path = std::env::temp_dir();
    out_path.push(format!("nirdosha_test_sha256_{}_{}", std::process::id(), unique_suffix()));
    codegen::build(&program, &report, &out_path, codegen::OptLevel::O2).expect("codegen::build should succeed");
    let status = Command::new(&out_path).status().expect("compiled binary should run");
    let _ = std::fs::remove_file(&out_path);
    match status.code() {
        Some(0) => false,
        _ => true, // bool true main-return sign-extends to i32 -1, exit code 255 -- any nonzero code means true
    }
}

#[test]
fn hashes_a_known_string_to_the_known_digest() {
    // sha256("") -- a standard, independently-verifiable test vector.
    // `str` can't be `main`'s return type (`StrInFnSignature`, no
    // exception for `main`), so `print` it instead -- the same pattern
    // `tests/codegen.rs::main_printing_a_str_directly_compiles_and_prints_it`
    // already establishes as the real, working one.
    let src = r#"
        fn main() {
            print(sha256_hex(""))
        }
    "#;
    assert_eq!(compile_and_run_str(src), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\n");
}

#[test]
fn is_deterministic_and_input_sensitive() {
    let src = r#"
        fn main() -> bool {
            let a: str = sha256_hex("hello")
            let b: str = sha256_hex("hello")
            let c: str = sha256_hex("hellO")
            return a == b && a != c
        }
    "#;
    assert!(compile_and_run_bool(src));
}

#[test]
fn two_arg_form_hashes_both_parts_without_needing_concatenation() {
    // Locks in the actual tamper-evidence property a hash chain needs:
    // swapping either the previous hash or the payload changes the result.
    let src = r#"
        fn main() -> bool {
            let genesis: str = sha256_hex("genesis")
            let h1: str = sha256_hex(genesis, "entry one")
            let h1_again: str = sha256_hex(genesis, "entry one")
            let h1_tampered_payload: str = sha256_hex(genesis, "entry ONE")
            let h1_tampered_prev: str = sha256_hex(sha256_hex("different genesis"), "entry one")
            return h1 == h1_again && h1 != h1_tampered_payload && h1 != h1_tampered_prev
        }
    "#;
    assert!(compile_and_run_bool(src), "hash chain must be deterministic and sensitive to both prev_hash and payload");
}
