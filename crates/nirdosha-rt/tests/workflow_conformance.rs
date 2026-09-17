//! `workflow!`'s `against = "spec.json"` clause (issue #73): a macro-
//! expansion-time conformance check against a PRD-extraction spec, the
//! same shape `.nir`'s `extraction_schema::ExtractedWorkflow` parses.
//! This file's own successful compilation *is* the positive test — a
//! mismatched spec is verified manually (no `trybuild` in this repo;
//! same practice `landing!`'s own validation errors were checked with)
//! since a real mismatch would fail this whole file's compilation.

nirdosha_rt::workflow! {
    name: Onboarding,
    against: "tests/fixtures/onboarding_spec.json",
    data: [ applicant_name: String ],
    states: [
        state Draft {
            on Submit -> UnderReview,
        },
        state UnderReview {
            on Approve -> Approved,
            on Reject -> Draft,
        },
        state Approved {
            terminal,
            on_entry: [notify],
        },
    ],
}

#[test]
fn a_workflow_matching_its_spec_compiles_and_still_behaves() {
    let s = advance_onboarding(OnboardingState::Draft, "Submit").unwrap();
    assert_eq!(s, OnboardingState::UnderReview);
}
