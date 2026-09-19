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

#[test]
fn guard_policy_macro_compiles() {
    // The test is that the macro expanded without a compile error.
    assert!(true);
}
