//! RTM live-demo plan, Phase B final batch: proves M7 (Network/Link),
//! M11 (Real-Time Intervention), M12 (SAR), and M17 (Knowledge) are
//! real, guard-enforced HTTP screens through `Router::dispatch`. Also
//! regression-covers this batch's shared-crate fix:
//! `apply_condition_filter_in_process` -- a matching Allow record's
//! `Condition::Expr` (`requires field(status) == "..."`) now really
//! narrows a scan/export, proven end to end via `sar-export`.

use std::collections::HashMap;

use nirdosha_rt::{Auth, Request, Response, Router};
use rtm::bridge::{payment_table, sar_bundle_table, PaymentRow, SarBundleRow, DEMO_TENANT};

fn router() -> Router {
    let router = rtm::m02_dashboards::mount_app_shell(Router::new(|_req| Auth::login("anon", &[])));
    let router = rtm::m01_auth::mount_login(router);
    let router = rtm::m01_auth::mount_avatar_picker(router);
    let router = rtm::m07_network::mount_lineage_explore(router);
    let router = rtm::m07_network::mount_lineage_audit(router);
    let router = rtm::m11_intervention::mount_hold_stats(router);
    let router = rtm::m11_intervention::mount_hold_decision(router);
    let router = rtm::m11_intervention::mount_hold_queue(router);
    let router = rtm::m11_intervention::mount_hold_detail(router);
    let router = rtm::m12_sar::mount_sar_mlro_decision(router);
    let router = rtm::m12_sar::mount_sar_export(router);
    let router = rtm::m12_sar::mount_sar_screens(router);
    rtm::m17_knowledge::mount_knowledge(router)
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

// ---- M7 Network / Link Analysis ----

#[test]
fn analyst_sees_the_real_seeded_lineage_graph_and_other_roles_are_denied() {
    let router = router();
    let analyst_cookie = login_as(&router, "analyst", "analyst-demo");
    let resp = get_as(&router, "/lineage/downstream", &analyst_cookie);
    assert_eq!(resp.status, 200, "lineage-explore must serve the real seeded graph to Analyst: {resp:?}");
    assert!(resp.body.contains("txn_in") && resp.body.contains("rt_fraud_v1"), "the SVG must render the real corpus-consistent pipeline nodes, not a placeholder: {}", resp.body);

    let cs_cookie = login_as(&router, "csagent", "csagent-demo");
    let denied = get_as(&router, "/lineage/downstream", &cs_cookie);
    assert_eq!(denied.status, 403, "a role lineage-explore doesn't name must be denied: {denied:?}");
}

#[test]
fn auditor_gets_the_wider_unpinned_lineage_view() {
    let router = router();
    let auditor_cookie = login_as(&router, "auditor", "auditor-demo");
    let resp = get_as(&router, "/lineage/audit", &auditor_cookie);
    assert_eq!(resp.status, 200, "lineage-audit must serve the same real graph to Auditor: {resp:?}");
    assert!(resp.body.contains("<svg"));
}

// ---- M11 Real-Time Payment Intervention ----

#[test]
fn ops_analyst_decides_a_low_value_hold_through_a_plain_guarded_update() {
    let router = router();
    payment_table().raw_driver_seed(DEMO_TENANT, &PaymentRow {
        id: 0, payment_id: "pay-lowval".into(), tenant_id: DEMO_TENANT.into(), rail_ref: Some("rtp-1".into()),
        originator: Some("s1".into()), beneficiary: Some("s2".into()), amount: Some("500.00".into()), currency: "USD".into(),
        status: "held".into(), hold_reason: Some("velocity".into()), hold_expires_at: 0, decision_by: None, decision_rationale: None,
    });
    let ops_cookie = login_as(&router, "opsanalyst", "opsanalyst-demo");
    let resp = post_form_as(&router, "/holds/pay-lowval/decide", &ops_cookie, "status=released&decision_rationale=cleared+after+review");
    assert_eq!(resp.status, 200, "at/under $100k must be a plain committed update, not an escalation: {resp:?}");
    let body = body_json(&resp);
    assert_eq!(body["status"], "released");
}

#[test]
fn high_value_hold_release_escalates_through_override_release_quorum() {
    let router = router();
    payment_table().raw_driver_seed(DEMO_TENANT, &PaymentRow {
        id: 0, payment_id: "pay-highval".into(), tenant_id: DEMO_TENANT.into(), rail_ref: Some("rtp-2".into()),
        originator: Some("s3".into()), beneficiary: Some("s4".into()), amount: Some("250000.00".into()), currency: "USD".into(),
        status: "held".into(), hold_reason: Some("sanctions_screen".into()), hold_expires_at: 0, decision_by: None, decision_rationale: None,
    });
    let ops_cookie = login_as(&router, "opsanalyst", "opsanalyst-demo");
    let propose = post_form_as(&router, "/holds/pay-highval/decide", &ops_cookie, "status=released&decision_rationale=confirmed+legitimate");
    assert_eq!(propose.status, 202, "above $100k must escalate via override_release, not commit directly: {propose:?}");
    let body = body_json(&propose);
    assert_eq!(body["quorum"], 1, "override_release is quorum(1, of=[ComplianceLead])");

    let lead_cookie = login_as(&router, "compliancelead", "compliancelead-demo");
    let chain = body["chain"].as_str().unwrap();
    let escalation_id = body["escalation_id"].as_str().unwrap();
    let confirm = post_form_as(&router, "/holds/pay-highval/confirm-decide", &lead_cookie, &format!("chain={chain}&escalation_id={escalation_id}&status=released&decision_rationale=confirmed+legitimate"));
    assert_eq!(confirm.status, 200, "a single distinct ComplianceLead confirmation reaches quorum(1) and commits: {confirm:?}");
}

#[test]
fn auto_release_on_timeout_is_a_real_denial_not_a_silent_release() {
    let router = router();
    payment_table().raw_driver_seed(DEMO_TENANT, &PaymentRow {
        id: 0, payment_id: "pay-expired".into(), tenant_id: DEMO_TENANT.into(), rail_ref: Some("rtp-3".into()),
        originator: Some("s5".into()), beneficiary: Some("s6".into()), amount: Some("50.00".into()), currency: "USD".into(),
        status: "released".into(), hold_reason: Some("velocity".into()), hold_expires_at: 1, decision_by: None, decision_rationale: None,
    });
    // `auto-release-on-timeout` denies `SvcIngest` attempting the
    // release write once `expired(hold_expires_at)` — no HTTP route
    // exists for `SvcIngest` (a service principal, not a human login,
    // matching every other Svc* boundary in this rollout), so this is
    // proven at the `GuardedTable` level directly, the same way
    // `guarded_ingest.rs`'s own SoD test proves `ingest-no-readback`.
    let svc = Auth::login("svc-ingest", &["SvcIngest"]);
    let changed: std::collections::HashSet<String> = ["status".into()].into();
    let result = payment_table().guarded_update(&svc, "FraudMonitoring", "pay-expired", &changed, |row| row.status = "released".into());
    assert!(result.is_err(), "auto-release-on-timeout must really deny SvcIngest, not silently succeed: {result:?}");
}

// ---- M12 SAR / Regulatory Reporting ----

#[test]
fn analyst_drafts_a_sar_through_the_real_invariant_and_mlro_decides_it() {
    let router = router();
    let analyst_cookie = login_as(&router, "analyst", "analyst-demo");
    let create = post_form_as(&router, "/sar", &analyst_cookie, "sar_id=sar-001&case_id=case-77&subject=subj-77");
    assert!(create.status == 200 || create.status == 201 || create.status == 302, "analyst-draft-sar with a real case_id/subject must satisfy sar_subject_in_case and commit: {create:?}");

    let bad = post_form_as(&router, "/sar", &analyst_cookie, "sar_id=sar-bad&case_id=&subject=");
    assert!(bad.status >= 400, "an empty case_id/subject must fail the real sar_subject_in_case invariant, not commit: {bad:?}");

    let mlro_cookie = login_as(&router, "mlro", "mlro-demo");
    let decide = post_form_as(&router, "/sar/sar-001/decide", &mlro_cookie, "status=in_review&mlro_rationale=proceeding");
    assert_eq!(decide.status, 200, "mlro-decide-sar's real SarStatus transition draft->in_review must succeed: {decide:?}");
    let illegal = post_form_as(&router, "/sar/sar-001/decide", &mlro_cookie, "status=ceased&mlro_rationale=skip");
    assert!(illegal.status >= 400, "in_review->ceased is not a legal SarStatus edge and must be rejected: {illegal:?}");
}

#[test]
fn sar_export_narrows_to_confirmed_fraud_rows_only_via_the_new_condition_filter() {
    let router = router();
    sar_bundle_table().raw_driver_seed(DEMO_TENANT, &SarBundleRow { id: 0, sar_id: "sar-confirmed".into(), tenant_id: DEMO_TENANT.into(), case_id: "case-1".into(), subject: "subj-1".into(), status: "confirmed_fraud".into(), narrative: "n".into(), activity_codes: String::new(), amount_total: 100.0, txn_refs: String::new(), filed_at: 0, goaml_ref: String::new(), next_review_at: 0, mlro_rationale: String::new() });
    sar_bundle_table().raw_driver_seed(DEMO_TENANT, &SarBundleRow { id: 0, sar_id: "sar-draft".into(), tenant_id: DEMO_TENANT.into(), case_id: "case-2".into(), subject: "subj-2".into(), status: "draft".into(), narrative: "n".into(), activity_codes: String::new(), amount_total: 50.0, txn_refs: String::new(), filed_at: 0, goaml_ref: String::new(), next_review_at: 0, mlro_rationale: String::new() });

    let lead_cookie = login_as(&router, "compliancelead", "compliancelead-demo");
    let propose = post_form_as(&router, "/sar/export/propose", &lead_cookie, "");
    assert_eq!(propose.status, 202, "sar-export escalates via sar_release quorum(2): {propose:?}");
    let body = body_json(&propose);
    let chain = body["chain"].as_str().unwrap().to_string();
    let escalation_id = body["escalation_id"].as_str().unwrap().to_string();

    // `sar_release` is `quorum(2, of = [ComplianceLead])` and this demo
    // login system has only one `ComplianceLead` credential -- reaching
    // a real quorum(2) needs a second, DISTINCT approver identity, which
    // `guarded_confirm_escalated_export` (a real `GuardedTable` API, not
    // HTTP-only) can supply directly, matching this crate's own
    // `escalated_export_blocks_self_review_and_commits...` unit test
    // precedent exactly (same reason M4's four-eyes test needed a
    // `lead-b` identity for `case_review`, another `ComplianceLead`-only
    // quorum(2) chain).
    let lead_b = Auth::login("lead-b", &["ComplianceLead"]);
    let now_ms = 5_000u64;
    let confirm = sar_bundle_table().guarded_confirm_escalated_export(&lead_b, "AmlInvestigation", nirdosha_guard_core::Action::Export, "export", &chain, &escalation_id, now_ms);
    match confirm {
        Ok(nirdosha_guard_screens::EscalatedWrite::Committed(rows)) => {
            assert_eq!(rows.len(), 1, "the export must include ONLY the confirmed_fraud row, not the draft one: {rows:?}");
            assert_eq!(rows[0].sar_id, "sar-confirmed");
        }
        other => panic!("a second, distinct ComplianceLead approver must reach quorum(2) and commit the real, condition-filtered scan: {other:?}"),
    }
}

#[test]
fn tipping_off_denies_are_real_against_the_post_wildcard_fix_evaluator() {
    let router = router();
    sar_bundle_table().raw_driver_seed(DEMO_TENANT, &SarBundleRow { id: 0, sar_id: "sar-hidden".into(), tenant_id: DEMO_TENANT.into(), case_id: "case-9".into(), subject: "subj-9".into(), status: "draft".into(), ..Default::default() });
    let cs = Auth::login("cs1", &["CsAgent"]);
    let read = sar_bundle_table().guarded_get(&cs, "CustomerService", "sar-hidden");
    assert!(read.is_err(), "cs-no-sar (resource in [...], not a wildcard) must really deny CsAgent: {read:?}");
    let rm = Auth::login("rm1", &["RmUser"]);
    let read2 = sar_bundle_table().guarded_get(&rm, "CustomerService", "sar-hidden");
    assert!(read2.is_err(), "rm-no-sar must really deny RmUser: {read2:?}");
}

// ---- M17 Knowledge ----

#[test]
fn knowledge_base_is_real_static_content_unguarded_with_a_real_digest() {
    let router = router();
    // Genuinely unauthenticated -- no login, no cookie.
    let resp = router.dispatch(&Request { method: "GET".into(), path: "/knowledge".into(), headers: HashMap::new(), body: String::new() });
    assert_eq!(resp.status, 200, "M17 is real, unguarded static content: {resp:?}");
    assert!(resp.body.contains("Analyst") && resp.body.contains("ComplianceLead") && resp.body.contains("Workflow"), "content must be the real corpus glossary, not filler: {}", resp.body);
    assert!(resp.body.contains("sha256:"), "a real content digest must be exposed, matching this workspace's pinned-digest convention: {}", resp.body);
}
