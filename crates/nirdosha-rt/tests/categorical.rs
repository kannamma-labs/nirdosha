//! `categorical_actions!` end to end: Road 1 (typed, gated) actually
//! enforces; Road 2 (`role_for_*`) is a real exhaustive `match`, purely
//! derived, never consulted for the enforcement above.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

nirdosha_rt::roles! {
    Approver = "approver";
    ComplianceOfficer = "compliance_officer";
}

#[derive(Clone)]
struct Widget {
    id: i64,
    is_approved: bool,
}

fn widget_store() -> &'static Mutex<HashMap<i64, Widget>> {
    static STORE: OnceLock<Mutex<HashMap<i64, Widget>>> = OnceLock::new();
    STORE.get_or_init(|| {
        let mut m = HashMap::new();
        m.insert(1, Widget { id: 1, is_approved: false });
        Mutex::new(m)
    })
}

nirdosha_rt::categorical_actions! {
    entity: Widget,
    store: widget_store,
    field: is_approved: bool,
    actions {
        true  => approve_widget    requires role "approver",
        false => disapprove_widget requires role "compliance_officer",
    }
}

#[test]
fn road1_grants_only_to_the_declared_role() {
    let approver = nirdosha_rt::Auth::login("alice", &["approver"]);
    let proof = approver.prove::<nirdosha_roles::Approver>().unwrap();
    let widget = approve_widget(&proof, 1).unwrap();
    assert_eq!(widget.id, 1);
    assert!(widget.is_approved);

    let officer = nirdosha_rt::Auth::login("bob", &["compliance_officer"]);
    let proof = officer.prove::<nirdosha_roles::ComplianceOfficer>().unwrap();
    let widget = disapprove_widget(&proof, 1).unwrap();
    assert!(!widget.is_approved);
}

#[test]
fn road1_refuses_the_wrong_role() {
    let officer = nirdosha_rt::Auth::login("bob", &["compliance_officer"]);
    assert!(officer.prove::<nirdosha_roles::Approver>().is_err());
}

#[test]
fn road2_is_a_real_exhaustive_match_derived_from_the_same_arms() {
    assert_eq!(role_for_is_approved(true), "approver");
    assert_eq!(role_for_is_approved(false), "compliance_officer");
}

mod properties {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// The core authorization property, fuzzed over arbitrary role
        /// sets: `approve_widget` succeeds if and only if the session
        /// holds exactly "approver" (not e.g. "compliance_officer",
        /// which is a real, different, adjacent role for the same
        /// field).
        #[test]
        fn approve_requires_exactly_approver(
            has_approver in any::<bool>(),
            has_officer in any::<bool>(),
        ) {
            let mut roles = Vec::new();
            if has_approver { roles.push("approver"); }
            if has_officer { roles.push("compliance_officer"); }
            let auth = nirdosha_rt::Auth::login("user", &roles);
            let can_approve = auth.prove::<nirdosha_roles::Approver>().is_ok();
            prop_assert_eq!(can_approve, has_approver);
        }

        #[test]
        fn disapprove_requires_exactly_compliance_officer(
            has_approver in any::<bool>(),
            has_officer in any::<bool>(),
        ) {
            let mut roles = Vec::new();
            if has_approver { roles.push("approver"); }
            if has_officer { roles.push("compliance_officer"); }
            let auth = nirdosha_rt::Auth::login("user", &roles);
            let can_disapprove = auth.prove::<nirdosha_roles::ComplianceOfficer>().is_ok();
            prop_assert_eq!(can_disapprove, has_officer);
        }
    }
}
