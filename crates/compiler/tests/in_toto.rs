//! End-to-end tests for `--in-toto` on `nirdosha verify`/`fix`/
//! `certify` (`wrap_in_toto` in `main.rs`, `nirdosha-master-plan.md`
//! Part 3 Q1 2027's "Spec v1 published -- verdict schema + certificate
//! format + repair protocol as an in-toto predicate"). Checks the
//! wrapper against the real, published in-toto v1 Statement shape
//! (<https://in-toto.io/Statement/v1>) field by field, not just "it's
//! valid JSON" -- same `std::process::Command`-against-the-real-binary
//! pattern every other integration test in this crate uses.

use sha2::{Digest, Sha256};

fn unique_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn scratch_file(name: &str, src: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("nirdosha_in_toto_test_{}_{}_{name}.nir", std::process::id(), unique_suffix()));
    std::fs::write(&p, src).expect("scratch .nir file should write");
    p
}

fn run(args: &[&str]) -> (serde_json::Value, i32) {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha")).args(args).output().expect("nirdosha should run");
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|e| panic!("did not print valid JSON ({e}): stderr={}", String::from_utf8_lossy(&output.stderr)));
    (value, output.status.code().expect("process should exit with a status code, not be signal-killed"))
}

fn real_sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn assert_real_in_toto_statement(value: &serde_json::Value, path: &std::path::Path, expected_predicate_type: &str) {
    assert_eq!(value["_type"], "https://in-toto.io/Statement/v1", "value: {value}");
    assert_eq!(value["predicateType"], expected_predicate_type, "value: {value}");
    let subject = value["subject"].as_array().expect("subject should be an array");
    assert_eq!(subject.len(), 1, "value: {value}");
    assert_eq!(subject[0]["name"], path.to_str().unwrap(), "value: {value}");
    let expected_digest = real_sha256_hex(&std::fs::read(path).expect("reading the source file should succeed"));
    assert_eq!(subject[0]["digest"]["sha256"], expected_digest, "the subject digest must be the real SHA-256 of the exact file bytes: {value}");
    assert!(value["predicate"].is_object(), "value: {value}");
}

const PROVED_SOURCE: &str = "fn add(a: i64, b: i64) -> i64 {\n    return a + b\n}\n";

#[test]
fn verify_in_toto_wraps_the_real_verdict_with_a_real_subject_digest() {
    let path = scratch_file("verify", PROVED_SOURCE);
    let (statement, code) = run(&["verify", "--in-toto", path.to_str().unwrap()]);
    assert_eq!(code, 0);
    assert_real_in_toto_statement(&statement, &path, "https://nirdosha.dev/attestations/verify/v1");
    assert_eq!(statement["predicate"]["verdict"], "PROVED", "statement: {statement}");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn certify_in_toto_wraps_the_real_certificate_and_digests_agree() {
    let path = scratch_file("certify", PROVED_SOURCE);
    let (statement, code) = run(&["certify", "--in-toto", path.to_str().unwrap()]);
    assert_eq!(code, 0);
    assert_real_in_toto_statement(&statement, &path, "https://nirdosha.dev/attestations/certificate/v1");
    // The certificate's own source_hash (its own attestation-format
    // field, independent of the in-toto wrapper) must agree with the
    // in-toto subject digest -- two descriptions of the same fact,
    // never allowed to drift.
    let inner_hash = statement["predicate"]["source_hash"].as_str().expect("source_hash should be a string");
    let outer_hash = statement["subject"][0]["digest"]["sha256"].as_str().expect("subject digest should be a string");
    assert_eq!(inner_hash, format!("sha256:{outer_hash}"), "statement: {statement}");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn certify_in_toto_composes_with_sign() {
    let key_path = {
        let mut p = std::env::temp_dir();
        p.push(format!("nirdosha_in_toto_test_{}_{}_key.pk8", std::process::id(), unique_suffix()));
        p
    };
    let keygen_output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha")).args(["keygen", "-o", key_path.to_str().unwrap()]).output().expect("keygen should run");
    assert!(keygen_output.status.success());

    let path = scratch_file("signed_in_toto", PROVED_SOURCE);
    let (statement, code) = run(&["certify", path.to_str().unwrap(), "--sign", key_path.to_str().unwrap(), "--in-toto"]);
    assert_eq!(code, 0);
    assert_real_in_toto_statement(&statement, &path, "https://nirdosha.dev/attestations/certificate/v1");
    assert_eq!(statement["predicate"]["signature_algorithm"], "ed25519", "statement: {statement}");
    assert!(statement["predicate"]["signature"].as_str().is_some_and(|s| !s.is_empty()), "statement: {statement}");

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&key_path);
    let _ = std::fs::remove_file(format!("{}.pub", key_path.display()));
}

#[test]
fn fix_in_toto_wraps_the_real_fix_report() {
    let path = scratch_file("fix", "fn main() {\n    let amount: i64 = 5\n    print(ammount)\n}\n");
    let (statement, code) = run(&["fix", "--in-toto", path.to_str().unwrap()]);
    assert_eq!(code, 1, "a real, uncorrected typo should still be DISPROVED overall");
    assert_real_in_toto_statement(&statement, &path, "https://nirdosha.dev/attestations/fix/v1");
    assert_eq!(
        statement["predicate"]["before"]["typecheck"]["errors"][0]["fix"]["applicability"],
        "auto",
        "statement: {statement}"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn without_in_toto_the_output_is_the_plain_json_unwrapped() {
    let path = scratch_file("plain", PROVED_SOURCE);
    let (value, code) = run(&["certify", path.to_str().unwrap()]);
    assert_eq!(code, 0);
    assert!(value.get("_type").is_none(), "value: {value}");
    assert!(value.get("predicateType").is_none(), "value: {value}");
    assert_eq!(value["certificate_version"], "0", "value: {value}");
    let _ = std::fs::remove_file(&path);
}
