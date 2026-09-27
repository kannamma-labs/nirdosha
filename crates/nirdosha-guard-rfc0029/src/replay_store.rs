//! Where [`crate::gateway::GatewayCore`] records a capability nonce as
//! consumed (single-use enforcement). This used to be a bare
//! `Mutex<HashSet<String>>` living only in one process's heap -- fine for
//! tests and a single-process deployment, but silently unsafe the moment a
//! gateway is deployed across multiple processes (or restarted): a nonce
//! consumed by process A is invisible to process B, or forgotten across a
//! restart, so the exact same capability could be replayed. This module
//! makes the store pluggable and adds a real transaction-backed
//! implementation for that case.

use std::fmt;

#[derive(Debug)]
pub enum ReplayStoreError {
    Unavailable(String),
}

impl fmt::Display for ReplayStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReplayStoreError::Unavailable(msg) => write!(f, "replay store unavailable: {msg}"),
        }
    }
}

/// `try_consume` must be atomic: under concurrent callers racing the same
/// `key` -- in this process, or, for a store durable enough to back
/// multiple processes, in any of them -- exactly one may observe
/// `Ok(true)`; every other caller must observe `Ok(false)`. A store failure
/// is `Err`, never silently treated as `Ok(true)`: `GatewayCore` fails
/// closed on it (see `GatewayError::ReplayStoreUnavailable`), the same
/// discipline this crate already applies to bundle and capability checks.
pub trait ReplayStore: Send + Sync {
    /// Atomically record `key` as consumed. `Ok(true)` = first time seen,
    /// the caller may proceed; `Ok(false)` = already consumed, a replay.
    fn try_consume(&self, key: &str) -> Result<bool, ReplayStoreError>;
}

mod in_memory {
    use super::{ReplayStore, ReplayStoreError};
    use std::collections::HashSet;
    use std::sync::Mutex;

    /// The same single-process, non-durable behavior `GatewayCore` had
    /// before this module existed. Still the right choice for tests and
    /// single-process development -- not a real option once a gateway must
    /// survive a restart or be shared across processes; see
    /// [`super::SqliteReplayStore`] for that.
    #[derive(Default)]
    pub struct InMemoryReplayStore {
        consumed: Mutex<HashSet<String>>,
    }

    impl InMemoryReplayStore {
        pub fn new() -> Self {
            Self::default()
        }
    }

    impl ReplayStore for InMemoryReplayStore {
        fn try_consume(&self, key: &str) -> Result<bool, ReplayStoreError> {
            let mut consumed = self.consumed.lock().expect("in-memory replay store lock poisoned");
            Ok(consumed.insert(key.to_string()))
        }
    }
}
pub use in_memory::InMemoryReplayStore;

mod sqlite {
    use super::{ReplayStore, ReplayStoreError};
    use rusqlite::Connection;
    use std::path::Path;
    use std::sync::Mutex;

    const SCHEMA: &str = "\
        PRAGMA journal_mode=WAL;\
        CREATE TABLE IF NOT EXISTS gateway_consumed_nonces(\
            nonce TEXT PRIMARY KEY,\
            consumed_at_ms INTEGER NOT NULL\
        );";

    fn store_error(e: rusqlite::Error) -> ReplayStoreError {
        ReplayStoreError::Unavailable(e.to_string())
    }

    /// A real, transaction-backed idempotency/replay store: `nonce` is the
    /// table's `PRIMARY KEY`, so `try_consume`'s `INSERT OR IGNORE` is one
    /// atomic statement -- SQLite's own file locking (not the
    /// `Mutex<Connection>` below) is what makes this safe across multiple
    /// processes sharing the same database file, which an in-process
    /// `HashSet` could never be. The `Mutex` here only serializes this
    /// *one process's* handle to that file.
    pub struct SqliteReplayStore {
        conn: Mutex<Connection>,
    }

    impl SqliteReplayStore {
        pub fn open(path: impl AsRef<Path>) -> Result<Self, ReplayStoreError> {
            let conn = Connection::open(path.as_ref()).map_err(store_error)?;
            conn.busy_timeout(std::time::Duration::from_secs(5)).map_err(store_error)?;
            conn.execute_batch(SCHEMA).map_err(store_error)?;
            Ok(Self { conn: Mutex::new(conn) })
        }
    }

    impl ReplayStore for SqliteReplayStore {
        fn try_consume(&self, key: &str) -> Result<bool, ReplayStoreError> {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let conn = self.conn.lock().expect("sqlite replay store connection lock poisoned");
            conn.execute(
                "INSERT OR IGNORE INTO gateway_consumed_nonces(nonce, consumed_at_ms) VALUES(?1, ?2)",
                rusqlite::params![key, now_ms],
            )
            .map(|rows_inserted| rows_inserted == 1)
            .map_err(store_error)
        }
    }
}
pub use sqlite::SqliteReplayStore;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use std::thread;

    fn scratch_db_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("rfc0029_replay_store_test_{name}_{}.sqlite3", std::process::id()))
    }

    #[test]
    fn in_memory_store_consumes_once() {
        let store = InMemoryReplayStore::new();
        assert_eq!(store.try_consume("nonce-1").unwrap(), true);
        assert_eq!(store.try_consume("nonce-1").unwrap(), false);
        assert_eq!(store.try_consume("nonce-2").unwrap(), true);
    }

    #[test]
    fn sqlite_store_consumes_once() {
        let path = scratch_db_path("consume_once");
        let _ = std::fs::remove_file(&path);
        let store = SqliteReplayStore::open(&path).unwrap();
        assert_eq!(store.try_consume("nonce-1").unwrap(), true);
        assert_eq!(store.try_consume("nonce-1").unwrap(), false);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn sqlite_store_survives_a_fresh_connection_to_the_same_file() {
        // The actual "durable" claim: a nonce consumed by one connection
        // (standing in for one process) is still consumed when a
        // completely separate `SqliteReplayStore` opens the same file
        // (standing in for a restart, or a second process) -- unlike the
        // in-memory store, which forgets everything the moment its owning
        // `GatewayCore` is dropped.
        let path = scratch_db_path("fresh_connection");
        let _ = std::fs::remove_file(&path);
        {
            let store = SqliteReplayStore::open(&path).unwrap();
            assert_eq!(store.try_consume("nonce-1").unwrap(), true);
        }
        {
            let reopened = SqliteReplayStore::open(&path).unwrap();
            assert_eq!(reopened.try_consume("nonce-1").unwrap(), false);
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn sqlite_store_consume_is_atomic_under_concurrency() {
        let path = scratch_db_path("concurrency");
        let _ = std::fs::remove_file(&path);
        let store = Arc::new(SqliteReplayStore::open(&path).unwrap());
        let successes = Arc::new(AtomicU64::new(0));
        let handles: Vec<_> = (0..16)
            .map(|_| {
                let store = store.clone();
                let successes = successes.clone();
                thread::spawn(move || {
                    if store.try_consume("shared-nonce").unwrap() {
                        successes.fetch_add(1, Ordering::SeqCst);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(successes.load(Ordering::SeqCst), 1, "exactly one racer may consume a given nonce");
        let _ = std::fs::remove_file(&path);
    }
}
