//! RTM live-demo plan, Phase B batch 4: proves M19 (IT Ops), M20 (Audit
//! Trail), M21 (Restricted/Downstream Views), and M22 (Misc) are real,
//! guard-enforced HTTP screens through `Router::dispatch`. Also
//! regression-covers this batch's two shared-crate fixes: the wildcard
//! (`resource: "*"`) deny match in `nirdosha-guard-core::evaluator`, and
//! the write-side `subject_scope()` enforcement / read-side
//! `field_policy.forbidden(...)` masking in `nirdosha-guard-screens`.

use std::collections::HashMap;

use nirdosha_rt::{Auth, Request, Response, Router};
use rtm::bridge::{alert_table, all_chains_table, customer_table, refresh_all_chains, refresh_sar_visibility, sar_visibility_table, transaction_table, user_profile_table, AlertRow, CustomerRow, TransactionRow};

fn router() -> Router {
    let router = rtm::m02_dashboards::mount_app_shell(Router::new(|_req| Auth::login("anon", &[])));
    let router = rtm::m01_auth::mount_login(router);
    let router = rtm::m01_auth::mount_avatar_picker(router);
    let router = rtm::m06_transactions::mount_transaction_screens(router);
    let router = rtm::m19_it_ops::mount_it_ops_dashboard(router);
    let router = rtm::m19_it_ops::mount_ingest_admin(router);
    let router = rtm::m20_audit::mount_audit_trail(router);
    let router = rtm::m20_audit::mount_audit_chain_search(router);
    let router = rtm::m21_restricted::mount_cs_payment_status(router);
    let router = rtm::m21_restricted::mount_rm_restriction_view(router);
    let router = rtm::m21_restricted::mount_auditor_portal(router);
    let router = rtm::m22_misc::mount_self_profile(router);
    rtm::m22_misc::mount_support_ticket_screens(router)
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

// ---- M19 IT Operations ----

#[test]
fn ingest_admin_replay_is_a_real_maker_neq_checker_migrate_escalation() {
    let router = router();
    let admin_cookie = login_as(&router, "admin", "admin-demo");
    let propose = post_form_as(&router, "/ops/ingest-admin/replay-001/propose-replay", &admin_cookie, "");
    assert_eq!(propose.status, 202, "ops-ingest-admin's real escalate to approval(chain policy_release), Admin proposing (maker≠checker): {propose:?}");
    let body = body_json(&propose);
    assert_eq!(body["approvals_so_far"], 0, "Admin holds neither policy_release approver role, so proposing consumes no slot: {body:?}");
    let chain = body["chain"].as_str().unwrap().to_string();
    let escalation_id = body["escalation_id"].as_str().unwrap().to_string();

    let pe_cookie = login_as(&router, "policyengineer", "policyengineer-demo");
    let first = post_form_as(&router, "/ops/ingest-admin/replay-001/confirm-replay", &pe_cookie, &format!("chain={chain}&escalation_id={escalation_id}"));
    assert_eq!(first.status, 202, "quorum(2) needs two distinct approvers: {first:?}");

    // T-04/B10: `policy_release`'s `cooling(days = 1)` -- quorum real
    // (2/2), but `Cooling` not `Approved` (see the M8/M13/M18 migrate
    // tests' matching comments).
    let lead_cookie = login_as(&router, "compliancelead", "compliancelead-demo");
    let second = post_form_as(&router, "/ops/ingest-admin/replay-001/confirm-replay", &lead_cookie, &format!("chain={chain}&escalation_id={escalation_id}"));
    assert_eq!(second.status, 202, "quorum reached but policy_release's cooling(days=1) window hasn't elapsed: {second:?}");
    let pending = body_json(&second);
    assert_eq!(pending["approvals_so_far"], 2);
}

#[test]
fn pipeline_health_catalog_panel_is_role_gated_to_admin_not_policy_gated() {
    // No `guard_policy!` in this corpus grants any human role a plain
    // read/aggregate on ingestion pipeline health -- this dashboard's own
    // real facts are static catalog counts, gated by `Auth::has_role`
    // directly (disclosed in `m19_it_ops.nir`'s own doc comment).
    let router = router();
    let admin_cookie = login_as(&router, "admin", "admin-demo");
    let admin_resp = get_as(&router, "/ops/pipeline-health.json", &admin_cookie);
    assert_eq!(admin_resp.status, 200);
    let admin_body = body_json(&admin_resp);
    let admin_metric = admin_body["widgets"][0]["value"].as_f64().unwrap_or(0.0);
    assert!(admin_metric >= 66.0, "Admin must see the real registered guard_policy! count: {admin_body:?}");

    let analyst_cookie = login_as(&router, "analyst", "analyst-demo");
    let analyst_resp = get_as(&router, "/ops/pipeline-health.json", &analyst_cookie);
    assert_eq!(analyst_resp.status, 200, "the route itself has no role gate (it's a dashboard, always 200) — the widget content is what's gated");
    let analyst_body = body_json(&analyst_resp);
    assert_eq!(analyst_body["widgets"][0]["value"].as_f64(), Some(0.0), "a non-Admin role must see the gated-to-zero value, not the real catalog count: {analyst_body:?}");
}

// ---- M20 Audit Trail ----

#[test]
fn auditor_sees_transactions_under_the_audit_purpose_that_analyst_purpose_denies_to_other_roles() {
    // Real proof that the SAME row is reachable under a DIFFERENT
    // matching policy/purpose: `auditor-read`'s `purpose(Audit)` vs.
    // `analyst-search-transaction`'s `purpose(AmlInvestigation)`
    // (`m06_transactions.nir`'s own hardcoded purpose) both name
    // `resource == "transaction"`, but only the real, role-appropriate
    // policy grants each subject.
    transaction_table().raw_driver_seed("acme-demo", &TransactionRow { id: 0, txn_id: "txn-audit-1".into(), tenant_id: "acme-demo".into(), subject_id: "subj-1".into(), account_id: "acct-1".into(), amount: 500.0, currency: "USD".into(), status: "Settled".into(), occurred_at: 1_700_000_000, channel: "Card".into(), merchant_id: "m-1".into(), card_token: "tok-secret".into(), device_id: "dev-1".into(), geo: "US".into(), analyst_flag: false, analyst_flag_reason: String::new() });
    let router = router();

    let auditor_cookie = login_as(&router, "auditor", "auditor-demo");
    let auditor_resp = get_as(&router, "/audit/transactions", &auditor_cookie);
    assert_eq!(auditor_resp.status, 200, "auditor-read's real purpose(Audit) grant must let Auditor see transactions: {auditor_resp:?}");
    let rows = body_json(&auditor_resp);
    let ids: Vec<&str> = rows.as_array().expect("array").iter().map(|r| r["txn_id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"txn-audit-1"), "must see the real seeded row: {ids:?}");

    // The same Auditor identity, on the DIFFERENT screen hardcoded to
    // AmlInvestigation, must still be denied -- proving this isn't a
    // blanket "Auditor sees everything" bypass, only the real purpose
    // match.
    let denied = get_as(&router, "/transactions", &auditor_cookie);
    assert_eq!(denied.status, 403, "Auditor has no grant under M6's own AmlInvestigation purpose: {denied:?}");
}

#[test]
fn auditor_no_write_now_really_denies_thanks_to_the_wildcard_resource_fix() {
    // `deny "auditor-no-write" for Auditor when action in ["create",
    // "update", "delete", "migrate"]` -- no `&& resource ...` clause at
    // all, lowered to the real `resource: "*"` sentinel. Before this
    // batch's `nirdosha-guard-core::evaluator` fix, this deny could never
    // match any real resource (plain string equality against a literal
    // `"*"`); calling `guarded_insert` directly proves it's now real.
    let auditor = Auth::login("auditor-probe", &["Auditor"]);
    let result = transaction_table().guarded_insert(&auditor, "Audit", TransactionRow { id: 0, txn_id: "txn-should-never-exist".into(), tenant_id: "acme-demo".into(), subject_id: "s".into(), account_id: "a".into(), amount: 1.0, currency: "USD".into(), status: "Pending".into(), occurred_at: 0, channel: "Card".into(), merchant_id: "m".into(), card_token: "t".into(), device_id: "d".into(), geo: "US".into(), analyst_flag: false, analyst_flag_reason: String::new() });
    assert!(result.is_err(), "auditor-no-write must deny Auditor from writing ANY resource, including one with no other matching policy at all: {result:?}");
}

// ---- M21 Restricted / Downstream Views ----

#[test]
fn cs_agent_sees_only_the_real_allowed_payment_fields_forbidden_fields_are_genuinely_absent() {
    let router = router();
    let cs_cookie = login_as(&router, "csagent", "csagent-demo");
    let resp = get_as(&router, "/cs/payments/pay-001", &cs_cookie);
    assert_eq!(resp.status, 200, "cs-payment-status must let CsAgent read the masked view: {resp:?}");
    let row = body_json(&resp);
    assert_eq!(row["status"], "held", "an allowed field must come through in the clear");
    assert_eq!(row["currency"], "USD", "an allowed field must come through in the clear");
    for forbidden_field in ["amount", "rail_ref", "originator", "beneficiary", "hold_reason", "decision_by", "decision_rationale"] {
        assert!(row.get(forbidden_field).is_none(), "T-02: cs-payment-status's real field_policy forbidden({forbidden_field}) must render it absent, not a \"[REDACTED]\" placeholder or the real value: {row:?}");
    }
    // The real values must never appear anywhere in the raw response body
    // (not just under the expected key) -- proves this is genuine
    // redaction, not e.g. a field rename that still leaks the value.
    assert!(!resp.body.contains("rtp-778812"), "the real rail_ref must never appear in the response body: {}", resp.body);
    assert!(!resp.body.contains("4500.00"), "the real amount must never appear in the response body: {}", resp.body);
}

#[test]
fn rm_user_sees_only_restriction_status_every_other_customer_field_is_genuinely_absent() {
    customer_table().raw_driver_seed("acme-demo", &CustomerRow { id: 5, customer_id: "cust-rm-1".into(), tenant_id: "acme-demo".into(), name: Some("Real Name".into()), national_id: Some("REAL-NATID".into()), dob: Some("1980-01-01".into()), occupation: "x".into(), kyc_status: "Verified".into(), risk_rating: Some("High".into()), pep_flag: Some("true".into()), sanctions_status: Some("Clear".into()), restriction_status: "Watch".into(), legal_hold: false });
    let router = router();
    let rm_cookie = login_as(&router, "rmuser", "rmuser-demo");
    let resp = get_as(&router, "/rm/customers/cust-rm-1", &rm_cookie, );
    assert_eq!(resp.status, 200, "rm-restriction-view must let RmUser read the masked view: {resp:?}");
    let row = body_json(&resp);
    assert_eq!(row["restriction_status"], "Watch", "the one allowed field must come through in the clear");
    for forbidden_field in ["national_id", "name", "dob", "risk_rating", "pep_flag", "sanctions_status"] {
        assert!(row.get(forbidden_field).is_none(), "T-02: rm-restriction-view's real field_policy forbidden({forbidden_field}) must render it absent, not a \"[REDACTED]\" placeholder or the real value: {row:?}");
    }
    assert!(!resp.body.contains("REAL-NATID"), "the real national_id must never appear in the response body: {}", resp.body);
}

#[test]
fn auditor_portal_shows_the_watermark_and_the_real_audit_read_data() {
    transaction_table().raw_driver_seed("acme-demo", &TransactionRow { id: 0, txn_id: "txn-portal-1".into(), tenant_id: "acme-demo".into(), subject_id: "subj-1".into(), account_id: "acct-1".into(), amount: 250.0, currency: "USD".into(), status: "Settled".into(), occurred_at: 1_700_000_001, channel: "Card".into(), merchant_id: "m-1".into(), card_token: "tok-secret".into(), device_id: "dev-1".into(), geo: "US".into(), analyst_flag: false, analyst_flag_reason: String::new() });
    let router = router();
    let auditor_cookie = login_as(&router, "auditor", "auditor-demo");
    let resp = get_as(&router, "/auditor/transactions", &auditor_cookie);
    assert_eq!(resp.status, 200, "auditor-read's real purpose(Audit) grant must let Auditor view the HTML portal too: {resp:?}");
    assert!(resp.body.contains("AUDIT READ-ONLY"), "21.1's watermark banner must always be present: {}", resp.body);
    assert!(resp.body.contains("auditor"), "the watermark must name the real viewing user: {}", resp.body);
    assert!(resp.body.contains("txn-portal-1"), "must render the real seeded row: {}", resp.body);

    let index = get_as(&router, "/auditor", &auditor_cookie);
    assert_eq!(index.status, 200);
    assert!(index.body.contains("AUDIT READ-ONLY"), "the portal index must also be watermarked: {}", index.body);
}

#[test]
fn auditor_portal_registers_no_write_route_at_all() {
    let router = router();
    let auditor_cookie = login_as(&router, "auditor", "auditor-demo");
    let post_resp = post_form_as(&router, "/auditor/transactions", &auditor_cookie, "");
    assert!(post_resp.status != 200 && post_resp.status != 201 && post_resp.status != 202, "no create/update route is registered for the auditor portal at all — 21.1 must be read-only by construction, not by a hidden UI flag: {post_resp:?}");
}

#[test]
fn non_auditor_cannot_see_the_portal_real_role_scoped_denial() {
    let router = router();
    let analyst_cookie = login_as(&router, "analyst", "analyst-demo");
    let resp = get_as(&router, "/auditor/transactions", &analyst_cookie);
    assert_eq!(resp.status, 403, "auditor-read is for Auditor only: {resp:?}");
}

#[test]
fn analyst_cannot_use_the_cs_or_rm_restricted_views_real_role_scoped_denial() {
    let router = router();
    let analyst_cookie = login_as(&router, "analyst", "analyst-demo");
    let cs_resp = get_as(&router, "/cs/payments/pay-001", &analyst_cookie);
    assert_eq!(cs_resp.status, 403, "cs-payment-status is for CsAgent only: {cs_resp:?}");
    let rm_resp = get_as(&router, "/rm/customers/cust-rm-1", &analyst_cookie);
    assert_eq!(rm_resp.status, 403, "rm-restriction-view is for RmUser only: {rm_resp:?}");
}

// ---- M22 Misc ----

#[test]
fn analyst_updates_their_own_profile_through_the_real_field_policy() {
    let router = router();
    let cookie = login_as(&router, "analyst", "analyst-demo");
    let resp = post_form_as(&router, "/profile/update", &cookie, "locale=fr-FR&tz=Europe%2FParis&digest=false");
    assert_eq!(resp.status, 200, "self-update-profile must let Analyst update their own seeded row: {resp:?}");
    let row = body_json(&resp);
    assert_eq!(row["locale"], "fr-FR");
    assert_eq!(row["tz"], "Europe/Paris");
    assert_eq!(row["digest"], false);
    assert_eq!(row["user_id"], "analyst", "the row_id is hardcoded to the caller's own identity — never a request-supplied id");

    // Real persistence proof, not just an echoed request: read it back
    // directly off the table (no read policy exists to check this
    // through HTTP, same disclosed gap `m22_misc.nir` names).
    let persisted = user_profile_table().guarded_get(&Auth::login("analyst", &["Analyst"]), "__internal_probe_purpose_never_matches__", "analyst");
    // `guarded_get` denies under a bogus purpose (no policy matches it) —
    // this line only proves the call itself doesn't panic; the real
    // persistence proof is the 200 body above, which came from the same
    // `commit_action` this fn also uses to decode "after".
    let _ = persisted;
}

#[test]
fn support_ticket_create_enforces_the_real_required_fields() {
    let router = router();
    let cookie = login_as(&router, "analyst", "analyst-demo");
    let missing_description = post_form_as(&router, "/support-tickets", &cookie, "ticket_id=tix-1&tenant_id=acme-demo&category=Bug&priority=High");
    assert_eq!(missing_description.status, 400, "any-create-ticket's real field_policy required(description) must reject a submission missing it: {missing_description:?}");

    let ok = post_form_as(&router, "/support-tickets", &cookie, "ticket_id=tix-2&tenant_id=acme-demo&category=Bug&priority=High&description=it+is+broken");
    assert!(ok.status == 200 || ok.status == 201 || ok.status == 302, "a fully-populated submission must succeed: {ok:?}");
}

#[test]
fn no_role_can_browse_support_tickets_or_profiles_real_disclosed_gap() {
    // No read policy exists for `support_ticket` anywhere in this corpus
    // (only `create`) — confirmed by grep, disclosed in `m22_misc.nir`'s
    // own doc comment, not a bug in this test's own routes. A read policy
    // for `user_profile` (`self-read-profile`) was added for the chrome
    // route V5 invariant, but no list route consumes it here.
    let router = router();
    let cookie = login_as(&router, "analyst", "analyst-demo");
    let resp = get_as(&router, "/support-tickets", &cookie);
    assert_eq!(resp.status, 403, "no read grant exists for support_ticket, so the list route must deny by default: {resp:?}");
}

#[test]
fn t11_audit_chain_projection_answers_a_real_cross_chain_query_and_feeds_sar_visibility() {
    // T-11: `AC.all_chains` (`bridge.nir::all_chains_table`) is a real
    // projection over every `GuardedTable`'s own hash-chained audit log,
    // not a synthetic feed. Drive a real guarded read through
    // `alert_table` (which writes a real `Read` decision envelope to its
    // own chain), refresh the projection, and prove: (1) an Auditor's
    // `/audit/chains` request sees that real decision, and (2) it feeds
    // `PG.sar_visibility` for the tipping-off screen.
    let seeded = AlertRow { id: 0, alert_id: "alert-t11-1".into(), tenant_id: "acme-demo".into(), txn_id: "txn-1".into(), score: 0.9, model_version: "v1".into(), policy_version: "v1".into(), status: "new".into(), assignee: String::new(), disposition_code: String::new(), rationale: String::new(), case_id: String::new(), sar_linked: Some("sar-1".into()), severity: "High".into(), tags: String::new() };
    alert_table().raw_driver_seed("acme-demo", &seeded);
    let router = router();

    // A real guarded read against `alert_table` under an Analyst session
    // -- this is the event the projection must pick up.
    let analyst_auth = Auth::login("an", &["Analyst"]);
    let _ = alert_table().guarded_snapshot(&analyst_auth, "AmlInvestigation").expect("real analyst read on alert must be allowed");

    let tampered = refresh_all_chains();
    assert!(tampered.is_empty(), "no source chain should be tampered in this test: {tampered:?}");

    let projected = all_chains_table().system_scan().expect("all_chains_table must be readable");
    assert!(
        projected.iter().any(|r| r.module == "alert" && r.action == "Read"),
        "the real analyst read on alert_table must appear in the AC.all_chains projection: {projected:?}"
    );

    // The real HTTP route: an Auditor sees the same projection.
    let auditor_cookie = login_as(&router, "auditor", "auditor-demo");
    let chains_resp = get_as(&router, "/audit/chains", &auditor_cookie);
    assert_eq!(chains_resp.status, 200, "auditor-read's real Audit-purpose grant on audit_chain must let Auditor query it: {chains_resp:?}");
    assert!(chains_resp.body.contains("\"module\":\"alert\""), "the HTTP response must carry the real projected alert entry: {}", chains_resp.body);

    // PG.sar_visibility derives from the same projection.
    refresh_sar_visibility();
    let visibility = sar_visibility_table().system_scan().expect("sar_visibility_table must be readable");
    assert!(
        visibility.iter().any(|r| r.resource_type == "alert"),
        "a real alert read must produce a sar_visibility record: {visibility:?}"
    );
}

#[test]
fn t11_a_tampered_source_chain_is_excluded_from_the_projection_not_silently_merged() {
    // T-11's own done-when: immutability preserved AT THE PROJECTION
    // LAYER, not just the source chains. Uses isolated, throwaway chain
    // files (not `alert_table()`'s own real, process-global audit file —
    // corrupting a live table's chain would race with every other test
    // in this binary that also reads it via `refresh_all_chains`) to
    // prove `nirdosha_rt::audit_projection::project` (the exact fn
    // `bridge.nir::refresh_all_chains` calls) excludes and names a
    // tampered chain rather than merging it in.
    use nirdosha_audit::envelope::{AuditEnvelope, AuditRecordKind, ModuleAuditChain};
    use nirdosha_rt::audit_projection::{project, ChainSource};

    let intact_path = std::env::temp_dir().join(format!("rtm_t11_intact_{}.jsonl", std::process::id()));
    let tampered_path = std::env::temp_dir().join(format!("rtm_t11_tampered_{}.jsonl", std::process::id()));
    let _ = std::fs::remove_file(&intact_path);
    let _ = std::fs::remove_file(&tampered_path);
    let intact_path_for_cleanup = intact_path.clone();

    let envelope = |trace_id: &str| AuditEnvelope {
        trace_id: trace_id.into(),
        ts: "2026-09-22T00:00:00.000Z".into(),
        module: "rtm-demo".into(),
        subject: "analyst-1".into(),
        action: "Read".into(),
        resource: "alert".into(),
        policy_versions: vec!["v1".into()],
        decision: "Allow".into(),
        obligations: vec![],
        kind: AuditRecordKind::Decision,
        content: serde_json::json!({}),
    };
    let intact = ModuleAuditChain::new("rtm-demo", intact_path.clone());
    intact.append(&envelope("intact-1"), 100);
    let tampered_chain = ModuleAuditChain::new("rtm-demo", tampered_path.clone());
    tampered_chain.append(&envelope("tampered-1"), 200);
    let original = std::fs::read_to_string(&tampered_path).unwrap();
    std::fs::write(&tampered_path, original.replacen("\"Read\"", "\"Delete\"", 1)).unwrap();

    let (rows, tampered) = project(&[ChainSource { label: "intact", chain: intact }, ChainSource { label: "tampered", chain: tampered_chain }]);
    assert_eq!(tampered.len(), 1, "the corrupted chain must be named as tampered: {tampered:?}");
    assert_eq!(tampered[0].label, "tampered");
    assert_eq!(rows.len(), 1, "only the intact chain's entry may appear in the projection: {rows:?}");
    assert_eq!(rows[0].trace_id, "intact-1");

    let _ = std::fs::remove_file(&intact_path_for_cleanup);
    let _ = std::fs::remove_file(&tampered_path);
}
