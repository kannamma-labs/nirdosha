//! Transport-agnostic route table over `.nir/realm.db`
//! (rfcs/0014-generative-build-console.md) — shared by two transports
//! that each translate their own request/response types at the edges
//! and call [`handle`] for everything else:
//!
//! - `realm_window.rs`: `wry`'s custom-protocol handler, the RFC's own
//!   resolved default for build mode ("no network port at all").
//! - `realm_server.rs`: a real `tiny_http` socket, the RFC's own
//!   documented fallback for headless/scripting/CI use, never the
//!   default.
//!
//! Read-only, foundation-slice scope only (no write endpoints, no live
//! updates, no filtering beyond a hard cap) — see that RFC's own Open
//! Questions for the eventual full build-mode editing surface.

use std::path::Path;

use rusqlite::Connection;
use serde::Serialize;

/// Every `nodes`/`edges` listing is hard-capped, never unbounded —
/// rfcs/0013's own "every expensive operation is bounded" principle,
/// applied here to an HTTP response instead of a graph traversal.
const MAX_ROWS: u32 = 2000;

/// The build-mode page: a live 3D graph over `.nir/realm.db`'s nodes
/// and edges (rfcs/0014's "Build mode — the interactive Realm graph").
/// Fetches `/api/nodes`/`/api/edges` itself once loaded — see
/// `realm_graph.html`'s own header comment for the rendering approach,
/// the WebGL-feature-detect 2D fallback, and the tooltip's
/// DOM-not-innerHTML XSS mitigation.
const BUILD_MODE_HTML: &str = include_str!("realm_graph.html");

/// Vendored per rfcs/0014's own rule ("`three.js`/`3d-force-graph` ship
/// vendored into the generated page, never loaded from a CDN") —
/// `crates/compiler/src/vendor/3d-force-graph.LICENSE` carries its MIT
/// license. This one file is genuinely self-contained: `3d-force-graph`
/// bundles `three.js` (WebGLRenderer, OrbitControls, etc.) internally
/// via its own UMD build, so nothing else needs vendoring alongside it.
const FORCE_GRAPH_JS: &str = include_str!("vendor/3d-force-graph.min.js");

/// A transport-neutral HTTP-shaped response. Each transport module
/// converts this to its own native response type at the edge.
pub struct ApiResponse {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

impl ApiResponse {
    fn html(body: &str) -> Self {
        ApiResponse { status: 200, content_type: "text/html; charset=utf-8", body: body.as_bytes().to_vec() }
    }

    fn javascript(body: &str) -> Self {
        ApiResponse { status: 200, content_type: "application/javascript; charset=utf-8", body: body.as_bytes().to_vec() }
    }

    fn json<T: Serialize>(value: &T) -> Self {
        match serde_json::to_vec(value) {
            Ok(body) => ApiResponse { status: 200, content_type: "application/json", body },
            Err(e) => ApiResponse::error(500, &format!("serializing response: {e}")),
        }
    }

    pub fn error(status: u16, message: &str) -> Self {
        let body = serde_json::json!({ "error": message }).to_string().into_bytes();
        ApiResponse { status, content_type: "application/json", body }
    }
}

/// Routes one request against a fresh connection opened on `root`.
/// `method` is compared case-sensitively against `"GET"` (the only verb
/// this foundation slice serves); `path` excludes the query string,
/// `query` is the raw, still percent-encoded `key=value&...` tail.
pub fn handle(root: &Path, method: &str, path: &str, query: &str) -> ApiResponse {
    if method != "GET" {
        return ApiResponse::error(405, "only GET is supported");
    }
    // Static assets never touch `.nir/realm.db` -- served before opening
    // a connection so a DB problem can never take the page/script down
    // with it (the page's own fetches to /api/* report that separately).
    match path {
        "/" => return ApiResponse::html(BUILD_MODE_HTML),
        "/assets/3d-force-graph.min.js" => return ApiResponse::javascript(FORCE_GRAPH_JS),
        _ => {}
    }
    let conn = match crate::realm::open(root) {
        Ok(conn) => conn,
        Err(e) => return ApiResponse::error(500, &e),
    };
    match path {
        "/api/nodes" => match list_nodes(&conn) {
            Ok(rows) => ApiResponse::json(&rows),
            Err(e) => ApiResponse::error(500, &e),
        },
        "/api/edges" => match list_edges(&conn) {
            Ok(rows) => ApiResponse::json(&rows),
            Err(e) => ApiResponse::error(500, &e),
        },
        "/api/impact" => match query_param(query, "target") {
            Some(target) => match crate::realm::impact(&conn, &target) {
                Ok(report) => ApiResponse::json(&report),
                Err(e) => ApiResponse::error(400, &e),
            },
            None => ApiResponse::error(400, "missing required query param `target`"),
        },
        "/api/ask" => match query_param(query, "q") {
            Some(q) => match crate::realm::ask(&conn, &q) {
                Ok(hits) => ApiResponse::json(&hits),
                Err(e) => ApiResponse::error(500, &e),
            },
            None => ApiResponse::error(400, "missing required query param `q`"),
        },
        _ => ApiResponse::error(404, "not found"),
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

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("nirdosha_realm_api_test_{name}_{}", std::process::id()));
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

    #[test]
    fn handle_serves_the_build_mode_graph_page_at_root() {
        let dir = scratch_dir("root");
        let resp = handle(&dir, "GET", "/", "");
        assert_eq!(resp.status, 200);
        let body = String::from_utf8_lossy(&resp.body);
        assert!(body.contains("Nirdosha Realm"));
        assert!(body.contains("/assets/3d-force-graph.min.js"), "page should load the vendored graph library");
    }

    /// The static-asset routes are served without ever opening
    /// `.nir/realm.db` -- exercised here against a directory that was
    /// never scaffolded at all, unlike every other test in this file.
    #[test]
    fn handle_serves_the_vendored_graph_library_without_touching_the_db() {
        let mut dir = std::env::temp_dir();
        dir.push(format!("nirdosha_realm_api_test_unscaffolded_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let resp = handle(&dir, "GET", "/assets/3d-force-graph.min.js", "");
        assert_eq!(resp.status, 200);
        assert_eq!(resp.content_type, "application/javascript; charset=utf-8");
        assert!(String::from_utf8_lossy(&resp.body).contains("ForceGraph3D"));
        assert!(!dir.join(".nir").exists(), "static assets must not scaffold .nir/");
    }

    #[test]
    fn handle_lists_nodes_after_a_sync() {
        let dir = scratch_dir("nodes");
        std::fs::write(dir.join("a.nir"), "fn add(a: i64, b: i64) -> i64 { return a + b }\n").unwrap();
        let conn = crate::realm::open(&dir).expect("open");
        crate::realm::sync(&conn, &dir, &[]).expect("sync");
        drop(conn);

        let resp = handle(&dir, "GET", "/api/nodes", "");
        assert_eq!(resp.status, 200);
        let nodes: Vec<serde_json::Value> = serde_json::from_slice(&resp.body).expect("valid JSON array");
        assert!(nodes.iter().any(|n| n["id"] == "code:fn:add"), "expected code:fn:add in {resp:?}", resp = String::from_utf8_lossy(&resp.body));
    }

    #[test]
    fn handle_rejects_non_get_methods() {
        let dir = scratch_dir("non_get");
        let resp = handle(&dir, "POST", "/api/nodes", "");
        assert_eq!(resp.status, 405);
    }

    #[test]
    fn handle_reports_a_missing_query_param_as_a_client_error() {
        let dir = scratch_dir("missing_param");
        let resp = handle(&dir, "GET", "/api/impact", "");
        assert_eq!(resp.status, 400);
    }

    #[test]
    fn handle_reports_unknown_paths_as_not_found() {
        let dir = scratch_dir("not_found");
        let resp = handle(&dir, "GET", "/api/nope", "");
        assert_eq!(resp.status, 404);
    }
}
