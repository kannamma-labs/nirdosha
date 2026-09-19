//! Tests that the RFC 0023 guard attribute macros (`#[dataset]`,
//! `#[relation]`, `#[classify]`, `#[materialize]`, `#[reference]`,
//! `#[mask_transform]`, `#[invariant]`, `#[purpose]`) compile when
//! attached to v2 Rust declarations.

#[allow(dead_code)]
#[nirdosha_rt::dataset(entity = "customer", store = "pg_primary")]
struct Customer {
    id: i64,
    name: String,
}

#[allow(dead_code)]
#[nirdosha_rt::classify(level = "CONFIDENTIAL")]
struct CardNumber {
    value: String,
}

#[allow(dead_code)]
#[nirdosha_rt::mask_transform(name = "partial_last4")]
fn partial_last4(c: CardNumber) -> CardNumber {
    c
}

#[allow(dead_code)]
#[nirdosha_rt::relation(source = "org_registry", cardinality = 500, ttl = 60)]
fn branches_under(user_id: i64) -> Vec<i64> {
    vec![user_id]
}

#[allow(dead_code)]
#[nirdosha_rt::materialize(relation = "visible_closure")]
struct VisibleClosure {
    id: i64,
}

#[allow(dead_code)]
#[nirdosha_rt::reference(field = "customer.status", store = "refdata")]
struct Refdata {
    code: String,
    label: String,
}

#[allow(dead_code)]
#[nirdosha_rt::invariant(name = "no_overdraft", schema = "account")]
fn no_overdraft(old: i64, new: i64) -> bool {
    new >= old
}

#[allow(dead_code)]
#[nirdosha_rt::purpose(code = "clinical_trial_23", basis = "consent", review = "2025-06-01")]
enum LocalPurpose {
    ClinicalTrial23,
}

#[test]
fn guard_attributes_compile() {
    assert!(true);
}
