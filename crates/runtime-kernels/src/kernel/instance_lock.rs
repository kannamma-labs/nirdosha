//! An OS-level exclusive lock on a SQLite-backed durability log's file,
//! held for the log's whole process lifetime, so a second process
//! accidentally pointed at the exact same log file fails fast at
//! startup with a clear error, instead of the two processes silently
//! interleaving writes to it.
//!
//! **Ported from `crates/compiler/src/instance_lock.rs`, not
//! reinvented** — that copy is dead code today: it lives in the
//! compiler crate, which is never linked into a compiled `.nir`
//! binary's own output (only this crate, `runtime-kernels`, is). It
//! was written for the now-deleted interpreter's `nirdosha serve` and
//! never had a real caller since. This is the same design, same
//! mechanism, actually wired up this time — `kernel::transact`'s log
//! init is its first real caller. The old copy should be deleted once
//! this lands (tracked, not done silently in the same change).
//!
//! ## What this deliberately does NOT solve
//!
//! Two replicas each with their OWN independent local file (the real
//! horizontal-scaling wall) never touch the same filesystem path at
//! all — there is nothing for a local file lock to contend on. This
//! module only catches the narrower, same-host "started twice"
//! accident. A real multi-instance deployment needs a shared Postgres-
//! backed durability log instead (real, separate follow-up work,
//! `docs/adr/0009-transact-durability-and-replay.md`).
//!
//! ## Mechanism
//!
//! SQLite's own `PRAGMA locking_mode=EXCLUSIVE`, on a tiny sidecar file
//! (`<log path>.lock`) separate from the log's own connection/pragmas.
//! No `busy_timeout` is set on this connection, on purpose: a startup
//! lock conflict should fail immediately and loudly, not hang retrying.

use rusqlite::Connection;

/// Held only for its `Drop` impl (closing the connection releases the
/// OS-level lock) — never queried again once `acquire` returns `Ok`.
pub struct InstanceLock(#[allow(dead_code)] Connection);

// SAFETY: same reasoning as the original module — `InstanceLock` never
// touches the inner `Connection` again after construction, held purely
// for `Drop`; no second reference to it is ever created, so there's
// nothing for `Connection`'s non-`Sync` interior `RefCell`s to race on.
unsafe impl Sync for InstanceLock {}

impl InstanceLock {
    /// `log_path` is the durability log's own file path — the lock file
    /// itself lives alongside it at `<log_path>.lock`.
    pub fn acquire(log_path: &std::path::Path) -> Result<Self, String> {
        let lock_path = {
            let mut s = log_path.as_os_str().to_owned();
            s.push(".lock");
            std::path::PathBuf::from(s)
        };
        let conn = Connection::open(&lock_path).map_err(|e| format!("durability log lock I/O error opening {}: {e}", lock_path.display()))?;
        conn.pragma_update(None, "locking_mode", "EXCLUSIVE").map_err(|e| format!("durability log lock I/O error at {}: {e}", lock_path.display()))?;
        conn.execute("CREATE TABLE IF NOT EXISTS nirdosha_instance_lock (held_at INTEGER)", []).map_err(|e| busy_to_message(e, log_path, &lock_path))?;
        Ok(InstanceLock(conn))
    }
}

fn busy_to_message(e: rusqlite::Error, log_path: &std::path::Path, lock_path: &std::path::Path) -> String {
    let is_busy = matches!(&e, rusqlite::Error::SqliteFailure(err, _) if err.code == rusqlite::ErrorCode::DatabaseBusy);
    if is_busy {
        format!(
            "another nirdosha instance already holds the durability log at {} (lock file {}) -- \
             a second process pointed at the same file is refused here, to avoid the two silently \
             diverging. Stop the other instance, or for a real multi-instance deployment point the \
             durability log at a shared Postgres database instead of a local file.",
            log_path.display(),
            lock_path.display(),
        )
    } else {
        format!("durability log lock I/O error at {}: {e}", lock_path.display())
    }
}

/// One process-wide slot for the transact log's own held lock — `Some`
/// once `kernel::transact::nir_transact_log_init` has successfully
/// acquired it, kept alive (never dropped) for the process's whole
/// lifetime.
static HELD: std::sync::OnceLock<InstanceLock> = std::sync::OnceLock::new();

/// `true` if the lock was newly acquired (or already held by this same
/// call — idempotent, since `nir_transact_log_init` could in principle
/// run more than once in a test binary), `false` if a *different*
/// process already holds it.
pub fn acquire(log_path: &std::path::Path) -> bool {
    if HELD.get().is_some() {
        return true;
    }
    match InstanceLock::acquire(log_path) {
        Ok(lock) => {
            let _ = HELD.set(lock);
            true
        }
        Err(msg) => {
            eprintln!("nirdosha: {msg}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_lock_on_the_same_path_is_refused_while_the_first_is_held() {
        let dir = std::env::temp_dir().join(format!("nirdosha_instance_lock_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log_path = dir.join("workflow.db");

        let first = InstanceLock::acquire(&log_path).expect("first lock must succeed");
        let second = InstanceLock::acquire(&log_path);
        let msg = match second {
            Err(m) => m,
            Ok(_) => panic!("a second lock on the same path must be refused while the first is held"),
        };
        assert!(msg.contains("already holds"), "error should explain why, got: {msg}");

        drop(first);
        let third = InstanceLock::acquire(&log_path);
        assert!(third.is_ok(), "once the first lock is dropped, a new lock on the same path must succeed");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn locks_on_different_paths_never_conflict() {
        let dir = std::env::temp_dir().join(format!("nirdosha_instance_lock_test_diff_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = InstanceLock::acquire(&dir.join("a.db"));
        let b = InstanceLock::acquire(&dir.join("b.db"));
        assert!(a.is_ok() && b.is_ok(), "independent paths must never contend with each other");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
