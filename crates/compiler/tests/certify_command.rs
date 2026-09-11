//! End-to-end tests for `nirdosha certify` (`cmd_certify`/`build_certificate`
//! in `main.rs`, `nirdosha-master-plan.md` Part 3 Sprint 1's Certificate
//! v0, parity target: Velvet, Kōdo). Same `std::process::Command`-
//! against-the-real-binary pattern `verify_verdict.rs`/`fix_command.rs`
//! use.

use sha2::{Digest, Sha256};

fn unique_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn scratch_file(name: &str, src: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("nirdosha_certify_command_test_{}_{}_{name}.nir", std::process::id(), unique_suffix()));
    std::fs::write(&p, src).expect("scratch .nir file should write");
    p
}

fn run_certify(path: &std::path::Path) -> (serde_json::Value, i32) {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha"))
        .arg("certify")
        .arg(path)
        .output()
        .expect("nirdosha certify should run");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("certify should print valid JSON");
    let code = output.status.code().expect("process should exit with a status code, not be signal-killed");
    (value, code)
}

#[test]
fn a_proved_file_gets_evidence_tier_proved_and_exit_0() {
    let path = scratch_file("proved", "fn add(a: i64, b: i64) -> i64 {\n    return a + b\n}\n");
    let (cert, code) = run_certify(&path);
    assert_eq!(cert["certificate_version"], "0", "cert: {cert}");
    assert_eq!(cert["verdict_summary"]["verdict"], "PROVED", "cert: {cert}");
    assert_eq!(cert["evidence_tier"], "proved", "cert: {cert}");
    assert_eq!(code, 0);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_disproved_file_still_gets_a_certificate_with_evidence_tier_proved() {
    // A conclusive Z3 counterexample is still formal, conclusive
    // evidence -- evidence_tier describes the *kind* of evidence, not
    // whether the outcome was a pass.
    let src = "fn bad(a: i64) -> i64 {\n    return a - 1\n}\n\nvalidate bad {\n    post: result >= a\n}\n";
    let path = scratch_file("disproved", src);
    let (cert, code) = run_certify(&path);
    assert_eq!(cert["verdict_summary"]["verdict"], "DISPROVED", "cert: {cert}");
    assert_eq!(cert["evidence_tier"], "proved", "cert: {cert}");
    assert_eq!(cert["verdict_summary"]["contracts_failed"], 1, "cert: {cert}");
    assert_eq!(code, 1);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn an_unknown_file_gets_evidence_tier_unknown_and_exit_2() {
    let src = "fn flip(x: bool) -> bool {\n    return x\n}\n\nvalidate flip {\n    post: true\n}\n";
    let path = scratch_file("unknown", src);
    let (cert, code) = run_certify(&path);
    assert_eq!(cert["verdict_summary"]["verdict"], "UNKNOWN", "cert: {cert}");
    assert_eq!(cert["evidence_tier"], "unknown", "cert: {cert}");
    assert_eq!(code, 2);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_hard_pipeline_failure_before_contracts_ever_ran_is_also_evidence_tier_unknown() {
    let path = scratch_file("typecheck_fail", "fn broken() -> i64 {\n    return \"not an i64\"\n}\n");
    let (cert, code) = run_certify(&path);
    assert_eq!(cert["verdict_summary"]["verdict"], "DISPROVED", "cert: {cert}");
    assert_eq!(
        cert["evidence_tier"], "unknown",
        "contract-check never ran (typecheck failed first) -- no formal evidence was produced, so this must not claim `proved`: {cert}"
    );
    assert_eq!(code, 1);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn source_hash_is_a_real_sha256_of_the_exact_file_bytes() {
    let src = "fn add(a: i64, b: i64) -> i64 {\n    return a + b\n}\n";
    let path = scratch_file("hash", src);
    let (cert, _code) = run_certify(&path);
    let mut hasher = Sha256::new();
    hasher.update(src.as_bytes());
    let expected = format!("sha256:{:x}", hasher.finalize());
    assert_eq!(cert["source_hash"], expected, "cert: {cert}");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn grammar_hash_matches_the_real_nirdosha_gbnf_shipped_in_the_binary() {
    let path = scratch_file("grammar_hash", "fn add(a: i64, b: i64) -> i64 {\n    return a + b\n}\n");
    let (cert, _code) = run_certify(&path);
    let gbnf = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/nirdosha.gbnf")).expect("nirdosha.gbnf should exist");
    let mut hasher = Sha256::new();
    hasher.update(&gbnf);
    let expected = format!("sha256:{:x}", hasher.finalize());
    assert_eq!(cert["grammar_hash"], expected, "cert: {cert}");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn certifying_the_same_file_twice_is_byte_for_byte_deterministic() {
    let path = scratch_file("deterministic", "fn add(a: i64, b: i64) -> i64 {\n    return a + b\n}\n");
    let output1 = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha")).arg("certify").arg(&path).output().unwrap();
    let output2 = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha")).arg("certify").arg(&path).output().unwrap();
    assert_eq!(output1.stdout, output2.stdout, "certifying an unchanged file twice must produce byte-identical JSON");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn certificate_never_embeds_the_source_file_path() {
    // Deterministic/reproducible-by-a-third-party means content-addressed
    // (source_hash), never the caller's own filesystem path.
    let path = scratch_file("no_path_leak", "fn add(a: i64, b: i64) -> i64 {\n    return a + b\n}\n");
    let (cert, _code) = run_certify(&path);
    let text = serde_json::to_string(&cert).unwrap();
    assert!(!text.contains(path.to_str().unwrap()), "certificate must not embed the source file's own path: {cert}");
    let _ = std::fs::remove_file(&path);
}
