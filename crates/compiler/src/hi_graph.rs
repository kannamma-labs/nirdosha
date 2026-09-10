//! Nirdosha Hi's own knowledge graph (originally "Realm",
//! rfcs/0013-nirdosha-realm.md) -- a local-first, bounded knowledge
//! graph auto-scaffolded under `.nir/` on every `nirdosha hi` startup
//! (`main.rs::cmd_hi`'s own sync-then-open-window sequence). v1 slice:
//! SQLite schema (`nodes`/`edges`/`provenance`/`chunks`+FTS5), per-item
//! code hashing reusing the same lex/parse primitives `emit-ast`/
//! `loader::load_program` are built from, bounded bidirectional impact
//! queries, manual requirement<->code links, and a minimal document
//! ingest/ask path. Never mandatory: every entry point here degrades to
//! "log a warning, keep going" for its caller rather than ever being
//! the thing that breaks `hi` itself (the RFC's own "must run on an
//! ordinary laptop, must never be the thing that crashes" posture).
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

/// `NIRDOSHA_HI_DISABLE=1` skips the auto-scaffold and auto-sync
/// `hi` would otherwise run on startup -- same `NIRDOSHA_`-prefixed,
/// `.ok()`-based env-var convention this crate uses elsewhere.
pub const HI_DISABLE_VAR: &str = "NIRDOSHA_HI_DISABLE";

/// `env` is injected rather than calling `std::env::var` directly --
/// a pure, testable check, no direct env access baked in.
pub fn is_disabled(env: &dyn Fn(&str) -> Option<String>) -> bool {
    env(HI_DISABLE_VAR).as_deref() == Some("1")
}

/// `.nir/` under `root` -- rfcs/0013's "Physical layout." Named `.nir`
/// (not a feature-specific extension) per that RFC's own revision; see
/// its Open Questions for the acknowledged overlap with the `.nir`
/// source-file extension.
pub fn hi_dir(root: &Path) -> PathBuf {
    root.join(".nir")
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// Creates `.nir/` and `.nir/content/` if absent, opens (or creates)
/// `.nir/hi.db`, and migrates the schema. Every migration statement
/// is `IF NOT EXISTS` -- safe to call on every `hi` startup, which is
/// exactly how it's used.
pub fn open(root: &Path) -> Result<Connection, String> {
    let dir = hi_dir(root);
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    std::fs::create_dir_all(dir.join("content")).map_err(|e| format!("creating {}: {e}", dir.join("content").display()))?;
    let db_path = dir.join("hi.db");
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
            source_ref TEXT,
            line INTEGER,
            col INTEGER
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
    .map_err(|e| format!("migrating .nir/hi.db schema: {e}"))?;
    // `CREATE TABLE IF NOT EXISTS` above never widens an already-existing
    // `nodes` table -- a `.nir/hi.db` created before `line`/`col`
    // existed needs them added explicitly, once, idempotently. Real gap
    // this closes: `source_ref` (the file path) was captured from v1 but
    // never surfaced anywhere a human could see it (rfcs/0013's own
    // Open Questions record this); `line`/`col` finish that half-done
    // fix by giving `:impact`/`hi impact` an actual place in the file
    // to point at, not just which file.
    add_column_if_missing(conn, "nodes", "line", "INTEGER")?;
    add_column_if_missing(conn, "nodes", "col", "INTEGER")?;
    // rfcs/0014's prompt/build/generate/publish pipeline: the text layer
    // a `CodeUnit` node carries *before* any `.nir` exists
    // (`driving_text`), who/what put it there (`created_by` --
    // `NULL` for anything `hi sync` found in real code, `llm-prompt-
    // mode` for a prompt-mode candidate), the human review/lock/waive
    // state Build/Generate mode gate on, and the free-text attribute
    // lines Build mode's attribute editor attaches. `last_materialized_
    // hash` is deliberately separate from `content_hash`: the latter is
    // re-synced by every `hi sync`, so it can never answer "does this
    // file still match what Generate mode itself last wrote" -- see RFC
    // 0014's "What's authoritative, when" for the full reasoning this
    // column exists to satisfy. `0`/`1` integers, not a real SQLite
    // `BOOLEAN` type (SQLite has none), matching this schema's own
    // existing convention.
    add_column_if_missing(conn, "nodes", "driving_text", "TEXT")?;
    add_column_if_missing(conn, "nodes", "created_by", "TEXT")?;
    add_column_if_missing(conn, "nodes", "confirmed", "INTEGER NOT NULL DEFAULT 0")?;
    add_column_if_missing(conn, "nodes", "locked", "INTEGER NOT NULL DEFAULT 0")?;
    add_column_if_missing(conn, "nodes", "waived", "INTEGER NOT NULL DEFAULT 0")?;
    add_column_if_missing(conn, "nodes", "waive_reason", "TEXT")?;
    add_column_if_missing(conn, "nodes", "last_materialized_hash", "TEXT")?;
    add_column_if_missing(conn, "nodes", "attributes", "TEXT")?;
    Ok(())
}

fn add_column_if_missing(conn: &Connection, table: &str, column: &str, sql_type: &str) -> Result<(), String> {
    let exists: bool = conn
        .prepare(&format!("SELECT 1 FROM pragma_table_info('{table}') WHERE name = ?1"))
        .and_then(|mut stmt| stmt.exists([column]))
        .map_err(|e| format!("checking whether {table}.{column} already exists: {e}"))?;
    if !exists {
        conn.execute(&format!("ALTER TABLE {table} ADD COLUMN {column} {sql_type}"), [])
            .map_err(|e| format!("adding {table}.{column}: {e}"))?;
    }
    Ok(())
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
    line: usize,
    col: usize,
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
    // Filtered by (name, content_hash) together, not name alone: a
    // user-defined type that legitimately shadows a prelude name (their
    // own `struct Money` with different fields, say) has a different
    // hash than the real prelude `Money` and must survive filtering --
    // name-only filtering silently dropped it before this fix.
    let mut prelude_struct_hashes: HashSet<(String, String)> = HashSet::new();
    for s in crate::ast::prelude_structs() {
        let hash = hash_item(&s)?;
        prelude_struct_hashes.insert((s.name, hash));
    }
    let mut prelude_enum_hashes: HashSet<(String, String)> = HashSet::new();
    for e in crate::ast::prelude_enums() {
        let hash = hash_item(&e)?;
        prelude_enum_hashes.insert((e.name, hash));
    }

    let mut units = Vec::with_capacity(program.fns.len() + program.structs.len() + program.enums.len() + program.screens.len());
    for f in &program.fns {
        units.push(CodeUnit { qualified_name: f.name.clone(), kind: "fn", content_hash: hash_item(f)?, line: f.span.line, col: f.span.col });
    }
    for s in &program.structs {
        let hash = hash_item(s)?;
        if prelude_struct_hashes.contains(&(s.name.clone(), hash.clone())) {
            continue;
        }
        units.push(CodeUnit { qualified_name: s.name.clone(), kind: "struct", content_hash: hash, line: s.span.line, col: s.span.col });
    }
    for e in &program.enums {
        let hash = hash_item(e)?;
        if prelude_enum_hashes.contains(&(e.name.clone(), hash.clone())) {
            continue;
        }
        units.push(CodeUnit { qualified_name: e.name.clone(), kind: "enum", content_hash: hash, line: e.span.line, col: e.span.col });
    }
    for sc in &program.screens {
        units.push(CodeUnit { qualified_name: sc.struct_name.clone(), kind: "screen", content_hash: hash_item(sc)?, line: sc.span.line, col: sc.span.col });
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
        // `Option<String>` twice over, deliberately: the outer one (via
        // `.optional()`) covers "no such node yet" (a real `.nir` file
        // being synced for the first time); the inner one covers "the
        // node exists but has no `content_hash` yet" -- true of a
        // prompt-mode candidate (`hi_graph::add_candidate`) that Generate
        // mode is only now materializing into this call's own real
        // source. Reading column 0 as a bare `String` instead would
        // error out on that NULL rather than treating it as "no prior
        // hash," the same way a genuinely new node does.
        let prev_hash: Option<String> = conn
            .query_row("SELECT content_hash FROM nodes WHERE id = ?1", [&id], |r| r.get::<_, Option<String>>(0))
            .optional()
            .map_err(|e| format!("reading node {id}: {e}"))?
            .flatten();

        match &prev_hash {
            None => report.units_added += 1,
            Some(h) if h != &u.content_hash => report.units_changed += 1,
            _ => {}
        }

        conn.execute(
            "INSERT INTO nodes (id, kind, title, status, content_hash, source_ref, line, col)
             VALUES (?1, 'CodeUnit', ?2, NULL, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET content_hash = excluded.content_hash, source_ref = excluded.source_ref, line = excluded.line, col = excluded.col",
            params![id, u.qualified_name, u.content_hash, source_ref, u.line as i64, u.col as i64],
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

/// The code half of `hi sync` -- run automatically on every `hi`
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

/// Resolves a `hi link`/`hi impact` code-side target: either an
/// explicit `kind:name` (`fn:transfer_funds`), or a bare name that must
/// uniquely identify one `CodeUnit` -- ambiguity (e.g. a `fn` and a
/// `struct` sharing a name) is reported, not silently guessed.
fn resolve_code_unit_id(conn: &Connection, target: &str) -> Result<String, String> {
    if let Some((kind, name)) = target.split_once(':') {
        if ["fn", "struct", "enum", "screen"].contains(&kind) {
            let id = code_unit_node_id(kind, name);
            let exists: Option<String> = conn.query_row("SELECT id FROM nodes WHERE id = ?1", [&id], |r| r.get(0)).optional().map_err(|e| e.to_string())?;
            return exists.ok_or_else(|| format!("no CodeUnit `{id}` in the hi graph -- run `nirdosha hi sync` first"));
        }
    }
    let mut stmt = conn.prepare("SELECT id FROM nodes WHERE kind = 'CodeUnit' AND title = ?1").map_err(|e| e.to_string())?;
    let ids: Vec<String> = stmt.query_map([target], |r| r.get(0)).map_err(|e| e.to_string())?.filter_map(Result::ok).collect();
    match ids.len() {
        0 => Err(format!("no CodeUnit named `{target}` in the hi graph -- run `nirdosha hi sync` first")),
        1 => Ok(ids.into_iter().next().expect("len checked above")),
        _ => Err(format!("`{target}` is ambiguous ({} matches: {}) -- disambiguate with `kind:name`, e.g. `fn:{target}`", ids.len(), ids.join(", "))),
    }
}

/// Records a manual `IMPLEMENTS`/`IMPLEMENTED_BY` edge pair between a
/// requirement/decision id and a `CodeUnit` -- for code that predates
/// the hi graph or was written by hand, outside `hi`'s own generation-time
/// auto-linking (rfcs/0013's "How a code<->knowledge link actually gets
/// created"). Upserts a stub `Requirement` node if `requirement_id`
/// isn't already known, so `link` works standalone before any
/// `hi ingest` has run.
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
        return exists.ok_or_else(|| format!("no node `{target}` in the hi graph"));
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
#[derive(serde::Serialize)]
pub struct ImpactHit {
    pub node_id: String,
    pub kind: String,
    pub title: Option<String>,
    pub edge_kind: String,
    pub flag: Option<String>,
    pub flag_reason: Option<String>,
    pub depth: u32,
    /// The file this `CodeUnit` node was parsed from (`nodes.source_ref`)
    /// -- `None` for a `Requirement`/`Document`/`Chunk` node, which has
    /// no `.nir` file to point at. Previously captured but never
    /// surfaced anywhere a human could see it (rfcs/0013's own Open
    /// Questions recorded this); this finishes that fix.
    pub source_ref: Option<String>,
    /// 1-based line/col within `source_ref`, from the declaration's own
    /// AST span -- `None` on the same terms as `source_ref`.
    pub line: Option<i64>,
    pub col: Option<i64>,
}

#[derive(serde::Serialize)]
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
            let (kind, title, source_ref, line, col): (String, Option<String>, Option<String>, Option<i64>, Option<i64>) = conn
                .query_row("SELECT kind, title, source_ref, line, col FROM nodes WHERE id = ?1", [&neighbor_id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
                })
                .map_err(|e| format!("reading node {neighbor_id}: {e}"))?;
            hits.push(ImpactHit { node_id: neighbor_id.clone(), kind, title, edge_kind, flag, flag_reason, depth: depth + 1, source_ref, line, col });
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

#[derive(serde::Serialize, Debug)]
pub struct AskHit {
    pub doc_id: String,
    pub content: String,
}

const DEFAULT_ASK_LIMIT: u32 = 10;

/// FTS5-only retrieval -- rfcs/0013's "v1 floor": no vector search, no
/// embeddings, no LLM call, must still answer "what does R17 say."
/// FTS5 over ingested documents, plus (rfcs/0014) a plain per-term
/// substring search over every `CodeUnit`'s own name and driving text
/// -- "what does tick do" finds a `fn tick` that FTS5's strict,
/// implicit-AND-of-every-term matching against short driving text
/// almost never would, and finds it whether or not that unit's own
/// prose ever literally says "tick": the qualified name is searched
/// too, not just `driving_text`.
///
/// Deliberately *not* folded into `chunks_fts` alongside ingested
/// documents: that table is content-addressed (`chunks.id` = a hash of
/// the text), a fit for near-immutable document chunks, not a
/// candidate's `driving_text`, which `hi_graph::edit_driving_text`
/// expects to change often -- indexing it there would leave a stale
/// chunk behind on every edit, with nothing to ever clean it up. A
/// live query against the current column has no such staleness
/// problem, at the cost of FTS5's own ranking/tokenization.
pub fn ask(conn: &Connection, query: &str) -> Result<Vec<AskHit>, String> {
    let mut hits: Vec<AskHit> = Vec::new();

    let mut doc_stmt = conn.prepare("SELECT doc_id, content FROM chunks_fts WHERE chunks_fts MATCH ?1 ORDER BY rank LIMIT ?2").map_err(|e| format!("preparing FTS query: {e}"))?;
    let doc_hits = doc_stmt
        .query_map(params![query, DEFAULT_ASK_LIMIT], |r| Ok(AskHit { doc_id: r.get(0)?, content: r.get(1)? }))
        .map_err(|e| format!("running FTS query `{query}`: {e}"))?
        .filter_map(Result::ok);
    hits.extend(doc_hits);

    // Words under 3 characters ("do", "a", "of", ...) are almost always
    // noise for a substring match this loose -- dropped rather than
    // matched against everything.
    let terms: Vec<String> = query.split_whitespace().map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()).filter(|w| w.len() >= 3).collect();
    if !terms.is_empty() && hits.len() < DEFAULT_ASK_LIMIT as usize {
        let mut code_stmt = conn.prepare("SELECT id, title, driving_text, source_ref, line FROM nodes WHERE kind = 'CodeUnit'").map_err(|e| format!("preparing CodeUnit search: {e}"))?;
        let rows: Vec<(String, Option<String>, Option<String>, Option<String>, Option<i64>)> =
            code_stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))).map_err(|e| format!("listing CodeUnit content: {e}"))?.filter_map(Result::ok).collect();
        for (id, title, driving_text, source_ref, line) in rows {
            let name = title.unwrap_or_default();
            let haystack = format!("{name} {}", driving_text.as_deref().unwrap_or("")).to_lowercase();
            if !terms.iter().any(|t| haystack.contains(t.as_str())) {
                continue;
            }
            let content = match driving_text.filter(|dt| !dt.is_empty()) {
                Some(dt) => dt,
                None => match (source_ref, line) {
                    (Some(sref), Some(l)) => format!("(no driving text yet -- see {sref}:{l})"),
                    (Some(sref), None) => format!("(no driving text yet -- see {sref})"),
                    _ => "(no description available yet)".to_string(),
                },
            };
            hits.push(AskHit { doc_id: id, content });
            if hits.len() >= DEFAULT_ASK_LIMIT as usize {
                break;
            }
        }
    }

    Ok(hits)
}

/// A bounded plain-text summary of the whole graph's `CodeUnit`
/// content (name: driving text, or source location when there's no
/// driving text) -- context for a project-level question `ask`'s own
/// local keyword search can't answer at all ("what is this project
/// about" matches no single node's name or text, because it isn't
/// really about any one node). See `hi_llm::answer_question`, the only
/// caller: this function itself stays network-free, same as everything
/// else in this module. Capped at `PROJECT_CONTEXT_MAX_UNITS`, not the
/// entire graph verbatim -- this module's own "every expensive
/// operation is bounded" principle, applied here so a very large
/// project can't blow an LLM prompt past a reasonable size.
const PROJECT_CONTEXT_MAX_UNITS: u32 = 200;

pub fn project_context(conn: &Connection) -> Result<String, String> {
    let mut stmt = conn.prepare("SELECT id, title, driving_text, source_ref FROM nodes WHERE kind = 'CodeUnit' ORDER BY id LIMIT ?1").map_err(|e| e.to_string())?;
    let rows: Vec<(String, Option<String>, Option<String>, Option<String>)> =
        stmt.query_map([PROJECT_CONTEXT_MAX_UNITS], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).map_err(|e| e.to_string())?.filter_map(Result::ok).collect();
    let mut out = String::new();
    for (id, title, driving_text, source_ref) in rows {
        let name = title.unwrap_or(id);
        let desc = driving_text.filter(|d| !d.is_empty()).or(source_ref).unwrap_or_else(|| "(no description)".to_string());
        out.push_str(&format!("- {name}: {desc}\n"));
    }
    Ok(out)
}

// ---- Build/Generate mode (rfcs/0014) -- the write surface over the
// same `CodeUnit` nodes `sync` populates from real code. Deliberately
// kept network/LLM-free, same as everything else in this module: the
// LLM calls that produce candidates/generated source live in
// `hi_llm.rs`, which calls back into these functions with plain data,
// so this module stays a pure, offline-testable graph store.

const CODE_UNIT_KINDS: &[&str] = &["fn", "struct", "enum", "screen"];

fn require_node_exists(conn: &Connection, id: &str) -> Result<(), String> {
    let exists: bool = conn.prepare("SELECT 1 FROM nodes WHERE id = ?1").and_then(|mut s| s.exists([id])).map_err(|e| e.to_string())?;
    if exists {
        Ok(())
    } else {
        Err(format!("no node `{id}` in the hi graph"))
    }
}

/// Prompt mode's own write path (rfcs/0014's "Build mode — the
/// interactive Hi graph"): inserts (or, on a name collision with a node
/// that already exists, refines the driving text of) one `CodeUnit`
/// candidate. No `.nir` exists yet, so only `driving_text`/`created_by`
/// are set here; `content_hash`/`source_ref` stay whatever they already
/// were (`NULL` for a genuinely new candidate) until Generate mode
/// writes real code and an ordinary `sync` picks it up under the exact
/// same id -- the same upsert `sync_file` already does, composing for
/// free because both write paths key off `code_unit_node_id`.
pub fn add_candidate(conn: &Connection, kind: &str, name: &str, driving_text: &str, created_by: &str) -> Result<String, String> {
    if !CODE_UNIT_KINDS.contains(&kind) {
        return Err(format!("`{kind}` isn't a legal CodeUnit kind -- one of {CODE_UNIT_KINDS:?}"));
    }
    let id = code_unit_node_id(kind, name);
    conn.execute(
        "INSERT INTO nodes (id, kind, title, status, content_hash, source_ref, driving_text, created_by)
         VALUES (?1, 'CodeUnit', ?2, NULL, NULL, NULL, ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET driving_text = excluded.driving_text, created_by = excluded.created_by",
        params![id, name, driving_text, created_by],
    )
    .map_err(|e| format!("upserting candidate node {id}: {e}"))?;
    Ok(id)
}

/// A structural relationship between two prompt-mode candidates --
/// deliberately a plain, undirected-in-spirit `RELATES_TO`, not
/// `IMPLEMENTS`/`IMPLEMENTED_BY` (RFC 0013's requirement<->code link
/// kind, a different relationship entirely). Silently a no-op on a
/// duplicate -- population re-running over a refined prompt shouldn't
/// error on an edge it already recorded.
pub fn add_relation(conn: &Connection, src: &str, dst: &str) -> Result<(), String> {
    conn.execute("INSERT INTO edges (src, dst, kind, hash_at_link, flag, flag_reason) VALUES (?1, ?2, 'RELATES_TO', NULL, NULL, NULL) ON CONFLICT(src, dst, kind) DO NOTHING", params![src, dst])
        .map_err(|e| format!("recording a RELATES_TO edge {src} -> {dst}: {e}"))?;
    Ok(())
}

/// Build mode's confirm action (rfcs/0014's "confirmation is an
/// explicit, dedicated action, never a side effect of anything else in
/// this list") -- the one-way promotion Generate mode's gate below
/// requires before a candidate's driving text is trusted enough to
/// reach the LLM again as something to compile.
pub fn confirm_node(conn: &Connection, id: &str) -> Result<(), String> {
    require_node_exists(conn, id)?;
    conn.execute("UPDATE nodes SET confirmed = 1 WHERE id = ?1", [id]).map_err(|e| format!("confirming {id}: {e}"))?;
    Ok(())
}

/// The bulk form of `confirm_node`: "everything is to be compiled" --
/// confirms every still-unconfirmed, non-waived `CodeUnit` in one call,
/// returning the ids it actually confirmed. An already-confirmed node
/// isn't touched; neither is a waived one -- waiving stays its own
/// explicit, separate decision (rfcs/0014's own definition), not
/// something a blanket confirm silently overrides.
pub fn confirm_all(conn: &Connection) -> Result<Vec<String>, String> {
    let mut stmt = conn.prepare("SELECT id FROM nodes WHERE kind = 'CodeUnit' AND confirmed = 0 AND waived = 0").map_err(|e| e.to_string())?;
    let ids: Vec<String> = stmt.query_map([], |r| r.get(0)).map_err(|e| e.to_string())?.filter_map(Result::ok).collect();
    conn.execute("UPDATE nodes SET confirmed = 1 WHERE kind = 'CodeUnit' AND confirmed = 0 AND waived = 0", []).map_err(|e| format!("confirming all: {e}"))?;
    Ok(ids)
}

/// Build mode's delete action -- removes a candidate (and every edge
/// touching it) outright, for content that's simply wrong, not worth
/// confirming.
pub fn delete_node(conn: &Connection, id: &str) -> Result<(), String> {
    require_node_exists(conn, id)?;
    conn.execute("DELETE FROM edges WHERE src = ?1 OR dst = ?1", [id]).map_err(|e| format!("deleting edges touching {id}: {e}"))?;
    conn.execute("DELETE FROM nodes WHERE id = ?1", [id]).map_err(|e| format!("deleting node {id}: {e}"))?;
    Ok(())
}

/// Edits a candidate's driving text. If the unit had already locked (an
/// earlier Generate pass materialized real `.nir` from it), this v1
/// slice unlocks it outright rather than RFC 0014's own finer-grained
/// `graph-edit-post-lock` edge flag (which would leave the last-known-
/// good `.nir` runnable while flagging the disagreement) -- a real,
/// disclosed simplification: the unit just needs regenerating before it
/// can publish again, same as any other unlocked unit.
pub fn edit_driving_text(conn: &Connection, id: &str, text: &str) -> Result<(), String> {
    require_node_exists(conn, id)?;
    conn.execute("UPDATE nodes SET driving_text = ?2, locked = 0 WHERE id = ?1", params![id, text]).map_err(|e| format!("editing {id}: {e}"))?;
    Ok(())
}

/// Build mode's attribute editor -- appends one free-text attribute
/// line (e.g. `requires(role: admin)`, `nfr(latency_ms: 200)`) to a
/// node's running list. Deliberately not parsed/validated against the
/// language's real annotation grammar here (RFC 0014's own Open
/// Question 5, "attribute-legality-per-kind," is disclosed as real,
/// unbuilt work) -- Generate mode's own compile step is what actually
/// proves an attribute is legal, by rejecting a generated program that
/// misuses one.
pub fn attach_attribute(conn: &Connection, id: &str, attr: &str) -> Result<(), String> {
    require_node_exists(conn, id)?;
    let existing: Option<String> = conn.query_row("SELECT attributes FROM nodes WHERE id = ?1", [id], |r| r.get(0)).map_err(|e| format!("reading {id}: {e}"))?;
    let merged = match existing {
        Some(s) if !s.is_empty() => format!("{s}\n{attr}"),
        _ => attr.to_string(),
    };
    conn.execute("UPDATE nodes SET attributes = ?2 WHERE id = ?1", params![id, merged]).map_err(|e| format!("attaching an attribute to {id}: {e}"))?;
    Ok(())
}

/// Generate mode's escape hatch for a unit that genuinely cannot lock
/// (rfcs/0014's "Generate mode"): marks it out-of-scope for this
/// publish with a required, audit-visible reason -- distinct from
/// deleting the requirement or hand-authoring `.nir` around it.
pub fn waive_node(conn: &Connection, id: &str, reason: &str) -> Result<(), String> {
    require_node_exists(conn, id)?;
    if reason.trim().is_empty() {
        return Err("a waive reason is required".to_string());
    }
    conn.execute("UPDATE nodes SET waived = 1, waive_reason = ?2 WHERE id = ?1", params![id, reason]).map_err(|e| format!("waiving {id}: {e}"))?;
    Ok(())
}

pub fn unwaive_node(conn: &Connection, id: &str) -> Result<(), String> {
    require_node_exists(conn, id)?;
    conn.execute("UPDATE nodes SET waived = 0, waive_reason = NULL WHERE id = ?1", [id]).map_err(|e| format!("unwaiving {id}: {e}"))?;
    Ok(())
}

/// One `CodeUnit` candidate, as Generate mode needs it: enough to build
/// a prompt from (`driving_text`/`attributes`) and enough to know which
/// real `fn`/`struct`/`enum`/`screen` declaration it should end up
/// matching (`kind`/`name`, parsed back out of the node id).
#[derive(Debug, Clone)]
pub struct CandidateUnit {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub driving_text: String,
    pub attributes: Vec<String>,
}

fn parse_code_unit_id(id: &str) -> Option<(String, String)> {
    let rest = id.strip_prefix("code:")?;
    let (kind, name) = rest.split_once(':')?;
    Some((kind.to_string(), name.to_string()))
}

/// Generate mode's own input set: every `CodeUnit` that's passed Build
/// mode's review gate (`confirmed = 1` -- rfcs/0014's own "a provisional
/// unit requires explicit human confirmation before it's eligible to
/// generate at all") and hasn't already locked or been waived out of
/// scope. `target` narrows to one specific node id; `None` means every
/// eligible node. A node `hi sync` found in real code (no
/// `driving_text` of its own) is never generatable, confirmed or not --
/// there's nothing here for the LLM to write from.
fn query_confirmed_units(conn: &Connection, where_extra: &str, target: Option<&str>) -> Result<Vec<CandidateUnit>, String> {
    let sql = format!("SELECT id, title, driving_text, attributes FROM nodes WHERE kind = 'CodeUnit' AND confirmed = 1 AND waived = 0 AND {where_extra} AND (?1 IS NULL OR id = ?1)");
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows: Vec<(String, Option<String>, Option<String>, Option<String>)> =
        stmt.query_map(params![target], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).map_err(|e| e.to_string())?.filter_map(Result::ok).collect();
    let mut units = Vec::new();
    for (id, title, driving_text, attributes) in rows {
        let Some((kind, name_from_id)) = parse_code_unit_id(&id) else { continue };
        let driving_text = driving_text.unwrap_or_default();
        if driving_text.trim().is_empty() {
            continue;
        }
        let name = title.unwrap_or(name_from_id);
        let attrs = attributes.map(|a| a.lines().map(str::to_string).collect()).unwrap_or_default();
        units.push(CandidateUnit { id, kind, name, driving_text, attributes: attrs });
    }
    Ok(units)
}

pub fn generatable_units(conn: &Connection, target: Option<&str>) -> Result<Vec<CandidateUnit>, String> {
    query_confirmed_units(conn, "locked = 0", target)
}

/// Every confirmed, in-scope `CodeUnit` regardless of lock state --
/// what a regenerate pass needs to send the LLM so an already-locked
/// unit's code stays represented in the one combined file this v1
/// slice regenerates each time (see `hi_llm::generate_program`'s own
/// "v1 scope cut" doc comment: one file for the whole confirmed set,
/// not incremental per-unit files), not just the newly-eligible subset
/// `generatable_units` reports.
pub fn confirmed_units(conn: &Connection, target: Option<&str>) -> Result<Vec<CandidateUnit>, String> {
    query_confirmed_units(conn, "1=1", target)
}

/// Marks a unit locked after Generate mode's whole-composed-program
/// build actually succeeds, recording the exact per-unit hash `sync`
/// just computed for it (called right after `sync`, so `content_hash`
/// already reflects the fresh AST) as `last_materialized_hash`. A
/// target the generated program didn't actually declare (the LLM
/// omitted it) has no post-sync `content_hash` to copy and is skipped,
/// not locked -- left for the caller to report.
pub fn lock_units_after_sync(conn: &Connection, ids: &[String]) -> Result<Vec<String>, String> {
    let mut locked = Vec::new();
    for id in ids {
        let hash: Option<String> = conn.query_row("SELECT content_hash FROM nodes WHERE id = ?1", [id], |r| r.get(0)).map_err(|e| format!("reading {id}: {e}"))?;
        if let Some(h) = hash {
            conn.execute("UPDATE nodes SET locked = 1, last_materialized_hash = ?2 WHERE id = ?1", params![id, h]).map_err(|e| format!("locking {id}: {e}"))?;
            locked.push(id.clone());
        }
    }
    Ok(locked)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("nirdosha_hi_graph_test_{name}_{}", std::process::id()));
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
        assert!(hi_dir(&dir).join("hi.db").is_file());
        assert!(hi_dir(&dir).join("content").is_dir());
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
        assert!(err.contains("hi sync"), "error should point at the fix, got: {err}");
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
    fn ask_finds_a_code_unit_by_name_even_when_the_question_is_natural_language() {
        let dir = scratch_dir("ask_code_unit");
        let conn = open(&dir).expect("open");
        add_candidate(&conn, "fn", "tick", "advances the game clock by one frame", "llm-prompt-mode").expect("add_candidate");

        let hits = ask(&conn, "what does tick do ?").expect("ask");
        assert!(hits.iter().any(|h| h.doc_id == "code:fn:tick"), "expected code:fn:tick among hits, got: {hits:?}");
    }

    #[test]
    fn ask_finds_a_synced_code_unit_with_no_driving_text_at_all() {
        let dir = scratch_dir("ask_synced_no_text");
        write_nir(&dir, "a.nir", "fn transfer_funds(amount: i64) -> i64 { return amount }\n");
        let conn = open(&dir).expect("open");
        sync(&conn, &dir, &[]).expect("sync");

        let hits = ask(&conn, "what does transfer_funds do").expect("ask");
        let hit = hits.iter().find(|h| h.doc_id == "code:fn:transfer_funds").expect("expected a hit for transfer_funds");
        assert!(hit.content.contains("a.nir"), "should point at the source file when there's no driving text, got: {}", hit.content);
    }

    #[test]
    fn ask_ignores_short_common_words() {
        let dir = scratch_dir("ask_short_words");
        let conn = open(&dir).expect("open");
        add_candidate(&conn, "fn", "add", "adds two numbers", "llm-prompt-mode").expect("add_candidate");

        // "do" is 2 characters and should be dropped rather than
        // matching every CodeUnit's driving text indiscriminately.
        let hits = ask(&conn, "do").expect("ask");
        assert!(hits.is_empty(), "a bare short word should match nothing, got: {hits:?}");
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

    #[test]
    fn a_struct_shadowing_a_prelude_name_with_different_fields_survives_filtering() {
        let dir = scratch_dir("prelude_shadow");
        // "Money" is a real prelude struct name (`ast::prelude_structs`)
        // -- redeclaring it with different fields must NOT be silently
        // dropped by prelude filtering, only the real, unmodified
        // prelude Money should ever be.
        write_nir(&dir, "a.nir", "struct Money { cents: i64 }\n");
        let conn = open(&dir).expect("open");
        let report = sync(&conn, &dir, &[]).expect("sync");
        assert_eq!(report.units_added, 1, "a user-defined Money with different fields than the real prelude Money must survive prelude filtering");
    }

    #[test]
    fn code_units_parse_path_matches_loader_for_an_import_free_file() {
        // `code_units_in_file` deliberately bypasses `loader::
        // load_program` (see this module's own doc comment) -- for a
        // file with no `use` directives, the two parse paths must still
        // agree item-for-item, or the hi graph's view of a project has quietly
        // diverged from what the compiler itself sees.
        let dir = scratch_dir("parse_path_parity");
        let path = write_nir(&dir, "a.nir", "fn add(a: i64, b: i64) -> i64 { return a + b }\nstruct Point { x: i64, y: i64 }\nenum Color { Red, Green, Blue }\n");
        let path_str = path.to_str().expect("utf8 path");

        let (loader_program, _src) = crate::loader::load_program(path_str).expect("loader parse");
        let prelude_s: HashSet<String> = crate::ast::prelude_structs().into_iter().map(|s| s.name).collect();
        let prelude_e: HashSet<String> = crate::ast::prelude_enums().into_iter().map(|e| e.name).collect();
        let mut expected: Vec<(&str, String)> = Vec::new();
        for f in &loader_program.fns {
            expected.push(("fn", f.name.clone()));
        }
        for s in &loader_program.structs {
            if !prelude_s.contains(&s.name) {
                expected.push(("struct", s.name.clone()));
            }
        }
        for e in &loader_program.enums {
            if !prelude_e.contains(&e.name) {
                expected.push(("enum", e.name.clone()));
            }
        }

        let units = code_units_in_file(&path).expect("code_units_in_file parse");
        let actual: Vec<(&str, String)> = units.iter().map(|u| (u.kind, u.qualified_name.clone())).collect();

        assert_eq!(actual.len(), expected.len(), "expected {expected:?}, got {actual:?}");
        for item in &expected {
            assert!(actual.contains(item), "missing {item:?} in {actual:?}");
        }
    }

    #[test]
    fn code_unit_span_is_captured_and_surfaced_through_impact() {
        let dir = scratch_dir("span_capture");
        write_nir(&dir, "a.nir", "fn first() -> i64 { return 1 }\nfn second() -> i64 { return 2 }\n");
        let conn = open(&dir).expect("open");
        sync(&conn, &dir, &[]).expect("sync");
        link(&conn, "R1", "fn:second").expect("link");

        let report = impact(&conn, "R1").expect("impact");
        let hit = report.hits.iter().find(|h| h.node_id == "code:fn:second").expect("second should be reachable");
        assert_eq!(hit.line, Some(2), "fn second is declared on line 2");
        assert!(hit.source_ref.as_deref().unwrap_or("").ends_with("a.nir"));
    }

    #[test]
    fn a_candidate_is_unconfirmed_and_ungeneratable_until_confirmed() {
        let dir = scratch_dir("candidate_confirm");
        let conn = open(&dir).expect("open");
        let id = add_candidate(&conn, "fn", "transfer_funds", "moves money between two accounts", "llm-prompt-mode").expect("add_candidate");
        assert_eq!(id, "code:fn:transfer_funds");
        assert!(generatable_units(&conn, None).expect("generatable_units").is_empty(), "an unconfirmed candidate must not be generatable");

        confirm_node(&conn, &id).expect("confirm_node");
        let units = generatable_units(&conn, None).expect("generatable_units");
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].kind, "fn");
        assert_eq!(units[0].name, "transfer_funds");
    }

    #[test]
    fn confirm_all_confirms_every_unconfirmed_candidate_but_not_a_waived_one() {
        let dir = scratch_dir("confirm_all");
        let conn = open(&dir).expect("open");
        let a = add_candidate(&conn, "fn", "add", "adds two numbers", "llm-prompt-mode").expect("add a");
        let b = add_candidate(&conn, "fn", "subtract", "subtracts two numbers", "llm-prompt-mode").expect("add b");
        let c = add_candidate(&conn, "fn", "risky", "does something risky", "llm-prompt-mode").expect("add c");
        confirm_node(&conn, &a).expect("pre-confirm a"); // already confirmed -- confirm_all must not choke on it
        waive_node(&conn, &c, "not needed").expect("waive c");

        let confirmed = confirm_all(&conn).expect("confirm_all");
        assert_eq!(confirmed, vec![b.clone()], "only the genuinely unconfirmed, non-waived candidate should be reported");

        let generatable: Vec<String> = generatable_units(&conn, None).expect("generatable_units").into_iter().map(|u| u.id).collect();
        assert!(generatable.contains(&a));
        assert!(generatable.contains(&b));
        assert!(!generatable.contains(&c), "a waived node must stay out of scope even after confirm_all");
    }

    #[test]
    fn add_candidate_rejects_an_unknown_kind() {
        let dir = scratch_dir("candidate_bad_kind");
        let conn = open(&dir).expect("open");
        let err = add_candidate(&conn, "trait", "Foo", "text", "llm-prompt-mode").unwrap_err();
        assert!(err.contains("trait"), "error should name the bad kind, got: {err}");
    }

    #[test]
    fn re_adding_a_candidate_refines_text_without_resetting_review_state() {
        let dir = scratch_dir("candidate_refine");
        let conn = open(&dir).expect("open");
        let id = add_candidate(&conn, "fn", "add", "adds two numbers", "llm-prompt-mode").expect("add_candidate");
        confirm_node(&conn, &id).expect("confirm");

        add_candidate(&conn, "fn", "add", "adds two i64 numbers and returns the sum", "llm-prompt-mode").expect("re-add");
        let units = generatable_units(&conn, None).expect("generatable_units");
        assert_eq!(units.len(), 1, "confirming must survive a refined re-population of the same candidate");
        assert_eq!(units[0].driving_text, "adds two i64 numbers and returns the sum");
    }

    #[test]
    fn waiving_requires_a_reason_and_removes_a_unit_from_the_generatable_set() {
        let dir = scratch_dir("candidate_waive");
        let conn = open(&dir).expect("open");
        let id = add_candidate(&conn, "fn", "risky", "does something risky", "llm-prompt-mode").expect("add_candidate");
        confirm_node(&conn, &id).expect("confirm");

        let err = waive_node(&conn, &id, "").unwrap_err();
        assert!(err.contains("reason"));

        waive_node(&conn, &id, "not needed for this publish").expect("waive");
        assert!(generatable_units(&conn, None).expect("generatable_units").is_empty());

        unwaive_node(&conn, &id).expect("unwaive");
        assert_eq!(generatable_units(&conn, None).expect("generatable_units").len(), 1);
    }

    #[test]
    fn editing_a_locked_unit_unlocks_it() {
        let dir = scratch_dir("candidate_edit_unlocks");
        let conn = open(&dir).expect("open");
        let id = add_candidate(&conn, "fn", "add", "adds two numbers", "llm-prompt-mode").expect("add_candidate");
        confirm_node(&conn, &id).expect("confirm");
        // Simulate a successful Generate pass without a real compiler run.
        write_nir(&dir, "a.nir", "fn add(a: i64, b: i64) -> i64 { return a + b }\n");
        sync(&conn, &dir, &[]).expect("sync");
        lock_units_after_sync(&conn, &[id.clone()]).expect("lock");
        assert!(generatable_units(&conn, None).expect("generatable_units").is_empty(), "a locked unit isn't generatable");

        edit_driving_text(&conn, &id, "adds two numbers, but faster").expect("edit");
        let units = generatable_units(&conn, None).expect("generatable_units");
        assert_eq!(units.len(), 1, "editing a locked unit's text must unlock it for regeneration");
    }

    #[test]
    fn confirmed_units_includes_locked_ones_but_generatable_units_does_not() {
        let dir = scratch_dir("candidate_confirmed_vs_generatable");
        let conn = open(&dir).expect("open");
        let locked_id = add_candidate(&conn, "fn", "add", "adds two numbers", "llm-prompt-mode").expect("add_candidate");
        let fresh_id = add_candidate(&conn, "fn", "subtract", "subtracts two numbers", "llm-prompt-mode").expect("add_candidate");
        confirm_node(&conn, &locked_id).expect("confirm");
        confirm_node(&conn, &fresh_id).expect("confirm");
        write_nir(&dir, "a.nir", "fn add(a: i64, b: i64) -> i64 { return a + b }\n");
        sync(&conn, &dir, &[]).expect("sync");
        lock_units_after_sync(&conn, &[locked_id.clone()]).expect("lock");

        let confirmed: Vec<String> = confirmed_units(&conn, None).expect("confirmed_units").into_iter().map(|u| u.id).collect();
        assert_eq!(confirmed.len(), 2, "confirmed_units must include the already-locked unit too");
        assert!(confirmed.contains(&locked_id));
        assert!(confirmed.contains(&fresh_id));

        let generatable: Vec<String> = generatable_units(&conn, None).expect("generatable_units").into_iter().map(|u| u.id).collect();
        assert_eq!(generatable, vec![fresh_id], "generatable_units must exclude the already-locked unit");
    }

    #[test]
    fn locking_only_covers_units_the_generated_program_actually_declared() {
        let dir = scratch_dir("candidate_partial_lock");
        let conn = open(&dir).expect("open");
        let declared = add_candidate(&conn, "fn", "add", "adds two numbers", "llm-prompt-mode").expect("add_candidate");
        let omitted = add_candidate(&conn, "fn", "subtract", "subtracts two numbers", "llm-prompt-mode").expect("add_candidate");
        confirm_node(&conn, &declared).expect("confirm");
        confirm_node(&conn, &omitted).expect("confirm");

        // The LLM's generated program only actually declared `add`.
        write_nir(&dir, "a.nir", "fn add(a: i64, b: i64) -> i64 { return a + b }\n");
        sync(&conn, &dir, &[]).expect("sync");
        let locked = lock_units_after_sync(&conn, &[declared.clone(), omitted.clone()]).expect("lock");

        assert_eq!(locked, vec![declared]);
        let still_generatable: Vec<String> = generatable_units(&conn, None).expect("generatable_units").into_iter().map(|u| u.id).collect();
        assert_eq!(still_generatable, vec![omitted]);
    }

    #[test]
    fn attach_attribute_appends_rather_than_overwrites() {
        let dir = scratch_dir("candidate_attach");
        let conn = open(&dir).expect("open");
        let id = add_candidate(&conn, "fn", "add", "adds two numbers", "llm-prompt-mode").expect("add_candidate");
        attach_attribute(&conn, &id, "requires(role: admin)").expect("attach 1");
        attach_attribute(&conn, &id, "nfr(latency_ms: 200)").expect("attach 2");

        let attrs: String = conn.query_row("SELECT attributes FROM nodes WHERE id = ?1", [&id], |r| r.get(0)).expect("read attributes");
        assert_eq!(attrs, "requires(role: admin)\nnfr(latency_ms: 200)");
    }

    #[test]
    fn delete_node_removes_it_and_its_edges() {
        let dir = scratch_dir("candidate_delete");
        let conn = open(&dir).expect("open");
        let a = add_candidate(&conn, "fn", "a", "does a", "llm-prompt-mode").expect("add a");
        let b = add_candidate(&conn, "fn", "b", "does b", "llm-prompt-mode").expect("add b");
        add_relation(&conn, &a, &b).expect("relate");

        delete_node(&conn, &a).expect("delete");
        let err = confirm_node(&conn, &a).unwrap_err();
        assert!(err.contains("no node"));
        let edge_count: i64 = conn.query_row("SELECT COUNT(*) FROM edges WHERE src = ?1 OR dst = ?1", [&a], |r| r.get(0)).expect("count edges");
        assert_eq!(edge_count, 0, "deleting a node must also delete edges touching it");
    }
}
