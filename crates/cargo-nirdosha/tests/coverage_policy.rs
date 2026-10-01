//! Migration gate: scanner success must never imply runtime safety.
use cargo_nirdosha::verify_sources;
use nirdosha_contract_core::{certificate::Certificate, evidence::check_policy};
use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new(source: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "nir-coverage-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src/main.rs"), source).unwrap();
        Self(dir)
    }
    fn mint(&self) -> (PathBuf, Certificate) {
        let summary = verify_sources("coverage", &self.0, &[self.0.join("src/main.rs")], false);
        let path = summary.write_report(&self.0.join("target"), false).unwrap();
        let cert = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        (path, cert)
    }

    /// Same as `mint`, plus a real `Cargo.lock` at the fixture root so
    /// G6's provenance binding has something to hash.
    fn mint_with_provenance(&self) -> (PathBuf, Certificate) {
        fs::write(self.0.join("Cargo.lock"), "# fixture lockfile\n").unwrap();
        let summary = verify_sources("coverage", &self.0, &[self.0.join("src/main.rs")], false);
        let path = summary.write_report(&self.0.join("target"), true).unwrap();
        let cert = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        (path, cert)
    }
    fn cli(&self, path: &std::path::Path, required: &[&str]) -> std::process::Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_cargo-nirdosha"));
        cmd.arg("check-certificate")
            .arg(path)
            .arg("--root")
            .arg(&self.0);
        for name in required {
            cmd.arg("--require").arg(name);
        }
        cmd.output().unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const CLEAN: &str = "/// nirdosha:contract {\"effects\":[\"pure\"]}\nfn value() -> u64 { 1 }";

#[test]
fn clean_scan_satisfies_only_source_scan() {
    let fixture = Fixture::new(CLEAN);
    let (path, cert) = fixture.mint();
    assert!(cert.binding_valid());
    assert!(fixture.cli(&path, &["source_scan"]).status.success());
    for guarantee in nirdosha_contract_core::evidence::GUARANTEES {
        if *guarantee == "source_scan" {
            continue;
        }
        let result = fixture.cli(&path, &[guarantee]);
        assert!(!result.status.success(), "{guarantee}");
        assert!(String::from_utf8_lossy(&result.stderr).contains("unsupported"));
    }
    assert!(!fixture.cli(&path, &[]).status.success());
    assert!(!fixture.cli(&path, &["typo"]).status.success());
    assert!(
        !fixture
            .cli(&path, &["source_scan", "durable_transactions"])
            .status
            .success()
    );
}

#[test]
fn indirect_lie_and_macro_are_not_certified_as_pure() {
    for source in [
        "fn hidden() { std::fs::read_to_string(\"x\"); }\n/// nirdosha:contract {\"effects\":[\"pure\"]}\nfn claim() { hidden(); }",
        "macro_rules! hidden { () => { std::fs::read_to_string(\"x\") }; }\n/// nirdosha:contract {\"effects\":[\"pure\"]}\nfn claim() { hidden!(); }",
    ] {
        let fixture = Fixture::new(source);
        let (_, cert) = fixture.mint();
        assert_eq!(
            cert.verification["violations"], 0,
            "pins source scanner's known blind spot"
        );
        assert!(check_policy(&cert, &["effects_pure".into()]).is_err());
    }
}

#[test]
fn stale_failed_legacy_and_promoted_reports_fail_closed() {
    let fixture = Fixture::new(CLEAN);
    let (path, cert) = fixture.mint();
    fs::write(fixture.0.join("src/main.rs"), "fn changed() {}").unwrap();
    assert!(!fixture.cli(&path, &["source_scan"]).status.success());
    fs::remove_file(fixture.0.join("src/main.rs")).unwrap();
    assert!(!fixture.cli(&path, &["source_scan"]).status.success());

    let mut legacy_payload = cert.verification.clone();
    legacy_payload.as_object_mut().unwrap().remove("coverage");
    let legacy = Certificate::new(
        cert.subject.clone(),
        cert.tool.clone(),
        cert.sources.clone(),
        legacy_payload,
    );
    assert!(legacy.binding_valid());
    assert!(check_policy(&legacy, &["source_scan".into()]).is_err());

    let mut promoted = cert.verification.clone();
    promoted["coverage"]["claims"][1]["outcome"] = "passed".into();
    let promoted = Certificate::new(
        cert.subject.clone(),
        cert.tool.clone(),
        cert.sources.clone(),
        promoted,
    );
    assert!(promoted.binding_valid());
    assert!(check_policy(&promoted, &["effects_pure".into()]).is_err());

    let lying = Fixture::new(
        "/// nirdosha:contract {\"effects\":[\"pure\"]}\nfn lie() { println!(\"oops\"); }",
    );
    let (path, cert) = lying.mint();
    assert!(cert.verification["violations"].as_u64().unwrap() > 0);
    assert!(!lying.cli(&path, &["source_scan"]).status.success());
}

#[test]
fn provenance_binding_passes_then_fails_on_a_changed_lockfile() {
    let fixture = Fixture::new(CLEAN);
    let (path, cert) = fixture.mint_with_provenance();
    assert!(cert.binding_valid());
    assert!(cert.verification.get("provenance").is_some());
    assert!(cert.tool.toolchain.is_some(), "provenance binds a real toolchain string");

    // build_provenance passes alongside source_scan.
    assert!(fixture.cli(&path, &["source_scan", "build_provenance"]).status.success());

    // Change the dependency closure: the same certificate must now refuse.
    fs::write(fixture.0.join("Cargo.lock"), "# a different lockfile\n").unwrap();
    let result = fixture.cli(&path, &["build_provenance"]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("dependency closure"));

    // source_scan alone is unaffected by the lockfile changing.
    assert!(fixture.cli(&path, &["source_scan"]).status.success());
}

#[test]
fn provenance_is_not_required_and_not_claimed_without_the_flag() {
    let fixture = Fixture::new(CLEAN);
    let (_, cert) = fixture.mint();
    assert!(cert.verification.get("provenance").is_none());
    assert!(cert.tool.toolchain.is_none());
    let coverage = &cert.verification["coverage"];
    let claims = coverage["claims"].as_array().unwrap();
    let build_provenance = claims
        .iter()
        .find(|c| c["guarantee"] == "build_provenance")
        .unwrap();
    assert_eq!(build_provenance["outcome"], "unsupported");
}

#[test]
fn workspace_hash_failure_is_not_silently_omitted() {
    let fixture = Fixture::new(CLEAN);
    let summary = verify_sources(
        "coverage",
        &fixture.0,
        &[fixture.0.join("src/main.rs")],
        false,
    );
    fs::remove_file(fixture.0.join("src/main.rs")).unwrap();
    let workspace = cargo_nirdosha::WorkspaceSummary {
        packages: vec![summary],
        all_packages: vec!["coverage".into()],
    };
    assert!(workspace.write_reports(&fixture.0.join("target"), false).is_err());
}

#[test]
fn edited_coverage_and_outside_source_paths_are_refused() {
    let fixture = Fixture::new(CLEAN);
    let (path, mut cert) = fixture.mint();
    cert.verification["coverage"]["claims"][0]["outcome"] = "unsupported".into();
    fs::write(&path, serde_json::to_vec(&cert).unwrap()).unwrap();
    let result = fixture.cli(&path, &["source_scan"]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("binding"));

    let (_, mut cert) = fixture.mint();
    cert.sources[0].path = "../outside.rs".into();
    let cert = Certificate::new(cert.subject, cert.tool, cert.sources, cert.verification);
    fs::write(&path, serde_json::to_vec(&cert).unwrap()).unwrap();
    let result = fixture.cli(&path, &["source_scan"]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("invalid source path"));
}
