//! Issue #75 item 1: "`cargo nirdosha build` produces a binary and a
//! separate certificate file on disk; nothing travels *with* the
//! artifact." Before this fix, no `guarantees-<package>.json` was ever
//! emitted — this test fails against that prior behavior (no such
//! file) and passes once `cargo nirdosha verify` (and by extension
//! `build`/`check`/`run`/`test`/`doc`/`bench`, which all funnel
//! through the same `verify_cwd`) emits the guarantee bundle.

use std::path::PathBuf;
use std::process::Command;

fn example_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("examples")
        .join("rt-payroll")
}

#[test]
fn verify_emits_a_guarantee_bundle_alongside_the_certificate() {
    let out = Command::new(env!("CARGO_BIN_EXE_cargo-nirdosha"))
        .current_dir(example_dir())
        .arg("verify")
        .output()
        .expect("cargo-nirdosha must run");
    assert!(
        out.status.success(),
        "verify must pass on the compliant flagship example — stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    let bundle_line = stderr
        .lines()
        .find(|l| l.contains("guarantee bundle"))
        .unwrap_or_else(|| panic!("no guarantee bundle line in stderr: {stderr}"));
    let bundle_path = bundle_line
        .rsplit("→ ")
        .next()
        .expect("bundle line names a path")
        .trim();

    let bytes = std::fs::read(bundle_path)
        .unwrap_or_else(|e| panic!("guarantee bundle was not written to {bundle_path}: {e}"));
    let bundle: serde_json::Value = serde_json::from_slice(&bytes).expect("bundle is valid JSON");

    assert_eq!(bundle["bundle_version"], "1");
    assert_eq!(bundle["package"], "rt-payroll");
    // Provenance link back to the source certificate (RFC 0017 §4's
    // design goal, applied to the dialect): a hash that changes if any
    // bound fact does, plus the certificate's own path.
    assert!(
        bundle["source_hash"].as_str().is_some_and(|h| h.len() == 64),
        "source_hash must be the certificate's hex binding: {bundle}"
    );
    assert!(bundle["certificate"].as_str().is_some_and(|p| p.contains("contract-report-rt-payroll.json")));

    // Data already computed by the scanner, just serialized: inferred
    // effects, the exposed/gated surface, and declared NFRs.
    assert_eq!(bundle["inferred_effects"]["compute_payroll"], serde_json::json!(["pure"]));
    assert_eq!(bundle["gated_exports"]["compute_payroll"], "role:hr_staff");
    assert_eq!(bundle["nfr_tracked"]["compute_payroll"]["latency_ms"], 250.0);
}
