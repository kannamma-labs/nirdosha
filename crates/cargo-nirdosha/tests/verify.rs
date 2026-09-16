//! Verify the verifier: fixtures must produce exactly the findings the
//! product story promises.

use cargo_nirdosha::{verify_sources, ContractInfo, Severity};
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(name)
}

fn run(names: &[&str]) -> cargo_nirdosha::ScanSummary {
    let files: Vec<PathBuf> = names.iter().map(|n| fixture(n)).collect();
    verify_sources("fixture-pkg", std::path::Path::new("."), &files, false)
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
    let s = verify_sources("fixture-pkg", std::path::Path::new("."), &files, true);
    assert!(
        s.violations()
            .iter()
            .any(|f| f.message.contains("strict mode")),
        "strict mode must flag uncontracted pub fns"
    );
    // and non-strict stays silent for the same file
    let s = verify_sources("fixture-pkg", std::path::Path::new("."), &files, false);
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

// ---------------------------------------------------------------------------
// Stage 1.5: in-dialect detection + bench gates
// ---------------------------------------------------------------------------

#[test]
fn in_dialect_detection() {
    let tmp = std::env::temp_dir().join(format!("nirdosha-detect-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("src")).unwrap();
    // 1. plain Rust crate — NOT in dialect, contract strings in source
    //    don't opt a crate in (the toolchain itself mentions them).
    std::fs::write(
        tmp.join("Cargo.toml"),
        "[package]\nname = \"plain\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(
        tmp.join("src/main.rs"),
        "/// nirdosha:contract {\"effects\":[\"pure\"]}\nfn f() {}\nfn main() {}",
    )
    .unwrap();
    assert!(!cargo_nirdosha::in_dialect(&tmp));
    // 2. depending on nirdosha-rt — in dialect
    std::fs::write(
        tmp.join("Cargo.toml"),
        "[package]\nname = \"user\"\n\n[dependencies]\nnirdosha-rt = \"0.1\"\n",
    )
    .unwrap();
    assert!(cargo_nirdosha::in_dialect(&tmp));
    // ...but a *name* containing the substring must not self-opt-in
    std::fs::write(
        tmp.join("Cargo.toml"),
        "[package]\nname = \"nirdosha-rt\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    assert!(!cargo_nirdosha::in_dialect(&tmp));
    // 3. zero-dependency explicit opt-in — in dialect
    std::fs::write(
        tmp.join("Cargo.toml"),
        "[package]\nname = \"zero\"\n\n[package.metadata.nirdosha-rt]\ndialect = true\n",
    )
    .unwrap();
    assert!(cargo_nirdosha::in_dialect(&tmp));
    // 4. the toolchain exemption wins over everything
    std::fs::write(
        tmp.join("Cargo.toml"),
        "[package]\nname = \"t\"\n\n[dependencies]\nnirdosha-rt = \"0.1\"\n\n[package.metadata.nirdosha-rt]\ntoolchain = true\n",
    )
    .unwrap();
    assert!(!cargo_nirdosha::in_dialect(&tmp));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn bench_gates_hold_and_breach() {
    use cargo_nirdosha::{BenchEvent, ContractForm};
    use nirdosha_contract_core::model::{Contract, Nfr};
    use std::path::PathBuf;

    let contract = ContractInfo {
        file: PathBuf::from("src/main.rs"),
        line: 3,
        function: "compute_payroll".to_string(),
        form: ContractForm::Attribute,
        contract: Contract {
            effects: Some(vec!["pure".into()]),
            requires: None,
            nfr: Some(Nfr {
                error_rate_max: None,
                throughput_min_per_sec: None,
                latency_ms: Some(10.0),
                concurrency_max: Some(4),
            }),
        },
    };

    // Fast calls: p95 under the limit — PASS
    let fast: Vec<BenchEvent> = (0..20)
        .map(|i| BenchEvent {
            function: "compute_payroll".into(),
            latency_ms: i as f64 * 0.1,
        })
        .collect();
    let verdicts = cargo_nirdosha::evaluate_bench(&fast, std::slice::from_ref(&contract));
    assert_eq!(verdicts.len(), 1);
    assert!(verdicts[0].ok, "fast calls must pass the gate");
    assert_eq!(verdicts[0].calls, 20);

    // One slow call among 21 must NOT breach — p95 is robust to a
    // single outlier, and that's the point of using p95, not max.
    let mut outlier = fast.clone();
    outlier.push(BenchEvent {
        function: "compute_payroll".into(),
        latency_ms: 500.0,
    });
    let verdicts = cargo_nirdosha::evaluate_bench(&outlier, std::slice::from_ref(&contract));
    assert!(verdicts[0].ok, "one outlier must not fail a p95 gate");
    assert_eq!(verdicts[0].max_ms, 500.0);

    // But 3 slow of 21 (> 5%) drags p95 over the limit — FAIL
    let mut slow = fast.clone();
    for _ in 0..3 {
        slow.push(BenchEvent {
            function: "compute_payroll".into(),
            latency_ms: 500.0,
        });
    }
    let verdicts = cargo_nirdosha::evaluate_bench(&slow, std::slice::from_ref(&contract));
    assert!(!verdicts[0].ok, "repeated 500ms calls against a 10ms limit must fail");

    // Zero calls — unmeasured is not passing
    let verdicts = cargo_nirdosha::evaluate_bench(&[], std::slice::from_ref(&contract));
    assert!(!verdicts[0].ok, "an unmeasured SLA must not count as passing");
    assert_eq!(verdicts[0].calls, 0);
}
