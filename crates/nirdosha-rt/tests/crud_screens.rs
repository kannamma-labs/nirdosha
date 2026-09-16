//! `crud_screens!` end to end: List/Detail/Form/Delete, attached to a
//! real datasource, both HTML and JSON surfaces.
//!
//! One sequential scenario, not several independent `#[test]` fns:
//! `widget_store()` is a process-global `static`, and `cargo test` runs
//! `#[test]` functions in parallel threads within one binary — separate
//! tests would race on the same shared data. A real app has exactly
//! this same "one mutable datastore" shape, so testing it as one
//! deterministic session is the honest model, not a workaround.

use nirdosha_rt::{Auth, Request, Response, Router};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

nirdosha_rt::roles! {
    Admin = "admin";
}

#[derive(Clone, Default, serde::Serialize)]
struct Widget {
    id: i64,
    name: String,
    price_cents: i64,
}

fn widget_store() -> &'static Mutex<HashMap<i64, Widget>> {
    static STORE: OnceLock<Mutex<HashMap<i64, Widget>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

nirdosha_rt::crud_screens! {
    mount: mount_widget_screens,
    entity: Widget,
    store: widget_store,
    path: "/widgets",
    fields: [ name: String, price_cents: i64 ],
    create: requires role "admin",
    read: public,
    update: requires role "admin",
    delete: requires role "admin",
}

fn router() -> Router {
    mount_widget_screens(Router::new(|req: &Request| {
        let roles: Vec<&str> = req.header("x-roles").map(|s| s.split(',').collect()).unwrap_or_default();
        Auth::login("test", &roles)
    }))
}

fn req(method: &str, path: &str, roles: Option<&str>, body: &str) -> Request {
    let mut headers = HashMap::new();
    if let Some(r) = roles {
        headers.insert("x-roles".to_string(), r.to_string());
    }
    if body.trim_start().starts_with('{') {
        headers.insert("content-type".to_string(), "application/json".to_string());
    }
    Request {
        method: method.into(),
        path: path.into(),
        headers,
        body: body.into(),
    }
}

fn dispatch(method: &str, path: &str, roles: Option<&str>, body: &str) -> Response {
    router().dispatch(&req(method, path, roles, body))
}

#[test]
fn crud_screens_full_session() {
    // 1. Empty list shows the empty state (System State archetype).
    let resp = dispatch("GET", "/widgets", None, "");
    assert_eq!(resp.status, 200);
    assert!(resp.body.contains("No widgets yet"));

    // 2. Anonymous create is refused; admin create succeeds and round-trips.
    let denied = dispatch("POST", "/api/widgets", None, r#"{"name":"Gadget","price_cents":500}"#);
    assert_eq!(denied.status, 403);

    let created = dispatch("POST", "/api/widgets", Some("admin"), r#"{"name":"Gadget","price_cents":500}"#);
    assert_eq!(created.status, 201);
    let entity: serde_json::Value = serde_json::from_str(&created.body).unwrap();
    assert_eq!(entity["name"], "Gadget");
    let id = entity["id"].as_i64().unwrap();

    let list = dispatch("GET", "/api/widgets", None, "");
    let items: serde_json::Value = serde_json::from_str(&list.body).unwrap();
    assert_eq!(items.as_array().unwrap().len(), 1);

    let detail = dispatch("GET", &format!("/api/widgets/{id}"), None, "");
    assert_eq!(detail.status, 200);
    let detail_json: serde_json::Value = serde_json::from_str(&detail.body).unwrap();
    assert_eq!(detail_json["price_cents"], 500);

    let html_detail = dispatch("GET", &format!("/widgets/{id}"), None, "");
    assert!(html_detail.body.contains("Gadget"));

    // 3. A validation failure is a real 400 with a field-scoped message.
    let bad = dispatch("POST", "/api/widgets", Some("admin"), r#"{"name":"X","price_cents":"not-a-number"}"#);
    assert_eq!(bad.status, 400);
    assert!(bad.body.contains("price_cents"));

    // 4. Update requires admin and actually mutates the stored entity.
    let denied_update = dispatch("PUT", &format!("/api/widgets/{id}"), None, r#"{"name":"Gadget","price_cents":999}"#);
    assert_eq!(denied_update.status, 403);

    let updated = dispatch("PUT", &format!("/api/widgets/{id}"), Some("admin"), r#"{"name":"Gadget","price_cents":999}"#);
    assert_eq!(updated.status, 200);
    let after_update: serde_json::Value = serde_json::from_str(&dispatch("GET", &format!("/api/widgets/{id}"), None, "").body).unwrap();
    assert_eq!(after_update["price_cents"], 999);

    // 5. Delete needs the admin role to even see the confirm page.
    let confirm_denied = dispatch("GET", &format!("/widgets/{id}/delete"), None, "");
    assert_eq!(confirm_denied.status, 403);
    let confirm = dispatch("GET", &format!("/widgets/{id}/delete"), Some("admin"), "");
    assert!(confirm.body.contains("Type DELETE to confirm"));

    // 6. Wrong confirmation text does not delete.
    let wrong = dispatch("POST", &format!("/widgets/{id}/delete"), Some("admin"), "confirm=nope");
    assert_eq!(wrong.status, 400);
    assert_eq!(dispatch("GET", &format!("/api/widgets/{id}"), None, "").status, 200);

    // 7. Right confirmation text deletes and redirects.
    let ok = dispatch("POST", &format!("/widgets/{id}/delete"), Some("admin"), "confirm=DELETE");
    assert_eq!(ok.status, 302);
    assert_eq!(dispatch("GET", &format!("/api/widgets/{id}"), None, "").status, 404);

    // 8. Search filters generically across every field (not just
    // strings), and CSV export respects the same filter. The store is
    // empty again after step 7's delete, so these two are the only rows.
    dispatch("POST", "/api/widgets", Some("admin"), r#"{"name":"Red Gadget","price_cents":100}"#);
    dispatch("POST", "/api/widgets", Some("admin"), r#"{"name":"Blue Gizmo","price_cents":200}"#);

    let (_, body) = dispatch_query("GET", "/api/widgets", "q=gadget");
    let items: serde_json::Value = serde_json::from_str(&body).unwrap();
    let names: Vec<&str> = items.as_array().unwrap().iter().map(|v| v["name"].as_str().unwrap()).collect();
    assert!(names.iter().any(|n| n.contains("Gadget")));
    assert!(!names.iter().any(|n| n.contains("Gizmo")));

    // A numeric field is searchable too -- the point of formatting
    // every field generically rather than special-casing strings.
    let (_, body) = dispatch_query("GET", "/api/widgets", "q=200");
    let items: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(items.as_array().unwrap().iter().any(|v| v["name"] == "Blue Gizmo"));

    // CSV export respects the same filter and includes a header row.
    let (status, csv) = dispatch_query("GET", "/widgets/export.csv", "q=gadget");
    assert!(status.contains("200"), "got: {status}");
    assert!(csv.starts_with("name,price_cents\n"), "got: {csv}");
    assert!(csv.contains("Gadget"));
    assert!(!csv.contains("Gizmo"));
}

#[test]
fn literal_suffix_routes_are_not_swallowed_by_the_id_wildcard() {
    // A real bug, found live: `/{id}` is a wildcard segment that
    // matches the literal text "new" just as readily as a real id, so
    // `/widgets/new` must be registered (and therefore matched) before
    // `/widgets/{id}` or it's swallowed and never reached.
    let resp = dispatch("GET", "/widgets/new", Some("admin"), "");
    assert_eq!(resp.status, 200);
    assert!(resp.body.contains("<form"), "got: {}", resp.body);
}

fn dispatch_query(method: &str, path: &str, query: &str) -> (String, String) {
    let full_path = format!("{path}?{query}");
    let resp = dispatch(method, &full_path, None, "");
    (format!("{}", resp.status), resp.body)
}

#[test]
fn openapi_lists_every_route_with_the_right_roles() {
    let resp = dispatch("GET", "/openapi.json", None, "");
    let doc: serde_json::Value = serde_json::from_str(&resp.body).unwrap();
    assert_eq!(doc["paths"]["/api/widgets"]["post"]["security"][0]["nirdoshaRole"][0], "admin");
    assert!(doc["paths"]["/api/widgets"]["get"]["security"].is_null());
    assert!(doc["paths"]["/api/widgets/{id}"]["delete"].is_object());
}
