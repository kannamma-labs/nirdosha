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
/// license. `3d-force-graph` bundles `three.js` (WebGLRenderer,
/// OrbitControls, etc.) internally via its own UMD build for everything
/// it does on its own -- but github #57's per-node custom 3D shapes
/// (`run3D`'s `nodeThreeObject`) need a real `THREE` constructor
/// reachable from *our own* code too, which the bundle's internal copy
/// never exposes (see `THREE_JS` just below). "Nothing else needs
/// vendoring alongside it" is no longer true; see that const instead.
const FORCE_GRAPH_JS: &str = include_str!("vendor/3d-force-graph.min.js");

/// A real `THREE` global, vendored (not CDN-loaded, same rfcs/0014
/// rule as `FORCE_GRAPH_JS` above) and served *before*
/// `FORCE_GRAPH_JS` in `hi_graph.html` so that bundle's own
/// `typeof window.THREE !== 'undefined' ? window.THREE : <internal
/// copy>` check (confirmed against its source -- see `hi_graph.html`'s
/// `colorFor` comment) picks this one up and uses it for everything,
/// rather than its internal, unexported copy -- letting this same
/// `THREE` also be used directly by `run3D`'s own `nodeThreeObject`
/// accessor. `crates/compiler/src/vendor/three.LICENSE` carries its
/// MIT license; that vendored file's own header comment has the exact
/// `npm install`/`esbuild` invocation this was built with -- modern
/// `three.js` only publishes ESM builds, no classic global/window
/// script, so unlike `FORCE_GRAPH_JS` this isn't a ready-made download.
const THREE_JS: &str = include_str!("vendor/three.min.js");

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
const MUTATING_PATHS: &[&str] = &["/api/prompt", "/api/confirm", "/api/delete", "/api/edit", "/api/attach", "/api/waive", "/api/unwaive", "/api/packs/install", "/api/generate", "/api/publish", "/api/preview/start", "/api/preview/stop"];

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
        "/assets/three.min.js" => return ApiResponse::javascript(THREE_JS),
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
            Some(q) => handle_ask(&conn, &q),
            None => ApiResponse::error(400, "missing required query param `q`"),
        },
        "/api/suggest" => handle_suggest(&conn),
        "/api/packs" => handle_packs_list(&conn),
        "/api/prompt" => handle_prompt(root, &conn, body),
        "/api/confirm" => handle_confirm(&conn, body),
        "/api/delete" => handle_delete(&conn, body),
        "/api/edit" => handle_edit(&conn, body),
        "/api/attach" => handle_attach(&conn, body),
        "/api/waive" => handle_waive(&conn, body),
        "/api/unwaive" => handle_unwaive(&conn, body),
        "/api/packs/install" => handle_pack_install(root, &conn, body),
        "/api/generate" => handle_generate(root, &conn, body),
        "/api/publish" => handle_publish(root, &conn),
        "/api/preview/start" => handle_preview_start(root, &conn),
        "/api/preview/stop" => {
            crate::hi_preview::stop();
            ok_response()
        }
        "/api/preview/status" => {
            let port = crate::hi_preview::status();
            ApiResponse::json(&serde_json::json!({ "running": port.is_some(), "port": port }))
        }
        _ => ApiResponse::error(404, "not found"),
    }
}

/// RFC 0014's 2026-09-14 amendment, step 4 (Governing Rules panel):
/// `known_installable_packs` is the fixed, safe set `hi`'s own UI may
/// offer -- never an arbitrary file path from the webview. Reports
/// `installed: true` for anything `installed_pack_ids` already lists,
/// so the panel can grey out "already installed" rather than letting a
/// second click silently no-op (`install_pack_from_bytes` upserts by
/// pack id, harmless but confusing to invite).
fn handle_packs_list(conn: &Connection) -> ApiResponse {
    let installed = match crate::hi_plugin::installed_pack_ids(conn) {
        Ok(ids) => ids,
        Err(e) => return ApiResponse::error(500, &e),
    };
    let packs: Vec<serde_json::Value> = crate::hi_plugin::known_installable_packs()
        .into_iter()
        .map(|(id, description, _bytes)| serde_json::json!({ "id": id, "description": description, "installed": installed.contains(&id.to_string()) }))
        .collect();
    ApiResponse::json(&packs)
}

/// Installs one of `known_installable_packs`' fixed, embedded packs by
/// id -- the webview never supplies pack bytes or a file path itself,
/// only which known-safe name to install, closing the path-traversal/
/// arbitrary-content-install surface a raw "install this JSON" route
/// would otherwise open up to whatever the page's own JS sends.
fn handle_pack_install(root: &Path, conn: &Connection, body: &[u8]) -> ApiResponse {
    let Some(pack_id) = body_param(body, "pack_id") else {
        return ApiResponse::error(400, "missing required field `pack_id`");
    };
    let Some((_, _, bytes)) = crate::hi_plugin::known_installable_packs().into_iter().find(|(id, _, _)| *id == pack_id) else {
        return ApiResponse::error(400, &format!("`{pack_id}` is not one of the packs this UI can install"));
    };
    match crate::hi_plugin::install_pack_from_bytes(conn, root, bytes.as_bytes(), &format!("hi UI install: {pack_id}")) {
        Ok(id) => ApiResponse::json(&serde_json::json!({ "ok": true, "pack_id": id })),
        Err(e) => ApiResponse::error(500, &e),
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
    crate::hi_llm::LlmClient::new(activation)
}

/// `hi_graph::ask`'s local keyword search first, always -- fast, free,
/// no network call, and always returned as supporting evidence. **No
/// longer a gate on the LLM answer** (github "Ask relevance" fix,
/// 2026-09-14): the old version returned local `hits` alone and never
/// even asked the model the instant *any* hit existed, even one
/// matching on a single generic 3+ character word buried in some
/// unrelated unit's driving text -- a weak match won by default, with
/// no chance at a better answer. Now, whenever an LLM is already
/// configured (`require_llm_client` failing here just means "local
/// hits only," the same result an unconfigured session already gave
/// before this fallback existed -- never a hard error), `hits` and a
/// real `answer` both come back together: `hi_llm::answer_question`
/// runs regardless of whether local search found anything, and can
/// search the project itself (`search_project`, as many times as it
/// needs) before answering, so it's grounded rather than guessing from
/// one flattened summary. See that function's own doc comment for why
/// this doesn't violate RFC 0014's "semantic search stays opt-in"
/// posture. The two are still rendered as distinct as ever client-side
/// (`hi_graph.html`'s `renderAskResult`) -- a hit is a passage, an
/// answer is a real model reply, never blurred into one bubble that
/// overstates what either actually is.
fn handle_ask(conn: &Connection, q: &str) -> ApiResponse {
    let hits = match crate::hi_graph::ask(conn, q) {
        Ok(h) => h,
        Err(e) => return ApiResponse::error(500, &e),
    };
    let Ok(client) = require_llm_client() else {
        return ApiResponse::json(&serde_json::json!({ "hits": hits, "answer": null }));
    };
    let context = match crate::hi_graph::project_context(conn) {
        Ok(c) => c,
        Err(e) => return ApiResponse::error(500, &e),
    };
    match crate::hi_llm::answer_question(&client, q, &context, conn) {
        Ok(answer) => ApiResponse::json(&serde_json::json!({ "hits": hits, "answer": answer })),
        Err(e) => ApiResponse::json(&serde_json::json!({ "hits": hits, "answer": null, "error": e })),
    }
}

/// Github #49's proactive-suggestions route: a read-only LLM pass over
/// the confirmed graph (`hi_graph::suggestion_context`,
/// `hi_llm::suggest_gaps`), returning what looks missing without
/// writing anything to `hi.db` -- deliberately GET, not in
/// `MUTATING_PATHS`, since this call itself never mutates state; only
/// a later, explicit Accept click (replayed through the already-
/// mutating `/api/attach`+`/api/confirm` or `/api/prompt`) does. No LLM
/// configured is the same graceful "nothing to show" `handle_ask`
/// already gives its own fallback, never a hard error -- a project
/// with no model configured just sees an empty suggestion rail instead
/// of a broken one.
fn handle_suggest(conn: &Connection) -> ApiResponse {
    let Ok(client) = require_llm_client() else {
        return ApiResponse::json(&serde_json::json!({ "suggestions": [] }));
    };
    let context = match crate::hi_graph::suggestion_context(conn) {
        Ok(c) => c,
        Err(e) => return ApiResponse::error(500, &e),
    };
    match crate::hi_llm::suggest_gaps(&client, &context) {
        Ok(items) => ApiResponse::json(&serde_json::json!({ "suggestions": items })),
        Err(e) => ApiResponse::json(&serde_json::json!({ "suggestions": [], "error": e })),
    }
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

/// `node` omitted (no body, or a body with no `node` param) means
/// "everything is to be compiled" -- confirms every still-unconfirmed,
/// non-waived candidate in one call (`hi_graph::confirm_all`) rather
/// than requiring one request per node.
fn handle_confirm(conn: &Connection, body: &[u8]) -> ApiResponse {
    match body_param(body, "node") {
        Some(node) => match crate::hi_graph::confirm_node(conn, &node) {
            Ok(()) => ok_response(),
            Err(e) => ApiResponse::error(400, &e),
        },
        None => match crate::hi_graph::confirm_all(conn) {
            Ok(ids) => ApiResponse::json(&serde_json::json!({ "ok": true, "confirmed": ids })),
            Err(e) => ApiResponse::error(500, &e),
        },
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
    // The relational half of the confirmed graph: the decompose step
    // stored `depends_on` edges at prompt time, and until 2026-09-11
    // Generate mode dropped them on the floor -- the code model got a
    // flat component list and free-ranged the wiring.
    let edges = match crate::hi_graph::confirmed_edges(conn) {
        Ok(e) => e,
        Err(e) => return ApiResponse::error(500, &e),
    };
    let client = match require_llm_client() {
        Ok(c) => c,
        Err(e) => return ApiResponse::error(500, &e),
    };
    let mut log_lines: Vec<String> = Vec::new();
    let path = match crate::hi_llm::generate_program(conn, root, &client, &units, &edges, &mut |line| log_lines.push(line.to_string())) {
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

/// The env var that opts a deployment into real Ed25519 signing of the
/// certificate every publish now emits (RFC 0016 Phase 4). Unset by
/// default, and never something `generate_program`/the model can reach
/// -- reading process environment happens only here, in a route the
/// LLM has no path to invoke on itself -- mirroring 5b's existing "AI
/// never gets signing" line and `certify --sign`'s own explicit
/// `key.pk8` CLI argument. A deployment with an operator key and a
/// governance story sets this; everyone else still gets a certificate,
/// just an unsigned one -- "mandatory certificate, optional signature,"
/// per the RFC's own "5b must not gate 5a/4" principle applied one
/// layer up: don't let governance-blocked work (real signing) block
/// something shippable (the certificate itself).
const PUBLISH_SIGNING_KEY_VAR: &str = "NIRDOSHA_PUBLISH_SIGNING_KEY";

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
fn handle_publish(root: &Path, conn: &Connection) -> ApiResponse {
    let source_path = crate::hi_llm::generated_source_path(root);
    if !source_path.exists() {
        return ApiResponse::error(400, "nothing generated yet -- run :generate first");
    }
    let result: Result<(std::path::PathBuf, std::path::PathBuf, bool), String> = (|| {
        let path_str = source_path.to_str().ok_or_else(|| format!("generated source path {} is not valid UTF-8", source_path.display()))?;
        let (program, _src): (crate::ast::Program, String) = crate::loader::load_program(path_str)?;
        if let Err(errors) = crate::typeck::typecheck(&program) {
            return Err(errors.iter().map(|e| format!("type error: {e}")).collect::<Vec<_>>().join("\n"));
        }
        if let Err(errors) = crate::ownership::check_ownership(&program) {
            return Err(errors.iter().map(|e| format!("ownership error: {e}")).collect::<Vec<_>>().join("\n"));
        }
        // RFC 0016 Phase 1: the coverage gate re-checks the file actually
        // being published, not the one generate produced -- a hand edit
        // between :generate and :publish (or a stale draft from an older
        // graph state) must not sneak a demanded-contract violation past
        // the gate. Same check, same classes, as the generate loop ran.
        let units = crate::hi_graph::confirmed_units(conn, None)?;
        if let Err(failure) = crate::hi_llm::contract_coverage_check_program(&program, &units) {
            return Err(format!("publish refused -- the file fails the contract coverage re-check (RFC 0016): {}", failure.diagnostic));
        }
        let smt_report = crate::smt::analyze(&program);
        let out_path = crate::hi_graph::hi_dir(root).join("generated").join("hi_build");
        crate::codegen::build(&program, &smt_report, &out_path, crate::codegen::OptLevel::O2)?;

        // RFC 0016 Phase 4 (issue #59): every successful publish also
        // emits a certificate, closing the gap where a project could
        // `:publish` a real binary with zero certificate ever produced.
        // Additive to, not a replacement for, `nirdosha certify`'s
        // standalone CLI path -- this just means nobody has to remember
        // to run it separately. `run_verify_pipeline` re-derives the
        // same load/typecheck/ownership/contract-check verdict `certify`
        // itself would over this exact file; `governing_packs`/
        // `governing_invariants`/`nfr_commitments` are the three fields
        // only this call site can fill in (a bare `cmd_certify`/
        // `certify_code` call has no project graph or parsed `Program`
        // to attribute against -- see `Certificate`'s own doc comments).
        let source_bytes = std::fs::read(&source_path).map_err(|e| format!("reading {} to certify it: {e}", source_path.display()))?;
        let pipeline = crate::mcp_tools::run_verify_pipeline(path_str);
        let mut certificate = crate::mcp_tools::build_certificate(&source_bytes, pipeline);
        certificate.governing_packs = crate::hi_plugin::installed_pack_ids(conn)?;
        certificate.governing_invariants = crate::hi_plugin::governing_invariants(conn, root, &program)?;
        certificate.nfr_commitments = crate::mcp_tools::nfr_commitments_from_program(&program);

        let cert_path = out_path.with_extension("certificate.json");
        let signing_key_path = std::env::var(PUBLISH_SIGNING_KEY_VAR).ok();
        let (json, signed) = certificate_json_for_publish(&certificate, signing_key_path.as_deref())?;
        std::fs::write(&cert_path, json).map_err(|e| format!("writing {}: {e}", cert_path.display()))?;

        Ok((out_path, cert_path, signed))
    })();
    match result {
        Ok((out_path, cert_path, signed)) => {
            ApiResponse::json(&serde_json::json!({ "ok": true, "binary": out_path.display().to_string(), "certificate": cert_path.display().to_string(), "signed": signed }))
        }
        Err(e) => ApiResponse::json(&serde_json::json!({ "ok": false, "error": e })),
    }
}

/// The publish-time certificate write, factored out of `handle_publish`
/// as a pure function of `(certificate, signing_key_path)` -- reading
/// `PUBLISH_SIGNING_KEY_VAR` stays in `handle_publish` itself, so this
/// function never touches real process environment and is directly
/// exercisable by tests for both the signed and unsigned branches
/// without racing every other test in this file that publishes
/// concurrently (`cargo test`'s default multi-threaded runner would
/// otherwise let one test's env mutation leak into another's assertion
/// -- the same hazard `hi_llm::resolve_activation`'s own injected-`env`
/// pattern exists to avoid, applied here as an injected argument
/// instead since `handle_publish`'s signature is fixed by the route
/// dispatcher).
fn certificate_json_for_publish(certificate: &crate::mcp_tools::Certificate, signing_key_path: Option<&str>) -> Result<(String, bool), String> {
    match signing_key_path {
        Some(key_path) => {
            let signed_cert = crate::mcp_tools::sign_certificate(certificate, key_path)?;
            Ok((serde_json::to_string_pretty(&signed_cert).expect("SignedCertificate always serializes"), true))
        }
        None => Ok((serde_json::to_string_pretty(certificate).expect("Certificate always serializes"), false)),
    }
}

/// Github #45's "ship now" half: build a real *servable* binary
/// (`codegen::build_serve`, `main.rs::cmd_build`'s own `--serve`
/// pipeline -- unlike `handle_publish` above, which calls the plain,
/// non-serving `codegen::build`) and hand it to `hi_preview::restart`
/// to run. Same typecheck/ownership/RFC-0016-coverage re-check as
/// Publish, over the same `hi_llm::generated_source_path` file, so a
/// preview never shows something that wouldn't actually pass Publish
/// either -- "preview" here means "run the real thing," not a mockup.
/// Demo-mode identity is the served app's own already-real login
/// screen (see `hi_preview.rs`'s own doc comment) -- nothing new here
/// has to know about roles/claims at all.
fn handle_preview_start(root: &Path, conn: &Connection) -> ApiResponse {
    let source_path = crate::hi_llm::generated_source_path(root);
    if !source_path.exists() {
        return ApiResponse::error(400, "nothing generated yet -- run :generate first");
    }
    let result: Result<u16, String> = (|| {
        let path_str = source_path.to_str().ok_or_else(|| format!("generated source path {} is not valid UTF-8", source_path.display()))?;
        let (program, _src): (crate::ast::Program, String) = crate::loader::load_program(path_str)?;
        if let Err(errors) = crate::typeck::typecheck(&program) {
            return Err(errors.iter().map(|e| format!("type error: {e}")).collect::<Vec<_>>().join("\n"));
        }
        if let Err(errors) = crate::ownership::check_ownership(&program) {
            return Err(errors.iter().map(|e| format!("ownership error: {e}")).collect::<Vec<_>>().join("\n"));
        }
        let units = crate::hi_graph::confirmed_units(conn, None)?;
        if let Err(failure) = crate::hi_llm::contract_coverage_check_program(&program, &units) {
            return Err(format!("preview refused -- the file fails the contract coverage re-check (RFC 0016): {}", failure.diagnostic));
        }
        let smt_report = crate::smt::analyze(&program);
        // Same UI-generation call `cmd_build --serve` makes: `demo_mode:
        // true`, `production_mode: false` -- a compiled process has no
        // `Program` AST left at runtime, so the UI is generated now and
        // baked into the binary as bytes (`ServeCodegenOptions::ui_html`).
        let registry = crate::ast::TypeRegistry::build(&program);
        let effects = crate::effects::infer_effects(&program, &registry);
        let ui_html = crate::ui_gen::generate(&program, &effects, None, false, true, false, None).into_bytes();
        let require_sender_constrained_tokens = crate::hi_plugin::wiring_requires_sender_constrained_tokens(conn, root).unwrap_or(false);
        let port = crate::hi_preview::pick_free_port()?;
        let opts = crate::codegen::ServeCodegenOptions { port, ui_html, require_sender_constrained_tokens };
        let out_path = crate::hi_graph::hi_dir(root).join("generated").join("hi_preview");
        crate::codegen::build_serve(&program, &smt_report, &out_path, crate::codegen::OptLevel::O2, &opts)?;
        crate::hi_preview::restart(&out_path, port)?;
        Ok(port)
    })();
    match result {
        Ok(port) => ApiResponse::json(&serde_json::json!({ "ok": true, "port": port })),
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
    /// RFC 0016's "sealed domain plugin" provenance (5a: unsigned,
    /// trust-on-first-use) -- `None` for anything the user's own
    /// prompt proposed. Exposed here (previously read only by
    /// `hi_plugin.rs`'s own Rust-side checks, never serialized to the
    /// webview) so the build-mode UI can show *which* nodes are sealed
    /// law rather than leaving `:waive`'s refusal as the only place a
    /// user ever learns one exists (`hi_graph.rs::waive_node`'s own
    /// refuse-on-plugin-origin check, RFC 0014's 2026-09-14 amendment,
    /// "Governing rules" panel).
    plugin_origin: Option<String>,
    /// Same RFC 0016 provenance: `true` means `:waive`/`:delete` (and
    /// this panel's own edit form) refuse outright -- surfaced so a
    /// refusal reads as "this is sealed law" instead of "the button is
    /// broken."
    non_waivable: bool,
}

fn list_nodes(conn: &Connection) -> Result<Vec<NodeRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, kind, title, status, content_hash, source_ref, line, col, driving_text, created_by, confirmed, locked, waived, waive_reason, attributes, plugin_origin, non_waivable FROM nodes LIMIT ?1",
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
                plugin_origin: r.get(15)?,
                non_waivable: r.get::<_, i64>(16)? != 0,
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
        assert!(body.contains("/assets/three.min.js"), "page should load the vendored THREE global");
        let three_idx = body.find("/assets/three.min.js").expect("checked above");
        let force_graph_idx = body.find("/assets/3d-force-graph.min.js").expect("checked above");
        assert!(three_idx < force_graph_idx, "three.min.js must load BEFORE 3d-force-graph.min.js -- that bundle picks its internal-vs-page-supplied THREE once, at its own script-execution time (github #57)");
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

        let three_resp = handle(&dir, "GET", "/assets/three.min.js", "", b"");
        assert_eq!(three_resp.status, 200);
        assert_eq!(three_resp.content_type, "application/javascript; charset=utf-8");
        let three_body = String::from_utf8_lossy(&three_resp.body);
        assert!(three_body.contains("var THREE="), "must assign a classic global THREE, not just export an ES module");
        assert!(three_body.contains("REVISION"), "sanity: this is really three.js, not an empty/placeholder file");

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
        // An ordinary, non-plugin node reports the RFC 0016 provenance
        // fields as absent/false, not merely omitted -- the build-mode
        // UI's "governing rules" panel (RFC 0014's 2026-09-14 amendment)
        // needs to tell "no pack governs this" apart from "the field
        // wasn't sent."
        let add = nodes.iter().find(|n| n["id"] == "code:fn:add").unwrap();
        assert_eq!(add["plugin_origin"], serde_json::Value::Null);
        assert_eq!(add["non_waivable"], false);
    }

    /// RFC 0014's 2026-09-14 amendment ("Governing rules" panel) needs
    /// `/api/nodes` to expose which nodes a sealed domain plugin owns —
    /// previously read only on the Rust side (`hi_graph::waive_node`'s
    /// own refuse-on-plugin-origin check), never serialized to the
    /// webview at all.
    #[test]
    fn handle_lists_nodes_reports_plugin_origin_and_non_waivable() {
        let dir = scratch_dir("nodes_plugin_origin");
        let conn = crate::hi_graph::open(&dir).expect("open");
        crate::hi_plugin::ensure_default_packs(&conn, &dir).expect("ensure default packs");
        drop(conn);

        let resp = handle(&dir, "GET", "/api/nodes", "", b"");
        assert_eq!(resp.status, 200);
        let nodes: Vec<serde_json::Value> = serde_json::from_slice(&resp.body).expect("valid JSON array");
        let sealed = nodes.iter().find(|n| n["plugin_origin"] != serde_json::Value::Null).unwrap_or_else(|| {
            panic!("expected at least one plugin-sourced node after ensure_default_packs, got {resp:?}", resp = String::from_utf8_lossy(&resp.body))
        });
        assert_eq!(sealed["non_waivable"], true, "a plugin-sourced node must report non_waivable: true, got {sealed:?}");
    }

    /// RFC 0014's 2026-09-14 amendment, step 4: the Governing Rules
    /// panel needs to list what it can offer to install, and know
    /// what's already there.
    #[test]
    fn handle_packs_list_reports_known_packs_and_installed_state() {
        let dir = scratch_dir("packs_list");
        let conn = crate::hi_graph::open(&dir).expect("open");
        crate::hi_plugin::ensure_default_packs(&conn, &dir).expect("ensure default packs");
        drop(conn);

        let resp = handle(&dir, "GET", "/api/packs", "", b"");
        assert_eq!(resp.status, 200);
        let packs: Vec<serde_json::Value> = serde_json::from_slice(&resp.body).expect("valid JSON array");
        let banking = packs.iter().find(|p| p["id"] == "banking-v0").expect("banking-v0 should be a known pack");
        assert_eq!(banking["installed"], true, "ensure_default_packs already installed it: {banking:?}");
        let fapi = packs.iter().find(|p| p["id"] == "fapi-2.0").expect("fapi-2.0 should be a known pack");
        assert_eq!(fapi["installed"], false, "fapi-2.0 is never auto-installed: {fapi:?}");
    }

    #[test]
    fn handle_pack_install_installs_a_known_pack_and_refuses_an_unknown_one() {
        let dir = scratch_dir("pack_install");
        crate::hi_graph::open(&dir).expect("open"); // scaffold only, no default packs

        let resp = handle(&dir, "POST", "/api/packs/install", "", b"pack_id=fapi-2.0");
        assert_eq!(resp.status, 200, "install should succeed: {}", String::from_utf8_lossy(&resp.body));
        let body: serde_json::Value = serde_json::from_slice(&resp.body).expect("valid JSON");
        assert_eq!(body["pack_id"], "fapi-2.0");

        let conn = crate::hi_graph::open(&dir).expect("reopen");
        let installed = crate::hi_plugin::installed_pack_ids(&conn).expect("list installed packs");
        assert!(installed.contains(&"fapi-2.0".to_string()), "expected fapi-2.0 among installed packs: {installed:?}");

        let refused = handle(&dir, "POST", "/api/packs/install", "", b"pack_id=not-a-real-pack");
        assert_eq!(refused.status, 400, "an unknown pack id must be refused, not silently accepted");
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

    #[test]
    fn publish_refuses_when_a_demanded_contract_is_dropped() {
        // RFC 0016 Phase 1: the coverage gate re-checks the file actually
        // being published. This is the hand-edit hole: generate produced a
        // draft whose contracts passed (or was written before the demand
        // existed), a hand edit between :generate and :publish removed the
        // contract -- publish must refuse, not compile it silently.
        let dir = scratch_dir("publish_coverage_dropped");
        let conn = crate::hi_graph::open(&dir).expect("open");
        let id = crate::hi_graph::add_candidate(&conn, "fn", "charge_cents", "charges money", "test").expect("add");
        crate::hi_graph::attach_attribute(&conn, &id, "validate contract balance_nonnegative: result >= 0").expect("attach");
        crate::hi_graph::confirm_node(&conn, &id).expect("confirm");
        drop(conn);
        let out_path = crate::hi_llm::generated_source_path(&dir);
        std::fs::create_dir_all(out_path.parent().unwrap()).expect("mkdir");
        std::fs::write(&out_path, "fn charge_cents(amount_cents: i64, balance_cents: i64) -> i64 {\n    return balance_cents - amount_cents\n}\n\nfn main() requires(public) {\n    print(\"charge\", charge_cents(100, 500))\n}\n").expect("write");
        let resp = handle(&dir, "POST", "/api/publish", "", b"");
        let body = String::from_utf8_lossy(&resp.body);
        let json: serde_json::Value = serde_json::from_str(&body).expect("valid JSON: {body}");
        assert_eq!(json["ok"], serde_json::json!(false), "publish must refuse a file without the demanded contract: {body}");
        assert!(body.contains("coverage re-check"), "the refusal must name the gate: {body}");
        assert!(body.contains("carries none"), "the refusal must carry the class's own marker: {body}");
    }

    #[test]
    fn publish_succeeds_when_the_demanded_contract_proves() {
        // The happy path through the same gate: a demanded, proving
        // contract must not false-positive the publish re-check.
        let dir = scratch_dir("publish_coverage_proves");
        let conn = crate::hi_graph::open(&dir).expect("open");
        let id = crate::hi_graph::add_candidate(&conn, "fn", "charge_cents", "charges money", "test").expect("add");
        crate::hi_graph::attach_attribute(&conn, &id, "validate contract balance_nonnegative: result >= 0").expect("attach");
        crate::hi_graph::confirm_node(&conn, &id).expect("confirm");
        drop(conn);
        let out_path = crate::hi_llm::generated_source_path(&dir);
        std::fs::create_dir_all(out_path.parent().unwrap()).expect("mkdir");
        std::fs::write(
            &out_path,
            "fn charge_cents(amount_cents: i64, balance_cents: i64) -> i64 {\n    return balance_cents - amount_cents\n}\n\nvalidate charge_cents {\n    pre: amount_cents >= 0 && amount_cents <= balance_cents\n    post: result >= 0\n}\n\nfn main() requires(public) {\n    print(\"charge\", charge_cents(100, 500))\n}\n",
        ).expect("write");
        let resp = handle(&dir, "POST", "/api/publish", "", b"");
        let body = String::from_utf8_lossy(&resp.body);
        let json: serde_json::Value = serde_json::from_str(&body).expect("valid JSON: {body}");
        assert_eq!(json["ok"], serde_json::json!(true), "a demanded, proving contract must publish: {body}");
    }

    /// RFC 0016 Phase 4 (issue #59): a successful publish now also
    /// writes a certificate alongside the binary, unconditionally --
    /// this closes the gap where `:publish` produced a real binary with
    /// zero certificate ever produced. No signing key is configured in
    /// this test, so it must come back unsigned (`signed: false`), and
    /// the certificate itself must carry a `"proved"` evidence tier for
    /// this exact fixture (the same proving contract
    /// `publish_succeeds_when_the_demanded_contract_proves` uses).
    /// `governing_packs`/`governing_invariants` must both be empty --
    /// this scratch project never installs a pack.
    #[test]
    fn publish_writes_an_unsigned_certificate_by_default() {
        let dir = scratch_dir("publish_certificate_unsigned");
        let conn = crate::hi_graph::open(&dir).expect("open");
        let id = crate::hi_graph::add_candidate(&conn, "fn", "charge_cents", "charges money", "test").expect("add");
        crate::hi_graph::attach_attribute(&conn, &id, "validate contract balance_nonnegative: result >= 0").expect("attach");
        crate::hi_graph::confirm_node(&conn, &id).expect("confirm");
        drop(conn);
        let out_path = crate::hi_llm::generated_source_path(&dir);
        std::fs::create_dir_all(out_path.parent().unwrap()).expect("mkdir");
        std::fs::write(
            &out_path,
            "fn charge_cents(amount_cents: i64, balance_cents: i64) -> i64 {\n    return balance_cents - amount_cents\n}\n\nvalidate charge_cents {\n    pre: amount_cents >= 0 && amount_cents <= balance_cents\n    post: result >= 0\n}\n\nfn main() requires(public) {\n    print(\"charge\", charge_cents(100, 500))\n}\n",
        ).expect("write");

        let resp = handle(&dir, "POST", "/api/publish", "", b"");
        let body = String::from_utf8_lossy(&resp.body);
        let json: serde_json::Value = serde_json::from_str(&body).expect("valid JSON: {body}");
        assert_eq!(json["ok"], serde_json::json!(true), "publish must succeed: {body}");
        assert_eq!(json["signed"], serde_json::json!(false), "no signing key is configured in this test: {body}");

        let cert_path = json["certificate"].as_str().expect("certificate path in the response");
        let cert_bytes = std::fs::read(cert_path).expect("the certificate file must actually exist on disk");
        let cert: serde_json::Value = serde_json::from_slice(&cert_bytes).expect("the certificate file must be valid JSON");
        assert_eq!(cert["evidence_tier"], serde_json::json!("proved"), "this fixture's contract proves: {cert}");
        assert_eq!(cert["governing_packs"], serde_json::json!([]), "no pack is installed in this scratch project: {cert}");
        assert_eq!(cert["governing_invariants"], serde_json::json!([]), "no pack is installed, so no invariant can be attributed to one: {cert}");
        assert_eq!(cert["nfr_commitments"], serde_json::json!([]), "this fixture declares no nfr(...): {cert}");
        // An unsigned certificate must carry none of `SignedCertificate`'s
        // three extra fields -- a plain `Certificate`, not a signed one
        // that merely lacks a valid signature.
        assert!(cert.get("signature").is_none(), "unsigned certificate must not carry a signature field: {cert}");
    }

    /// RFC 0016 Phase 4: `certificate_json_for_publish` is the pure core
    /// `handle_publish` calls after resolving `PUBLISH_SIGNING_KEY_VAR`
    /// -- exercised directly here (a real, freshly generated Ed25519
    /// keypair, mirroring `nirdosha keygen`'s own `generate_pkcs8` call)
    /// so this test never has to mutate real process environment and
    /// race every other test in this file that publishes concurrently
    /// (the function's own doc comment explains why).
    #[test]
    fn certificate_json_for_publish_signs_when_given_a_real_key() {
        let dir = scratch_dir("certificate_signing");
        std::fs::create_dir_all(&dir).expect("mkdir");
        let key_path = dir.join("signing_key.pk8");
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("key generation");
        std::fs::write(&key_path, pkcs8.as_ref()).expect("write key");

        let source_path = dir.join("cert_fixture.nir");
        let source_bytes = b"fn main() requires(public) {\n    print(\"hello\", 1)\n}\n";
        std::fs::write(&source_path, source_bytes).expect("write fixture source");
        let pipeline = crate::mcp_tools::run_verify_pipeline(source_path.to_str().unwrap());
        let certificate = crate::mcp_tools::build_certificate(source_bytes, pipeline);

        let (unsigned_json, unsigned) = certificate_json_for_publish(&certificate, None).expect("unsigned branch never fails");
        assert!(!unsigned, "no key path given");
        let unsigned_value: serde_json::Value = serde_json::from_str(&unsigned_json).expect("valid JSON");
        assert!(unsigned_value.get("signature").is_none(), "unsigned certificate must carry no signature field: {unsigned_value}");

        let (signed_json, signed) = certificate_json_for_publish(&certificate, Some(key_path.to_str().unwrap())).expect("signing with a real key must succeed");
        assert!(signed, "a key path was given");
        let signed_value: serde_json::Value = serde_json::from_str(&signed_json).expect("valid JSON");
        assert_eq!(signed_value["signature_algorithm"], serde_json::json!("ed25519"));
        assert!(signed_value["signature"].as_str().is_some_and(|s| !s.is_empty()), "must carry a real, non-empty signature: {signed_value}");
        assert!(signed_value["public_key"].as_str().is_some_and(|s| !s.is_empty()), "must carry the public key alongside it: {signed_value}");
        // The signed form is additive over the plain certificate -- every
        // v0 field the unsigned form has must still be present verbatim.
        assert_eq!(signed_value["source_hash"], unsigned_value["source_hash"]);
        assert_eq!(signed_value["evidence_tier"], unsigned_value["evidence_tier"]);
    }

    /// RFC 0016 Phase 4: `governing_packs`/`nfr_commitments` are the two
    /// fields only a publish (not a bare `cmd_certify`) can fill in --
    /// this drives the same banking-pack fixture the RFC's own Phase 2
    /// acceptance test uses, then checks the certificate names the pack.
    #[test]
    fn publish_certificate_names_the_governing_pack() {
        let dir = scratch_dir("publish_certificate_governing_pack");
        let conn = crate::hi_graph::open(&dir).expect("open");
        crate::hi_plugin::ensure_default_packs(&conn, &dir).expect("ensure default packs");
        let draft = r#"
fn charge_cents(balance_cents: i64, amount_cents: i64) -> i64 {
    return balance_cents - amount_cents
}

fn credit_cents(balance_cents: i64, amount_cents: i64) -> i64 {
    return balance_cents + amount_cents
}

fn net_change_cents(credits: i64, debits: i64) -> i64 {
    return credits - debits
}

fn main() requires(public) {
    let after_charge: i64 = charge_cents(10000, 1200)
    let after_credit: i64 = credit_cents(after_charge, 300)
    print("balance", after_credit)
    print("net change", net_change_cents(300, 1200))
}
"#;
        let injected = crate::hi_plugin::inject_pack_validates_into_source(&dir, draft).expect("pack injection must succeed over a signature-matching draft");
        drop(conn);
        let out_path = crate::hi_llm::generated_source_path(&dir);
        std::fs::create_dir_all(out_path.parent().unwrap()).expect("mkdir");
        std::fs::write(&out_path, &injected).expect("write generated source");

        let resp = handle(&dir, "POST", "/api/publish", "", b"");
        let body = String::from_utf8_lossy(&resp.body);
        let json: serde_json::Value = serde_json::from_str(&body).expect("valid JSON: {body}");
        assert_eq!(json["ok"], serde_json::json!(true), "the pack-governed draft must publish: {body}");

        let cert_path = json["certificate"].as_str().expect("certificate path in the response");
        let cert: serde_json::Value = serde_json::from_slice(&std::fs::read(cert_path).expect("read certificate")).expect("valid JSON");
        assert_eq!(cert["governing_packs"], serde_json::json!(["banking-v0"]), "the banking pack must be named in the certificate: {cert}");
    }

    /// RFC 0016 Phase 4: an `nfr(...)` commitment must land in the
    /// certificate tagged `"monitored"`, never `"proved"` -- it's an
    /// APM-kernel-tracked runtime claim, not a Z3 proof, and must not
    /// read as one just because it rode along in the same certificate.
    #[test]
    fn publish_certificate_tiers_nfr_commitments_as_monitored_not_proved() {
        let dir = scratch_dir("publish_certificate_nfr");
        crate::hi_graph::open(&dir).expect("open"); // scaffold .nir/ so /api/publish's own `hi_graph::open` succeeds
        let out_path = crate::hi_llm::generated_source_path(&dir);
        std::fs::create_dir_all(out_path.parent().unwrap()).expect("mkdir");
        std::fs::write(
            &out_path,
            "fn slow_lookup(id: i64) -> i64 nfr(latency_ms: 50, concurrency_max: 10) {\n    return id\n}\n\nfn main() requires(public) {\n    print(\"lookup\", slow_lookup(1))\n}\n",
        ).expect("write");

        let resp = handle(&dir, "POST", "/api/publish", "", b"");
        let body = String::from_utf8_lossy(&resp.body);
        let json: serde_json::Value = serde_json::from_str(&body).expect("valid JSON: {body}");
        assert_eq!(json["ok"], serde_json::json!(true), "a program with an nfr(...) fn must still publish: {body}");

        let cert_path = json["certificate"].as_str().expect("certificate path in the response");
        let cert: serde_json::Value = serde_json::from_slice(&std::fs::read(cert_path).expect("read certificate")).expect("valid JSON");
        let commitments = cert["nfr_commitments"].as_array().expect("nfr_commitments array");
        assert_eq!(commitments.len(), 1, "exactly one fn declares nfr(...): {cert}");
        assert_eq!(commitments[0]["fn_name"], serde_json::json!("slow_lookup"));
        assert_eq!(commitments[0]["evidence_tier"], serde_json::json!("monitored"), "an NFR claim is runtime-monitored, never Z3-proved: {commitments:?}");
        assert_eq!(commitments[0]["nfr"]["latency_ms"], serde_json::json!(50));
        assert_eq!(commitments[0]["nfr"]["concurrency_max"], serde_json::json!(10));
    }

    /// RFC 0016 implementation plan, Phase 2 item 7: "the fintech v4 run
    /// under the pack -- generate in one shot, `nirdosha verify` reports
    /// `PROVED n/n` with the governing pack named." No standalone test
    /// exercised this full round trip before this test -- the injection
    /// unit tests in `hi_plugin.rs` prove the mechanics in isolation, but
    /// nothing before this drove a banking-shaped draft all the way
    /// through `/api/publish`'s real coverage re-check *and* independently
    /// re-proved every contract to confirm PROVED n/n, naming the pack
    /// that governed it.
    ///
    /// `generate_program` itself is not driven here (it calls out to a
    /// real LLM) -- like `publish_succeeds_when_the_demanded_contract_proves`
    /// above, this writes the draft `generate_program` would have
    /// produced directly to the generated-source path, which is exactly
    /// what `/api/publish` reads and re-checks.
    #[test]
    fn fintech_app_under_the_banking_pack_publishes_proved_n_of_n_with_the_governing_pack_named() {
        let dir = scratch_dir("fintech_v4_under_pack");
        let conn = crate::hi_graph::open(&dir).expect("open");
        crate::hi_plugin::ensure_default_packs(&conn, &dir).expect("ensure default packs");

        // A banking-day draft using the pack's own mandatory fn names/
        // signatures (`agent-skills/nirdosha/packs/banking-v0.json`) --
        // the shape a real generate run under the pack must produce,
        // since `contract_coverage_check`'s injected mode demands these
        // exact signatures.
        let draft = r#"
fn charge_cents(balance_cents: i64, amount_cents: i64) -> i64 {
    return balance_cents - amount_cents
}

fn credit_cents(balance_cents: i64, amount_cents: i64) -> i64 {
    return balance_cents + amount_cents
}

fn net_change_cents(credits: i64, debits: i64) -> i64 {
    return credits - debits
}

fn main() requires(public) {
    let after_charge: i64 = charge_cents(10000, 1200)
    let after_credit: i64 = credit_cents(after_charge, 300)
    print("balance", after_credit)
    print("net change", net_change_cents(300, 1200))
}
"#;
        let injected = crate::hi_plugin::inject_pack_validates_into_source(&dir, draft).expect("pack injection must succeed over a signature-matching draft");
        drop(conn);

        let out_path = crate::hi_llm::generated_source_path(&dir);
        std::fs::create_dir_all(out_path.parent().unwrap()).expect("mkdir");
        std::fs::write(&out_path, &injected).expect("write generated source");

        // 1. `/api/publish`'s real coverage re-check must accept it.
        let resp = handle(&dir, "POST", "/api/publish", "", b"");
        let body = String::from_utf8_lossy(&resp.body);
        let json: serde_json::Value = serde_json::from_str(&body).expect("valid JSON: {body}");
        assert_eq!(json["ok"], serde_json::json!(true), "the pack-governed draft must publish: {body}");

        // 2. PROVED n/n: every `validate` block the pack injected proves,
        // none dropped, none merely non-vacuous-but-weak.
        let (program, _src) = crate::loader::load_program(out_path.to_str().unwrap()).expect("published file must load");
        let outcomes = crate::contract_check::run_program_validates(&program);
        assert_eq!(outcomes.len(), 3, "all three pack-demanded contracts must be present, got {} named {:?}", outcomes.len(), outcomes.iter().map(|o| &o.fn_name).collect::<Vec<_>>());
        for outcome in &outcomes {
            assert_eq!(outcome.result, crate::contract_check::ContractCheckResult::Proved, "{} must be PROVED", outcome.fn_name);
        }

        // 3. The governing pack named -- exactly one active pack
        // governed this graph, and it is `banking-v0`.
        let conn = crate::hi_graph::open(&dir).expect("reopen");
        let governing = crate::hi_plugin::installed_pack_ids(&conn).expect("list installed packs");
        assert_eq!(governing, vec!["banking-v0".to_string()], "the banking pack must be the sole governing pack");

        // 4. Real per-invariant attribution (`hi_plugin::
        // governing_invariants`, built 2026-09-15 -- this used to be the
        // RFC's own named-but-unimplemented "generation audit and
        // governing-set snapshot" gap): all three pack-demanded
        // contracts, each attributed to `banking-v0` by name, not just
        // "some active pack governs this graph somehow."
        let invariants = crate::hi_plugin::governing_invariants(&conn, &dir, &program).expect("compute governing invariants");
        let mut fn_names: Vec<&str> = invariants.iter().map(|i| i.fn_name.as_str()).collect();
        fn_names.sort_unstable();
        assert_eq!(fn_names, vec!["charge_cents", "credit_cents", "net_change_cents"], "every pack-demanded contract must be attributed by fn name: {fn_names:?}");
        assert!(invariants.iter().all(|i| i.pack_id == "banking-v0"), "every attribution must name banking-v0 as the governing pack: {invariants:?}");

        // 5. The certificate a real `/api/publish` call issues carries
        // the identical attribution -- not just something a direct
        // `governing_invariants` call happens to compute correctly.
        let cert_path = crate::hi_graph::hi_dir(&dir).join("generated").join("hi_build.certificate.json");
        let cert_json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&cert_path).expect("read the real certificate /api/publish wrote")).expect("valid certificate JSON");
        let cert_invariants = cert_json["governing_invariants"].as_array().expect("governing_invariants should be an array");
        assert_eq!(cert_invariants.len(), 3, "cert: {cert_json}");
        assert!(cert_invariants.iter().all(|i| i["pack_id"] == "banking-v0"), "cert: {cert_json}");
    }

    #[test]
    fn confirm_with_no_node_param_confirms_everything() {
        let dir = scratch_dir("confirm_all_http");
        let conn = crate::hi_graph::open(&dir).expect("open");
        let a = crate::hi_graph::add_candidate(&conn, "fn", "add", "adds two numbers", "llm-prompt-mode").expect("add a");
        let b = crate::hi_graph::add_candidate(&conn, "fn", "subtract", "subtracts two numbers", "llm-prompt-mode").expect("add b");
        drop(conn);

        let resp = handle(&dir, "POST", "/api/confirm", "", b"");
        assert_eq!(resp.status, 200, "bulk confirm should succeed: {}", String::from_utf8_lossy(&resp.body));
        let body: serde_json::Value = serde_json::from_slice(&resp.body).expect("valid JSON");
        let confirmed: Vec<String> = body["confirmed"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
        assert!(confirmed.contains(&a));
        assert!(confirmed.contains(&b));

        let resp = handle(&dir, "GET", "/api/nodes", "", b"");
        let nodes: Vec<serde_json::Value> = serde_json::from_slice(&resp.body).expect("valid JSON array");
        assert!(nodes.iter().all(|n| n["confirmed"] == 1), "every candidate should now be confirmed");
    }

    #[test]
    fn ask_finds_a_local_keyword_hit_without_ever_needing_an_llm() {
        // No LLM is configured in this test process, so `answer` stays
        // null here -- but as of the github "Ask relevance" fix
        // (2026-09-14), that's only because no client is configured,
        // not because a local hit rules an LLM answer out by design.
        // When a client *is* configured, `hits` and a real `answer`
        // are both returned together (see `handle_ask`'s own doc
        // comment) -- a weak local hit no longer silently wins over a
        // real, grounded answer the way it used to.
        let dir = scratch_dir("ask_local_hit");
        let conn = crate::hi_graph::open(&dir).expect("open");
        crate::hi_graph::add_candidate(&conn, "fn", "tick", "advances the game clock", "llm-prompt-mode").expect("add_candidate");
        drop(conn);

        let resp = handle(&dir, "GET", "/api/ask", "q=tick", b"");
        assert_eq!(resp.status, 200);
        let body: serde_json::Value = serde_json::from_slice(&resp.body).expect("valid JSON");
        assert_eq!(body["hits"].as_array().unwrap().len(), 1);
        assert!(body["answer"].is_null(), "no LLM is configured in this test process, so answer must stay null");
    }

    #[test]
    fn ask_degrades_to_no_matches_rather_than_erroring_when_nothing_local_and_no_llm_configured() {
        let dir = scratch_dir("ask_no_local_no_llm");
        crate::hi_graph::open(&dir).expect("open");

        let resp = handle(&dir, "GET", "/api/ask", "q=this+project+about", b"");
        assert_eq!(resp.status, 200, "an unconfigured LLM must degrade, not error: {}", String::from_utf8_lossy(&resp.body));
        let body: serde_json::Value = serde_json::from_slice(&resp.body).expect("valid JSON");
        assert!(body["hits"].as_array().unwrap().is_empty());
        assert!(body["answer"].is_null());
    }

    #[test]
    fn suggest_degrades_to_an_empty_list_rather_than_erroring_when_no_llm_configured() {
        // github #49: an unconfigured project must see an empty
        // suggestion rail, not a broken one -- same "GET never hard-
        // fails on a missing LLM" posture `handle_ask` already has.
        let dir = scratch_dir("suggest_no_llm");
        crate::hi_graph::open(&dir).expect("open");

        let resp = handle(&dir, "GET", "/api/suggest", "", b"");
        assert_eq!(resp.status, 200, "an unconfigured LLM must degrade, not error: {}", String::from_utf8_lossy(&resp.body));
        let body: serde_json::Value = serde_json::from_slice(&resp.body).expect("valid JSON");
        assert!(body["suggestions"].as_array().unwrap().is_empty());
    }

    #[test]
    fn preview_start_refuses_before_anything_has_been_generated() {
        // Same "run :generate first" gate `handle_publish` has, over the
        // same `hi_llm::generated_source_path` file -- preview builds
        // from that file too, it just uses `codegen::build_serve`
        // instead of `codegen::build` once it exists.
        let dir = scratch_dir("preview_nothing_generated");
        crate::hi_graph::open(&dir).expect("open");
        let resp = handle(&dir, "POST", "/api/preview/start", "", b"");
        assert_eq!(resp.status, 400);
        assert!(String::from_utf8_lossy(&resp.body).contains("nothing generated"));
    }

    #[test]
    fn preview_start_builds_and_runs_a_real_server_then_stop_tears_it_down() {
        // github #45's "ship now" half, end to end: a real
        // `codegen::build_serve` binary, actually spawned, actually
        // bound to a real port -- not a mock of any of that. Ensures
        // `hi_preview::stop()` afterward regardless of how the
        // assertions below turn out, so this test never leaves a real
        // child process (and a bound TCP port) behind for the rest of
        // the test binary's run.
        let _g = crate::hi_preview::PREVIEW_TESTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = scratch_dir("preview_start_real_server");
        crate::hi_graph::open(&dir).expect("open");
        let out_path = crate::hi_llm::generated_source_path(&dir);
        std::fs::create_dir_all(out_path.parent().unwrap()).expect("mkdir");
        std::fs::write(&out_path, "fn public_action() -> i64 requires(public) { return 1 }\nserve { expose public_action }\nfn main() requires(public) { }\n").expect("write");

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let resp = handle(&dir, "POST", "/api/preview/start", "", b"");
            let body = String::from_utf8_lossy(&resp.body).into_owned();
            let json: serde_json::Value = serde_json::from_str(&body).expect("valid JSON: {body}");
            assert_eq!(json["ok"], serde_json::json!(true), "a servable program must start a preview: {body}");
            let port = json["port"].as_u64().expect("a successful start reports a port");
            assert_ne!(port, 0);

            let status_resp = handle(&dir, "GET", "/api/preview/status", "", b"");
            let status: serde_json::Value = serde_json::from_slice(&status_resp.body).expect("valid JSON");
            assert_eq!(status["running"], serde_json::json!(true));
            assert_eq!(status["port"].as_u64(), Some(port));
        }));

        let stop_resp = handle(&dir, "POST", "/api/preview/stop", "", b"");
        assert_eq!(stop_resp.status, 200);
        let status_resp = handle(&dir, "GET", "/api/preview/status", "", b"");
        let status: serde_json::Value = serde_json::from_slice(&status_resp.body).expect("valid JSON");
        assert_eq!(status["running"], serde_json::json!(false), "stop must actually tear the preview down");

        result.expect("assertions inside the guarded block");
    }

    #[test]
    fn suggest_is_a_get_route_not_in_the_mutating_allowlist() {
        // github #49's own route doc comment: this call never writes to
        // `hi.db` itself, so it must stay outside `MUTATING_PATHS` --
        // a regression here would silently start requiring POST for a
        // route that has nothing to protect with that check.
        assert!(!MUTATING_PATHS.contains(&"/api/suggest"));
    }
}
