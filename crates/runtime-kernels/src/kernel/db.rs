//! Real connection pooling plus Postgres support for `db_connect`/
//! `db_query`/`db_execute`, layered onto the SQLite path that already
//! shipped connection-per-call (`lib.rs`'s original `nir_db_connect`).
//! Two independent [`PoolRegistry`] instances — one per backend manager
//! type, since `PoolRegistry<M>` is generic over the connection manager
//! (`pool.rs`'s own doc comment already anticipated exactly this: "a
//! Postgres-with-TLS pool and a plain-Postgres pool... get their own
//! independent `PoolRegistry` instances with zero duplicated logic, and
//! so does any future resource kind").
//!
//! **`:memory:` is deliberately never pooled.** A pooled `:memory:`
//! connection would silently hand two unrelated `db_connect(":memory:")`
//! callers either the *same* database or two different, independently
//! empty ones depending on pool state — a `:memory:` SQLite database is
//! private to the one connection that created it, full stop. The
//! now-deleted interpreter-era `dbconn.rs` already drew this exact line
//! (`Sqlite` unpooled vs. `SqlitePooled` for a real file path) — kept
//! here for the same reason, not reinvented.
//!
//! **Postgres pulls in `tokio` transitively** (the synchronous
//! `postgres` crate wraps `tokio-postgres` internally, running its own
//! private runtime under the hood) — a disclosed departure from this
//! crate's otherwise-real "no async runtime" posture elsewhere
//! (`pool.rs`/`mailbox.rs`'s own doc comments), not a silent one. Every
//! `nir_db_*` kernel below stays a plain, synchronous `extern "C"`
//! function; nothing here is `async`, and no caller of this module ever
//! needs to know `postgres` happens to be implemented on top of tokio.
//!
//! **TLS is verify-by-default for any non-`localhost`/non-`127.0.0.1`
//! Postgres connection string** (this phase's own ADR,
//! `docs/adr/0005-postgres-pooling-and-tls.md`) — a caller can still
//! opt out explicitly with `sslmode=disable` in their own connection
//! string; nothing here silently downgrades a caller's own explicit
//! choice, only fills in a safe default when they didn't make one.

use r2d2::ManageConnection;
use std::str::FromStr;
use std::sync::{Once, OnceLock};

use super::pool::{self, PoolConfig, PoolRegistry};

// ---- SQLite: a minimal custom manager, not `r2d2_sqlite` -----------
//
// `r2d2_sqlite`'s only version compatible with this workspace's
// already-pinned `rusqlite = "0.31"` (0.24.0) predates that crate's
// `is-valid` feature — its default `is_valid` is a no-op. Rather than
// pull in a dependency whose real liveness check isn't available at
// this pin, or bump `rusqlite` project-wide as an unrelated side effect
// of this phase (it's pinned identically in `crates/compiler/Cargo.toml`
// too), this is a direct, five-method `ManageConnection` impl — the
// full interface has no real complexity `r2d2_sqlite` would have saved.
pub struct SqliteManager {
    path: String,
}

impl ManageConnection for SqliteManager {
    type Connection = rusqlite::Connection;
    type Error = rusqlite::Error;

    fn connect(&self) -> Result<Self::Connection, Self::Error> {
        rusqlite::Connection::open(&self.path)
    }

    /// A real round-trip, not `is_closed()`-equivalent — SQLite
    /// connections rarely go stale the way a networked backend's do
    /// (no peer to restart out from under a local file handle), but the
    /// mandatory-round-trip rule this phase's own review settled on
    /// applies uniformly across backends, not backend-specifically.
    ///
    /// `r2d2::Pool::try_get_inner` (confirmed by reading `r2d2`'s own
    /// source, not assumed) calls exactly this method, and only this
    /// one, on every checkout of an idle pooled connection
    /// (`test_on_check_out`, `true` by default, never overridden by
    /// this module) — an `Err` here makes r2d2 itself drop the bad
    /// connection and transparently retry with a fresh one before
    /// `Pool::get` ever returns. That's the whole "APM rehydrates a
    /// stale connection before the caller sees it" guarantee, already
    /// built into r2d2; this method existing to fail honestly on a
    /// dead connection is the only piece this phase had to add. The
    /// `record_stale_rehydrated` call on the error path is exact, not
    /// approximate, because this is r2d2's one and only call site for
    /// this method.
    fn is_valid(&self, conn: &mut Self::Connection) -> Result<(), Self::Error> {
        let result = conn.query_row("SELECT 1", [], |_| Ok(())).map(|_| ());
        if result.is_err() {
            super::record_stale_rehydrated(super::domain::db());
        }
        result
    }

    fn has_broken(&self, _conn: &mut Self::Connection) -> bool {
        false
    }
}

fn sqlite_pool_registry() -> &'static PoolRegistry<SqliteManager> {
    static REGISTRY: OnceLock<PoolRegistry<SqliteManager>> = OnceLock::new();
    static REGISTERED: Once = Once::new();
    let registry = REGISTRY.get_or_init(PoolRegistry::new);
    // RFC 0011 §5: every pool-backed registry registers itself with the
    // reaper once, the first time it's actually reached — a lazily-built
    // process, same posture `domain::db()`'s own self-registration
    // already has (Phase 2a).
    REGISTERED.call_once(|| pool::register_for_reaping(registry));
    registry
}

// ---- Postgres --------------------------------------------------------

pub struct PostgresManager {
    config: postgres::Config,
    tls: postgres_native_tls::MakeTlsConnector,
}

impl ManageConnection for PostgresManager {
    type Connection = postgres::Client;
    type Error = postgres::Error;

    fn connect(&self) -> Result<Self::Connection, Self::Error> {
        self.config.connect(self.tls.clone())
    }

    /// A real `SELECT 1` round-trip — deliberately not `Client::is_closed()`
    /// alone. After a server-side restart (exactly the kind of staleness
    /// this pool exists to recover from), a client-side socket typically
    /// has no idea the peer is gone until it tries to use the
    /// connection; `is_closed()` would false-negative straight through
    /// that window. Same single-call-site reasoning as
    /// `SqliteManager::is_valid` above — r2d2 calls this exactly once,
    /// on checkout, and transparently retries with a fresh connection
    /// on `Err`, which is the real mechanism behind
    /// `record_stale_rehydrated` below being exact rather than a guess.
    fn is_valid(&self, conn: &mut Self::Connection) -> Result<(), Self::Error> {
        let result = conn.simple_query("SELECT 1").map(|_| ());
        if result.is_err() {
            super::record_stale_rehydrated(super::domain::db());
        }
        result
    }

    fn has_broken(&self, conn: &mut Self::Connection) -> bool {
        conn.is_closed()
    }
}

fn postgres_pool_registry() -> &'static PoolRegistry<PostgresManager> {
    static REGISTRY: OnceLock<PoolRegistry<PostgresManager>> = OnceLock::new();
    static REGISTERED: Once = Once::new();
    let registry = REGISTRY.get_or_init(PoolRegistry::new);
    REGISTERED.call_once(|| pool::register_for_reaping(registry));
    registry
}

fn is_local_host(host: &postgres::config::Host) -> bool {
    match host {
        postgres::config::Host::Tcp(h) => h == "localhost" || h == "127.0.0.1" || h == "::1",
        // A Unix socket directory is inherently local.
        #[cfg(unix)]
        postgres::config::Host::Unix(_) => true,
    }
}

/// Builds the real `postgres::Config`/TLS connector pair for one
/// connection string, filling in a safe TLS default (`verify-full`)
/// only when the caller didn't already choose an `sslmode` themselves,
/// and only for a non-local host — never overriding an explicit choice.
fn build_postgres_manager(conn_str: &str) -> Result<PostgresManager, String> {
    let mut config = postgres::Config::from_str(conn_str).map_err(|e| e.to_string())?;
    let caller_set_sslmode = conn_str.contains("sslmode=");
    if !caller_set_sslmode {
        let all_local = config.get_hosts().iter().all(is_local_host);
        if !all_local {
            // `SslMode` itself only distinguishes disable/prefer/require
            // — real certificate + hostname verification is entirely
            // the `native_tls::TlsConnector`'s job below, not this
            // enum's. `Require` is what actually enforces TLS gets used
            // at all (`Prefer` would silently allow plaintext fallback,
            // defeating the ADR's whole point); the builder immediately
            // below is what makes that TLS session verified, not just
            // encrypted.
            config.ssl_mode(postgres::config::SslMode::Require);
        }
    }
    let builder = native_tls::TlsConnector::builder();
    // Deliberately no `danger_accept_invalid_certs`/`danger_accept_invalid_hostnames`
    // call anywhere in this module — verification stays on by default,
    // the whole point of the ADR decision above.
    let connector = builder.build().map_err(|e| e.to_string())?;
    let tls = postgres_native_tls::MakeTlsConnector::new(connector);
    Ok(PostgresManager { config, tls })
}

// ---- the `DbConn` handle behind `db_table()` --------------------------

/// One handle behind `lib.rs`'s `db_table()` — either a pooled checkout
/// (returned to its pool automatically on `Drop`, r2d2's own
/// "checkin-on-drop" guarantee, `pool.rs`'s own doc comment) or a
/// standalone, unpooled connection (`:memory:` only, see this module's
/// own doc comment for why).
pub enum DbConn {
    SqliteUnpooled(rusqlite::Connection),
    SqlitePooled(r2d2::PooledConnection<SqliteManager>),
    Postgres(r2d2::PooledConnection<PostgresManager>),
}

impl DbConn {
    /// `Some` for either SQLite variant, regardless of pooled-or-not —
    /// the one place the two SQLite variants collapse back into a
    /// single `&mut rusqlite::Connection`, which is all
    /// `nir_db_execute`/`nir_db_query`'s existing SQLite-side logic
    /// (`conn.execute(...)`/`conn.prepare(...)`) ever needed.
    pub fn as_sqlite_mut(&mut self) -> Option<&mut rusqlite::Connection> {
        match self {
            DbConn::SqliteUnpooled(c) => Some(c),
            DbConn::SqlitePooled(c) => Some(&mut *c),
            DbConn::Postgres(_) => None,
        }
    }

    pub fn as_postgres_mut(&mut self) -> Option<&mut postgres::Client> {
        match self {
            DbConn::Postgres(c) => Some(&mut *c),
            _ => None,
        }
    }
}

/// `env_var`/`default_max` pair this phase's pool sizing follows —
/// `pool.max_size` must be `≥` the `db` domain's own admission ceiling
/// (this phase's own review: admission never blocks, but `Pool::get()`
/// does, so the ceiling — not a smaller pool — has to stay the one real
/// choke point). The `db` domain's default ceiling is 10,000
/// (`kernel/mod.rs::BUILTIN_DOMAINS`) — deliberately generous, not
/// production-tuned, so this phase's own pool default follows the same
/// posture rather than silently becoming the tighter constraint.
fn db_pool_config() -> PoolConfig {
    let mut cfg = PoolConfig::from_env("DB");
    if cfg.max_size < 1000 {
        cfg.max_size = 1000;
    }
    // Red-team report A12 (`scratch/red-team-report-main-d7fae42.md`):
    // the unconditional floor-to-1000 above ignored an operator's own
    // smaller `NIRDOSHA_KERNEL_MAX_DB` -- `plugin_pool_config`
    // (`plugin_provider.rs`) already clamps *down* to a provider's
    // ceiling; this floor-to-1000 clamped back *up* past it, so a
    // deliberately small `db` ceiling silently stopped being "the
    // choke point" (the RFC's own claim) the moment the pool itself
    // became the tighter constraint instead. Never let `max_size`
    // exceed the domain's own resolved ceiling, regardless of the
    // 1000-floor above.
    pool::clamp_max_size_to_ceiling(&mut cfg, super::ceiling_for(super::domain::db()));
    cfg
}

/// Connects `conn_str`, real pooling for every case except `:memory:`.
/// `NIRDOSHA_DB_POOL_CONNECT_TIMEOUT_SECS` (via `PoolConfig::from_env`)
/// is this phase's `r2d2::Builder::connection_timeout` — sized against
/// the validation round-trip, not just the connect (this phase's own
/// review): the default budget now covers connect *plus* the mandatory
/// `SELECT 1` above, not connect alone.
///
/// **This function itself calls `kernel::acquire(domain::db())` at its
/// own start** (red-team report A8, `scratch/red-team-report-main-d7fae42.md`
/// — a future third caller of this function could otherwise forget to
/// bracket admission itself; making `connect` self-contained closes
/// that hazard structurally instead of relying on every caller to
/// remember). A successful `Ok(_)` return here holds exactly one
/// `domain::db()` admission slot that the caller now owns and must
/// release exactly once, when this connection's session ends — `lib.rs`'s
/// `nir_db_stop` (`kernel::release(domain::db())` on handle close) is
/// that release point today; the connect-to-stop *session* lifecycle
/// still lives at the caller's own `HandleTable`, since the affine `db`
/// handle (and thus the session's true end) is a concept `lib.rs` owns,
/// not something this module tracks. Every `Err(_)` return path below
/// releases its own admission before returning — a failed connect
/// attempt is fully self-contained, nothing for the caller to release.
/// RFC 0011 §2 step 1: "check built-in schemes first... if none match,"
/// fall through to the plugin provider table. `lib.rs`'s `nir_db_connect`
/// needs this *before* it can decide which domain to
/// `kernel::acquire` — a bare path/`:memory:`/`postgres[ql]://` always
/// means the built-in `db` domain and this module's own `connect`
/// below; anything else means a plugin's own registered domain and
/// `plugin_provider::connect_conn_shape` instead. Kept in sync with
/// `connect`'s own scheme matching by construction: both read this one
/// function's result rather than duplicating the scheme list.
pub fn is_builtin_scheme(conn_str: &str) -> bool {
    conn_str.starts_with("postgres://") || conn_str.starts_with("postgresql://") || !conn_str.contains("://")
}

pub fn connect(conn_str: &str) -> Result<DbConn, String> {
    if !super::acquire(super::domain::db()) {
        return Err("too many open db connections".to_string());
    }
    match connect_inner(conn_str) {
        Ok(conn) => Ok(conn),
        Err(e) => {
            super::release(super::domain::db());
            Err(e)
        }
    }
}

fn connect_inner(conn_str: &str) -> Result<DbConn, String> {
    if conn_str.starts_with("postgres://") || conn_str.starts_with("postgresql://") {
        let pool = postgres_pool_registry().get_or_create(conn_str, db_pool_config(), || build_postgres_manager(conn_str))?;
        let conn = pool.get().map_err(|e| e.to_string())?;
        return Ok(DbConn::Postgres(conn));
    }
    if conn_str == ":memory:" {
        return rusqlite::Connection::open(conn_str).map(DbConn::SqliteUnpooled).map_err(|e| e.to_string());
    }
    let pool = sqlite_pool_registry().get_or_create(conn_str, db_pool_config(), || Ok(SqliteManager { path: conn_str.to_string() }))?;
    let conn = pool.get().map_err(|e| e.to_string())?;
    Ok(DbConn::SqlitePooled(conn))
}

// ---- `?` -> `$1, $2, ...` placeholder rewriting for Postgres ----------

/// String-literal-aware — a `?` inside `'...'` (or a `--`-style line
/// comment) is never rewritten, matching the recovered interpreter-era
/// `dbconn.rs::rewrite_placeholders`'s exact contract. SQLite already
/// accepts bare `?` natively, so this only ever runs on the Postgres
/// path.
pub fn rewrite_placeholders(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len() + 8);
    let mut n = 0u32;
    let mut chars = sql.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_string = !in_string;
                out.push(c);
            }
            '-' if !in_string && chars.peek() == Some(&'-') => {
                // Line comment: copy verbatim through end-of-line.
                out.push('-');
                out.push(chars.next().unwrap());
                for c2 in chars.by_ref() {
                    out.push(c2);
                    if c2 == '\n' {
                        break;
                    }
                }
            }
            '?' if !in_string => {
                n += 1;
                out.push('$');
                out.push_str(&n.to_string());
            }
            _ => out.push(c),
        }
    }
    out
}

// ---- Postgres bind values / row -> JSON -------------------------------

/// One bind value, owned, `ToSql`-implementing — built from the same
/// `NirBindValue` tags `codegen.rs`'s `emit_db_binds` already produces
/// for the SQLite path, so `nir_db_execute`/`nir_db_query`'s Postgres
/// branch needs no codegen-side changes, only a different Rust-side
/// conversion of the identical wire format.
#[derive(Debug)]
pub enum PgBindValue {
    I64(i64),
    F64(f64),
    Str(String),
    Bool(bool),
}

/// Converts the exact same `NirBindValue` wire format
/// `crate::bind_values_from_raw` (the SQLite path) already consumes —
/// same tags, same `#[repr(C)]` struct, no `codegen.rs` change needed
/// for the Postgres path to exist alongside it.
pub unsafe fn pg_bind_values_from_raw(ptr: *const crate::NirBindValue, len: i64) -> Vec<PgBindValue> {
    if len == 0 {
        return Vec::new();
    }
    let raw = unsafe { std::slice::from_raw_parts(ptr, len as usize) };
    raw.iter()
        .map(|b| match b.tag {
            0 => PgBindValue::I64(b.i),
            1 => PgBindValue::F64(b.f),
            2 => {
                let s = unsafe { crate::str_from_raw(b.s_ptr, b.s_len) }.unwrap_or("").to_string();
                PgBindValue::Str(s)
            }
            3 => PgBindValue::Bool(b.i != 0),
            _ => PgBindValue::Str(String::new()),
        })
        .collect()
}

impl postgres::types::ToSql for PgBindValue {
    /// Encodes according to the server's own reported parameter type,
    /// not a fixed Rust-side assumption — found necessary by testing
    /// against a real server, not designed in advance. Nirdosha's own
    /// type system has exactly one integer width (`i64`) and one float
    /// width (`f64`), but a real column can be `int2`/`int4`/`int8` or
    /// `float4`/`float8`; blindly encoding every `i64` bind value as an
    /// 8-byte `int8` wire value against an `INTEGER` (`int4`) column is
    /// a real wire-format mismatch, not a hypothetical one -- this is
    /// exactly the failure this match was added to fix.
    fn to_sql(&self, ty: &postgres::types::Type, out: &mut postgres::types::private::BytesMut) -> Result<postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        match self {
            PgBindValue::I64(v) => match *ty {
                postgres::types::Type::INT2 => (*v as i16).to_sql(ty, out),
                postgres::types::Type::INT4 => (*v as i32).to_sql(ty, out),
                _ => v.to_sql(ty, out),
            },
            PgBindValue::F64(v) => match *ty {
                postgres::types::Type::FLOAT4 => (*v as f32).to_sql(ty, out),
                _ => v.to_sql(ty, out),
            },
            PgBindValue::Str(v) => v.to_sql(ty, out),
            PgBindValue::Bool(v) => v.to_sql(ty, out),
        }
    }

    fn accepts(_ty: &postgres::types::Type) -> bool {
        true
    }

    postgres::types::to_sql_checked!();
}

pub fn pg_row_to_json(row: &postgres::Row) -> serde_json::Value {
    let mut obj = serde_json::Map::with_capacity(row.len());
    for (i, col) in row.columns().iter().enumerate() {
        let name = col.name().to_string();
        let value = match col.type_().name() {
            "bool" => row.try_get::<_, Option<bool>>(i).ok().flatten().map(serde_json::Value::from).unwrap_or(serde_json::Value::Null),
            "int2" => row.try_get::<_, Option<i16>>(i).ok().flatten().map(|v| serde_json::Value::from(v as i64)).unwrap_or(serde_json::Value::Null),
            "int4" => row.try_get::<_, Option<i32>>(i).ok().flatten().map(|v| serde_json::Value::from(v as i64)).unwrap_or(serde_json::Value::Null),
            "int8" => row.try_get::<_, Option<i64>>(i).ok().flatten().map(serde_json::Value::from).unwrap_or(serde_json::Value::Null),
            "float4" => row
                .try_get::<_, Option<f32>>(i)
                .ok()
                .flatten()
                .and_then(|v| serde_json::Number::from_f64(v as f64))
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            "float8" => row
                .try_get::<_, Option<f64>>(i)
                .ok()
                .flatten()
                .and_then(serde_json::Number::from_f64)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            // text/varchar/bpchar/name and anything else unrecognized —
            // read as text; a genuinely unsupported column type
            // (matching the SQLite path's own disclosed BLOB gap) comes
            // back `null` rather than failing the whole row.
            _ => row.try_get::<_, Option<String>>(i).ok().flatten().map(serde_json::Value::String).unwrap_or(serde_json::Value::Null),
        };
        obj.insert(name, value);
    }
    serde_json::Value::Object(obj)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrite_placeholders_counts_positionally() {
        assert_eq!(rewrite_placeholders("SELECT * FROM t WHERE a = ? AND b = ?"), "SELECT * FROM t WHERE a = $1 AND b = $2");
    }

    #[test]
    fn rewrite_placeholders_never_touches_a_question_mark_inside_a_string_literal() {
        assert_eq!(rewrite_placeholders("SELECT ? FROM t WHERE name = 'what?'"), "SELECT $1 FROM t WHERE name = 'what?'");
    }

    #[test]
    fn rewrite_placeholders_never_touches_a_question_mark_inside_a_line_comment() {
        let sql = "SELECT ? -- what about this ?\nFROM t";
        let rewritten = rewrite_placeholders(sql);
        assert_eq!(rewritten, "SELECT $1 -- what about this ?\nFROM t");
    }

    #[test]
    fn memory_never_gets_a_pool_entry() {
        let _ = connect(":memory:");
        let _ = connect(":memory:");
        // `:memory:` never reaches `sqlite_pool_registry` at all (an
        // early return in `connect`) -- if it did, "same key" would
        // wrongly collapse two independent in-memory databases into one
        // shared pool, exactly the correctness bug this module's own
        // doc comment opens with. `contains_key`, not `pool_count`:
        // this registry is one process-wide static shared by other
        // concurrently-running tests, so only a check of this exact key
        // is race-free.
        assert!(!sqlite_pool_registry().contains_key(":memory:"), ":memory: must never create a pool entry");
    }

    #[test]
    fn same_file_path_reuses_one_pool_different_paths_get_different_pools() {
        let dir = std::env::temp_dir();
        // Unique per test *invocation*, not just per-process, so this
        // is race-free against any other concurrently-running test that
        // also happens to touch `sqlite_pool_registry()` -- a random
        // suffix, not just `process::id()` (shared by every test in
        // this binary).
        let unique: u64 = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64;
        let path_a = dir.join(format!("nirdosha_db_pool_registry_test_a_{unique}.sqlite")).to_str().unwrap().to_string();
        let path_b = dir.join(format!("nirdosha_db_pool_registry_test_b_{unique}.sqlite")).to_str().unwrap().to_string();
        let _ = std::fs::remove_file(&path_a);
        let _ = std::fs::remove_file(&path_b);

        assert!(!sqlite_pool_registry().contains_key(&path_a));
        let _ = connect(&path_a).unwrap();
        let _ = connect(&path_a).unwrap();
        assert!(sqlite_pool_registry().contains_key(&path_a), "the same connection string must share one pool");

        assert!(!sqlite_pool_registry().contains_key(&path_b));
        let _ = connect(&path_b).unwrap();
        assert!(sqlite_pool_registry().contains_key(&path_b), "a different connection string must get its own pool");

        let _ = std::fs::remove_file(&path_a);
        let _ = std::fs::remove_file(&path_b);
    }

    /// Real, opt-in, `#[ignore]`d Postgres coverage for the pooling
    /// mechanism itself (`lib.rs`'s `db_kernel_tests` covers the
    /// public `db_connect`/`db_query`/`db_execute` ABI surface; this
    /// covers `PoolRegistry` internals only visible from inside this
    /// module).
    fn test_postgres_url() -> String {
        std::env::var("NIRDOSHA_TEST_POSTGRES_URL").unwrap_or_else(|_| "postgres://postgres@127.0.0.1:5432/postgres".to_string())
    }

    #[test]
    #[ignore]
    fn postgres_same_connection_string_reuses_one_pool() {
        let url = test_postgres_url();
        let conn1 = connect(&url).expect("real postgres server must be reachable");
        let conn2 = connect(&url).expect("real postgres server must be reachable");
        assert!(postgres_pool_registry().contains_key(&url), "the same connection string must share one pool, not open two");
        drop(conn1);
        drop(conn2);
    }

    /// Forces the exact staleness scenario this phase's own review
    /// insisted on testing for real: kill the live connection out from
    /// under the pool (via `pg_terminate_backend`, from a second,
    /// separate connection), then prove the next checkout transparently
    /// rehydrates — `record_stale_rehydrated` fires and a query through
    /// the "same" handle still succeeds, rather than the caller ever
    /// seeing a stale-connection error.
    #[test]
    #[ignore]
    fn postgres_checkout_rehydrates_a_connection_killed_out_from_under_the_pool() {
        // A connection string distinct from every other Postgres test's
        // (`?application_name=...` doesn't change which server/database
        // this reaches, only the pool-registry *key*) -- this test's
        // precise "kill exactly one connection, then immediately check
        // it back out" assumption isn't safe to share a pool with
        // concurrently-running tests that also check connections in and
        // out of the *same* underlying pool (confirmed: this test flakes
        // when run alongside the other Postgres tests, which all default
        // to the identical connection string otherwise, and passes
        // reliably alone).
        let url = format!("{}?application_name=nirdosha_rehydrate_test", test_postgres_url());
        let pool = postgres_pool_registry().get_or_create(&url, super::db_pool_config(), || build_postgres_manager(&url)).unwrap();

        let backend_pid: i32 = {
            let mut conn = pool.get().unwrap();
            let row = conn.query_one("SELECT pg_backend_pid()", &[]).unwrap();
            row.get(0)
        };
        // Connection above drops here, returning to the *pool under
        // test* -- idle, valid, and about to be killed. The killer
        // itself must be a raw, separate connection, deliberately never
        // going through `pool` -- r2d2 checks out idle connections
        // LIFO (`Vec::pop`, confirmed by reading r2d2's own source), so
        // if the killer's own connection returned to this same pool
        // afterward, the next `pool.get()` below would very likely hand
        // back the killer's still-alive connection instead of the one
        // actually under test, and this test would pass for the wrong
        // reason (or flake).
        {
            let manager = build_postgres_manager(&url).unwrap();
            let mut killer = manager.connect().unwrap();
            killer.execute("SELECT pg_terminate_backend($1)", &[&backend_pid]).ok();
        }
        // Give the terminated backend a moment to actually tear its
        // socket down -- `pg_terminate_backend` signals the backend,
        // it doesn't synchronously guarantee the client-side socket has
        // already observed the close by the time this call returns.
        std::thread::sleep(std::time::Duration::from_millis(200));

        let before = super::super::stats(super::super::domain::db()).3;
        // The pooled connection killer's own checkout is now itself
        // stale for the *next* caller too, in general -- but the one
        // under test here is the original, now-terminated backend:
        // r2d2 must detect it via `is_valid` on this checkout and
        // transparently hand back a fresh one instead.
        let mut conn = pool.get().expect("checkout must succeed by rehydrating, not by returning the dead connection");
        let row = conn.query_one("SELECT 1", &[]).expect("the rehydrated connection must actually work");
        let one: i32 = row.get(0);
        assert_eq!(one, 1);
        let after = super::super::stats(super::super::domain::db()).3;
        assert!(after > before, "record_stale_rehydrated must have fired for the killed connection");
    }
}
