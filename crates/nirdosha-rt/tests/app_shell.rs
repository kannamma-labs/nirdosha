//! `app_shell! { .. }` end to end: generates title, static nav, role-
//! filtered nav, landing helpers, and a mount function that wires the
//! shell into `Router`.

use std::collections::HashMap;

use nirdosha_rt::{Auth, Request, Router};

nirdosha_rt::app_shell! {
    mount: mount_app_shell,
    title: "Task Tracker",
    nav: [
        { label: "Dashboard", href: "/dashboard" },
        { label: "Tasks",     href: "/tasks",     role: "user" },
        { label: "Admin",     href: "/admin",     role: "admin" },
    ],
    landing: landing_path,
}

nirdosha_rt::landing! {
    role("admin") -> "/admin",
    default -> "/dashboard",
}

#[test]
fn title_is_generated() {
    assert_eq!(app_shell_title(), "Task Tracker");
}

#[test]
fn static_nav_includes_every_link() {
    let nav = app_shell_nav();
    assert_eq!(nav.len(), 3);
    assert!(nav.iter().any(|l| l.label == "Dashboard" && l.href == "/dashboard"));
    assert!(nav.iter().any(|l| l.label == "Tasks" && l.href == "/tasks"));
    assert!(nav.iter().any(|l| l.label == "Admin" && l.href == "/admin"));
}

#[test]
fn role_filtered_nav_omits_inaccessible_links() {
    let user = Auth::login("sita", &["user"]);
    let nav = app_shell_nav_for(&user);
    assert!(nav.iter().any(|l| l.label == "Dashboard"));
    assert!(nav.iter().any(|l| l.label == "Tasks"));
    assert!(!nav.iter().any(|l| l.label == "Admin"));

    let admin = Auth::login("priya", &["admin"]);
    let nav = app_shell_nav_for(&admin);
    assert!(nav.iter().any(|l| l.label == "Admin"));
}

#[test]
fn mount_applies_role_filtered_nav_to_html_responses() {
    let router = mount_app_shell(Router::new(|_| Auth::login("anon", &["user"])))
        .get("/", "home", |_, _| nirdosha_rt::Response::html(200, "<html><body><h1>Home</h1></body></html>"));
    let resp = router.dispatch(&Request { method: "GET".into(), path: "/".into(), headers: HashMap::new(), body: String::new() });
    assert!(resp.body.contains("<a href=\"/dashboard\""), "dashboard link should appear; got: {}", resp.body);
    assert!(resp.body.contains("<a href=\"/tasks\""), "tasks link should appear for user role; got: {}", resp.body);
    assert!(!resp.body.contains("<a href=\"/admin\""), "admin link should be hidden from user; got: {}", resp.body);
}

#[test]
fn landing_delegates_to_named_mount() {
    let admin = Auth::login("priya", &["admin"]);
    assert_eq!(app_shell_landing(&admin), "/admin");

    let user = Auth::login("sita", &["user"]);
    assert_eq!(app_shell_landing(&user), "/dashboard");
}
