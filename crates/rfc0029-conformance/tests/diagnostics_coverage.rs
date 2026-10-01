//! Negative-vector coverage matrix, generated from the corpus itself rather
//! than hand-maintained prose. This is the "complete negative-vector
//! coverage" freeze contract, made executable: every diagnostic the closed
//! `AdmissionDiagnostic`/`ReviewFinding` taxonomies declare must actually be
//! reachable by at least one of the 50 canonical fixtures, computed fresh
//! each run -- a diagnostic declared in code but never exercised by any
//! fixture is exactly as much a coverage gap as a missing enum variant, and
//! this test fails loudly on either direction of that gap.

use rfc0029_conformance::{
    evaluate_admission, evaluate_review, parse, AdmissionDiagnostic, ComputedReview,
    ReviewFinding,
};
use serde::Deserialize;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
struct Manifest {
    fixtures: Vec<Entry>,
}
#[derive(Deserialize)]
struct Entry {
    json_path: String,
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rfcs/fixtures/0029-canonical")
}

// `CapabilityEffectMismatch` cannot be reached by any fixture the symbolic
// YAML -> generate.py pipeline can produce: whenever a fixture has both a
// Capability and an Effect node, generate.py always derives
// `admission_policy.effect_class` from the Effect's `effect_id` (effects
// take precedence over capabilities in that derivation), and
// `evaluate_admission` hard-errors *before* reaching this diagnostic if
// `admission_policy.effect_class` disagrees with the Capability's own
// `effect_class`. Combined, those two facts make it impossible for a
// generator-produced fixture to reach the soft `CapabilityEffectMismatch`
// diagnostic rather than the harder `InvalidGraph` error -- a real,
// deliberate architectural property (the generator prevents this
// inconsistency from ever being representable), not a coverage gap to
// paper over with a contrived fixture. It is covered instead by a direct
// Rust-level test that constructs the inconsistency programmatically:
// `capability_binding_rejects_profile_and_effect_mismatch` in
// `tests/vectors.rs`.
const UNREACHABLE_BY_ANY_FIXTURE: &[AdmissionDiagnostic] =
    &[AdmissionDiagnostic::CapabilityEffectMismatch];

#[test]
fn every_admission_diagnostic_is_reachable_by_at_least_one_fixture_or_a_documented_unit_test() {
    let root = root();
    let manifest: Manifest =
        serde_json::from_slice(&fs::read(root.join("manifest.json")).unwrap()).unwrap();

    let mut reached: BTreeSet<AdmissionDiagnostic> = BTreeSet::new();
    for entry in &manifest.fixtures {
        let fixture = parse(&fs::read(root.join(&entry.json_path)).unwrap()).unwrap();
        if fixture.input.nodes.is_empty() {
            continue;
        }
        if let Ok(got) = evaluate_admission(&fixture) {
            if let Some(d) = got.diagnostic {
                reached.insert(d);
            }
        }
    }

    let missing: Vec<String> = AdmissionDiagnostic::ALL
        .iter()
        .filter(|d| !reached.contains(d) && !UNREACHABLE_BY_ANY_FIXTURE.contains(d))
        .map(|d| d.to_string())
        .collect();
    assert!(
        missing.is_empty(),
        "AdmissionDiagnostic variants declared in code but not reached by any of the 50 \
         canonical fixtures, and not in the documented UNREACHABLE_BY_ANY_FIXTURE exception \
         list (a real coverage gap -- add a fixture that reaches them): {missing:?}"
    );
}

#[test]
fn every_review_finding_is_reachable_by_at_least_one_fixture() {
    let root = root();
    let manifest: Manifest =
        serde_json::from_slice(&fs::read(root.join("manifest.json")).unwrap()).unwrap();

    // Unlike admission/capability, review predicates are meaningful on
    // node-less fixtures too (R1-R9 are pure review-contract test cases
    // with no graph at all) -- do not skip them here.
    let mut reached: BTreeSet<ReviewFinding> = BTreeSet::new();
    for entry in &manifest.fixtures {
        let fixture = parse(&fs::read(root.join(&entry.json_path)).unwrap()).unwrap();
        if let Ok(ComputedReview::Fail(f) | ComputedReview::Indeterminate(f)) =
            evaluate_review(&fixture)
        {
            reached.insert(f);
        }
    }

    let missing: Vec<String> = ReviewFinding::ALL
        .iter()
        .filter(|f| !reached.contains(f))
        .map(|f| f.to_string())
        .collect();
    assert!(
        missing.is_empty(),
        "ReviewFinding variants declared in code but not reached by any of the 50 canonical \
         fixtures: {missing:?}"
    );
}

#[test]
fn admission_diagnostic_and_review_finding_string_forms_are_all_distinct() {
    // A closed taxonomy is only meaningfully "stable" if its wire-format
    // strings can't collide with each other -- two different variants
    // rendering to the same string would silently merge them from any
    // consumer that only sees the JSON string, not the Rust type.
    let mut seen = BTreeSet::new();
    for d in AdmissionDiagnostic::ALL {
        assert!(seen.insert(d.to_string()), "duplicate diagnostic string: {d}");
    }
    for f in ReviewFinding::ALL {
        assert!(seen.insert(f.to_string()), "duplicate finding string: {f}");
    }
}
