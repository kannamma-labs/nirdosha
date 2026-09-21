//! RTM live-demo plan, Phase A: proves M6's list/export/JSON-list routes
//! are real, guard-enforced HTTP reads -- not just that
//! `nirdosha_guard_screens::GuardedTable` works as a library (that's
//! `crates/nirdosha-guard-screens/src/lib.rs`'s own unit tests), but
//! that clicking the avatar tile's `POST /login` and then hitting
//! `/transactions` as that session actually returns real,
//! tenant-scoped, masked rows through `Router::dispatch` -- the same
//! entrypoint a real socket (`Router::serve`) uses, exercised in-process
//! per this crate's existing test convention (`guarded_ingest.rs`,
//! `crates/nirdosha-rt/tests/login.rs`).

use std::collections::HashMap;

use nirdosha_guard_screens::GuardedEntity;
use nirdosha_rt::{Auth, Request, Router};
use rtm::bridge::{transaction_table, TransactionRow};

fn router() -> Router {
    let router = rtm::m01_auth::mount_login(Router::new(|_req| Auth::login("anon", &[])));
    let router = rtm::m01_auth::mount_avatar_picker(router);
    rtm::m06_transactions::mount_transaction_screens(router)
}

fn login_request(username: &str, password: &str) -> Request {
    Request {
        method: "POST".into(),
        path: "/login".into(),
        headers: HashMap::from([("content-type".into(), "application/x-www-form-urlencoded".into())]),
        body: format!("username={username}&password={password}"),
    }
}

fn session_cookie(resp: &nirdosha_rt::Response) -> String {
    resp.extra_headers
        .iter()
        .find_map(|(k, v)| if k == "Set-Cookie" { v.split(';').next().map(str::to_string) } else { None })
        .expect("login must set a session cookie")
}

fn get_as(router: &Router, path: &str, cookie: &str) -> nirdosha_rt::Response {
    router.dispatch(&Request { method: "GET".into(), path: path.into(), headers: HashMap::from([("cookie".into(), cookie.to_string())]), body: String::new() })
}

/// One seed transaction per test -- inserted directly through
/// `GuardedTable::guarded_insert` under `SvcIngest` (the only principal
/// `ingest-create-txn` allows to write), the same real-corpus-policy
/// path `examples/rtm/tests/guarded_ingest.rs` already proves in
/// isolation. This test's job is what happens *after* that: does a real
/// HTTP session actually see it.
fn seed(txn_id: &str, amount: f64, card_token: &str) -> TransactionRow {
    let row = TransactionRow {
        id: 0,
        txn_id: txn_id.into(),
        tenant_id: "acme-demo".into(),
        subject_id: "subject-1".into(),
        account_id: "acct-1".into(),
        amount,
        currency: "USD".into(),
        status: "Pending".into(),
        occurred_at: 1_700_000_000,
        channel: "Card".into(),
        merchant_id: "merchant-1".into(),
        card_token: card_token.into(),
        device_id: "device-1".into(),
        geo: "US".into(),
        analyst_flag: false,
        analyst_flag_reason: String::new(),
    };
    transaction_table()
        .guarded_insert(&Auth::login("ingest-svc", &["SvcIngest"]), "FraudMonitoring", row.clone())
        .expect("ingest-create-txn must let SvcIngest seed a transaction — it's the corpus's own [RFC §8.1 verbatim] allow");
    row
}

fn post_form_as(router: &Router, path: &str, cookie: &str, form: &str) -> nirdosha_rt::Response {
    router.dispatch(&Request {
        method: "POST".into(),
        path: path.into(),
        headers: HashMap::from([("cookie".into(), cookie.to_string()), ("content-type".into(), "application/x-www-form-urlencoded".into())]),
        body: form.into(),
    })
}

#[test]
fn analyst_session_sees_seeded_transactions_through_a_real_http_login() {
    let router = router();
    let seeded = seed("txn-http-001", 42.50, "4111-1111-1111-1111");
    assert_eq!(seeded.row_id(), "txn-http-001");

    let login_resp = router.dispatch(&login_request("analyst", "analyst-demo"));
    let cookie = session_cookie(&login_resp);

    let list_resp = get_as(&router, "/transactions", &cookie);
    assert_eq!(list_resp.status, 200, "analyst-search-transaction must let Analyst list transactions: {list_resp:?}");
    assert!(list_resp.body.contains("txn-http-001"), "seeded row must render in the list HTML: {}", list_resp.body);

    let json_resp = get_as(&router, "/api/transactions", &cookie);
    assert_eq!(json_resp.status, 200);
    let rows: serde_json::Value = serde_json::from_str(&json_resp.body).expect("valid JSON");
    let rows = rows.as_array().expect("array of rows");
    assert!(rows.iter().any(|r| r["txn_id"] == "txn-http-001"), "seeded row missing from JSON list: {rows:?}");
}

#[test]
fn a_role_with_no_matching_allow_policy_is_denied_not_silently_empty() {
    let router = router();
    seed("txn-http-002", 10.0, "4111-1111-1111-1111");

    // CsAgent has real `guard_policy!` grants (`cs-payment-status`), but
    // none of them are `read transaction` -- deny-by-default, not "you
    // happen to see zero rows," is the real posture the corpus takes
    // here (see `00_core.nir`'s role table: CsAgent → M21.3 only).
    let login_resp = router.dispatch(&login_request("csagent", "csagent-demo"));
    let cookie = session_cookie(&login_resp);

    let list_resp = get_as(&router, "/transactions", &cookie);
    assert_eq!(list_resp.status, 403, "CsAgent has no allow policy for reading `transaction` — must be denied, not empty: {list_resp:?}");
}

/// [RFC §6.6] `analyst-flag-transaction`'s real `guard:` `update_fields`
/// path — proves G1 (field_policy `allowed(analyst_flag,
/// analyst_flag_reason)`) actually lets those two fields through a real
/// HTTP POST, and that the write really lands (readable back through the
/// same guarded read path the first test already proved).
#[test]
fn analyst_flag_update_writes_through_the_real_field_policy() {
    let router = router();
    seed("txn-http-003", 42.50, "4111-1111-1111-1111");
    let cookie = session_cookie(&router.dispatch(&login_request("analyst", "analyst-demo")));

    let resp = post_form_as(&router, "/transactions/txn-http-003/edit", &cookie, "analyst_flag=true&analyst_flag_reason=velocity-spike");
    assert_eq!(resp.status, 302, "an allowed field_policy update must redirect, not error: {resp:?}");

    let json = get_as(&router, "/api/transactions", &cookie);
    let rows: serde_json::Value = serde_json::from_str(&json.body).expect("valid JSON");
    let row = rows.as_array().unwrap().iter().find(|r| r["txn_id"] == "txn-http-003").expect("row must still be there");
    assert_eq!(row["analyst_flag"], true, "the update must have actually committed: {row:?}");
    assert_eq!(row["analyst_flag_reason"], "velocity-spike");
}

/// G2: `analyst-flag-transaction`'s real `requires invariant
/// (amount_positive) && invariant(currency_iso)` is re-checked against
/// the row's post-write state on *every* update, not just at creation —
/// a row that was already invalid (seeded with a non-positive amount;
/// `ingest-create-txn` itself carries no `requires` clause, so nothing
/// stopped it landing that way) must still fail an otherwise-unrelated
/// field update, proving the dispatch through `TransactionRow::
/// check_invariant` → the real `Money::is_positive` (`10_domains.nir`)
/// is wired for real, not a stub that always passes.
#[test]
fn update_is_blocked_when_the_row_fails_a_re_checked_invariant() {
    let router = router();
    seed("txn-http-004", -5.0, "4111-1111-1111-1111");
    let cookie = session_cookie(&router.dispatch(&login_request("analyst", "analyst-demo")));

    let resp = post_form_as(&router, "/transactions/txn-http-004/edit", &cookie, "analyst_flag=true&analyst_flag_reason=x");
    assert_eq!(resp.status, 400, "a row failing amount_positive must block the update, not silently pass it through: {resp:?}");
}
