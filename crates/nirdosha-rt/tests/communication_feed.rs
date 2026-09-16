//! `communication_feed!` end to end: an append-only, newest-first feed
//! -- honest client-side polling via `<meta http-equiv="refresh">`, the
//! same mechanism `dashboard!` uses, not real server push (see the
//! module doc on `nirdosha_rt::feed` for why).

use nirdosha_rt::{Auth, Request, Response, Router};
use std::sync::Mutex;

nirdosha_rt::roles! {
    Member = "member";
}

#[derive(Clone, Default, serde::Serialize)]
struct Message {
    id: i64,
    author: String,
    body: String,
}

fn message_store() -> &'static Mutex<Vec<Message>> {
    static STORE: std::sync::OnceLock<Mutex<Vec<Message>>> = std::sync::OnceLock::new();
    STORE.get_or_init(|| Mutex::new(Vec::new()))
}

nirdosha_rt::communication_feed! {
    mount: mount_team_feed,
    entity: Message,
    store: message_store,
    path: "/feed",
    fields: [ author: String, body: String ],
    post_access: requires role "member",
    read_access: public,
    refresh_seconds: 5,
}

fn router() -> Router {
    mount_team_feed(Router::new(|req: &Request| {
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
fn communication_feed_full_session() {
    // 1. Anonymous can read the (empty) feed -- it's public.
    let empty = dispatch("GET", "/feed", None, "");
    assert_eq!(empty.status, 200);
    assert!(empty.body.contains("No messages yet"));
    assert!(empty.body.contains("http-equiv=\"refresh\" content=\"5\""), "must be honest polling, not a claim of push: {}", empty.body);

    // 2. Anonymous can't post.
    let denied = dispatch("POST", "/feed", None, "author=Eve&body=hi");
    assert_eq!(denied.status, 403);

    // 3. A member can post, and it shows up newest-first.
    let first = dispatch("POST", "/feed", Some("member"), "author=Alice&body=Hello");
    assert_eq!(first.status, 302);
    let second = dispatch("POST", "/feed", Some("member"), "author=Bob&body=Hi+back");
    assert_eq!(second.status, 302);

    let api = dispatch("GET", "/api/feed", None, "");
    assert_eq!(api.status, 200);
    let messages: serde_json::Value = serde_json::from_str(&api.body).unwrap();
    let messages = messages.as_array().unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["author"], "Bob", "newest message must come first");
    assert_eq!(messages[1]["author"], "Alice");

    let view = dispatch("GET", "/feed", None, "");
    let bob_idx = view.body.find("Bob").unwrap();
    let alice_idx = view.body.find("Alice").unwrap();
    assert!(bob_idx < alice_idx, "HTML view must also show newest first");
}

#[test]
fn openapi_lists_feed_routes_with_the_right_roles() {
    let resp = dispatch("GET", "/openapi.json", None, "");
    let doc: serde_json::Value = serde_json::from_str(&resp.body).unwrap();
    assert_eq!(doc["paths"]["/feed"]["post"]["security"][0]["nirdoshaRole"][0], "member");
    assert!(doc["paths"]["/api/feed"]["get"]["security"].is_null());
}
