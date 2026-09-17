//! Host-owned operation identity and restart recovery, independent of provider IDs.
use crate::{Error, Graph, Result, hash, store::id};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::path::Path;

pub struct Journal {
    conn: Connection,
    pub session_id: String,
}
impl Journal {
    pub fn open(path: &Path, session_id: &str, graph: &Graph) -> Result<Self> {
        let c = Connection::open(path)?;
        c.busy_timeout(std::time::Duration::from_secs(2))?;
        c.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS sessions(id TEXT PRIMARY KEY,project TEXT NOT NULL,epoch TEXT NOT NULL,actor TEXT NOT NULL,stream TEXT NOT NULL,generation INTEGER NOT NULL,next_sequence INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS operations(session TEXT NOT NULL,task TEXT NOT NULL,sequence INTEGER NOT NULL,hash TEXT NOT NULL,patch TEXT NOT NULL,state TEXT NOT NULL,receipt TEXT,PRIMARY KEY(session,task));
            CREATE UNIQUE INDEX IF NOT EXISTS operation_sequence ON operations(session,sequence) WHERE state!='rejected';
            CREATE TABLE IF NOT EXISTS conversations(session TEXT PRIMARY KEY,payload TEXT NOT NULL);")?;
        let v = graph.version()?;
        let row: Option<(String, String, String)> = c
            .query_row(
                "SELECT project,epoch,actor FROM sessions WHERE id=?1",
                [session_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        if let Some((p, e, a)) = row {
            if p != v.project_id || e != v.epoch || a != graph.access.actor {
                return Err(Error::new(
                    "UNAUTHORIZED",
                    "Journal binding differs from graph session",
                ));
            }
        } else {
            let stream = graph.stream_open(&format!("journal:{session_id}"))?;
            c.execute(
                "INSERT INTO sessions VALUES(?1,?2,?3,?4,?5,1,1)",
                params![
                    session_id,
                    v.project_id,
                    v.epoch,
                    graph.access.actor,
                    stream["stream_id"].as_str().unwrap()
                ],
            )?;
        }
        Ok(Self {
            conn: c,
            session_id: session_id.into(),
        })
    }
    pub fn generation(&self) -> Result<u64> {
        Ok(self.conn.query_row(
            "SELECT generation FROM sessions WHERE id=?1",
            [&self.session_id],
            |r| r.get(0),
        )?)
    }
    /// Host-defined logical task identity, not the provider's changing tool-call ID.
    pub fn prepare(&self, generation: u64, task: &str, mut patch: Value) -> Result<Value> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| {
            if generation != self.generation()? {
                return Err(Error::new(
                    "REVISION_CONFLICT",
                    "Superseded provider attempt cannot dispatch",
                ));
            }
            patch
                .as_object_mut()
                .ok_or_else(|| Error::new("SCHEMA_INVALID", "Patch must be an object"))?
                .remove("mutation_id");
            let digest = hash::structured("logical-operation", &patch)?;
            let old: Option<(String, String)> = self
                .conn
                .query_row(
                    "SELECT hash,patch FROM operations WHERE session=?1 AND task=?2",
                    params![self.session_id, task],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some((h, p)) = old {
                if h != digest {
                    return Err(Error::new(
                        "IDEMPOTENCY_CONFLICT",
                        "Logical task changed; reconcile before allocating new intent",
                    ));
                }
                return Ok(serde_json::from_str(&p)?);
            }
            if !self.pending()?.is_empty() { return Err(Error::new("BACKPRESSURE","Recover pending operation before preparing another")); }
            let seq: u64 = self.conn.query_row(
                "SELECT next_sequence FROM sessions WHERE id=?1",
                [&self.session_id],
                |r| r.get(0),
            )?;
            if seq >= hash::MAX_INTEGER {
                return Err(Error::new(
                    "REVISION_EXHAUSTED",
                    "Journal sequence exhausted",
                ));
            }
            patch["mutation_id"] = id("mutation")?.into();
            self.conn.execute(
                "INSERT INTO operations VALUES(?1,?2,?3,?4,?5,'prepared',NULL)",
                params![
                    self.session_id,
                    task,
                    seq,
                    digest,
                    serde_json::to_string(&patch)?
                ],
            )?;
            self.conn.execute(
                "UPDATE sessions SET next_sequence=?2 WHERE id=?1",
                params![self.session_id, seq + 1],
            )?;
            Ok(patch)
        })();
        match result {
            Ok(v) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(v)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }
    pub fn pending(&self) -> Result<Vec<Value>> {
        let mut q=self.conn.prepare("SELECT task,sequence,patch,state FROM operations WHERE session=?1 AND state='prepared' ORDER BY sequence")?;
        let rows = q.query_map([&self.session_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        rows.map(|row|{let(t,s,p,state)=row?;Ok(json!({"task":t,"sequence":s,"patch":serde_json::from_str::<Value>(&p)?,"state":state}))}).collect()
    }
    pub fn stream_id(&self) -> Result<String> {
        Ok(self.conn.query_row(
            "SELECT stream FROM sessions WHERE id=?1",
            [&self.session_id],
            |r| r.get(0),
        )?)
    }
    pub fn recover(&self, graph: &Graph) -> Result<Vec<Value>> {
        // The journal transaction serializes dispatch/recovery leaders. No LLM wait here.
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| {
            let stream = self.stream_id()?;
            let mut results = vec![];
            for op in self.pending()? {
                let seq = op["sequence"].as_u64().unwrap();
                let receipt = match graph.stream_append(&stream, seq, &op["patch"]) {
                    Ok(v)=>v,
                    Err(e) if !e.retryable && matches!(e.code.as_str(), "SCHEMA_INVALID"|"REVISION_CONFLICT"|"PRECONDITION_REQUIRED"|"UNRESOLVED_REFERENCE"|"PROTECTED_ENTITY"|"GAP_TARGET_INVALID"|"LIMIT_EXCEEDED"|"NOT_FOUND") => {
                        // These errors definitively rolled back the graph transaction.
                        // Preserve the attempted payload and outcome; reclaim only this
                        // unconsumed sequence (one prepared operation is allowed).
                        self.conn.execute("UPDATE operations SET state='rejected',receipt=?3 WHERE session=?1 AND sequence=?2 AND state='prepared'",params![self.session_id,seq,e.envelope().to_string()])?;
                        self.conn.execute("UPDATE sessions SET next_sequence=?2 WHERE id=?1",params![self.session_id,seq])?;
                        self.conn.execute_batch("COMMIT")?;
                        return Err(e);
                    },
                    Err(e)=>return Err(e),
                };
                self.conn.execute("UPDATE operations SET state='acknowledged',receipt=?3 WHERE session=?1 AND sequence=?2",params![self.session_id,seq,serde_json::to_string(&receipt)?])?;
                results.push(receipt);
            }
            Ok(results)
        })();
        match result {
            Ok(v) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(v)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }
    pub fn next_attempt(&self) -> Result<u64> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| {
            if !self.pending()?.is_empty() {
                return Err(Error::new(
                    "REVISION_CONFLICT",
                    "Reconcile pending writes before starting another provider attempt",
                ));
            }
            let g = self.generation()?;
            if g >= hash::MAX_INTEGER {
                return Err(Error::new(
                    "REVISION_EXHAUSTED",
                    "Attempt generation exhausted",
                ));
            }
            self.conn.execute(
                "UPDATE sessions SET generation=?2 WHERE id=?1",
                params![self.session_id, g + 1],
            )?;
            Ok(g + 1)
        })();
        match result {
            Ok(v) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(v)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }
    pub fn conversation(&self)->Result<Option<Value>> {
        let text:Option<String>=self.conn.query_row("SELECT payload FROM conversations WHERE session=?1",[&self.session_id],|r|r.get(0)).optional()?;
        Ok(text.map(|s|serde_json::from_str(&s)).transpose()?)
    }
    pub fn save_conversation(&self,generation:u64,value:&Value)->Result<()> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result=(||{if self.generation()?!=generation{return Err(Error::new("REVISION_CONFLICT","Provider attempt was superseded"))}
            self.conn.execute("INSERT INTO conversations VALUES(?1,?2) ON CONFLICT(session) DO UPDATE SET payload=excluded.payload",params![self.session_id,value.to_string()])?;Ok(())})();
        match result{Ok(())=>{self.conn.execute_batch("COMMIT")?;Ok(())},Err(e)=>{let _=self.conn.execute_batch("ROLLBACK");Err(e)}}
    }
    pub fn checkpoint(&self) -> Result<Value> {
        let mut q=self.conn.prepare("SELECT task,receipt FROM operations WHERE session=?1 AND state='acknowledged' ORDER BY sequence")?;
        let rows = q.query_map([&self.session_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        let receipts = rows
            .map(|row| {
                let (t, r) = row?;
                Ok(json!({"task":t,"receipt":serde_json::from_str::<Value>(&r)?}))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(
            json!({"session_id":self.session_id,"generation":self.generation()?,"acknowledged":receipts,"pending":self.pending()?}),
        )
    }
}

/// NDJSON framing is a provider adapter, not an alternate MCP wire protocol.
#[derive(Default)]
pub struct Frames {
    buffer: Vec<u8>,
}
impl Frames {
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Value>> {
        if bytes.len() + self.buffer.len() > 2 * 1024 * 1024 {
            return Err(Error::new("BACKPRESSURE", "Frame queue exceeds 2 MiB"));
        }
        self.buffer.extend_from_slice(bytes);
        let mut out = vec![];
        while let Some(end) = self.buffer.iter().position(|b| *b == b'\n') {
            if end > 256 * 1024 {
                return Err(Error::new("LIMIT_EXCEEDED", "Frame exceeds 256 KiB"));
            }
            let line = self.buffer.drain(..=end).collect::<Vec<_>>();
            let line = std::str::from_utf8(&line)
                .map_err(|_| Error::new("SCHEMA_INVALID", "Invalid UTF-8 frame"))?;
            if line.trim().is_empty() {
                continue;
            }
            out.push(hash::parse(line)?);
            if out.len() > 8 {
                return Err(Error::new(
                    "BACKPRESSURE",
                    "More than 8 frames per dispatch",
                ));
            }
        }
        if self.buffer.len() > 256 * 1024 {
            return Err(Error::new(
                "LIMIT_EXCEEDED",
                "Incomplete frame exceeds 256 KiB",
            ));
        }
        Ok(out)
    }
    pub fn interrupted(&mut self) {
        self.buffer.clear();
    }
}
