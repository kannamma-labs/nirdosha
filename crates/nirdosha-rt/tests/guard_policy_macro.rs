//! Tests that the RFC 0023 guard macros re-exported from `nirdosha_rt`
//! compile and parse in the v2 dialect. This is the correct place for
//! guard-surface smoke tests, not the deprecated `.nir` compiler.

#[allow(dead_code)]
#[derive(Clone)]
struct CardNumber {
    value: String,
}

#[allow(dead_code)]
#[derive(Clone)]
struct Cvv {
    value: String,
}

#[allow(dead_code)]
#[derive(Clone)]
struct CustomerAccount {
    status: String,
    balance: i64,
    credit_limit: i64,
}

nirdosha_rt::guard_policy! {
    allow "support-view-card" when action == "read" && resource == "payment_card"
    mask(card_number, partial_last4)
    mask(cvv, full)
}

nirdosha_rt::guard_policy! {
    allow "limit-adjust" when action == "update" && resource == "account"
    requires field(status).in(["pending", "active"])
    ensures field(balance) <= field(credit_limit)
    field_policy {
        required(status)
        allowed(balance)
        forbidden(credit_limit)
    }
}

// Regression coverage for the three shapes that previously failed to parse
// at all: `for <Role>` (dead branch — `for` is a keyword, not an `Ident`),
// `action in [...]`, and `resource in [...]`. All three appear throughout
// `examples/rtm/roles-N-guard_policy.md`; none compiled before this fix.

nirdosha_rt::guard_policy! {
    allow "auditor-read" for Auditor
    when action == "read" && resource == "alert"
}

nirdosha_rt::guard_policy! {
    allow "lead-bulk-alert" for ComplianceLead, Auditor
    when action == "update" && resource == "alert"
}

nirdosha_rt::guard_policy! {
    deny "auditor-no-write" for Auditor
    when action in ["create", "update", "delete", "migrate"]
}

nirdosha_rt::guard_policy! {
    allow "auditor-read-many" for Auditor
    when action == "read" && resource in ["alert", "case", "transaction"]
}

nirdosha_rt::guard_policy! {
    allow "wide" for Auditor
    when action in ["read", "export"] && resource in ["alert", "case"]
}

#[test]
fn guard_policy_macro_compiles() {
    // The test is that the macro expanded without a compile error.
    assert!(true);
}

#[test]
fn registered_policy_subjects_are_not_empty_for_a_for_role_policy() {
    let found = nirdosha_guard_registry::POLICIES
        .iter()
        .find(|p| p.id == "auditor-read")
        .expect("auditor-read must be registered");
    assert_eq!(found.subjects, &["Auditor"]);
    assert_eq!(found.action, "read");
    assert_eq!(found.resource, "alert");
}

#[test]
fn multi_role_for_clause_registers_every_role() {
    let found = nirdosha_guard_registry::POLICIES
        .iter()
        .find(|p| p.id == "lead-bulk-alert")
        .expect("lead-bulk-alert must be registered");
    assert_eq!(found.subjects, &["ComplianceLead", "Auditor"]);
}

#[test]
fn action_in_list_registers_one_record_per_action() {
    let matches: Vec<_> = nirdosha_guard_registry::POLICIES
        .iter()
        .filter(|p| p.id == "auditor-no-write")
        .collect();
    assert_eq!(matches.len(), 4, "expected one registration per action in the list");
    let actions: std::collections::BTreeSet<_> = matches.iter().map(|p| p.action).collect();
    assert_eq!(
        actions,
        std::collections::BTreeSet::from(["create", "update", "delete", "migrate"])
    );
    assert!(matches.iter().all(|p| p.resource == "*"));
}

#[test]
fn resource_in_list_registers_one_record_per_resource() {
    let matches: Vec<_> = nirdosha_guard_registry::POLICIES
        .iter()
        .filter(|p| p.id == "auditor-read-many")
        .collect();
    assert_eq!(matches.len(), 3);
    let resources: std::collections::BTreeSet<_> = matches.iter().map(|p| p.resource).collect();
    assert_eq!(
        resources,
        std::collections::BTreeSet::from(["alert", "case", "transaction"])
    );
}

#[test]
fn action_and_resource_lists_cross_product_correctly() {
    let matches: Vec<_> = nirdosha_guard_registry::POLICIES
        .iter()
        .filter(|p| p.id == "wide")
        .collect();
    assert_eq!(matches.len(), 4, "2 actions x 2 resources = 4 registrations");
    let pairs: std::collections::BTreeSet<_> =
        matches.iter().map(|p| (p.action, p.resource)).collect();
    assert_eq!(
        pairs,
        std::collections::BTreeSet::from([
            ("read", "alert"),
            ("read", "case"),
            ("export", "alert"),
            ("export", "case"),
        ])
    );
}

#[test]
fn policy_registration_round_trips_through_to_candidate_and_is_role_matched() {
    use nirdosha_guard_core::evaluator::{evaluate, PolicyEffect};
    use nirdosha_guard_core::{
        Action, Classification, Destination, Environment, EvaluationContext, PaginationMode,
        Purpose, QueryShape, Subject, Tenant,
    };

    let registration = nirdosha_guard_registry::POLICIES
        .iter()
        .find(|p| p.id == "auditor-read")
        .expect("auditor-read must be registered");
    let candidate = registration.to_candidate().expect("action == \"read\" must parse");
    assert_eq!(candidate.effect, PolicyEffect::Allow);
    assert_eq!(candidate.subjects, vec!["Auditor".to_string()]);
    assert_eq!(candidate.action, Action::Read);
    assert_eq!(candidate.resource, "alert");

    let context = EvaluationContext {
        subject: Subject {
            id: "u1".into(),
            roles: vec!["Auditor".into()],
            claims: vec![],
            clearance: Classification::Internal,
        },
        tenant: Tenant("t1".into()),
        entity: "alert".into(),
        dataset: "pg".into(),
        action: Action::Read,
        destination: Destination::Browser,
        environment: Environment {
            env: "test".into(),
            ip: None,
            geo: None,
            device_posture: None,
            session_freshness: None,
        },
        time_bucket: "now".into(),
        query_shape: QueryShape {
            verbs: vec![],
            aggregate: None,
            grouping_keys: vec![],
            subject_dimension: None,
            ordering: vec![],
            pagination: PaginationMode::LimitOnly { limit: 10 },
        },
        purpose: Purpose("audit".into()),
        policy_version: "v1".into(),
    };
    let result = evaluate(&context, &[candidate.clone()]);
    assert_eq!(result.decision, nirdosha_guard_core::Decision::Allow);

    // A different role must NOT be matched by a `for Auditor`-scoped policy.
    let mut other_role_context = context.clone();
    other_role_context.subject.roles = vec!["Analyst".into()];
    let denied = evaluate(&other_role_context, &[candidate]);
    assert!(matches!(denied.decision, nirdosha_guard_core::Decision::Deny { .. }));
}
