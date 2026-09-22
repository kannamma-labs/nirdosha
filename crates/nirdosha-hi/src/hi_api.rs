//! Transport-agnostic route table over `.nir/hi.db`
//! (rfcs/0014-generative-build-console.md, 2026-09-20 amendment) —
//! called by `hi_server.rs`, a real `tiny_http` socket, `127.0.0.1`-only
//! with an `Origin` allowlist, serving both the CLI's `--app=`-mode
//! browser window and headless/scripting/CI use. The RFC's original
//! embedded-`wry`/`tao` webview transport (`hi_window.rs`, "no network
//! port at all") was retired in favor of this single transport.
//!
//! Read-only, foundation-slice scope only (no write endpoints, no live
//! updates, no filtering beyond a hard cap) — see that RFC's own Open
//! Questions for the eventual full build-mode editing surface.

use std::path::Path;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use rusqlite::{Connection, OptionalExtension};
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
/// plain `<img src>`/link from any page anywhere, without ever carrying
/// the `Origin` header `hi_server.rs`'s own `has_browser_origin` check
/// relies on to reject browser-originated cross-origin requests. POST
/// closes that gap (modern browsers do send `Origin` on a cross-origin
/// POST) -- the two checks together are this transport's real CSRF
/// defense now that there's no embedded-webview transport left with no
/// network origin to attack at all.
const MUTATING_PATHS: &[&str] = &["/api/prompt", "/api/confirm", "/api/delete", "/api/edit", "/api/attach", "/api/waive", "/api/unwaive", "/api/packs/install", "/api/generate", "/api/publish", "/api/preview/start", "/api/preview/stop", "/api/screen-register/add"];

/// Routes one request against a fresh connection opened on `root`.
/// `path` excludes the query string; `query` is the raw, still
/// percent-encoded `key=value&...` tail; `body` is the raw request
/// body, form-encoded the same way `query` is (`body_param` decodes it
/// with the exact same `query_param`/`percent_decode` this module
/// already uses for `?key=value`) -- one encoding, read from whichever
/// side of the request a given route needs it.
pub fn handle(root: &Path, method: &str, path: &str, query: &str, body: &[u8]) -> ApiResponse {
    let is_mutating = MUTATING_PATHS.contains(&path) || path == "/api/graph/call";
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
    if path.starts_with("/api/graph/") || crate::graph_transport::is_typed(root) {
        return typed_graph_route(root, method, path, query, body);
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
        "/api/screens" => match list_screens(&conn) {
            Ok(screens) => ApiResponse::json(&screens),
            Err(e) => ApiResponse::error(500, &e),
        },
        "/api/screen-register" => handle_screen_register_get(root),
        "/api/screen-register/add" => handle_screen_register_add(root, &conn, body),
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
        "/api/git" => handle_git(root, query),
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
            let path = crate::hi_preview::path();
            ApiResponse::json(&serde_json::json!({ "running": port.is_some(), "port": port, "path": path }))
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
/// **Real trust-indicator wiring (2026-09-15) -- previously the
/// disclosed gap `pack_signer_identity`'s own doc comment named ("not
/// wired in yet") and `agent-skills/nirdosha/hi_ux_redesign_options.md`'s
/// "trust indicator (who signed it...)" mockup asked for.** Every
/// installed pack now carries its real `signer_identity` (from
/// `hi_plugin::pack_signer_identity`, `None` for a plain 5a unsigned
/// install -- this column's own migration default) and a derived
/// `trust_indicator` ("signed" / "unsigned") a caller can render
/// directly, no client-side inference needed. **Real, disclosed scope
/// cut, not the RFC 0016 mockup's full richness**: this exposes *who*
/// signed a pack, not *how* (`trust_anchor` vs `tofu` -- that
/// distinction currently lives only in the local append-only signing
/// log `append_pack_signing_log` writes, which has no reader yet; a
/// real, separate follow-up, not silently assumed done here). The live
/// Fulcio/OIDC-issued-identity half (RFC 0016's own "registry-governed"
/// trust tier) stays genuinely blocked on the registry-governance
/// question this doc's own position hasn't changed
/// (`rfcs/0016-implementation-plan.md`'s "Blocked" section) -- this is
/// the local-signing-only half that was never actually blocked on it.
fn handle_packs_list(conn: &Connection) -> ApiResponse {
    let installed = match crate::hi_plugin::installed_pack_ids(conn) {
        Ok(ids) => ids,
        Err(e) => return ApiResponse::error(500, &e),
    };
    let packs: Vec<serde_json::Value> = crate::hi_plugin::known_installable_packs()
        .into_iter()
        .map(|(id, description, _bytes)| {
            let is_installed = installed.contains(&id.to_string());
            let signer_identity = if is_installed { crate::hi_plugin::pack_signer_identity(conn, id).unwrap_or(None) } else { None };
            let trust_indicator = if signer_identity.is_some() { "signed" } else { "unsigned" };
            serde_json::json!({
                "id": id,
                "description": description,
                "installed": is_installed,
                "signer_identity": signer_identity,
                "trust_indicator": trust_indicator,
            })
        })
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

/// Locates the project's screen register: `screens.toml` at the sync
/// root, falling back to `screen.toml` (the singular spelling some
/// projects use). `None` when neither exists -- the + Screen rail then
/// only offers the free-text describe flow, never a half-working form.
fn screen_register_path(root: &Path) -> Option<std::path::PathBuf> {
    for name in ["screens.toml", "screen.toml"] {
        let p = root.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// GET /api/screen-register -- everything the + Screen rail's register
/// form needs to render itself from the project's own register: the
/// module/archetype/stage/role/file vocabularies *as that register
/// spells them*, the existing ids (uniqueness), and a next-id
/// suggestion per module. A project without a register answers
/// `{exists: false}` (200, not an error) so the UI can degrade to the
/// describe-only rail instead of showing a form that can't validate.
fn handle_screen_register_get(root: &Path) -> ApiResponse {
    let Some(path) = screen_register_path(root) else {
        return ApiResponse::json(&serde_json::json!({ "exists": false }));
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => return ApiResponse::error(500, &format!("reading {}: {e}", path.display())),
    };
    let doc: toml::Value = match text.parse() {
        Ok(d) => d,
        Err(e) => return ApiResponse::error(500, &format!("{} did not parse as TOML: {e}", path.display())),
    };
    let screens = doc.get("screen").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let mut modules: Vec<String> = Vec::new();
    let mut archetypes: Vec<String> = Vec::new();
    let mut stages: Vec<String> = Vec::new();
    let mut role_names: Vec<String> = Vec::new();
    let mut files: Vec<String> = Vec::new();
    let mut ids: Vec<String> = Vec::new();
    for s in &screens {
        if let Some(m) = s.get("module").and_then(|v| v.as_str()) {
            if !modules.contains(&m.to_string()) {
                modules.push(m.to_string());
            }
        }
        if let Some(a) = s.get("archetype").and_then(|v| v.as_str()) {
            if !archetypes.contains(&a.to_string()) {
                archetypes.push(a.to_string());
            }
        }
        if let Some(st) = s.get("stage").and_then(|v| v.as_str()) {
            if !stages.contains(&st.to_string()) {
                stages.push(st.to_string());
            }
        }
        if let Some(rs) = s.get("roles").and_then(|v| v.as_array()) {
            for r in rs.iter().filter_map(|r| r.as_str()) {
                if let Some((name, _mode)) = r.split_once(':') {
                    if !role_names.iter().any(|n| n == name) {
                        role_names.push(name.to_string());
                    }
                }
            }
        }
        if let Some(f) = s.get("file").and_then(|v| v.as_str()) {
            if !files.contains(&f.to_string()) {
                files.push(f.to_string());
            }
        }
        if let Some(id) = s.get("id").and_then(|v| v.as_str()) {
            ids.push(id.to_string());
        }
    }
    modules.sort();
    archetypes.sort();
    role_names.sort();
    files.sort();
    // next-id per module: ids are "<module-serial>.<seq>" (module M4 owns
    // the "4." prefix), so the suggestion is max-seen-seq + 1 -- the
    // register's own numbering, extended, never a renumbering of it.
    let mut next_ids = serde_json::Map::new();
    for m in &modules {
        let Some(serial) = m.strip_prefix('M') else { continue };
        if serial.is_empty() || !serial.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let prefix = format!("{serial}.");
        let max_seq = ids
            .iter()
            .filter_map(|id| id.strip_prefix(&prefix))
            .filter_map(|seq| seq.parse::<u64>().ok())
            .max()
            .unwrap_or(0);
        next_ids.insert(m.clone(), serde_json::json!(format!("{prefix}{}", max_seq + 1)));
    }
    ApiResponse::json(&serde_json::json!({
        "exists": true,
        "path": path.file_name().and_then(|p| p.to_str()).unwrap_or("screens.toml"),
        "metadata": doc.get("metadata"),
        "modules": modules,
        "archetypes": archetypes,
        "stages": stages,
        "roles": role_names,
        "files": files,
        "ids": ids,
        "next_ids": next_ids,
    }))
}

/// POST /api/screen-register/add -- validates a structured screen entry
/// against the project's own register vocabulary and, on success, (1)
/// appends the `[[screen]]` block to that register (bumping
/// `[metadata].total_screens`) and (2) records a reviewable `screen`
/// candidate in the hi graph, so the entry flows through the ordinary
/// confirm -> generate loop instead of silently claiming to exist.
/// Every failure is a 400 with a per-field error map, never a partial
/// write: the register file is only touched when the whole entry
/// validates.
fn handle_screen_register_add(root: &Path, conn: &Connection, body: &[u8]) -> ApiResponse {
    let Some(path) = screen_register_path(root) else {
        return ApiResponse::error(400, "no screens.toml / screen.toml in this project -- a register entry needs a register");
    };
    let Some(name) = body_param(body, "name").filter(|s| !s.trim().is_empty()) else {
        return ApiResponse::error(400, "missing required body param `name`");
    };
    let name = name.trim().to_string();
    let Some(id) = body_param(body, "id").filter(|s| !s.trim().is_empty()) else {
        return ApiResponse::error(400, "missing required body param `id`");
    };
    let id = id.trim().to_string();
    let module = body_param(body, "module").unwrap_or_default();
    let archetype = body_param(body, "archetype").unwrap_or_default();
    let stage_in = body_param(body, "stage").unwrap_or_default();
    let roles_in = body_param(body, "roles").unwrap_or_default();
    let route = body_param(body, "path").unwrap_or_default();
    let file = body_param(body, "file").unwrap_or_default();
    let notes = body_param(body, "notes").unwrap_or_default();
    let datasets_in = body_param(body, "datasets").unwrap_or_default();
    let blocked_by_in = body_param(body, "blocked_by").unwrap_or_default();

    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => return ApiResponse::error(500, &format!("reading {}: {e}", path.display())),
    };
    let doc: toml::Value = match text.parse() {
        Ok(d) => d,
        Err(e) => return ApiResponse::error(500, &format!("{} did not parse as TOML: {e}", path.display())),
    };
    let screens = doc.get("screen").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let existing_ids: Vec<String> = screens.iter().filter_map(|s| s.get("id").and_then(|v| v.as_str()).map(str::to_string)).collect();
    let modules: Vec<String> = screens.iter().filter_map(|s| s.get("module").and_then(|v| v.as_str()).map(str::to_string)).collect();
    let archetypes: Vec<String> = screens.iter().filter_map(|s| s.get("archetype").and_then(|v| v.as_str()).map(str::to_string)).collect();
    let files: Vec<String> = screens.iter().filter_map(|s| s.get("file").and_then(|v| v.as_str()).map(str::to_string)).collect();
    let stage_vocab: Vec<String> = screens.iter().filter_map(|s| s.get("stage").and_then(|v| v.as_str()).map(str::to_string)).collect();

    // Validation -- every check appends to a per-field map so the form
    // can mark exactly what's wrong, in one round trip.
    let mut fields = serde_json::Map::new();
    let name_ok = name.len() <= 120;
    if !name_ok {
        fields.insert("name".into(), serde_json::json!("keep the name under 120 characters"));
    }
    let id_parts: Vec<&str> = id.split('.').collect();
    let id_shape_ok = id_parts.len() == 2 && id_parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    if !id_shape_ok {
        fields.insert("id".to_string(), serde_json::json!("use the register's <module-serial>.<seq> shape, e.g. 4.14"));
    } else if existing_ids.iter().any(|existing| existing == &id) {
        fields.insert("id".to_string(), serde_json::json!(format!("screen {id} already exists in the register")));
    }
    if !modules.iter().any(|m| m == &module) {
        fields.insert("module".to_string(), serde_json::json!(format!("unknown module '{module}' -- pick one the register already uses")));
    }
    if !archetypes.iter().any(|a| a == &archetype) {
        fields.insert("archetype".to_string(), serde_json::json!(format!("unknown archetype '{archetype}' -- pick one the register uses")));
    }
    let roles: Vec<String> = roles_in.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    if roles.is_empty() {
        fields.insert("roles".to_string(), serde_json::json!("pick at least one role"));
    } else {
        let mut bad = Vec::new();
        for r in &roles {
            let ok = match r.split_once(':') {
                Some((role, mode)) if !role.is_empty() && matches!(mode, "R" | "R/W" | "W") => true,
                _ => false,
            };
            if !ok {
                bad.push(r.clone());
            }
        }
        if !bad.is_empty() {
            fields.insert("roles".to_string(), serde_json::json!(format!("roles must be `Name:R`, `Name:R/W` or `Name:W` -- got {}", bad.join(", "))));
        }
    }
    let route = route.trim().to_string();
    if !route.is_empty() {
        let chars_ok = route.starts_with('/')
            && !route.contains("//")
            && route
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '/' | '-' | '{' | '}' | '_'));
        let params_ok = route.split('/').filter(|s| s.starts_with('{')).all(|s| s.ends_with('}'));
        if !chars_ok || !params_ok {
            fields.insert("path".to_string(), serde_json::json!("routes are kebab-case with {param} placeholders, e.g. /holds/{id}"));
        }
    }
    let file = file.trim().to_string();
    if !file.is_empty() && !files.iter().any(|f| f == &file) {
        fields.insert("file".to_string(), serde_json::json!("the file must be one the register already tracks"));
    }
    let stage = if stage_in.trim().is_empty() {
        // The register's own convention: a declared-but-unwritten screen
        // is `emittable`, unless something blocks it.
        if blocked_by_in.trim().is_empty() { "emittable".to_string() } else { "blocked".to_string() }
    } else {
        let s = stage_in.trim().to_string();
        if !stage_vocab.iter().any(|st| st == &s) {
            fields.insert("stage".to_string(), serde_json::json!(format!("unknown stage '{s}'")));
        }
        s
    };
    let split_list = |raw: &str| -> Vec<String> { raw.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect() };
    let datasets = split_list(&datasets_in);
    let blocked_by = split_list(&blocked_by_in);
    if !fields.is_empty() {
        let body = serde_json::json!({ "error": "invalid screen entry", "fields": fields });
        return ApiResponse { status: 400, content_type: "application/json", body: serde_json::to_vec(&body).unwrap_or_default() };
    }

    // The graph half: a reviewable candidate node (unconfirmed, driving
    // text = the structured entry), so the screen enters the ordinary
    // review -> confirm -> generate loop instead of pretending the code
    // already exists. A name that would overwrite a REAL synced unit is
    // refused -- candidates never get to clobber synced knowledge. This
    // check runs BEFORE the register write so a refused entry leaves
    // every artifact (file and graph) untouched.
    let graph_name: String = {
        let mut out = String::new();
        for part in name.split(|c: char| !c.is_ascii_alphanumeric()).filter(|p| !p.is_empty()) {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => {
                    out.extend(first.to_uppercase());
                    out.push_str(&chars.as_str().to_lowercase());
                }
                None => {}
            }
        }
        if out.is_empty() { "NewScreen".to_string() } else { out }
    };
    let node_id = crate::hi_graph::code_unit_node_id("screen", &graph_name);
    let synced: Option<String> = conn
        .query_row("SELECT content_hash FROM nodes WHERE id = ?1", [&node_id], |r| r.get::<_, Option<String>>(0))
        .optional()
        .map_err(|e| format!("checking node {node_id}: {e}"))
        .unwrap_or(None)
        .flatten();
    if synced.is_some() {
        let body = serde_json::json!({
            "error": format!("the graph already has a synced code unit named `{graph_name}` -- tweak the name so the candidate is a distinct node"),
        });
        return ApiResponse { status: 400, content_type: "application/json", body: serde_json::to_vec(&body).unwrap_or_default() };
    }

    // The register entry, escaped for TOML (values are single-line;
    // backslash and double-quote are the only escapes a plain string
    // needs here).
    fn toml_escape(s: &str) -> String {
        s.replace('\\', "\\\\").replace('"', "\\\"")
    }
    fn toml_str_array(items: &[String]) -> String {
        format!("[{}]", items.iter().map(|i| format!("\"{}\"", toml_escape(i))).collect::<Vec<_>>().join(", "))
    }
    let mut block = String::new();
    block.push_str("\n[[screen]]\n");
    block.push_str(&format!("id = \"{}\"\n", toml_escape(&id)));
    block.push_str(&format!("name = \"{}\"\n", toml_escape(&name)));
    block.push_str(&format!("module = \"{}\"\n", toml_escape(&module)));
    block.push_str(&format!("archetype = \"{}\"\n", toml_escape(&archetype)));
    block.push_str("combo = []\n");
    block.push_str(&format!("roles = {}\n", toml_str_array(&roles)));
    if !datasets.is_empty() {
        block.push_str(&format!("datasets = {}\n", toml_str_array(&datasets)));
    }
    if !blocked_by.is_empty() {
        block.push_str(&format!("blocked_by = {}\n", toml_str_array(&blocked_by)));
    }
    block.push_str(&format!("stage = \"{}\"\n", toml_escape(&stage)));
    if !file.is_empty() {
        block.push_str(&format!("file = \"{}\"\n", toml_escape(&file)));
    }
    if !notes.trim().is_empty() {
        block.push_str(&format!("notes = \"{}\"\n", toml_escape(notes.trim())));
    }
    // The route is deliberately NOT written into the register: routes
    // live in menus.toml (its V6 invariant tracks route uniqueness), and
    // inventing a schema key here would be quiet drift. The route still
    // reaches the graph through the candidate's driving text.

    // total_screens is the register's own declared count -- bump it when
    // present, so the register never disagrees with itself.
    let mut updated = text;
    if let Some(n) = doc
        .get("metadata")
        .and_then(|m| m.get("total_screens"))
        .and_then(|v| v.as_integer())
    {
        let old_line = format!("total_screens = {n}");
        let new_line = format!("total_screens = {}", n + 1);
        updated = updated.replacen(&old_line, &new_line, 1);
    }
    updated.push_str(&block);
    if let Err(e) = std::fs::write(&path, updated) {
        return ApiResponse::error(500, &format!("writing {}: {e}", path.display()));
    }
    let mut driving = format!(
        "Screen {id} ({module}) -- {name}. Archetype: {archetype}. Stage: {}.",
        if stage.is_empty() { "emittable" } else { &stage }
    );
    if !roles.is_empty() {
        driving.push_str(&format!(" Roles: {}.", roles.join(", ")));
    }
    if !route.is_empty() {
        driving.push_str(&format!(" Route: {route}."));
    }
    if !file.is_empty() {
        driving.push_str(&format!(" File: {file}."));
    }
    if !notes.trim().is_empty() {
        driving.push_str(&format!(" Notes: {}.", notes.trim()));
    }
    let node = match crate::hi_graph::add_candidate(conn, "screen", &graph_name, &driving, "screen-register-form") {
        Ok(n) => n,
        Err(e) => return ApiResponse::error(500, &e),
    };
    ApiResponse::json(&serde_json::json!({
        "ok": true,
        "node": node,
        "screen_id": id,
        "register_path": path.display().to_string(),
        "appended_toml": block.trim_end().to_string(),
    }))
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

/// `hi`'s own revision history, read-only: `?op=log` (the default) for
/// the commit list `hi_revision::log` returns, `?op=diff&rev=<hash>`
/// for one commit's own diff. GET-only, deliberately never in
/// `MUTATING_PATHS` -- a revert/checkout-style mutation is real,
/// separate danger (overwriting an uncommitted draft) this first pass
/// doesn't attempt (`hi_revision.rs`'s own module doc comment).
fn handle_git(root: &Path, query: &str) -> ApiResponse {
    match query_param(query, "op").as_deref() {
        Some("diff") => match query_param(query, "rev") {
            Some(rev) => match crate::hi_revision::diff(root, &rev) {
                Ok(text) => ApiResponse::json(&serde_json::json!({ "ok": true, "diff": text })),
                Err(e) => ApiResponse::error(400, &e),
            },
            None => ApiResponse::error(400, "?op=diff needs a `rev` query param"),
        },
        None | Some("log") => match crate::hi_revision::log(root, 50) {
            Ok(entries) => ApiResponse::json(&serde_json::json!({ "ok": true, "log": entries })),
            Err(e) => ApiResponse::error(500, &e),
        },
        Some(other) => ApiResponse::error(400, &format!("unknown ?op={other} -- expected `log` (default) or `diff`")),
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
    // Best-effort: a git problem (no git binary, a detached HEAD, ...)
    // degrades to a logged warning in the response, never blocks a
    // successful generate from being reported as one -- same "must
    // never be the thing that crashes" posture `hi_graph.rs` already
    // holds itself to for `sync`.
    let revision = match crate::hi_revision::commit_revision(root, &format!("generate: {} unit(s)", ids.len())) {
        Ok(hash) => hash,
        Err(e) => {
            log_lines.push(format!("warning: could not commit this generate to git: {e}"));
            None
        }
    };
    ApiResponse::json(&serde_json::json!({ "ok": true, "path": path.display().to_string(), "locked": locked, "not_declared": not_declared, "revision": revision, "log": log_lines }))
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
fn handle_publish(root: &Path, _conn: &Connection) -> ApiResponse {
    let source_path = crate::hi_llm::generated_source_path(root);
    if !source_path.exists() {
        return ApiResponse::error(400, "nothing generated yet -- run :generate first");
    }
    let result: Result<(std::path::PathBuf, std::path::PathBuf, bool), String> = (|| {
        let publish = crate::v2_verify::publish_project(root, &source_path)?;
        let signing_key_path = std::env::var(PUBLISH_SIGNING_KEY_VAR).ok();
        let signed = if let Some(key_path) = &signing_key_path {
            let cert_bytes = std::fs::read(&publish.certificate_path).map_err(|e| format!("reading {} to sign it: {e}", publish.certificate_path.display()))?;
            let (public_key, signature) = nirdosha_audit::signing::sign_bytes(&cert_bytes, key_path)?;
            // Preserve the exact certificate emitted by publish_project,
            // including its UI assurance proof. Reconstructing the payload
            // from only the source/verdict here would silently discard that
            // evidence before signing it.
            let certificate: serde_json::Value = serde_json::from_slice(&cert_bytes)
                .map_err(|e| format!("parsing {} before signing it: {e}", publish.certificate_path.display()))?;
            let signed_envelope = serde_json::json!({
                "certificate": certificate,
                "signature_algorithm": "ed25519",
                "public_key": public_key,
                "signature": signature,
            });
            std::fs::write(&publish.certificate_path, serde_json::to_string_pretty(&signed_envelope).expect("this JSON value always serializes"))
                .map_err(|e| format!("writing {}: {e}", publish.certificate_path.display()))?;
            true
        } else {
            false
        };
        Ok((publish.binary_path, publish.certificate_path, signed))
    })();
    match result {
        Ok((out_path, cert_path, signed)) => {
            let revision = crate::hi_revision::commit_revision(root, &format!("publish: {}", out_path.display())).ok().flatten();
            ApiResponse::json(&serde_json::json!({ "ok": true, "binary": out_path.display().to_string(), "certificate": cert_path.display().to_string(), "signed": signed, "revision": revision }))
        }
        Err(e) => ApiResponse::json(&serde_json::json!({ "ok": false, "error": e })),
    }
}

/// Github #45's "ship now" half: build a real, `cargo`-produced v2
/// binary (`v2_verify::preview_start`, which itself calls `v2_verify::
/// build_project`) and run it via `hi_preview::restart` -- "preview"
/// here means "run the real thing," not a mockup. Authentication comes
/// from the generated app; the preview API reports its first UI route.
fn handle_preview_start(root: &Path, _conn: &Connection) -> ApiResponse {
    let source_path = crate::hi_llm::generated_source_path(root);
    if !source_path.exists() {
        return ApiResponse::error(400, "nothing generated yet -- run :generate first");
    }
    match crate::v2_verify::preview_start(root, &source_path) {
        Ok((port, path)) => ApiResponse::json(&serde_json::json!({ "ok": true, "port": port, "path": path })),
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
    /// Which UI archetype this `screen`-kind node represents (crud,
    /// dashboard, login, shell, ...). `None` for non-screen nodes.
    screen_type: Option<String>,
    /// The route path for screen-kind nodes (`path: "/tasks"`). `None`
    /// for non-screen nodes or screens that do not declare a path.
    screen_path: Option<String>,
    /// For `app_shell`-kind nodes, the JSON nav entries.
    screen_nav: Option<String>,
}

fn list_nodes(conn: &Connection) -> Result<Vec<NodeRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, kind, title, status, content_hash, source_ref, line, col, driving_text, created_by, confirmed, locked, waived, waive_reason, attributes, plugin_origin, non_waivable, screen_type, screen_path, screen_nav FROM nodes LIMIT ?1",
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
                screen_type: r.get(17)?,
                screen_path: r.get(18)?,
                screen_nav: r.get(19)?,
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

#[derive(Serialize)]
struct ScreenRow {
    id: String,
    name: String,
    screen_type: String,
    screen_path: Option<String>,
    screen_nav: Option<String>,
}

#[derive(Serialize, Debug)]
struct NavigationRow {
    source_id: String,
    target_id: String,
    label: String,
    inferred: bool,
}

#[derive(Serialize)]
struct ScreenMap {
    screens: Vec<ScreenRow>,
    navigations: Vec<NavigationRow>,
}

/// Screen-only graph payload: every `screen`-kind CodeUnit plus the
/// screen-to-screen navigation edges (RFC 0022 §2).
///
/// Edges come from two tiers, in this order:
/// 1. **Derived `NAVIGATES_TO` edges in the graph itself** (`hi sync`'s
///    reverse-engineering pass: `menus.toml` nav/landing entries, static
///    redirects, approval-inbox detail paths) -- these are `inferred:
///    false` because they were read out of real code/registers, not
///    guessed.
/// 2. **Fallback edges, only where a screen would otherwise be
///    unreachable**: a synthetic shell fan-out ("app shell") to every
///    screen no derived edge reaches, and -- only when the project has
///    no derived nav edges at all -- the legacy synthetic login→landing
///    and shell→login pairs, so a screen map is never disconnected
///    even for a project that has no nav register yet.
fn list_screens(conn: &Connection) -> Result<ScreenMap, String> {
    let mut stmt = conn
        .prepare("SELECT id, title, screen_type, screen_path, screen_nav FROM nodes WHERE kind = 'CodeUnit' AND screen_type IS NOT NULL LIMIT ?1")
        .map_err(|e| format!("preparing screen listing: {e}"))?;
    let rows = stmt
        .query_map([MAX_ROWS], |r| {
            Ok(ScreenRow {
                id: r.get(0)?,
                name: r.get(1)?,
                screen_type: r.get(2)?,
                screen_path: r.get(3)?,
                screen_nav: r.get(4)?,
            })
        })
        .map_err(|e| format!("listing screens: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("reading screen row: {e}"))?;

    // Derived nav edges straight from the graph -- the real
    // reverse-engineered navigation, exactly what `hi sync` last
    // read out of the app's own routes and nav register.
    let mut nav_stmt = conn
        .prepare(
            "SELECT e.src, e.dst, COALESCE(e.label, e.kind) FROM edges e
             JOIN nodes a ON a.id = e.src AND a.kind = 'CodeUnit' AND a.screen_type IS NOT NULL
             JOIN nodes b ON b.id = e.dst AND b.kind = 'CodeUnit' AND b.screen_type IS NOT NULL
             WHERE e.kind = 'NAVIGATES_TO' LIMIT ?1",
        )
        .map_err(|e| format!("preparing nav edge listing: {e}"))?;
    let derived: Vec<NavigationRow> = nav_stmt
        .query_map([MAX_ROWS], |r| {
            Ok(NavigationRow { source_id: r.get(0)?, target_id: r.get(1)?, label: r.get(2)?, inferred: false })
        })
        .map_err(|e| format!("listing nav edges: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("reading nav edge row: {e}"))?;
    drop(nav_stmt);

    let has_derived = !derived.is_empty();
    let mut navigations = derived;

    // Build a path -> screen lookup for inferred nav edges.
    let path_to_id: std::collections::HashMap<String, String> = rows
        .iter()
        .filter_map(|s| s.screen_path.as_ref().map(|p| (p.clone(), s.id.clone())))
        .collect();

    let mut shell_id = None;
    let mut shell_nav_entries: Vec<NavEntryJson> = Vec::new();

    for screen in &rows {
        if screen.screen_type == "shell" {
            shell_id = Some(screen.id.clone());
            if let Some(json) = &screen.screen_nav {
                if let Ok(entries) = serde_json::from_str::<Vec<NavEntryJson>>(json) {
                    shell_nav_entries = entries;
                }
            }
        }
    }

    if !has_derived {
        // No derived edges at all (a project with no menus.toml, no
        // redirects, no inboxes): the legacy synthetic layer keeps the
        // map connected exactly as before this pass existed.
        if let Some(shell) = &shell_id {
            for entry in &shell_nav_entries {
                if let Some(target) = path_to_id.get(&entry.href) {
                    navigations.push(NavigationRow {
                        source_id: shell.clone(),
                        target_id: target.clone(),
                        label: entry.label.clone(),
                        inferred: false,
                    });
                }
            }
            // Fallback: connect the shell to every screen so the map is
            // never disconnected, even when paths do not match.
            for screen in &rows {
                if screen.id != *shell {
                    navigations.push(NavigationRow {
                        source_id: shell.clone(),
                        target_id: screen.id.clone(),
                        label: "app shell".to_string(),
                        inferred: true,
                    });
                }
            }
        }
        let login = rows.iter().find(|s| s.screen_type == "login");
        let landing = rows.iter().find(|s| s.screen_type == "landing");
        if let (Some(login), Some(landing), Some(shell)) = (login, landing, shell_id) {
            navigations.push(NavigationRow {
                source_id: login.id.clone(),
                target_id: landing.id.clone(),
                label: "post-login landing".to_string(),
                inferred: true,
            });
            navigations.push(NavigationRow {
                source_id: shell.clone(),
                target_id: login.id.clone(),
                label: "login".to_string(),
                inferred: true,
            });
        }
    } else {
        // The map's connectivity guarantee, downgraded from "every
        // screen is a shell child" to "no screen is orphaned": once
        // real derived edges exist, only screens NO edge reaches get
        // the synthetic shell link -- a screen reached by a real menu
        // entry must not also carry a fake "app shell" edge implying
        // the shell links to it directly.
        let reached: std::collections::HashSet<String> = navigations.iter().map(|n| n.target_id.clone()).collect();
        if let Some(shell) = &shell_id {
            for screen in &rows {
                if screen.id != *shell && !reached.contains(&screen.id) {
                    navigations.push(NavigationRow {
                        source_id: shell.clone(),
                        target_id: screen.id.clone(),
                        label: "app shell".to_string(),
                        inferred: true,
                    });
                }
            }
        }
    }

    Ok(ScreenMap { screens: rows, navigations })
}

#[derive(serde::Deserialize)]
struct NavEntryJson {
    label: String,
    href: String,
    #[allow(dead_code)]
    role: Option<String>,
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

    /// RFC 0022 §2's /api/screens contract, two tiers: derived
    /// NAVIGATES_TO edges (real labels, inferred:false) come straight
    /// from the graph, and the synthetic shell fan-out now applies only
    /// to screens no real edge reaches -- a screen reached by a real
    /// menu entry must not ALSO carry a fake "app shell" edge.
    #[test]
    fn list_screens_serves_derived_nav_edges_and_falls_back_only_for_orphans() {
        let dir = scratch_dir("list_screens_nav");
        let conn = crate::hi_graph::open(&dir).expect("open");
        for (id, title, st, path) in [
            ("code:screen:app_shell", "Shell", "shell", None::<String>),
            ("code:screen:my_day", "My Day", "dashboard", Some("/my-day".to_string())),
            ("code:screen:orph", "Orphan", "page", Some("/orphan".to_string())),
        ] {
            conn.execute(
                "INSERT INTO nodes (id, kind, title, screen_type, screen_path) VALUES (?1, 'CodeUnit', ?2, ?3, ?4)",
                rusqlite::params![id, title, st, path],
            )
            .expect("insert screen");
        }
        conn.execute(
            "INSERT INTO edges (src, dst, kind, label) VALUES ('code:screen:app_shell', 'code:screen:my_day', 'NAVIGATES_TO', 'nav.day')",
            [],
        )
        .expect("insert nav edge");
        let map = list_screens(&conn).expect("list_screens");
        assert_eq!(map.screens.len(), 3);
        let real: Vec<_> = map.navigations.iter().filter(|n| !n.inferred).collect();
        let nav_debug = format!("{:?}", map.navigations);
        assert_eq!(real.len(), 1, "exactly one derived edge: {nav_debug}");
        assert_eq!(real[0].source_id, "code:screen:app_shell");
        assert_eq!(real[0].target_id, "code:screen:my_day");
        assert_eq!(real[0].label, "nav.day");
        // The orphan page still gets its synthetic shell link; the
        // reached screen does not.
        let orphan_fallback: Vec<_> = map
            .navigations
            .iter()
            .filter(|n| n.inferred && n.target_id == "code:screen:orph")
            .collect();
        assert_eq!(orphan_fallback.len(), 1, "orphan screen gets the synthetic shell edge");
        let myday_fallback: Vec<_> = map
            .navigations
            .iter()
            .filter(|n| n.inferred && n.target_id == "code:screen:my_day")
            .collect();
        assert!(myday_fallback.is_empty(), "a screen reached by a real edge must not also get a synthetic one");
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
        // `ensure_default_packs` installs banking-v0 as a plain 5a
        // unsigned install -- the real trust-indicator wiring must say
        // so honestly, not claim a signer that was never recorded.
        assert_eq!(banking["signer_identity"], serde_json::Value::Null, "an unsigned 5a install must not claim a signer: {banking:?}");
        assert_eq!(banking["trust_indicator"], "unsigned", "an unsigned 5a install's trust indicator must say so: {banking:?}");
        // A pack never installed at all has nothing to report either --
        // same "not applicable" shape as an unsigned one, not an error.
        assert_eq!(fapi["signer_identity"], serde_json::Value::Null, "{fapi:?}");
        assert_eq!(fapi["trust_indicator"], "unsigned", "{fapi:?}");
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

    /// The native-pipeline tests this replaced (coverage-gate re-check,
    /// certificate `governing_packs`/`governing_invariants`/
    /// `compliance_profiles`/`nfr_commitments` attribution) tested
    /// `handle_publish`'s old `loader::load_program` + native `Certificate`
    /// pipeline, which no longer exists in this crate at all (2026-09-16,
    /// the `hi` v2 extraction). These two exercise the real thing
    /// `handle_publish` does now: a real `cargo build --release` of the
    /// generated v2 source via `v2_verify::publish_project`. Real cargo
    /// builds, not free -- kept to two, not the native suite's dozen.
    #[test]
    fn publish_builds_a_real_v2_binary_and_writes_a_certificate() {
        let dir = scratch_dir("publish_v2_binary");
        crate::hi_graph::open(&dir).expect("open"); // scaffold .nir/ so /api/publish's own `hi_graph::open` succeeds
        let out_path = crate::hi_llm::generated_source_path(&dir);
        std::fs::create_dir_all(out_path.parent().unwrap()).expect("mkdir");
        std::fs::write(&out_path, "fn add(a: i64, b: i64) -> i64 {\n    a + b\n}\n\nfn main() {\n    println!(\"{}\", add(2, 3));\n}\n").expect("write");

        let resp = handle(&dir, "POST", "/api/publish", "", b"");
        let body = String::from_utf8_lossy(&resp.body);
        let json: serde_json::Value = serde_json::from_str(&body).expect("valid JSON: {body}");
        assert_eq!(json["ok"], serde_json::json!(true), "a valid v2 program must publish: {body}");
        assert_eq!(json["signed"], serde_json::json!(false), "no signing key configured in this test");

        let binary_path = json["binary"].as_str().expect("binary path in the response");
        assert!(std::path::Path::new(binary_path).exists(), "the published binary must actually exist on disk");
        let cert_path = json["certificate"].as_str().expect("certificate path in the response");
        let cert: serde_json::Value = serde_json::from_slice(&std::fs::read(cert_path).expect("read certificate")).expect("valid JSON");
        assert_eq!(cert["certificate_version"], serde_json::json!("nirdosha.certificate/v2-source-scan"));
        assert_eq!(cert["builds"], serde_json::json!(true));
    }

    #[test]
    fn publish_refuses_a_v2_source_that_does_not_build() {
        let dir = scratch_dir("publish_v2_broken");
        crate::hi_graph::open(&dir).expect("open");
        let out_path = crate::hi_llm::generated_source_path(&dir);
        std::fs::create_dir_all(out_path.parent().unwrap()).expect("mkdir");
        std::fs::write(&out_path, "fn main() {\n    this is not valid rust\n}\n").expect("write");

        let resp = handle(&dir, "POST", "/api/publish", "", b"");
        let body = String::from_utf8_lossy(&resp.body);
        let json: serde_json::Value = serde_json::from_str(&body).expect("valid JSON: {body}");
        assert_eq!(json["ok"], serde_json::json!(false), "a source that doesn't build must refuse to publish: {body}");
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
        // from that same file, via `v2_verify::preview_start`.
        let dir = scratch_dir("preview_nothing_generated");
        crate::hi_graph::open(&dir).expect("open");
        let resp = handle(&dir, "POST", "/api/preview/start", "", b"");
        assert_eq!(resp.status, 400);
        assert!(String::from_utf8_lossy(&resp.body).contains("nothing generated"));
    }

    #[test]
    fn preview_start_builds_and_runs_a_real_server_then_stop_tears_it_down() {
        // github #45's "ship now" half, end to end: a real v2 binary
        // (`v2_verify::build_project`, real `cargo build --release`),
        // actually spawned by `hi_preview::restart`. Ensures
        // `hi_preview::stop()` afterward regardless of how the
        // assertions below turn out, so this test never leaves a real
        // child process (and a bound TCP port) behind for the rest of
        // the test binary's run.
        let _g = crate::hi_preview::PREVIEW_TESTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = scratch_dir("preview_start_real_server");
        crate::hi_graph::open(&dir).expect("open");
        let out_path = crate::hi_llm::generated_source_path(&dir);
        std::fs::create_dir_all(out_path.parent().unwrap()).expect("mkdir");
        std::fs::write(&out_path, "fn count_tasks() -> i64 { 1 }\nnirdosha_rt::dashboard! { mount: mount_TaskListScreen, path: \"/tasks\", title: \"Tasks\", refresh_seconds: 30, widgets { Metric { label: \"Tasks\", fn: count_tasks }, } }\nfn main() {\n    let router = nirdosha_rt::Router::new(|_| nirdosha_rt::Auth::login(\"anon\", &[]));\n    let router = mount_TaskListScreen(router);\n    router.serve(18099);\n}\n").expect("write");

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let resp = handle(&dir, "POST", "/api/preview/start", "", b"");
            let body = String::from_utf8_lossy(&resp.body).into_owned();
            let json: serde_json::Value = serde_json::from_str(&body).expect("valid JSON: {body}");
            assert_eq!(json["ok"], serde_json::json!(true), "a servable program must start a preview: {body}");
            let port = json["port"].as_u64().expect("a successful start reports a port");
            assert_ne!(port, 0);
            assert_eq!(json["path"], "/tasks");
            let page = reqwest::blocking::get(format!("http://127.0.0.1:{port}/tasks")).expect("preview screen responds");
            assert_eq!(page.status(), 200);

            let status_resp = handle(&dir, "GET", "/api/preview/status", "", b"");
            let status: serde_json::Value = serde_json::from_slice(&status_resp.body).expect("valid JSON");
            assert_eq!(status["running"], serde_json::json!(true));
            assert_eq!(status["port"].as_u64(), Some(port));
            assert_eq!(status["path"], "/tasks");
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

    /// The + Screen rail's register form is entirely driven by the
    /// project's own screens.toml: the GET must surface the register's
    /// vocabularies (modules/archetypes/stages/roles/files), the
    /// existing ids, and a next-id suggestion per module that extends
    /// the register's own numbering (M4's max "4.x" + 0.1).
    #[test]
    fn screen_register_get_surfaces_vocabularies_and_next_ids() {
        let dir = scratch_dir("screen_register_get");
        std::fs::write(
            dir.join("screens.toml"),
            "[metadata]\ntotal_screens = 3\n\n[[screen]]\nid = \"4.1\"\nname = \"Hold List\"\nmodule = \"M4\"\narchetype = \"crud_screens!\"\ncombo = []\nroles = [\"OpsAnalyst:R\", \"Admin:R\"]\nstage = \"built\"\n\n[[screen]]\nid = \"4.13\"\nname = \"Hold Detail\"\nmodule = \"M4\"\narchetype = \"dashboard!\"\ncombo = []\nroles = [\"OpsAnalyst:R\"]\nstage = \"emittable\"\n\n[[screen]]\nid = \"7.1\"\nname = \"Feed\"\nmodule = \"M7\"\narchetype = \"communication_feed!\"\ncombo = []\nroles = [\"CsAgent:R/W\"]\nstage = \"interim\"\n",
        )
        .unwrap();
        let resp = handle_screen_register_get(&dir);
        assert_eq!(resp.status, 200);
        let data: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
        assert_eq!(data["exists"], true);
        assert_eq!(data["metadata"]["total_screens"], 3);
        assert_eq!(data["modules"], serde_json::json!(["M4", "M7"]));
        assert_eq!(data["archetypes"][0], "communication_feed!");
        assert!(data["roles"].as_array().unwrap().contains(&serde_json::json!("OpsAnalyst")));
        assert_eq!(data["next_ids"]["M4"], "4.14");
        assert_eq!(data["next_ids"]["M7"], "7.2");
    }

    #[test]
    fn screen_register_get_reports_missing_register_as_exists_false() {
        let dir = scratch_dir("screen_register_missing");
        let resp = handle_screen_register_get(&dir);
        assert_eq!(resp.status, 200);
        let data: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
        assert_eq!(data["exists"], false);
    }

    /// The POST path end-to-end: a valid entry is appended verbatim to
    /// the register (total_screens bumped), AND a reviewable screen
    /// candidate appears in the graph with the structured driving text
    /// -- the entry enters the ordinary confirm -> generate loop, it
    /// never pretends the code exists.
    #[test]
    fn screen_register_add_appends_entry_and_records_candidate() {
        let dir = scratch_dir("screen_register_add");
        let conn = crate::hi_graph::open(&dir).expect("open");
        std::fs::write(
            dir.join("screens.toml"),
            "[metadata]\ntotal_screens = 1\n\n[[screen]]\nid = \"4.1\"\nname = \"Hold List\"\nmodule = \"M4\"\narchetype = \"crud_screens!\"\ncombo = []\nroles = [\"OpsAnalyst:R\"]\nstage = \"built\"\n",
        )
        .unwrap();
        let body = "name=Payment+Hold+Detail&id=4.14&module=M4&archetype=crud_screens%21&stage=&path=%2Fholds%2F%7Bid%7D&roles=OpsAnalyst%3AR%2CAdmin%3AR%2FW&file=&datasets=PG.payments&blocked_by=&notes=Second+hold+view";
        let resp = handle_screen_register_add(&dir, &conn, body.as_bytes());
        assert_eq!(resp.status, 200, "{}", String::from_utf8_lossy(&resp.body));
        let data: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
        assert_eq!(data["ok"], true);
        assert_eq!(data["node"], "code:screen:PaymentHoldDetail");
        assert_eq!(data["screen_id"], "4.14");

        let register = std::fs::read_to_string(dir.join("screens.toml")).unwrap();
        assert!(register.contains("total_screens = 2"), "count bumped: {register}");
        assert!(register.contains("[[screen]]\nid = \"4.14\""));
        assert!(register.contains("name = \"Payment Hold Detail\""));
        assert!(register.contains("roles = [\"OpsAnalyst:R\", \"Admin:R/W\"]"));
        assert!(register.contains("stage = \"emittable\""), "empty stage defaults to emittable");

        let candidate: Option<(String, Option<String>)> = conn
            .query_row(
                "SELECT driving_text, created_by FROM nodes WHERE id = 'code:screen:PaymentHoldDetail'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .unwrap();
        let (driving, created_by) = candidate.expect("candidate node recorded");
        assert_eq!(created_by.as_deref(), Some("screen-register-form"));
        assert!(driving.contains("Screen 4.14 (M4)"));
        assert!(driving.contains("Route: /holds/{id}"));
    }

    /// Rejections never touch the file: a duplicate id and a bad stage
    /// both come back as per-field errors with the register byte-identical.
    #[test]
    fn screen_register_add_rejects_without_writing_on_invalid_entry() {
        let dir = scratch_dir("screen_register_reject");
        let conn = crate::hi_graph::open(&dir).expect("open");
        let register = "[metadata]\ntotal_screens = 1\n\n[[screen]]\nid = \"4.1\"\nname = \"Hold List\"\nmodule = \"M4\"\narchetype = \"crud_screens!\"\ncombo = []\nroles = [\"OpsAnalyst:R\"]\nstage = \"built\"\n";
        std::fs::write(dir.join("screens.toml"), register).unwrap();

        let dup = handle_screen_register_add(&dir, &conn, b"name=Another&id=4.1&module=M4&archetype=crud_screens%21&roles=OpsAnalyst%3AR");
        assert_eq!(dup.status, 400);
        let data: serde_json::Value = serde_json::from_slice(&dup.body).unwrap();
        assert!(data["fields"]["id"].as_str().unwrap().contains("already exists"));

        let bad_stage = handle_screen_register_add(&dir, &conn, b"name=Another&id=4.2&module=M4&archetype=crud_screens%21&stage=shipped&roles=OpsAnalyst%3AR");
        assert_eq!(bad_stage.status, 400);
        let data: serde_json::Value = serde_json::from_slice(&bad_stage.body).unwrap();
        assert!(data["fields"]["stage"].as_str().unwrap().contains("unknown stage"));

        let no_roles = handle_screen_register_add(&dir, &conn, b"name=Another&id=4.2&module=M4&archetype=crud_screens%21&roles=");
        assert_eq!(no_roles.status, 400);
        let data: serde_json::Value = serde_json::from_slice(&no_roles.body).unwrap();
        assert!(data["fields"]["roles"].as_str().unwrap().contains("at least one role"));

        assert_eq!(std::fs::read_to_string(dir.join("screens.toml")).unwrap(), register, "register untouched on every rejection");
        let count: i64 = conn.query_row("SELECT count(*) FROM nodes WHERE id LIKE 'code:screen:%'", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 0, "no candidate recorded for a rejected entry");
    }

    /// A candidate must never overwrite a REAL synced unit: if the graph
    /// already holds a synced node under the derived name, the add is
    /// refused with a name-tweak hint (the register append also must not
    /// survive -- the entry is only written when the whole request
    /// validates, graph collision included).
    #[test]
    fn screen_register_add_refuses_name_collision_with_synced_unit() {
        let dir = scratch_dir("screen_register_collision");
        let conn = crate::hi_graph::open(&dir).expect("open");
        std::fs::write(
            dir.join("screens.toml"),
            "[metadata]\ntotal_screens = 1\n\n[[screen]]\nid = \"4.1\"\nname = \"Hold List\"\nmodule = \"M4\"\narchetype = \"crud_screens!\"\ncombo = []\nroles = [\"OpsAnalyst:R\"]\nstage = \"built\"\n",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO nodes (id, kind, title, content_hash) VALUES ('code:screen:PaymentHoldDetail', 'CodeUnit', 'Payment Hold Detail', 'abc123')",
            [],
        )
        .unwrap();
        let resp = handle_screen_register_add(&dir, &conn, b"name=Payment+Hold+Detail&id=4.14&module=M4&archetype=crud_screens%21&roles=OpsAnalyst%3AR");
        assert_eq!(resp.status, 400, "{}", String::from_utf8_lossy(&resp.body));
        let text = String::from_utf8_lossy(&resp.body);
        assert!(text.contains("synced code unit"), "error names the collision: {text}");
        let register = std::fs::read_to_string(dir.join("screens.toml")).unwrap();
        assert!(!register.contains("4.14"), "register untouched on collision: {register}");
    }
}

fn typed_graph_route(root: &Path, method: &str, path: &str, query: &str, body: &[u8]) -> ApiResponse {
    let result = (|| -> nirdosha_graph::Result<serde_json::Value> {
        let options=crate::graph_transport::Options::parse(std::iter::empty())?;
        let access=if method == "POST" { nirdosha_graph::store::Access::reviewer("local") } else { nirdosha_graph::store::Access::read("local") };
        let graph=nirdosha_graph::Graph::open(root,&options.state,access)?;
        match path {
            "/api/graph/head" => Ok(serde_json::json!({"graph":graph.version()?})),
            "/api/graph/page" => {
                let mut args=serde_json::json!({});
                for key in ["snapshot","cursor","filter_hash","view","entity_type","kind"] {
                    if let Some(value)=query_param(query,key) { args[key]=value.into(); }
                }
                graph.page(&args)
            },
            "/api/graph/call" => {
                let text=std::str::from_utf8(body).map_err(|_|nirdosha_graph::Error::new("SCHEMA_INVALID","Expected UTF-8 JSON"))?;
                let request=nirdosha_graph::hash::parse(text)?;
                nirdosha_graph::mcp::execute(&graph,request["name"].as_str().unwrap_or(""),&request["arguments"])
            },
            "/api/nodes"|"/api/edges" => {
                let kind=if path.ends_with("nodes") { "node" } else { "edge" };
                let collection=if kind=="node" { "nodes" } else { "edges" };
                let mut args=serde_json::json!({"entity_type":kind});let mut out=vec![];
                loop { let page=graph.page(&args)?;out.extend(page[collection].as_array().unwrap().clone());
                    if page["next_cursor"].is_null() { break; }
                    args["cursor"]=page["next_cursor"].clone();args["filter_hash"]=page["filter_hash"].clone();
                }
                Ok(serde_json::json!(out))
            },
            _=>Err(nirdosha_graph::Error::new("UNSUPPORTED_TARGET","This legacy action is unavailable on a typed graph; use the graph MCP tools")),
        }
    })();
    match result { Ok(value)=>ApiResponse::json(&value),Err(e)=>ApiResponse { status:if e.code=="PROJECT_NOT_INITIALIZED"||e.code=="MIGRATION_REQUIRED" {404}else{400},content_type:"application/json",body:e.envelope().to_string().into_bytes() } }
}
