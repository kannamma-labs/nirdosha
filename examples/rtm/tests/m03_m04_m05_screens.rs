//! RTM live-demo plan, Phase B batch 1: proves M3 (Alerts), M4 (Cases),
//! and M5 (Customers) are real, guard-enforced HTTP screens through
//! `Router::dispatch` — the same entrypoint `m06_transactions_screen.rs`
//! already proves for M6, extended to the create/board/escalation shapes
//! this batch added.

use std::collections::HashMap;

use nirdosha_rt::{Auth, Request, Response, Router};
use rtm::bridge::{alert_table, case_table, customer_table, AlertRow, CaseRow, CustomerRow};

fn router() -> Router {
    let router = rtm::m02_dashboards::mount_app_shell(Router::new(|_req| Auth::login("anon", &[])));
    let router = rtm::m01_auth::mount_login(router);
    let router = rtm::m01_auth::mount_avatar_picker(router);
    let router = rtm::m02_dashboards::mount_l1_dashboard(router);
    let router = rtm::m02_dashboards::mount_team_dashboard(router);
    // Board mounts first -- see `serve.nir`'s identical fix/comment: a
    // sibling `crud_screens!`'s own `/{id}` wildcard would otherwise
    // swallow `/alerts/board`/`/cases/board`'s literal suffix.
    let router = rtm::m03_alerts::mount_alert_board(router);
    let router = rtm::m03_alerts::mount_alert_screens(router);
    let router = rtm::m03_alerts::mount_bulk_alert_reassign(router);
    let router = rtm::m04_cases::mount_case_board(router);
    let router = rtm::m04_cases::mount_case_screens(router);
    let router = rtm::m04_cases::mount_case_four_eyes(router);
    rtm::m05_customer::mount_customer_screens(router)
}

fn login_request(username: &str, password: &str) -> Request {
    Request { method: "POST".into(), path: "/login".into(), headers: HashMap::from([("content-type".into(), "application/x-www-form-urlencoded".into())]), body: format!("username={username}&password={password}") }
}
fn session_cookie(resp: &Response) -> String {
    resp.extra_headers.iter().find_map(|(k, v)| if k == "Set-Cookie" { v.split(';').next().map(str::to_string) } else { None }).expect("login must set a session cookie")
}
fn login_as(router: &Router, username: &str, password: &str) -> String {
    session_cookie(&router.dispatch(&login_request(username, password)))
}
fn get_as(router: &Router, path: &str, cookie: &str) -> Response {
    router.dispatch(&Request { method: "GET".into(), path: path.into(), headers: HashMap::from([("cookie".into(), cookie.to_string())]), body: String::new() })
}
fn post_form_as(router: &Router, path: &str, cookie: &str, form: &str) -> Response {
    router.dispatch(&Request { method: "POST".into(), path: path.into(), headers: HashMap::from([("cookie".into(), cookie.to_string()), ("content-type".into(), "application/x-www-form-urlencoded".into())]), body: form.into() })
}

fn seed_alert(alert_id: &str, status: &str) -> AlertRow {
    // `alert-raise`'s real `requires field(score) >= model(rt_fraud_v1)
    // .threshold_alert or field(rule_hits).nonempty()` is a model-
    // threshold/relation condition, not a literal `field(x)==y` or a
    // named `invariant(...)` -- `check_custom_conditions` correctly
    // fails closed on it (no ONNX scoring driver exists in this phase to
    // resolve `model(...)` against, see the live-demo plan's Phase C
    // scope). Seeding for this test goes straight at the driver, the
    // same "arrange state outside the boundary under test" posture
    // `seed_case`/`seed_customer` already use.
    let row = AlertRow { id: 0, alert_id: alert_id.into(), tenant_id: "acme-demo".into(), txn_id: "txn-1".into(), score: 0.9, model_version: "v1".into(), policy_version: "v1".into(), status: status.into(), assignee: String::new(), disposition_code: String::new(), rationale: String::new(), case_id: String::new(), sar_linked: String::new(), severity: "High".into(), tags: String::new() };
    alert_table().raw_driver_seed("acme-demo", &row);
    row
}

fn seed_case(case_id: &str, status: &str) -> CaseRow {
    // `analyst-create-case`'s `field_policy` FORBIDS the `status` field
    // at create (`SvcCase`'s own `case-svc-write` is `update`-only, no
    // create policy exists for it either) -- a test needing a case
    // pre-seeded at e.g. `confirmed_fraud` has no guarded path to get
    // there, so this seeds directly against the driver, same posture as
    // `seed_customer` below.
    let row = CaseRow { id: 0, case_id: case_id.into(), tenant_id: "acme-demo".into(), status: status.into(), assigned_to: String::new(), alert_ids: String::new(), subject_ref: "cust-1".into(), sar_id: String::new(), disposition: String::new(), rationale: "seed".into(), review_verdict: String::new(), review_note: String::new() };
    case_table().raw_driver_seed("acme-demo", &row);
    row
}

fn seed_customer(customer_id: &str) -> CustomerRow {
    // No corpus write policy grants ANY subject `create` on `customer`
    // (`90_ops_admin.nir` only has `export`) -- seeding for this test
    // goes straight at the driver, bypassing policy evaluation entirely,
    // the same "test fixture, not a policy proof" posture
    // `crates/nirdosha-guard-mic`'s own driver tests use for arranging
    // state outside the guard boundary being tested.
    let row = CustomerRow { id: 0, customer_id: customer_id.into(), tenant_id: "acme-demo".into(), name: "Jane Doe".into(), national_id: "N123".into(), dob: "1990-01-01".into(), occupation: "Engineer".into(), kyc_status: "Verified".into(), risk_rating: "Low".into(), pep_flag: String::new(), sanctions_status: "Clear".into(), restriction_status: "None".into(), legal_hold: false };
    customer_table().raw_driver_seed("acme-demo", &row);
    row
}

// ---- M3 Alerts ----

#[test]
fn analyst_reads_alert_queue_and_dispositions_it_through_real_field_policy() {
    let router = router();
    seed_alert("alert-001", "new");
    let cookie = login_as(&router, "analyst", "analyst-demo");

    let list = get_as(&router, "/alerts", &cookie);
    assert_eq!(list.status, 200, "analyst-read-alert must let Analyst list alerts: {list:?}");
    assert!(list.body.contains("alert-001"));

    // analyst-disposition-alert: requires field(status).transition_allowed()
    // (new -> in_progress is legal) + invariant(rationale_present).
    let resp = post_form_as(&router, "/alerts/alert-001/edit", &cookie, "status=in_progress&assignee=analyst&disposition_code=&rationale=reviewed+txn+history&case_id=");
    assert_eq!(resp.status, 302, "a legal transition with a rationale must succeed: {resp:?}");

    let json = get_as(&router, "/api/alerts", &cookie);
    let rows: serde_json::Value = serde_json::from_str(&json.body).unwrap();
    let row = rows.as_array().unwrap().iter().find(|r| r["alert_id"] == "alert-001").unwrap();
    assert_eq!(row["status"], "in_progress");
}

#[test]
fn illegal_alert_status_transition_is_rejected() {
    let router = router();
    seed_alert("alert-002", "new");
    let cookie = login_as(&router, "analyst", "analyst-demo");

    // `new` cannot jump straight to `escalated` per the declared
    // `machine AlertStatus` graph (`10_domains.nir`) -- must go through
    // `in_progress` first.
    let resp = post_form_as(&router, "/alerts/alert-002/edit", &cookie, "status=escalated&assignee=&disposition_code=&rationale=trying+to+skip+states&case_id=");
    assert_eq!(resp.status, 400, "an illegal transition must be rejected, not silently applied: {resp:?}");
}

#[test]
fn compliance_lead_bulk_reassigns_within_the_real_operations_purpose() {
    let router = router();
    seed_alert("alert-010", "new");
    seed_alert("alert-011", "new");
    let cookie = login_as(&router, "compliancelead", "compliancelead-demo");

    let resp = post_form_as(&router, "/alerts/bulk-reassign", &cookie, "ids=alert-010,alert-011&assignee=lead-1");
    assert_eq!(resp.status, 200, "{resp:?}");
    let body: serde_json::Value = serde_json::from_str(&resp.body).unwrap();
    let results = body["results"].as_array().unwrap();
    assert!(results.iter().all(|r| r["ok"] == true), "both rows must succeed under lead-bulk-alert: {results:?}");
}

// ---- M4 Cases ----

#[test]
fn analyst_creates_a_case_through_the_real_field_policy() {
    let router = router();
    let cookie = login_as(&router, "analyst", "analyst-demo");

    let resp = post_form_as(&router, "/cases", &cookie, "case_id=case-100&tenant_id=acme-demo&subject_ref=cust-1&alert_ids=alert-001&rationale=escalating+from+alert+review");
    assert_eq!(resp.status, 302, "analyst-create-case's required fields are all present — must succeed: {resp:?}");

    let json = get_as(&router, "/api/cases", &cookie);
    let rows: serde_json::Value = serde_json::from_str(&json.body).unwrap();
    assert!(rows.as_array().unwrap().iter().any(|r| r["case_id"] == "case-100"));
}

#[test]
fn case_create_missing_required_rationale_is_rejected() {
    let router = router();
    let cookie = login_as(&router, "analyst", "analyst-demo");
    // `analyst-create-case`'s `field_policy { required(..., rationale) }`
    // — an empty rationale is a missing required field, not a valid
    // empty string.
    let resp = post_form_as(&router, "/cases", &cookie, "case_id=case-101&tenant_id=acme-demo&subject_ref=cust-1&alert_ids=alert-001&rationale=");
    assert_eq!(resp.status, 400, "a missing required field_policy field is a field-policy violation (400), not a role denial (403): {resp:?}");
}

#[test]
fn four_eyes_review_blocks_self_approval_and_commits_on_a_second_distinct_lead() {
    let router = router();
    seed_case("case-200", "confirmed_fraud");
    let lead_a_cookie = login_as(&router, "compliancelead", "compliancelead-demo");

    let propose = post_form_as(&router, "/cases/case-200/propose-close", &lead_a_cookie, "review_verdict=confirmed&review_note=first+pass");
    assert_eq!(propose.status, 202, "quorum(2) must not commit on the proposer's own approval alone: {propose:?}");
    let proposed: serde_json::Value = serde_json::from_str(&propose.body).unwrap();
    assert_eq!(proposed["approvals_so_far"], 1);
    assert_eq!(proposed["quorum"], 2);
    let chain = proposed["chain"].as_str().unwrap().to_string();
    let escalation_id = proposed["escalation_id"].as_str().unwrap().to_string();

    // The SAME ComplianceLead confirming again must be rejected —
    // self-review blocked, not a second vote for the same person.
    let self_confirm = post_form_as(&router, "/cases/case-200/confirm-close", &lead_a_cookie, &format!("chain={chain}&escalation_id={escalation_id}&review_verdict=confirmed&review_note=trying+to+approve+myself"));
    assert_eq!(self_confirm.status, 403, "self-review must be blocked: {self_confirm:?}");

    // A DIFFERENT ComplianceLead confirming reaches quorum and commits.
    // The corpus's own `demo_users:` has exactly one ComplianceLead demo
    // identity, so this proves the *mechanism* via a second `Auth::login`
    // value carrying the same role directly against `GuardedTable` (the
    // same call `/cases/{id}/confirm-close`'s own handler makes) rather
    // than through a second HTTP login round-trip — the route wiring
    // itself is already proven by the self-review assertion above using
    // the real HTTP path; this half proves quorum completion end to end.
    let confirm = case_table().guarded_confirm_escalated_update(&Auth::login("lead-b", &["ComplianceLead"]), "Operations", "case-200", &chain, &escalation_id, &["review_verdict".to_string(), "review_note".to_string()].into_iter().collect(), |row| { row.review_verdict = "confirmed".into(); row.review_note = "second pass".into(); }, 5_000);
    match confirm.expect("a second, distinct ComplianceLead must reach quorum and commit") {
        nirdosha_guard_screens::EscalatedWrite::Committed(row) => assert_eq!(row.review_verdict, "confirmed"),
        nirdosha_guard_screens::EscalatedWrite::Pending(p) => panic!("must commit at quorum: {p:?}"),
    }
}

/// Regression: `crud_screens!`'s own `/{id}` wildcard (registered by
/// `mount_case_screens`) must not swallow the sibling `kanban_board!`'s
/// literal `/cases/board` route -- found live via `cargo run --bin
/// rtm-serve` + curl (a real `guarded_get(..., "board")` returning 404
/// for a nonexistent row id, not a routing error, which is what made it
/// non-obvious). Fixed by mounting the board before the CRUD screens in
/// both `serve.nir` and this file's own `router()`; this test pins the
/// fix.
#[test]
fn case_board_route_is_not_swallowed_by_the_id_wildcard() {
    let router = router();
    let cookie = login_as(&router, "analyst", "analyst-demo");
    let resp = get_as(&router, "/cases/board", &cookie);
    assert_eq!(resp.status, 200, "the board route must render, not be mistaken for /cases/{{id}} with id=\"board\": {resp:?}");
}

#[test]
fn case_board_drags_gray_out_transitions_the_real_casestatus_machine_forbids() {
    // T-14: `kanban_board!`'s `machine: "CaseStatus"` clause (m04_cases.nir)
    // must render each card's *real* allowed next columns from the
    // `workflow! { machine CaseStatus { .. } }` declared in 10_domains.nir
    // -- `open -> investigating -> [confirmed_fraud, false_positive, escalate]`
    // -- not a second, driftable transitions list.
    let router = router();
    seed_case("case-300", "open");
    seed_case("case-301", "confirmed_fraud");
    let cookie = login_as(&router, "analyst", "analyst-demo");

    let resp = get_as(&router, "/cases/board", &cookie);
    assert_eq!(resp.status, 200, "{resp:?}");

    // An `open` card may legally move only to `investigating` -- every
    // other column (confirmed_fraud, false_positive, sar_filed, closed)
    // is absent from its `data-allowed-to`, which is what drives
    // `board_js`'s pre-drop graying.
    let open_card_marker = format!("data-id=\"{}\"", "case-300");
    let open_card_start = resp.body.find(&open_card_marker).expect("case-300 card must render");
    let open_card_tag_end = resp.body[..open_card_start].rfind('<').unwrap();
    let open_card_tag = &resp.body[open_card_tag_end..open_card_start + open_card_marker.len() + 40];
    assert!(open_card_tag.contains("data-allowed-to=\"investigating\""), "open's only legal move is investigating: {open_card_tag}");
    assert!(!open_card_tag.contains("confirmed_fraud"), "open must not list confirmed_fraud as an allowed target: {open_card_tag}");

    // `confirmed_fraud` may legally move only to `sar_filed` per the
    // declared machine.
    let cf_card_marker = format!("data-id=\"{}\"", "case-301");
    let cf_card_start = resp.body.find(&cf_card_marker).expect("case-301 card must render");
    let cf_card_tag_end = resp.body[..cf_card_start].rfind('<').unwrap();
    let cf_card_tag = &resp.body[cf_card_tag_end..cf_card_start + cf_card_marker.len() + 40];
    assert!(cf_card_tag.contains("data-allowed-to=\"sar_filed\""), "confirmed_fraud's only legal move is sar_filed: {cf_card_tag}");

    // The board's own JS grays a column pre-drop and refuses the drop --
    // never a post-hoc reject only.
    let js = get_as(&router, "/cases/board/board.js", &cookie);
    assert_eq!(js.status, 200, "{js:?}");
    assert!(js.body.contains("kanban-column--disallowed"), "board_js must gray disallowed columns: {}", js.body);
    assert!(js.body.contains("dataset.allowedTo"), "board_js must read the card's real allowed-to set: {}", js.body);
}

// ---- M5 Customers ----

#[test]
fn auditor_reads_customer_and_analyst_is_denied() {
    let router = router();
    seed_customer("cust-500");

    let auditor_cookie = login_as(&router, "auditor", "auditor-demo");
    let resp = get_as(&router, "/customers", &auditor_cookie);
    assert_eq!(resp.status, 200, "auditor-read is a real broad grant on customer: {resp:?}");
    assert!(resp.body.contains("cust-500"));

    // The real, disclosed corpus gap `bridge.nir`'s `CustomerRow` doc
    // comment describes: no Analyst grant on `customer` exists at all.
    let analyst_cookie = login_as(&router, "analyst", "analyst-demo");
    let denied = get_as(&router, "/customers", &analyst_cookie);
    assert_eq!(denied.status, 403, "Analyst has no real guard_policy! grant on customer: {denied:?}");
}
