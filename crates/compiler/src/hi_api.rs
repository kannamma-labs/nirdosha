//! Transport-agnostic route table over `.nir/hi.db`
//! (rfcs/0014-generative-build-console.md) — shared by two transports
//! that each translate their own request/response types at the edges
//! and call [`handle`] for everything else:
//!
//! - `hi_window.rs`: `wry`'s custom-protocol handler, the RFC's own
//!   resolved default for build mode ("no network port at all").
//! - `hi_server.rs`: a real `tiny_http` socket, the RFC's own
//!   documented fallback for headless/scripting/CI use, never the
//!   default.
//!
//! Read-only, foundation-slice scope only (no write endpoints, no live
//! updates, no filtering beyond a hard cap) — see that RFC's own Open
//! Questions for the eventual full build-mode editing surface.

use std::path::Path;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use rusqlite::Connection;
use serde::Serialize;

/// Every `nodes`/`edges` listing is hard-capped, never unbounded —
/// rfcs/0013's own "every expensive operation is bounded" principle,
/// applied here to an HTTP response instead of a graph traversal.
const MAX_ROWS: u32 = 2000;

/// The build-mode page: a live 3D graph over `.nir/hi.db`'s nodes
/// and edges (rfcs/0014's "Build mode — the interactive hi graph").
/// Fetches `/api/nodes`/`/api/edges` itself once loaded — see
/// `hi_graph.html`'s own header comment for the rendering approach,
/// the WebGL-feature-detect 2D fallback, and the tooltip's
/// DOM-not-innerHTML XSS mitigation.
const BUILD_MODE_HTML: &str = include_str!("hi_graph.html");

/// Same brand mark `ui_gen.rs`'s own `logo_data_uri()` bakes into every
/// compiled program's generated UI (`nirdosha-app-bar-icon.png`,
/// already vendored alongside this file) — reused as-is here so the
/// splash/header in `hi_graph.html` matches `hi`'s own branding
/// exactly, not a second logo asset to keep in sync. Substituted into
/// `BUILD_MODE_HTML`'s `__NIRDOSHA_LOGO__` placeholder at request time.
fn logo_data_uri() -> String {
    const LOGO_PNG: &[u8] = include_bytes!("nirdosha-app-bar-icon.png");
    format!("data:image/png;base64,{}", BASE64_STANDARD.encode(LOGO_PNG))
}

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

/// The write surface rfcs/0014's Build/Generate/Publish modes need --
/// every one of these mutates `.nir/hi.db` (or, for `/api/generate`/
/// `/api/publish`, the filesystem under `.nir/generated/` too) and so
/// is POST-only, never GET: a mutating GET would be triggerable by a
/// plain `<img src>`/link from any page in `hi_server.rs`'s headless
/// fallback mode, without ever carrying the `Origin` header that
/// fallback's own `has_browser_origin` check relies on to reject
/// browser-originated requests. POST closes that gap (modern browsers
/// do send `Origin` on a cross-origin POST) on top of `hi_window.rs`'s
/// transport already having no network origin to attack at all.
const MUTATING_PATHS: &[&str] = &["/api/prompt", "/api/confirm", "/api/delete", "/api/edit", "/api/attach", "/api/waive", "/api/unwaive", "/api/generate", "/api/publish"];

/// Routes one request against a fresh connection opened on `root`.
/// `path` excludes the query string; `query` is the raw, still
/// percent-encoded `key=value&...` tail; `body` is the raw request
/// body, form-encoded the same way `query` is (`body_param` decodes it
/// with the exact same `query_param`/`percent_decode` this module
/// already uses for `?key=value`) -- one encoding, read from whichever
/// side of the request a given route needs it.
pub fn handle(root: &Path, method: &str, path: &str, query: &str, body: &[u8]) -> ApiResponse {
    let is_mutating = MUTATING_PATHS.contains(&path);
    if is_mutating {
        if method != "POST" {
            return ApiResponse::error(405, "this route only accepts POST");
        }
    } else if method != "GET" {
        return ApiResponse::error(405, "only GET is supported");
    }
    // Static assets never touch `.nir/hi.db` -- served before opening
    // a connection so a DB problem can never take the page/script down
    // with it (the page's own fetches to /api/* report that separately).
    match path {
        "/" => return ApiResponse::html(&BUILD_MODE_HTML.replace("__NIRDOSHA_LOGO__", &logo_data_uri())),
        "/assets/3d-force-graph.min.js" => return ApiResponse::javascript(FORCE_GRAPH_JS),
        _ => {}
    }
    let conn = match crate::hi_graph::open(root) {
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
            Some(target) => match crate::hi_graph::impact(&conn, &target) {
                Ok(report) => ApiResponse::json(&report),
                Err(e) => ApiResponse::error(400, &e),
            },
            None => ApiResponse::error(400, "missing required query param `target`"),
        },
        "/api/ask" => match query_param(query, "q") {
            Some(q) => match crate::hi_graph::ask(&conn, &q) {
                Ok(hits) => ApiResponse::json(&hits),
                Err(e) => ApiResponse::error(500, &e),
            },
            None => ApiResponse::error(400, "missing required query param `q`"),
        },
        "/api/prompt" => handle_prompt(root, &conn, body),
        "/api/confirm" => handle_confirm(&conn, body),
        "/api/delete" => handle_delete(&conn, body),
        "/api/edit" => handle_edit(&conn, body),
        "/api/attach" => handle_attach(&conn, body),
        "/api/waive" => handle_waive(&conn, body),
        "/api/unwaive" => handle_unwaive(&conn, body),
        "/api/generate" => handle_generate(root, &conn, body),
        "/api/publish" => handle_publish(root),
        _ => ApiResponse::error(404, "not found"),
    }
}

fn body_param(body: &[u8], key: &str) -> Option<String> {
    std::str::from_utf8(body).ok().and_then(|s| query_param(s, key))
}

fn ok_response() -> ApiResponse {
    ApiResponse::json(&serde_json::json!({ "ok": true }))
}

fn require_llm_client() -> Result<crate::hi_llm::LlmClient, String> {
    let activation = crate::hi_llm::resolve_activation(&|k| std::env::var(k).ok())?;
    Ok(crate::hi_llm::LlmClient::new(activation))
}

/// Prompt mode's own write path (rfcs/0014's "1. Prompt mode"): calls
/// the LLM once (`hi_llm::populate_candidates`), then turns every
/// candidate it proposed into a real `hi_graph::add_candidate` node --
/// each lands `confirmed = 0` by construction, so nothing this route
/// adds is reachable by Generate mode until a human reviews it in Build
/// mode. `depends_on` edges are only ever recorded between two
/// candidates from the *same* population call (looked up by name in
/// this response's own batch) -- resolving a dependency against a
/// pre-existing node from an earlier call is real, disclosed follow-on
/// work this v1 slice doesn't attempt.
fn handle_prompt(root: &Path, conn: &Connection, body: &[u8]) -> ApiResponse {
    let _ = root;
    let Some(text) = body_param(body, "text") else {
        return ApiResponse::error(400, "missing required body param `text`");
    };
    let client = match require_llm_client() {
        Ok(c) => c,
        Err(e) => return ApiResponse::error(500, &e),
    };
    let candidates = match crate::hi_llm::populate_candidates(&client, &text) {
        Ok(c) => c,
        Err(e) => return ApiResponse::error(502, &e),
    };
    let mut ids = Vec::with_capacity(candidates.len());
    for c in &candidates {
        match crate::hi_graph::add_candidate(conn, &c.kind, &c.name, &c.driving_text, "llm-prompt-mode") {
            Ok(id) => ids.push(id),
            Err(e) => return ApiResponse::error(500, &e),
        }
    }
    let name_to_id: std::collections::HashMap<&str, &str> = candidates.iter().zip(ids.iter()).map(|(c, id)| (c.name.as_str(), id.as_str())).collect();
    let mut edges_added = 0u32;
    for c in &candidates {
        for dep in &c.depends_on {
            if let Some(dst) = name_to_id.get(dep.as_str()) {
                let src = &name_to_id[c.name.as_str()];
                if crate::hi_graph::add_relation(conn, src, dst).is_ok() {
                    edges_added += 1;
                }
            }
        }
    }
    ApiResponse::json(&serde_json::json!({ "ok": true, "nodes_added": ids.len(), "edges_added": edges_added, "nodes": ids }))
}

fn handle_confirm(conn: &Connection, body: &[u8]) -> ApiResponse {
    let Some(node) = body_param(body, "node") else {
        return ApiResponse::error(400, "missing required body param `node`");
    };
    match crate::hi_graph::confirm_node(conn, &node) {
        Ok(()) => ok_response(),
        Err(e) => ApiResponse::error(400, &e),
    }
}

fn handle_delete(conn: &Connection, body: &[u8]) -> ApiResponse {
    let Some(node) = body_param(body, "node") else {
        return ApiResponse::error(400, "missing required body param `node`");
    };
    match crate::hi_graph::delete_node(conn, &node) {
        Ok(()) => ok_response(),
        Err(e) => ApiResponse::error(400, &e),
    }
}

fn handle_edit(conn: &Connection, body: &[u8]) -> ApiResponse {
    let (Some(node), Some(text)) = (body_param(body, "node"), body_param(body, "text")) else {
        return ApiResponse::error(400, "missing required body params `node`/`text`");
    };
    match crate::hi_graph::edit_driving_text(conn, &node, &text) {
        Ok(()) => ok_response(),
        Err(e) => ApiResponse::error(400, &e),
    }
}

fn handle_attach(conn: &Connection, body: &[u8]) -> ApiResponse {
    let (Some(node), Some(attr)) = (body_param(body, "node"), body_param(body, "attr")) else {
        return ApiResponse::error(400, "missing required body params `node`/`attr`");
    };
    match crate::hi_graph::attach_attribute(conn, &node, &attr) {
        Ok(()) => ok_response(),
        Err(e) => ApiResponse::error(400, &e),
    }
}

fn handle_waive(conn: &Connection, body: &[u8]) -> ApiResponse {
    let Some(node) = body_param(body, "node") else {
        return ApiResponse::error(400, "missing required body param `node`");
    };
    let reason = body_param(body, "reason").unwrap_or_default();
    match crate::hi_graph::waive_node(conn, &node, &reason) {
        Ok(()) => ok_response(),
        Err(e) => ApiResponse::error(400, &e),
    }
}

fn handle_unwaive(conn: &Connection, body: &[u8]) -> ApiResponse {
    let Some(node) = body_param(body, "node") else {
        return ApiResponse::error(400, "missing required body param `node`");
    };
    match crate::hi_graph::unwaive_node(conn, &node) {
        Ok(()) => ok_response(),
        Err(e) => ApiResponse::error(400, &e),
    }
}

/// Generate mode (rfcs/0014's "3. Generate mode"): sends every
/// confirmed, in-scope candidate (`hi_graph::confirmed_units` -- the
/// *whole* confirmed set, not just newly-eligible units, since this v1
/// slice regenerates one combined file each time -- see
/// `hi_llm::generate_program`'s own doc comment) to the LLM with RFC
/// 0012's bounded self-repair discipline, folds the result back into
/// the graph via an ordinary `sync` (which upserts each unit's real
/// `content_hash` under the exact id it already had as a candidate),
/// then locks every unit the generated program actually declared.
/// `target` (an optional body param) is accepted for
/// `hi_graph::confirmed_units`'s own sake but unused by this v1's
/// console/UI, which always regenerates everything confirmed.
fn handle_generate(root: &Path, conn: &Connection, body: &[u8]) -> ApiResponse {
    let target = body_param(body, "target");
    let units = match crate::hi_graph::confirmed_units(conn, target.as_deref()) {
        Ok(u) => u,
        Err(e) => return ApiResponse::error(500, &e),
    };
    if units.is_empty() {
        return ApiResponse::error(400, "nothing confirmed to generate -- `:confirm <node>` at least one candidate first");
    }
    let client = match require_llm_client() {
        Ok(c) => c,
        Err(e) => return ApiResponse::error(500, &e),
    };
    let mut log_lines: Vec<String> = Vec::new();
    let path = match crate::hi_llm::generate_program(root, &client, &units, &mut |line| log_lines.push(line.to_string())) {
        Ok(p) => p,
        Err(e) => return ApiResponse::json(&serde_json::json!({ "ok": false, "error": e, "log": log_lines })),
    };
    if let Err(e) = crate::hi_graph::sync(conn, root, &[path.display().to_string()]) {
        return ApiResponse::json(&serde_json::json!({ "ok": false, "error": format!("generated {} but sync failed: {e}", path.display()), "log": log_lines }));
    }
    let ids: Vec<String> = units.iter().map(|u| u.id.clone()).collect();
    let locked = match crate::hi_graph::lock_units_after_sync(conn, &ids) {
        Ok(l) => l,
        Err(e) => return ApiResponse::json(&serde_json::json!({ "ok": false, "error": e, "log": log_lines })),
    };
    let not_declared: Vec<&String> = ids.iter().filter(|id| !locked.contains(id)).collect();
    ApiResponse::json(&serde_json::json!({ "ok": true, "path": path.display().to_string(), "locked": locked, "not_declared": not_declared, "log": log_lines }))
}

/// Publish mode (rfcs/0014's "4. Publish mode"), scoped down hard: RFC
/// 0014's own `deployment_provider` abstraction and credential storage
/// are explicitly undecided ("a real decision this RFC doesn't make" --
/// see its Open Questions). This v1 slice publishes to *local disk*
/// only -- one real whole-program build (typecheck, ownership-check,
/// codegen) of `hi_llm::generated_source_path`, the same combined file
/// every Generate pass writes, producing one runnable binary under
/// `.nir/generated/`. No deploy, no credentials, no provider config --
/// "publish" here means "produce the final compiled artifact this
/// project's confirmed graph currently describes," the honest subset of
/// the RFC's own larger, undesigned ambition.
fn handle_publish(root: &Path) -> ApiResponse {
    let source_path = crate::hi_llm::generated_source_path(root);
    if !source_path.exists() {
        return ApiResponse::error(400, "nothing generated yet -- run :generate first");
    }
    let result: Result<std::path::PathBuf, String> = (|| {
        let path_str = source_path.to_str().ok_or_else(|| format!("generated source path {} is not valid UTF-8", source_path.display()))?;
        let (program, _src): (crate::ast::Program, String) = crate::loader::load_program(path_str)?;
        if let Err(errors) = crate::typeck::typecheck(&program) {
            return Err(errors.iter().map(|e| format!("type error: {e}")).collect::<Vec<_>>().join("\n"));
        }
        if let Err(errors) = crate::ownership::check_ownership(&program) {
            return Err(errors.iter().map(|e| format!("ownership error: {e}")).collect::<Vec<_>>().join("\n"));
        }
        let smt_report = crate::smt::analyze(&program);
        let out_path = crate::hi_graph::hi_dir(root).join("generated").join("hi_build");
        crate::codegen::build(&program, &smt_report, &out_path, crate::codegen::OptLevel::O2)?;
        Ok(out_path)
    })();
    match result {
        Ok(out_path) => ApiResponse::json(&serde_json::json!({ "ok": true, "binary": out_path.display().to_string() })),
        Err(e) => ApiResponse::json(&serde_json::json!({ "ok": false, "error": e })),
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
    /// rfcs/0014's Build/Generate/Publish state -- see
    /// `hi_graph.rs`'s own migration comment for what each column
    /// means. `driving_text`/`created_by` are `None` for anything
    /// `hi sync` found in real code; `confirmed`/`locked`/`waived` are
    /// plain `0`/`1` (SQLite has no real boolean type) rather than
    /// `bool`, so a client reading raw JSON sees the same shape the
    /// database itself stores.
    driving_text: Option<String>,
    created_by: Option<String>,
    confirmed: i64,
    locked: i64,
    waived: i64,
    waive_reason: Option<String>,
    attributes: Option<String>,
}

fn list_nodes(conn: &Connection) -> Result<Vec<NodeRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, kind, title, status, content_hash, source_ref, line, col, driving_text, created_by, confirmed, locked, waived, waive_reason, attributes FROM nodes LIMIT ?1",
        )
        .map_err(|e| format!("preparing node listing: {e}"))?;
    let rows = stmt
        .query_map([MAX_ROWS], |r| {
            Ok(NodeRow {
                id: r.get(0)?,
                kind: r.get(1)?,
                title: r.get(2)?,
                status: r.get(3)?,
                content_hash: r.get(4)?,
                source_ref: r.get(5)?,
                line: r.get(6)?,
                col: r.get(7)?,
                driving_text: r.get(8)?,
                created_by: r.get(9)?,
                confirmed: r.get(10)?,
                locked: r.get(11)?,
                waived: r.get(12)?,
                waive_reason: r.get(13)?,
                attributes: r.get(14)?,
            })
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
        path.push(format!("nirdosha_hi_api_test_{name}_{}", std::process::id()));
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
        let resp = handle(&dir, "GET", "/", "", b"");
        assert_eq!(resp.status, 200);
        let body = String::from_utf8_lossy(&resp.body);
        assert!(body.contains("Nirdosha Hi"));
        assert!(body.contains("/assets/3d-force-graph.min.js"), "page should load the vendored graph library");
        assert!(!body.contains("__NIRDOSHA_LOGO__"), "the logo placeholder must be substituted, not leaked verbatim");
        assert!(body.contains("data:image/png;base64,"), "the brand logo should be inlined as a data: URI");
        assert!(body.contains("id=\"console-input\""), "build mode should have a bottom text-entry console, matching hi's own front ends");
        assert!(body.contains("id=\"splash\""), "build mode should open with the same logo splash hi's other front ends show");
    }

    /// The static-asset routes are served without ever opening
    /// `.nir/hi.db` -- exercised here against a directory that was
    /// never scaffolded at all, unlike every other test in this file.
    #[test]
    fn handle_serves_the_vendored_graph_library_without_touching_the_db() {
        let mut dir = std::env::temp_dir();
        dir.push(format!("nirdosha_hi_api_test_unscaffolded_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let resp = handle(&dir, "GET", "/assets/3d-force-graph.min.js", "", b"");
        assert_eq!(resp.status, 200);
        assert_eq!(resp.content_type, "application/javascript; charset=utf-8");
        assert!(String::from_utf8_lossy(&resp.body).contains("ForceGraph3D"));
        assert!(!dir.join(".nir").exists(), "static assets must not scaffold .nir/");
    }

    #[test]
    fn handle_lists_nodes_after_a_sync() {
        let dir = scratch_dir("nodes");
        std::fs::write(dir.join("a.nir"), "fn add(a: i64, b: i64) -> i64 { return a + b }\n").unwrap();
        let conn = crate::hi_graph::open(&dir).expect("open");
        crate::hi_graph::sync(&conn, &dir, &[]).expect("sync");
        drop(conn);

        let resp = handle(&dir, "GET", "/api/nodes", "", b"");
        assert_eq!(resp.status, 200);
        let nodes: Vec<serde_json::Value> = serde_json::from_slice(&resp.body).expect("valid JSON array");
        assert!(nodes.iter().any(|n| n["id"] == "code:fn:add"), "expected code:fn:add in {resp:?}", resp = String::from_utf8_lossy(&resp.body));
    }

    #[test]
    fn handle_rejects_non_get_methods() {
        let dir = scratch_dir("non_get");
        let resp = handle(&dir, "POST", "/api/nodes", "", b"");
        assert_eq!(resp.status, 405);
    }

    #[test]
    fn handle_reports_a_missing_query_param_as_a_client_error() {
        let dir = scratch_dir("missing_param");
        let resp = handle(&dir, "GET", "/api/impact", "", b"");
        assert_eq!(resp.status, 400);
    }

    #[test]
    fn handle_reports_unknown_paths_as_not_found() {
        let dir = scratch_dir("not_found");
        let resp = handle(&dir, "GET", "/api/nope", "", b"");
        assert_eq!(resp.status, 404);
    }

    #[test]
    fn mutating_routes_reject_get() {
        let dir = scratch_dir("mutating_rejects_get");
        for path in MUTATING_PATHS {
            let resp = handle(&dir, "GET", path, "", b"");
            assert_eq!(resp.status, 405, "{path} should reject GET");
        }
    }

    #[test]
    fn confirm_delete_edit_attach_waive_round_trip_over_http() {
        let dir = scratch_dir("build_mode_round_trip");
        let conn = crate::hi_graph::open(&dir).expect("open");
        let id = crate::hi_graph::add_candidate(&conn, "fn", "transfer_funds", "moves money", "llm-prompt-mode").expect("add_candidate");
        drop(conn);

        let resp = handle(&dir, "POST", "/api/confirm", "", format!("node={id}").as_bytes());
        assert_eq!(resp.status, 200, "confirm should succeed: {}", String::from_utf8_lossy(&resp.body));

        let resp = handle(&dir, "POST", "/api/attach", "", format!("node={id}&attr=requires(role%3A+admin)").as_bytes());
        assert_eq!(resp.status, 200, "attach should succeed: {}", String::from_utf8_lossy(&resp.body));

        let resp = handle(&dir, "POST", "/api/edit", "", format!("node={id}&text=moves+money+between+two+accounts").as_bytes());
        assert_eq!(resp.status, 200, "edit should succeed: {}", String::from_utf8_lossy(&resp.body));

        let resp = handle(&dir, "GET", "/api/nodes", "", b"");
        let nodes: Vec<serde_json::Value> = serde_json::from_slice(&resp.body).expect("valid JSON array");
        let node = nodes.iter().find(|n| n["id"] == id).expect("node should be listed");
        assert_eq!(node["confirmed"], 1);
        assert_eq!(node["driving_text"], "moves money between two accounts");
        assert_eq!(node["attributes"], "requires(role: admin)");

        let resp = handle(&dir, "POST", "/api/waive", "", b"");
        assert_eq!(resp.status, 400, "waive with no node/reason should be rejected: {}", String::from_utf8_lossy(&resp.body));
        let resp = handle(&dir, "POST", "/api/waive", "", format!("node={id}&reason=not+needed").as_bytes());
        assert_eq!(resp.status, 200, "waive should succeed: {}", String::from_utf8_lossy(&resp.body));

        let resp = handle(&dir, "POST", "/api/unwaive", "", format!("node={id}").as_bytes());
        assert_eq!(resp.status, 200);

        let resp = handle(&dir, "POST", "/api/delete", "", format!("node={id}").as_bytes());
        assert_eq!(resp.status, 200);
        let resp = handle(&dir, "GET", "/api/nodes", "", b"");
        let nodes: Vec<serde_json::Value> = serde_json::from_slice(&resp.body).expect("valid JSON array");
        assert!(!nodes.iter().any(|n| n["id"] == id), "deleted node should no longer be listed");
    }

    #[test]
    fn generate_refuses_when_nothing_is_confirmed() {
        let dir = scratch_dir("generate_nothing_confirmed");
        crate::hi_graph::open(&dir).expect("open"); // scaffold .nir/ so `hi_graph::open` inside handle succeeds
        let resp = handle(&dir, "POST", "/api/generate", "", b"");
        assert_eq!(resp.status, 400);
        assert!(String::from_utf8_lossy(&resp.body).contains("nothing confirmed"));
    }

    #[test]
    fn publish_refuses_before_anything_has_been_generated() {
        let dir = scratch_dir("publish_nothing_generated");
        crate::hi_graph::open(&dir).expect("open");
        let resp = handle(&dir, "POST", "/api/publish", "", b"");
        assert_eq!(resp.status, 400);
        assert!(String::from_utf8_lossy(&resp.body).contains("nothing generated"));
    }
}
