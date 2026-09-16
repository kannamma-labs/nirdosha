//! `wizard!` end to end: a two-step onboarding flow whose in-progress
//! answers live server-side, keyed by an opaque cookie, with no
//! client-side JS -- each step is a plain browser `<form>` POST that
//! redirects to the next step.

use nirdosha_rt::{Auth, Request, Response, Router};
use std::collections::HashMap;
use std::sync::Mutex;

nirdosha_rt::roles! {
    Admin = "admin";
}

#[derive(Clone, Default, serde::Serialize)]
struct Employee {
    id: i64,
    name: String,
    department: String,
    salary: f64,
}

fn employee_store() -> &'static Mutex<HashMap<i64, Employee>> {
    static STORE: std::sync::OnceLock<Mutex<HashMap<i64, Employee>>> = std::sync::OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

nirdosha_rt::wizard! {
    mount: mount_onboarding_wizard,
    entity: Employee,
    store: employee_store,
    path: "/onboarding",
    access: requires role "admin",
    steps: [
        { name: "Basics", fields: [ name: String, department: String ] },
        { name: "Compensation", fields: [ salary: f64 ] },
    ],
}

fn router() -> Router {
    mount_onboarding_wizard(Router::new(|req: &Request| {
        let roles: Vec<&str> = req.header("x-roles").map(|s| s.split(',').collect()).unwrap_or_default();
        Auth::login("test", &roles)
    }))
}

fn req(method: &str, path: &str, roles: Option<&str>, cookie: Option<&str>, body: &str) -> Request {
    let mut headers = HashMap::new();
    if let Some(r) = roles {
        headers.insert("x-roles".to_string(), r.to_string());
    }
    if let Some(c) = cookie {
        headers.insert("cookie".to_string(), c.to_string());
    }
    Request { method: method.into(), path: path.into(), headers, body: body.into() }
}

fn dispatch(method: &str, path: &str, roles: Option<&str>, cookie: Option<&str>, body: &str) -> Response {
    router().dispatch(&req(method, path, roles, cookie, body))
}

fn set_cookie(resp: &Response) -> Option<String> {
    resp.extra_headers.iter().find(|(k, _)| k == "Set-Cookie").map(|(_, v)| v.split(';').next().unwrap().to_string())
}

#[test]
fn wizard_full_session() {
    // 1. Anonymous can't even start the wizard.
    let denied = dispatch("GET", "/onboarding", None, None, "");
    assert_eq!(denied.status, 403);

    // 2. Starting redirects to step 1.
    let start = dispatch("GET", "/onboarding", Some("admin"), None, "");
    assert_eq!(start.status, 302);
    assert_eq!(start.extra_headers.iter().find(|(k, _)| k == "Location").unwrap().1, "/onboarding/step/1");

    let step1_form = dispatch("GET", "/onboarding/step/1", Some("admin"), None, "");
    assert_eq!(step1_form.status, 200);
    assert!(step1_form.body.contains("Step 1 of 2"));

    // 3. A valid step-1 submission mints a run cookie and redirects to step 2.
    let step1_ok = dispatch("POST", "/onboarding/step/1", Some("admin"), None, "name=Alice&department=Engineering");
    assert_eq!(step1_ok.status, 302);
    assert_eq!(step1_ok.extra_headers.iter().find(|(k, _)| k == "Location").unwrap().1, "/onboarding/step/2");
    let cookie = set_cookie(&step1_ok).expect("step 1 must mint a wizard-run cookie");

    // 4. Step 2's own form renders with its own fields.
    let step2_form = dispatch("GET", "/onboarding/step/2", Some("admin"), Some(&cookie), "");
    assert_eq!(step2_form.status, 200);
    assert!(step2_form.body.contains("Step 2 of 2"));

    // 5. A non-numeric value for step 2's numeric field is a real 400.
    let bad2 = dispatch("POST", "/onboarding/step/2", Some("admin"), Some(&cookie), "salary=lots");
    assert_eq!(bad2.status, 400);
    assert!(bad2.body.contains("salary"));

    // 6. The store has nothing yet -- a rejected final step must not
    // create a half-finished entity.
    assert!(employee_store().lock().unwrap().is_empty());

    // 7. Completing step 2 assembles the FULL entity (step 1's answers
    // plus step 2's), inserts it into the shared datasource, clears the
    // cookie, and redirects to the done page.
    let step2_ok = dispatch("POST", "/onboarding/step/2", Some("admin"), Some(&cookie), "salary=95000");
    assert_eq!(step2_ok.status, 302);
    let done_location = step2_ok.extra_headers.iter().find(|(k, _)| k == "Location").unwrap().1.clone();
    assert!(done_location.starts_with("/onboarding/done/"), "got: {done_location}");
    let cleared_cookie = step2_ok.extra_headers.iter().find(|(k, _)| k == "Set-Cookie").unwrap().1.clone();
    assert!(cleared_cookie.contains("Max-Age=0"), "got: {cleared_cookie}");

    {
        let store = employee_store().lock().unwrap();
        assert_eq!(store.len(), 1);
        let employee = store.values().next().unwrap();
        assert_eq!(employee.name, "Alice");
        assert_eq!(employee.department, "Engineering");
        assert_eq!(employee.salary, 95000.0);
    }

    // 8. The done page shows the completed record.
    let done = dispatch("GET", &done_location, Some("admin"), None, "");
    assert_eq!(done.status, 200);
    assert!(done.body.contains("Alice"));
    assert!(done.body.contains("95000"));
}

#[test]
fn openapi_lists_wizard_steps_with_the_right_role() {
    let resp = dispatch("GET", "/openapi.json", None, None, "");
    let doc: serde_json::Value = serde_json::from_str(&resp.body).unwrap();
    assert_eq!(doc["paths"]["/onboarding/step/1"]["post"]["security"][0]["nirdoshaRole"][0], "admin");
    assert_eq!(doc["paths"]["/onboarding/step/2"]["get"]["security"][0]["nirdoshaRole"][0], "admin");
}
