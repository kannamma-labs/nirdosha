//! Nirdosha Realm (rfcs/0013-nirdosha-realm.md) -- a local-first,
//! bounded knowledge graph auto-scaffolded under `.nir/` on every
//! successful `nirdosha hi` startup (`hi::run_console`'s own
//! `open_realm_or_warn`). v1 slice: SQLite schema (`nodes`/`edges`/
//! `provenance`/`chunks`+FTS5), per-item code hashing reusing the same
//! lex/parse primitives `emit-ast`/`loader::load_program` are built
//! from, bounded bidirectional impact queries, manual requirement<->
//! code links, and a minimal document ingest/ask path. Never mandatory:
//! every entry point here degrades to "log a warning, keep going" for
//! its caller rather than ever being the thing that breaks `hi` itself
//! (the RFC's own "must run on an ordinary laptop, must never be the
//! thing that crashes" posture -- the same one `hi.rs`'s own
//! `SessionLog`/`FailureCounters` already follow for their own
//! diagnostics).
//!
//! Deliberately reuses `token::Lexer`/`parser::Parser::parse_program`
//! directly rather than `loader::load_program`: that function resolves
//! and merges every `use "..."` a file declares, so syncing a project
//! file-by-file through the merged view would record each imported
//! file's items once per importer instead of once, total. Parsing each
//! file on its own (no import resolution) gives exactly that file's own
//! declared items -- the same "AST of a program that doesn't yet
//! typecheck is still legitimate to inspect" contract `emit-ast` already
//! relies on (`main.rs::cmd_emit_ast`'s own doc comment).

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

/// `NIRDOSHA_REALM_DISABLE=1` skips the auto-scaffold and auto-sync
/// `hi` would otherwise run on startup -- same `NIRDOSHA_`-prefixed,
/// `.ok()`-based env-var convention RFC 0012's own activation trio
/// uses (`hi::PROVIDER_KEY_VAR` etc.).
pub const REALM_DISABLE_VAR: &str = "NIRDOSHA_REALM_DISABLE";

/// `env` is injected the same way `hi::resolve_activation` injects it
/// -- a pure, testable check, no direct `std::env::var` call baked in.
pub fn is_disabled(env: &dyn Fn(&str) -> Option<String>) -> bool {
    env(REALM_DISABLE_VAR).as_deref() == Some("1")
}

/// `.nir/` under `root` -- rfcs/0013's "Physical layout." Named `.nir`
/// (not `.realm`) per that RFC's own revision; see its Open Questions
/// for the acknowledged overlap with the `.nir` source-file extension.
pub fn realm_dir(root: &Path) -> PathBuf {
    root.join(".nir")
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// Creates `.nir/` and `.nir/content/` if absent, opens (or creates)
/// `.nir/realm.db`, and migrates the schema. Every migration statement
/// is `IF NOT EXISTS` -- safe to call on every `hi` startup, which is
/// exactly how it's used.
pub fn open(root: &Path) -> Result<Connection, String> {
    let dir = realm_dir(root);
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    std::fs::create_dir_all(dir.join("content")).map_err(|e| format!("creating {}: {e}", dir.join("content").display()))?;
    let db_path = dir.join("realm.db");
    let conn = Connection::open(&db_path).map_err(|e| format!("opening {}: {e}", db_path.display()))?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS nodes (
            id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            title TEXT,
            status TEXT,
            content_hash TEXT,
            source_ref TEXT
        );
        CREATE TABLE IF NOT EXISTS edges (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            src TEXT NOT NULL,
            dst TEXT NOT NULL,
            kind TEXT NOT NULL,
            hash_at_link TEXT,
            flag TEXT,
            flag_reason TEXT,
            UNIQUE(src, dst, kind)
        );
        CREATE TABLE IF NOT EXISTS provenance (
            edge_id INTEGER NOT NULL,
            created_by TEXT,
            model TEXT,
            prompt TEXT,
            ts TEXT
        );
        CREATE TABLE IF NOT EXISTS chunks (
            id TEXT PRIMARY KEY,
            doc_id TEXT NOT NULL,
            content TEXT NOT NULL
        );
        CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
            chunk_id UNINDEXED, doc_id UNINDEXED, content
        );
        ",
    )
    .map_err(|e| format!("migrating .nir/realm.db schema: {e}"))
}

/// One `fn`/`struct`/`enum`/`screen` top-level item, identity =
/// `(kind, qualified_name)` -- deliberately not line/col (shifts on
/// unrelated edits) and not a hash (changes on every edit), the same
/// "chunk identity independent of version" rule the RFC's document
/// side uses. `content_hash` is that identity's *version*.
struct CodeUnit {
    qualified_name: String,
    kind: &'static str,
    content_hash: String,
}

fn hash_item<T: serde::Serialize>(item: &T) -> Result<String, String> {
    let json = serde_json::to_vec(item).map_err(|e| format!("serializing AST item: {e}"))?;
    Ok(sha256_hex(&json))
}

/// Parses one file's own declared items (no `use` resolution -- see
/// this module's doc comment for why) and hashes each one's own AST
/// subtree, whitespace/comment-insensitive by construction because
/// it's computed post-parse, the same property
/// `docs/nirdosha-agent-api.md` E1's `ast_hash` already documents at
/// the whole-program level.
fn code_units_in_file(path: &Path) -> Result<Vec<CodeUnit>, String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    let toks = crate::token::Lexer::new(&src)
        .tokenize()
        .map_err(|e| format!("lex error in {}: {}:{}: {}", path.display(), e.span.line, e.span.col, e.message))?;
    let program = crate::parser::Parser::new(toks)
        .parse_program()
        .map_err(|e| format!("parse error in {}: {}:{}: {}", path.display(), e.span.line, e.span.col, e.message))?;

    // `parser::parse_program` seeds `ast::prelude_structs`/
    // `prelude_enums` (Option/Result/Money/HttpResponse/... -- Row 11
    // layer 7's prelude) into *every* `Program`, before any of a file's
    // own declarations -- language-level vocabulary, not this project's
    // code, so it must never show up as a `CodeUnit` (every file would
    // otherwise report the same ~15 "units" on its very first sync).
    let prelude_struct_names: HashSet<String> = crate::ast::prelude_structs().into_iter().map(|s| s.name).collect();
    let prelude_enum_names: HashSet<String> = crate::ast::prelude_enums().into_iter().map(|e| e.name).collect();

    let mut units = Vec::with_capacity(program.fns.len() + program.structs.len() + program.enums.len() + program.screens.len());
    for f in &program.fns {
        units.push(CodeUnit { qualified_name: f.name.clone(), kind: "fn", content_hash: hash_item(f)? });
    }
    for s in program.structs.iter().filter(|s| !prelude_struct_names.contains(&s.name)) {
        units.push(CodeUnit { qualified_name: s.name.clone(), kind: "struct", content_hash: hash_item(s)? });
    }
    for e in program.enums.iter().filter(|e| !prelude_enum_names.contains(&e.name)) {
        units.push(CodeUnit { qualified_name: e.name.clone(), kind: "enum", content_hash: hash_item(e)? });
    }
    for sc in &program.screens {
        units.push(CodeUnit { qualified_name: sc.struct_name.clone(), kind: "screen", content_hash: hash_item(sc)? });
    }
    Ok(units)
}

fn code_unit_node_id(kind: &str, qualified_name: &str) -> String {
    format!("code:{kind}:{qualified_name}")
}

#[derive(Default, Debug, Clone, Copy)]
pub struct SyncReport {
    pub files_scanned: usize,
    pub units_seen: usize,
    pub units_added: usize,
    pub units_changed: usize,
    pub edges_flagged: usize,
}

impl SyncReport {
    fn merge(&mut self, other: SyncReport) {
        self.files_scanned += other.files_scanned;
        self.units_seen += other.units_seen;
        self.units_added += other.units_added;
        self.units_changed += other.units_changed;
        self.edges_flagged += other.edges_flagged;
    }
}

/// Re-runs the per-item hash walk for one file, upserts each
/// `CodeUnit` node, and -- the "code -> knowledge" half of bidirectional
/// impact (rfcs/0013) -- flags every `IMPLEMENTS` edge out of a
/// `CodeUnit` whose hash just changed as `possibly_stale`. Never
/// deletes or rewrites an edge, a node's title, or the `.nir` source
/// itself -- only ever adds a flag, per the RFC's "explicit vs.
/// inferred knowledge stay separate" principle.
fn sync_file(conn: &Connection, path: &Path) -> Result<SyncReport, String> {
    let mut report = SyncReport { files_scanned: 1, ..Default::default() };
    let units = code_units_in_file(path)?;
    report.units_seen = units.len();
    let source_ref = path.display().to_string();

    for u in &units {
        let id = code_unit_node_id(u.kind, &u.qualified_name);
        let prev_hash: Option<String> =
            conn.query_row("SELECT content_hash FROM nodes WHERE id = ?1", [&id], |r| r.get(0)).optional().map_err(|e| format!("reading node {id}: {e}"))?;

        match &prev_hash {
            None => report.units_added += 1,
            Some(h) if h != &u.content_hash => report.units_changed += 1,
            _ => {}
        }

        conn.execute(
            "INSERT INTO nodes (id, kind, title, status, content_hash, source_ref)
             VALUES (?1, 'CodeUnit', ?2, NULL, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET content_hash = excluded.content_hash, source_ref = excluded.source_ref",
            params![id, u.qualified_name, u.content_hash, source_ref],
        )
        .map_err(|e| format!("upserting node {id}: {e}"))?;

        if let Some(prev) = &prev_hash {
            if prev != &u.content_hash {
                // `link` always records both directions of one
                // relationship (`IMPLEMENTS` code->req and its inverse
                // `IMPLEMENTED_BY` req->code) -- flag both, not just
                // one: `impact`'s BFS dedups by neighbor id regardless
                // of which edge reached it, so leaving the inverse
                // unflagged risks the flag silently disappearing
                // whenever that edge happens to be visited first.
                let flagged = conn
                    .execute(
                        "UPDATE edges SET flag = 'possibly_stale', flag_reason = ?2
                         WHERE flag IS NULL AND (
                             (src = ?1 AND kind = 'IMPLEMENTS')
                             OR (dst = ?1 AND kind = 'IMPLEMENTED_BY')
                         )",
                        params![id, format!("CodeUnit content changed ({prev} -> {})", u.content_hash)],
                    )
                    .map_err(|e| format!("flagging edges touching {id}: {e}"))?;
                report.edges_flagged += flagged;
            }
        }
    }
    Ok(report)
}

const SKIP_DIRS: &[&str] = &[".nir", ".git", "target", "node_modules"];

fn find_nir_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            // A directory that vanished mid-walk (or was never
            // readable) shouldn't abort the whole sync -- skip it.
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if !SKIP_DIRS.contains(&name) {
                    stack.push(path);
                }
            } else if path.extension().and_then(|e| e.to_str()) == Some("nir") {
                out.push(path);
            }
        }
    }
    Ok(out)
}

/// The code half of `realm sync` -- run automatically on every `hi`
/// startup (`root`-wide, `files` empty) or standalone for CI/manual use
/// (`files` explicit). Content-addressing means a repeat call with no
/// code changes touches zero `nodes` rows beyond a hash comparison per
/// file -- the "fast no-op" property rfcs/0013's auto-scaffold section
/// depends on.
pub fn sync(conn: &Connection, root: &Path, files: &[String]) -> Result<SyncReport, String> {
    if files.is_empty() {
        let mut total = SyncReport::default();
        for path in find_nir_files(root)? {
            let r = sync_file(conn, &path)?;
            total.merge(r);
        }
        Ok(total)
    } else {
        let mut total = SyncReport::default();
        for f in files {
            let r = sync_file(conn, Path::new(f))?;
            total.merge(r);
        }
        Ok(total)
    }
}

/// Resolves a `realm link`/`realm impact` code-side target: either an
/// explicit `kind:name` (`fn:transfer_funds`), or a bare name that must
/// uniquely identify one `CodeUnit` -- ambiguity (e.g. a `fn` and a
/// `struct` sharing a name) is reported, not silently guessed.
fn resolve_code_unit_id(conn: &Connection, target: &str) -> Result<String, String> {
    if let Some((kind, name)) = target.split_once(':') {
        if ["fn", "struct", "enum", "screen"].contains(&kind) {
            let id = code_unit_node_id(kind, name);
            let exists: Option<String> = conn.query_row("SELECT id FROM nodes WHERE id = ?1", [&id], |r| r.get(0)).optional().map_err(|e| e.to_string())?;
            return exists.ok_or_else(|| format!("no CodeUnit `{id}` in the realm graph -- run `nirdosha realm sync` first"));
        }
    }
    let mut stmt = conn.prepare("SELECT id FROM nodes WHERE kind = 'CodeUnit' AND title = ?1").map_err(|e| e.to_string())?;
    let ids: Vec<String> = stmt.query_map([target], |r| r.get(0)).map_err(|e| e.to_string())?.filter_map(Result::ok).collect();
    match ids.len() {
        0 => Err(format!("no CodeUnit named `{target}` in the realm graph -- run `nirdosha realm sync` first")),
        1 => Ok(ids.into_iter().next().expect("len checked above")),
        _ => Err(format!("`{target}` is ambiguous ({} matches: {}) -- disambiguate with `kind:name`, e.g. `fn:{target}`", ids.len(), ids.join(", "))),
    }
}

/// Records a manual `IMPLEMENTS`/`IMPLEMENTED_BY` edge pair between a
/// requirement/decision id and a `CodeUnit` -- for code that predates
/// Realm or was written by hand, outside `hi`'s own generation-time
/// auto-linking (rfcs/0013's "How a code<->knowledge link actually gets
/// created"). Upserts a stub `Requirement` node if `requirement_id`
/// isn't already known, so `link` works standalone before any
/// `realm ingest` has run.
pub fn link(conn: &Connection, requirement_id: &str, target: &str) -> Result<(), String> {
    let req_node_id = format!("requirement:{requirement_id}");
    conn.execute(
        "INSERT INTO nodes (id, kind, title, status, content_hash, source_ref)
         VALUES (?1, 'Requirement', ?2, NULL, NULL, NULL)
         ON CONFLICT(id) DO NOTHING",
        params![req_node_id, requirement_id],
    )
    .map_err(|e| format!("upserting requirement node {req_node_id}: {e}"))?;

    let code_id = resolve_code_unit_id(conn, target)?;
    let hash: String = conn
        .query_row("SELECT content_hash FROM nodes WHERE id = ?1", [&code_id], |r| r.get(0))
        .map_err(|e| format!("reading node {code_id}: {e}"))?;

    for (src, dst, kind) in [(code_id.as_str(), req_node_id.as_str(), "IMPLEMENTS"), (req_node_id.as_str(), code_id.as_str(), "IMPLEMENTED_BY")] {
        conn.execute(
            "INSERT INTO edges (src, dst, kind, hash_at_link, flag, flag_reason)
             VALUES (?1, ?2, ?3, ?4, NULL, NULL)
             ON CONFLICT(src, dst, kind) DO UPDATE SET hash_at_link = excluded.hash_at_link, flag = NULL, flag_reason = NULL",
            params![src, dst, kind, hash],
        )
        .map_err(|e| format!("recording {kind} edge {src} -> {dst}: {e}"))?;
    }
    Ok(())
}

fn resolve_any_node_id(conn: &Connection, target: &str) -> Result<String, String> {
    if target.starts_with("requirement:") || target.starts_with("decision:") || target.starts_with("code:") {
        let exists: Option<String> = conn.query_row("SELECT id FROM nodes WHERE id = ?1", [target], |r| r.get(0)).optional().map_err(|e| e.to_string())?;
        return exists.ok_or_else(|| format!("no node `{target}` in the realm graph"));
    }
    for prefix in ["requirement:", "decision:"] {
        let id = format!("{prefix}{target}");
        if conn.query_row("SELECT 1 FROM nodes WHERE id = ?1", [&id], |_| Ok(())).optional().map_err(|e| e.to_string())?.is_some() {
            return Ok(id);
        }
    }
    resolve_code_unit_id(conn, target)
}

/// Bounded (`max_depth`/`max_nodes`) traversal, principle 4 of
/// rfcs/0013 -- returns `partial: true` on exhaustion rather than
/// walking further. Same query works for both directions named in the
/// RFC ("code -> knowledge" and "requirement -> code"): `link` always
/// records the `IMPLEMENTS`/`IMPLEMENTED_BY` pair together, so walking
/// every edge touching a node (either as `src` or `dst`) reaches
/// whichever side is relevant regardless of which kind of id `target`
/// names.
pub struct ImpactHit {
    pub node_id: String,
    pub kind: String,
    pub title: Option<String>,
    pub edge_kind: String,
    pub flag: Option<String>,
    pub flag_reason: Option<String>,
    pub depth: u32,
}

pub struct ImpactReport {
    pub hits: Vec<ImpactHit>,
    pub partial: bool,
}

const DEFAULT_MAX_DEPTH: u32 = 5;
const DEFAULT_MAX_NODES: usize = 500;

pub fn impact(conn: &Connection, target: &str) -> Result<ImpactReport, String> {
    let start_id = resolve_any_node_id(conn, target)?;
    let mut visited: HashSet<String> = HashSet::new();
    visited.insert(start_id.clone());
    let mut queue: VecDeque<(String, u32)> = VecDeque::new();
    queue.push_back((start_id, 0));
    let mut hits = Vec::new();
    let mut partial = false;

    'walk: while let Some((id, depth)) = queue.pop_front() {
        if depth >= DEFAULT_MAX_DEPTH {
            continue;
        }
        let mut stmt = conn
            .prepare(
                "SELECT dst, kind, flag, flag_reason FROM edges WHERE src = ?1
                 UNION ALL
                 SELECT src, kind, flag, flag_reason FROM edges WHERE dst = ?1",
            )
            .map_err(|e| e.to_string())?;
        let rows: Vec<(String, String, Option<String>, Option<String>)> =
            stmt.query_map([&id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).map_err(|e| e.to_string())?.filter_map(Result::ok).collect();

        for (neighbor_id, edge_kind, flag, flag_reason) in rows {
            if !visited.insert(neighbor_id.clone()) {
                continue;
            }
            if hits.len() >= DEFAULT_MAX_NODES {
                partial = true;
                break 'walk;
            }
            let (kind, title): (String, Option<String>) = conn
                .query_row("SELECT kind, title FROM nodes WHERE id = ?1", [&neighbor_id], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(|e| format!("reading node {neighbor_id}: {e}"))?;
            hits.push(ImpactHit { node_id: neighbor_id.clone(), kind, title, edge_kind, flag, flag_reason, depth: depth + 1 });
            queue.push_back((neighbor_id, depth + 1));
        }
    }
    // Flagged nodes first -- the RFC's own "possibly_stale nodes called
    // out first" report ordering.
    hits.sort_by_key(|h| (h.flag.is_none(), h.depth));
    Ok(ImpactReport { hits, partial })
}

/// Content-addresses `path` as a `Document` node, splits it into
/// blank-line-separated chunks, and FTS5-indexes each chunk not already
/// seen (by content hash) -- the document-ingestion half of rfcs/0013,
/// deliberately explicit/opt-in, never run by `hi`'s auto-scaffold: no
/// heuristic here can reliably tell a requirements doc from a README.
pub fn ingest_document(conn: &Connection, path: &Path) -> Result<usize, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    let doc_id = format!("document:{}", path.display());
    conn.execute(
        "INSERT INTO nodes (id, kind, title, status, content_hash, source_ref)
         VALUES (?1, 'Document', ?2, NULL, ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET content_hash = excluded.content_hash",
        params![doc_id, path.file_name().and_then(|n| n.to_str()).unwrap_or(""), sha256_hex(text.as_bytes()), path.display().to_string()],
    )
    .map_err(|e| format!("upserting document node {doc_id}: {e}"))?;

    let mut new_chunks = 0;
    for para in text.split("\n\n") {
        let para = para.trim();
        if para.is_empty() {
            continue;
        }
        let chunk_id = sha256_hex(para.as_bytes());
        let inserted = conn
            .execute("INSERT OR IGNORE INTO chunks (id, doc_id, content) VALUES (?1, ?2, ?3)", params![chunk_id, doc_id, para])
            .map_err(|e| format!("inserting chunk {chunk_id}: {e}"))?;
        if inserted > 0 {
            conn.execute("INSERT INTO chunks_fts (chunk_id, doc_id, content) VALUES (?1, ?2, ?3)", params![chunk_id, doc_id, para])
                .map_err(|e| format!("indexing chunk {chunk_id}: {e}"))?;
            new_chunks += 1;
        }
    }
    Ok(new_chunks)
}

pub struct AskHit {
    pub doc_id: String,
    pub content: String,
}

const DEFAULT_ASK_LIMIT: u32 = 10;

/// FTS5-only retrieval -- rfcs/0013's "v1 floor": no vector search, no
/// embeddings, no LLM call, must still answer "what does R17 say."
pub fn ask(conn: &Connection, query: &str) -> Result<Vec<AskHit>, String> {
    let mut stmt = conn.prepare("SELECT doc_id, content FROM chunks_fts WHERE chunks_fts MATCH ?1 ORDER BY rank LIMIT ?2").map_err(|e| format!("preparing FTS query: {e}"))?;
    let hits = stmt
        .query_map(params![query, DEFAULT_ASK_LIMIT], |r| Ok(AskHit { doc_id: r.get(0)?, content: r.get(1)? }))
        .map_err(|e| format!("running FTS query `{query}`: {e}"))?
        .filter_map(Result::ok)
        .collect();
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("nirdosha_realm_test_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn write_nir(dir: &Path, name: &str, src: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, src).unwrap();
        path
    }

    #[test]
    fn scaffolding_is_idempotent_and_creates_expected_layout() {
        let dir = scratch_dir("scaffold");
        let conn1 = open(&dir).expect("first open");
        drop(conn1);
        let conn2 = open(&dir).expect("second open, same dir");
        drop(conn2);
        assert!(realm_dir(&dir).join("realm.db").is_file());
        assert!(realm_dir(&dir).join("content").is_dir());
    }

    #[test]
    fn sync_adds_then_leaves_unchanged_units_alone_on_repeat_sync() {
        let dir = scratch_dir("sync_repeat");
        let file = write_nir(&dir, "a.nir", "fn add(a: i64, b: i64) -> i64 { return a + b }\n");
        let conn = open(&dir).expect("open");

        let first = sync(&conn, &dir, &[]).expect("first sync");
        assert_eq!(first.units_added, 1);
        assert_eq!(first.units_changed, 0);

        let second = sync(&conn, &dir, &[]).expect("second sync, no changes");
        assert_eq!(second.units_added, 0);
        assert_eq!(second.units_changed, 0);
        let _ = file;
    }

    #[test]
    fn changed_code_flags_its_linked_requirement_as_possibly_stale() {
        let dir = scratch_dir("flag_stale");
        write_nir(&dir, "a.nir", "fn transfer_funds(amount: i64) -> i64 { return amount }\n");
        let conn = open(&dir).expect("open");
        sync(&conn, &dir, &[]).expect("initial sync");

        link(&conn, "R17", "fn:transfer_funds").expect("link");
        let report = impact(&conn, "R17").expect("impact before change");
        assert!(report.hits.iter().all(|h| h.flag.is_none()));

        // Edit the function body -- same qualified name, different AST.
        write_nir(&dir, "a.nir", "fn transfer_funds(amount: i64) -> i64 { return amount + 1 }\n");
        let resync = sync(&conn, &dir, &[]).expect("resync after edit");
        assert_eq!(resync.units_changed, 1);
        // Both directions of the IMPLEMENTS/IMPLEMENTED_BY pair get
        // flagged -- see sync_file's own comment for why.
        assert_eq!(resync.edges_flagged, 2);

        let report = impact(&conn, "R17").expect("impact after change");
        assert!(report.hits.iter().any(|h| h.flag.as_deref() == Some("possibly_stale")), "expected a possibly_stale hit, got: {:?}", report.hits.iter().map(|h| &h.node_id).collect::<Vec<_>>());
    }

    #[test]
    fn impact_reaches_a_requirement_from_its_code_unit_and_back() {
        let dir = scratch_dir("bidirectional");
        write_nir(&dir, "a.nir", "fn correct_entry(id: i64) -> i64 { return id }\n");
        let conn = open(&dir).expect("open");
        sync(&conn, &dir, &[]).expect("sync");
        link(&conn, "R55", "fn:correct_entry").expect("link");

        let from_code = impact(&conn, "fn:correct_entry").expect("impact from code");
        assert!(from_code.hits.iter().any(|h| h.node_id == "requirement:R55"));

        let from_req = impact(&conn, "R55").expect("impact from requirement");
        assert!(from_req.hits.iter().any(|h| h.node_id == "code:fn:correct_entry"));
    }

    #[test]
    fn link_without_a_prior_sync_reports_the_fix_precisely() {
        let dir = scratch_dir("link_missing");
        let conn = open(&dir).expect("open");
        let err = link(&conn, "R1", "fn:nope").unwrap_err();
        assert!(err.contains("realm sync"), "error should point at the fix, got: {err}");
    }

    #[test]
    fn ingest_then_ask_finds_a_matching_chunk() {
        let dir = scratch_dir("ingest_ask");
        let doc = dir.join("prd.md");
        std::fs::write(&doc, "The ledger must never be altered once written.\n\nAdministrators may not modify past entries.").unwrap();
        let conn = open(&dir).expect("open");

        let n = ingest_document(&conn, &doc).expect("ingest");
        assert_eq!(n, 2);

        let hits = ask(&conn, "ledger").expect("ask");
        assert!(hits.iter().any(|h| h.content.contains("ledger")));
    }

    #[test]
    fn re_ingesting_the_same_document_does_not_duplicate_chunks() {
        let dir = scratch_dir("ingest_repeat");
        let doc = dir.join("prd.md");
        std::fs::write(&doc, "One paragraph only.").unwrap();
        let conn = open(&dir).expect("open");
        assert_eq!(ingest_document(&conn, &doc).expect("first ingest"), 1);
        assert_eq!(ingest_document(&conn, &doc).expect("second ingest"), 0);
    }

    #[test]
    fn is_disabled_only_true_for_exactly_one() {
        let on = |_: &str| Some("1".to_string());
        let off = |_: &str| Some("0".to_string());
        let unset = |_: &str| None;
        assert!(is_disabled(&on));
        assert!(!is_disabled(&off));
        assert!(!is_disabled(&unset));
    }
}
