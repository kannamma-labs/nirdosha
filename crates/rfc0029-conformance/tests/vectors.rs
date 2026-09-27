use rfc0029_conformance::{
    canonical_bytes, compute_influence, evaluate_admission, evaluate_fact_provenance,
    evaluate_review, parse, sha256, AdmissionDiagnostic, ComputedFactProvenance, ComputedReview,
    Error, ModelLevel, ReviewFinding, ReviewMode, SCHEMA_VERSION,
};
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
struct Manifest {
    schema_version: String,
    fixture_count: usize,
    fixtures: Vec<Entry>,
}
#[derive(Deserialize)]
struct Entry {
    fixture_id: String,
    json_path: String,
    jcs_path: String,
    sha256: String,
    bytes: usize,
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rfcs/fixtures/0029-canonical")
}

#[test]
fn all_vectors_round_trip_to_independent_jcs_and_hashes() {
    let root = root();
    let manifest: Manifest =
        serde_json::from_slice(&fs::read(root.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest.schema_version, SCHEMA_VERSION);
    assert_eq!(manifest.fixture_count, 107);
    assert_eq!(manifest.fixtures.len(), 107);
    for entry in manifest.fixtures {
        let readable = fs::read(root.join(&entry.json_path)).unwrap();
        let fixture = parse(&readable).unwrap_or_else(|e| panic!("{}: {e}", entry.fixture_id));
        assert_eq!(fixture.fixture_id, entry.fixture_id);
        let encoded = canonical_bytes(&fixture).unwrap();
        assert_eq!(
            encoded,
            fs::read(root.join(&entry.jcs_path)).unwrap(),
            "{}",
            entry.fixture_id
        );
        assert_eq!(encoded.len(), entry.bytes, "{}", entry.fixture_id);
        assert_eq!(
            sha256(&fixture).unwrap(),
            entry.sha256,
            "{}",
            entry.fixture_id
        );
    }
}

#[test]
fn influence_is_computed_from_inputs_for_every_graph_fixture() {
    let root = root();
    let manifest: Manifest =
        serde_json::from_slice(&fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let mut checked = 0;
    for entry in manifest.fixtures {
        let fixture = parse(&fs::read(root.join(entry.json_path)).unwrap()).unwrap();
        if fixture.input.nodes.is_empty()
            || fixture.expected.model_level == ModelLevel::NotEvaluated
        {
            continue;
        }
        let got = compute_influence(&fixture).unwrap();
        assert_eq!(
            got.provenance, fixture.expected.provenance,
            "{} provenance",
            fixture.fixture_id
        );
        assert_eq!(
            got.level, fixture.expected.model_level,
            "{} level",
            fixture.fixture_id
        );
        assert_eq!(
            got.model_authorized, fixture.expected.model_authorized,
            "{} authority",
            fixture.fixture_id
        );
        checked += 1;
    }
    assert_eq!(checked, 91);
}

#[test]
fn rejects_unknown_top_level_field() {
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(root().join("vectors/EQ1A.json")).unwrap()).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("future_field".into(), true.into());
    assert!(parse(&serde_json::to_vec(&value).unwrap()).is_err());
}

#[test]
fn rejects_version_downgrade() {
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(root().join("vectors/EQ1A.json")).unwrap()).unwrap();
    value["schema_version"] = "rfc0029.influence-review.fixture.v0".into();
    assert!(matches!(
        parse(&serde_json::to_vec(&value).unwrap()),
        Err(Error::UnsupportedSchemaVersion(_))
    ));
}

#[test]
fn rejects_unknown_enum_and_dangling_edge() {
    let bytes = fs::read(root().join("vectors/EQ1A.json")).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value["input"]["edges"][0]["kind"] = "MagicFlow".into();
    assert!(parse(&serde_json::to_vec(&value).unwrap()).is_err());
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value["input"]["edges"][0]["target"] = "node:missing".into();
    assert!(matches!(
        parse(&serde_json::to_vec(&value).unwrap()),
        Err(Error::InvalidGraph(_))
    ));
}

#[test]
fn rejects_cycles() {
    let bytes = fs::read(root().join("vectors/EQ1A.json")).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let reverse =
        serde_json::json!({"source":"node:EQ1A:d","kind":"DataFlow","target":"node:EQ1A:m"});
    value["input"]["edges"]
        .as_array_mut()
        .unwrap()
        .push(reverse);
    assert!(matches!(
        parse(&serde_json::to_vec(&value).unwrap()),
        Err(Error::InvalidGraph(_))
    ));
}

#[test]
fn review_results_are_computed_for_nine_review_fixtures() {
    let cases = [
        ("R1_ACK", ComputedReview::Pass(ReviewMode::Acknowledgement)),
        (
            "R2_ACK_FOR_APPROVAL",
            ComputedReview::Fail(ReviewFinding::ReviewModeInsufficient),
        ),
        (
            "R3_NOT_ACCESSED",
            ComputedReview::Fail(ReviewFinding::RequiredEvidenceNotAccessed),
        ),
        (
            "R4_CONFLICT",
            ComputedReview::Fail(ReviewFinding::IndependenceConflict),
        ),
        (
            "R5_PASS",
            ComputedReview::Pass(ReviewMode::IndependentDecision),
        ),
        (
            "R6_STALE",
            ComputedReview::Fail(ReviewFinding::ReviewPredicateFailedClosed),
        ),
        (
            "R7_CAPACITY",
            ComputedReview::Fail(ReviewFinding::CapacityExceeded),
        ),
        ("R8_MONITOR_MISSING", ComputedReview::NotEvaluated),
        (
            "R9_EVALUATION_MISSING",
            ComputedReview::Fail(ReviewFinding::ReviewPredicateMissing),
        ),
    ];
    for (id, expected) in cases {
        let fixture = parse(&fs::read(root().join(format!("vectors/{id}.json"))).unwrap()).unwrap();
        assert_eq!(evaluate_review(&fixture).unwrap(), expected, "{id}");
    }
}

#[test]
fn rejects_duplicate_json_members_at_any_depth() {
    let duplicate = br#"{"schema_version":"rfc0029.influence-review.fixture.v1","fixture_id":"x","fixture_id":"y"}"#;
    assert!(matches!(parse(duplicate), Err(Error::DuplicateMember(name)) if name == "fixture_id"));
}

#[test]
fn review_rejects_duplicate_evaluations_and_wrong_authority() {
    let path = root().join("vectors/R1_ACK.json");
    let mut fixture = parse(&fs::read(path).unwrap()).unwrap();
    let evaluations = fixture
        .input
        .parameters
        .get_mut("evaluations")
        .unwrap()
        .as_array_mut()
        .unwrap();
    evaluations.push(evaluations[0].clone());
    assert!(
        matches!(evaluate_review(&fixture), Err(Error::InvalidReview(message)) if message.contains("duplicate predicate"))
    );

    let mut fixture = parse(&fs::read(root().join("vectors/R1_ACK.json")).unwrap()).unwrap();
    let predicate = fixture
        .input
        .catalog_entries
        .iter_mut()
        .find(|e| e.name == "authenticated")
        .unwrap();
    predicate
        .value
        .insert("authority".into(), "authority:wrong".into());
    assert!(
        matches!(evaluate_review(&fixture), Err(Error::InvalidReview(message)) if message.contains("authority mismatch"))
    );
}

#[test]
fn review_rejects_bad_versions_authorities_and_commitments() {
    let bytes = fs::read(root().join("vectors/R1_ACK.json")).unwrap();
    for (field, value, needle) in [
        (
            "definition_version",
            serde_json::json!(2),
            "version mismatch",
        ),
        (
            "evaluator_authority",
            serde_json::json!("authority:wrong"),
            "evaluator authority mismatch",
        ),
        (
            "evidence_commitment",
            serde_json::json!(""),
            "missing commitment",
        ),
    ] {
        let mut fixture = parse(&bytes).unwrap();
        fixture.input.parameters.get_mut("evaluations").unwrap()[0][field] = value;
        assert!(
            matches!(evaluate_review(&fixture), Err(Error::InvalidReview(message)) if message.contains(needle)),
            "{field}"
        );
    }
}

#[test]
fn revoked_review_evaluation_fails_closed() {
    let mut fixture = parse(&fs::read(root().join("vectors/R1_ACK.json")).unwrap()).unwrap();
    fixture.input.parameters.get_mut("evaluations").unwrap()[0]["revoked"] = true.into();
    assert_eq!(
        evaluate_review(&fixture).unwrap(),
        ComputedReview::Fail(ReviewFinding::ReviewPredicateFailedClosed)
    );
}

#[test]
fn admission_and_capability_results_are_computed_from_inputs() {
    let root = root();
    let manifest: Manifest =
        serde_json::from_slice(&fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let mut admissions = 0;
    let mut capabilities = 0;
    for entry in manifest.fixtures {
        let fixture = parse(&fs::read(root.join(entry.json_path)).unwrap()).unwrap();
        if fixture.input.nodes.is_empty() {
            continue;
        }
        let got = evaluate_admission(&fixture).unwrap();
        if fixture.expected.admission.status != "not_evaluated" {
            assert_eq!(
                got.status, fixture.expected.admission.status,
                "{} admission",
                fixture.fixture_id
            );
            assert_eq!(
                got.diagnostic.map(|d| d.to_string()),
                fixture.expected.admission.diagnostic,
                "{} diagnostic",
                fixture.fixture_id
            );
            admissions += 1;
        }
        if fixture.expected.capability.status != "not_evaluated" {
            assert_eq!(
                got.capability_status, fixture.expected.capability.status,
                "{} capability",
                fixture.fixture_id
            );
            assert_eq!(
                got.effect_class, fixture.expected.capability.effect_class,
                "{} effect",
                fixture.fixture_id
            );
            capabilities += 1;
        }
    }
    assert_eq!(admissions, 81);
    assert_eq!(capabilities, 66);
}

#[test]
fn fact_provenance_results_are_computed_for_six_lineage_fixtures() {
    // RFC 0029 §36.8 dependency-invalidation / RFC 0029.a §13.1 semantic-
    // input-contract provenance: a model-produced Fact consumed by a later,
    // separate Decision must carry checkable lineage (producer identity,
    // freshness, revocation, authorized consumer) — this is deliberately a
    // check over Fact attribute *values*, orthogonal to compute_influence's
    // topology-only fold.
    let cases = [
        ("L8", ComputedFactProvenance::Satisfied),
        ("L9", ComputedFactProvenance::Failed(AdmissionDiagnostic::FactStale)),
        ("L10", ComputedFactProvenance::Failed(AdmissionDiagnostic::FactRevoked)),
        (
            "L11",
            ComputedFactProvenance::Failed(AdmissionDiagnostic::FactWrongModel),
        ),
        (
            "L12",
            ComputedFactProvenance::Failed(AdmissionDiagnostic::FactWrongDeployment),
        ),
        (
            "L13",
            ComputedFactProvenance::Failed(AdmissionDiagnostic::FactUnauthorizedConsumer),
        ),
    ];
    for (id, expected) in cases {
        let fixture = parse(&fs::read(root().join(format!("vectors/{id}.json"))).unwrap()).unwrap();
        assert_eq!(evaluate_fact_provenance(&fixture).unwrap(), expected, "{id}");
    }
    let unrelated = parse(&fs::read(root().join("vectors/L1.json")).unwrap()).unwrap();
    assert_eq!(
        evaluate_fact_provenance(&unrelated).unwrap(),
        ComputedFactProvenance::NotApplicable
    );
}

#[test]
fn every_effect_id_has_one_reversibility_class_across_the_whole_corpus() {
    // RFC 0029 §36.2: a canonical effect identity has one closure meaning.
    // Nothing in the schema or generator currently cross-checks this across
    // fixtures on its own, so two fixtures could otherwise silently disagree
    // about whether the same effect is reversible, compensatable, or
    // irreversible. Caught for real here: an earlier pass of the logistics
    // fixtures gave `cargo.release` two different classes in two fixtures.
    let root = root();
    let manifest: Manifest =
        serde_json::from_slice(&fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let mut classes: std::collections::BTreeMap<String, (String, String)> =
        std::collections::BTreeMap::new();
    for entry in manifest.fixtures {
        let fixture = parse(&fs::read(root.join(entry.json_path)).unwrap()).unwrap();
        for node in &fixture.input.nodes {
            if node.kind != rfc0029_conformance::NodeKind::Effect {
                continue;
            }
            let effect_id = node
                .attributes
                .get("effect_id")
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| panic!("{}: effect node missing effect_id", entry.fixture_id));
            let reversibility = node
                .attributes
                .get("reversibility_class")
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| panic!("{}: effect node missing reversibility_class", entry.fixture_id));
            if let Some((seen_class, seen_fixture)) = classes.get(effect_id) {
                assert_eq!(
                    seen_class, reversibility,
                    "effect_id {effect_id:?} is {seen_class:?} in {seen_fixture} but {reversibility:?} in {}",
                    entry.fixture_id
                );
            } else {
                classes.insert(
                    effect_id.to_string(),
                    (reversibility.to_string(), entry.fixture_id.clone()),
                );
            }
        }
    }
    assert!(!classes.is_empty());
}

#[test]
fn capability_binding_rejects_profile_and_effect_mismatch() {
    let bytes = fs::read(root().join("vectors/M2.json")).unwrap();
    let mut fixture = parse(&bytes).unwrap();
    fixture
        .input
        .parameters
        .get_mut("admission_policy")
        .unwrap()["effect_class"] = "machine.restart".into();
    assert!(
        matches!(evaluate_admission(&fixture), Err(Error::InvalidGraph(message)) if message.contains("profile effect mismatch"))
    );

    let mut fixture = parse(&bytes).unwrap();
    let effect = fixture
        .input
        .nodes
        .iter_mut()
        .find(|n| n.kind == rfc0029_conformance::NodeKind::Effect)
        .unwrap();
    effect
        .attributes
        .insert("effect_id".into(), "machine.restart".into());
    let result = evaluate_admission(&fixture).unwrap();
    assert_eq!(result.status, "rejected");
    assert_eq!(result.diagnostic, Some(AdmissionDiagnostic::CapabilityEffectMismatch));
    assert_eq!(result.capability_status, "not_issued");
}
