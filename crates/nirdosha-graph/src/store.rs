//! SQLite transactions, immutable entity history, authorization and receipts.
use crate::{
    Error, Result, hash,
    schema::{self, Edge, Gap, Node, State},
};
use nirdosha_audit::crypto_backend::rand::{SecureRandom, SystemRandom};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub(crate) mod acceptance;
pub mod migration;
mod mutations;
mod reads;
mod streams;

pub const STORAGE_VERSION: u64 = 2;
pub const MAX_ENTITIES: usize = 100_000;
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub fn id(prefix: &str) -> Result<String> {
    let mut bytes = [0u8; 16];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| Error::new("STORAGE_UNAVAILABLE", "OS randomness unavailable"))?;
    Ok(format!(
        "{prefix}_{}",
        bytes.iter().map(|x| format!("{x:02x}")).collect::<String>()
    ))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Version {
    pub schema_version: String,
    pub project_id: String,
    pub epoch: String,
    pub revision: u64,
}

#[derive(Clone, Debug)]
pub struct Access {
    pub actor: String,
    pub grant_id: String,
    pub author: bool,
    pub accept: bool,
}
impl Access {
    pub fn read(actor: &str) -> Self {
        Self {
            actor: actor.into(),
            grant_id: format!("{actor}:read"),
            author: false,
            accept: false,
        }
    }
    pub fn author(actor: &str) -> Self {
        Self {
            actor: actor.into(),
            grant_id: format!("{actor}:author"),
            author: true,
            accept: false,
        }
    }
    pub fn reviewer(actor: &str) -> Self {
        Self {
            actor: actor.into(),
            grant_id: format!("{actor}:reviewer"),
            author: true,
            accept: true,
        }
    }
}

pub struct Graph {
    pub(crate) conn: Connection,
    pub(crate) root: PathBuf,
    pub(crate) host: PathBuf,
    pub(crate) access: Access,
}

impl Graph {
    /// Explicit initialization only. Existing unversioned databases require migration.
    pub fn initialize(root: &Path, host: &Path, access: Access) -> Result<Self> {
        if !access.author {
            return Err(Error::new(
                "UNAUTHORIZED",
                "Initialization requires author access",
            ));
        }
        std::fs::create_dir_all(root)?;
        let root = root.canonicalize()?;
        let dir = root.join(".nir");
        ensure_dir(&root, &dir)?;
        let db = dir.join("hi.db");
        if db.exists() {
            return Self::open(&root, host, access);
        }
        ensure_dir(&root, &dir.join("content"))?;
        let conn = Connection::open(&db)?;
        configure(&conn, true)?;
        conn.execute_batch(SCHEMA)?;
        let project = id("p")?;
        let epoch = id("ep")?;
        conn.execute(
            "INSERT INTO graph_meta VALUES (1,?1,?2,?3,0,NULL)",
            params![STORAGE_VERSION, project, epoch],
        )?;
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

    pub fn open(root: &Path, host: &Path, access: Access) -> Result<Self> {
        let root = root.canonicalize().map_err(|_| {
            Error::new(
                "PROJECT_NOT_INITIALIZED",
                "Project directory does not exist",
            )
        })?;
        let db = root.join(".nir/hi.db");
        if !db.exists() {
            return Err(Error::new("PROJECT_NOT_INITIALIZED", "No .nir/hi.db"));
        }
        ensure_existing(&root, &db)?;
        let flags = if access.author {
            OpenFlags::SQLITE_OPEN_READ_WRITE
        } else {
            OpenFlags::SQLITE_OPEN_READ_ONLY
        };
        let conn = Connection::open_with_flags(db, flags)?;
        let meta: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='graph_meta')",
            [],
            |r| r.get(0),
        )?;
        if !meta {
            let legacy: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='nodes')",
                [],
                |r| r.get(0),
            )?;
            return Err(Error::new(if legacy{"MIGRATION_REQUIRED"}else{"DATABASE_INVALID"},"Database has no versioned graph metadata").details(json!({"found_storage_version":null,"supported_storage_versions":[STORAGE_VERSION],"required_action":"migrate_legacy_hi_storage"})));
        }
        let version: u64 = conn
            .query_row(
                "SELECT storage_schema_version FROM graph_meta WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .map_err(|_| Error::new("DATABASE_INVALID", "Invalid graph metadata"))?;
        if version != STORAGE_VERSION {
            return Err(Error::new(if version<STORAGE_VERSION{"MIGRATION_REQUIRED"}else{"STORAGE_SCHEMA_UNSUPPORTED"},"Unsupported storage version").details(json!({"found_storage_version":version,"supported_storage_versions":[STORAGE_VERSION],"required_action":"use_compatible_service_or_migrate"})));
        }
        configure(&conn, access.author)?;
        let g = Self {
            conn,
            root,
            host: host.into(),
            access,
        };
        if g.access.author {
            g.install_binding(false)?;
            g.register_grant()?;
        }
        g.check_access(false)?;
        Ok(g)
    }

    /// Only the locally configured host calls this; no graph mutation can edit grants.
    fn register_grant(&self) -> Result<()> {
        self.conn.execute("INSERT OR IGNORE INTO graph_grants(id,actor,can_write,can_accept,revoked) VALUES(?1,?2,?3,?4,0)",params![self.access.grant_id,self.access.actor,self.access.author,self.access.accept])?;
        self.conn.execute("INSERT OR IGNORE INTO graph_grants(id,actor,can_write,can_accept,revoked) VALUES(?1,?2,0,0,0)",params![format!("{}:read",self.access.actor),self.access.actor])?;
        Ok(())
    }
    pub fn revoke_grant(&self, grant_id: &str) -> Result<()> {
        self.check_access(true)?;
        self.conn
            .execute("UPDATE graph_grants SET revoked=1 WHERE id=?1", [grant_id])?;
        Ok(())
    }
    pub(crate) fn check_access(&self, write: bool) -> Result<()> {
        let grant: Option<(String, bool, bool, bool)> = self
            .conn
            .query_row(
                "SELECT actor,can_write,can_accept,revoked FROM graph_grants WHERE id=?1",
                [&self.access.grant_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let Some((actor, w, _a, revoked)) = grant else {
            return Err(Error::new(
                "UNAUTHORIZED",
                "Grant not registered by project host",
            ));
        };
        if revoked {
            return Err(Error::new(
                "GRANT_REVOKED",
                "Project grant has been revoked",
            ));
        }
        if actor != self.access.actor || write && (!w || !self.access.author) {
            return Err(Error::new("UNAUTHORIZED", "Insufficient project grant"));
        }
        Ok(())
    }
    pub(crate) fn check_accept(&self) -> Result<()> {
        self.check_access(true)?;
        let yes: bool = self.conn.query_row(
            "SELECT can_accept FROM graph_grants WHERE id=?1",
            [&self.access.grant_id],
            |r| r.get(0),
        )?;
        if !yes || !self.access.accept {
            return Err(Error::new(
                "UNAUTHORIZED",
                "Promotion requires reviewer grant",
            ));
        }
        Ok(())
    }
    pub fn version(&self) -> Result<Version> {
        self.check_access(false)?;
        self.raw_version()
    }
    pub(crate) fn raw_version(&self) -> Result<Version> {
        Ok(self.conn.query_row(
            "SELECT project_id,epoch,revision FROM graph_meta WHERE singleton=1",
            [],
            |r| {
                Ok(Version {
                    schema_version: schema::GRAPH_SCHEMA.into(),
                    project_id: r.get(0)?,
                    epoch: r.get(1)?,
                    revision: r.get(2)?,
                })
            },
        )?)
    }
    fn host_connection(&self, create: bool) -> Result<Connection> {
        if create {
            std::fs::create_dir_all(&self.host)?;
        }
        let path = self.host.join("graph-bindings.db");
        if !path.exists() && !create {
            return Err(Error::new(
                "RESTORE_REQUIRED",
                "No independent host binding; use explicit restore/fork",
            ));
        }
        let c = Connection::open_with_flags(
            path,
            if create {
                OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
            } else {
                OpenFlags::SQLITE_OPEN_READ_WRITE
            },
        )?;
        c.busy_timeout(Duration::from_secs(2))?;
        c.execute_batch("PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS bindings(root TEXT PRIMARY KEY,project TEXT NOT NULL,epoch TEXT NOT NULL,revision INTEGER NOT NULL)")?;
        Ok(c)
    }
    fn install_binding(&self, create: bool) -> Result<()> {
        let c = self.host_connection(create)?;
        let v = self.raw_version()?;
        let path = self.root.to_string_lossy();
        let old: Option<(String, String, u64)> = c
            .query_row(
                "SELECT project,epoch,revision FROM bindings WHERE root=?1",
                [path.as_ref()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        if let Some((p, e, r)) = old {
            if p != v.project_id || e != v.epoch || r > v.revision {
                return Err(Error::new(
                    "RESTORE_REQUIRED",
                    "Host epoch/watermark disagrees with database",
                ));
            }
        } else if create {
            c.execute(
                "INSERT INTO bindings VALUES(?1,?2,?3,?4)",
                params![path, v.project_id, v.epoch, v.revision],
            )?;
        } else {
            return Err(Error::new(
                "RESTORE_REQUIRED",
                "Workspace has no host binding",
            ));
        }
        Ok(())
    }
    pub(crate) fn acknowledge(&self) -> Result<()> {
        self.install_binding(false)?;
        let c = self.host_connection(false)?;
        let v = self.raw_version()?;
        c.execute(
            "UPDATE bindings SET revision=MAX(revision,?2) WHERE root=?1 AND epoch=?3",
            params![self.root.to_string_lossy(), v.revision, v.epoch],
        )?;
        Ok(())
    }
    pub(crate) fn begin(&self) -> Result<()> {
        self.check_access(true)?;
        self.install_binding(false)?;
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        if let Err(e) = self.check_access(true) {
            let _ = self.conn.execute_batch("ROLLBACK");
            return Err(e);
        }
        Ok(())
    }
    pub(crate) fn transaction<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        self.begin()?;
        match f() {
            Ok(v) => {
                if let Err(e) = self
                    .check_access(true)
                    .and_then(|_| self.conn.execute_batch("COMMIT").map_err(Into::into))
                {
                    let _ = self.conn.execute_batch("ROLLBACK");
                    return Err(e);
                }
                self.acknowledge()?;
                Ok(v)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }
    pub(crate) fn load_state(&self, revision: Option<u64>) -> Result<State> {
        let revision = revision.unwrap_or(self.raw_version()?.revision);
        let mut stmt=self.conn.prepare("SELECT entity_type,payload FROM graph_entity_versions v WHERE revision=(SELECT MAX(revision) FROM graph_entity_versions WHERE entity_type=v.entity_type AND id=v.id AND revision<=?1) ORDER BY entity_type,id LIMIT ?2")?;
        let rows = stmt.query_map(params![revision, (MAX_ENTITIES + 1) as u64], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut s = State::default();
        let mut count = 0;
        for row in rows {
            let (t, v) = row?;
            count += 1;
            if count > MAX_ENTITIES {
                return Err(Error::new(
                    "LIMIT_EXCEEDED",
                    "Project entity quota exceeded",
                ));
            }
            match t.as_str() {
                "node" => {
                    let n: Node = serde_json::from_str(&v)?;
                    s.nodes.insert(n.id.clone(), n);
                }
                "edge" => {
                    let e: Edge = serde_json::from_str(&v)?;
                    s.edges.insert(e.id.clone(), e);
                }
                "gap" => {
                    let g: Gap = serde_json::from_str(&v)?;
                    s.gaps.insert(g.id.clone(), g);
                }
                _ => {
                    return Err(Error::new(
                        "DATABASE_INVALID",
                        "Unknown persisted entity type",
                    ));
                }
            }
        }
        Ok(s)
    }
    pub(crate) fn next_revision(&self) -> Result<u64> {
        let r = self.raw_version()?.revision;
        if r >= hash::MAX_INTEGER {
            return Err(Error::new("REVISION_EXHAUSTED", "Graph revision exhausted"));
        }
        Ok(r + 1)
    }
    pub(crate) fn persist_state(
        &self,
        before: &State,
        after: &mut State,
        revision: u64,
        mutation: &str,
    ) -> Result<Vec<Value>> {
        let mut changes = vec![];
        macro_rules! save {($field:ident,$kind:literal)=>{for(k,e)in &mut after.$field {
            if before.$field.get(k)!=Some(e){
                e.entity_revision=revision;
                let v=serde_json::to_value(&*e)?;let encoded=serde_json::to_string(&v)?;
                self.conn.execute("INSERT INTO graph_entity_versions VALUES(?1,?2,?3,?4)",params![$kind,k,revision,encoded])?;
                self.conn.execute("INSERT INTO graph_entities VALUES(?1,?2,?3,?4) ON CONFLICT(entity_type,id) DO UPDATE SET revision=excluded.revision,payload=excluded.payload",params![$kind,k,revision,encoded])?;
                changes.push(json!({"entity_type":$kind,"id":k,"entity_revision":revision,"value":v}));
            }
        }}}
        save!(nodes, "node");
        save!(edges, "edge");
        save!(gaps, "gap");
        if !changes.is_empty() {
            self.conn
                .execute("UPDATE graph_meta SET revision=?1", [revision])?;
            self.conn.execute(
                "INSERT INTO graph_changes VALUES(?1,?2)",
                params![
                    revision,
                    serde_json::to_string(
                        &json!({"revision":revision,"mutation_id":mutation,"changes":changes})
                    )?
                ],
            )?;
        }
        Ok(changes)
    }
    pub fn put_blob(&self, bytes: &[u8]) -> Result<String> {
        self.check_access(true)?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err(Error::new("LIMIT_EXCEEDED", "Artifact exceeds 4 MiB"));
        }
        let hash = hash::bytes(bytes);
        let dir = self.root.join(".nir/content");
        ensure_dir(&self.root, &dir)?;
        let target = dir.join(&hash);
        if target.exists() {
            ensure_existing(&self.root, &target)?;
            if std::fs::read(&target)? != bytes {
                return Err(Error::new("DATABASE_INVALID", "Artifact digest mismatch"));
            }
            return Ok(hash);
        }
        let temp = dir.join(id("tmp")?);
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)?;
        use std::io::Write;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temp, &target)?;
        std::fs::File::open(&dir)?.sync_all()?;
        Ok(hash)
    }
    pub fn blob(&self, hash: &str) -> Result<Vec<u8>> {
        self.check_access(false)?;
        if hash.len() != 64
            || !hash
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err(Error::new("SCHEMA_INVALID", "Invalid content hash"));
        }
        let path = self.root.join(".nir/content").join(hash);
        ensure_existing(&self.root, &path)?;
        let bytes = std::fs::read(path)?;
        if hash::bytes(&bytes) != hash {
            return Err(Error::new("DATABASE_INVALID", "Artifact digest mismatch"));
        }
        self.check_access(false)?;
        Ok(bytes)
    }
}

pub(crate) fn configure(c: &Connection, write: bool) -> Result<()> {
    c.busy_timeout(Duration::from_secs(2))?;
    c.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;")?;
    if write {
        c.execute_batch("PRAGMA journal_mode=WAL;")?;
    }
    Ok(())
}
pub(crate) fn ensure_existing(root: &Path, path: &Path) -> Result<()> {
    if !path.exists() {
        return Err(Error::new("NOT_FOUND", "Project artifact not found"));
    }
    if !path.canonicalize()?.starts_with(root) {
        return Err(Error::new("UNAUTHORIZED", "Path escapes project"));
    }
    Ok(())
}
pub(crate) fn ensure_dir(root: &Path, path: &Path) -> Result<()> {
    if path.exists() {
        ensure_existing(root, path)?;
    } else {
        if let Some(p) = path.parent() {
            ensure_existing(root, p)?;
        }
        std::fs::create_dir(path)?;
    }
    Ok(())
}

pub(crate) const SCHEMA: &str = r#"
CREATE TABLE graph_meta(singleton INTEGER PRIMARY KEY CHECK(singleton=1),storage_schema_version INTEGER NOT NULL,project_id TEXT NOT NULL,epoch TEXT NOT NULL,revision INTEGER NOT NULL,accepted_id TEXT);
CREATE TABLE graph_grants(id TEXT PRIMARY KEY,actor TEXT NOT NULL,can_write INTEGER NOT NULL,can_accept INTEGER NOT NULL,revoked INTEGER NOT NULL DEFAULT 0);
CREATE TABLE graph_entities(entity_type TEXT NOT NULL,id TEXT NOT NULL,revision INTEGER NOT NULL,payload TEXT NOT NULL,PRIMARY KEY(entity_type,id));
CREATE TABLE graph_entity_versions(entity_type TEXT NOT NULL,id TEXT NOT NULL,revision INTEGER NOT NULL,payload TEXT NOT NULL,PRIMARY KEY(entity_type,id,revision));
CREATE INDEX graph_version_revision ON graph_entity_versions(revision);
CREATE TABLE graph_receipts(actor TEXT NOT NULL,mutation_id TEXT NOT NULL,epoch TEXT NOT NULL,hash TEXT NOT NULL,binding TEXT NOT NULL,receipt TEXT NOT NULL,PRIMARY KEY(actor,mutation_id,epoch));
CREATE TABLE graph_changes(revision INTEGER PRIMARY KEY,payload TEXT NOT NULL);
CREATE TABLE graph_provenance(id TEXT PRIMARY KEY,revision INTEGER NOT NULL,actor TEXT NOT NULL,mutation_id TEXT NOT NULL,evidence TEXT NOT NULL);
CREATE TABLE graph_streams(id TEXT PRIMARY KEY,actor TEXT NOT NULL,epoch TEXT NOT NULL,client_key TEXT NOT NULL,next_sequence INTEGER NOT NULL,state TEXT NOT NULL,UNIQUE(actor,epoch,client_key));
CREATE TABLE graph_stream_receipts(stream_id TEXT NOT NULL REFERENCES graph_streams(id),sequence INTEGER NOT NULL,hash TEXT NOT NULL,receipt TEXT NOT NULL,PRIMARY KEY(stream_id,sequence));
CREATE TABLE graph_snapshots(id TEXT PRIMARY KEY,grant_id TEXT NOT NULL,epoch TEXT NOT NULL,revision INTEGER NOT NULL,view TEXT NOT NULL,created INTEGER NOT NULL,expires INTEGER NOT NULL);
CREATE TABLE graph_acceptances(id TEXT PRIMARY KEY,revision INTEGER NOT NULL,manifest TEXT NOT NULL);
CREATE TABLE graph_analysis_jobs(id TEXT PRIMARY KEY,grant_id TEXT NOT NULL,state TEXT NOT NULL,request TEXT NOT NULL,result TEXT,started INTEGER NOT NULL);
CREATE TABLE graph_aliases(alias TEXT NOT NULL,scope TEXT NOT NULL,id TEXT NOT NULL,PRIMARY KEY(alias,scope,id));
CREATE VIRTUAL TABLE graph_chunks_fts USING fts5(chunk_id UNINDEXED,entity_revision UNINDEXED,content);
"#;
