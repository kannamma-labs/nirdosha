//! `AdmissionReportV1` generalized beyond the `funds.reserve` pilot
//! (readiness matrix §7/§9 item 8): `compile_from_fixture` must run over
//! any canonical fixture, distinguishing `Accepted`/`Rejected`/
//! `Indeterminate` from `Unsupported` rather than force-fitting every
//! input into the first three.

use rfc0029_conformance::pilot::report::{compile_from_fixture, OverallVerdict, RequirementOutcome};
use rfc0029_conformance::parse;
use serde::Deserialize;
use std::{fs, path::Path};

fn root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rfcs/fixtures/0029-canonical")
}

#[derive(Deserialize)]
struct Manifest {
    fixtures: Vec<Entry>,
}
#[derive(Deserialize)]
struct Entry {
    fixture_id: String,
    json_path: String,
}

#[test]
fn every_canonical_fixture_compiles_to_a_report_with_no_panic() {
    let root = root();
    let manifest: Manifest =
        serde_json::from_slice(&fs::read(root.join("manifest.json")).unwrap()).unwrap();
    assert!(!manifest.fixtures.is_empty());
    for entry in &manifest.fixtures {
        let fixture = parse(&fs::read(root.join(&entry.json_path)).unwrap()).unwrap();
        let report = compile_from_fixture(&fixture);
        assert_eq!(report.policy_id, format!("policy:rfc0029-fixture:{}:v1", entry.fixture_id));
    }
}

#[test]
fn node_less_fixtures_are_unsupported_not_indeterminate() {
    // R1_ACK is a pure review-contract fixture with no graph and no
    // dependency/monitor parameter -- this report compiler genuinely has
    // no admission decision to describe for it.
    let root = root();
    let fixture = parse(&fs::read(root.join("vectors/R1_ACK.json")).unwrap()).unwrap();
    let report = compile_from_fixture(&fixture);
    assert_eq!(report.overall, OverallVerdict::Unsupported);
}

#[test]
fn a_distributed_finality_abort_fixture_reports_rejected_not_unsupported() {
    let root = root();
    let fixture = parse(&fs::read(root.join("vectors/INV_PRECOMMIT.json")).unwrap()).unwrap();
    let report = compile_from_fixture(&fixture);
    assert_eq!(report.overall, OverallVerdict::Rejected);
    let check = report
        .requirements
        .iter()
        .find(|r| r.id == "distributed-finality-declared")
        .unwrap();
    assert_eq!(check.outcome, RequirementOutcome::Failed);
}

#[test]
fn a_distributed_finality_reconcile_fixture_reports_indeterminate_not_unsupported() {
    let root = root();
    let fixture = parse(&fs::read(root.join("vectors/INV_EXTERNAL_UNKNOWN.json")).unwrap()).unwrap();
    let report = compile_from_fixture(&fixture);
    assert_eq!(report.overall, OverallVerdict::Indeterminate);
    let check = report
        .requirements
        .iter()
        .find(|r| r.id == "distributed-finality-declared")
        .unwrap();
    assert_eq!(check.outcome, RequirementOutcome::Indeterminate);
}

#[test]
fn a_monitor_health_unknown_fixture_reports_indeterminate_not_unsupported() {
    let root = root();
    let fixture = parse(&fs::read(root.join("vectors/R8_MONITOR_MISSING.json")).unwrap()).unwrap();
    let report = compile_from_fixture(&fixture);
    assert_eq!(report.overall, OverallVerdict::Indeterminate);
    let check = report
        .requirements
        .iter()
        .find(|r| r.id == "monitor-health-declared")
        .unwrap();
    assert_eq!(check.note.as_deref(), Some("declared monitor health: unknown"));
}

#[test]
fn an_accepted_fixture_reports_accepted_with_satisfied_requirements() {
    let root = root();
    let fixture = parse(&fs::read(root.join("vectors/L8.json")).unwrap()).unwrap();
    let report = compile_from_fixture(&fixture);
    assert_eq!(report.overall, OverallVerdict::Accepted);
    assert_eq!(report.effect_closure, vec!["cargo.release".to_string()]);
    let admission_decided = report
        .requirements
        .iter()
        .find(|r| r.id == "admission-decided")
        .unwrap();
    assert_eq!(admission_decided.outcome, RequirementOutcome::Satisfied);
}

#[test]
fn a_rejected_fixture_reports_rejected_with_the_diagnostic_in_the_note() {
    let root = root();
    let fixture = parse(&fs::read(root.join("vectors/L10.json")).unwrap()).unwrap();
    let report = compile_from_fixture(&fixture);
    assert_eq!(report.overall, OverallVerdict::Rejected);
    let admission_decided = report
        .requirements
        .iter()
        .find(|r| r.id == "admission-decided")
        .unwrap();
    assert_eq!(admission_decided.outcome, RequirementOutcome::Failed);
    assert_eq!(admission_decided.note.as_deref(), Some("FactRevoked"));
    let fact_check = report
        .requirements
        .iter()
        .find(|r| r.id == "fact-provenance-checked")
        .unwrap();
    assert_eq!(fact_check.outcome, RequirementOutcome::Failed);
    assert_eq!(fact_check.note.as_deref(), Some("FactRevoked"));
}

#[test]
fn an_indeterminate_fixture_reports_indeterminate_not_unsupported() {
    // K1_STRUCTURAL_ONLY has a graph (so it is "supported"), but its own
    // semantics are genuinely ambiguous -- Indeterminate, distinct from
    // Unsupported's "not covered by this compiler at all."
    let root = root();
    let fixture = parse(&fs::read(root.join("vectors/K1_STRUCTURAL_ONLY.json")).unwrap()).unwrap();
    let report = compile_from_fixture(&fixture);
    assert_eq!(report.overall, OverallVerdict::Indeterminate);
    let influence_check = report
        .requirements
        .iter()
        .find(|r| r.id == "influence-computed")
        .unwrap();
    assert_eq!(influence_check.outcome, RequirementOutcome::Indeterminate);
}

#[test]
fn report_compilation_is_byte_deterministic_across_independent_compiles() {
    let root = root();
    let fixture = parse(&fs::read(root.join("vectors/L8.json")).unwrap()).unwrap();
    let a = serde_json::to_vec(&compile_from_fixture(&fixture)).unwrap();
    let b = serde_json::to_vec(&compile_from_fixture(&fixture)).unwrap();
    assert_eq!(a, b);
}
