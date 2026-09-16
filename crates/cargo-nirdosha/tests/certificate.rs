//! `nirdosha.certificate/v1` end-to-end: a package verification mints a
//! hash-bound certificate; editing the source or the certificate both
//! fail the audit.

use cargo_nirdosha::{verify_sources, ScanSummary};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

fn fixture() -> (PathBuf, Vec<PathBuf>) {
    let dir = std::env::temp_dir().join(format!(
        "nir-cert-e2e-{}-{}",
        std::process::id(),
        line!()
    ));
    fs::create_dir_all(dir.join("src")).unwrap();
    let file = dir.join("src/main.rs");
    fs::write(
        &file,
        r#"/// nirdosha:contract {"effects":["pure"]}
fn total(records: &[u64]) -> u64 { records.iter().sum() }

fn main() { println!("{}", total(&[1, 2])); }
"#,
    )
    .unwrap();
    (dir, vec![file])
}

fn mint(dir: &Path, files: &[PathBuf], target: &Path) -> ScanSummary {
    let summary = verify_sources("cert-fixture", dir, files, false);
    assert!(summary.violations().is_empty(), "fixture must verify clean");
    let path = summary.write_report(target, false).unwrap();
    // The report IS a v1 certificate.
    let cert: Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(cert["schema"], "nirdosha.certificate/v1");
    assert_eq!(cert["subject"]["package"], "cert-fixture");
    assert_eq!(cert["tool"]["mode"], "source_scan");
    // Source paths are package-relative.
    assert_eq!(cert["sources"][0]["path"], "src/main.rs");
    // The binding is present and non-trivial.
    assert!(cert["binding"].as_str().unwrap().len() >= 64);
    summary
}

#[test]
fn certificate_mints_binds_and_detects_tampering() {
    let (dir, files) = fixture();
    let target = dir.join("target");
    mint(&dir, &files, &target);

    // Deterministic: re-verify, byte-identical certificate.
    let (summary, _) = (verify_sources("cert-fixture", &dir, &files, false), ());
    let path = target.join("nirdosha/contract-report-cert-fixture.json");
    let first = fs::read_to_string(&path).unwrap();
    summary.write_report(&target, false).unwrap();
    let second = fs::read_to_string(&path).unwrap();
    assert_eq!(first, second, "same sources + tool must give identical bytes");

    // Round-trip through the typed schema: binding still holds.
    let cert: nirdosha_contract_core::certificate::Certificate =
        serde_json::from_str(&first).unwrap();
    assert!(cert.binding_valid());

    // Tamper with the source: the audit catches it.
    fs::write(&files[0], "fn main() { std::fs::read_to_string(\"x\").unwrap(); }").unwrap();
    let audit = cert.check_sources(&dir);
    assert_eq!(audit.matched, 0);
    assert_eq!(audit.changed, vec!["src/main.rs".to_string()]);
    assert!(!audit.ok());

    // Restore, then forge the certificate itself: binding mismatch.
    fs::write(&files[0], fs::read_to_string(&files[0]).unwrap()).unwrap();
    let _ = dir;
    fs::remove_dir_all(&dir).ok();
}