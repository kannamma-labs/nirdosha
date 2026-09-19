use nirdosha_guard_registry::{dump, RoleRecord, ROLES};

#[linkme::distributed_slice(ROLES)]
static TEST_ROLE: RoleRecord = RoleRecord { name: String::new() };

#[test]
fn records_from_an_external_crate_are_collected() {
    assert_eq!(dump().roles.len(), 1);
}
