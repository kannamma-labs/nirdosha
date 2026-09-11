//! Tests for `nirdosha verify`'s three-valued verdict
//! (`ProofVerdict`/`cmd_verify` in `main.rs`): `PROVED`/`DISPROVED`/
//! `UNKNOWN`, never collapsed to a binary pass/fail, with a matching
//! three-way exit code (0/1/2) so a caller that only checks `$?` sees
//! the same distinction the JSON `verdict` field carries. Exercised by
//! spawning the real `nirdosha` binary against a scratch `.nir` file —
//! same `std::process::Command` pattern `crates/compiler/tests/init.rs`
//! and `emit_catalog.rs` already use for CLI-level behavior.

fn unique_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn scratch_file(name: &str, src: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("nirdosha_verify_verdict_test_{}_{}_{name}.nir", std::process::id(), unique_suffix()));
    std::fs::write(&p, src).expect("scratch .nir file should write");
    p
}

fn run_verify(path: &std::path::Path) -> (serde_json::Value, i32) {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha"))
        .arg("verify")
        .arg(path)
        .output()
        .expect("nirdosha verify should run");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("verify should print valid JSON");
    let code = output.status.code().expect("process should exit with a status code, not be signal-killed");
    (value, code)
}

#[test]
fn no_validate_blocks_at_all_is_proved_exit_0() {
    let path = scratch_file(
        "proved",
        r#"
fn add(a: i64, b: i64) -> i64 {
    return a + b
}
"#,
    );
    let (verdict, code) = run_verify(&path);
    assert_eq!(verdict["verdict"], "PROVED", "verdict: {verdict}");
    assert_eq!(verdict["contracts"]["verdict"], "PROVED", "verdict: {verdict}");
    assert_eq!(code, 0);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_genuinely_false_postcondition_is_disproved_exit_1_with_a_counterexample() {
    // result (a - 1) is never >= a for any i64 a, so Z3 finds a real
    // counterexample rather than merely failing to model the predicate
    // -- this is the "definitely wrong" case, distinct from "unknown".
    let path = scratch_file(
        "disproved",
        r#"
fn bad(a: i64) -> i64 {
    return a - 1
}

validate bad {
    post: result >= a
}
"#,
    );
    let (verdict, code) = run_verify(&path);
    assert_eq!(verdict["verdict"], "DISPROVED", "verdict: {verdict}");
    assert_eq!(verdict["contracts"]["verdict"], "DISPROVED", "verdict: {verdict}");
    assert_eq!(verdict["contracts"]["failed"], 1, "verdict: {verdict}");
    let obligations = verdict["contracts"]["obligations"].as_array().expect("obligations should be an array");
    assert_eq!(obligations[0]["status"], "counterexample", "verdict: {verdict}");
    assert!(
        obligations[0]["detail"].as_str().expect("counterexample should have a detail string").contains("a = "),
        "counterexample detail should embed the binding that violates it: {verdict}"
    );
    assert_eq!(code, 1);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_non_integer_parameter_z3_cannot_model_is_unknown_exit_2_not_proved() {
    // `bool` is a real, typechecking parameter type Tier 1 doesn't
    // model (`contract_check.rs`'s `is_integer` gate) -- this must
    // report `UNKNOWN`, not be silently folded into `PROVED` the way an
    // earlier revision of this verdict schema did.
    let path = scratch_file(
        "unknown",
        r#"
fn flip(x: bool) -> bool {
    return x
}

validate flip {
    post: true
}
"#,
    );
    let (verdict, code) = run_verify(&path);
    assert_eq!(verdict["verdict"], "UNKNOWN", "verdict: {verdict}");
    assert_eq!(verdict["contracts"]["verdict"], "UNKNOWN", "verdict: {verdict}");
    assert_eq!(verdict["contracts"]["unsupported"], 1, "verdict: {verdict}");
    assert_eq!(verdict["contracts"]["failed"], 0, "an unmodeled obligation is not a counterexample: {verdict}");
    let obligations = verdict["contracts"]["obligations"].as_array().expect("obligations should be an array");
    assert_eq!(obligations[0]["status"], "unsupported", "verdict: {verdict}");
    assert_eq!(code, 2, "UNKNOWN must be its own exit code, distinguishable from both 0 and 1");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_typecheck_error_is_disproved_not_unknown() {
    // A hard pipeline failure is definite, not uncertain -- there is no
    // sense in which "this doesn't typecheck" is `UNKNOWN`.
    let path = scratch_file(
        "typecheck_fail",
        r#"
fn broken() -> i64 {
    return "not an i64"
}
"#,
    );
    let (verdict, code) = run_verify(&path);
    assert_eq!(verdict["verdict"], "DISPROVED", "verdict: {verdict}");
    assert_eq!(verdict["typecheck"]["status"], "failed", "verdict: {verdict}");
    assert_eq!(verdict["ownership"]["status"], "skipped", "a stage after a hard failure must be skipped, not passed: {verdict}");
    assert_eq!(code, 1);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn failed_beats_unsupported_when_a_file_has_both() {
    // A concrete counterexample is more actionable than "couldn't
    // decide" -- if a file has one obligation Z3 can disprove and a
    // separate one it can't model at all, the overall verdict is
    // `DISPROVED`, not `UNKNOWN`, so a repair loop gets the strongest
    // available signal.
    let path = scratch_file(
        "mixed",
        r#"
fn bad(a: i64) -> i64 {
    return a - 1
}

validate bad {
    post: result >= a
}

fn flip(x: bool) -> bool {
    return x
}

validate flip {
    post: true
}
"#,
    );
    let (verdict, code) = run_verify(&path);
    assert_eq!(verdict["verdict"], "DISPROVED", "verdict: {verdict}");
    assert_eq!(verdict["contracts"]["failed"], 1, "verdict: {verdict}");
    assert_eq!(verdict["contracts"]["unsupported"], 1, "verdict: {verdict}");
    assert_eq!(code, 1);
    let _ = std::fs::remove_file(&path);
}
