//! `settings_screen!` end to end: a singleton record with view + edit,
//! no list/create/delete, no id — RFC 0009 Track C's Settings archetype.

use nirdosha_rt::{Auth, Request, Response, Router};
use nirdosha_rt::prelude::SharedCell;

nirdosha_rt::roles! {
    Admin = "admin";
}

#[derive(Clone, Default, serde::Serialize)]
struct AppSettings {
    site_name: String,
    max_upload_mb: i64,
    maintenance_mode: bool,
}

fn app_settings_store() -> &'static nirdosha_rt::prelude::SharedCell<AppSettings> {
    static STORE: std::sync::OnceLock<nirdosha_rt::prelude::SharedCell<AppSettings>> = std::sync::OnceLock::new();
    STORE.get_or_init(|| {
        SharedCell::new(AppSettings {
            site_name: "Acme".to_string(),
            max_upload_mb: 10,
            maintenance_mode: false,
        })
    })
}

nirdosha_rt::settings_screen! {
    mount: mount_app_settings,
    entity: AppSettings,
    store: app_settings_store,
    path: "/settings",
    fields: [ site_name: String, max_upload_mb: i64, maintenance_mode: bool ],
    access: requires role "admin",
}

fn router() -> Router {
    mount_app_settings(Router::new(|req: &Request| {
        let roles: Vec<&str> = req.header("x-roles").map(|s| s.split(',').collect()).unwrap_or_default();
        Auth::login("test", &roles)
    }))
}

fn req(method: &str, path: &str, roles: Option<&str>, body: &str) -> Request {
    let mut headers = std::collections::HashMap::new();
    if let Some(r) = roles {
        headers.insert("x-roles".to_string(), r.to_string());
    }
    if body.trim_start().starts_with('{') {
        headers.insert("content-type".to_string(), "application/json".to_string());
    }
    Request { method: method.into(), path: path.into(), headers, body: body.into() }
}

fn dispatch(method: &str, path: &str, roles: Option<&str>, body: &str) -> Response {
    router().dispatch(&req(method, path, roles, body))
}

#[test]
fn settings_screen_full_session() {
    // 1. Anonymous can't view or edit settings.
    let denied = dispatch("GET", "/settings", None, "");
    assert_eq!(denied.status, 403);

    // 2. Admin sees the current settings, seeded at startup.
    let view = dispatch("GET", "/settings", Some("admin"), "");
    assert_eq!(view.status, 200);
    assert!(view.body.contains("Acme"));
    assert!(view.body.contains("/settings/edit"));

    let api_view = dispatch("GET", "/api/settings", Some("admin"), "");
    assert_eq!(api_view.status, 200);
    let doc: serde_json::Value = serde_json::from_str(&api_view.body).unwrap();
    assert_eq!(doc["site_name"], "Acme");
    assert_eq!(doc["max_upload_mb"], 10);
    assert_eq!(doc["maintenance_mode"], false);

    // 3. Edit form is pre-filled with current values.
    let form = dispatch("GET", "/settings/edit", Some("admin"), "");
    assert_eq!(form.status, 200);
    assert!(form.body.contains("value=\"Acme\""));

    // 4. A non-numeric value for a numeric field is a real 400.
    let bad = dispatch("POST", "/settings/edit", Some("admin"), "site_name=Acme&max_upload_mb=lots&maintenance_mode=on");
    assert_eq!(bad.status, 400);
    assert!(bad.body.contains("max_upload_mb"));

    // 5. A good update actually mutates the singleton -- no id involved,
    // just one row updated in place -- and redirects back to the view.
    let ok = dispatch(
        "PUT",
        "/api/settings",
        Some("admin"),
        r#"{"site_name":"Acme Corp","max_upload_mb":25,"maintenance_mode":true}"#,
    );
    assert_eq!(ok.status, 200);
    let after: serde_json::Value = serde_json::from_str(&after_body()).unwrap();
    assert_eq!(after["site_name"], "Acme Corp");
    assert_eq!(after["max_upload_mb"], 25);
    assert_eq!(after["maintenance_mode"], true);

    // 6. The HTML edit path also mutates the same singleton store.
    let redirect = dispatch("POST", "/settings/edit", Some("admin"), "site_name=Acme+Again&max_upload_mb=50&maintenance_mode=");
    assert_eq!(redirect.status, 302);
    let after2: serde_json::Value = serde_json::from_str(&after_body()).unwrap();
    assert_eq!(after2["site_name"], "Acme Again");
    assert_eq!(after2["max_upload_mb"], 50);
    assert_eq!(after2["maintenance_mode"], false);

    // 7. Non-admin can't update either.
    let denied_update = dispatch("PUT", "/api/settings", None, r#"{"site_name":"Hacked","max_upload_mb":1,"maintenance_mode":false}"#);
    assert_eq!(denied_update.status, 403);
}

fn after_body() -> String {
    dispatch("GET", "/api/settings", Some("admin"), "").body
}

#[test]
fn openapi_lists_settings_routes_with_the_right_roles() {
    let resp = dispatch("GET", "/openapi.json", None, "");
    let doc: serde_json::Value = serde_json::from_str(&resp.body).unwrap();
    assert_eq!(doc["paths"]["/api/settings"]["put"]["security"][0]["nirdoshaRole"][0], "admin");
    assert!(doc["paths"]["/settings"]["get"]["security"].is_object() || doc["paths"]["/settings"]["get"]["security"][0]["nirdoshaRole"][0] == "admin");
}
