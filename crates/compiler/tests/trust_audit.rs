//! End-to-end tests for `nirdosha attest`/`nirdosha audit`
//! (`cmd_attest`/`cmd_audit`/`audit_one_attestation` in `main.rs`,
//! `nirdosha-master-plan.md` Part 3 Dec 2026's "Confidence/trust
//! propagation + reviewer-forgery prevention"). Real Ed25519 keys,
//! generated for real via `nirdosha keygen`, signing real attestations
//! checked against a real trust config -- no mocked crypto, matching
//! `signed_certificate.rs`'s own discipline.

fn unique_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn scratch_path(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("nirdosha_trust_audit_test_{}_{}_{name}", std::process::id(), unique_suffix()));
    p
}

fn run(args: &[&str]) -> (Vec<u8>, Vec<u8>, i32) {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha")).args(args).output().expect("nirdosha should run");
    (output.stdout, output.stderr, output.status.code().expect("process should exit with a status code, not be signal-killed"))
}

fn keygen(name: &str) -> (std::path::PathBuf, String) {
    let key_path = scratch_path(&format!("{name}.pk8"));
    let (_stdout, stderr, code) = run(&["keygen", "-o", key_path.to_str().unwrap()]);
    assert_eq!(code, 0, "keygen should succeed: {}", String::from_utf8_lossy(&stderr));
    let public_key = std::fs::read_to_string(format!("{}.pub", key_path.display())).expect("public key file should be readable").trim().to_string();
    (key_path, public_key)
}

fn write_trust_config(human_reviewers: &[(&str, &str)], known_agents: &[(&str, &str)]) -> std::path::PathBuf {
    let path = scratch_path("trust_config.json");
    let value = serde_json::json!({
        "trust_config_version": "0",
        "known_agents": known_agents.iter().map(|(name, key)| serde_json::json!({"name": name, "public_key": key})).collect::<Vec<_>>(),
        "human_reviewers": human_reviewers.iter().map(|(name, key)| serde_json::json!({"name": name, "public_key": key})).collect::<Vec<_>>(),
    });
    std::fs::write(&path, serde_json::to_vec(&value).unwrap()).expect("writing trust config should succeed");
    path
}

fn scratch_nir(name: &str, src: &str) -> std::path::PathBuf {
    let p = scratch_path(&format!("{name}.nir"));
    std::fs::write(&p, src).expect("scratch .nir file should write");
    p
}

const PROVED_SOURCE: &str = "fn add(a: i64, b: i64) -> i64 {\n    return a + b\n}\n";

#[test]
fn attest_refuses_an_unregistered_reviewer() {
    let (key_path, _pub_key) = keygen("mallory");
    let trust_config = write_trust_config(&[("alice", "not-mallorys-key")], &[]);
    let file = scratch_nir("unregistered", PROVED_SOURCE);

    let (_stdout, stderr, code) = run(&["attest", file.to_str().unwrap(), "--reviewer", "mallory", "--role", "human", "--key", key_path.to_str().unwrap(), "--trust-config", trust_config.to_str().unwrap()]);
    assert_eq!(code, 1);
    assert!(String::from_utf8_lossy(&stderr).contains("not registered"), "stderr: {}", String::from_utf8_lossy(&stderr));

    for p in [&key_path, &trust_config, &file] {
        let _ = std::fs::remove_file(p);
    }
    let _ = std::fs::remove_file(format!("{}.pub", key_path.display()));
}

#[test]
fn a_current_valid_attestation_from_a_registered_reviewer_reports_current() {
    let (key_path, public_key) = keygen("alice");
    let trust_config = write_trust_config(&[("alice", &public_key)], &[]);
    let file = scratch_nir("current", PROVED_SOURCE);
    let attestation_path = scratch_path("attestation.json");

    let (_stdout, stderr, code) = run(&[
        "attest",
        file.to_str().unwrap(),
        "--reviewer",
        "alice",
        "--role",
        "human",
        "--key",
        key_path.to_str().unwrap(),
        "--trust-config",
        trust_config.to_str().unwrap(),
        "-o",
        attestation_path.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));

    let (audit_stdout, audit_stderr, audit_code) = run(&["audit", file.to_str().unwrap(), "--trust-config", trust_config.to_str().unwrap(), "--attestation", attestation_path.to_str().unwrap()]);
    assert_eq!(audit_code, 0, "stderr: {}", String::from_utf8_lossy(&audit_stderr));
    let report: serde_json::Value = serde_json::from_slice(&audit_stdout).expect("audit should print valid JSON");
    assert_eq!(report["attestations"][0]["status"], "CURRENT", "report: {report}");
    assert_eq!(report["verify_verdict"], "PROVED", "report: {report}");
    assert_eq!(report["trust_summary"], "PROVED_AND_HUMAN_REVIEWED", "report: {report}");

    for p in [&key_path, &trust_config, &file, &attestation_path] {
        let _ = std::fs::remove_file(p);
    }
    let _ = std::fs::remove_file(format!("{}.pub", key_path.display()));
}

#[test]
fn a_known_agent_reviewer_never_counts_as_human_reviewed() {
    let (key_path, public_key) = keygen("claude-code");
    let trust_config = write_trust_config(&[], &[("claude-code", &public_key)]);
    let file = scratch_nir("agent_reviewed", PROVED_SOURCE);
    let attestation_path = scratch_path("agent_attestation.json");

    let (_stdout, stderr, code) = run(&[
        "attest",
        file.to_str().unwrap(),
        "--reviewer",
        "claude-code",
        "--role",
        "agent",
        "--key",
        key_path.to_str().unwrap(),
        "--trust-config",
        trust_config.to_str().unwrap(),
        "-o",
        attestation_path.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));

    let (audit_stdout, _stderr, audit_code) = run(&["audit", file.to_str().unwrap(), "--trust-config", trust_config.to_str().unwrap(), "--attestation", attestation_path.to_str().unwrap()]);
    assert_eq!(audit_code, 0);
    let report: serde_json::Value = serde_json::from_slice(&audit_stdout).expect("audit should print valid JSON");
    assert_eq!(report["attestations"][0]["status"], "CURRENT", "report: {report}");
    assert_eq!(report["attestations"][0]["role"], "known_agent", "report: {report}");
    assert_eq!(report["trust_summary"], "PROVED_AND_AGENT_REVIEWED", "an agent attestation must never be reported as human-reviewed: {report}");

    for p in [&key_path, &trust_config, &file, &attestation_path] {
        let _ = std::fs::remove_file(p);
    }
    let _ = std::fs::remove_file(format!("{}.pub", key_path.display()));
}

#[test]
fn a_tampered_attestation_is_detected_as_forged_and_rejects_the_whole_report() {
    let (key_path, public_key) = keygen("bob");
    let trust_config = write_trust_config(&[("bob", &public_key)], &[]);
    let file = scratch_nir("tampered", PROVED_SOURCE);
    let attestation_path = scratch_path("tampered_attestation.json");

    let (stdout, stderr, code) = run(&[
        "attest",
        file.to_str().unwrap(),
        "--reviewer",
        "bob",
        "--role",
        "human",
        "--key",
        key_path.to_str().unwrap(),
        "--trust-config",
        trust_config.to_str().unwrap(),
        "-o",
        attestation_path.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));

    // Tamper with the attestation after signing -- the note is
    // real-looking content an attacker might want to change while
    // keeping the original (now-invalid) signature attached.
    let mut attestation: serde_json::Value = serde_json::from_slice(&stdout).expect("attest should print valid JSON");
    attestation["note"] = serde_json::json!("actually never reviewed, forged after the fact");
    std::fs::write(&attestation_path, serde_json::to_vec(&attestation).unwrap()).expect("writing the tampered attestation should succeed");

    let (audit_stdout, _stderr, audit_code) = run(&["audit", file.to_str().unwrap(), "--trust-config", trust_config.to_str().unwrap(), "--attestation", attestation_path.to_str().unwrap()]);
    assert_eq!(audit_code, 1, "a report containing a forged attestation must fail");
    let report: serde_json::Value = serde_json::from_slice(&audit_stdout).expect("audit should print valid JSON even on failure");
    assert_eq!(report["attestations"][0]["status"], "FORGED_OR_TAMPERED", "report: {report}");
    assert_eq!(report["trust_summary"], "REJECTED_ATTESTATION_PRESENT", "report: {report}");

    for p in [&key_path, &trust_config, &file, &attestation_path] {
        let _ = std::fs::remove_file(p);
    }
    let _ = std::fs::remove_file(format!("{}.pub", key_path.display()));
}

#[test]
fn an_attestation_for_an_old_version_of_the_file_is_reported_stale() {
    let (key_path, public_key) = keygen("carol");
    let trust_config = write_trust_config(&[("carol", &public_key)], &[]);
    let file = scratch_nir("stale", PROVED_SOURCE);
    let attestation_path = scratch_path("stale_attestation.json");

    let (_stdout, stderr, code) = run(&[
        "attest",
        file.to_str().unwrap(),
        "--reviewer",
        "carol",
        "--role",
        "human",
        "--key",
        key_path.to_str().unwrap(),
        "--trust-config",
        trust_config.to_str().unwrap(),
        "-o",
        attestation_path.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));

    // The code moves on after the review -- append a harmless comment.
    let mut updated = std::fs::read_to_string(&file).unwrap();
    updated.push_str("// a change made after carol's review\n");
    std::fs::write(&file, updated).expect("updating the file should succeed");

    let (audit_stdout, _stderr, audit_code) = run(&["audit", file.to_str().unwrap(), "--trust-config", trust_config.to_str().unwrap(), "--attestation", attestation_path.to_str().unwrap()]);
    assert_eq!(audit_code, 0, "a stale attestation alone (no forgery) should not fail the report");
    let report: serde_json::Value = serde_json::from_slice(&audit_stdout).expect("audit should print valid JSON");
    assert_eq!(report["attestations"][0]["status"], "STALE", "report: {report}");
    assert_eq!(report["trust_summary"], "PROVED_ONLY", "a stale review must not count toward trust_summary: {report}");

    for p in [&key_path, &trust_config, &file, &attestation_path] {
        let _ = std::fs::remove_file(p);
    }
    let _ = std::fs::remove_file(format!("{}.pub", key_path.display()));
}

#[test]
fn no_attestations_at_all_is_unverified_not_an_error() {
    let trust_config = write_trust_config(&[], &[]);
    let file = scratch_nir("no_attestations", PROVED_SOURCE);

    let (stdout, stderr, code) = run(&["audit", file.to_str().unwrap(), "--trust-config", trust_config.to_str().unwrap()]);
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    let report: serde_json::Value = serde_json::from_slice(&stdout).expect("audit should print valid JSON");
    assert_eq!(report["trust_summary"], "PROVED_ONLY", "report: {report}");
    assert_eq!(report["attestations"].as_array().unwrap().len(), 0, "report: {report}");

    let _ = std::fs::remove_file(&trust_config);
    let _ = std::fs::remove_file(&file);
}
