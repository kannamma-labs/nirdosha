//! Generic, backend-agnostic resource pooling — ported near-verbatim
//! from the interpreter-side `crates/compiler/src/pool.rs` (removed
//! along with the interpreter itself), because its actual design has
//! zero interpreter dependency: `PoolConfig`/`PoolRegistry<M>` are
//! generic over [`r2d2::ManageConnection`], never over anything
//! `nirdosha`-specific. This is [`super`]'s "resource *pooling*, not
//! just admission" half — [`super::acquire`]/[`super::release`] answer
//! "are we under the ceiling," this module answers "reuse an
//! already-open connection instead of paying to open a new one." The
//! two are complementary, not redundant: a future `db`/`mq` domain
//! would plausibly want both — a `PoolRegistry` for reuse, plus
//! [`super::acquire`]/[`super::release`] admission at the same call
//! site if the domain's ceiling needs to be enforced independently of
//! whatever `max_size` a specific pool key happens to have.
//!
//! **Not wired to anything yet, on purpose** — same treatment
//! [`super::HandleTable`] gets. `db` and `mq` don't compile at all
//! today (`codegen.rs` hard-rejects them); this exists now so whichever
//! lands first (Track B items B2/B3) gets real, proven connection
//! pooling from its very first line of codegen, instead of "every call
//! opens a fresh connection" the way the interpreter's own `dbconn.rs`
//! originally worked before it grew this exact module.
//!
//! ## Why r2d2, not hand-rolled
//!
//! A connection pool's correctness hinges entirely on a handful of
//! concurrency primitives (bounded checkout, checkin-on-drop, idle/
//! lifetime recycling, health-checking a connection before handing it
//! out) that are easy to get subtly wrong — a deadlock under load, a
//! leaked permit, a connection handed out mid-teardown. `r2d2` is a
//! small (no async runtime, no tokio dependency), extremely
//! well-established crate that already gets all of this right, and
//! already has first-party `ManageConnection` impls for the backends a
//! future `db` codegen effort would need (`r2d2_sqlite`/
//! `r2d2_postgres`) — neither added as a dependency yet, since nothing
//! calls this module yet either.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

pub use r2d2::{ManageConnection, Pool};

/// Tunables for one pool. Every field has a sensible default
/// (`PoolConfig::default`); [`PoolConfig::from_env`] lets an operator
/// override them per resource kind without a rebuild — e.g.
/// `NIRDOSHA_DB_POOL_MAX_SIZE=50` for a future database pool
/// specifically, so an independent `mq` pool under a different prefix
/// (`NIRDOSHA_MQ_POOL_*`) can be tuned separately.
#[derive(Debug, Clone, Copy)]
pub struct PoolConfig {
    /// Hard cap on concurrently open physical connections for one pool
    /// key.
    pub max_size: u32,
    /// Connections r2d2 tries to keep idle/ready rather than opening
    /// lazily on first checkout. `Some(0)` (the default) means every
    /// pool starts empty and grows on demand, up to `max_size`.
    /// Deliberately never `None` here: r2d2 treats a `None` `min_idle`
    /// as "eagerly establish `max_size` idle connections at build
    /// time" — the opposite of "no minimum" (a real bug the original,
    /// interpreter-side version of this module caught by actually
    /// running it: every SQLite pool was silently opening 10 file
    /// handles on the very first connect, for a program that might only
    /// ever need one).
    pub min_idle: Option<u32>,
    /// How long a checkout call blocks waiting for a free connection
    /// before giving up. Bounded deliberately — an unbounded wait under
    /// real overload would just convert "too many connections" into
    /// "every request hangs forever."
    pub connect_timeout: Duration,
    /// A connection idle longer than this is closed and not reused.
    pub idle_timeout: Option<Duration>,
    /// A connection open longer than this, regardless of activity, is
    /// recycled.
    pub max_lifetime: Option<Duration>,
}

impl Default for PoolConfig {
    fn default() -> Self {
        PoolConfig {
            max_size: 10,
            min_idle: Some(0),
            connect_timeout: Duration::from_secs(5),
            idle_timeout: Some(Duration::from_secs(10 * 60)),
            max_lifetime: Some(Duration::from_secs(30 * 60)),
        }
    }
}

impl PoolConfig {
    /// Reads `NIRDOSHA_{PREFIX}_POOL_MAX_SIZE`/`_MIN_IDLE`/
    /// `_CONNECT_TIMEOUT_SECS`/`_IDLE_TIMEOUT_SECS`/`_MAX_LIFETIME_SECS`
    /// (a `0` for either timeout means "disabled", matching
    /// `Option<Duration>` -> `None`), falling back to
    /// [`PoolConfig::default`] field-by-field for anything unset or
    /// unparseable — a malformed env var degrades to the default for
    /// that one field, it never fails the whole program.
    pub fn from_env(prefix: &str) -> Self {
        let d = PoolConfig::default();
        let env_u32 = |suffix: &str, default: u32| -> u32 {
            std::env::var(format!("NIRDOSHA_{prefix}_POOL_{suffix}")).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
        };
        let env_secs = |suffix: &str, default: Option<Duration>| -> Option<Duration> {
            match std::env::var(format!("NIRDOSHA_{prefix}_POOL_{suffix}")) {
                Ok(v) => match v.parse::<u64>() {
                    Ok(0) => None,
                    Ok(n) => Some(Duration::from_secs(n)),
                    Err(_) => default,
                },
                Err(_) => default,
            }
        };
        PoolConfig {
            max_size: env_u32("MAX_SIZE", d.max_size),
            min_idle: Some(env_u32("MIN_IDLE", d.min_idle.unwrap_or(0))),
            connect_timeout: env_secs("CONNECT_TIMEOUT_SECS", Some(d.connect_timeout)).unwrap_or(d.connect_timeout),
            idle_timeout: env_secs("IDLE_TIMEOUT_SECS", d.idle_timeout),
            max_lifetime: env_secs("MAX_LIFETIME_SECS", d.max_lifetime),
        }
    }

    fn apply<M: ManageConnection>(self, builder: r2d2::Builder<M>) -> r2d2::Builder<M> {
        builder.max_size(self.max_size).min_idle(self.min_idle).connection_timeout(self.connect_timeout).idle_timeout(self.idle_timeout).max_lifetime(self.max_lifetime)
    }
}

/// A process-wide, keyed cache of pools: one real [`r2d2::Pool<M>`] per
/// distinct key (a future `db` caller would use the connection string
/// itself), created lazily on first use and shared by every later
/// caller with the same key. Generic over `M: ManageConnection` so a
/// Postgres-with-TLS pool and a plain-Postgres pool — two different
/// concrete types, since TLS-vs-not is decided per connection string,
/// not globally — get their own independent `PoolRegistry` instances
/// with zero duplicated logic, and so does any future resource kind.
pub struct PoolRegistry<M: ManageConnection> {
    pools: Mutex<HashMap<String, Pool<M>>>,
}

impl<M: ManageConnection> Default for PoolRegistry<M> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M: ManageConnection> PoolRegistry<M> {
    pub fn new() -> Self {
        PoolRegistry { pools: Mutex::new(HashMap::new()) }
    }

    /// The pool for `key`, creating it (via `make_manager`) on first
    /// call for that key. `make_manager` is only invoked when no pool
    /// for `key` exists yet. Returns the manager's own connect error
    /// verbatim (stringified) rather than a pool-specific wrapper.
    ///
    /// `r2d2::Pool::builder().build()` (unlike `build_unchecked`) does
    /// validate the manager, but only up to `min_idle` connections worth
    /// — with this module's default `min_idle: Some(0)`, that means
    /// ZERO connections at build time, so `get_or_create` alone can
    /// succeed even for an unreachable server/bad connection string.
    /// The real "fail now, not on the first query later" guarantee has
    /// to come from the caller following `get_or_create` with an
    /// immediate `pool.get()` in the same call — that `.get()` is where
    /// a bad connection string actually surfaces.
    pub fn get_or_create(&self, key: &str, config: PoolConfig, make_manager: impl FnOnce() -> Result<M, String>) -> Result<Pool<M>, String> {
        let mut pools = self.pools.lock().unwrap();
        if let Some(pool) = pools.get(key) {
            return Ok(pool.clone());
        }
        let manager = make_manager()?;
        let pool = config.apply(Pool::builder()).build(manager).map_err(|e| e.to_string())?;
        pools.insert(key.to_string(), pool.clone());
        Ok(pool)
    }

    /// Number of distinct keys with a live pool — test/diagnostic
    /// visibility into the registry, not used on any hot path.
    #[cfg(test)]
    pub fn pool_count(&self) -> usize {
        self.pools.lock().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A trivial in-memory "connection" — a unique id per physical
    /// connection created, so tests can assert on REUSE (same id handed
    /// out twice) vs. NEW (a fresh, never-seen id) without needing a
    /// real database at all. `ManageConnection` is the only contract
    /// this module actually depends on.
    struct CountingManager {
        next_id: AtomicU32,
        fail: bool,
    }

    #[derive(Debug)]
    struct CountingConn(u32);

    #[derive(Debug)]
    struct CountingError(String);
    impl std::fmt::Display for CountingError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.0)
        }
    }
    impl std::error::Error for CountingError {}

    impl ManageConnection for CountingManager {
        type Connection = CountingConn;
        type Error = CountingError;
        fn connect(&self) -> Result<Self::Connection, Self::Error> {
            if self.fail {
                return Err(CountingError("simulated connect failure".to_string()));
            }
            Ok(CountingConn(self.next_id.fetch_add(1, Ordering::SeqCst)))
        }
        fn is_valid(&self, _conn: &mut Self::Connection) -> Result<(), Self::Error> {
            Ok(())
        }
        fn has_broken(&self, _conn: &mut Self::Connection) -> bool {
            false
        }
    }

    fn manager() -> CountingManager {
        CountingManager { next_id: AtomicU32::new(0), fail: false }
    }

    #[test]
    fn same_key_shares_one_pool_different_keys_get_different_pools() {
        let registry: PoolRegistry<CountingManager> = PoolRegistry::new();
        let cfg = PoolConfig::default();

        let pool_a1 = registry.get_or_create("conn-a", cfg, || Ok(manager())).unwrap();
        let pool_a2 = registry.get_or_create("conn-a", cfg, || Ok(manager())).unwrap();
        assert_eq!(registry.pool_count(), 1, "same key must not create a second pool");

        let conn1 = pool_a1.get().unwrap().0;
        let conn2 = pool_a2.get().unwrap().0;
        assert_eq!(conn1, conn2, "same-key lookups must share one physical pool");

        registry.get_or_create("conn-b", cfg, || Ok(manager())).unwrap();
        assert_eq!(registry.pool_count(), 2, "a different key must get its own pool");
    }

    #[test]
    fn max_size_bounds_concurrently_checked_out_connections() {
        let registry: PoolRegistry<CountingManager> = PoolRegistry::new();
        let cfg = PoolConfig { max_size: 2, connect_timeout: Duration::from_millis(200), ..PoolConfig::default() };
        let pool = registry.get_or_create("bounded", cfg, || Ok(manager())).unwrap();

        let _c1 = pool.get().unwrap();
        let _c2 = pool.get().unwrap();
        let third = pool.get();
        assert!(third.is_err(), "a third checkout must be rejected, not silently exceed max_size");
    }

    #[test]
    fn dropping_a_checked_out_connection_returns_it_to_the_pool_for_reuse() {
        let registry: PoolRegistry<CountingManager> = PoolRegistry::new();
        let cfg = PoolConfig { max_size: 1, ..PoolConfig::default() };
        let pool = registry.get_or_create("reuse", cfg, || Ok(manager())).unwrap();

        let first_id = {
            let conn = pool.get().unwrap();
            conn.0
            // `conn` (an r2d2::PooledConnection) drops here.
        };
        let second_id = pool.get().unwrap().0;
        assert_eq!(first_id, second_id, "dropping must return the connection, not leak/close it");
    }

    #[test]
    fn get_or_create_alone_does_not_validate_the_connection_with_the_default_lazy_min_idle() {
        let registry: PoolRegistry<CountingManager> = PoolRegistry::new();
        let cfg = PoolConfig::default();
        let result = registry.get_or_create("bad", cfg, || Ok(CountingManager { next_id: AtomicU32::new(0), fail: true }));
        assert!(result.is_ok(), "get_or_create alone must succeed -- it only builds a pool object, it never proves a connection works");
    }

    #[test]
    fn a_bad_connection_string_fails_fast_once_a_real_checkout_happens() {
        let registry: PoolRegistry<CountingManager> = PoolRegistry::new();
        let cfg = PoolConfig::default();
        let pool = registry.get_or_create("bad", cfg, || Ok(CountingManager { next_id: AtomicU32::new(0), fail: true })).unwrap();
        assert!(pool.get().is_err(), "the first real checkout must surface the connect failure");
    }

    #[test]
    fn pool_config_from_env_overrides_and_falls_back_per_field() {
        // SAFETY (test-only): sets/removes env vars for this test's own
        // unique prefix -- no other test reads NIRDOSHA_TESTPFX_*.
        unsafe {
            std::env::set_var("NIRDOSHA_TESTPFX_POOL_MAX_SIZE", "42");
            std::env::remove_var("NIRDOSHA_TESTPFX_POOL_MIN_IDLE");
            std::env::set_var("NIRDOSHA_TESTPFX_POOL_IDLE_TIMEOUT_SECS", "0");
        }
        let cfg = PoolConfig::from_env("TESTPFX");
        assert_eq!(cfg.max_size, 42, "explicit env var must override the default");
        assert_eq!(cfg.min_idle, PoolConfig::default().min_idle, "unset env var must fall back to default");
        assert_eq!(cfg.idle_timeout, None, "0 must mean disabled, not Duration::from_secs(0)");
        unsafe {
            std::env::remove_var("NIRDOSHA_TESTPFX_POOL_MAX_SIZE");
            std::env::remove_var("NIRDOSHA_TESTPFX_POOL_IDLE_TIMEOUT_SECS");
        }
    }
}
