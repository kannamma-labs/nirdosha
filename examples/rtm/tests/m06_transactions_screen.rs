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

/// T-12 (I15): `analyst-search-transaction`'s real `grant
/// predicate_use(amount, currency, channel, merchant_id, device_id,
/// geo)` — `?q=` pushed down through `GuardedTable::guarded_search`
/// must actually narrow the result set (not just render decoratively),
/// restricted to those granted fields. `merchant-zz` only matches one of
/// the two seeded rows' `merchant_id`.
#[test]
fn q_search_on_a_granted_predicate_use_field_returns_policy_correct_rows() {
    let router = router();
    let matching = TransactionRow {
        id: 0,
        txn_id: "txn-http-005".into(),
        tenant_id: "acme-demo".into(),
        subject_id: "subject-1".into(),
        account_id: "acct-1".into(),
        amount: 42.50,
        currency: "USD".into(),
        status: "Pending".into(),
        occurred_at: 1_700_000_000,
        channel: "Card".into(),
        merchant_id: "merchant-zz".into(),
        card_token: "4111-1111-1111-1111".into(),
        device_id: "device-1".into(),
        geo: "US".into(),
        analyst_flag: false,
        analyst_flag_reason: String::new(),
    };
    transaction_table().raw_driver_seed("acme-demo", &matching);
    seed("txn-http-006", 42.50, "4111-1111-1111-1111"); // merchant-1, from seed()

    let cookie = session_cookie(&router.dispatch(&login_request("analyst", "analyst-demo")));

    let json = get_as(&router, "/api/transactions?q=merchant-zz", &cookie);
    assert_eq!(json.status, 200, "a search on a granted predicate_use field must not be denied: {json:?}");
    let rows: serde_json::Value = serde_json::from_str(&json.body).expect("valid JSON");
    let rows = rows.as_array().expect("array of rows");
    assert!(rows.iter().any(|r| r["txn_id"] == "txn-http-005"), "the matching row must be present: {rows:?}");
    assert!(!rows.iter().any(|r| r["txn_id"] == "txn-http-006"), "a non-matching row must be filtered out, not just decoratively rendered: {rows:?}");

    let html = get_as(&router, "/transactions?q=merchant-zz", &cookie);
    assert!(html.body.contains("txn-http-005") && !html.body.contains("txn-http-006"), "the HTML list must reflect the same real filter: {}", html.body);
}

/// T-12 (I15): `card_token` is a real `TransactionRow` field but is NOT
/// in `analyst-search-transaction`'s `predicate_use` grant — a `q` that
/// only exists in `card_token` must find nothing (the field is silently
/// excluded from the search, per I15's "masked fields excluded from
/// WHERE"), never accidentally leak via a full-field scan the way the
/// old decorative substring search would have.
#[test]
fn q_search_cannot_reach_a_field_outside_the_predicate_use_grant() {
    let router = router();
    seed("txn-http-007", 42.50, "4111-CARDTOKEN-ONLY-VALUE");
    let cookie = session_cookie(&router.dispatch(&login_request("analyst", "analyst-demo")));

    let resp = get_as(&router, "/api/transactions?q=4111-CARDTOKEN-ONLY-VALUE", &cookie);
    assert_eq!(resp.status, 200, "the screen still has other granted fields, so this is an empty match, not a deny: {resp:?}");
    let rows: serde_json::Value = serde_json::from_str(&resp.body).expect("valid JSON");
    assert!(rows.as_array().unwrap().is_empty(), "card_token is not in predicate_use — a value only present there must not surface the row: {rows:?}");
}

/// T-01: the guarded CSV export route no longer hands back
/// `guarded_snapshot`'s rows straight from `Response::csv` (the
/// documented bypass) — it must route through
/// `nirdosha_rt::export::write_governed_export` and carry a real
/// watermark footer (purpose, export id, expiry, content hash) that a
/// plain unguarded CSV could never have produced.
#[test]
fn guarded_csv_export_carries_a_governed_watermark_footer() {
    let router = router();
    seed("txn-http-008", 99.0, "4111-1111-1111-1111");
    let cookie = session_cookie(&router.dispatch(&login_request("analyst", "analyst-demo")));

    let resp = get_as(&router, "/transactions/export.csv", &cookie);
    assert_eq!(resp.status, 200, "a purpose-carrying guarded export must succeed: {resp:?}");
    let lines: Vec<&str> = resp.body.lines().collect();
    let footer = lines.last().expect("export must have a footer line");
    assert!(footer.starts_with("# governed-export"), "last line must be the governed-export watermark, not a data row: {footer:?}");
    assert!(footer.contains("purpose=AmlInvestigation"), "watermark must carry the screen's real declared purpose: {footer:?}");
    assert!(footer.contains("export_id=exp-"), "watermark must carry a real minted export id: {footer:?}");
    assert!(footer.contains("sha256="), "watermark must carry a real content hash: {footer:?}");
    assert!(resp.body.contains("txn-http-008"), "the actual guarded row must still be present in the artifact: {}", resp.body);
}
