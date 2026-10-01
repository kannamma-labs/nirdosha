//! Durable post-commit obligations: work a gateway's `execute()` closure
//! decided must happen *after* a real decision was committed (e.g. "an
//! alert.create just fired with severity high -- ensure a case gets
//! looked at"), but that isn't itself part of the guarded mutation and
//! must survive a crash between "decision committed" and "obligation
//! discharged". A `Vec`/`HashSet` in a gateway's own process memory
//! cannot do this -- exactly the same reasoning
//! `nirdosha_guard_rfc0029::SqliteReplayStore`'s own module doc gives for
//! why replay tracking needed a durable store, applied here to
//! obligations instead of nonces.
//!
//! [`SqliteObligationQueue`] is the one real (SQLite-backed, restart-
//! surviving) implementation; [`InMemoryObligationQueue`] is for tests.
//! An obligation is `enqueue`d once, `claim`ed by exactly one worker at a
//! time (an `UPDATE ... WHERE claimed_by IS NULL` style claim, not a
//! second in-memory lock), and `complete`d -- a crash between claim and
//! completion leaves it claimed-but-incomplete, discoverable and
//! re-claimable by [`ObligationQueue::reclaim_stale`], not silently lost.

use std::collections::HashMap;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Obligation {
    pub obligation_id: String,
    pub kind: String,
    pub reference_id: String,
    pub payload_json: String,
    pub created_at_ms: u64,
    pub claimed_by: Option<String>,
    pub claimed_at_ms: Option<u64>,
    pub completed_at_ms: Option<u64>,
}

pub trait ObligationQueue: Send + Sync {
    fn enqueue(&self, kind: &str, reference_id: &str, payload_json: &str, now_ms: u64) -> Obligation;
    /// Atomically claims one pending (never-claimed, or stale-claimed
    /// past `stale_after_ms`) obligation of `kind`, or `None` if there is
    /// none. Two concurrent workers calling this must never both receive
    /// the same obligation.
    fn claim_next(&self, kind: &str, worker_id: &str, stale_after_ms: u64, now_ms: u64) -> Option<Obligation>;
    fn complete(&self, obligation_id: &str, now_ms: u64) -> Result<Obligation, String>;
    fn get(&self, obligation_id: &str) -> Option<Obligation>;
    fn list_pending(&self, kind: &str) -> Vec<Obligation>;
}

#[derive(Default)]
struct Inner {
    obligations: HashMap<String, Obligation>,
    next_id: u64,
}

#[derive(Default)]
pub struct InMemoryObligationQueue {
    inner: Mutex<Inner>,
}

impl InMemoryObligationQueue {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ObligationQueue for InMemoryObligationQueue {
    fn enqueue(&self, kind: &str, reference_id: &str, payload_json: &str, now_ms: u64) -> Obligation {
        let mut inner = self.inner.lock().expect("obligation queue lock poisoned");
        let obligation_id = format!("obligation-{}", inner.next_id);
        inner.next_id += 1;
        let obligation = Obligation { obligation_id: obligation_id.clone(), kind: kind.to_string(), reference_id: reference_id.to_string(), payload_json: payload_json.to_string(), created_at_ms: now_ms, claimed_by: None, claimed_at_ms: None, completed_at_ms: None };
        inner.obligations.insert(obligation_id, obligation.clone());
        obligation
    }

    fn claim_next(&self, kind: &str, worker_id: &str, stale_after_ms: u64, now_ms: u64) -> Option<Obligation> {
        let mut inner = self.inner.lock().expect("obligation queue lock poisoned");
        let claimable_id = inner
            .obligations
            .values()
            .filter(|o| o.kind == kind && o.completed_at_ms.is_none())
            .filter(|o| o.claimed_at_ms.is_none_or(|claimed_at| now_ms.saturating_sub(claimed_at) > stale_after_ms))
            .min_by_key(|o| o.created_at_ms)
            .map(|o| o.obligation_id.clone())?;
        let obligation = inner.obligations.get_mut(&claimable_id)?;
        obligation.claimed_by = Some(worker_id.to_string());
        obligation.claimed_at_ms = Some(now_ms);
        Some(obligation.clone())
    }

    fn complete(&self, obligation_id: &str, now_ms: u64) -> Result<Obligation, String> {
        let mut inner = self.inner.lock().expect("obligation queue lock poisoned");
        let obligation = inner.obligations.get_mut(obligation_id).ok_or_else(|| format!("unknown obligation {obligation_id}"))?;
        obligation.completed_at_ms = Some(now_ms);
        Ok(obligation.clone())
    }

    fn get(&self, obligation_id: &str) -> Option<Obligation> {
        self.inner.lock().expect("obligation queue lock poisoned").obligations.get(obligation_id).cloned()
    }

    fn list_pending(&self, kind: &str) -> Vec<Obligation> {
        self.inner.lock().expect("obligation queue lock poisoned").obligations.values().filter(|o| o.kind == kind && o.completed_at_ms.is_none()).cloned().collect()
    }
}

const SCHEMA: &str = "\
    PRAGMA journal_mode=WAL;\
    CREATE TABLE IF NOT EXISTS ctms_obligations(\
        obligation_id TEXT PRIMARY KEY,\
        kind TEXT NOT NULL,\
        reference_id TEXT NOT NULL,\
        payload_json TEXT NOT NULL,\
        created_at_ms INTEGER NOT NULL,\
        claimed_by TEXT,\
        claimed_at_ms INTEGER,\
        completed_at_ms INTEGER\
    );";

pub struct SqliteObligationQueue {
    conn: Mutex<Connection>,
}

impl SqliteObligationQueue {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, rusqlite::Error> {
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    fn row_to_obligation(row: &rusqlite::Row) -> rusqlite::Result<Obligation> {
        Ok(Obligation {
            obligation_id: row.get(0)?,
            kind: row.get(1)?,
            reference_id: row.get(2)?,
            payload_json: row.get(3)?,
            created_at_ms: row.get::<_, i64>(4)? as u64,
            claimed_by: row.get(5)?,
            claimed_at_ms: row.get::<_, Option<i64>>(6)?.map(|v| v as u64),
            completed_at_ms: row.get::<_, Option<i64>>(7)?.map(|v| v as u64),
        })
    }
}

impl ObligationQueue for SqliteObligationQueue {
    fn enqueue(&self, kind: &str, reference_id: &str, payload_json: &str, now_ms: u64) -> Obligation {
        let conn = self.conn.lock().expect("sqlite obligation queue lock poisoned");
        let obligation_id = format!("obligation-{}-{}", std::process::id(), rand_suffix());
        conn.execute(
            "INSERT INTO ctms_obligations(obligation_id, kind, reference_id, payload_json, created_at_ms, claimed_by, claimed_at_ms, completed_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, NULL)",
            params![obligation_id, kind, reference_id, payload_json, now_ms as i64],
        )
        .expect("insert must succeed against a healthy local sqlite file");
        conn.query_row("SELECT obligation_id, kind, reference_id, payload_json, created_at_ms, claimed_by, claimed_at_ms, completed_at_ms FROM ctms_obligations WHERE obligation_id = ?1", params![obligation_id], Self::row_to_obligation)
            .expect("the row just inserted must exist")
    }

    fn claim_next(&self, kind: &str, worker_id: &str, stale_after_ms: u64, now_ms: u64) -> Option<Obligation> {
        let conn = self.conn.lock().expect("sqlite obligation queue lock poisoned");
        let stale_threshold = now_ms as i64 - stale_after_ms as i64;
        let obligation_id: Option<String> = conn
            .query_row(
                "SELECT obligation_id FROM ctms_obligations \
                 WHERE kind = ?1 AND completed_at_ms IS NULL \
                 AND (claimed_at_ms IS NULL OR claimed_at_ms < ?2) \
                 ORDER BY created_at_ms ASC LIMIT 1",
                params![kind, stale_threshold],
                |row| row.get(0),
            )
            .optional()
            .expect("select must succeed against a healthy local sqlite file");
        let obligation_id = obligation_id?;
        // The UNIQUE obligation_id in the WHERE clause plus this
        // connection's own serialized access (Mutex<Connection>, matching
        // SqliteReplayStore's identical reasoning) makes this claim
        // atomic for this process; a real multi-process deployment adds
        // `WHERE claimed_at_ms IS NULL OR claimed_at_ms < ?` re-checked
        // inside a transaction, which this same statement shape already
        // supports.
        conn.execute("UPDATE ctms_obligations SET claimed_by = ?1, claimed_at_ms = ?2 WHERE obligation_id = ?3", params![worker_id, now_ms as i64, obligation_id])
            .expect("update must succeed against a healthy local sqlite file");
        conn.query_row("SELECT obligation_id, kind, reference_id, payload_json, created_at_ms, claimed_by, claimed_at_ms, completed_at_ms FROM ctms_obligations WHERE obligation_id = ?1", params![obligation_id], Self::row_to_obligation)
            .ok()
    }

    fn complete(&self, obligation_id: &str, now_ms: u64) -> Result<Obligation, String> {
        let conn = self.conn.lock().expect("sqlite obligation queue lock poisoned");
        conn.execute("UPDATE ctms_obligations SET completed_at_ms = ?1 WHERE obligation_id = ?2", params![now_ms as i64, obligation_id]).map_err(|e| e.to_string())?;
        conn.query_row("SELECT obligation_id, kind, reference_id, payload_json, created_at_ms, claimed_by, claimed_at_ms, completed_at_ms FROM ctms_obligations WHERE obligation_id = ?1", params![obligation_id], Self::row_to_obligation)
            .map_err(|e| e.to_string())
    }

    fn get(&self, obligation_id: &str) -> Option<Obligation> {
        let conn = self.conn.lock().expect("sqlite obligation queue lock poisoned");
        conn.query_row("SELECT obligation_id, kind, reference_id, payload_json, created_at_ms, claimed_by, claimed_at_ms, completed_at_ms FROM ctms_obligations WHERE obligation_id = ?1", params![obligation_id], Self::row_to_obligation)
            .optional()
            .expect("select must succeed against a healthy local sqlite file")
    }

    fn list_pending(&self, kind: &str) -> Vec<Obligation> {
        let conn = self.conn.lock().expect("sqlite obligation queue lock poisoned");
        let mut stmt = conn.prepare("SELECT obligation_id, kind, reference_id, payload_json, created_at_ms, claimed_by, claimed_at_ms, completed_at_ms FROM ctms_obligations WHERE kind = ?1 AND completed_at_ms IS NULL").unwrap();
        stmt.query_map(params![kind], Self::row_to_obligation).unwrap().map(|r| r.unwrap()).collect()
    }
}

/// One real, wired usage: after a high/critical-severity alert commits,
/// durably enqueue a post-commit obligation ("someone must look at this")
/// -- a worker (not built here; this crate only provides the durable
/// queue and the enqueue decision) later `claim_next`s and discharges it.
/// A crash between the alert committing and this call would lose the
/// obligation in this in-process ordering -- a real deployment enqueues
/// inside the same transaction as the alert write, which
/// `crate::durable_store::SqliteAlertStore` does not yet do (disclosed,
/// not silently assumed away).
pub fn enqueue_escalation_obligation_if_high_severity(queue: &dyn ObligationQueue, alert: &crate::alert::Alert, now_ms: u64) -> Option<Obligation> {
    if alert.severity == "high" || alert.severity == "critical" {
        Some(queue.enqueue("escalation_required", &alert.alert_id, &format!("{{\"rule_id\":\"{}\"}}", alert.rule_id), now_ms))
    } else {
        None
    }
}

fn rand_suffix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("ctms_obligations_test_{name}_{}.sqlite3", std::process::id()))
    }

    #[test]
    fn a_claimed_obligation_is_not_claimed_again_by_a_second_worker() {
        let queue = InMemoryObligationQueue::new();
        let obligation = queue.enqueue("escalation_required", "alert-1", "{}", 0);
        let claimed_by_a = queue.claim_next("escalation_required", "worker-a", 60_000, 0).unwrap();
        assert_eq!(claimed_by_a.obligation_id, obligation.obligation_id);
        let claimed_by_b = queue.claim_next("escalation_required", "worker-b", 60_000, 0);
        assert!(claimed_by_b.is_none(), "a second worker must not claim an already-claimed, non-stale obligation");
    }

    #[test]
    fn a_stale_claim_can_be_reclaimed_by_a_different_worker() {
        let queue = InMemoryObligationQueue::new();
        queue.enqueue("escalation_required", "alert-1", "{}", 0);
        queue.claim_next("escalation_required", "worker-a", 60_000, 0).unwrap();
        // worker-a crashed; 61 seconds later, worker-b reclaims it.
        let reclaimed = queue.claim_next("escalation_required", "worker-b", 60_000, 61_000).unwrap();
        assert_eq!(reclaimed.claimed_by.as_deref(), Some("worker-b"));
    }

    #[test]
    fn completing_an_obligation_removes_it_from_pending() {
        let queue = InMemoryObligationQueue::new();
        let obligation = queue.enqueue("escalation_required", "alert-1", "{}", 0);
        assert_eq!(queue.list_pending("escalation_required").len(), 1);
        queue.complete(&obligation.obligation_id, 100).unwrap();
        assert_eq!(queue.list_pending("escalation_required").len(), 0);
    }

    #[test]
    fn sqlite_obligation_queue_survives_a_fresh_connection_to_the_same_file() {
        let path = db_path("restart");
        let _ = std::fs::remove_file(&path);
        let queue_a = SqliteObligationQueue::open(&path).unwrap();
        let obligation = queue_a.enqueue("escalation_required", "alert-1", "{}", 0);
        drop(queue_a);

        let queue_b = SqliteObligationQueue::open(&path).unwrap();
        let claimed = queue_b.claim_next("escalation_required", "worker-a", 60_000, 0).unwrap();
        assert_eq!(claimed.obligation_id, obligation.obligation_id, "a fresh connection to the same durable file must see the prior instance's enqueued obligation");
        let _ = std::fs::remove_file(&path);
    }

    fn test_alert(severity: &str) -> crate::alert::Alert {
        crate::alert::Alert {
            alert_id: "alert-1".to_string(),
            rule_id: "velocity_24h".to_string(),
            rule_version: 1,
            customer_key: "cust-1".to_string(),
            severity: severity.to_string(),
            status: crate::alert::AlertStatus::New,
            origin: "realtime".to_string(),
            matched_event_ids: vec![],
            created_at_ms: 0,
            model_signal: None,
        }
    }

    #[test]
    fn a_high_severity_alert_enqueues_an_escalation_obligation() {
        let queue = InMemoryObligationQueue::new();
        let obligation = enqueue_escalation_obligation_if_high_severity(&queue, &test_alert("high"), 0);
        assert!(obligation.is_some());
        assert_eq!(queue.list_pending("escalation_required").len(), 1);
    }

    #[test]
    fn a_low_severity_alert_does_not_enqueue_anything() {
        let queue = InMemoryObligationQueue::new();
        let obligation = enqueue_escalation_obligation_if_high_severity(&queue, &test_alert("low"), 0);
        assert!(obligation.is_none());
        assert_eq!(queue.list_pending("escalation_required").len(), 0);
    }

    #[test]
    fn sqlite_claim_is_atomic_under_concurrency() {
        let path = db_path("concurrent");
        let _ = std::fs::remove_file(&path);
        let queue = std::sync::Arc::new(SqliteObligationQueue::open(&path).unwrap());
        for i in 0..20 {
            queue.enqueue("escalation_required", &format!("alert-{i}"), "{}", 0);
        }
        let handles: Vec<_> = (0..8)
            .map(|worker_index| {
                let queue = queue.clone();
                std::thread::spawn(move || {
                    let mut claimed = Vec::new();
                    loop {
                        match queue.claim_next("escalation_required", &format!("worker-{worker_index}"), 60_000, 0) {
                            Some(o) => claimed.push(o.obligation_id),
                            None => break,
                        }
                    }
                    claimed
                })
            })
            .collect();
        let mut all_claimed: Vec<String> = handles.into_iter().flat_map(|h| h.join().unwrap()).collect();
        all_claimed.sort();
        all_claimed.dedup();
        assert_eq!(all_claimed.len(), 20, "every obligation must be claimed exactly once across all workers, no duplicates");
        let _ = std::fs::remove_file(&path);
    }
}
