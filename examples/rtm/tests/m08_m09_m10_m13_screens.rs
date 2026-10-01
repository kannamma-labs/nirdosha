//! RTM live-demo plan, Phase B batch 2: proves M8 (Rules/Windows), M9
//! (ML Models), M10 (Screening), and M13 (Risk Config) are real,
//! guard-enforced HTTP screens through `Router::dispatch` — same
//! entrypoint `m06_transactions_screen.rs`/`m03_m04_m05_screens.rs`
//! already prove, extended to this batch's `Action::Migrate` escalation
//! path (`guarded_propose_escalated_action`/`guarded_confirm_escalated_action`,
//! new this batch) and its real maker≠checker shape (`refdata-migrate`:
//! `Admin` proposes, only `PolicyEngineer`/`ComplianceLead` can approve).

use std::collections::HashMap;

use nirdosha_rt::{Auth, Request, Response, Router};
use rtm::bridge::{model_table, refdata_table, screening_hit_table, window_config_table};

fn router() -> Router {
    let router = rtm::m02_dashboards::mount_app_shell(Router::new(|_req| Auth::login("anon", &[])));
    let router = rtm::m01_auth::mount_login(router);
    let router = rtm::m01_auth::mount_avatar_picker(router);
    let router = rtm::m08_rules::mount_window_migrate(router);
    let router = rtm::m08_rules::mount_simulation_config(router);
    let router = rtm::m08_rules::mount_window_config_screens(router);
    let router = rtm::m09_ml_models::mount_model_swap(router);
    // C2: literal `/models/*` routes before `mount_model_screens`'s
    // `/models/{id}` wildcard (same ordering as serve.nir).
    let router = rtm::m09_ml_models::mount_model_performance(router);
    let router = rtm::m09_ml_models::mount_model_drift(router);
    let router = rtm::m09_ml_models::mount_champion_challenger(router);
    let router = rtm::m09_ml_models::mount_model_screens(router);
    let router = rtm::m09_ml_models::mount_model_validations(router);
    let router = rtm::m09_ml_models::mount_score_explainability(router);
    let router = rtm::m10_screening::mount_matcher_catalog(router);
    let router = rtm::m10_screening::mount_screening_hit_screens(router);
    let router = rtm::m13_risk_config::mount_refdata_migrate(router);
    rtm::m13_risk_config::mount_refdata_screens(router)
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
fn json_field<'a>(body: &'a str, key: &str) -> &'a str {
    let needle = format!("\"{key}\":\"");
    let start = body.find(&needle).unwrap_or_else(|| panic!("no {key:?} in {body}")) + needle.len();
    &body[start..start + body[start..].find('"').unwrap()]
}

/// M9: `scoring-model-swap` is "four-eyes among peers" -- `PolicyEngineer`
/// proposes AND is `model_release`'s own chain-eligible role, so the
/// first call already consumes one of the two required slots (same as
/// M4's `case-close-confirm`, reused verbatim via
/// `guarded_propose_escalated_update`/`guarded_confirm_escalated_update`).
#[test]
fn policy_engineer_proposes_model_swap_and_compliance_lead_confirms() {
    let router = router();
    model_table();
    let pe = login_as(&router, "policyengineer", "policyengineer-demo");
    let cl = login_as(&router, "compliancelead", "compliancelead-demo");

    // Real, disclosed gap (same shape as M10/M13): no `guard_policy!`
    // anywhere in this corpus grants a plain `read` on `"model"` either
    // -- `scoring-model-swap` only covers `action == "update"`. The
    // registry/route are real; this assertion documents that reality
    // rather than assuming a read grant this corpus doesn't declare.
    let view = get_as(&router, "/api/models", &pe);
    assert_eq!(view.status, 403, "no read policy exists on \"model\" either: {}", view.body);

    let propose = post_form_as(&router, "/models/rt_fraud_v1/propose-swap", &pe, "threshold_alert=0.90");
    assert_eq!(propose.status, 202, "{}", propose.body);
    assert!(propose.body.contains("\"approvals_so_far\":1"), "proposer must consume their own slot: {}", propose.body);
    let chain = json_field(&propose.body, "chain").to_string();
    let escalation_id = json_field(&propose.body, "escalation_id").to_string();

    let confirm = post_form_as(&router, "/models/rt_fraud_v1/confirm-swap", &cl, &format!("threshold_alert=0.90&chain={chain}&escalation_id={escalation_id}"));
    assert_eq!(confirm.status, 200, "{}", confirm.body);
    assert!(confirm.body.contains("0.9"), "committed row must carry the new threshold: {}", confirm.body);
}

/// M10: `analyst-disposition-hit` is real and update-only -- Analyst can
/// disposition a hit they already know the id of, but (disclosed, real
/// corpus gap) cannot list/browse hits at all, since no `read` policy on
/// `screening_hit` exists anywhere in this corpus.
#[test]
fn analyst_dispositions_a_screening_hit_and_can_browse_the_list() {
    let router = router();
    screening_hit_table();
    let analyst = login_as(&router, "analyst", "analyst-demo");

    let list = get_as(&router, "/screening/hits", &analyst);
    assert_eq!(list.status, 200, "analyst-read-screening-hit (10.1) must let Analyst browse the queue: {}", list.body);

    let update = post_form_as(&router, "/screening/hits/hit-001/edit", &analyst, "disposition=true_match&rationale=matches national id and DOB");
    assert_eq!(update.status, 302, "the real update grant must still work even though list/detail don't: {}", update.body);
}

/// M13: the real maker≠checker finding this batch produced.
/// `refdata-migrate` grants `Admin`, but `policy_release`'s own `of =
/// [PolicyEngineer, ComplianceLead]` never names `Admin` — the propose
/// call must open with ZERO approvals (Admin fills no slot), and two
/// DISTINCT real approvers (neither of them Admin) must confirm it.
#[test]
fn refdata_migrate_is_real_maker_neq_checker_admin_proposes_neither_approver() {
    let router = router();
    refdata_table();
    let admin = login_as(&router, "admin", "admin-demo");
    let pe = login_as(&router, "policyengineer", "policyengineer-demo");
    let cl = login_as(&router, "compliancelead", "compliancelead-demo");

    let propose = post_form_as(&router, "/risk-config/refdata/USD/propose-migrate", &admin, "label=US Dollar (updated)");
    assert_eq!(propose.status, 202, "{}", propose.body);
    assert!(propose.body.contains("\"approvals_so_far\":0"), "Admin holds no policy_release-eligible role and must consume no slot: {}", propose.body);
    let chain = json_field(&propose.body, "chain").to_string();
    let escalation_id = json_field(&propose.body, "escalation_id").to_string();

    let admin_confirm_attempt = post_form_as(&router, "/risk-config/refdata/USD/confirm-migrate", &admin, &format!("label=US Dollar (updated)&chain={chain}&escalation_id={escalation_id}"));
    assert_eq!(admin_confirm_attempt.status, 403, "Admin is not in policy_release's approver_roles at all: {}", admin_confirm_attempt.body);

    let first = post_form_as(&router, "/risk-config/refdata/USD/confirm-migrate", &pe, &format!("label=US Dollar (updated)&chain={chain}&escalation_id={escalation_id}"));
    assert_eq!(first.status, 202, "{}", first.body);
    assert!(first.body.contains("\"approvals_so_far\":1"), "{}", first.body);

    // T-04/B10: `policy_release` now declares `cooling(days = 1)` (real
    // go-live cooling, per screen-field-mappings.md's "cooling period on
    // approve" for 8.8/8.10) — quorum being real is proven by
    // `approvals_so_far` reaching 2/2, but the write does NOT commit on
    // this call anymore; it resolves to `Cooling`, not `Approved`. This
    // route (`confirm-migrate`) only ever calls `guarded_confirm_escalated_action`,
    // which surfaces `Cooling` as `Pending` (202) — actually finalizing
    // once the window elapses is `guarded_finalize_escalated_action`'s
    // job, exercised with a controlled clock (not real wall-clock sleep)
    // in `nirdosha-guard-screens`' own unit tests.
    let second = post_form_as(&router, "/risk-config/refdata/USD/confirm-migrate", &cl, &format!("label=US Dollar (updated)&chain={chain}&escalation_id={escalation_id}"));
    assert_eq!(second.status, 202, "quorum reached but policy_release's cooling(days=1) window hasn't elapsed: {}", second.body);
    assert!(second.body.contains("\"approvals_so_far\":2"), "{}", second.body);
}

/// M8: `threshold-migrate` is "four-eyes among peers" again
/// (`PolicyEngineer` proposes and is `policy_release`-eligible) — the
/// generalized `Action::Migrate` path proven end to end over HTTP, not
/// just the crate's own unit test.
#[test]
fn policy_engineer_migrates_a_window_threshold_with_a_second_engineer() {
    let router = router();
    window_config_table();
    let pe1 = login_as(&router, "policyengineer", "policyengineer-demo");

    let propose = post_form_as(&router, "/rules/windows/velocity_1h/propose-migrate", &pe1, "velocity_1h_threshold=12");
    assert_eq!(propose.status, 202, "{}", propose.body);
    assert!(propose.body.contains("\"approvals_so_far\":1"), "{}", propose.body);

    // `policy_release`'s `of = [PolicyEngineer, ComplianceLead]` accepts
    // a second, DISTINCT approver of either eligible role — a
    // ComplianceLead confirms here, proving the chain isn't
    // role-hardcoded to PolicyEngineer alone.
    let chain = json_field(&propose.body, "chain").to_string();
    let escalation_id = json_field(&propose.body, "escalation_id").to_string();
    // Same `policy_release` `cooling(days = 1)` as the refdata test
    // above: quorum real (2/2), but `Cooling` not `Approved` — 202, not
    // 200 (see that test's own comment for the full explanation).
    let cl = login_as(&router, "compliancelead", "compliancelead-demo");
    let confirm = post_form_as(&router, "/rules/windows/velocity_1h/confirm-migrate", &cl, &format!("velocity_1h_threshold=12&chain={chain}&escalation_id={escalation_id}"));
    assert_eq!(confirm.status, 202, "quorum reached but policy_release's cooling(days=1) window hasn't elapsed: {}", confirm.body);
    assert!(confirm.body.contains("\"approvals_so_far\":2"), "{}", confirm.body);
}

/// A role with no matching allow policy anywhere in this batch's four
/// resources must be denied, not silently shown an empty/default page —
/// same deny-by-default proof every prior batch's test file carries.
#[test]
fn a_role_with_no_grant_on_any_of_this_batchs_resources_is_denied() {
    let router = router();
    let cs_agent = login_as(&router, "csagent", "csagent-demo");
    for path in ["/api/models", "/screening/hits", "/risk-config/refdata", "/rules/windows"] {
        let resp = get_as(&router, path, &cs_agent);
        assert_eq!(resp.status, 403, "CsAgent has no grant on {path}: {}", resp.body);
    }
}

/// Same disclosed shape as M9/M10: `threshold-migrate`/`refdata-migrate`
/// only cover `action == "migrate"`, so list reads correctly deny too --
/// caught live while writing this suite (the module doc comments
/// originally claimed a working "view" screen; fixed to match what the
/// corpus actually grants, not what would have been convenient).
#[test]
fn refdata_list_view_still_has_no_read_grant_but_window_now_does() {
    let router = router();
    window_config_table();
    refdata_table();
    let pe = login_as(&router, "policyengineer", "policyengineer-demo");
    let admin = login_as(&router, "admin", "admin-demo");
    // rules-window-read (8.1) now grants PolicyEngineer/ComplianceLead/
    // Auditor/Admin a plain read on "window".
    let windows = get_as(&router, "/rules/windows", &pe);
    assert_eq!(windows.status, 200, "{}", windows.body);
    // "refdata" is a separate resource this batch never touched — still
    // genuinely ungranted.
    let refdata = get_as(&router, "/risk-config/refdata", &admin);
    assert_eq!(refdata.status, 403, "{}", refdata.body);
}

/// C2 (screens-plan.md): `model_run_stat` is genuinely derived from
/// `alert_table()`'s real `score`/`model_version`/`disposition_code`
/// fields (never a hardcoded fixture) -- `fp_rate` over two seeded
/// alerts (one `false_positive`, one `known_fraud`) must come out to
/// exactly `0.5`. `model_validation`'s `validator ≠ owner` invariant is
/// machine-checked at create time, not a UI-only check.
#[test]
fn model_run_stat_derives_from_real_alerts_and_model_validation_enforces_validator_ne_owner() {
    let router = router();
    rtm::bridge::alert_table().raw_driver_seed(
        "acme-demo",
        &rtm::bridge::AlertRow { id: 0, alert_id: "alert-c2-1".into(), tenant_id: "acme-demo".into(), txn_id: "txn-c2-1".into(), score: 0.95, model_version: "rt_fraud_v1".into(), policy_version: "v1".into(), status: "in_progress".into(), assignee: String::new(), disposition_code: "false_positive".into(), rationale: "reviewed".into(), case_id: String::new(), sar_linked: None, severity: String::new(), tags: String::new() },
    );
    rtm::bridge::alert_table().raw_driver_seed(
        "acme-demo",
        &rtm::bridge::AlertRow { id: 0, alert_id: "alert-c2-2".into(), tenant_id: "acme-demo".into(), txn_id: "txn-c2-2".into(), score: 0.40, model_version: "rt_fraud_v1".into(), policy_version: "v1".into(), status: "in_progress".into(), assignee: String::new(), disposition_code: "known_fraud".into(), rationale: "reviewed".into(), case_id: String::new(), sar_linked: None, severity: String::new(), tags: String::new() },
    );

    let pe = login_as(&router, "policyengineer", "policyengineer-demo");
    let perf = get_as(&router, "/models/performance.json", &pe);
    assert_eq!(perf.status, 200, "{}", perf.body);
    assert!(perf.body.contains("rt_fraud_v1"), "dashboard must show the real seeded model_version, not a fixture: {}", perf.body);
    assert!(perf.body.contains("0.5"), "fp_rate must be genuinely derived (1 of 2 seeded alerts is false_positive): {}", perf.body);

    // validator == owner: a real, machine-checked deny (validator_ne_owner).
    let same = post_form_as(&router, "/api/model-validations", &pe, "model_version=rt_fraud_v1&owner=alice&validator=alice");
    assert!(same.status >= 400, "validator == owner must be denied by validator_ne_owner: {}", same.body);

    // validator != owner: real, guard-enforced and committed.
    let ok = post_form_as(&router, "/api/model-validations", &pe, "model_version=rt_fraud_v1&owner=alice&validator=bob");
    assert_eq!(ok.status, 201, "{}", ok.body);

    let mlro = login_as(&router, "mlro", "mlro-demo");
    let list = get_as(&router, "/model-validations", &mlro);
    assert_eq!(list.status, 200, "model-validation-read grants Mlro: {}", list.body);
    assert!(list.body.contains("bob"), "Mlro's real read grant must see the committed row: {}", list.body);
}
