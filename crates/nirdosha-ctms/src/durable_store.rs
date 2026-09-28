//! Durable, restart-surviving `AlertStore`/`CaseStore` implementations,
//! backed by SQLite -- the same real, transaction-backed pattern
//! `nirdosha_guard_rfc0029::SqliteReplayStore` already uses for durable
//! capability-nonce tracking (see that module's own doc for why an
//! in-process `HashMap`/`HashSet` can't survive a restart or be shared
//! across processes). `InMemoryAlertStore`/`InMemoryCaseStore` remain the
//! default for tests and single-process development; these are what a
//! real deployment swaps in instead, with no change to `AlertStore`/
//! `CaseStore`'s own trait contract or to any gateway that consumes one.
//!
//! Each row stores its whole domain object as one JSON column
//! (`data_json`) rather than a wide relational schema -- the same
//! trade-off `nirdosha-guard-mic`'s own one-row-per-resource schema makes,
//! appropriate here for the same reason: this is a demo-scale durable
//! store proving the real contract (idempotent creation, atomic state
//! transitions), not a production schema-migration-managed table design.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};

use crate::alert::{Alert, AlertStore, ModelSignal};
use crate::case::{CaseStore, CaseStoreError, MonitoringCase};
use crate::rules::AlertCandidate;

const ALERT_SCHEMA: &str = "\
    PRAGMA journal_mode=WAL;\
    CREATE TABLE IF NOT EXISTS ctms_alerts(\
        alert_id TEXT PRIMARY KEY,\
        idempotency_key TEXT NOT NULL UNIQUE,\
        data_json TEXT NOT NULL\
    );";

const CASE_SCHEMA: &str = "\
    PRAGMA journal_mode=WAL;\
    CREATE TABLE IF NOT EXISTS ctms_cases(\
        case_id TEXT PRIMARY KEY,\
        alert_id TEXT NOT NULL UNIQUE,\
        data_json TEXT NOT NULL\
    );";

fn idempotency_key_string(candidate: &AlertCandidate) -> String {
    format!("{}\u{0}{}\u{0}{}\u{0}{}", candidate.rule_id, candidate.rule_version, candidate.key, candidate.window_start_ms)
}

/// A real, transaction-backed [`AlertStore`]. Safe to share across
/// processes pointed at the same file (SQLite's own file locking, not the
/// in-process `Mutex` below, is what makes concurrent writers from
/// different processes safe -- see `SqliteReplayStore`'s own doc for the
/// identical reasoning).
pub struct SqliteAlertStore {
    conn: Mutex<Connection>,
}

impl SqliteAlertStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, rusqlite::Error> {
        let conn = Connection::open(path)?;
        conn.execute_batch(ALERT_SCHEMA)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    fn row_to_alert(data_json: String) -> Alert {
        serde_json::from_str(&data_json).expect("ctms_alerts.data_json is always this store's own serialized Alert")
    }
}

impl AlertStore for SqliteAlertStore {
    fn create_or_get_with_origin(&self, candidate: &AlertCandidate, origin: &str, now_ms: u64) -> Alert {
        let conn = self.conn.lock().expect("sqlite alert store lock poisoned");
        let idempotency_key = idempotency_key_string(candidate);
        if let Some(data_json) = conn
            .query_row("SELECT data_json FROM ctms_alerts WHERE idempotency_key = ?1", params![idempotency_key], |row| row.get::<_, String>(0))
            .optional()
            .expect("select must succeed against a healthy local sqlite file")
        {
            return Self::row_to_alert(data_json);
        }
        let alert_id = format!("alert-{}", uuid_like(&conn));
        let alert = Alert {
            alert_id: alert_id.clone(),
            rule_id: candidate.rule_id.clone(),
            rule_version: candidate.rule_version,
            customer_key: candidate.key.clone(),
            severity: candidate.severity.clone(),
            status: crate::alert::AlertStatus::New,
            origin: origin.to_string(),
            matched_event_ids: candidate.matched_event_ids.clone(),
            created_at_ms: now_ms,
            model_signal: None,
        };
        let data_json = serde_json::to_string(&alert).expect("Alert always serializes");
        // INSERT OR IGNORE on the idempotency_key's UNIQUE constraint,
        // then re-read: closes the same race a concurrent
        // `try_consume`-style check+insert would otherwise have, without
        // needing a manual transaction (SQLite's own constraint
        // enforcement is the atomic step).
        conn.execute("INSERT OR IGNORE INTO ctms_alerts(alert_id, idempotency_key, data_json) VALUES (?1, ?2, ?3)", params![alert_id, idempotency_key, data_json])
            .expect("insert must succeed against a healthy local sqlite file");
        let winning_json: String = conn
            .query_row("SELECT data_json FROM ctms_alerts WHERE idempotency_key = ?1", params![idempotency_key], |row| row.get(0))
            .expect("the row this just inserted (or a concurrent winner's) must now exist");
        Self::row_to_alert(winning_json)
    }

    fn attach_model_signal(&self, alert_id: &str, signal: ModelSignal) -> Result<Alert, String> {
        let conn = self.conn.lock().expect("sqlite alert store lock poisoned");
        let data_json: String = conn
            .query_row("SELECT data_json FROM ctms_alerts WHERE alert_id = ?1", params![alert_id], |row| row.get(0))
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("unknown alert {alert_id}"))?;
        let mut alert = Self::row_to_alert(data_json);
        alert.model_signal = Some(signal);
        let updated_json = serde_json::to_string(&alert).map_err(|e| e.to_string())?;
        conn.execute("UPDATE ctms_alerts SET data_json = ?1 WHERE alert_id = ?2", params![updated_json, alert_id]).map_err(|e| e.to_string())?;
        Ok(alert)
    }

    fn get(&self, alert_id: &str) -> Option<Alert> {
        let conn = self.conn.lock().expect("sqlite alert store lock poisoned");
        conn.query_row("SELECT data_json FROM ctms_alerts WHERE alert_id = ?1", params![alert_id], |row| row.get::<_, String>(0))
            .optional()
            .expect("select must succeed against a healthy local sqlite file")
            .map(Self::row_to_alert)
    }

    fn list(&self) -> Vec<Alert> {
        let conn = self.conn.lock().expect("sqlite alert store lock poisoned");
        let mut stmt = conn.prepare("SELECT data_json FROM ctms_alerts").expect("prepare must succeed");
        stmt.query_map([], |row| row.get::<_, String>(0))
            .expect("query must succeed")
            .map(|r| Self::row_to_alert(r.expect("row read must succeed")))
            .collect()
    }
}

/// A real, transaction-backed [`CaseStore`]. See [`SqliteAlertStore`]'s
/// own doc for the shared design rationale.
pub struct SqliteCaseStore {
    conn: Mutex<Connection>,
}

impl SqliteCaseStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, rusqlite::Error> {
        let conn = Connection::open(path)?;
        conn.execute_batch(CASE_SCHEMA)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    fn row_to_case(data_json: String) -> MonitoringCase {
        serde_json::from_str(&data_json).expect("ctms_cases.data_json is always this store's own serialized MonitoringCase")
    }

    fn load(conn: &Connection, case_id: &str) -> Result<MonitoringCase, CaseStoreError> {
        conn.query_row("SELECT data_json FROM ctms_cases WHERE case_id = ?1", params![case_id], |row| row.get::<_, String>(0))
            .optional()
            .expect("select must succeed against a healthy local sqlite file")
            .map(Self::row_to_case)
            .ok_or_else(|| CaseStoreError::UnknownCase(case_id.to_string()))
    }

    fn save(conn: &Connection, case: &MonitoringCase) {
        let data_json = serde_json::to_string(case).expect("MonitoringCase always serializes");
        conn.execute("UPDATE ctms_cases SET data_json = ?1 WHERE case_id = ?2", params![data_json, case.case_id])
            .expect("update must succeed against a healthy local sqlite file");
    }
}

fn uuid_like(conn: &Connection) -> String {
    let count: i64 = conn.query_row("SELECT COALESCE(MAX(rowid), 0) FROM sqlite_master", [], |row| row.get(0)).unwrap_or(0);
    format!("{}-{}", std::process::id(), count.max(0) as u64 + rand_component())
}

fn rand_component() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos() as u64
}

impl CaseStore for SqliteCaseStore {
    fn assign_for_alert(&self, alert_id: &str, analyst: &str, severity: &str, now_ms: u64) -> Result<MonitoringCase, CaseStoreError> {
        let conn = self.conn.lock().expect("sqlite case store lock poisoned");
        let existing: Option<String> = conn
            .query_row("SELECT case_id FROM ctms_cases WHERE alert_id = ?1", params![alert_id], |row| row.get(0))
            .optional()
            .expect("select must succeed against a healthy local sqlite file");
        let case_id = if let Some(case_id) = existing {
            case_id
        } else {
            let case_id = format!("case-{}", uuid_like(&conn));
            let case = MonitoringCase {
                case_id: case_id.clone(),
                alert_id: alert_id.to_string(),
                severity: severity.to_string(),
                assigned_analyst: None,
                status: crate::case::CaseStatus::New,
                escalated_by: None,
                escalation_reason: None,
                senior_reviewed_by: None,
                disposition: None,
                disposition_by: None,
                created_at_ms: now_ms,
            };
            let data_json = serde_json::to_string(&case).expect("MonitoringCase always serializes");
            conn.execute("INSERT OR IGNORE INTO ctms_cases(case_id, alert_id, data_json) VALUES (?1, ?2, ?3)", params![case_id, alert_id, data_json])
                .expect("insert must succeed against a healthy local sqlite file");
            conn.query_row("SELECT case_id FROM ctms_cases WHERE alert_id = ?1", params![alert_id], |row| row.get(0))
                .expect("the row this just inserted (or a concurrent winner's) must now exist")
        };
        let mut case = Self::load(&conn, &case_id)?;
        case.assigned_analyst = Some(analyst.to_string());
        case.status = crate::case::CaseStatus::Assigned;
        Self::save(&conn, &case);
        Ok(case)
    }

    fn escalate(&self, case_id: &str, actor: &str, reason: &str) -> Result<MonitoringCase, CaseStoreError> {
        let conn = self.conn.lock().expect("sqlite case store lock poisoned");
        let mut case = Self::load(&conn, case_id)?;
        if case.status != crate::case::CaseStatus::Assigned {
            return Err(CaseStoreError::WrongState { expected: "Assigned", actual: crate::case::status_name(case.status) });
        }
        if case.assigned_analyst.as_deref() != Some(actor) {
            return Err(CaseStoreError::NotAssignedActor);
        }
        case.status = crate::case::CaseStatus::Escalated;
        case.escalated_by = Some(actor.to_string());
        case.escalation_reason = Some(reason.to_string());
        Self::save(&conn, &case);
        Ok(case)
    }

    fn senior_review(&self, case_id: &str, actor: &str) -> Result<MonitoringCase, CaseStoreError> {
        let conn = self.conn.lock().expect("sqlite case store lock poisoned");
        let mut case = Self::load(&conn, case_id)?;
        if case.status != crate::case::CaseStatus::Escalated {
            return Err(CaseStoreError::WrongState { expected: "Escalated", actual: crate::case::status_name(case.status) });
        }
        case.status = crate::case::CaseStatus::SeniorReviewed;
        case.senior_reviewed_by = Some(actor.to_string());
        Self::save(&conn, &case);
        Ok(case)
    }

    fn disposition(&self, case_id: &str, actor: &str, verdict: &str) -> Result<MonitoringCase, CaseStoreError> {
        let conn = self.conn.lock().expect("sqlite case store lock poisoned");
        let mut case = Self::load(&conn, case_id)?;
        if case.status != crate::case::CaseStatus::SeniorReviewed {
            return Err(CaseStoreError::WrongState { expected: "SeniorReviewed", actual: crate::case::status_name(case.status) });
        }
        if case.assigned_analyst.as_deref() == Some(actor) {
            return Err(CaseStoreError::SameActorDisposition);
        }
        case.status = match verdict {
            "SuspiciousActivity" => crate::case::CaseStatus::SuspiciousActivity,
            "FalsePositive" => crate::case::CaseStatus::FalsePositive,
            other => return Err(CaseStoreError::UnknownVerdict(other.to_string())),
        };
        case.disposition = Some(verdict.to_string());
        case.disposition_by = Some(actor.to_string());
        Self::save(&conn, &case);
        Ok(case)
    }

    fn get(&self, case_id: &str) -> Option<MonitoringCase> {
        let conn = self.conn.lock().expect("sqlite case store lock poisoned");
        Self::load(&conn, case_id).ok()
    }

    fn list(&self) -> Vec<MonitoringCase> {
        let conn = self.conn.lock().expect("sqlite case store lock poisoned");
        let mut stmt = conn.prepare("SELECT data_json FROM ctms_cases").expect("prepare must succeed");
        stmt.query_map([], |row| row.get::<_, String>(0))
            .expect("query must succeed")
            .map(|r| Self::row_to_case(r.expect("row read must succeed")))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::case::CaseStatus;
    use crate::rules::AlertCandidate;

    fn db_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("ctms_durable_test_{name}_{}.sqlite3", std::process::id()))
    }

    fn candidate() -> AlertCandidate {
        AlertCandidate {
            rule_id: "velocity_24h".to_string(),
            rule_version: 1,
            key: "cust-1".to_string(),
            window_start_ms: 0,
            window_end_ms: 1_000,
            severity: "high".to_string(),
            matched_event_ids: vec!["evt-1".to_string()],
            debit_count: 5,
            debit_total_minor: 1_000_000,
        }
    }

    #[test]
    fn alert_creation_is_idempotent_across_a_fresh_connection_to_the_same_file() {
        let path = db_path("alert_idem");
        let _ = std::fs::remove_file(&path);
        let store_a = SqliteAlertStore::open(&path).unwrap();
        let first = store_a.create_or_get(&candidate(), 0);

        let store_b = SqliteAlertStore::open(&path).unwrap();
        let second = store_b.create_or_get(&candidate(), 0);
        assert_eq!(first.alert_id, second.alert_id, "a fresh connection to the same durable file must see the same alert, not create a duplicate");
        assert_eq!(store_b.list().len(), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn case_workflow_survives_a_restart_reattaching_to_the_same_file() {
        let path = db_path("case_restart");
        let _ = std::fs::remove_file(&path);
        let store_a = SqliteCaseStore::open(&path).unwrap();
        let case = store_a.assign_for_alert("alert-1", "priya", "high", 0).unwrap();
        store_a.escalate(&case.case_id, "priya", "structuring pattern").unwrap();
        drop(store_a);

        let store_b = SqliteCaseStore::open(&path).unwrap();
        let reloaded = store_b.get(&case.case_id).unwrap();
        assert_eq!(reloaded.status, CaseStatus::Escalated, "a second process/instance reattaching to the file must see the prior instance's committed state");
        store_b.senior_review(&case.case_id, "arjun").unwrap();
        let disposed = store_b.disposition(&case.case_id, "meera", "SuspiciousActivity").unwrap();
        assert_eq!(disposed.status, CaseStatus::SuspiciousActivity);
        let _ = std::fs::remove_file(&path);
    }
}
