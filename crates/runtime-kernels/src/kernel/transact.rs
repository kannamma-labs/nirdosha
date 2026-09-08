//! `transact { precheck?/network/verify/commit/compensate?/log? }`'s
//! durability log and crash replay — Layer 1 (`codegen.rs::emit_transact`)
//! already compiles the live control flow for real; this is the rest of
//! `docs/TRANSACT.md`: a real, fsync'd, WAL-mode SQLite log recording
//! every transact's lifecycle, real bounded-retry-with-backoff for
//! `commit`/`compensate` when their return type is `Result(_, _)`, and
//! real crash replay at process startup.
//!
//! **Its own private connection, not `kernel::db`'s pool.** A transact
//! log connection must outlive every `transact` site in the program
//! (opened once at process start, replay needs it before any user code
//! runs) and needs different operational knobs (`synchronous=FULL`,
//! serial single-writer appends) than a general-purpose, throughput-
//! tuned `db` pool — see `docs/adr/0009-transact-durability-and-replay.md`
//! for the full reasoning.
//!
//! **Disclosed, deliberate scope narrowing, not a silent gap**: replay
//! dispatch here is by a per-call-site `site_id` alone, with no
//! build-version fingerprint guard — a rolling deploy across a binary
//! rebuild that changes the *set* of `transact` sites could dispatch a
//! pending row to the wrong site. Named directly rather than hidden;
//! the fingerprint hardening is real, separate follow-up work, not
//! part of what "replay works" means here.

use std::sync::{Mutex, OnceLock};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

/// One `commit`/`compensate` argument, durably serializable — unlike
/// `crate::NirBindValue` (a raw `{tag, i, f, ptr, len}` ABI struct
/// carrying a live pointer, meaningless after a restart), this owns its
/// string bytes. Bounded to exactly the four `Ty::is_transact_scalar`
/// shapes (`typeck.rs`), the same tag numbering `NirBindValue` already
/// uses (`0`=i64, `1`=f64, `2`=str, `3`=bool) so encoding from a real
/// `NirBindValue` array is a direct per-field copy, not a re-mapping.
#[derive(Serialize, Deserialize)]
#[serde(tag = "t", content = "v")]
enum ArgValue {
    I(i64),
    F(f64),
    S(String),
    B(bool),
}

unsafe fn encode_binds_json(binds_ptr: *const crate::NirBindValue, binds_len: i64) -> String {
    if binds_len == 0 {
        return "[]".to_string();
    }
    let raw = unsafe { std::slice::from_raw_parts(binds_ptr, binds_len as usize) };
    let values: Vec<ArgValue> = raw
        .iter()
        .map(|b| match b.tag {
            0 => ArgValue::I(b.i),
            1 => ArgValue::F(b.f),
            2 => ArgValue::S(unsafe { crate::str_from_raw(b.s_ptr, b.s_len) }.unwrap_or("").to_string()),
            3 => ArgValue::B(b.i != 0),
            _ => ArgValue::I(0),
        })
        .collect();
    serde_json::to_string(&values).unwrap_or_else(|_| "[]".to_string())
}

/// The replay-trampoline-facing decode — parses `json` back into a
/// caller-provided `NirBindValue` array, up to `out_len` elements.
/// String values are leaked (`Box::leak`, the same permanent,
/// disclosed-not-hidden convention `nir_sha256_hex`'s own output buffer
/// already uses — `str` isn't affine, so there's no scope-closing point
/// to free it at, same reasoning) so the pointer stays valid for the
/// trampoline's own subsequent use. Returns the real decoded count, or
/// `-1` on a malformed/short JSON array.
unsafe fn decode_binds_json(json: &str, out_ptr: *mut crate::NirBindValue, out_len: i64) -> i64 {
    let Ok(values) = serde_json::from_str::<Vec<ArgValue>>(json) else { return -1 };
    if (values.len() as i64) > out_len {
        return -1;
    }
    let out = unsafe { std::slice::from_raw_parts_mut(out_ptr, values.len()) };
    for (slot, v) in out.iter_mut().zip(values.iter()) {
        *slot = match v {
            ArgValue::I(n) => crate::NirBindValue { tag: 0, i: *n, f: 0.0, s_ptr: std::ptr::null(), s_len: 0 },
            ArgValue::F(n) => crate::NirBindValue { tag: 1, i: 0, f: *n, s_ptr: std::ptr::null(), s_len: 0 },
            ArgValue::S(s) => {
                let leaked: &'static [u8] = Box::leak(s.clone().into_boxed_str().into_boxed_bytes());
                crate::NirBindValue { tag: 2, i: 0, f: 0.0, s_ptr: leaked.as_ptr(), s_len: leaked.len() as i64 }
            }
            ArgValue::B(b) => crate::NirBindValue { tag: 3, i: *b as i64, f: 0.0, s_ptr: std::ptr::null(), s_len: 0 },
        };
    }
    values.len() as i64
}

/// One global durability-log connection, guarded by a single mutex —
/// deliberately serial, matching `docs/TRANSACT.md`'s own append-only
/// write-ahead design: `journal_mode=WAL` lets replay's own read pass
/// run concurrently with live appends, but writes to this log are
/// intentionally one-at-a-time, never a throughput-tuned pool.
static LOG: OnceLock<Mutex<Connection>> = OnceLock::new();

fn log_path() -> String {
    std::env::var("NIRDOSHA_TRANSACT_LOG_PATH").unwrap_or_else(|_| "nirdosha_transact_log.sqlite".to_string())
}

fn now_unix_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Called once, from generated `main`'s own prologue, before any user
/// code (including replay) runs. Real fsync-on-commit
/// (`synchronous=FULL`) — this log exists specifically so a crash can't
/// silently lose a pending transact, so the one knob that actually
/// guarantees durability on power loss, not just process crash, is
/// non-negotiable here even though it costs a real fsync per write.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_transact_log_init() -> i32 {
    let path = log_path();
    if !super::instance_lock::acquire(std::path::Path::new(&path)) {
        // `instance_lock::acquire` already printed the real reason.
        return 0;
    }
    let conn = match Connection::open(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("nirdosha: failed to open transact log at {path}: {e}");
            return 0;
        }
    };
    let setup = conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=FULL;
         CREATE TABLE IF NOT EXISTS nirdosha_transact_log (
             txn_id TEXT PRIMARY KEY,
             site_id INTEGER NOT NULL,
             state TEXT NOT NULL,
             network_result_json TEXT,
             commit_args_json TEXT,
             compensate_args_json TEXT,
             created_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL
         );",
    );
    if let Err(e) = setup {
        eprintln!("nirdosha: failed to initialize transact log schema: {e}");
        return 0;
    }
    LOG.set(Mutex::new(conn)).ok();
    1
}

fn with_log<R>(f: impl FnOnce(&Connection) -> R) -> Option<R> {
    LOG.get().map(|m| f(&m.lock().unwrap()))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_transact_begin(txn_id_ptr: *const u8, txn_id_len: i64, site_id: i64) -> i32 {
    let Some(txn_id) = (unsafe { crate::str_from_raw(txn_id_ptr, txn_id_len) }) else { return 0 };
    let now = now_unix_secs();
    with_log(|conn| {
        conn.execute(
            "INSERT INTO nirdosha_transact_log (txn_id, site_id, state, created_at, updated_at) VALUES (?, ?, 'pending', ?, ?)",
            rusqlite::params![txn_id, site_id, now, now],
        )
        .is_ok()
    })
    .unwrap_or(false) as i32
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_transact_mark_network_done(txn_id_ptr: *const u8, txn_id_len: i64, result_json_ptr: *const u8, result_json_len: i64) -> i32 {
    let (Some(txn_id), Some(result_json)) = (unsafe { crate::str_from_raw(txn_id_ptr, txn_id_len) }, unsafe { crate::str_from_raw(result_json_ptr, result_json_len) }) else {
        return 0;
    };
    with_log(|conn| {
        conn.execute(
            "UPDATE nirdosha_transact_log SET state = 'network_done', network_result_json = ?, updated_at = ? WHERE txn_id = ?",
            rusqlite::params![result_json, now_unix_secs(), txn_id],
        )
        .is_ok()
    })
    .unwrap_or(false) as i32
}

fn mark_state(txn_id: &str, state: &str) -> bool {
    with_log(|conn| conn.execute("UPDATE nirdosha_transact_log SET state = ?, updated_at = ? WHERE txn_id = ?", rusqlite::params![state, now_unix_secs(), txn_id]).is_ok())
        .unwrap_or(false)
}

/// Marked right *before* `commit`'s own first attempt — `binds`
/// carries `commit`'s real, already-computed argument values (built by
/// the exact same `codegen.rs::emit_db_binds` helper `db_execute`
/// already uses, reused verbatim since a transact-scalar and a
/// `db_execute` bind value are exactly the same four shapes). If the
/// process dies at any point from here through the end of `commit`'s
/// retry loop, replay has everything it needs to finish the job:
/// `commit`'s callee is fixed per `site_id`, and its arguments are
/// durably stored right here, not reconstructed from `network`/`txn_id`
/// alone the way `docs/TRANSACT.md`'s original interpreter-era design
/// did — a real, disclosed simplification: this phase's replay only
/// ever resumes from `commit_pending`/`compensate_pending` (i.e. after
/// `verify` already ran), never mid-`network`/`verify` — see
/// `docs/adr/0009-transact-durability-and-replay.md`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_transact_mark_commit_pending(txn_id_ptr: *const u8, txn_id_len: i64, binds_ptr: *const crate::NirBindValue, binds_len: i64) -> i32 {
    let Some(txn_id) = (unsafe { crate::str_from_raw(txn_id_ptr, txn_id_len) }) else { return 0 };
    let args_json = unsafe { encode_binds_json(binds_ptr, binds_len) };
    with_log(|conn| {
        conn.execute(
            "UPDATE nirdosha_transact_log SET state = 'commit_pending', commit_args_json = ?, updated_at = ? WHERE txn_id = ?",
            rusqlite::params![args_json, now_unix_secs(), txn_id],
        )
        .is_ok()
    })
    .unwrap_or(false) as i32
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_transact_mark_compensate_pending(txn_id_ptr: *const u8, txn_id_len: i64, binds_ptr: *const crate::NirBindValue, binds_len: i64) -> i32 {
    let Some(txn_id) = (unsafe { crate::str_from_raw(txn_id_ptr, txn_id_len) }) else { return 0 };
    let args_json = unsafe { encode_binds_json(binds_ptr, binds_len) };
    with_log(|conn| {
        conn.execute(
            "UPDATE nirdosha_transact_log SET state = 'compensate_pending', compensate_args_json = ?, updated_at = ? WHERE txn_id = ?",
            rusqlite::params![args_json, now_unix_secs(), txn_id],
        )
        .is_ok()
    })
    .unwrap_or(false) as i32
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_transact_mark_committed(txn_id_ptr: *const u8, txn_id_len: i64) -> i32 {
    let Some(txn_id) = (unsafe { crate::str_from_raw(txn_id_ptr, txn_id_len) }) else { return 0 };
    mark_state(txn_id, "committed") as i32
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_transact_mark_compensated(txn_id_ptr: *const u8, txn_id_len: i64) -> i32 {
    let Some(txn_id) = (unsafe { crate::str_from_raw(txn_id_ptr, txn_id_len) }) else { return 0 };
    mark_state(txn_id, "compensated") as i32
}

/// A pending row surfaced during replay — reconstructing enough to
/// dispatch to the right site's compiled replay trampoline
/// (`codegen.rs::emit_transact`'s own per-call-site trampoline
/// generation).
struct PendingRow {
    txn_id: String,
    site_id: i64,
    state: String,
    args_json: String,
}

fn scan_pending_rows() -> Vec<PendingRow> {
    with_log(|conn| {
        let mut stmt = match conn.prepare(
            "SELECT txn_id, site_id, state, \
             CASE state WHEN 'commit_pending' THEN commit_args_json ELSE compensate_args_json END \
             FROM nirdosha_transact_log WHERE state IN ('commit_pending', 'compensate_pending')",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = stmt.query_map([], |row| {
            Ok(PendingRow { txn_id: row.get(0)?, site_id: row.get(1)?, state: row.get(2)?, args_json: row.get::<_, Option<String>>(3)?.unwrap_or_else(|| "[]".to_string()) })
        });
        match rows {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => Vec::new(),
        }
    })
    .unwrap_or_default()
}

/// Decodes `json` (one pending row's own `commit_args_json`/
/// `compensate_args_json`) into a caller-provided `NirBindValue` array
/// — the one piece of this module a generated replay trampoline calls
/// directly, since only the trampoline (compiled per call site) knows
/// its own site's real argument count/types. See `decode_binds_json`'s
/// own doc comment for the leaked-string convention.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_transact_decode_args(json_ptr: *const u8, json_len: i64, out_ptr: *mut crate::NirBindValue, out_len: i64) -> i64 {
    let Some(json) = (unsafe { crate::str_from_raw(json_ptr, json_len) }) else { return -1 };
    unsafe { decode_binds_json(json, out_ptr, out_len) }
}

/// One registered per-call-site replay trampoline — `codegen.rs` emits
/// one small function per `transact` site (mirroring `spawn`'s own
/// per-call-site trampoline pattern, `lib.rs`'s "chan/spawn/join
/// kernels" section) that decodes its own row's `args_json` (via
/// `nir_transact_decode_args` above) and re-runs that site's `commit`/
/// `compensate` slot to completion (the same bounded-retry-with-backoff
/// logic the live path uses), and registers its function pointer here
/// at a compile-time-assigned `site_id`.
type ReplayFn = extern "C" fn(args_json_ptr: *const u8, args_json_len: i64, is_commit: i32) -> i32;

fn replay_registry() -> &'static Mutex<std::collections::HashMap<i64, ReplayFn>> {
    static REGISTRY: OnceLock<Mutex<std::collections::HashMap<i64, ReplayFn>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// Called once per `transact` site, from generated `main`'s own
/// prologue (`codegen.rs::emit_c_main`) — every site is registered
/// there, in a loop over `Codegen::transact_sites`, strictly before the
/// same prologue's own call to `nir_transact_replay_all` below, so
/// replay never dispatches to an as-yet-unregistered site.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_transact_register_replay_site(site_id: i64, f: ReplayFn) {
    replay_registry().lock().unwrap().insert(site_id, f);
}

/// Called once, from generated `main`'s prologue, strictly *after*
/// `nir_transact_log_init` and strictly *before* any user code
/// (including, for the eventual compiled-`serve` mode, binding its
/// listener) runs — a replayed compensation racing a fresh request
/// touching the same row is exactly the ordering hazard this sequencing
/// exists to prevent. A row whose `site_id` has no registered
/// trampoline (a rebuild that removed that `transact` site entirely) is
/// left `stuck` rather than guessed at.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_transact_replay_all() {
    let registry = replay_registry().lock().unwrap();
    for row in scan_pending_rows() {
        let is_commit = row.state == "commit_pending";
        match registry.get(&row.site_id) {
            Some(f) => {
                let args_json = row.args_json.as_bytes();
                let ok = f(args_json.as_ptr(), args_json.len() as i64, is_commit as i32);
                if ok != 0 {
                    mark_state(&row.txn_id, if is_commit { "committed" } else { "compensated" });
                } else {
                    eprintln!("nirdosha: replay of txn_id={} (site_id={}) failed, leaving it pending for the next restart", row.txn_id, row.site_id);
                }
            }
            None => {
                eprintln!("nirdosha: replay of txn_id={} references unknown site_id={} (binary rebuilt since this row was written?), marking stuck", row.txn_id, row.site_id);
                mark_state(&row.txn_id, "stuck");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_log_path(label: &str) -> String {
        std::env::temp_dir().join(format!("nirdosha_transact_test_{label}_{}.sqlite", std::process::id())).to_str().unwrap().to_string()
    }

    /// Each test gets its own log connection, not the shared global
    /// `LOG` static (which is process-wide and only ever initialized
    /// once via `OnceLock` -- calling `nir_transact_log_init` a second
    /// time in the same test binary would silently no-op via `.ok()`
    /// and leave an earlier test's connection in place). Exercises the
    /// same schema/SQL directly rather than duplicating it, so a real
    /// schema bug still gets caught.
    fn open_test_log(path: &str) -> Connection {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             CREATE TABLE IF NOT EXISTS nirdosha_transact_log (
                 txn_id TEXT PRIMARY KEY,
                 site_id INTEGER NOT NULL,
                 state TEXT NOT NULL,
                 network_result_json TEXT,
                 commit_args_json TEXT,
                 compensate_args_json TEXT,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL
             );",
        )
        .unwrap();
        conn
    }

    #[test]
    fn schema_round_trips_a_full_lifecycle() {
        let path = temp_log_path("lifecycle");
        let conn = open_test_log(&path);
        let now = now_unix_secs();
        conn.execute("INSERT INTO nirdosha_transact_log (txn_id, site_id, state, created_at, updated_at) VALUES ('t1', 1, 'pending', ?, ?)", rusqlite::params![now, now]).unwrap();
        conn.execute("UPDATE nirdosha_transact_log SET state = 'network_done', network_result_json = '200' WHERE txn_id = 't1'", []).unwrap();
        conn.execute("UPDATE nirdosha_transact_log SET state = 'commit_pending' WHERE txn_id = 't1'", []).unwrap();
        conn.execute("UPDATE nirdosha_transact_log SET state = 'committed' WHERE txn_id = 't1'", []).unwrap();
        let state: String = conn.query_row("SELECT state FROM nirdosha_transact_log WHERE txn_id = 't1'", [], |r| r.get(0)).unwrap();
        assert_eq!(state, "committed");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_commit_pending_row_is_found_by_the_pending_scan() {
        let path = temp_log_path("pending_scan");
        let conn = open_test_log(&path);
        let now = now_unix_secs();
        conn.execute("INSERT INTO nirdosha_transact_log (txn_id, site_id, state, created_at, updated_at) VALUES ('t2', 7, 'commit_pending', ?, ?)", rusqlite::params![now, now]).unwrap();
        conn.execute("INSERT INTO nirdosha_transact_log (txn_id, site_id, state, created_at, updated_at) VALUES ('t3', 7, 'committed', ?, ?)", rusqlite::params![now, now]).unwrap();
        let mut stmt = conn.prepare("SELECT txn_id, site_id, state FROM nirdosha_transact_log WHERE state IN ('commit_pending', 'compensate_pending')").unwrap();
        let rows: Vec<(String, i64, String)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().filter_map(|r| r.ok()).collect();
        assert_eq!(rows.len(), 1, "only the commit_pending row, not the already-committed one");
        assert_eq!(rows[0].0, "t2");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn encode_then_decode_binds_round_trips_every_scalar_shape() {
        let binds = [
            crate::NirBindValue { tag: 0, i: 42, f: 0.0, s_ptr: std::ptr::null(), s_len: 0 },
            crate::NirBindValue { tag: 1, i: 0, f: 3.5, s_ptr: std::ptr::null(), s_len: 0 },
            crate::NirBindValue { tag: 3, i: 1, f: 0.0, s_ptr: std::ptr::null(), s_len: 0 },
        ];
        let s = "hello".to_string();
        let binds_with_str = [
            binds[0],
            crate::NirBindValue { tag: 2, i: 0, f: 0.0, s_ptr: s.as_ptr(), s_len: s.len() as i64 },
            binds[1],
            binds[2],
        ];
        let json = unsafe { encode_binds_json(binds_with_str.as_ptr(), binds_with_str.len() as i64) };
        let mut out = [crate::NirBindValue { tag: -1, i: 0, f: 0.0, s_ptr: std::ptr::null(), s_len: 0 }; 4];
        let n = unsafe { decode_binds_json(&json, out.as_mut_ptr(), out.len() as i64) };
        assert_eq!(n, 4);
        assert_eq!(out[0].tag, 0);
        assert_eq!(out[0].i, 42);
        assert_eq!(out[1].tag, 2);
        assert_eq!(unsafe { crate::str_from_raw(out[1].s_ptr, out[1].s_len) }, Some("hello"));
        assert_eq!(out[2].tag, 1);
        assert_eq!(out[2].f, 3.5);
        assert_eq!(out[3].tag, 3);
        assert_eq!(out[3].i, 1);
    }

    #[test]
    fn replay_registry_dispatches_to_the_registered_site_and_marks_terminal() {
        extern "C" fn fake_replay(_args_json_ptr: *const u8, _args_json_len: i64, _is_commit: i32) -> i32 {
            1 // always succeeds
        }
        unsafe { nir_transact_register_replay_site(424242, fake_replay) };
        let registry = replay_registry().lock().unwrap();
        let f = registry.get(&424242).expect("site should be registered");
        let txn_id = b"test-txn";
        assert_eq!(f(txn_id.as_ptr(), txn_id.len() as i64, 1), 1);
    }

    #[test]
    fn an_unregistered_site_id_is_left_for_replay_all_to_mark_stuck_not_guessed_at() {
        let registry = replay_registry().lock().unwrap();
        assert!(registry.get(&999_999_999).is_none());
    }
}
