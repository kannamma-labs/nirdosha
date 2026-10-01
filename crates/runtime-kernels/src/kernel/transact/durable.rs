//! Safe Rust adapter over the native transaction log and instance lock.
//!
//! External operations MUST implement idempotency using the supplied txn_id.
//! A crash between an external effect and its log update necessarily retries
//! that operation. This is a recoverable saga, not distributed exactly-once.
use rusqlite::{OptionalExtension, params};
use serde_json::Value;
use std::path::Path;

/// An immutable protocol/version identifies the meanings of persisted inputs
/// and operations. Change it whenever recovery semantics become incompatible.
pub trait Saga {
    fn protocol(&self) -> &'static str;
    fn network(&mut self, txn_id: &str, input: &Value) -> Result<Value, String>;
    fn verify(&self, input: &Value, result: &Value) -> Result<bool, String>;
    fn commit(&mut self, txn_id: &str, input: &Value, result: &Value) -> Result<(), String>;
    fn compensate(&mut self, txn_id: &str, input: &Value, result: &Value) -> Result<(), String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Committed,
    Compensated,
}

pub struct DurableSaga<S> {
    connection: rusqlite::Connection,
    saga: S,
    _lock: super::super::instance_lock::InstanceLock,
}

impl<S: Saga> DurableSaga<S> {
    pub fn open(path: &Path, saga: S) -> Result<Self, String> {
        if saga.protocol().is_empty() {
            return Err("saga protocol/version is required".into());
        }
        let lock = super::super::instance_lock::InstanceLock::acquire(path)?;
        let connection = super::open_log(path)?;
        connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS nirdosha_transact_context (
                txn_id TEXT PRIMARY KEY REFERENCES nirdosha_transact_log(txn_id),
                protocol TEXT NOT NULL,
                input_json TEXT NOT NULL
            );",
            )
            .map_err(|e| e.to_string())?;
        Ok(Self {
            connection,
            saga,
            _lock: lock,
        })
    }

    pub fn execute(&mut self, txn_id: &str, input: &Value) -> Result<Outcome, String> {
        self.execute_observed(txn_id, input, |_| {})
    }

    /// Observer runs after each durable boundary, for fault-injection and
    /// telemetry. It cannot change persisted state or bypass transitions.
    pub fn execute_observed(
        &mut self,
        txn_id: &str,
        input: &Value,
        mut boundary: impl FnMut(&str),
    ) -> Result<Outcome, String> {
        if txn_id.is_empty() {
            return Err("transaction id is required".into());
        }
        let input_json = serde_json::to_string(input).map_err(|e| e.to_string())?;
        let tx = self.connection.transaction().map_err(|e| e.to_string())?;
        let existing: Option<(String, String)> = tx
            .query_row(
                "SELECT protocol, input_json FROM nirdosha_transact_context WHERE txn_id = ?",
                [txn_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        match existing {
            Some((protocol, original))
                if protocol != self.saga.protocol() || original != input_json =>
            {
                return Err(
                    "transaction id reused with different input or protocol version".into(),
                );
            }
            Some(_) => {}
            None => {
                let now = super::now_unix_secs();
                // Negative site ids are reserved for the safe adapter; native
                // numeric replay trampolines must not interpret their inputs.
                tx.execute("INSERT INTO nirdosha_transact_log (txn_id, site_id, state, created_at, updated_at) VALUES (?, -1, 'pending', ?, ?)", params![txn_id, now, now]).map_err(|e| e.to_string())?;
                tx.execute(
                    "INSERT INTO nirdosha_transact_context VALUES (?, ?, ?)",
                    params![txn_id, self.saga.protocol(), input_json],
                )
                .map_err(|e| e.to_string())?;
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
        boundary("pending");
        loop {
            let (state, result): (String, Option<String>) = self.connection.query_row(
                "SELECT state, network_result_json FROM nirdosha_transact_log WHERE txn_id = ? AND site_id = -1",
                [txn_id], |row| Ok((row.get(0)?, row.get(1)?))
            ).map_err(|e| e.to_string())?;
            match state.as_str() {
                "pending" => {
                    let result = self.saga.network(txn_id, input)?;
                    boundary("network_effect");
                    let json = serde_json::to_string(&result).map_err(|e| e.to_string())?;
                    self.transition(txn_id, "pending", "network_done", Some(&json))?;
                    boundary("network_done");
                }
                "network_done" => {
                    let result = decode_result(result)?;
                    let next = if self.saga.verify(input, &result)? {
                        "commit_pending"
                    } else {
                        "compensate_pending"
                    };
                    self.transition(txn_id, "network_done", next, None)?;
                    boundary(next);
                }
                "commit_pending" | "compensate_pending" => {
                    let result = decode_result(result)?;
                    let next = if state == "commit_pending" {
                        self.saga.commit(txn_id, input, &result)?;
                        boundary("commit_effect");
                        "committed"
                    } else {
                        self.saga.compensate(txn_id, input, &result)?;
                        boundary("compensate_effect");
                        "compensated"
                    };
                    self.transition(txn_id, &state, next, None)?;
                    boundary(next);
                }
                "committed" => return Ok(Outcome::Committed),
                "compensated" => return Ok(Outcome::Compensated),
                _ => return Err(format!("unsupported transaction state `{state}`")),
            }
        }
    }

    /// Call before accepting new business requests. Unknown protocol versions
    /// fail recovery rather than dispatching old rows to a new implementation.
    pub fn recover(&mut self) -> Result<Vec<(String, Outcome)>, String> {
        let pending = {
            let mut stmt = self
                .connection
                .prepare(
                    "SELECT c.txn_id, c.protocol, c.input_json FROM nirdosha_transact_context c
                 JOIN nirdosha_transact_log l ON c.txn_id = l.txn_id
                 WHERE l.state NOT IN ('committed', 'compensated') ORDER BY c.txn_id",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
        };
        if pending
            .iter()
            .any(|(_, protocol, _)| protocol != self.saga.protocol())
        {
            return Err("pending transaction belongs to a different protocol version".into());
        }
        let mut outcomes = Vec::new();
        for (id, _, input) in pending {
            let input = serde_json::from_str(&input).map_err(|e| e.to_string())?;
            outcomes.push((id.clone(), self.execute(&id, &input)?));
        }
        Ok(outcomes)
    }

    fn transition(
        &mut self,
        id: &str,
        from: &str,
        to: &str,
        result: Option<&str>,
    ) -> Result<(), String> {
        let changed = self.connection.execute(
            "UPDATE nirdosha_transact_log SET state = ?, network_result_json = COALESCE(?, network_result_json), updated_at = ? WHERE txn_id = ? AND state = ? AND site_id = -1",
            params![to, result, super::now_unix_secs(), id, from]
        ).map_err(|e| e.to_string())?;
        if changed != 1 {
            return Err("transaction transition did not update exactly one row".into());
        }
        Ok(())
    }
}

fn decode_result(result: Option<String>) -> Result<Value, String> {
    serde_json::from_str(&result.ok_or("transaction is missing its durable network result")?)
        .map_err(|e| e.to_string())
}
