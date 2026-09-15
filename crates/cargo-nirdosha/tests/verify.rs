//! Verify the verifier: fixtures must produce exactly the findings the
//! product story promises.

use cargo_nirdosha::{verify_sources, Severity};
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(name)
}

fn run(names: &[&str]) -> cargo_nirdosha::ScanSummary {
    let files: Vec<PathBuf> = names.iter().map(|n| fixture(n)).collect();
    verify_sources("fixture-pkg", &files, false)
}

#[test]
fn honest_program_passes() {
    let s = run(&["clean.rs"]);
    assert!(
        s.violations().is_empty(),
        "honest fixture must pass, got: {:?}",
        s.findings
    );
    assert_eq!(s.contracts.len(), 2);
}

#[test]
fn lying_program_is_refused() {
    let s = run(&["lying.rs"]);
    let violations = s.violations();
    assert!(
        !violations.is_empty(),
        "a pure claim over file I/O must be a violation"
    );
    assert!(
        violations
            .iter()
            .any(|f| f.message.contains("std::fs::read_to_string")),
        "violation should name the impure call: {:?}",
        violations
            .iter()
            .map(|f| f.message.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        violations
            .iter()
            .any(|f| f.message.contains("Instant")),
        "violation should catch the clock: {:?}",
        violations
            .iter()
            .map(|f| f.message.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn typoed_contract_key_is_rejected() {
    let s = run(&["typo.rs"]);
    let violations = s.violations();
    assert!(
        violations.iter().any(|f| f.message.contains("effekts")),
        "unknown contract key must be named in the error: {:?}",
        violations
            .iter()
            .map(|f| f.message.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn dialect_restrictions_hold_without_contracts() {
    let s = run(&["unsafe_thread.rs"]);
    let violations = s.violations();
    assert!(
        violations
            .iter()
            .any(|f| f.message.contains("unsafe")),
        "unsafe must be denied: {:?}",
        violations
            .iter()
            .map(|f| f.message.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        violations
            .iter()
            .any(|f| f.message.contains("thread::spawn")),
        "raw threads must be denied: {:?}",
        violations
            .iter()
            .map(|f| f.message.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn strict_mode_demands_contracts_on_pub_fns() {
    let files = vec![fixture("uncontracted.rs")];
    let s = verify_sources("fixture-pkg", &files, true);
    assert!(
        s.violations()
            .iter()
            .any(|f| f.message.contains("strict mode")),
        "strict mode must flag uncontracted pub fns"
    );
    // and non-strict stays silent for the same file
    let s = verify_sources("fixture-pkg", &files, false);
    assert!(s.violations().is_empty());
}

#[test]
fn warnings_do_not_fail_the_build() {
    let s = run(&["warning_only.rs"]);
    assert!(s.violations().is_empty());
    assert!(
        s.findings
            .iter()
            .any(|f| matches!(f.severity, Severity::Warning)),
        "trait-decl pure claim should warn, not error"
    );
}