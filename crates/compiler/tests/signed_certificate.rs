//! End-to-end tests for Certificate v1 (`nirdosha keygen`,
//! `nirdosha certify --sign`, `nirdosha verify-certificate` in
//! `main.rs`, `nirdosha-master-plan.md` Part 3 Nov 2026's "Signed
//! certificates (v1) -- key-pinned verdicts"). Same
//! `std::process::Command`-against-the-real-binary pattern
//! `certify_command.rs`/`fix_command.rs` use -- a real Ed25519
//! keypair, generated for real, signing a real certificate, verified
//! for real; no mocked crypto.

fn unique_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn scratch_path(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("nirdosha_signed_cert_test_{}_{}_{name}", std::process::id(), unique_suffix()));
    p
}

fn scratch_nir(name: &str, src: &str) -> std::path::PathBuf {
    let p = scratch_path(&format!("{name}.nir"));
    std::fs::write(&p, src).expect("scratch .nir file should write");
    p
}

fn run(args: &[&str]) -> (Vec<u8>, Vec<u8>, i32) {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha")).args(args).output().expect("nirdosha should run");
    (output.stdout, output.stderr, output.status.code().expect("process should exit with a status code, not be signal-killed"))
}

fn keygen() -> std::path::PathBuf {
    let key_path = scratch_path("key.pk8");
    let (stdout, stderr, code) = run(&["keygen", "-o", key_path.to_str().unwrap()]);
    assert_eq!(code, 0, "keygen should succeed: {}", String::from_utf8_lossy(&stderr));
    let value: serde_json::Value = serde_json::from_slice(&stdout).expect("keygen should print valid JSON");
    assert_eq!(value["algorithm"], "ed25519", "value: {value}");
    assert!(key_path.exists(), "private key file should exist");
    let pub_path = format!("{}.pub", key_path.display());
    assert!(std::path::Path::new(&pub_path).exists(), "public key file should exist");
    key_path
}

#[test]
fn keygen_writes_a_private_and_public_key() {
    let key_path = keygen();
    let private_bytes = std::fs::read(&key_path).expect("private key should be readable");
    assert!(!private_bytes.is_empty(), "private key file should not be empty");
    let pub_path = format!("{}.pub", key_path.display());
    let public_text = std::fs::read_to_string(&pub_path).expect("public key file should be readable");
    assert!(!public_text.trim().is_empty(), "public key file should not be empty");
    let _ = std::fs::remove_file(&key_path);
    let _ = std::fs::remove_file(&pub_path);
}

#[test]
fn certify_without_sign_has_no_signature_fields() {
    let path = scratch_nir("unsigned", "fn add(a: i64, b: i64) -> i64 {\n    return a + b\n}\n");
    let (stdout, _stderr, code) = run(&["certify", path.to_str().unwrap()]);
    assert_eq!(code, 0);
    let cert: serde_json::Value = serde_json::from_slice(&stdout).expect("certify should print valid JSON");
    assert!(cert.get("signature").is_none(), "cert: {cert}");
    assert!(cert.get("public_key").is_none(), "cert: {cert}");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn certify_sign_produces_a_certificate_that_verifies() {
    let key_path = keygen();
    let path = scratch_nir("signed", "fn add(a: i64, b: i64) -> i64 {\n    return a + b\n}\n");
    let (stdout, stderr, code) = run(&["certify", path.to_str().unwrap(), "--sign", key_path.to_str().unwrap()]);
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    let cert: serde_json::Value = serde_json::from_slice(&stdout).expect("certify --sign should print valid JSON");
    assert_eq!(cert["signature_algorithm"], "ed25519", "cert: {cert}");
    assert!(cert["signature"].as_str().is_some_and(|s| !s.is_empty()), "cert: {cert}");
    assert!(cert["public_key"].as_str().is_some_and(|s| !s.is_empty()), "cert: {cert}");
    // Every v0 field must still be present (additive, per
    // docs/STABILITY_AND_RELEASES.md's rule for this schema).
    assert_eq!(cert["certificate_version"], "0", "cert: {cert}");
    assert_eq!(cert["evidence_tier"], "proved", "cert: {cert}");

    let cert_path = scratch_path("cert.json");
    std::fs::write(&cert_path, &stdout).expect("writing the certificate should succeed");

    let (verify_stdout, verify_stderr, verify_code) = run(&["verify-certificate", cert_path.to_str().unwrap()]);
    assert_eq!(verify_code, 0, "stderr: {}", String::from_utf8_lossy(&verify_stderr));
    let verify_result: serde_json::Value = serde_json::from_slice(&verify_stdout).expect("verify-certificate should print valid JSON");
    assert_eq!(verify_result["valid"], true, "result: {verify_result}");

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&cert_path);
    let _ = std::fs::remove_file(&key_path);
    let _ = std::fs::remove_file(format!("{}.pub", key_path.display()));
}

#[test]
fn verify_certificate_rejects_a_tampered_certificate() {
    let key_path = keygen();
    let path = scratch_nir("tamper", "fn add(a: i64, b: i64) -> i64 {\n    return a + b\n}\n");
    let (stdout, _stderr, code) = run(&["certify", path.to_str().unwrap(), "--sign", key_path.to_str().unwrap()]);
    assert_eq!(code, 0);
    let mut cert: serde_json::Value = serde_json::from_slice(&stdout).expect("certify --sign should print valid JSON");
    // Tamper with a real field after signing -- the signature must no
    // longer validate.
    cert["evidence_tier"] = serde_json::json!("checked");

    let cert_path = scratch_path("tampered_cert.json");
    std::fs::write(&cert_path, serde_json::to_vec(&cert).unwrap()).expect("writing should succeed");

    let (verify_stdout, _verify_stderr, verify_code) = run(&["verify-certificate", cert_path.to_str().unwrap()]);
    assert_eq!(verify_code, 1, "a tampered certificate must fail verification");
    let verify_result: serde_json::Value = serde_json::from_slice(&verify_stdout).expect("verify-certificate should print valid JSON even on failure");
    assert_eq!(verify_result["valid"], false, "result: {verify_result}");

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&cert_path);
    let _ = std::fs::remove_file(&key_path);
    let _ = std::fs::remove_file(format!("{}.pub", key_path.display()));
}

#[test]
fn verify_certificate_rejects_the_wrong_public_key() {
    let key_path_a = keygen();
    let key_path_b = keygen();
    let path = scratch_nir("wrong_key", "fn add(a: i64, b: i64) -> i64 {\n    return a + b\n}\n");
    let (stdout, _stderr, code) = run(&["certify", path.to_str().unwrap(), "--sign", key_path_a.to_str().unwrap()]);
    assert_eq!(code, 0);
    let mut cert: serde_json::Value = serde_json::from_slice(&stdout).expect("certify --sign should print valid JSON");

    let pub_b = std::fs::read_to_string(format!("{}.pub", key_path_b.display())).expect("reading key B's public key should succeed");
    cert["public_key"] = serde_json::json!(pub_b.trim());

    let cert_path = scratch_path("wrong_key_cert.json");
    std::fs::write(&cert_path, serde_json::to_vec(&cert).unwrap()).expect("writing should succeed");

    let (_verify_stdout, _verify_stderr, verify_code) = run(&["verify-certificate", cert_path.to_str().unwrap()]);
    assert_eq!(verify_code, 1, "a certificate whose signature doesn't match the embedded key must fail verification");

    for p in [&path, &cert_path, &key_path_a, &key_path_b] {
        let _ = std::fs::remove_file(p);
    }
    let _ = std::fs::remove_file(format!("{}.pub", key_path_a.display()));
    let _ = std::fs::remove_file(format!("{}.pub", key_path_b.display()));
}

#[test]
fn verify_certificate_on_an_unsigned_certificate_fails_cleanly() {
    let path = scratch_nir("no_sig", "fn add(a: i64, b: i64) -> i64 {\n    return a + b\n}\n");
    let (stdout, _stderr, code) = run(&["certify", path.to_str().unwrap()]);
    assert_eq!(code, 0);
    let cert_path = scratch_path("unsigned_cert.json");
    std::fs::write(&cert_path, &stdout).expect("writing should succeed");

    let (_verify_stdout, verify_stderr, verify_code) = run(&["verify-certificate", cert_path.to_str().unwrap()]);
    assert_eq!(verify_code, 1);
    assert!(
        String::from_utf8_lossy(&verify_stderr).contains("not a signed certificate"),
        "stderr: {}",
        String::from_utf8_lossy(&verify_stderr)
    );

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&cert_path);
}
