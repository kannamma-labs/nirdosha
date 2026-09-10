//! Local HTTP surface for Realm (rfcs/0014-generative-build-console.md)
//! — a read-only JSON API over `.nir/realm.db`, plus a small placeholder
//! page, for the native window shell (`realm_window.rs`) to load. This
//! is deliberately the *foundation* slice only: proving the local-
//! server → window pipeline works end to end, not RFC 0014's full
//! build-mode editing surface (no write endpoints, no live updates, no
//! filtering beyond a hard cap) — see that RFC's own Open Questions for
//! what the API's eventual real shape still needs to answer.
//!
//! `tiny_http` is already a workspace dependency
//! (`crates/compiler/Cargo.toml`, used elsewhere for `compiled-serve`-
//! adjacent work) — this adds no new dependency to the tree.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::thread;

use rusqlite::Connection;
use serde::Serialize;
use tiny_http::{Header, Method, Response, Server};

/// Every `nodes`/`edges` listing is hard-capped, never unbounded —
/// rfcs/0013's own "every expensive operation is bounded" principle,
/// applied here to an HTTP response instead of a graph traversal.
const MAX_ROWS: u32 = 2000;

const PLACEHOLDER_HTML: &str = r#"<!doctype html>
<html>
<head>
<meta charset="utf-8">
<title>Nirdosha Realm</title>
<style>
  body { font-family: ui-monospace, monospace; background: #0b0b12; color: #e8e8f0; padding: 2rem; }
  h1 { color: #ef9f30; }
  pre { background: #15151f; padding: 1rem; border-radius: 6px; overflow: auto; max-height: 70vh; }
</style>
</head>
<body>
<h1>Nirdosha Realm</h1>
<p id="status">connecting…</p>
<pre id="nodes"></pre>
<script>
fetch('/api/nodes').then(r => r.json()).then(data => {
  document.getElementById('status').textContent = data.length + ' node(s) in .nir/realm.db';
  document.getElementById('nodes').textContent = JSON.stringify(data, null, 2);
}).catch(e => {
  document.getElementById('status').textContent = 'error: ' + e;
});
</script>
</body>
</html>
"#;

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
                // A fresh connection per request — `rusqlite::Connection`
                // isn't `Sync`, and a request-per-connection is cheap for
                // a local, single-user, low-QPS API like this one, not
                // the pooled/admission-controlled model RFC 0011 built
                // for a *compiled program's* own `db_connect` — this is
                // host-tooling, same "no cross-workspace pooling" reason
                // `hi`'s own LLM client already gives (RFC 0012).
                let response = match crate::realm::open(&root) {
                    Ok(conn) => route(&request, &conn),
                    Err(e) => error_response(500, &e),
                };
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

fn route(request: &tiny_http::Request, conn: &Connection) -> Response<Cursor<Vec<u8>>> {
    if has_browser_origin(request) {
        return error_response(403, "this local API does not accept browser-originated requests");
    }
    let (path, query) = request.url().split_once('?').unwrap_or((request.url(), ""));
    match (request.method(), path) {
        (Method::Get, "/") => html_response(PLACEHOLDER_HTML),
        (Method::Get, "/api/nodes") => match list_nodes(conn) {
            Ok(rows) => json_response(&rows),
            Err(e) => error_response(500, &e),
        },
        (Method::Get, "/api/edges") => match list_edges(conn) {
            Ok(rows) => json_response(&rows),
            Err(e) => error_response(500, &e),
        },
        (Method::Get, "/api/impact") => match query_param(query, "target") {
            Some(target) => match crate::realm::impact(conn, &target) {
                Ok(report) => json_response(&report),
                Err(e) => error_response(400, &e),
            },
            None => error_response(400, "missing required query param `target`"),
        },
        (Method::Get, "/api/ask") => match query_param(query, "q") {
            Some(q) => match crate::realm::ask(conn, &q) {
                Ok(hits) => json_response(&hits),
                Err(e) => error_response(500, &e),
            },
            None => error_response(400, "missing required query param `q`"),
        },
        _ => error_response(404, "not found"),
    }
}

/// A minimal `?key=value&key2=value2` extractor with percent-decoding —
/// deliberately hand-rolled rather than a new dependency (`url`'s
/// `form_urlencoded` would do this, but isn't a direct dependency here
/// and pulling it in for a ~15-line utility isn't worth it, the same
/// "no dependency this repo doesn't already need" posture the rest of
/// this project holds itself to).
fn query_param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        if k == key {
            Some(percent_decode(v))
        } else {
            None
        }
    })
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                if let Ok(byte) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    out.push(byte);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn html_response(body: &str) -> Response<Cursor<Vec<u8>>> {
    let header = Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]).expect("static header is always valid");
    Response::from_string(body.to_string()).with_header(header)
}

fn json_response<T: Serialize>(value: &T) -> Response<Cursor<Vec<u8>>> {
    match serde_json::to_string(value) {
        Ok(body) => {
            let header = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).expect("static header is always valid");
            Response::from_string(body).with_header(header)
        }
        Err(e) => error_response(500, &format!("serializing response: {e}")),
    }
}

fn error_response(code: u32, message: &str) -> Response<Cursor<Vec<u8>>> {
    let body = serde_json::json!({ "error": message }).to_string();
    let header = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).expect("static header is always valid");
    Response::from_string(body).with_status_code(tiny_http::StatusCode(code as u16)).with_header(header)
}

#[derive(Serialize)]
struct NodeRow {
    id: String,
    kind: String,
    title: Option<String>,
    status: Option<String>,
    content_hash: Option<String>,
    source_ref: Option<String>,
    line: Option<i64>,
    col: Option<i64>,
}

fn list_nodes(conn: &Connection) -> Result<Vec<NodeRow>, String> {
    let mut stmt = conn
        .prepare("SELECT id, kind, title, status, content_hash, source_ref, line, col FROM nodes LIMIT ?1")
        .map_err(|e| format!("preparing node listing: {e}"))?;
    let rows = stmt
        .query_map([MAX_ROWS], |r| {
            Ok(NodeRow { id: r.get(0)?, kind: r.get(1)?, title: r.get(2)?, status: r.get(3)?, content_hash: r.get(4)?, source_ref: r.get(5)?, line: r.get(6)?, col: r.get(7)? })
        })
        .map_err(|e| format!("listing nodes: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("reading node row: {e}"))?;
    Ok(rows)
}

#[derive(Serialize)]
struct EdgeRow {
    src: String,
    dst: String,
    kind: String,
    flag: Option<String>,
    flag_reason: Option<String>,
}

fn list_edges(conn: &Connection) -> Result<Vec<EdgeRow>, String> {
    let mut stmt = conn.prepare("SELECT src, dst, kind, flag, flag_reason FROM edges LIMIT ?1").map_err(|e| format!("preparing edge listing: {e}"))?;
    let rows = stmt
        .query_map([MAX_ROWS], |r| Ok(EdgeRow { src: r.get(0)?, dst: r.get(1)?, kind: r.get(2)?, flag: r.get(3)?, flag_reason: r.get(4)? }))
        .map_err(|e| format!("listing edges: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("reading edge row: {e}"))?;
    Ok(rows)
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

    #[test]
    fn percent_decode_handles_plus_and_hex_escapes() {
        assert_eq!(percent_decode("hello+world"), "hello world");
        assert_eq!(percent_decode("a%20b%3F"), "a b?");
        assert_eq!(percent_decode("plain"), "plain");
    }

    #[test]
    fn query_param_finds_the_named_key_among_several() {
        assert_eq!(query_param("a=1&b=hello+world&c=3", "b"), Some("hello world".to_string()));
        assert_eq!(query_param("a=1", "missing"), None);
        assert_eq!(query_param("", "a"), None);
    }

    /// Real end-to-end: start the server, make a real HTTP request over
    /// loopback, parse the real JSON response — not just unit-testing
    /// the route function in isolation.
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
