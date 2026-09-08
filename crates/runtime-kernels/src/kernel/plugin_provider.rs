//! RFC 0011 §2/§4's runtime provider table and kernel-owned
//! `HandleTable<PluginConn>` — the actual "look up a normalized scheme
//! at runtime, dispatch through *that handle's own recorded* function
//! pointers" mechanism `db.rs`'s `connect`/`nir_db_query`/`nir_db_stop`
//! fall through to when a URL's scheme doesn't match a built-in one.
//!
//! **Scope note, disclosed rather than left implicit**: this phase
//! wires exactly two consumers — `db_connect`'s `conn`-shape fallback
//! and the new `call_via` builtin's `call`-shape fallback.
//! `mq_connect_via`'s own fallback is explicitly deferred (RFC 0011's
//! own Context section, `crates/compiler/src/codegen.rs`'s `MQ_BUILTINS`
//! doc comment) — a follow-up that copies this module's `conn`-shape
//! path as its template, not a gap in this module's own design.
//!
//! **`_op`'s signature, pinned here since the RFC's own text never
//! fully nails it down**: a single `str` in, `str` out
//! (`extern "C" fn(i64, NirStr) -> NirStr`) — the same minimal-ABI
//! posture every other plugin function already has (RFC 0011 §2: "the
//! plugin's four functions never see a kernel id... never a
//! `handle(Kind)`"). This deliberately does **not** extend `db_query`/
//! `db_execute`'s full bind-value array (`NirBindValue`) across the
//! plugin boundary — that's a real, disclosed narrower cut, not
//! something the RFC's own text promises: a plugin-routed `db_query`/
//! `db_execute` call with any bind values supplied is a clean, named
//! error (below), not a silent drop of the bind values or an attempt to
//! marshal an aggregate across an ABI that's supposed to stay
//! scalar/`str`/handle-only. `db_execute`'s "affected row count" is
//! represented as `_op`'s returned string parsed as a decimal `i64` —
//! reusing the one `_op` signature for both `db_query`/`db_execute`
//! rather than inventing a second op-shaped function, at the cost of
//! that one parse-convention document requirement on plugin authors.

use super::pool::{self, Checkout, NirStr, PluginManagedConnection, PoolConfig, PoolRegistry};
use super::{DomainId, HandleTable};
use std::collections::HashMap;
use std::sync::{Mutex, Once, OnceLock};

/// One provider's shape-appropriate second function — `_op` for
/// `conn`/`stream`-shape (`db_provider_.../mq_provider_...`), `_request`
/// for `call`-shape (`<name>_provider_..._request`, RFC 0011 §2's
/// corrected `call` contract). Which variant a provider gets is decided
/// once, at registration time, by which sibling function
/// `NativePluginBuiltin::validate`/`validate_plugin_roster` found next
/// to `_connect` (`compiler/src/plugin.rs`) — never re-inferred here.
#[derive(Clone, Copy)]
pub enum ProviderOp {
    ConnStream { op_fn: extern "C" fn(i64, NirStr) -> NirStr },
    Call { request_fn: extern "C" fn(i64, NirStr, NirStr) -> NirStr },
}

/// Everything the kernel needs to dispatch through one registered
/// plugin provider — populated once, at startup, by
/// [`register`] (`codegen.rs` emits one `nir_kernel_register_plugin_
/// provider` call per distinct validated provider, right after that
/// same provider's `nir_kernel_register_domain` call, RFC 0011 §4).
/// `Copy` so a `PluginConn` (below) can carry its own snapshot of the
/// exact `ProviderFns` it was connected through, per RFC §2's
/// structural-safety argument: "no code path calls a provider's `_op`
/// based on anything but what that specific handle was given at
/// `_connect` time."
#[derive(Clone, Copy)]
pub struct ProviderFns {
    pub connect_fn: extern "C" fn(NirStr) -> i64,
    pub is_valid_fn: extern "C" fn(i64) -> i64,
    pub close_fn: extern "C" fn(i64) -> i64,
    pub op: ProviderOp,
    pub domain: DomainId,
}

static PLUGIN_PROVIDERS: OnceLock<Mutex<HashMap<String, ProviderFns>>> = OnceLock::new();

/// Registered once per distinct provider, at process startup, before
/// any user code runs (RFC 0011 §4 — the identical eager-registration
/// posture Phase 4 already established for domains, extended here to
/// also index dispatch function pointers). Keyed by **normalized
/// scheme alone**, not `(shape, scheme)`: `compiler/src/plugin.rs`'s
/// own `validate_plugin_roster` collision check already treats the
/// normalized-scheme namespace as global across every shape (a `conn`-
/// shape provider and a `call`-shape provider can't register the same
/// scheme identifier at all), so one flat map is the RFC-consistent
/// representation, not a simplification that loses information.
pub fn register(scheme: String, fns: ProviderFns) {
    let map = PLUGIN_PROVIDERS.get_or_init(|| Mutex::new(HashMap::new()));
    map.lock().unwrap_or_else(|e| e.into_inner()).insert(scheme, fns);
}

fn lookup(normalized_scheme: &str) -> Option<ProviderFns> {
    let map = PLUGIN_PROVIDERS.get_or_init(|| Mutex::new(HashMap::new()));
    map.lock().unwrap_or_else(|e| e.into_inner()).get(normalized_scheme).copied()
}

/// rfcs/0011-uniform-service-provider-model.md §2: "take the substring
/// before `://`, lowercase it, map `+`/`.`/`-` to `_`." **Duplicated
/// verbatim from `compiler/src/plugin.rs::normalize_scheme`** — the
/// RFC's own "one function, not two independent implementations that
/// could drift" instruction, kept in sync by hand since there is no
/// shared crate between `compiler` and `runtime-kernels` to put a
/// single copy in (this crate's own module doc: "can't depend on the
/// compiler crate at all"). Any change here needs the identical change
/// there, and vice versa — both copies carry this same test vector set.
pub fn normalize_scheme(scheme: &str) -> String {
    let bare = scheme.split("://").next().unwrap_or(scheme);
    bare.to_ascii_lowercase().chars().map(|c| if c == '+' || c == '.' || c == '-' { '_' } else { c }).collect()
}

/// RFC 0011 §2's credential-stripped `(provider, identity)` pool-key
/// rule: `scheme://user@host:port/path`, password and query string
/// both removed, username kept (a different user is a different
/// privilege scope). Best-effort against a malformed URL — a schemeless
/// or otherwise odd string just gets used mostly as-is rather than
/// erroring, since even a degenerate case still partitions distinct
/// URLs into distinct keys correctly, which is this function's only
/// real correctness obligation (it's a pool/cache key, not a validator).
pub fn pool_key_identity(normalized_scheme: &str, url: &str) -> String {
    let after_scheme = url.split("://").nth(1).unwrap_or(url);
    let before_query = after_scheme.split('?').next().unwrap_or(after_scheme);
    let identity = match before_query.find('@') {
        Some(at) => {
            let (userinfo, host_and_path) = before_query.split_at(at);
            let host_and_path = &host_and_path[1..]; // skip '@'
            let user = userinfo.split(':').next().unwrap_or(userinfo);
            format!("{user}@{host_and_path}")
        }
        None => before_query.to_string(),
    };
    format!("{normalized_scheme}://{identity}")
}

/// One process-wide `PoolRegistry<PluginManagedConnection>`, shared by
/// every registered provider regardless of shape — RFC 0011 §5: "every
/// plugin provider of a given shape reuses this same adapter type, so
/// `PoolRegistry<PluginManagedConnection>` is the one pool type covering
/// all of them." Providers don't collide within it because
/// [`pool_key_identity`]'s key is already `(normalized_scheme,
/// credential-stripped identity)`-shaped — two different providers can
/// never produce the same key, since normalized schemes are globally
/// unique per [`register`]'s own doc comment.
fn plugin_pool() -> &'static PoolRegistry<PluginManagedConnection> {
    static POOL: OnceLock<PoolRegistry<PluginManagedConnection>> = OnceLock::new();
    static REGISTERED: Once = Once::new();
    let pool_registry = POOL.get_or_init(PoolRegistry::new);
    // RFC 0011 §5: registers once with the reaper, lazily. Unlike
    // `db.rs`/`http.rs`'s per-backend registries (each pinned to one
    // domain), this single registry spans every registered provider's
    // own domain at once -- `PluginManagedConnection::domain` (set per
    // instance at `connect_conn_shape`/`call_via`'s construction time
    // below) is what lets `record_stale_rehydrated` still land against
    // the *right* domain per connection, not this registry as a whole.
    REGISTERED.call_once(|| pool::register_for_reaping(pool_registry));
    pool_registry
}

/// One kernel-owned `HandleTable<PluginConn>`, playing the identical
/// role `lib.rs`'s `db_table()` plays for core `db` — RFC 0011 §2:
/// "the kernel keeps one `HandleTable<PluginConn>` per shape." A single
/// shared table (not one per shape) is enough here for the same reason
/// [`plugin_pool`] is single and shared: a `PluginConn`'s own recorded
/// `ProviderFns` (not which table it lives in) is what makes dispatch
/// structurally safe, per §2's own argument.
pub fn plugin_conn_table() -> &'static HandleTable<PluginConn> {
    static TABLE: OnceLock<HandleTable<PluginConn>> = OnceLock::new();
    // A disjoint id range from `lib.rs`'s core `db_table()` (which
    // starts at 1) -- see `HandleTable::new_starting_at`'s own doc
    // comment for why this must never overlap.
    TABLE.get_or_init(|| HandleTable::new_starting_at(1i64 << 62))
}

/// Which of the two checkout shapes (RFC 0011 §5's key-cap fallback,
/// [`Checkout`]) this `PluginConn` was actually given. `Pooled` needs an
/// explicit `kernel::release` on close (mirroring exactly how
/// `lib.rs`'s `nir_db_stop` already releases `domain::db()` for core
/// connections); `Unpooled`'s own [`super::pool::UnpooledConnection`]
/// releases automatically on `Drop` (Phase 5), so nothing extra is
/// needed for that variant here.
pub enum PluginConnHold {
    Pooled(r2d2::PooledConnection<PluginManagedConnection>),
    Unpooled(super::pool::UnpooledConnection<PluginManagedConnection>),
}

/// The kernel-owned entry `HandleTable<PluginConn>` stores — RFC 0011
/// §2: "`PluginConn` pairing the plugin's raw `i64` with the
/// `ProviderFns` it was connected through." `.nir` code only ever holds
/// this table's own id; a plugin's four functions only ever receive the
/// raw `i64` unwrapped from here, on every call, not just `_connect`'s
/// return.
pub struct PluginConn {
    hold: PluginConnHold,
    pub provider: ProviderFns,
}

impl PluginConn {
    pub fn raw(&self) -> i64 {
        match &self.hold {
            PluginConnHold::Pooled(c) => c.raw,
            PluginConnHold::Unpooled(g) => g.get().raw,
        }
    }
}

impl Drop for PluginConn {
    fn drop(&mut self) {
        // `Unpooled`'s own guard releases its admission automatically
        // when `self.hold` itself drops, right after this fn returns —
        // nothing to do for that variant here. `Pooled` has no such
        // automatic release built in (a `PooledConnection`'s own Drop
        // only returns the checkout to the pool for reuse), so this is
        // the one place that release call has to live, symmetric with
        // the explicit `kernel::acquire` `connect_conn_shape`/
        // `connect_call_shape` (below) make for exactly this variant.
        if let PluginConnHold::Pooled(_) = &self.hold {
            super::release(self.provider.domain);
        }
    }
}

/// A plugin provider's `_connect`/`_is_valid` call returned a failure
/// sentinel, or the plugin ABI's own scalar `str` result (`_op`/
/// `_request`) needs to be reported as a `.nir`-visible `Err` — RFC 0011
/// §2's "0/1/negative sentinels every other kernel boundary already
/// uses" convention. `Display`/`Error`, matching every other
/// `ManageConnection::Error` in this crate.
pub fn no_provider_registered_error(normalized_scheme: &str) -> String {
    // RFC 0011 §2: "the error echoes only the normalized scheme... so
    // it can never leak a credential even though it does echo
    // tenant-controlled input" — never interpolate the raw url here.
    format!("no provider registered for scheme {normalized_scheme:?}")
}

/// `PoolConfig::from_env`'s own default `max_size` (10, `pool.rs`'s own
/// doc comment) has no idea what a given provider's admission ceiling
/// actually is — for a provider whose `default_max`/env-var ceiling is
/// below 10 (this module's own test coverage exercises exactly this: a
/// `default_max: Some(1)` provider), an un-clamped config would fail
/// `get_or_create_within_ceiling`'s own check on every single checkout,
/// making a legitimately low-ceiling provider unusable rather than
/// merely rate-limited. Clamping here — once, in the one place both
/// `connect_conn_shape` and `call_via` build a `PoolConfig` — keeps
/// `pool.rs`'s ceiling check doing its real job (catching a genuinely
/// oversized `max_size`) instead of also having to absorb "a provider's
/// ceiling happens to be smaller than the generic pool default."
fn plugin_pool_config(normalized_scheme: &str, domain: DomainId) -> PoolConfig {
    let mut cfg = PoolConfig::from_env(&format!("PLUGIN_{}", normalized_scheme.to_ascii_uppercase()));
    let ceiling = super::ceiling_for(domain);
    if ceiling > 0 && i64::from(cfg.max_size) > ceiling {
        cfg.max_size = ceiling as u32;
    }
    cfg
}

/// `db_connect`'s (and, once wired, `mq_connect_via`'s) fallback: a URL
/// whose scheme matched no built-in. Returns a real, disclosed error —
/// never a panic — for every failure mode: unregistered scheme, a
/// `call`-shape provider found where a `conn`/`stream`-shape one was
/// expected, admission denial, or the plugin's own connect failure.
pub fn connect_conn_shape(url: &str) -> Result<PluginConn, String> {
    let scheme = url.split("://").next().unwrap_or(url);
    let normalized = normalize_scheme(scheme);
    let Some(fns) = lookup(&normalized) else {
        return Err(no_provider_registered_error(&normalized));
    };
    let ProviderOp::ConnStream { .. } = fns.op else {
        return Err(format!(
            "provider registered for scheme {normalized:?} is call-shape (registered for call_via), not usable from db_connect/mq_connect_via"
        ));
    };
    let key = pool_key_identity(&normalized, url);
    let connect_fn = fns.connect_fn;
    let is_valid_fn = fns.is_valid_fn;
    let close_fn = fns.close_fn;
    let host = url.to_string();
    let domain = fns.domain;
    let make_manager = move || Ok(PluginManagedConnection { connect_fn, is_valid_fn, close_fn, host, domain });

    match plugin_pool().checkout_with_key_cap(&key, plugin_pool_config(&normalized, fns.domain), fns.domain, make_manager) {
        Ok(Checkout::Pooled(pool)) => {
            if !super::acquire(fns.domain) {
                return Err("too many concurrently held connections for this provider".to_string());
            }
            match pool.get() {
                Ok(pooled) => Ok(PluginConn { hold: PluginConnHold::Pooled(pooled), provider: fns }),
                Err(e) => {
                    super::release(fns.domain);
                    Err(e.to_string())
                }
            }
        }
        // The key-cap fallback's own `acquire` already ran, inside
        // `checkout_with_key_cap` itself (Phase 5) — this branch must
        // NOT acquire again, or every fallback connection would consume
        // two admission slots for one real connection.
        Ok(Checkout::Unpooled(guard)) => Ok(PluginConn { hold: PluginConnHold::Unpooled(guard), provider: fns }),
        Err(e) => Err(e),
    }
}

/// `db_query`/`db_execute`'s plugin-routed op dispatch — called only
/// after a `HandleTable<PluginConn>` lookup already found `conn`
/// (never resolved any other way, per §2's structural-safety
/// argument). `binds_present` is a clean, named error rather than a
/// silent drop — see this module's own doc comment for why bind values
/// don't cross the plugin `_op` boundary in this phase.
pub fn op(conn: &PluginConn, arg: &str, binds_present: bool) -> Result<String, String> {
    if binds_present {
        // "Inline the value into the query string instead" is not
        // actually actionable advice for a genuinely dynamic value --
        // `str` has no concatenation in this language (docs/LANGUAGE.md
        // §2), so the only "inline" a caller can do is a compile-time
        // literal already baked into the `sql` argument. Say that
        // plainly rather than pointing at a workaround that doesn't
        // exist for runtime-computed values.
        return Err("bind parameters are not supported for plugin-routed db_query/db_execute calls in this phase (rfcs/0011 §2) -- \
                     a dynamic value can't be spliced into the query string either, since this language has no string \
                     concatenation; only a query with every value already fixed at compile time works against a plugin-routed connection today"
            .to_string());
    }
    let ProviderOp::ConnStream { op_fn } = conn.provider.op else {
        return Err("internal error: a call-shape provider's PluginConn reached conn-shape op dispatch".to_string());
    };
    let raw = conn.raw();
    let arg = NirStr { ptr: arg.as_ptr(), len: arg.len() as i64 };
    let result = op_fn(raw, arg);
    // SAFETY: the plugin's own `_op` contract (RFC 0011 §2, `plugin.rs`'s
    // doc comment) is the same `(ptr, len)`-by-value convention every
    // other `str`-returning plugin function already uses -- the plugin
    // allocates its own buffer (typically `Box::leak`) and this is
    // never freed, the same permanent-leak posture `sha256_hex`/every
    // other `str`-returning native builtin already has.
    let bytes = if result.len == 0 { &[][..] } else { unsafe { std::slice::from_raw_parts(result.ptr, result.len as usize) } };
    String::from_utf8(bytes.to_vec()).map_err(|_| "plugin _op returned a value that is not valid UTF-8".to_string())
}

/// `call_via`'s fallback: a URL whose scheme isn't plain `http`/
/// `https`. Mirrors [`connect_conn_shape`] structurally (same admission
/// bracketing, same pool-key rule) but dispatches through `_request`
/// instead of `_op`, and — per RFC 0011 §2's corrected `call` contract
/// — brackets admission **per request**, not per session: `kernel::
/// acquire(domain)` around each checkout, `release` immediately after
/// this one call returns, rather than held across multiple calls the
/// way `conn`/`stream`'s handle-table session lifecycle is.
pub fn call_via(url: &str, path: &str, body: &str) -> Result<String, String> {
    let scheme = url.split("://").next().unwrap_or(url);
    let normalized = normalize_scheme(scheme);
    let Some(fns) = lookup(&normalized) else {
        return Err(no_provider_registered_error(&normalized));
    };
    let ProviderOp::Call { request_fn } = fns.op else {
        return Err(format!("provider registered for scheme {normalized:?} is conn/stream-shape (registered for db_connect/mq_connect_via), not usable from call_via"));
    };
    let key = pool_key_identity(&normalized, url);
    let connect_fn = fns.connect_fn;
    let is_valid_fn = fns.is_valid_fn;
    let close_fn = fns.close_fn;
    let host = url.to_string();
    let domain = fns.domain;
    let make_manager = move || Ok(PluginManagedConnection { connect_fn, is_valid_fn, close_fn, host, domain });

    let checkout = plugin_pool().checkout_with_key_cap(&key, plugin_pool_config(&normalized, fns.domain), fns.domain, make_manager)?;
    let raw = match &checkout {
        Checkout::Pooled(pool) => {
            if !super::acquire(fns.domain) {
                return Err("too many concurrently held connections for this provider".to_string());
            }
            match pool.get() {
                Ok(pooled) => pooled.raw,
                Err(e) => {
                    super::release(fns.domain);
                    return Err(e.to_string());
                }
            }
        }
        // Same as `connect_conn_shape`: the fallback's own `acquire`
        // already ran inside `checkout_with_key_cap`.
        Checkout::Unpooled(guard) => guard.get().raw,
    };

    let path_arg = NirStr { ptr: path.as_ptr(), len: path.len() as i64 };
    let body_arg = NirStr { ptr: body.as_ptr(), len: body.len() as i64 };
    let result = request_fn(raw, path_arg, body_arg);

    // Per-request release: this checkout was only ever meant to live for
    // the duration of this one call (RFC 0011 §2's corrected `call`
    // admission scope) — for the `Pooled` branch, release right here,
    // now that `_request` has returned, rather than holding it across
    // calls the way `conn`/`stream`'s session-scoped `PluginConn` does.
    // The `Unpooled` branch's guard (`checkout`, still in scope) releases
    // automatically when it drops at the end of this function.
    if let Checkout::Pooled(_) = &checkout {
        super::release(fns.domain);
    }

    let bytes = if result.len == 0 { &[][..] } else { unsafe { std::slice::from_raw_parts(result.ptr, result.len as usize) } };
    String::from_utf8(bytes.to_vec()).map_err(|_| "plugin _request returned a value that is not valid UTF-8".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Same test vectors as `compiler/src/plugin.rs`'s
    /// `normalize_scheme_matches_the_rfc_examples` — this module's own
    /// doc comment on `normalize_scheme` names that file as the sibling
    /// copy that must stay in sync; this test is what actually checks
    /// that claim rather than just asserting it in prose.
    #[test]
    fn normalize_scheme_matches_the_rfc_examples() {
        assert_eq!(normalize_scheme("mysql+tls://host"), "mysql_tls");
        assert_eq!(normalize_scheme("POSTGRESQL://host"), "postgresql");
        assert_eq!(normalize_scheme("s3.compat-v2://host"), "s3_compat_v2");
    }

    /// RFC 0011 §2's credential-stripped identity rule: password
    /// removed, username kept, query string removed.
    #[test]
    fn pool_key_identity_strips_password_and_query_but_keeps_username_and_path() {
        let key = pool_key_identity("s3", "s3://readonly:secret-pw@bucket.example.com:9000/my/path?token=abc123");
        assert_eq!(key, "s3://readonly@bucket.example.com:9000/my/path");
        assert!(!key.contains("secret-pw"), "password must never appear in a pool key");
        assert!(!key.contains("token=abc123"), "query string (a common credential carrier) must never appear in a pool key");
    }

    /// No userinfo at all -- a bare `host[:port][/path]` after the
    /// scheme, the common case for a provider whose credential lives
    /// entirely in the path or is implicit (e.g. IAM-role-based auth).
    #[test]
    fn pool_key_identity_with_no_userinfo_is_used_as_is() {
        let key = pool_key_identity("authedhttp", "authedhttp://api.example.com/v1/endpoint");
        assert_eq!(key, "authedhttp://api.example.com/v1/endpoint");
    }

    /// Rotating a password behind the same user/host/path must NOT
    /// change the identity -- RFC 0011 §2's own disclosed operational
    /// trap ("a real operational trap this identity rule creates"):
    /// same pool, same physical connections, until the pool evicts the
    /// stale-credentialed ones.
    #[test]
    fn pool_key_identity_is_unchanged_by_a_password_rotation() {
        let before = pool_key_identity("postgres_like", "custom://admin:pw1@h/db");
        let after = pool_key_identity("postgres_like", "custom://admin:pw2@h/db");
        assert_eq!(before, after, "rotating only the password must not change the pool key");
    }

    /// A different username IS a different privilege scope, and must
    /// key a different pool -- the other half of the same RFC rule.
    #[test]
    fn pool_key_identity_differs_for_different_usernames() {
        let admin = pool_key_identity("custom", "custom://admin:pw@h/db");
        let readonly = pool_key_identity("custom", "custom://readonly:pw@h/db");
        assert_ne!(admin, readonly, "a different user is a different privilege scope and must key a different pool");
    }

    /// Two URLs differing only in query parameters share a pool -- the
    /// RFC's own disclosed cost of excluding the query string entirely,
    /// stated as an explicit, checked property rather than an assumed
    /// one.
    #[test]
    fn pool_key_identity_ignores_all_query_parameters_including_connection_affecting_ones() {
        let a = pool_key_identity("custom", "custom://user@h/db?sslmode=require");
        let b = pool_key_identity("custom", "custom://user@h/db?sslmode=disable");
        assert_eq!(a, b, "query strings are excluded from identity entirely, including connection-affecting ones (RFC's own disclosed cost)");
    }

    /// Different normalized schemes must never collide on identity,
    /// even against an otherwise-identical rest-of-url -- the pool key
    /// is `(provider, identity)`, not identity alone.
    #[test]
    fn pool_key_identity_is_scoped_by_scheme() {
        let a = pool_key_identity("schemea", "schemea://user@h/db");
        let b = pool_key_identity("schemeb", "schemeb://user@h/db");
        assert_ne!(a, b);
    }

    /// Caught by `call_via_brackets_admission_per_request_not_per_session`
    /// (`native_plugin_codegen.rs`) failing before this existed:
    /// `PoolConfig::from_env`'s own default `max_size` (10) is bigger
    /// than a low-ceiling provider's own admission ceiling, so an
    /// un-clamped config would fail `get_or_create_within_ceiling`'s
    /// check on *every* checkout for such a provider -- making a
    /// legitimately low-ceiling provider unusable, not merely rate-
    /// limited. This is the regression test for that fix, isolated from
    /// the full compiled-binary test that originally caught it.
    #[test]
    fn plugin_pool_config_clamps_max_size_down_to_a_low_domain_ceiling() {
        let domain = super::super::register_domain("plugin_pool_config_clamp_test_domain", "NIRDOSHA_KERNEL_MAX_PLUGIN_POOL_CONFIG_CLAMP_TEST", 1);
        let cfg = plugin_pool_config("clamp_test_scheme", domain);
        assert_eq!(cfg.max_size, 1, "max_size must be clamped down to the domain's own ceiling, not left at PoolConfig::default's 10");
    }

    /// The other direction: a generous ceiling must not be clamped
    /// *up* -- `PoolConfig::default`'s own `max_size` (10) stays
    /// whatever it resolved to.
    #[test]
    fn plugin_pool_config_leaves_max_size_alone_when_already_under_a_generous_ceiling() {
        let domain = super::super::register_domain("plugin_pool_config_no_clamp_test_domain", "NIRDOSHA_KERNEL_MAX_PLUGIN_POOL_CONFIG_NO_CLAMP_TEST", 10_000);
        let cfg = plugin_pool_config("no_clamp_test_scheme", domain);
        assert_eq!(cfg.max_size, PoolConfig::default().max_size, "a ceiling far above the default max_size must not change it");
    }
}
