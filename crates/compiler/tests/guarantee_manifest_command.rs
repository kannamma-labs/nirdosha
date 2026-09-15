//! End-to-end tests for RFC 0017's CLI surface -- `nirdosha check-
//! guarantees`, `nirdosha build`'s emitted `<out>.guarantees.json`
//! bundle and its build-time hard failure on a real violation, and
//! `nirdosha verify-binary`. Same `std::process::Command`-against-the-
//! real-binary pattern `check_isolation_command.rs`/`certify_command.rs`
//! use -- `guarantee_manifest.rs` has its own lower-level unit tests;
//! these confirm the CLI wiring around it against a real compiled
//! binary, not just the pure functions.

fn unique_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn scratch_path(name: &str, ext: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("nirdosha_guarantee_manifest_command_test_{}_{}_{name}.{ext}", std::process::id(), unique_suffix()));
    p
}

fn write_source(name: &str, src: &str) -> std::path::PathBuf {
    let p = scratch_path(name, "nir");
    std::fs::write(&p, src).expect("scratch source file should write");
    p
}

const SOURCE: &str = "fn main() requires(public) {\n    print(\"hi\")\n}\n";

/// No `<file.nir>.guarantees.json` at all -- RFC 0017's own "additive
/// only" compatibility rule: reported clean (`no_manifest`), never an
/// error, exit 0.
#[test]
fn check_guarantees_with_no_manifest_present_is_clean() {
    let src = write_source("no_manifest", SOURCE);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha")).arg("check-guarantees").arg(&src).output().expect("nirdosha check-guarantees should run");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let result: serde_json::Value = serde_json::from_str(&stdout).expect("should print valid JSON");
    assert_eq!(result["verdict"], "no_manifest", "result: {result}");
    assert!(output.status.success());
    let _ = std::fs::remove_file(&src);
}

/// A real manifest the source actually satisfies -- clean, exit 0.
#[test]
fn check_guarantees_reports_clean_when_the_source_satisfies_the_manifest() {
    let src = write_source("clean", SOURCE);
    let manifest_path = format!("{}.guarantees.json", src.display());
    std::fs::write(&manifest_path, r#"{"capabilities":{"allowed_effects":["io"]},"exports":{"main":{"public":true}}}"#).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha")).arg("check-guarantees").arg(&src).output().expect("should run");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let result: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(result["verdict"], "clean", "result: {result}");
    assert!(output.status.success());
    let _ = std::fs::remove_file(&src);
    let _ = std::fs::remove_file(&manifest_path);
}

/// A manifest the source actually violates (forbids the `io` effect
/// `print` really performs) -- exit 1, a real, named violation, not
/// just a generic failure.
#[test]
fn check_guarantees_reports_a_real_capability_ceiling_violation() {
    let src = write_source("violation", SOURCE);
    let manifest_path = format!("{}.guarantees.json", src.display());
    std::fs::write(&manifest_path, r#"{"capabilities":{"forbidden_effects":["io"]}}"#).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha")).arg("check-guarantees").arg(&src).output().expect("should run");
    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let result: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(result["verdict"], "violation_found", "result: {result}");
    let violations = result["violations"].as_array().expect("violations should be an array");
    assert_eq!(violations.len(), 1, "result: {result}");
    assert!(violations[0].as_str().unwrap().contains("io"), "violations: {violations:?}");
    let _ = std::fs::remove_file(&src);
    let _ = std::fs::remove_file(&manifest_path);
}

/// `nirdosha build` always emits `<out>.guarantees.json`, manifest or
/// not -- RFC §4's bundle. Real compiled output, not a mock.
#[test]
fn build_always_emits_a_guarantee_bundle() {
    let src = write_source("build_bundle", SOURCE);
    let out = scratch_path("build_bundle_bin", "out");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha")).arg("build").arg(&src).arg("-o").arg(&out).output().expect("nirdosha build should run");
    assert!(output.status.success(), "build should succeed: {}", String::from_utf8_lossy(&output.stderr));
    let bundle_path = format!("{}.guarantees.json", out.display());
    let bundle: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&bundle_path).expect("the bundle file should exist")).expect("valid JSON");
    assert_eq!(bundle["inferred_effects"]["main"], serde_json::json!(["io"]), "bundle: {bundle}");
    assert_eq!(bundle["public_exports"], serde_json::json!(["main"]), "bundle: {bundle}");
    assert!(bundle["source_hash"].as_str().unwrap().starts_with("sha256:"), "bundle: {bundle}");
    let _ = std::fs::remove_file(&src);
    let _ = std::fs::remove_file(&out);
    let _ = std::fs::remove_file(&bundle_path);
}

/// A real guarantee-manifest violation fails the *build itself*, not
/// just `check-guarantees` -- RFC 0017's own "hard error" semantics.
/// No binary or bundle is written for a build that fails this gate.
#[test]
fn build_fails_hard_on_a_real_guarantee_violation() {
    let src = write_source("build_violation", SOURCE);
    let manifest_path = format!("{}.guarantees.json", src.display());
    std::fs::write(&manifest_path, r#"{"capabilities":{"forbidden_effects":["io"]}}"#).unwrap();
    let out = scratch_path("build_violation_bin", "out");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha")).arg("build").arg(&src).arg("-o").arg(&out).output().expect("should run");
    assert!(!output.status.success(), "a real guarantee violation must fail the build");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("guarantee-manifest violation"), "stderr should name the real gate that failed: {stderr}");
    assert!(!out.exists(), "no binary should be written when the guarantee gate fails");
    let _ = std::fs::remove_file(&src);
    let _ = std::fs::remove_file(&manifest_path);
}

/// `verify-binary` checks an already-built bundle against an operator
/// policy with no recompilation -- both a satisfying and a violating
/// policy against the same real bundle.
#[test]
fn verify_binary_checks_a_real_bundle_against_an_operator_policy_both_ways() {
    let src = write_source("verify_binary", SOURCE);
    let out = scratch_path("verify_binary_bin", "out");
    let build_output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha")).arg("build").arg(&src).arg("-o").arg(&out).output().expect("build should run");
    assert!(build_output.status.success(), "build should succeed: {}", String::from_utf8_lossy(&build_output.stderr));
    let bundle_path = format!("{}.guarantees.json", out.display());

    let ok_policy_path = scratch_path("ok_policy", "json");
    std::fs::write(&ok_policy_path, r#"{"capabilities":{"allowed_effects":["io"]}}"#).unwrap();
    let ok_output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha")).arg("verify-binary").arg(&bundle_path).arg("--against").arg(&ok_policy_path).output().expect("should run");
    assert!(ok_output.status.success(), "the bundle really only performs io, so this policy must be satisfied: {}", String::from_utf8_lossy(&ok_output.stdout));
    let ok_result: serde_json::Value = serde_json::from_str(&String::from_utf8(ok_output.stdout).unwrap()).unwrap();
    assert_eq!(ok_result["verdict"], "satisfies_policy", "result: {ok_result}");

    let bad_policy_path = scratch_path("bad_policy", "json");
    std::fs::write(&bad_policy_path, r#"{"capabilities":{"forbidden_effects":["io"]}}"#).unwrap();
    let bad_output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha")).arg("verify-binary").arg(&bundle_path).arg("--against").arg(&bad_policy_path).output().expect("should run");
    assert!(!bad_output.status.success(), "the bundle really does perform the forbidden io effect");
    let bad_result: serde_json::Value = serde_json::from_str(&String::from_utf8(bad_output.stdout).unwrap()).unwrap();
    assert_eq!(bad_result["verdict"], "violation_found", "result: {bad_result}");

    let _ = std::fs::remove_file(&src);
    let _ = std::fs::remove_file(&out);
    let _ = std::fs::remove_file(&bundle_path);
    let _ = std::fs::remove_file(&ok_policy_path);
    let _ = std::fs::remove_file(&bad_policy_path);
}

/// A missing bundle/policy file is a clean error, not a panic.
#[test]
fn verify_binary_with_a_missing_bundle_fails_cleanly() {
    let policy_path = scratch_path("policy_for_missing_bundle", "json");
    std::fs::write(&policy_path, "{}").unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha")).arg("verify-binary").arg("/nonexistent/does/not/exist.guarantees.json").arg("--against").arg(&policy_path).output().expect("should run");
    assert!(!output.status.success());
    let _ = std::fs::remove_file(&policy_path);
}

/// No arguments at all -- usage error, not a panic.
#[test]
fn check_guarantees_with_no_argument_prints_usage_and_fails() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha")).arg("check-guarantees").output().expect("should run");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("usage:"), "stderr: {stderr}");
}

#[test]
fn verify_binary_with_no_against_flag_prints_usage_and_fails() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha")).arg("verify-binary").arg("some_bundle.json").output().expect("should run");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("usage:"), "stderr: {stderr}");
}
