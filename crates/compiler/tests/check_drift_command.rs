//! End-to-end tests for `nirdosha check-drift` (`cmd_check_drift` in
//! `main.rs`) -- Phase 4 item 1 of `docs/research/2026-09-pending-
//! verification-differentiation-work.md`: "detect when a compile-time
//! `nfr(...)` assumption diverges from what the APM kernel actually
//! observes in production, and trigger re-verification instead of just
//! firing an alert." Same `std::process::Command`-against-the-real-
//! binary pattern `certify_command.rs`/`check_isolation_command.rs` use.

fn unique_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn scratch_file(name: &str, ext: &str, content: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("nirdosha_check_drift_command_test_{}_{}_{name}.{ext}", std::process::id(), unique_suffix()));
    std::fs::write(&p, content).expect("scratch file should write");
    p
}

const NFR_SOURCE: &str = "fn slow_lookup(x: i64) -> i64 nfr(latency_ms: 50, concurrency_max: 10) {\n    return x + 1\n}\n\nfn main() {\n    print(slow_lookup(1))\n}\n";

fn run_check_drift(nir_path: &std::path::Path, log_path: &std::path::Path, extra: &[&str]) -> (serde_json::Value, i32) {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha"));
    cmd.arg("check-drift").arg(nir_path).arg(log_path);
    for a in extra {
        cmd.arg(a);
    }
    let output = cmd.output().expect("nirdosha check-drift should run");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("check-drift should print valid JSON: {stdout}");
    let code = output.status.code().expect("process should exit with a status code, not be signal-killed");
    (value, code)
}

/// The core claim: a real observed escalation for a declared `nfr(...)`
/// commitment is reported as drift, AND real re-verification actually
/// ran (not just an alert with nothing behind it) -- `reverification`
/// carries a real `PROVED` verdict for this fixture.
#[test]
fn a_real_escalation_against_a_declared_commitment_is_reported_as_drift_and_triggers_reverification() {
    let nir = scratch_file("drift_source", "nir", NFR_SOURCE);
    let log = scratch_file(
        "drift_log",
        "json",
        r#"[{"function":"slow_lookup","nfr":"latency_ms","threshold":50,"actual":210,"timestamp_ms":1}]"#,
    );
    let (result, code) = run_check_drift(&nir, &log, &[]);
    assert_eq!(result["verdict"], "drift_found", "result: {result}");
    let findings = result["findings"].as_array().expect("findings should be an array");
    assert_eq!(findings.len(), 1, "result: {result}");
    assert_eq!(findings[0]["fn_name"], "slow_lookup");
    assert_eq!(findings[0]["nfr_kind"], "latency_ms");
    assert_eq!(findings[0]["worst_observed"], 210.0);
    assert!(!result["reverification"].is_null(), "a real drift finding must trigger real re-verification: {result}");
    assert_eq!(result["reverification"]["verdict"], "PROVED", "result: {result}");
    assert_eq!(code, 1);
    let _ = std::fs::remove_file(&nir);
    let _ = std::fs::remove_file(&log);
}

/// No escalations at all -- clean, no re-verification triggered (there
/// is nothing to re-verify against), exit 0.
#[test]
fn no_escalations_is_no_drift_and_does_not_trigger_reverification() {
    let nir = scratch_file("clean_source", "nir", NFR_SOURCE);
    let log = scratch_file("clean_log", "json", "[]");
    let (result, code) = run_check_drift(&nir, &log, &[]);
    assert_eq!(result["verdict"], "no_drift", "result: {result}");
    assert_eq!(result["findings"].as_array().unwrap().len(), 0);
    assert!(result["reverification"].is_null(), "no drift means nothing to re-verify: {result}");
    assert_eq!(code, 0);
    let _ = std::fs::remove_file(&nir);
    let _ = std::fs::remove_file(&log);
}

/// An escalation naming a function/nfr-kind combination this source
/// never declared must not be reported as drift against a commitment
/// that doesn't exist.
#[test]
fn an_escalation_for_an_undeclared_commitment_is_not_drift() {
    let nir = scratch_file("undeclared_source", "nir", NFR_SOURCE);
    let log = scratch_file(
        "undeclared_log",
        "json",
        r#"[{"function":"some_other_fn","nfr":"latency_ms","threshold":50,"actual":999,"timestamp_ms":1}]"#,
    );
    let (result, code) = run_check_drift(&nir, &log, &[]);
    assert_eq!(result["verdict"], "no_drift", "result: {result}");
    assert_eq!(code, 0);
    let _ = std::fs::remove_file(&nir);
    let _ = std::fs::remove_file(&log);
}

/// `--teach` records a runtime lesson only when drift is actually
/// found, and it's readable back via `NIRDOSHA_RUNTIME_LESSONS_PATH`.
#[test]
fn teach_records_a_runtime_lesson_only_on_a_real_finding() {
    let nir = scratch_file("teach_source", "nir", NFR_SOURCE);
    let log = scratch_file(
        "teach_log",
        "json",
        r#"[{"function":"slow_lookup","nfr":"latency_ms","threshold":50,"actual":210,"timestamp_ms":1}]"#,
    );
    let lessons_path = std::env::temp_dir().join(format!("nirdosha_check_drift_teach_test_{}_{}.json", std::process::id(), unique_suffix()));

    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha"));
    cmd.arg("check-drift").arg(&nir).arg(&log).arg("--teach").arg("watch slow_lookup under real load").env("NIRDOSHA_RUNTIME_LESSONS_PATH", &lessons_path);
    let output = cmd.output().expect("nirdosha check-drift --teach should run");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let result: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(result["taught"], serde_json::json!(true), "result: {result}");

    let lessons: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&lessons_path).expect("the runtime lessons file should exist")).expect("valid JSON");
    assert_eq!(lessons["nfr_drift"], serde_json::json!("watch slow_lookup under real load"), "lessons: {lessons}");

    let _ = std::fs::remove_file(&nir);
    let _ = std::fs::remove_file(&log);
    let _ = std::fs::remove_file(&lessons_path);
}

/// Malformed escalations JSON fails cleanly.
#[test]
fn malformed_escalations_json_fails_cleanly() {
    let nir = scratch_file("malformed_source", "nir", NFR_SOURCE);
    let log = scratch_file("malformed_log", "json", "not json");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha")).arg("check-drift").arg(&nir).arg(&log).output().unwrap();
    assert!(!output.status.success());
    let _ = std::fs::remove_file(&nir);
    let _ = std::fs::remove_file(&log);
}

/// Missing arguments print usage and fail, not panic.
#[test]
fn missing_arguments_print_usage_and_fail() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha")).arg("check-drift").output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("usage:"), "stderr: {stderr}");
}
