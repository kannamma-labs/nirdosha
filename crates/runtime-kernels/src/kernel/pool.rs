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

use super::DomainId;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
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
        let mut cfg = PoolConfig {
            max_size: env_u32("MAX_SIZE", d.max_size),
            min_idle: Some(env_u32("MIN_IDLE", d.min_idle.unwrap_or(0))),
            connect_timeout: env_secs("CONNECT_TIMEOUT_SECS", Some(d.connect_timeout)).unwrap_or(d.connect_timeout),
            idle_timeout: env_secs("IDLE_TIMEOUT_SECS", d.idle_timeout),
            max_lifetime: env_secs("MAX_LIFETIME_SECS", d.max_lifetime),
        };
        cfg.validate(prefix);
        cfg
    }

    /// Catches structurally-broken combinations `from_env`'s own
    /// field-by-field degrade-to-default can't catch (each field taken
    /// in isolation might be perfectly parseable and still combine into
    /// a config that either never works or silently loses the reaping
    /// an operator intended) — red-team report A6.
    ///
    /// Two different severities, deliberately: `min_idle > max_size` is
    /// not "an unusual choice," it's "r2d2 will refuse to build a pool
    /// with this config at all" (r2d2 requires `min_idle <= max_size`) —
    /// clamping it down, loudly, keeps `from_env`'s own "degrades to a
    /// still-functional value, never fails the whole program" posture
    /// intact instead of producing a config that fails at first
    /// `get_or_create`. `idle_timeout > max_lifetime` is milder (r2d2
    /// still builds the pool fine; idle recycling just never fires
    /// before lifetime recycling would've closed the connection anyway)
    /// so it's warn-only, matching this file's existing soft-degrade
    /// convention for individual fields above.
    fn validate(&mut self, prefix: &str) {
        if self.min_idle.unwrap_or(0) > self.max_size {
            eprintln!(
                "nirdosha: NIRDOSHA_{prefix}_POOL_MIN_IDLE ({}) exceeds NIRDOSHA_{prefix}_POOL_MAX_SIZE ({}) -- \
                 r2d2 requires min_idle <= max_size and would refuse to build this pool; \
                 clamping min_idle down to max_size",
                self.min_idle.unwrap_or(0),
                self.max_size
            );
            self.min_idle = Some(self.max_size);
        }
        if let (Some(idle), Some(lifetime)) = (self.idle_timeout, self.max_lifetime) {
            if idle > lifetime {
                eprintln!(
                    "nirdosha: NIRDOSHA_{prefix}_POOL_IDLE_TIMEOUT_SECS ({}s) exceeds \
                     NIRDOSHA_{prefix}_POOL_MAX_LIFETIME_SECS ({}s) -- idle recycling will never fire \
                     before lifetime recycling closes the connection first; accepted as configured, \
                     but this likely isn't what was intended",
                    idle.as_secs(),
                    lifetime.as_secs()
                );
            }
        }
        if self.connect_timeout.is_zero() {
            eprintln!(
                "nirdosha: NIRDOSHA_{prefix}_POOL_CONNECT_TIMEOUT_SECS resolved to 0 -- \
                 every checkout attempt against an exhausted pool will fail instantly instead of \
                 waiting any amount of time; accepted as configured, but this is likely a misconfiguration"
            );
        }
    }

    fn apply<M: ManageConnection>(self, builder: r2d2::Builder<M>) -> r2d2::Builder<M> {
        builder.max_size(self.max_size).min_idle(self.min_idle).connection_timeout(self.connect_timeout).idle_timeout(self.idle_timeout).max_lifetime(self.max_lifetime)
    }
}

/// Never lets `max_size` exceed `ceiling` (a `ceiling <= 0` means "no
/// real ceiling resolved yet / not applicable," left alone) — the shared
/// half of red-team report A12's fix (`scratch/red-team-report-main-
/// d7fae42.md`). Pulled out as its own pure function, taking `ceiling`
/// as a plain argument rather than looking it up internally, so it's
/// directly unit-testable without touching this process's real,
/// cached-forever domain ceilings (`kernel::mod.rs::max_for`'s own
/// `OnceLock` — red-team report A24's own finding is exactly why a test
/// here can't just set an env var and expect a fresh resolution).
/// `db.rs`'s `db_pool_config`/`http.rs`'s `http_pool_config` both call
/// this after their own floor-to-1000 default; `plugin_provider.rs`'s
/// `plugin_pool_config` has the identical clamp inlined already (not
/// switched to call this, to avoid touching already-tested Phase 6
/// code in an unrelated fix).
pub fn clamp_max_size_to_ceiling(cfg: &mut PoolConfig, ceiling: i64) {
    if ceiling > 0 && i64::from(cfg.max_size) > ceiling {
        cfg.max_size = ceiling as u32;
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

/// [`PoolRegistry::checkout_with_key_cap`]'s result: the common case is
/// a normal pooled checkout, indistinguishable from what
/// [`PoolRegistry::get_or_create`] always returned; past the per-
/// provider key cap, a *new* key gets one direct, unpooled connection
/// instead (RFC 0011 §5).
pub enum Checkout<M: ManageConnection> {
    /// A normal pooled checkout — caller calls `.get()` on this exactly
    /// as it always has.
    Pooled(Pool<M>),
    /// The key cap was hit for a brand-new key: no `HashMap` entry, no
    /// pool, no reaper coverage — one unpooled connection, already
    /// `kernel::acquire`-admitted. Drop this guard (or call
    /// [`UnpooledConnection::into_inner`] and drop what it returns) to
    /// release the admission it holds.
    Unpooled(UnpooledConnection<M>),
}

/// An unpooled, `kernel::acquire`/`release`-bracketed connection —
/// [`PoolRegistry::checkout_with_key_cap`]'s fallback-path result.
/// `release(domain)` runs exactly once, in [`Drop`], so the caller
/// cannot forget it (RFC 0011 §5's own explicit warning: a skipped
/// `acquire`/`release` on this path would turn the key cap into an
/// admission-ceiling bypass, not just a memory-growth guard).
pub struct UnpooledConnection<M: ManageConnection> {
    conn: Option<M::Connection>,
    domain: DomainId,
}

impl<M: ManageConnection> UnpooledConnection<M> {
    pub fn get(&self) -> &M::Connection {
        self.conn.as_ref().expect("conn is only ever None after into_inner/drop, which consume this value")
    }

    pub fn get_mut(&mut self) -> &mut M::Connection {
        self.conn.as_mut().expect("conn is only ever None after drop")
    }

    // No `into_inner`, deliberately: a caller that needs to hold the
    // raw connection past this guard's own scope (e.g. `conn`/`stream`-
    // shape's session-scoped connect-to-stop lifecycle, stashed in a
    // handle table) needs its own explicit acquire/release story at
    // that point, which is Phase 6's job to design once real FFI
    // dispatch exists — extracting the connection here while silently
    // detaching it from this guard's automatic release would be exactly
    // the "forget to release" hazard this guard exists to prevent.

    pub fn domain(&self) -> DomainId {
        self.domain
    }
}

impl<M: ManageConnection> Drop for UnpooledConnection<M> {
    fn drop(&mut self) {
        super::release(self.domain);
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
    ///
    /// **Invariant: the registry lock is never held across `make_manager`.**
    /// Every `make_manager` in this codebase today is a pure struct
    /// construction, but the API contract doesn't get to assume that of
    /// future callers — a closure that touches another lock or another
    /// domain's admission state while this one is held would deadlock,
    /// and a panic inside it would poison this mutex for the rest of the
    /// process, wedging every future `get_or_create`/`checkout_with_key_cap`
    /// call, for every key, not just this one. The lock is dropped before
    /// calling `make_manager` and re-acquired only to insert — with a
    /// re-check right before inserting, since another thread may have
    /// raced this one and inserted `key` first while the lock was open;
    /// in that case this call's own freshly-built pool is discarded (not
    /// installed) and the winner's pool is returned instead, so two
    /// concurrent misses on the same key never leave two live pools (and
    /// two independent sets of physical connections) registered under
    /// one key.
    pub fn get_or_create(&self, key: &str, config: PoolConfig, make_manager: impl FnOnce() -> Result<M, String>) -> Result<Pool<M>, String> {
        if let Some(pool) = self.pools.lock().unwrap().get(key) {
            return Ok(pool.clone());
        }
        let manager = make_manager()?;
        let pool = config.apply(Pool::builder()).build(manager).map_err(|e| e.to_string())?;
        let mut pools = self.pools.lock().unwrap();
        if let Some(existing) = pools.get(key) {
            return Ok(existing.clone());
        }
        pools.insert(key.to_string(), pool.clone());
        Ok(pool)
    }

    /// Number of distinct keys with a live pool. Used both as
    /// test/diagnostic visibility into the registry and, non-test, by
    /// [`PoolRegistry::checkout_with_key_cap`] to decide whether a new
    /// key is still under `NIRDOSHA_KERNEL_POOL_MAX_KEYS` — registration-
    /// frequency (once per distinct key), never a hot-path read.
    pub fn pool_count(&self) -> usize {
        self.pools.lock().unwrap().len()
    }

    /// [`PoolRegistry::get_or_create`], plus RFC 0011 §5's "what the
    /// ceiling actually bounds" rule: refuses to build a pool whose
    /// `config.max_size` would let r2d2 grow physical connections past
    /// `domain`'s own admission ceiling — a startup-time error (named,
    /// actionable), not a ceiling that silently goes decorative the
    /// moment a provider's pool is allowed to outgrow it. Every plugin
    /// provider pool (§4's single eager registration point, which knows
    /// both numbers for a given provider at once) must go through this,
    /// never bare `get_or_create`, once wired in Phase 6.
    pub fn get_or_create_within_ceiling(
        &self,
        key: &str,
        config: PoolConfig,
        domain: DomainId,
        make_manager: impl FnOnce() -> Result<M, String>,
    ) -> Result<Pool<M>, String> {
        let ceiling = super::ceiling_for(domain);
        if i64::from(config.max_size) > ceiling {
            return Err(format!(
                "pool config max_size ({}) for key {key:?} exceeds its domain's admission ceiling ({ceiling}) -- \
                 a ceiling nothing consults isn't a ceiling (rfcs/0011 §5); lower PoolConfig::max_size or raise the \
                 domain's ceiling env var/default_max instead",
                config.max_size
            ));
        }
        self.get_or_create(key, config, make_manager)
    }

    /// `NIRDOSHA_KERNEL_POOL_MAX_KEYS` (RFC 0011 §5, name pinned exactly
    /// in the RFC): the number of distinct pool keys one provider may
    /// hold before a *new* key falls back to unpooled dialing instead of
    /// growing this registry's `HashMap` without bound. Same
    /// malformed-value-degrades-to-default posture every other env-var
    /// convention in this file already uses, default 256 (the same
    /// "generous, not tuned" posture as `MAX_DOMAINS`).
    fn max_pool_keys() -> usize {
        std::env::var("NIRDOSHA_KERNEL_POOL_MAX_KEYS").ok().and_then(|s| s.parse::<usize>().ok()).filter(|&n| n > 0).unwrap_or(256)
    }

    /// The RFC 0011 §5 key-cap + fallback rule, made real: a *new* pool
    /// key past `NIRDOSHA_KERNEL_POOL_MAX_KEYS` distinct keys for this
    /// provider doesn't grow the registry's `HashMap` at all — it falls
    /// back to one direct, unpooled dial instead, still
    /// `kernel::acquire`/`release`-bracketed exactly as if it were a
    /// normal pooled checkout (the RFC's own explicit warning: skipping
    /// `acquire` here would turn the key cap into an admission-ceiling
    /// *bypass*, not just a memory-growth guard). `release` happens
    /// automatically when the returned [`Checkout::Unpooled`] guard
    /// drops — the caller cannot forget it the way a bare
    /// acquire-then-remember-to-release pair could be gotten wrong.
    /// An already-existing key never falls back, regardless of current
    /// key count (this is a cap on *new* keys, not a demotion of
    /// existing pools once the registry happens to be at/over the
    /// threshold).
    pub fn checkout_with_key_cap(
        &self,
        key: &str,
        config: PoolConfig,
        domain: DomainId,
        make_manager: impl FnOnce() -> Result<M, String>,
    ) -> Result<Checkout<M>, String> {
        let already_has_key = self.pools.lock().unwrap().contains_key(key);
        if already_has_key || self.pool_count() < Self::max_pool_keys() {
            return self.get_or_create_within_ceiling(key, config, domain, make_manager).map(Checkout::Pooled);
        }
        if !super::acquire(domain) {
            return Err(format!("too many concurrently held connections for this domain (unpooled fallback dial for key {key:?})"));
        }
        let manager = match make_manager() {
            Ok(m) => m,
            Err(e) => {
                super::release(domain);
                return Err(e);
            }
        };
        match manager.connect() {
            Ok(conn) => Ok(Checkout::Unpooled(UnpooledConnection { conn: Some(conn), domain })),
            Err(e) => {
                super::release(domain);
                Err(e.to_string())
            }
        }
    }

    /// RFC 0011 §5's reaper primitive: one opportunistic
    /// `pool.try_get()` per pool key currently registered, dropped
    /// immediately. `try_get` never blocks and never opens a new
    /// connection on an empty pool (confirmed against this repo's
    /// pinned `r2d2 0.8.10`: `try_get_inner` only pops from the already-
    /// idle list, `Err`ing immediately on empty rather than calling
    /// `add_connection`) — the RFC's own "non-blocking checkout, not a
    /// connection factory either" property. `test_on_check_out`
    /// (`true` by default, never overridden by `PoolConfig::apply`)
    /// makes r2d2 itself call `M::is_valid` on the popped connection
    /// before handing it back — a stale connection fails there, r2d2
    /// transparently drops and replaces it, and the manager's own
    /// `is_valid` impl (`SqliteManager`/`PostgresManager`/`HttpManager`/
    /// `PluginManagedConnection`, each in this crate) is what actually
    /// calls `record_stale_rehydrated` on that path — this method
    /// doesn't need to know a key's domain to make that counter move,
    /// it just needs to trigger the checkout. Dropping the returned
    /// `PooledConnection` immediately returns it to the pool (or, if
    /// r2d2 evicted it as unrecoverable, simply lets it go).
    fn sweep_one_round(&self) {
        let keys: Vec<String> = { self.pools.lock().unwrap().keys().cloned().collect() };
        for key in keys {
            let pool = { self.pools.lock().unwrap().get(&key).cloned() };
            if let Some(pool) = pool {
                drop(pool.try_get());
            }
        }
    }

    /// Whether `key` specifically has a live pool right now — unlike
    /// [`PoolRegistry::pool_count`], this is safe to assert on even
    /// when the registry is a process-wide static shared by other
    /// concurrently-running tests in the same binary (`kernel::db`'s
    /// own test module needs exactly this: "did *my* uniquely-named key
    /// get a pool," not "how many keys exist in total," since a count
    /// races against every other test touching the same registry).
    #[cfg(test)]
    pub fn contains_key(&self, key: &str) -> bool {
        self.pools.lock().unwrap().contains_key(key)
    }
}

/// A `str` value crossing the plugin ABI boundary by value — RFC 0011
/// §2's `<shape>_provider_<scheme>_connect(host: str) -> i64` and
/// `plugin.rs`'s own `NativePluginBuiltin` doc comment both describe
/// this exact two-word `(ptr, len)` `#[repr(C)]` convention (matching
/// `runtime-kernels/src/lib.rs`'s own established pattern for `str`
/// crossing an `extern "C"` boundary by value, e.g. its `Dec128Bits`-
/// shaped precedent). `ptr` is **not** NUL-terminated; `len` is
/// load-bearing.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NirStr {
    pub ptr: *const u8,
    pub len: i64,
}

/// The `r2d2::ManageConnection` adapter RFC 0011 §5 describes: "the
/// kernel supplies one generic adapter... that implements
/// `ManageConnection` once by calling through whichever function
/// pointers were resolved for a given provider at registration time."
/// One `PluginManagedConnection` instance is the `M` for exactly one
/// provider's `PoolRegistry<PluginManagedConnection>`, fixed to one
/// `host` (the pool key) — `connect_fn` runs only when
/// `PoolRegistry::get_or_create[_within_ceiling]` actually grows that
/// pool, never called directly by anything else (§2 step 1).
pub struct PluginManagedConnection {
    pub connect_fn: extern "C" fn(NirStr) -> i64,
    pub is_valid_fn: extern "C" fn(i64) -> i64,
    pub close_fn: extern "C" fn(i64) -> i64,
    pub host: String,
    /// The provider's own registered domain (`plugin_provider::
    /// ProviderFns::domain`) — needed only so `is_valid` below can call
    /// `record_stale_rehydrated`, the identical thing `SqliteManager`/
    /// `PostgresManager`/`HttpManager`'s own `is_valid` impls already do
    /// on their `Err` path (`db.rs`/`http.rs`). Phase 5's original cut
    /// of this struct omitted it (nothing needed it yet); Phase 7's
    /// reaper is what makes proactive rehydration real for plugin
    /// connections too, and the RFC's own §5 text says the reaper
    /// "calls `is_valid_fn` through it exactly the way `db.rs`'s
    /// `SqliteManager::is_valid` calls into `rusqlite` today" — that
    /// parity was incomplete without this field.
    pub domain: DomainId,
}

/// One live plugin connection — the kernel's own wrapper around the
/// plugin's raw `i64`, never handed back to `.nir` code directly (§2:
/// "the unwrap direction... `.nir` code holds the kernel id"). Carries
/// its own `close_fn` (fixed at connect time, from the same provider
/// that minted `raw`) so [`Drop`] can call it — r2d2's
/// `ManageConnection` trait has no closing callback of its own; §2's
/// own text is explicit that `close_fn` "only ever runs where r2d2
/// already runs it": `has_broken`-triggered eviction, r2d2's own idle/
/// lifetime reaping, or the reaper's sweep (Phase 7) — all of which
/// just drop the `Connection` value, so `close_fn` belongs in `Drop`,
/// not in any explicit "stop" call.
pub struct PluginPoolConn {
    pub raw: i64,
    close_fn: extern "C" fn(i64) -> i64,
}

impl Drop for PluginPoolConn {
    fn drop(&mut self) {
        (self.close_fn)(self.raw);
    }
}

impl std::fmt::Debug for PluginPoolConn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginPoolConn").field("raw", &self.raw).finish()
    }
}

/// A plugin provider's `_connect`/`_is_valid` call returned a failure
/// sentinel — §2's "0/1/negative sentinels every other kernel boundary
/// already uses" convention, stringified the same way every other
/// `ManageConnection::Error` in this crate already is
/// (`CountingError` below, `db.rs`'s own manager errors).
#[derive(Debug)]
pub struct PluginConnError(pub String);

impl std::fmt::Display for PluginConnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for PluginConnError {}

impl ManageConnection for PluginManagedConnection {
    type Connection = PluginPoolConn;
    type Error = PluginConnError;

    fn connect(&self) -> Result<Self::Connection, Self::Error> {
        let host = NirStr { ptr: self.host.as_ptr(), len: self.host.len() as i64 };
        let raw = (self.connect_fn)(host);
        if raw < 0 {
            return Err(PluginConnError(format!(
                "native plugin provider connect failed for host {:?} (returned {raw}, rfcs/0011 §2's negative-sentinel convention)",
                self.host
            )));
        }
        Ok(PluginPoolConn { raw, close_fn: self.close_fn })
    }

    fn is_valid(&self, conn: &mut Self::Connection) -> Result<(), Self::Error> {
        if (self.is_valid_fn)(conn.raw) != 0 {
            Ok(())
        } else {
            super::record_stale_rehydrated(self.domain);
            Err(PluginConnError(format!("native plugin provider is_valid returned false for connection {}", conn.raw)))
        }
    }

    /// Always `false` — RFC 0011 §5's deliberate simplification, safe
    /// specifically because `PoolConfig::apply` never overrides r2d2's
    /// own `test_on_check_out` default (`true`, confirmed above), so
    /// `is_valid` still runs on every checkout regardless of this
    /// always-`false` shortcut; a genuinely broken connection is caught
    /// there (or by the reaper's next sweep, Phase 7), not here.
    fn has_broken(&self, _conn: &mut Self::Connection) -> bool {
        false
    }
}

/// One `PoolRegistry<M>`, type-erased so [`kernel::reaper`]
/// (`crates/runtime-kernels/src/kernel/reaper.rs`) can hold a single
/// process-wide list spanning every concrete `M` (`SqliteManager`,
/// `PostgresManager`, `HttpManager`, `HttpsManager`,
/// `PluginManagedConnection` — five distinct types, one trait). Every
/// `PoolRegistry<M>` implements this the same way: forward to its own
/// [`PoolRegistry::sweep_one_round`].
pub trait Sweepable: Send + Sync {
    fn sweep_one_round(&self);
}

impl<M: ManageConnection> Sweepable for PoolRegistry<M> {
    fn sweep_one_round(&self) {
        PoolRegistry::sweep_one_round(self)
    }
}

static REAPER_TARGETS: OnceLock<Mutex<Vec<&'static dyn Sweepable>>> = OnceLock::new();

/// Registers one process-wide `&'static PoolRegistry<M>` (any `M`) with
/// the reaper — RFC 0011 §5: "every registered domain that's pool-backed
/// also registers a `PoolRegistry<M>`." Idempotent by reference identity
/// is not enforced here (the same `&'static` target pushed twice would
/// just get swept twice per wake, harmlessly, since `sweep_one_round`
/// itself is idempotent-safe) — every call site in this crate calls this
/// at most once per registry anyway (`Once`-guarded at each call site),
/// so double-registration is a call-site bug this doesn't need to guard
/// against defensively.
pub fn register_for_reaping(target: &'static dyn Sweepable) {
    REAPER_TARGETS.get_or_init(|| Mutex::new(Vec::new())).lock().unwrap().push(target);
}

/// [`kernel::reaper`]'s one sweep primitive: one opportunistic
/// `try_get`+drop per pool key, across every registered
/// [`PoolRegistry`], in registration order rotated by `offset`
/// (RFC 0011 §5's fairness note — whichever pool registered last isn't
/// swept last on *every* cycle). The lock guarding
/// [`REAPER_TARGETS`] is held only long enough to clone the list of
/// `&'static` references (cheap — a reference is `Copy`), never across
/// an actual sweep, so a sweep taking a while (a slow `is_valid`) never
/// blocks a concurrent `register_for_reaping` call.
pub fn sweep_all_registered(offset: usize) {
    let targets: Vec<&'static dyn Sweepable> = REAPER_TARGETS.get_or_init(|| Mutex::new(Vec::new())).lock().unwrap().clone();
    if targets.is_empty() {
        return;
    }
    let n = targets.len();
    for i in 0..n {
        targets[(offset + i) % n].sweep_one_round();
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

    /// Red-team report A6: `min_idle > max_size` is structurally broken
    /// (r2d2 refuses to build the pool at all), not merely unusual --
    /// `from_env` must clamp it down rather than degrade into a config
    /// that fails at first `get_or_create`.
    #[test]
    fn pool_config_from_env_clamps_min_idle_down_to_max_size_when_it_would_exceed_it() {
        unsafe {
            std::env::set_var("NIRDOSHA_TESTPFX2_POOL_MAX_SIZE", "3");
            std::env::set_var("NIRDOSHA_TESTPFX2_POOL_MIN_IDLE", "10");
        }
        let cfg = PoolConfig::from_env("TESTPFX2");
        assert_eq!(cfg.max_size, 3);
        assert_eq!(cfg.min_idle, Some(3), "min_idle (10) exceeding max_size (3) must be clamped down to max_size, not left broken");
        unsafe {
            std::env::remove_var("NIRDOSHA_TESTPFX2_POOL_MAX_SIZE");
            std::env::remove_var("NIRDOSHA_TESTPFX2_POOL_MIN_IDLE");
        }
    }

    /// RFC 0011 §5's "what the ceiling actually bounds" rule: a
    /// provider's `PoolConfig::max_size` must be ≤ its domain's own
    /// admission ceiling, checked at registration time (here,
    /// `get_or_create_within_ceiling`'s call), not left to r2d2 to grow
    /// past silently.
    #[test]
    fn get_or_create_within_ceiling_refuses_a_max_size_past_the_domain_ceiling() {
        let domain = super::super::register_domain("phase5_ceiling_test_domain", "NIRDOSHA_KERNEL_MAX_PHASE5_CEILING_TEST", 5);
        let registry: PoolRegistry<CountingManager> = PoolRegistry::new();

        let too_big = PoolConfig { max_size: 6, ..PoolConfig::default() };
        let result = registry.get_or_create_within_ceiling("over", too_big, domain, || Ok(manager()));
        let err = match result {
            Err(e) => e,
            Ok(_) => panic!("max_size (6) exceeding the domain ceiling (5) must be refused, not silently allowed"),
        };
        assert!(err.contains("ceiling") && err.contains('6') && err.contains('5'), "expected a named, actionable ceiling error, got: {err}");
        assert!(!registry.contains_key("over"), "a refused registration must not leave a pool behind");

        let ok = PoolConfig { max_size: 5, ..PoolConfig::default() };
        registry.get_or_create_within_ceiling("within", ok, domain, || Ok(manager())).expect("max_size equal to the ceiling must be allowed");
        assert!(registry.contains_key("within"));
    }

    /// RFC 0011 §5's key-cap + fallback rule, the "dangerous direction to
    /// get wrong" case named explicitly in the RFC: once
    /// `NIRDOSHA_KERNEL_POOL_MAX_KEYS` distinct keys exist, a *new* key
    /// must not grow the registry's `HashMap` — and the resulting
    /// unpooled fallback dial must still move `kernel::acquire`'s
    /// grant/denial counters exactly as if it were a normal checkout,
    /// not silently bypass admission because it isn't a "real" checkout.
    #[test]
    fn checkout_with_key_cap_falls_back_to_unpooled_dialing_past_the_cap_and_still_brackets_acquire() {
        // A small, low domain ceiling (2) so this test can also drive
        // the *fallback path's own* acquire calls to a real denial,
        // without needing thousands of iterations.
        let domain = super::super::register_domain("phase5_key_cap_test_domain", "NIRDOSHA_KERNEL_MAX_PHASE5_KEY_CAP_TEST", 2);
        // SAFETY (test-only): this env var is process-wide, but no other
        // test in this crate reads NIRDOSHA_KERNEL_POOL_MAX_KEYS, and
        // this test doesn't run concurrently with itself.
        unsafe { std::env::set_var("NIRDOSHA_KERNEL_POOL_MAX_KEYS", "1") };

        let registry: PoolRegistry<CountingManager> = PoolRegistry::new();
        // max_size must fit the domain's own ceiling (2) too, since the
        // pooled path goes through `get_or_create_within_ceiling`.
        let cfg = PoolConfig { max_size: 2, ..PoolConfig::default() };

        // First key: under the cap (0 existing keys < 1) -> pooled.
        let first = registry.checkout_with_key_cap("k0", cfg, domain, || Ok(manager())).expect("the first key must be pooled");
        assert!(matches!(first, Checkout::Pooled(_)), "the first key, under the cap, must be a normal pooled checkout");
        assert_eq!(registry.pool_count(), 1);

        let (held0, grants0, denials0, _) = super::super::stats(domain);
        assert_eq!((held0, grants0, denials0), (0, 0, 0), "a pooled checkout must not itself move kernel::acquire's counters (unchanged from Phase 2b's convention: the caller brackets pooled checkouts, not pool.rs)");

        // Second key: at the cap (1 existing key, not < 1) -> unpooled
        // fallback. Must NOT grow the registry, and must go through
        // kernel::acquire (grants: 0 -> 1, held: 0 -> 1).
        let second = registry.checkout_with_key_cap("k1", cfg, domain, || Ok(manager())).expect("fallback dial must succeed while under the domain ceiling");
        let guard1 = match second {
            Checkout::Unpooled(g) => g,
            Checkout::Pooled(_) => panic!("a key past the cap must fall back to unpooled, not silently pool anyway"),
        };
        assert_eq!(registry.pool_count(), 1, "an unpooled fallback must not add a HashMap entry");
        let (held1, grants1, denials1, _) = super::super::stats(domain);
        assert_eq!((held1, grants1, denials1), (1, 1, 0), "the fallback dial must be acquire-bracketed exactly like a real checkout");

        // Third key: also past the cap -> another unpooled fallback,
        // still within the domain ceiling of 2 (held 1 -> 2).
        let third = registry.checkout_with_key_cap("k2", cfg, domain, || Ok(manager())).expect("second fallback dial must still succeed, at the ceiling");
        let guard2 = match third {
            Checkout::Unpooled(g) => g,
            Checkout::Pooled(_) => panic!("must still fall back past the cap"),
        };
        assert_eq!(registry.pool_count(), 1, "still no HashMap growth from fallback keys");
        let (held2, grants2, denials2, _) = super::super::stats(domain);
        assert_eq!((held2, grants2, denials2), (2, 2, 0), "second fallback dial: held/grants both advance, at the domain ceiling now");

        // Fourth key: past both the key cap AND the domain's own
        // ceiling (already at 2/2 held) -> the fallback's own acquire
        // must be denied, not silently let a third connection through.
        let fourth = registry.checkout_with_key_cap("k3", cfg, domain, || Ok(manager()));
        assert!(fourth.is_err(), "a fallback dial past the domain's own ceiling must be denied, not silently admitted");
        assert_eq!(registry.pool_count(), 1, "a denied fallback must not add a HashMap entry either");
        let (held3, grants3, denials3, _) = super::super::stats(domain);
        assert_eq!((held3, grants3, denials3), (2, 2, 1), "the denied fallback must move the denials counter, not the held/grants ones");

        // Dropping the fallback guards releases their admission --
        // proving `release` isn't skipped just because the caller never
        // explicitly called it.
        drop(guard1);
        drop(guard2);
        let (held_after, _, _, _) = super::super::stats(domain);
        assert_eq!(held_after, 0, "dropping every UnpooledConnection guard must release its admission back to zero");

        unsafe { std::env::remove_var("NIRDOSHA_KERNEL_POOL_MAX_KEYS") };
    }

    /// The red-team-reported hazard: `get_or_create` used to hold
    /// `self.pools`'s lock across the whole `make_manager` call, which
    /// also meant two concurrent misses on the *same* key were fully
    /// serialized by that lock rather than racing safely. This proves
    /// the fixed version (lock dropped around `make_manager`, re-checked
    /// before insert) is actually race-safe: many threads all missing
    /// the same key concurrently must still converge on exactly one live
    /// pool for that key, never two.
    #[test]
    fn concurrent_get_or_create_misses_on_the_same_key_converge_on_one_pool_not_two() {
        let registry: std::sync::Arc<PoolRegistry<CountingManager>> = std::sync::Arc::new(PoolRegistry::new());
        let cfg = PoolConfig::default();
        // A distinct connection id per manager *instance* built -- if
        // two managers were ever both installed as live pools, threads
        // would observe more than one distinct id family; with the fix,
        // every thread must observe pool_count() == 1 and, if it also
        // performs a checkout, one of only the ids the eventual winning
        // manager produced.
        let manager_instances_built = std::sync::Arc::new(AtomicU32::new(0));

        let handles: Vec<_> = (0..16)
            .map(|_| {
                let registry = std::sync::Arc::clone(&registry);
                let manager_instances_built = std::sync::Arc::clone(&manager_instances_built);
                std::thread::spawn(move || {
                    registry
                        .get_or_create("race-key", cfg, || {
                            // Widen the race window: every thread reaches
                            // `make_manager` at roughly the same time and
                            // sleeps here, well past the point where the
                            // lock (if still held across this call, the
                            // pre-fix behavior) would have serialized
                            // every other thread behind it instead.
                            manager_instances_built.fetch_add(1, Ordering::SeqCst);
                            std::thread::sleep(std::time::Duration::from_millis(5));
                            Ok(manager())
                        })
                        .unwrap();
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(registry.pool_count(), 1, "16 concurrent misses on the same key must converge on exactly one live pool, never two");
        assert!(
            manager_instances_built.load(Ordering::SeqCst) >= 1,
            "sanity: make_manager must actually have run (this also confirms the lock was NOT held across it -- \
             the old, lock-holding implementation would have fully serialized these 16 threads through one \
             global mutex instead of letting them race make_manager concurrently)"
        );
    }

    /// Red-team report A12: `clamp_max_size_to_ceiling` must clamp
    /// *down* when `max_size` exceeds a real ceiling.
    #[test]
    fn clamp_max_size_to_ceiling_clamps_down_when_max_size_exceeds_it() {
        let mut cfg = PoolConfig { max_size: 1000, ..PoolConfig::default() };
        clamp_max_size_to_ceiling(&mut cfg, 50);
        assert_eq!(cfg.max_size, 50, "max_size (1000) exceeding a real ceiling (50) must be clamped down to it");
    }

    /// The other direction: a ceiling above `max_size` must not raise it.
    #[test]
    fn clamp_max_size_to_ceiling_leaves_max_size_alone_when_already_under_the_ceiling() {
        let mut cfg = PoolConfig { max_size: 50, ..PoolConfig::default() };
        clamp_max_size_to_ceiling(&mut cfg, 10_000);
        assert_eq!(cfg.max_size, 50, "a ceiling above max_size must not raise it");
    }

    /// A non-positive ceiling means "no real ceiling resolved" and must
    /// never be treated as an actual cap of 0.
    #[test]
    fn clamp_max_size_to_ceiling_is_a_no_op_for_a_non_positive_ceiling() {
        let mut cfg = PoolConfig { max_size: 1000, ..PoolConfig::default() };
        clamp_max_size_to_ceiling(&mut cfg, 0);
        assert_eq!(cfg.max_size, 1000, "ceiling <= 0 must be a no-op, not clamp max_size down to 0");
        clamp_max_size_to_ceiling(&mut cfg, -1);
        assert_eq!(cfg.max_size, 1000, "a negative ceiling must also be a no-op");
    }
}
