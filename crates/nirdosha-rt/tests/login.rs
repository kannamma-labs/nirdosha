//! `login! { .. }` end to end: demo mode matches listed users, production
//! mode rejects everything when `NIRDOSHA_LOGIN_USERS` is unset, and the
//! generated mount function wires into `Router::with_login`.

use std::collections::HashMap;

use nirdosha_rt::{Auth, Request, Router};

nirdosha_rt::login! {
    mount: mount_demo_login,
    path: "/login",
    mode: demo,
    demo_users: [
        { username: "admin", password: "secret", roles: ["admin"] },
        { username: "user",  password: "ok",     roles: ["user"] },
    ],
    landing: landing_path,
}

nirdosha_rt::landing! {
    role("admin") -> "/admin",
    default -> "/dashboard",
}

fn login_request(username: &str, password: &str) -> Request {
    let body = format!("username={}&password={}", username, password);
    Request {
        method: "POST".into(),
        path: "/login".into(),
        headers: HashMap::from([("content-type".into(), "application/x-www-form-urlencoded".into())]),
        body,
    }
}

fn cookie_from_set_cookie(resp: &nirdosha_rt::Response) -> Option<String> {
    resp.extra_headers.iter().find_map(|(k, v)| {
        if k == "Set-Cookie" {
            v.split(';').next().map(|s| s.split_once('=').map(|(_, v)| v.to_string()))?
        } else {
            None
        }
    })
}

#[test]
fn demo_login_accepts_matching_user_and_rejects_others() {
    let router = mount_demo_login(Router::new(|_| Auth::login("anon", &[])));

    let ok = router.dispatch(&login_request("admin", "secret"));
    assert!(cookie_from_set_cookie(&ok).is_some(), "successful demo login should set a session cookie");

    let bad = router.dispatch(&login_request("admin", "wrong"));
    assert!(cookie_from_set_cookie(&bad).is_none(), "wrong password should not set a session cookie");

    let unknown = router.dispatch(&login_request("nobody", "x"));
    assert!(cookie_from_set_cookie(&unknown).is_none(), "unknown user should not set a session cookie");
}

#[test]
fn demo_login_redirects_to_landing_path_based_on_role() {
    let router = mount_demo_login(Router::new(|_| Auth::login("anon", &[])));

    let admin_resp = router.dispatch(&login_request("admin", "secret"));
    let loc_admin = admin_resp.extra_headers.iter().find(|(k, _)| k == "Location").map(|(_, v)| v.as_str());
    assert_eq!(loc_admin, Some("/admin"), "admin should redirect to /admin");

    let user_resp = router.dispatch(&login_request("user", "ok"));
    let loc_user = user_resp.extra_headers.iter().find(|(k, _)| k == "Location").map(|(_, v)| v.as_str());
    assert_eq!(loc_user, Some("/dashboard"), "user should redirect to /dashboard");
}

#[test]
fn demo_login_sets_expected_roles() {
    let router = mount_demo_login(
        Router::new(|_| Auth::login("anon", &[]))
            .get("/", "home", |_, _| nirdosha_rt::Response::html(200, "<html><body><h1>Home</h1></body></html>"))
    );
    let resp = router.dispatch(&login_request("user", "ok"));
    let cookie = cookie_from_set_cookie(&resp).expect("login should succeed");

    let authed = router.dispatch(&Request {
        method: "GET".into(),
        path: "/".into(),
        headers: HashMap::from([("cookie".into(), format!("nirdosha_session={}", cookie))]),
        body: String::new(),
    });
    assert!(authed.body.contains("Logout"), "authenticated session should show logout button; got: {}", authed.body);
}
