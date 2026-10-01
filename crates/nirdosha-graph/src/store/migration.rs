//! Explicit older-graph migration entry points are added here, never on read.
use super::*;

impl Graph {
    pub fn migrate(root: &Path, host: &Path, access: Access) -> Result<Self> {
        if !access.author {
            return Err(Error::new(
                "UNAUTHORIZED",
                "Migration requires author access",
            ));
        }
        let root = root.canonicalize()?;
        let path = root.join(".nir/hi.db");
        ensure_existing(&root, &path)?;
        let conn = Connection::open(&path)?;
        configure(&conn, true)?;
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='graph_meta')",
            [],
            |r| r.get(0),
        )?;
        if exists {
            return Self::open(&root, host, access);
        }
        let backup = root
            .join(".nir")
            .join(format!("hi.pre-0021.{}.db", id("backup")?));
        conn.backup(rusqlite::DatabaseName::Main, &backup, None)?;
        let integrity: String =
            Connection::open(&backup)?.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        if integrity != "ok" {
            return Err(Error::new(
                "DATABASE_INVALID",
                "Backup integrity check failed",
            ));
        }
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<()> {
            conn.execute_batch(SCHEMA)?;
            conn.execute(
                "INSERT INTO graph_meta VALUES(1,?1,?2,?3,0,NULL)",
                params![STORAGE_VERSION, id("p")?, id("ep")?],
            )?;
            // Preserve every prior row as versioned knowledge; no inferred acceptance.
            let mut stmt=conn.prepare("SELECT id,kind,title,source_ref,driving_text,plugin_origin,non_waivable FROM nodes")?;
            let rows = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, Option<String>>(5)?,
                        r.get::<_, i64>(6)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let mut aliases = BTreeMap::new();
            let mut changes = vec![];
            for (old, kind, title, source, description, plugin, sealed) in rows {
                let key = id("n")?;
                let kind = if kind == "CodeUnit" {
                    match old.split(':').nth(1) {
                        Some("fn") => "Function",
                        Some("struct") => "Struct",
                        Some("enum") => "Enum",
                        Some("screen") => "Screen",
                        _ => "Requirement",
                    }
                } else if schema::KINDS.contains(&kind.as_str()) {
                    &kind
                } else {
                    "Requirement"
                };
                let spec = if kind == "Requirement" {
                    json!({"text":description.clone().unwrap_or_default()})
                } else {
                    json!({})
                };
                let n = Node {
                    id: key.clone(),
                    kind: kind.into(),
                    title: title.unwrap_or_else(|| old.clone()),
                    symbol: None,
                    entity_revision: 1,
                    deleted: false,
                    origin: if plugin.is_some() { "plugin" } else { "source" }.into(),
                    spec_schema_version: schema::spec_version(kind),
                    spec,
                    provenance_ids: vec![],
                    source_refs: vec![
                        json!({"legacy_id":old,"source_path":source,"driving_text":description,"plugin_origin":plugin}),
                    ],
                    protected: sealed != 0,
                    observation: None,
                };
                let payload = serde_json::to_string(&n)?;
                conn.execute(
                    "INSERT INTO graph_entities VALUES('node',?1,1,?2)",
                    params![key, payload],
                )?;
                conn.execute(
                    "INSERT INTO graph_entity_versions VALUES('node',?1,1,?2)",
                    params![key, payload],
                )?;
                conn.execute(
                    "INSERT INTO graph_aliases VALUES(?1,?2,?3)",
                    params![old, source.unwrap_or_default(), key],
                )?;
                aliases.insert(old, key.clone());
                changes.push(json!({"entity_type":"node","id":key,"entity_revision":1,"value":n}));
            }
            let mut stmt = conn.prepare("SELECT id,src,dst,kind FROM edges ORDER BY id")?;
            let rows = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let mut canonical: BTreeMap<(String, String, String), String> = BTreeMap::new();
            for (old, mut src, mut dst, mut kind) in rows {
                if kind == "IMPLEMENTED_BY" {
                    std::mem::swap(&mut src, &mut dst);
                    kind = "IMPLEMENTS".into();
                }
                let (Some(src), Some(dst)) = (aliases.get(&src), aliases.get(&dst)) else {
                    continue;
                };
                let tuple = (src.clone(), dst.clone(), kind.clone());
                let key = if let Some(key) = canonical.get(&tuple) {
                    key.clone()
                } else {
                    let key = id("e")?;
                    let e = Edge {
                        id: key.clone(),
                        src: src.clone(),
                        dst: dst.clone(),
                        kind,
                        role: None,
                        payload: json!({}),
                        authority: "asserted".into(),
                        entity_revision: 1,
                        deleted: false,
                        provenance_ids: vec![],
                    };
                    let payload = serde_json::to_string(&e)?;
                    conn.execute(
                        "INSERT INTO graph_entities VALUES('edge',?1,1,?2)",
                        params![key, payload],
                    )?;
                    conn.execute(
                        "INSERT INTO graph_entity_versions VALUES('edge',?1,1,?2)",
                        params![key, payload],
                    )?;
                    changes.push(json!({"entity_type":"edge","id":key,"value":e}));
                    canonical.insert(tuple, key.clone());
                    key
                };
                conn.execute(
                    "INSERT INTO graph_aliases VALUES(?1,'',?2)",
                    params![format!("edge:{old}"), key],
                )?;
            }
            conn.execute("UPDATE graph_meta SET revision=1", [])?;
            conn.execute(
                "INSERT INTO graph_changes VALUES(1,?1)",
                [serde_json::to_string(
                    &json!({"revision":1,"changes":changes,"migration":true}),
                )?],
            )?;
            // Prevent stale executables from bypassing the service through old tables.
            for table in ["nodes", "edges", "chunks", "provenance", "plugins"] {
                let present: bool = conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                    [table],
                    |r| r.get(0),
                )?;
                if present {
                    for action in ["INSERT", "UPDATE", "DELETE"] {
                        conn.execute_batch(&format!("CREATE TRIGGER graph_protect_{table}_{action} BEFORE {action} ON {table} BEGIN SELECT RAISE(ABORT,'RFC 0021 graph migrated: use GraphService'); END;"))?;
                    }
                }
            }
            Ok(())
        })();
        match result {
            Ok(()) => conn.execute_batch("COMMIT")?,
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(e);
            }
        }
        let g = Self {
            conn,
            root,
            host: host.into(),
            access,
        };
        g.register_grant()?;
        g.install_binding(true)?;
        Ok(g)
    }
}
