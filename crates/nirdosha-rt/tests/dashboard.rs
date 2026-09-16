//! `dashboard!` end to end: widget functions are called for real, JSON
//! and HTML routes agree (same widget list, one source), and a wrong
//! function name is a real compile error (proven manually — see the
//! module doc on `crates/nirdosha-macros/src/dashboard.rs`).

use nirdosha_rt::{Auth, Response, Router};

fn stat_revenue_mtd() -> i64 {
    1_250_000
}

fn chart_revenue_by_region() -> String {
    r#"[{"label":"East","value":40},{"label":"West","value":60}]"#.to_string()
}

nirdosha_rt::dashboard! {
    mount: mount_sales_dashboard,
    path: "/dashboard",
    title: "Sales Overview",
    refresh_seconds: 300,
    widgets {
        Metric { label: "Revenue MTD", fn: stat_revenue_mtd, target: 1000000, alert_below: false },
        Chart  { label: "By Region", fn: chart_revenue_by_region, mark: bar },
    }
}

fn router() -> Router {
    mount_sales_dashboard(Router::new(|_| Auth::login("anon", &[])))
}

fn dispatch(method: &str, path: &str) -> Response {
    let req = nirdosha_rt::Request {
        method: method.into(),
        path: path.into(),
        headers: Default::default(),
        body: String::new(),
    };
    router().dispatch(&req)
}

#[test]
fn json_route_reports_real_widget_values() {
    let resp = dispatch("GET", "/dashboard.json");
    assert_eq!(resp.status, 200);
    let doc: serde_json::Value = serde_json::from_str(&resp.body).unwrap();
    assert_eq!(doc["title"], "Sales Overview");
    let widgets = doc["widgets"].as_array().unwrap();
    assert_eq!(widgets.len(), 2);
    assert_eq!(widgets[0]["kind"], "metric");
    assert_eq!(widgets[0]["value"], 1_250_000.0);
    // 1,250,000 > target 1,000,000 and alert_below is false -> alert fires.
    assert_eq!(widgets[0]["alert"], true);
    assert_eq!(widgets[1]["kind"], "chart");
    assert_eq!(widgets[1]["data"][0]["label"], "East");
}

#[test]
fn html_route_renders_the_same_data_json_reports() {
    let resp = dispatch("GET", "/dashboard");
    assert_eq!(resp.status, 200);
    assert!(resp.content_type.starts_with("text/html"));
    assert!(resp.body.contains("Sales Overview"));
    assert!(resp.body.contains("1250000"));
    assert!(resp.body.contains("http-equiv=\"refresh\" content=\"300\""));
    assert!(resp.body.contains("<svg"));
}

#[test]
fn openapi_lists_both_routes() {
    let resp = dispatch("GET", "/openapi.json");
    let doc: serde_json::Value = serde_json::from_str(&resp.body).unwrap();
    assert!(doc["paths"]["/dashboard"]["get"].is_object());
    assert!(doc["paths"]["/dashboard.json"]["get"].is_object());
}

fn stat_exact_cost() -> i64 {
    42
}

nirdosha_rt::dashboard! {
    mount: mount_internal_dashboard,
    path: "/internal",
    title: "Internal",
    widgets {
        Metric { label: "Revenue", fn: stat_revenue_mtd },
        Metric { label: "Exact Cost", fn: stat_exact_cost, requires_role: "admin" },
    }
}

fn dispatch_as(path: &str, roles: &[&str]) -> Response {
    let router = mount_internal_dashboard(Router::new(|req: &nirdosha_rt::Request| {
        let roles: Vec<&str> = req.header("x-roles").map(|s| s.split(',').filter(|s| !s.is_empty()).collect()).unwrap_or_default();
        Auth::login("test", &roles)
    }));
    let mut headers = std::collections::HashMap::new();
    headers.insert("x-roles".to_string(), roles.join(","));
    let req = nirdosha_rt::Request { method: "GET".into(), path: path.into(), headers, body: String::new() };
    router.dispatch(&req)
}

#[test]
fn a_widget_with_requires_role_is_hidden_from_a_viewer_without_it() {
    let anon = dispatch_as("/internal.json", &[]);
    let doc: serde_json::Value = serde_json::from_str(&anon.body).unwrap();
    let widgets = doc["widgets"].as_array().unwrap();
    assert_eq!(widgets.len(), 1, "got: {widgets:?}");
    assert_eq!(widgets[0]["label"], "Revenue");

    let admin = dispatch_as("/internal.json", &["admin"]);
    let doc: serde_json::Value = serde_json::from_str(&admin.body).unwrap();
    let widgets = doc["widgets"].as_array().unwrap();
    assert_eq!(widgets.len(), 2, "got: {widgets:?}");
    assert!(widgets.iter().any(|w| w["label"] == "Exact Cost"));
}
