//! Headless local HTTP fallback for Realm build mode
//! (rfcs/0014-generative-build-console.md). The RFC's own resolved
//! default transport is `realm_window.rs`'s `wry` custom-protocol
//! handler — "no network port at all" — because it closes CSRF/DNS-
//! rebinding/port-squatting risk classes a real socket can't avoid.
//! This module *is* that real socket: the RFC's own documented
//! fallback for headless/scripting/CI use (`nirdosha realm serve`),
//! never the default a user hits by opening build mode normally. Both
//! transports share one route table (`realm_api::handle`) and differ
//! only in how they translate their native request/response types at
//! the edge, plus the hardening this module's own real network
//! exposure requires and `realm_window.rs`'s doesn't (see
//! `has_browser_origin` below).
//!
//! `tiny_http` is already a workspace dependency
//! (`crates/compiler/Cargo.toml`, used elsewhere for `compiled-serve`-
//! adjacent work) — this adds no new dependency to the tree.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::thread;

use tiny_http::{Header, Response, Server};

use crate::realm_api::{self, ApiResponse};

pub struct ServerHandle {
    pub port: u16,
}

/// Starts the local API server on an OS-assigned loopback port, on a
/// background thread, and returns immediately with the bound port.
/// Runs for the life of the process — no shutdown API yet, matching
/// this increment's "prove the pipeline" scope, not the full session
/// lifecycle RFC 0014 describes (still an open question there).
pub fn serve(root: &Path) -> Result<ServerHandle, String> {
    let server = Server::http("127.0.0.1:0").map_err(|e| format!("binding local Realm server: {e}"))?;
    let port = server.server_addr().to_ip().map(|a| a.port()).ok_or_else(|| "local Realm server has no bound IP address".to_string())?;
    let root: PathBuf = root.to_path_buf();
    thread::Builder::new()
        .name("nirdosha-realm-server".to_string())
        .spawn(move || {
            for request in server.incoming_requests() {
                let response = respond(&root, &request);
                let _ = request.respond(response);
            }
        })
        .map_err(|e| format!("spawning the local Realm server thread: {e}"))?;
    Ok(ServerHandle { port })
}

/// rfcs/0014's own hardening list for this fallback mode: "`Origin`
/// header checking" -- this is a headless/scripting surface, never a
/// browser-facing one (the browser-facing surface is the wry custom-
/// protocol handler, which has no network origin to check at all), so
/// any request carrying an `Origin` header at all -- meaning some page
/// in an actual browser sent it, not a script or CLI -- is rejected.
/// Closes the same DNS-rebinding/CSRF class the RFC's "kill the network
/// port" decision closes for the primary surface.
fn has_browser_origin(request: &tiny_http::Request) -> bool {
    request.headers().iter().any(|h| h.field.as_str().as_str().eq_ignore_ascii_case("Origin"))
}

fn respond(root: &Path, request: &tiny_http::Request) -> Response<Cursor<Vec<u8>>> {
    if has_browser_origin(request) {
        return to_tiny_http(ApiResponse::error(403, "this local API does not accept browser-originated requests"));
    }
    let (path, query) = request.url().split_once('?').unwrap_or((request.url(), ""));
    let method = request.method().as_str();
    to_tiny_http(realm_api::handle(root, method, path, query))
}

fn to_tiny_http(resp: ApiResponse) -> Response<Cursor<Vec<u8>>> {
    let header = Header::from_bytes(&b"Content-Type"[..], resp.content_type.as_bytes()).expect("static header is always valid");
    Response::from_data(resp.body).with_status_code(tiny_http::StatusCode(resp.status)).with_header(header)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("nirdosha_realm_server_test_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    /// Real end-to-end: start the server, make a real HTTP request over
    /// loopback, parse the real JSON response — not just unit-testing
    /// the route function in isolation (that's `realm_api`'s own tests).
    #[test]
    fn serve_answers_api_nodes_over_a_real_http_request() {
        let dir = scratch_dir("nodes_http");
        std::fs::write(dir.join("a.nir"), "fn add(a: i64, b: i64) -> i64 { return a + b }\n").unwrap();
        let conn = crate::realm::open(&dir).expect("open");
        crate::realm::sync(&conn, &dir, &[]).expect("sync");
        drop(conn); // release the file lock before the server thread opens its own connection

        let handle = serve(&dir).expect("serve");
        let url = format!("http://127.0.0.1:{}/api/nodes", handle.port);
        let body = reqwest::blocking::get(&url).expect("request should succeed").text().expect("response body");
        let nodes: Vec<serde_json::Value> = serde_json::from_str(&body).expect("valid JSON array");
        assert!(nodes.iter().any(|n| n["id"] == "code:fn:add"), "expected code:fn:add in {body}");
    }

    #[test]
    fn serve_answers_the_placeholder_page_at_root() {
        let dir = scratch_dir("root_http");
        let conn = crate::realm::open(&dir).expect("open");
        drop(conn);
        let handle = serve(&dir).expect("serve");
        let body = reqwest::blocking::get(format!("http://127.0.0.1:{}/", handle.port)).expect("request should succeed").text().expect("response body");
        assert!(body.contains("Nirdosha Realm"));
    }

    #[test]
    fn serve_answers_api_impact_for_a_linked_requirement() {
        let dir = scratch_dir("impact_http");
        std::fs::write(dir.join("a.nir"), "fn transfer_funds(amount: i64) -> i64 { return amount }\n").unwrap();
        let conn = crate::realm::open(&dir).expect("open");
        crate::realm::sync(&conn, &dir, &[]).expect("sync");
        crate::realm::link(&conn, "R17", "fn:transfer_funds").expect("link");
        drop(conn);

        let handle = serve(&dir).expect("serve");
        let url = format!("http://127.0.0.1:{}/api/impact?target=R17", handle.port);
        let body = reqwest::blocking::get(&url).expect("request should succeed").text().expect("response body");
        let report: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
        assert!(body.contains("code:fn:transfer_funds"), "expected the linked CodeUnit in {report}");
    }

    /// The RFC's own hardening requirement for this fallback mode: a
    /// request carrying a real `Origin` header (what an actual browser
    /// page sends, never a CLI/script) is rejected outright.
    #[test]
    fn serve_rejects_a_request_carrying_a_browser_origin_header() {
        let dir = scratch_dir("origin_http");
        let conn = crate::realm::open(&dir).expect("open");
        drop(conn);
        let handle = serve(&dir).expect("serve");
        let client = reqwest::blocking::Client::new();
        let resp = client
            .get(format!("http://127.0.0.1:{}/api/nodes", handle.port))
            .header("Origin", "http://evil.example")
            .send()
            .expect("request should succeed");
        assert_eq!(resp.status().as_u16(), 403);
    }

    #[test]
    fn serve_reports_a_missing_query_param_as_a_client_error() {
        let dir = scratch_dir("missing_param_http");
        let conn = crate::realm::open(&dir).expect("open");
        drop(conn);
        let handle = serve(&dir).expect("serve");
        let resp = reqwest::blocking::get(format!("http://127.0.0.1:{}/api/impact", handle.port)).expect("request should succeed");
        assert_eq!(resp.status().as_u16(), 400);
    }
}
