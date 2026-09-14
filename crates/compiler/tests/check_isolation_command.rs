//! End-to-end tests for `nirdosha check-isolation` (`cmd_check_isolation`
//! in `main.rs`) -- the CLI enforcement surface `crates/runtime-kernels/
//! src/kernel/isolation_check.rs`'s own module doc has named as a real
//! gap since RFC 0016 Phase 2 landed: detection-only, no way to run the
//! checker over a saved log on demand. Same `std::process::Command`-
//! against-the-real-binary pattern `certify_command.rs`/`verify_verdict.rs`
//! use. The detector itself (`crates/isolation-core`) has its own,
//! lower-level unit tests; these confirm the CLI wiring around it --
//! JSON parsing, exit codes, the printed verdict shape.

fn unique_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn scratch_ops_log(name: &str, json: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("nirdosha_check_isolation_command_test_{}_{}_{name}.json", std::process::id(), unique_suffix()));
    std::fs::write(&p, json).expect("scratch ops-log file should write");
    p
}

fn run_check_isolation(path: &std::path::Path) -> (serde_json::Value, i32) {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha")).arg("check-isolation").arg(path).output().expect("nirdosha check-isolation should run");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("check-isolation should print valid JSON");
    let code = output.status.code().expect("process should exit with a status code, not be signal-killed");
    (value, code)
}

/// The `killer_demo` shape itself, as a saved log -- exactly the
/// pattern `examples/isolation_demo/` proves the live path catches;
/// this confirms the standalone, on-demand CLI path catches the
/// identical shape from a file, with the real exit code (`1`, matching
/// `verify`'s own `DISPROVED` convention -- an anomaly found is a real
/// failure, not a warning).
#[test]
fn a_lost_update_log_is_reported_as_anomaly_found_with_exit_1() {
    let log = scratch_ops_log(
        "lost_update",
        r#"[
            {"txn":"a","resource":"balance:acct1","kind":"read","seq":0},
            {"txn":"b","resource":"balance:acct1","kind":"read","seq":1},
            {"txn":"a","resource":"balance:acct1","kind":"write","seq":2},
            {"txn":"b","resource":"balance:acct1","kind":"write","seq":3}
        ]"#,
    );
    let (result, code) = run_check_isolation(&log);
    assert_eq!(result["verdict"], "anomaly_found", "result: {result}");
    assert_eq!(result["evidence_tier"], "monitored", "result: {result}");
    let anomalies = result["anomalies"].as_array().expect("anomalies should be an array");
    assert_eq!(anomalies.len(), 1, "result: {result}");
    assert_eq!(code, 1);
    let _ = std::fs::remove_file(&log);
}

/// A correctly-serialized history (`b` only reads after `a`'s write
/// completes) must report clean, exit `0` -- a detector that flags a
/// correctly-serialized history would be useless, the same "never cry
/// wolf" bar `isolation-core`'s own unit tests already hold it to.
#[test]
fn a_properly_serialized_log_is_reported_clean_with_exit_0() {
    let log = scratch_ops_log(
        "clean",
        r#"[
            {"txn":"a","resource":"balance:acct1","kind":"read","seq":0},
            {"txn":"a","resource":"balance:acct1","kind":"write","seq":1},
            {"txn":"b","resource":"balance:acct1","kind":"read","seq":2},
            {"txn":"b","resource":"balance:acct1","kind":"write","seq":3}
        ]"#,
    );
    let (result, code) = run_check_isolation(&log);
    assert_eq!(result["verdict"], "clean", "result: {result}");
    assert_eq!(result["anomalies"].as_array().unwrap().len(), 0, "result: {result}");
    assert_eq!(code, 0);
    let _ = std::fs::remove_file(&log);
}

/// An empty log (no ops at all) is the trivial clean case -- must not
/// panic or error, exit `0`.
#[test]
fn an_empty_log_is_clean() {
    let log = scratch_ops_log("empty", "[]");
    let (result, code) = run_check_isolation(&log);
    assert_eq!(result["verdict"], "clean", "result: {result}");
    assert_eq!(code, 0);
    let _ = std::fs::remove_file(&log);
}

/// Malformed JSON (not a valid `Op` array) must fail cleanly with a
/// real error message, not panic -- exit `1` (this command's own
/// generic failure code for a bad-input case, distinct from the `2`
/// `verify`/`certify` reserve for a genuinely inconclusive *proof*
/// verdict, since a malformed log was never checked at all).
#[test]
fn malformed_json_fails_cleanly_not_a_panic() {
    let log = scratch_ops_log("malformed", "this is not json at all");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha")).arg("check-isolation").arg(&log).output().expect("nirdosha check-isolation should run");
    assert!(!output.status.success(), "malformed input must fail the command, not silently succeed");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("valid ops-log JSON") || stderr.contains("error reading"), "stderr should name the real parse failure: {stderr}");
    let _ = std::fs::remove_file(&log);
}

/// A missing file is a clean error, not a panic.
#[test]
fn a_missing_log_file_fails_cleanly() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha")).arg("check-isolation").arg("/nonexistent/path/does/not/exist.json").output().expect("nirdosha check-isolation should run");
    assert!(!output.status.success());
}

/// No path argument at all -- usage error, not a panic.
#[test]
fn no_argument_prints_usage_and_fails() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha")).arg("check-isolation").output().expect("nirdosha check-isolation should run");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("usage:"), "stderr: {stderr}");
}

/// `--in-toto` wraps the same verdict as an in-toto v1 Statement --
/// same convention `verify --in-toto`/`certify --in-toto` already use.
#[test]
fn in_toto_wraps_the_verdict_as_a_statement() {
    let log = scratch_ops_log("in_toto", "[]");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha")).arg("check-isolation").arg(&log).arg("--in-toto").output().unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("should still be valid JSON");
    assert_eq!(value["_type"], "https://in-toto.io/Statement/v1", "value: {value}");
    assert_eq!(value["predicateType"], "https://nirdosha.dev/attestations/check-isolation/v1", "value: {value}");
    assert_eq!(value["predicate"]["verdict"], "clean", "value: {value}");
    let _ = std::fs::remove_file(&log);
}
