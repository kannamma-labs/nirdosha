//! End-to-end tests for `nirdosha equivalence`
//! (`cmd_equivalence`/`contract_check::check_equivalence` in
//! `main.rs`/`contract_check.rs`, `nirdosha-master-plan.md` Part 3
//! Dec 2026's "Equivalence checking", parity target: Velvet, Imandra).
//! Same `std::process::Command`-against-the-real-binary pattern
//! `certify_command.rs`/`verify_verdict.rs` use.

fn unique_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn scratch_file(name: &str, src: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("nirdosha_equivalence_command_test_{}_{}_{name}.nir", std::process::id(), unique_suffix()));
    std::fs::write(&p, src).expect("scratch .nir file should write");
    p
}

fn run_equivalence(path: &std::path::Path, fn_a: &str, fn_b: &str) -> (serde_json::Value, i32) {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha"))
        .arg("equivalence")
        .arg(path)
        .arg(fn_a)
        .arg(fn_b)
        .output()
        .expect("nirdosha equivalence should run");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("equivalence should print valid JSON");
    let code = output.status.code().expect("process should exit with a status code, not be signal-killed");
    (value, code)
}

#[test]
fn two_differently_written_but_behaviorally_identical_functions_are_equivalent() {
    // max via `a > b` vs `a >= b` -- the tie-break differs internally
    // (which branch fires when a == b) but the *value* returned is
    // identical either way, a real refactor-preserves-behavior case.
    let path = scratch_file(
        "equivalent",
        r#"
fn max_v1(a: i64, b: i64) -> i64 {
    return if a > b { a } else { b }
}

fn max_v2(a: i64, b: i64) -> i64 {
    return if a >= b { a } else { b }
}
"#,
    );
    let (value, code) = run_equivalence(&path, "max_v1", "max_v2");
    assert_eq!(value["result"], "EQUIVALENT", "value: {value}");
    assert_eq!(code, 0);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_real_refactor_bug_is_caught_with_a_concrete_counterexample() {
    let path = scratch_file(
        "buggy_refactor",
        r#"
fn max_v1(a: i64, b: i64) -> i64 {
    return if a > b { a } else { b }
}

fn max_buggy(a: i64, b: i64) -> i64 {
    return if a > b { a } else { a }
}
"#,
    );
    let (value, code) = run_equivalence(&path, "max_v1", "max_buggy");
    assert_eq!(value["result"], "DIFFERENT", "value: {value}");
    assert_eq!(code, 1);
    let a = value["counterexample"]["a"].as_i64().expect("counterexample should bind `a`");
    let b = value["counterexample"]["b"].as_i64().expect("counterexample should bind `b`");
    // Re-derive both functions' real behavior at the reported inputs and
    // confirm the JSON's own result_a/result_b are the actual values,
    // not placeholders -- max_v1 is the real max, max_buggy always
    // returns `a` unless a > b.
    let expected_a = a.max(b);
    let expected_b = if a > b { a } else { a };
    assert_eq!(value["result_a"], expected_a, "value: {value}");
    assert_eq!(value["result_b"], expected_b, "value: {value}");
    assert_ne!(expected_a, expected_b, "the counterexample must be a real point of divergence: {value}");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_let_binding_is_an_honest_unsupported_not_a_wrong_answer() {
    let path = scratch_file(
        "unsupported_let",
        r#"
fn double_v1(a: i64) -> i64 {
    return a + a
}

fn double_v2(a: i64) -> i64 {
    let b: i64 = a
    return b + a
}
"#,
    );
    let (value, code) = run_equivalence(&path, "double_v1", "double_v2");
    assert_eq!(value["result"], "UNSUPPORTED", "value: {value}");
    assert!(value["detail"].as_str().expect("detail should be a string").contains("double_v2"), "value: {value}");
    assert_eq!(code, 2);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn mismatched_parameter_counts_are_unsupported() {
    let path = scratch_file(
        "mismatched_arity",
        r#"
fn f(a: i64) -> i64 {
    return a
}

fn g(a: i64, b: i64) -> i64 {
    return a + b
}
"#,
    );
    let (value, code) = run_equivalence(&path, "f", "g");
    assert_eq!(value["result"], "UNSUPPORTED", "value: {value}");
    assert_eq!(code, 2);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn an_unknown_function_name_is_unsupported_not_a_crash() {
    let path = scratch_file("no_such_fn", "fn f(a: i64) -> i64 {\n    return a\n}\n");
    let (value, code) = run_equivalence(&path, "f", "does_not_exist");
    assert_eq!(value["result"], "UNSUPPORTED", "value: {value}");
    assert!(value["detail"].as_str().expect("detail should be a string").contains("does_not_exist"), "value: {value}");
    assert_eq!(code, 2);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn identical_bodies_are_trivially_equivalent() {
    let path = scratch_file(
        "identical",
        r#"
fn f(a: i64, b: i64) -> i64 {
    return a + b
}

fn g(x: i64, y: i64) -> i64 {
    return x + y
}
"#,
    );
    let (value, code) = run_equivalence(&path, "f", "g");
    assert_eq!(value["result"], "EQUIVALENT", "value: {value}");
    assert_eq!(code, 0);
    let _ = std::fs::remove_file(&path);
}
