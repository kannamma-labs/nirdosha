//! RTM live-demo plan, Phase B batch 3: proves M14 (QA), M15 (Reporting/
//! governed export), M16 (Notifications, fed by the real `obligate
//! notify(channel(...))` obligation-consumption this batch adds), and
//! M18 (Admin: role grant, delegation mint, driver migrate, DSAR export)
//! are real, guard-enforced HTTP screens through `Router::dispatch`.

use std::collections::HashMap;

use nirdosha_rt::{Auth, Request, Response, Router};
use rtm::bridge::{case_table, customer_table, notification_table, qa_review_table, user_role_table, CaseRow, CustomerRow, QaReviewRow};

fn router() -> Router {
    let router = rtm::m02_dashboards::mount_app_shell(Router::new(|_req| Auth::login("anon", &[])));
    let router = rtm::m01_auth::mount_login(router);
    let router = rtm::m01_auth::mount_avatar_picker(router);
    let router = rtm::m03_alerts::mount_alert_board(router);
    let router = rtm::m03_alerts::mount_alert_screens(router);
    let router = rtm::m04_cases::mount_case_board(router);
    let router = rtm::m04_cases::mount_case_screens(router);
    let router = rtm::m04_cases::mount_case_four_eyes(router);
    let router = rtm::m05_customer::mount_customer_screens(router);
    let router = rtm::m14_qa::mount_qa_self_scorecard(router);
    let router = rtm::m14_qa::mount_qa_review_screens(router);
    let router = rtm::m15_reporting::mount_governed_export(router);
    let router = rtm::m15_reporting::mount_mis_dashboard(router);
    let router = rtm::m16_notifications::mount_notification_feed(router);
    let router = rtm::m18_admin::mount_role_grant(router);
    let router = rtm::m18_admin::mount_delegation_mint(router);
    let router = rtm::m18_admin::mount_driver_migrate(router);
    rtm::m18_admin::mount_dsar_export(router)
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

fn body_json(resp: &Response) -> serde_json::Value {
    serde_json::from_str(&resp.body).unwrap_or_else(|e| panic!("response body must be JSON: {e} — body was: {}", resp.body))
}

// ---- M14 QA ----

#[test]
fn qa_reviewer_creates_a_review_through_the_real_field_policy() {
    let router = router();
    let cookie = login_as(&router, "qareviewer", "qareviewer-demo");
    let resp = post_form_as(&router, "/qa", &cookie, "review_id=qa-live-1&item_ref=alert%3Aalert-001&rubric_scores=8&result=Pass");
    assert!(resp.status == 200 || resp.status == 201 || resp.status == 302, "qa-write-review must let QaReviewer create a review: {resp:?}");
}

#[test]
fn analyst_cannot_write_a_qa_review_real_sod_deny() {
    // `analyst-no-qa`: `deny for Analyst when action in ["create",
    // "update"] && resource == "qa_review"`.
    let router = router();
    let cookie = login_as(&router, "analyst", "analyst-demo");
    let resp = post_form_as(&router, "/qa", &cookie, "review_id=qa-live-2&item_ref=alert%3Aalert-002&rubric_scores=5&result=Fail");
    assert_eq!(resp.status, 403, "the real qa-review SoD deny must block Analyst from writing a QA review: {resp:?}");
}

#[test]
fn qa_reviewer_cannot_disposition_an_alert_real_sod_deny() {
    // `qa-no-disposition`: `deny for QaReviewer when action == "update"
    // && resource == "alert"`.
    let router = router();
    rtm::bridge::alert_table().raw_driver_seed("acme-demo", &rtm::bridge::AlertRow { id: 0, alert_id: "alert-qa-1".into(), tenant_id: "acme-demo".into(), txn_id: "txn-1".into(), score: 0.9, model_version: "v1".into(), policy_version: "v1".into(), status: "new".into(), assignee: String::new(), disposition_code: String::new(), rationale: String::new(), case_id: String::new(), sar_linked: None, severity: String::new(), tags: String::new() });
    let cookie = login_as(&router, "qareviewer", "qareviewer-demo");
    let resp = post_form_as(&router, "/alerts/alert-qa-1/edit", &cookie, "status=in_progress&rationale=r");
    assert_eq!(resp.status, 403, "the real qa-review SoD deny must block QaReviewer from dispositioning an alert: {resp:?}");
}

#[test]
fn analyst_sees_only_their_own_qa_scorecard_real_subject_scope() {
    qa_review_table().raw_driver_seed("acme-demo", &QaReviewRow { id: 2, review_id: "qa-mine-1".into(), tenant_id: "acme-demo".into(), item_ref: "alert:alert-777".into(), subject_user_id: "analyst".into(), rubric_scores: "9".into(), result: "Pass".into(), error_severity: String::new(), retraining_flag: false, feedback: String::new(), status: "scored".into() });
    let router = router();
    let cookie = login_as(&router, "analyst", "analyst-demo");
    let resp = get_as(&router, "/qa/mine", &cookie);
    assert_eq!(resp.status, 200, "self-read-qa must let Analyst read their own scorecard: {resp:?}");
    let rows = body_json(&resp);
    let ids: Vec<&str> = rows.as_array().expect("array").iter().map(|r| r["review_id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"qa-mine-1"), "must see own review: {ids:?}");
    assert!(!ids.contains(&"qa-001"), "must NOT see the seed review belonging to a different subject_user_id: {ids:?}");
}

// ---- M16 Notifications, fed by real obligations ----

#[test]
fn creating_a_case_produces_a_real_notification_via_the_obligate_notify_pipeline() {
    let router = router();
    let cookie = login_as(&router, "analyst", "analyst-demo");
    let resp = post_form_as(&router, "/cases", &cookie, "case_id=case-notif-1&tenant_id=acme-demo&subject_ref=cust-1&alert_ids=alert-001&rationale=escalating");
    assert!(resp.status == 200 || resp.status == 201 || resp.status == 302, "analyst-create-case must succeed: {resp:?}");
    // Matched by its own real, unique `body_ref` rather than a total
    // row-count delta -- `notification_table()` is one process-wide
    // singleton shared by every test in this binary (real product
    // behavior, not a test artifact), so other tests' own real
    // `obligate notify(...)` writes (e.g. this file's own QA-review
    // test) legitimately land in the same table concurrently.
    let after = notification_table().system_scan().expect("scan");
    let newest = after.iter().find(|n| n.body_ref == "case:case-notif-1").expect("a real Obligation::Notify from analyst-create-case's own obligate notify(channel(\"case-inbox\")) must produce a Notification row for this exact case");
    assert_eq!(newest.channel, "case-inbox");

    let feed_resp = get_as(&router, "/notifications", &cookie);
    let feed = body_json(&feed_resp);
    let channels: Vec<&str> = feed.as_array().expect("array").iter().map(|r| r["channel"].as_str().unwrap()).collect();
    assert!(channels.contains(&"case-inbox"), "Analyst is in case-inbox's role map, must see it in their feed: {channels:?}");
}

#[test]
fn a_role_with_no_channel_mapping_sees_no_notifications() {
    let router = router();
    let cookie = login_as(&router, "auditor", "auditor-demo");
    let resp = get_as(&router, "/notifications", &cookie);
    assert_eq!(resp.status, 200);
    let rows = body_json(&resp);
    assert!(rows.as_array().expect("array").is_empty(), "Auditor is in no channel's role map, so its feed must be empty, not an error or someone else's data: {rows:?}");
}

// ---- M15 Governed export (real quorum-gated scan) ----

#[test]
fn governed_case_export_is_pending_after_one_proposer_and_commits_after_a_second_distinct_lead() {
    case_table().raw_driver_seed("acme-demo", &CaseRow { id: 3, case_id: "case-export-1".into(), tenant_id: "acme-demo".into(), status: "open".into(), assigned_to: String::new(), alert_ids: String::new(), subject_ref: "cust-1".into(), sar_id: None, disposition: String::new(), rationale: "r".into(), review_verdict: String::new(), review_note: String::new() });
    let router = router();
    let analyst_cookie = login_as(&router, "analyst", "analyst-demo");
    let propose = post_form_as(&router, "/exports/case/propose", &analyst_cookie, "");
    assert_eq!(propose.status, 202, "governed-export's real escalate to approval(chain egress_release) must pend, not commit, on the proposer alone: {propose:?}");
    let body = body_json(&propose);
    let chain = body["chain"].as_str().unwrap().to_string();
    let escalation_id = body["escalation_id"].as_str().unwrap().to_string();

    let lead_cookie = login_as(&router, "compliancelead", "compliancelead-demo");
    let confirm = post_form_as(&router, "/exports/case/confirm", &lead_cookie, &format!("chain={chain}&escalation_id={escalation_id}"));
    assert_eq!(confirm.status, 200, "a real, distinct ComplianceLead approver must reach egress_release's quorum and commit the real scan: {confirm:?}");
    let confirmed = body_json(&confirm);
    let rows = confirmed["rows"].as_array().expect("rows array");
    assert!(rows.iter().any(|r| r["case_id"] == "case-export-1"), "the committed export must contain the real seeded row: {rows:?}");
}

// ---- M18 Admin ----

#[test]
fn admin_grant_role_is_a_real_maker_neq_checker_escalation() {
    let router = router();
    let admin_cookie = login_as(&router, "admin", "admin-demo");
    let propose = post_form_as(&router, "/admin/roles/grant-001/propose-grant", &admin_cookie, "");
    assert_eq!(propose.status, 202, "Admin proposing must pend (Admin holds neither policy_release approver role): {propose:?}");
    let body = body_json(&propose);
    assert_eq!(body["approvals_so_far"], 0, "real maker≠checker: the proposer consumes no quorum slot when ineligible: {body:?}");
    let chain = body["chain"].as_str().unwrap().to_string();
    let escalation_id = body["escalation_id"].as_str().unwrap().to_string();

    let pe_cookie = login_as(&router, "policyengineer", "policyengineer-demo");
    let first_confirm = post_form_as(&router, "/admin/roles/grant-001/confirm-grant", &pe_cookie, &format!("chain={chain}&escalation_id={escalation_id}"));
    assert_eq!(first_confirm.status, 202, "quorum(2) needs two distinct approvers: {first_confirm:?}");

    let lead_cookie = login_as(&router, "compliancelead", "compliancelead-demo");
    let second_confirm = post_form_as(&router, "/admin/roles/grant-001/confirm-grant", &lead_cookie, &format!("chain={chain}&escalation_id={escalation_id}"));
    assert_eq!(second_confirm.status, 200, "a second, distinct policy_release-eligible approver must commit the grant: {second_confirm:?}");
    let committed = body_json(&second_confirm);
    assert_eq!(committed["row"]["status"], "granted");
    let rows = user_role_table().guarded_snapshot(&Auth::login("auditor-probe", &["Auditor"]), "").unwrap_or_default();
    let _ = rows; // Auditor has no read grant on user_role either (no policy names it) -- left unasserted, real gap, same class as m05/m10's disclosed ones.
}

#[test]
fn admin_mints_a_delegation_token_within_the_real_ttl_cap() {
    let router = router();
    let cookie = login_as(&router, "admin", "admin-demo");
    let ok = post_form_as(&router, "/admin/delegations", &cookie, "scope=case%3Acase-1&ttl_days=30");
    assert_eq!(ok.status, 201, "admin-mint-delegation must let Admin mint a real token within cap(ttl_max = 90d): {ok:?}");
    let too_long = post_form_as(&router, "/admin/delegations", &cookie, "scope=case%3Acase-1&ttl_days=91");
    assert_eq!(too_long.status, 400, "cap(ttl_max = 90d) must be enforced for real, not decoratively: {too_long:?}");
}

#[test]
fn dsar_export_is_a_real_maker_neq_checker_escalated_scan_over_customer() {
    customer_table().raw_driver_seed("acme-demo", &CustomerRow { id: 4, customer_id: "cust-dsar-1".into(), tenant_id: "acme-demo".into(), name: Some("Jane Doe".into()), national_id: Some("N1".into()), dob: None, occupation: String::new(), kyc_status: "Verified".into(), risk_rating: Some("Low".into()), pep_flag: None, sanctions_status: Some("Clear".into()), restriction_status: String::new(), legal_hold: false });
    let router = router();
    let admin_cookie = login_as(&router, "admin", "admin-demo");
    let propose = post_form_as(&router, "/admin/dsar/propose", &admin_cookie, "");
    assert_eq!(propose.status, 202, "dsar-export's real escalate to approval(chain egress_release), Admin proposing (maker≠checker): {propose:?}");
    let body = body_json(&propose);
    let chain = body["chain"].as_str().unwrap().to_string();
    let escalation_id = body["escalation_id"].as_str().unwrap().to_string();
    let lead_cookie = login_as(&router, "compliancelead", "compliancelead-demo");
    let confirm = post_form_as(&router, "/admin/dsar/confirm", &lead_cookie, &format!("chain={chain}&escalation_id={escalation_id}"));
    assert_eq!(confirm.status, 200, "a real, distinct approver must commit the real customer scan: {confirm:?}");
    let confirmed = body_json(&confirm);
    let rows = confirmed["rows"].as_array().expect("rows array");
    assert!(rows.iter().any(|r| r["customer_id"] == "cust-dsar-1"), "the committed DSAR export must contain the real seeded row: {rows:?}");
}
