//! `workflow! { .. }` (issue #70's "item 4" gap / issue #73's own
//! conformance mechanism): a real state/event/transition graph,
//! independent of the `against = ..` spec-conformance check
//! (`tests/workflow_conformance.rs` covers that).

nirdosha_rt::workflow! {
    name: Onboarding,
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
fn advance_follows_the_declared_graph() {
    let s = advance_onboarding(OnboardingState::Draft, "Submit").unwrap();
    assert_eq!(s, OnboardingState::UnderReview);
    let s = advance_onboarding(s, "Approve").unwrap();
    assert_eq!(s, OnboardingState::Approved);
    assert!(onboarding_is_terminal(s));
    assert!(!onboarding_is_terminal(OnboardingState::Draft));
}

#[test]
fn rejection_loops_back_to_draft() {
    let s = advance_onboarding(OnboardingState::UnderReview, "Reject").unwrap();
    assert_eq!(s, OnboardingState::Draft);
}

#[test]
fn an_undeclared_transition_is_a_clean_error_not_a_panic() {
    let err = advance_onboarding(OnboardingState::Draft, "Approve").unwrap_err();
    assert!(err.contains("Draft"), "{err}");
    assert!(err.contains("Approve"), "{err}");
}
