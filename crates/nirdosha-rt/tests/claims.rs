//! Real end-to-end proof that `requires(claim = "..", "..")` actually
//! gates a function the same way `requires(role = "..")` already does
//! — not just that the macro expansion compiles (`lib.rs`'s own doc
//! comment examples only prove that much), but that the injected
//! `&ClaimProof<..>` parameter genuinely makes the function uncallable
//! without a session whose claim set holds the exact name *and* value.

nirdosha_rt::claims! {
    DepartmentCardiology = "department" -> "cardiology";
}

#[nirdosha_rt::contract(effects(pure), requires(claim = "department", "cardiology"))]
fn cardiology_only_report(patient_count: u64) -> u64 {
    patient_count
}

#[test]
fn a_session_holding_the_exact_claim_can_call_the_gated_fn() {
    use nirdosha_rt::Auth;
    let session = Auth::login("sita", &[]).with_claim("department", "cardiology");
    let proof = session.prove_claim::<nirdosha_claims::DepartmentCardiology>().expect("claim held");
    assert_eq!(cardiology_only_report(&proof, 42), 42);
}

/// Same claim *name*, a different *value* — proving the gate checks
/// both, not just presence of the `department` key the way a role
/// check would only ever check presence of `"admin"` or similar.
#[test]
fn a_session_with_a_different_value_for_the_same_claim_name_cannot_prove_it() {
    use nirdosha_rt::Auth;
    let session = Auth::login("ravana", &[]).with_claim("department", "oncology");
    assert!(session.prove_claim::<nirdosha_claims::DepartmentCardiology>().is_err());
}

#[test]
fn a_session_with_no_claims_at_all_cannot_prove_it() {
    use nirdosha_rt::Auth;
    let session = Auth::login("outsider", &[]);
    assert!(session.prove_claim::<nirdosha_claims::DepartmentCardiology>().is_err());
}
