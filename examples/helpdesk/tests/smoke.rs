//! Greenfield generator proof: the entire router below comes from
//! `cargo nirdosha generate-screens` (screens.toml + menus.toml). Every
//! screen here is a generated macro invocation whose data paths route
//! through `GuardedTable`, and whose guard policies were synthesized
//! from each screen's own `policy` block in the register. The guard is
//! the single data authority — these tests prove it is, not just that
//! the screens render.

use std::collections::HashMap;

use nirdosha_rt::{Auth, Request, Response, Router};

fn router() -> Router {
    // Singleton settings row: a `row_id`-bound guard on an empty table
    // would 404 its own singleton; the generated bridge seeds it the
    // same way the generated serve binary does.
    helpdesk::bridge::seed_singletons();
    // Mount order mirrors the generated serve binary: literal-path
    // mounts before crud_screens' `/{id}` wildcards (dispatch matches in
    // registration order — `/tickets/board` must not be swallowed by
    // `GET /tickets/{id}`).
    let router = helpdesk::app_shell::mount_app_shell(Router::new(|_req| Auth::login("anon", &[])));
    let router = helpdesk::app_shell::mount_login(router);
    let router = helpdesk::m03_tickets::mount_new_ticket_wizard(router);
    let router = helpdesk::m04_dashboards::mount_ops_dashboard(router);
    let router = helpdesk::m05_admin::mount_app_settings(router);
    let router = helpdesk::m05_admin::mount_team_feed(router);
    let router = helpdesk::m06_board::mount_ticket_board(router);
    let router = helpdesk::m06_board::mount_ticket_board_move(router);
    let router = helpdesk::m07_help::mount_ticket_report(router);
    let router = helpdesk::m07_help::mount_getting_started(router);
    let router = helpdesk::m08_relations::mount_relation_tree(router);
    let router = helpdesk::m06_board::mount_ticket_lookup(router);
    let router = helpdesk::m09_workflows::mount_ticket_close_inbox(router);
    let router = helpdesk::m09_workflows::mount_ticket_workspace(router);
    helpdesk::m03_tickets::mount_tickets(router)
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
fn put_json_as(router: &Router, path: &str, cookie: &str, json: &str) -> Response {
    router.dispatch(&Request { method: "PUT".into(), path: path.into(), headers: HashMap::from([("cookie".into(), cookie.to_string()), ("content-type".into(), "application/json".into())]), body: json.into() })
}

#[test]
fn agent_logs_in_and_sees_the_generated_ticket_list() {
    let router = router();
    let cookie = login_as(&router, "sam", "agent123");
    let resp = get_as(&router, "/tickets", &cookie);
    assert_eq!(resp.status, 200, "guard read policy helpdesk-3-1-read allows Agent: {resp:?}");
}

#[test]
fn agent_creates_a_ticket_through_the_real_guard_field_policy() {
    let router = router();
    let cookie = login_as(&router, "sam", "agent123");
    let resp = post_form_as(&router, "/tickets", &cookie, "ticket_id=t-1&title=VPN%20down&priority=high&body=building%204");
    assert_eq!(resp.status, 302, "guarded create (helpdesk-3-1-create) must allow Agent: {resp:?}");
    let detail = get_as(&router, "/tickets/t-1", &cookie);
    assert_eq!(detail.status, 200, "detail read (helpdesk-3-1-read) must see the created row: {detail:?}");
    assert!(detail.body.contains("VPN down"), "detail page must show the created title: {}", detail.body);
}

#[test]
fn board_move_is_a_guard_checked_status_write() {
    let router = router();
    let cookie = login_as(&router, "sam", "agent123");
    post_form_as(&router, "/tickets", &cookie, "ticket_id=t-2&title=Onboarding&priority=med&body=x");
    // The generated move route: guarded_update on the same Ticket
    // entity, purpose Operations — a policy-checked write, not a
    // presentational drag.
    let resp = post_form_as(&router, "/tickets/t-2/move/in_progress", &cookie, "");
    assert_eq!(resp.status, 302, "guarded move (helpdesk-6-1-update) must allow Agent: {resp:?}");
    let board = get_as(&router, "/tickets/board", &cookie);
    assert_eq!(board.status, 200);
    assert!(board.body.contains("in_progress"), "board must render the moved column: {}", board.body);
}

#[test]
fn wizard_completes_through_guarded_insert_checked() {
    let router = router();
    let cookie = login_as(&router, "sam", "agent123");
    // Step 1 of 2: Basics — a valid POST redirects to step 2.
    let step1 = post_form_as(&router, "/tickets/wizard/step/1", &cookie, "ticket_id=w-1&title=Access+card&priority=low");
    assert_eq!(step1.status, 302, "wizard step 1 must advance to step 2: {step1:?}");
    assert_eq!(step1.extra_headers.iter().find(|(k, _)| k == "Location").map(|(_, v)| v.as_str()), Some("/tickets/wizard/step/2"));
    // Step 2 of 2: Confirm — completing the wizard inserts through
    // GuardedTable::guarded_insert_checked (policy helpdesk-3-2-create).
    // The step's response carries the wizard-state cookie alongside the
    // session cookie; send both.
    let wizard_cookie = step1.extra_headers.iter().find(|(k, _)| k == "Set-Cookie").map(|(_, v)| v.split(';').next().unwrap().to_string()).expect("wizard state cookie");
    let both = format!("{cookie}; {wizard_cookie}");
    let step2 = post_form_as(&router, "/tickets/wizard/step/2", &both, "body=needs+a+new+access+card");
    assert_eq!(step2.status, 302, "wizard completion must redirect to the done page: {step2:?}");
    let done_path = step2.extra_headers.iter().find(|(k, _)| k == "Location").map(|(_, v)| v.clone()).expect("done redirect");
    let done = get_as(&router, &done_path, &both);
    assert_eq!(done.status, 200, "done page must read the created row back through guarded_get: {done:?}");
    assert!(done.body.contains("Access card"), "done page must show the wizard-created ticket: {}", done.body);
}

#[test]
fn settings_singleton_is_seeded_and_admin_only() {
    let router = router();
    // Agent has NO settings policy — the guard denies (single authority).
    let agent = login_as(&router, "sam", "agent123");
    let resp = get_as(&router, "/settings", &agent);
    assert_eq!(resp.status, 403, "Agent must be denied by the guard corpus (no settings read policy): {resp:?}");

    // Admin's synthesized policy reads the boot-seeded singleton.
    let admin = login_as(&router, "ada", "admin123");
    let resp = get_as(&router, "/settings", &admin);
    assert_eq!(resp.status, 200, "Admin (helpdesk-5-1-read) must read the seeded singleton: {resp:?}");
}

#[test]
fn admin_updates_settings_through_the_guard() {
    let router = router();
    let admin = login_as(&router, "ada", "admin123");
    let resp = put_json_as(&router, "/api/settings", &admin, r#"{"site_name":"Helpdesk QA","maintenance_mode":false}"#);
    assert_eq!(resp.status, 200, "guarded update (helpdesk-5-1-update) must allow Admin: {resp:?}");
    let json: serde_json::Value = serde_json::from_str(&resp.body).expect("settings update returns the row JSON");
    assert_eq!(json["site_name"], "Helpdesk QA", "guard must have committed the update");
}

#[test]
fn feed_post_roundtrips_through_the_guard() {
    let router = router();
    let agent = login_as(&router, "sam", "agent123");
    let resp = post_form_as(&router, "/feed", &agent, "author=sam&body=demo+message");
    assert_eq!(resp.status, 302, "guarded feed post (helpdesk-5-2-create) must allow Agent: {resp:?}");
    let feed = get_as(&router, "/feed", &agent);
    assert_eq!(feed.status, 200);
    assert!(feed.body.contains("demo message"), "feed view must show the posted message: {}", feed.body);
}

#[test]
fn dashboard_widgets_route_through_the_guard() {
    let router = router();
    let agent = login_as(&router, "sam", "agent123");
    let resp = get_as(&router, "/dashboard/ops", &agent);
    assert_eq!(resp.status, 200, "dashboard widgets (guarded: true) must read ticket_table through the guard: {resp:?}");
}

#[test]
fn static_help_page_serves_with_its_pinned_digest() {
    // Public: no cookie, no login — static_embed is presentational and
    // the route gate is the whole story.
    let router = router();
    let resp = router.dispatch(&Request { method: "GET".into(), path: "/help".into(), headers: HashMap::new(), body: String::new() });
    assert_eq!(resp.status, 200, "the help page is public: {resp:?}");
    assert!(resp.body.contains("Getting Started"), "page must render the embedded content");
    assert!(resp.body.contains("8b758e64117e038bfddb5132615ca6dce3ffa86a24a7f7106f2a6960e7bfa72a"), "page must disclose its pinned sha256: {}", resp.body);
}

#[test]
fn report_runs_under_the_aggregate_policy_and_counts_by_dimension() {
    let router = router();
    let agent = login_as(&router, "sam", "agent123");
    post_form_as(&router, "/tickets", &agent, "ticket_id=r-1&title=One&priority=high&body=x");
    post_form_as(&router, "/tickets", &agent, "ticket_id=r-2&title=Two&priority=low&body=y");
    let resp = post_form_as(&router, "/reports/tickets", &agent, "dimension=priority");
    assert_eq!(resp.status, 200, "aggregate policy helpdesk-7-2-aggregate must allow Agent: {resp:?}");
    assert!(resp.body.contains("high") && resp.body.contains("low"), "report must group counts by the chosen dimension: {}", resp.body);
    assert!(resp.body.contains("2"), "counts must sum the created rows: {}", resp.body);
    let unknown = post_form_as(&router, "/reports/tickets", &agent, "dimension=body");
    assert_eq!(unknown.status, 400, "a non-declared dimension must be refused, not silently run: {unknown:?}");
}

#[test]
fn anonymous_report_run_is_denied_by_the_guard() {
    // No session cookie: the *_with_auth fallback yields an Auth with no
    // roles, and no aggregate policy matches — deny by default, at the
    // data plane.
    let router = router();
    let resp = router.dispatch(&Request { method: "POST".into(), path: "/reports/tickets".into(), headers: HashMap::from([("content-type".into(), "application/x-www-form-urlencoded".into())]), body: "dimension=status".into() });
    assert_eq!(resp.status, 403, "no policy for anon = guard deny: {resp:?}");
}

#[test]
fn viewer_sees_tickets_without_the_masked_field_and_agent_still_does() {
    // Seed one ticket carrying a real internal note.
    use helpdesk::bridge::{ticket_table, TicketRow};
    ticket_table().system_write("default", &TicketRow { id: 0, ticket_id: "t-9".into(), tenant_id: "default".into(), title: "Mask probe".to_string(), priority: "high".into(), status: "open".into(), body: "visible body".into(), internal_notes: Some("vip escalation path".into()) });
    let router = router();
    // Agent: no field_policy on helpdesk-3-1-read — sees everything.
    let agent = login_as(&router, "sam", "agent123");
    let resp = get_as(&router, "/api/tickets/t-9", &agent);
    assert_eq!(resp.status, 200, "agent read must be allowed: {resp:?}");
    assert!(resp.body.contains("vip escalation"), "Agent must see internal_notes: {}", resp.body);
    // Viewer: helpdesk-6-2-read's forbidden(internal_notes) drops the
    // key — genuine absence, not a placeholder.
    let viewer = login_as(&router, "vic", "viewer123");
    let resp = get_as(&router, "/api/tickets/lookup/t-9", &viewer);
    assert_eq!(resp.status, 200, "viewer read must be allowed via 6.2's route: {resp:?}");
    assert!(!resp.body.contains("vip escalation"), "masked field must be absent for Viewer: {}", resp.body);
    assert!(!resp.body.contains("internal_notes"), "the key itself must be gone for Viewer: {}", resp.body);
    assert!(resp.body.contains("open"), "Viewer must still see the declared fields: {}", resp.body);
}

#[test]
fn relation_tree_nests_the_guard_decoded_rows() {
    // Seed a parent/child chain the way the serve binary seeds
    // singletons: system_write is the one legitimate non-screen write.
    use helpdesk::bridge::{relation_table, RelationRow};
    relation_table().system_write("default", &RelationRow { id: 0, relation_id: "rel-a".into(), parent_id: String::new(), customer_ref: "c-1".into(), tenant_id: "default".into(), name: "Acme Holdings".to_string() });
    relation_table().system_write("default", &RelationRow { id: 0, relation_id: "rel-b".into(), parent_id: "rel-a".into(), customer_ref: "c-1".into(), tenant_id: "default".into(), name: "Acme Subsidiary".to_string() });
    relation_table().system_write("default", &RelationRow { id: 0, relation_id: "rel-c".into(), parent_id: "rel-b".into(), customer_ref: "c-1".into(), tenant_id: "default".into(), name: "UBO Trust".to_string() });
    let router = router();
    let agent = login_as(&router, "sam", "agent123");
    let resp = get_as(&router, "/relations/tree", &agent);
    assert_eq!(resp.status, 200, "read policy helpdesk-8-1-read must allow Agent: {resp:?}");
    for label in ["Acme Holdings", "Acme Subsidiary", "UBO Trust"] {
        assert!(resp.body.contains(label), "tree must render `{label}`: {}", resp.body);
    }
    // Nested: the child <li> must come after the parent's nested <ul>.
    let parent_pos = resp.body.find("Acme Holdings").expect("parent present");
    let child_pos = resp.body.find("Acme Subsidiary").expect("child present");
    let nested = resp.body[parent_pos..child_pos].contains("<ul>");
    assert!(nested, "children must nest under the parent: {}", resp.body);
}
#[test]
fn approval_inbox_guard_mode_denies_anon_and_shows_the_worklist_to_admin() {
    use helpdesk::bridge::ticket_table;
    let router = router();
    // No session: the *_with_auth route resolves an anonymous Auth, and
    // guarded_list_pending_approvals denies the read (no policy matches
    // role-less anon) at the data plane.
    let resp = router.dispatch(&Request { method: "GET".into(), path: "/approvals".into(), headers: HashMap::new(), body: String::new() });
    assert_eq!(resp.status, 403, "anon must be denied the worklist read by the guard: {resp:?}");
    // Admin holds the inbox's own synthesized read record (helpdesk-9-1-read):
    // the guard-mode view route is get_with_auth + a guarded worklist fetch.
    let admin = login_as(&router, "ada", "admin123");
    let resp = get_as(&router, "/approvals", &admin);
    assert_eq!(resp.status, 200, "admin must read the guarded inbox: {resp:?}");
    assert!(resp.body.contains("Nothing pending"), "no escalation has opened in the demo, so the worklist renders honestly empty: {}", resp.body);
    let _ = &ticket_table;
}

#[test]
fn workspace_panels_are_fail_whole_not_partial_per_subject() {
    use helpdesk::bridge::{feed_post_table, FeedPostRow, ticket_table, TicketRow};
    ticket_table().system_write("default", &TicketRow { id: 0, ticket_id: "ws-1".into(), tenant_id: "default".into(), title: "Workspace subject".to_string(), priority: "high".into(), status: "open".into(), body: "subject body".into(), internal_notes: None });
    feed_post_table().system_write("default", &FeedPostRow { id: 0, post_id: "p-1".into(), tenant_id: "default".into(), author: "sam".into(), body: "related feed row".into(), ticket_id: "ws-1".into() });
    let router = router();
    // Agent: subject read (3-1) and panel read (5-2) both allowed — the
    // generated panel fn filters the guarded snapshot to the subject row.
    let agent = login_as(&router, "sam", "agent123");
    let resp = get_as(&router, "/tickets/ws-1/workspace", &agent);
    assert_eq!(resp.status, 200, "workspace must assemble for a viewer with both grants: {resp:?}");
    assert!(resp.body.contains("related feed row"), "panel must show the subject's own feed rows: {}", resp.body);
    // Viewer: subject read (6-2) is allowed but the feed_post panel read
    // has no record — fail-whole-not-partial means the whole assembly is
    // refused (422), never a partial workspace.
    let viewer = login_as(&router, "vic", "viewer123");
    let vresp = get_as(&router, "/tickets/ws-1/workspace", &viewer);
    assert_eq!(vresp.status, 422, "a panel the guard denies must fail the whole workspace: {:?}", vresp);
}
