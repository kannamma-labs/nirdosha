//! End-to-end test for `#[contract(logging(...))]` + `#[dataset(...)]` maps.

use nirdosha_rt::logging_guard::{Level, Policy, ScrubResult};

#[allow(dead_code)]
#[nirdosha_rt::dataset(
    entity = "transaction",
    store = "pg_transactions",
    maps = { cvv = ["_CCV", "card_verification_value"], pin = ["PIN_HASH"] }
)]
struct Transaction {}

#[nirdosha_rt::contract(
    logging(domain = "payments", country = "US", entity = "transaction")
)]
fn search_transactions() {
    let payload = serde_json::json!({
        "pan": "4111111111111111",
        "_CCV": "123",
        "timestamp": "t",
        "actor": "svc",
        "action": "auth",
        "outcome": "declined",
        "auth_result": "declined",
        "request_id": "r-1",
        "event_id": "e-1",
        "schema_version": "1",
    });
    let result = nirdosha_rt::logging_guard::scrub(payload);
    assert!(
        matches!(result, ScrubResult::ForbiddenField { field, .. } if field == "_CCV"),
        "cvv stored as _CCV must be forbidden"
    );
}

#[test]
fn logging_guard_compiles_and_resolves_field_maps() {
    search_transactions();
}

#[test]
fn logging_guard_masks_when_allowed() {
    let policy = Policy {
        rule_ids: "R-test",
        domain: "payments",
        country: "US",
        level: Level::Info,
        retention_days: 365,
        mask_fields: &[("pan", "partial(6,4)")],
        forbid_fields: &[],
        require_fields: &[],
        basis: "test",
    };
    let payload = serde_json::json!({ "pan": "4111111111111111" });
    match nirdosha_rt::logging_guard::scrub_with(&policy, None, payload) {
        ScrubResult::Clean(v) => assert_eq!(v["pan"].as_str(), Some("411111******1111")),
        other => panic!("expected clean, got {other:?}"),
    }
}
