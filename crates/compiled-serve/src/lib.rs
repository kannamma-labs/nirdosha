//! `crates/compiled-serve` — ROADMAP B8's compiled `serve` mode, the
//! framework half `rfcs/0010-landing-and-serve-exposure.md`'s own
//! Status box discloses as a deliberate departure from B8's real,
//! primitives-first shape (`examples/features/51_compiled_serve.nir`'s
//! hand-written `tcp_listener`/`accept` loop): a dispatch table built
//! from a compiled program's own exposure set (RFC 0010), with real
//! RBAC/session/CORS/rate-limiting machinery around it, additive
//! alongside the primitives-only style, never a replacement for it.
//!
//! **What this crate owns, and what it deliberately doesn't.** Every
//! [`RouteHandler`] is a real function pointer with a fixed, ABI-stable
//! signature — this crate's job is the HTTP transport around calling
//! it (parsing, timeouts, admission, sessions, CORS, rate limiting,
//! marshaling the JSON body in and the JSON result + optional cookie
//! out) and nothing about *what* a route does or *whether* its caller
//! is authorized: a `requires`-gated route's real RBAC check
//! (`nir_check_role`/`nir_extract_claim`) happens inside the handler
//! itself, compiled LLVM code this crate only ever calls through a
//! pointer, never reimplements. See [`RouteHandler`]'s own doc comment
//! for the exact contract a codegen-generated (or, for now, hand-
//! written test) handler must honor.
//!
//! **Identity is real; route dispatch is wired to `codegen.rs` too, as
//! of a later 2026-09 session than this comment originally described.**
//! `identity`'s bearer-token verification (demo mode's self-minted,
//! ephemeral-but-genuinely-signed tokens, and production mode's real IdP
//! JWKS — one provider or, since Multi-IdP registry landed, more than
//! one, dispatched by a token's own issuer claim, see
//! [`auth_providers_from_env`]) is real, exercised end to end both by
//! this crate's own tests (against hand-written `extern "C"` test
//! routes) *and* by `nirdosha build --serve`'s real production path:
//! `codegen.rs`'s Stage 3 emits real per-route wrapper functions from a
//! compiled program's own exposure set (RFC 0010) and links this crate
//! in — see `crates/compiler/tests/codegen.rs`'s
//! `compiled_serve_production_path_exposes_a_route_via_a_real_http_post_with_a_body`.
//! `nirdosha build --serve` still bakes in demo mode only at the
//! *codegen* layer (`ServeCodegenOptions` has no identity fields); real
//! production identity is chosen entirely at *runtime*, by this crate's
//! own env-var read (`auth_providers_from_env`) — no rebuild needed to
//! point a given binary at a different IdP.

use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use nirdosha_runtime_kernels::kernel::{self, domain};
use nirdosha_runtime_kernels::constant_time_eq;

mod http;
mod identity;
mod ratelimit;

pub use http::MAX_BODY_BYTES;
pub use identity::{AuthConfig, VerifiedClaims};

/// One exposed route's real dispatch target — a plain function pointer,
/// never a name string (`rfcs/0010-landing-and-serve-exposure.md`'s own
/// "strictly better than the old interpreter's `call_named` string
/// dispatch" framing). The fixed ABI every handler (codegen-generated
/// or, today, hand-written for testing) must implement:
///
/// - `args_json_{ptr,len}`: the request body, exactly as received —
///   `typeck`'s own "positional, by declared `Ty`" marshalling contract
///   means a handler decodes this itself (the same JSON args array a
///   compiled `transact` replay trampoline already decodes via
///   `nir_transact_decode_args`'s sibling mechanism, conceptually — a
///   real per-route decode is `codegen.rs`'s job once wired, not this
///   crate's).
/// - `identity_json_{ptr,len}`: `""`/zero-length when the request
///   carried no `Authorization` header at all, otherwise a JSON object
///   this crate's own request pipeline already resolved and *really*
///   verified upstream (`identity::validate_token`, real JWT/JWKS
///   signature + expiry checking — see that module) — a handler never
///   re-parses a raw `Authorization` header itself. Shape:
///   `{"subject":...,"issuer":...,"audience":...,"expires_at":...,
///   "issued_at":...,"claims_json":"..."}` — `VerifiedIdentity`'s own
///   real field names, not OIDC-standard abbreviations, and
///   `claims_json` embedded as an escaped JSON *string* (not a nested
///   object) — both deliberate, so a route wrapper can decode this
///   straight into a real `VerifiedIdentity` with the exact same
///   generic struct-JSON decoder it already uses for any other
///   struct-typed argument (`identity.rs::identity_json`'s own doc
///   comment has the full reasoning). A
///   request carrying an `Authorization` header that fails to verify
///   (malformed, wrong issuer/audience, bad signature, expired) never
///   reaches a handler at all — this crate answers `401` itself before
///   dispatch, the same "an invalid bearer token is a hard failure, not
///   silently treated as anonymous" behavior the deleted interpreted
///   `serve.rs::resolve_identity` already established.
/// - `out_body_{ptr,len}`: the handler writes a heap-allocated (leaked,
///   same disclosed-not-hidden convention `nir_transact_decode_args`'s
///   own string output already uses) UTF-8 JSON response body here.
/// - `out_cookie_{ptr,len}`: `null`/zero-length for "no `Set-Cookie`
///   this response", otherwise a heap-allocated (leaked) **complete,
///   fully-attributed** cookie string (e.g.
///   `session=<id>; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=<n>`,
///   `kernel::identity::nir_session_cookie`'s own output) — `http::
///   write_response` writes whatever is handed here to the wire
///   verbatim and does **not** append `HttpOnly`/`Secure`/`SameSite`/
///   `Path` of its own (red team finding A5,
///   `scratch/red-team-report-main-d7fae42.md`: two layers each adding
///   attributes produced one `Set-Cookie` header with two disagreeing
///   `SameSite` values). A handler that hands back a bare
///   `name=value` here with no attributes will ship an insecure cookie
///   with no `HttpOnly`/`Secure`/`SameSite` at all — this crate no
///   longer fixes that up.
/// - Return: `0` = success (`out_body` is the real JSON result, HTTP
///   200), `1` = business-level error (`out_body` is still a real JSON
///   error payload, HTTP 200 — matching this project's existing
///   `Result(_, _)`-over-JSON convention elsewhere), `2` = unauthorized/
///   forbidden (this crate writes a bare 401/403, `out_body` ignored).
pub type RouteHandler = extern "C" fn(
    args_json_ptr: *const u8,
    args_json_len: i64,
    identity_json_ptr: *const u8,
    identity_json_len: i64,
    out_body_ptr: *mut *mut u8,
    out_body_len: *mut i64,
    out_cookie_ptr: *mut *mut u8,
    out_cookie_len: *mut i64,
) -> i32;

#[derive(Clone, Copy)]
pub struct Route {
    pub path: &'static str,
    pub handler: RouteHandler,
}

#[derive(Clone)]
pub struct ServeConfig {
    /// Header-read timeout — a client that connects and sends nothing
    /// must not park a thread (and a `domain::serve_http()` lease) forever.
    pub header_timeout: Duration,
    /// Body-read timeout, separate from the header timeout — a slow
    /// body (not a slow header) gets its own budget.
    pub body_timeout: Duration,
    /// How long an idle keep-alive connection may wait for its *next*
    /// request before this thread gives up and closes it.
    pub keepalive_idle_timeout: Duration,
    /// A hard cap on requests served over one keep-alive connection —
    /// bounds a single connection's worst-case lease-hold time even
    /// under a client that never goes idle.
    pub max_requests_per_connection: u32,
    /// CORS: the exact origin(s) this server reflects back on a
    /// credentialed response. Never `*` — see `cors_headers_for`'s own
    /// doc comment for why a wildcard here would reopen the CSRF hole
    /// the custom-header requirement on mutating routes closes.
    pub allowed_origins: Vec<String>,
    /// Paths rate-limited by `ratelimit` — e.g. `/auth/login`,
    /// `/api/_demo_login`. Every other path is unlimited at this layer
    /// (`domain::serve_http()`'s own admission ceiling is the general
    /// backstop).
    pub rate_limited_paths: Vec<&'static str>,
    pub rate_limit_max_per_window: u32,
    pub rate_limit_window: Duration,
    /// Direct-peer addresses trusted to supply a real `X-Forwarded-For`
    /// — empty by default (rate-limit on the direct peer only). Never
    /// trust `X-Forwarded-For` from an untrusted peer: that's trivially
    /// spoofable by any client.
    ///
    /// `ServeConfig::default()` seeds this from
    /// `NIRDOSHA_SERVE_TRUSTED_PROXIES` (a comma-separated list of plain
    /// IPs, e.g. `10.0.0.1,10.0.0.2`) if set — red-team report A16
    /// (`scratch/red-team-report-main-d7fae42.md`): without this, adding
    /// a trusted proxy (a new ingress, say) required a rebuild. **CIDR
    /// ranges are not supported here** (the report's own text
    /// acknowledges this is real, separate follow-up work, not a small
    /// fix) — every entry must be a single, literal `IpAddr`; an invalid
    /// entry in the env var is skipped with a loud `eprintln!`, not a
    /// silent drop, matching this crate's own degrade-to-default-not-fail
    /// posture for config parsing elsewhere.
    pub trusted_proxies: Vec<IpAddr>,
    /// A bearer token `/metrics` requires — `None` means `/metrics`
    /// answers unauthenticated, which is fine only when the listener
    /// itself is bound to localhost; `Some` requires
    /// `Authorization: Bearer <token>` to match exactly.
    pub metrics_token: Option<String>,
    /// The JWKS/issuer/audience trio(s) every bearer token on this
    /// server is checked against — always non-empty, since every server
    /// has one, there is no "identity checking is off" mode. One entry
    /// (the common case): every token is checked against it directly, no
    /// issuer dispatch needed. More than one (2026-09, ROADMAP.md A6's
    /// "Multi-IdP registry"): [`identity::validate_token`] peeks the
    /// token's own *unverified* `iss` claim first to pick the matching
    /// entry, then runs the exact same real verification against it —
    /// an unrecognized issuer is a real 401, never a silent fallback.
    /// [`AuthConfig::demo()`] (this struct's own `Default`, a single
    /// entry, when whoever starts the binary supplied no real provider
    /// via `NIRDOSHA_JWKS_FILE`/`NIRDOSHA_IDENTITY_PROVIDERS`) or one or
    /// more real IdPs' own trios — see [`auth_providers_from_env`] for
    /// the env-var/config-file mechanism, deliberately not the
    /// interpreter-era DB-admin-editable pattern (that scaffolding was
    /// deleted along with the interpreter and isn't being rebuilt here;
    /// a real, disclosed narrowing of A6's original spec).
    pub auth: Vec<AuthConfig>,
    /// `true` exactly when `auth` is a single [`AuthConfig::demo()`]
    /// entry — gates whether `/api/_demo_login` exists at all (real
    /// production identity has no self-service login; a caller gets a
    /// token from the org's actual IdP). Tracked as its own flag rather
    /// than inferred from `auth`'s contents, so the "which mode"
    /// decision stays a single, explicit fact set once at startup, not a
    /// heuristic re-derived from a JWKS/issuer string shape.
    pub demo_mode: bool,
    /// `GET /`'s response body — the program's own `emit-ui`-derived
    /// UI, generated once at *compile* time (`ui_gen::generate`,
    /// `codegen.rs`'s Stage 3) and baked into the binary as a plain
    /// byte string, never regenerated at runtime — unlike the deleted
    /// interpreted `serve.rs`, a compiled binary has no `Program` AST
    /// left to call `ui_gen::generate` against once it's running.
    /// Empty means no UI to serve (`GET /` 404s) — `nirdosha build
    /// --serve` always sets this; only a hand-rolled `ServeConfig` (this
    /// crate's own tests) leaves it empty.
    pub ui_html: Vec<u8>,
}

impl Default for ServeConfig {
    fn default() -> Self {
        let (auth, demo_mode) = auth_providers_from_env();
        ServeConfig {
            header_timeout: Duration::from_secs(10),
            body_timeout: Duration::from_secs(30),
            keepalive_idle_timeout: Duration::from_secs(60),
            max_requests_per_connection: 1000,
            allowed_origins: Vec::new(),
            rate_limited_paths: Vec::new(),
            rate_limit_max_per_window: 20,
            rate_limit_window: Duration::from_secs(60),
            trusted_proxies: trusted_proxies_from_env(),
            metrics_token: None,
            auth,
            demo_mode,
            ui_html: Vec::new(),
        }
    }
}

/// See [`ServeConfig::trusted_proxies`]'s own doc comment for the format
/// and rationale (A16). Reads `NIRDOSHA_SERVE_TRUSTED_PROXIES` once, at
/// `ServeConfig::default()` construction time -- not cached across calls
/// the way `kernel`'s own domain ceilings are (A24's own finding is
/// exactly why this crate doesn't repeat that mistake for a value an
/// operator might reasonably expect to change between deployments of a
/// freshly-constructed config, e.g. in a test).
fn trusted_proxies_from_env() -> Vec<IpAddr> {
    let Ok(raw) = std::env::var("NIRDOSHA_SERVE_TRUSTED_PROXIES") else { return Vec::new() };
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(|s| match s.parse::<IpAddr>() {
            Ok(ip) => Some(ip),
            Err(_) => {
                eprintln!("nirdosha compiled-serve: NIRDOSHA_SERVE_TRUSTED_PROXIES entry {s:?} is not a valid IP address -- skipped");
                None
            }
        })
        .collect()
}

/// `ServeConfig::auth`/`demo_mode`'s real source (ROADMAP.md A6, "Multi-
/// IdP registry") — same "read once at process start, degrade to the
/// safe default on absence/parse failure with a loud `eprintln!`, never
/// fail the whole process" posture [`trusted_proxies_from_env`] already
/// established for `NIRDOSHA_SERVE_TRUSTED_PROXIES`. A **runtime** env
/// read, not a `nirdosha build` flag — no `codegen.rs`/LLVM IR change
/// needed at all, and it means the exact same built binary can be
/// redeployed against a different IdP (or a different provider list)
/// without a rebuild.
///
/// Three cases, checked in order:
/// - `NIRDOSHA_IDENTITY_PROVIDERS` set (a path to a JSON file: `[{
///   "jwks_file": "...", "issuer": "...", "audience": "..."}, ...]`) —
///   one or more real providers, [`identity::validate_token`] dispatches
///   between them by the token's own issuer claim. An empty list or a
///   read/parse failure degrades to demo mode, same as every other case
///   here — a real, deliberately non-fatal deployment mistake, not a
///   crash.
/// - `NIRDOSHA_JWKS_FILE`/`NIRDOSHA_ISSUER`/`NIRDOSHA_AUDIENCE` all set —
///   exactly one real provider, no issuer dispatch needed.
/// - Neither — [`AuthConfig::demo()`], this crate's long-standing
///   default; `demo_mode: true`.
fn auth_providers_from_env() -> (Vec<AuthConfig>, bool) {
    if let Ok(path) = std::env::var("NIRDOSHA_IDENTITY_PROVIDERS") {
        match load_identity_providers_file(&path) {
            Ok(providers) if !providers.is_empty() => return (providers, false),
            Ok(_) => {
                eprintln!("nirdosha compiled-serve: NIRDOSHA_IDENTITY_PROVIDERS at {path:?} is an empty list -- falling back to demo mode")
            }
            Err(e) => {
                eprintln!("nirdosha compiled-serve: NIRDOSHA_IDENTITY_PROVIDERS at {path:?} could not be read: {e} -- falling back to demo mode")
            }
        }
    } else if let Some(auth) = single_provider_from_env() {
        return (vec![auth], false);
    }
    (vec![AuthConfig::demo()], true)
}

/// The single-provider case: all three of `NIRDOSHA_JWKS_FILE`/
/// `NIRDOSHA_ISSUER`/`NIRDOSHA_AUDIENCE` must be set together, or this
/// falls back like any other absent/malformed config (a partially-set
/// trio almost certainly means a deployment mistake, not "two of three
/// intentionally unset").
fn single_provider_from_env() -> Option<AuthConfig> {
    let jwks_file = std::env::var("NIRDOSHA_JWKS_FILE").ok()?;
    let issuer = std::env::var("NIRDOSHA_ISSUER").ok()?;
    let audience = std::env::var("NIRDOSHA_AUDIENCE").ok()?;
    match std::fs::read_to_string(&jwks_file) {
        Ok(jwks_json) => Some(AuthConfig { jwks_json, issuer, audience }),
        Err(e) => {
            eprintln!("nirdosha compiled-serve: NIRDOSHA_JWKS_FILE at {jwks_file:?} could not be read: {e} -- falling back to demo mode");
            None
        }
    }
}

/// `NIRDOSHA_IDENTITY_PROVIDERS`'s own file format — a plain
/// `serde_json::Value` walk, not a `#[derive(Deserialize)]` struct
/// (this crate doesn't otherwise depend on `serde`'s derive machinery,
/// only `serde_json`, the same "walk `Value` directly" style
/// `identity::mock_issue_token` already uses for its own JSON building).
fn load_identity_providers_file(path: &str) -> Result<Vec<AuthConfig>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let entries = value.as_array().ok_or("expected a JSON array of provider objects")?;
    entries
        .iter()
        .map(|entry| {
            let jwks_file = entry.get("jwks_file").and_then(|v| v.as_str()).ok_or("each provider needs a string `jwks_file`")?;
            let issuer = entry.get("issuer").and_then(|v| v.as_str()).ok_or("each provider needs a string `issuer`")?;
            let audience = entry.get("audience").and_then(|v| v.as_str()).ok_or("each provider needs a string `audience`")?;
            let jwks_json = std::fs::read_to_string(jwks_file)
                .map_err(|e| format!("provider issuer {issuer:?}: jwks_file {jwks_file:?} could not be read: {e}"))?;
            Ok(AuthConfig { jwks_json, issuer: issuer.to_string(), audience: audience.to_string() })
        })
        .collect::<Result<Vec<AuthConfig>, String>>()
}

/// A bound-but-not-yet-accepting listener — the bind-before-replay
/// ordering `docs/adr/0009-transact-durability-and-replay.md` already
/// specifies for the eventual generated `main`: bind the socket first
/// (so `/healthz`-style liveness is real immediately, and the OS's own
/// listen backlog holds incoming connections rather than refusing them
/// outright), run `nir_transact_replay_all()` next, *then* call
/// [`Listener::run`] — never the other way around. Splitting `bind`
/// from `run` into two real steps is what makes this ordering
/// structural rather than a convention callers have to remember.
pub struct Listener {
    tcp: TcpListener,
}

pub fn bind(addr: &str) -> std::io::Result<Listener> {
    Ok(Listener { tcp: TcpListener::bind(addr)? })
}

impl Listener {
    /// The real bound address — needed by any caller that binds to
    /// port `0` (an OS-assigned free port, this crate's own test suite
    /// included) and needs to know which port that actually was.
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.tcp.local_addr()
    }
}

/// Shared, per-process readiness flag — `false` until the caller's own
/// replay step (or whatever else must finish before serving real
/// traffic) completes. `/healthz` never depends on this (a bound
/// listener is alive, full stop); `/readyz` does.
#[derive(Clone, Default)]
pub struct Readiness(Arc<std::sync::atomic::AtomicBool>);

impl Readiness {
    pub fn new() -> Self {
        Readiness(Arc::new(std::sync::atomic::AtomicBool::new(false)))
    }
    pub fn mark_ready(&self) {
        self.0.store(true, Ordering::Release);
    }
    fn is_ready(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

impl Listener {
    /// Runs the accept loop forever (or until the process exits).
    ///
    /// **`kernel::acquire(domain::serve_http())` is checked here, in the
    /// accept loop itself, before any thread is spawned** — red-team
    /// report A11/A23 (`scratch/red-team-report-main-d7fae42.md`): the
    /// old design span thread per connection unconditionally and only
    /// checked admission *inside* that thread, so a connection flood
    /// exhausted the OS thread ceiling long before `domain::serve_http()`'s
    /// own admission ceiling was ever consulted — the kernel's ceiling
    /// was real but never actually the bottleneck it claimed to be. This
    /// is safe to do here specifically because `kernel::acquire` is
    /// non-blocking, lock-free CAS (`kernel::mod.rs`'s own doc comment) —
    /// checking it costs nothing this loop couldn't already afford.
    ///
    /// **A denied connection is dropped immediately, not spawned into a
    /// thread to write a `503`** — a real, disclosed behavior change
    /// from the old design (which wrote a graceful `503 Service
    /// Unavailable` body before closing). Spawning *any* thread per
    /// denied connection — even a minimal one that only writes a
    /// response — reintroduces the exact unbounded-thread-creation
    /// problem this fix exists to close, since under a genuine
    /// saturation attack the volume of *denied* connections is the
    /// dominant cost, not the admitted ones. Writing the `503` directly
    /// in this loop, synchronously, is not a safe alternative either: a
    /// slow or malicious peer that accepts the TCP handshake and then
    /// never reads its receive buffer would block this loop's `write`
    /// call indefinitely (a classic slow-loris vector), stalling accept
    /// for every other connection, admitted or not. Dropping the
    /// `TcpStream` outright (its `Drop` impl closes the fd without
    /// waiting for any unsent data — `SO_LINGER` is not set, so this
    /// does not block) is the one response that's both cheap and safe
    /// under adversarial input: the client sees a connection reset
    /// instead of a graceful error body once truly at capacity, which
    /// matches how a production load balancer or reverse proxy already
    /// behaves at overload (fail fast and cheap, don't spend a thread or
    /// a blocking write explaining why).
    pub fn run(self, routes: &'static [Route], config: ServeConfig, readiness: Readiness) -> std::io::Result<()> {
        let config = Arc::new(config);
        let limiter = Arc::new(ratelimit::RateLimiter::new());
        for stream in self.tcp.incoming() {
            let stream = match stream {
                Ok(s) => s,
                Err(_) => continue,
            };
            if !kernel::acquire(domain::serve_http()) {
                // Dropped without a response -- see this fn's own doc
                // comment for why neither a synchronous write nor a
                // spawned thread is safe here.
                continue;
            }
            let config = Arc::clone(&config);
            let limiter = Arc::clone(&limiter);
            let readiness = readiness.clone();
            std::thread::spawn(move || handle_connection(stream, routes, &config, &limiter, &readiness));
        }
        Ok(())
    }
}

/// One exposed route, as `codegen.rs`'s generated `main` (Stage 3 of
/// reviving compiled `serve`) actually has it to give: raw pointer
/// parts, not a real `&'static str` — LLVM IR has no notion of a Rust
/// lifetime, only a global string constant's address. [`nir_compiled_serve_run`]
/// converts every entry to a real, leaked `'static` [`Route`] once, at
/// startup, before handing the whole table to [`Listener::run`].
#[repr(C)]
pub struct CRoute {
    pub path_ptr: *const u8,
    pub path_len: i64,
    pub handler: RouteHandler,
}

/// The real OS-level entry point a `--serve` binary's generated `main`
/// calls instead of the ordinary `nir_main()` — this crate's own
/// C-ABI bridge, since LLVM-emitted IR can only ever call a plain
/// `extern "C" fn`, never a generic Rust API like [`Listener::run`]
/// directly. Never returns under normal operation ([`Listener::run`]'s
/// own accept loop runs forever); a bind failure returns `1` instead
/// of panicking, so the generated `main` can report it and exit
/// cleanly rather than aborting.
///
/// Demo mode only, deliberately, for this first cut — `ServeConfig::default()`'s
/// own `AuthConfig::demo()` plus `demo_mode: true`. Real production
/// identity (a `--jwks-file`/`--issuer`/`--audience` trio threaded
/// through `nirdosha build --serve` itself) is real, disclosed
/// follow-up work: hosting a `nirdosha hi`-generated demo app (this
/// revival's own actual goal, Stage 5) needs demo mode, not a real IdP.
///
/// # Safety
/// `routes_ptr` must point to `routes_count` valid [`CRoute`]s, each
/// with a `path_ptr`/`path_len` naming a valid UTF-8 byte range that
/// stays readable for the length of this call (codegen emits these as
/// `private unnamed_addr constant` globals, which live for the whole
/// process). `ui_html_ptr`/`ui_html_len` (zero/null for "no UI") must
/// meet the same contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_compiled_serve_run(routes_ptr: *const CRoute, routes_count: i64, ui_html_ptr: *const u8, ui_html_len: i64, port: i64) -> i32 {
    let c_routes = unsafe { std::slice::from_raw_parts(routes_ptr, routes_count as usize) };
    let mut routes = Vec::with_capacity(c_routes.len());
    for r in c_routes {
        let path_bytes = unsafe { std::slice::from_raw_parts(r.path_ptr, r.path_len as usize) }.to_vec();
        let path: &'static str = match String::from_utf8(path_bytes) {
            Ok(s) => Box::leak(s.into_boxed_str()),
            Err(_) => {
                eprintln!("nirdosha serve: a route path was not valid UTF-8 -- refusing to start");
                return 1;
            }
        };
        routes.push(Route { path, handler: r.handler });
    }
    let routes: &'static [Route] = Box::leak(routes.into_boxed_slice());

    let ui_html = if ui_html_ptr.is_null() || ui_html_len <= 0 { Vec::new() } else { unsafe { std::slice::from_raw_parts(ui_html_ptr, ui_html_len as usize) }.to_vec() };

    let addr = format!("0.0.0.0:{port}");
    let listener = match bind(&addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("nirdosha serve: failed to bind {addr}: {e}");
            return 1;
        }
    };
    eprintln!("nirdosha serve: listening on http://{addr} (demo mode)");
    let readiness = Readiness::new();
    // `nir_transact_replay_all` already ran in the generated `main`
    // before this call (`docs/adr/0009`'s own bind-before-replay
    // ordering — bind happens above, replay happened earlier still, in
    // the caller) — this is "ready" the instant the listener is up.
    readiness.mark_ready();
    let config = ServeConfig { ui_html, ..ServeConfig::default() };
    match listener.run(routes, config, readiness) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("nirdosha serve: {e}");
            1
        }
    }
}

/// Releases a `domain::serve_http()` lease exactly once, on drop.
///
/// **Only covers a panic in this crate's own Rust logic** (`dispatch`'s
/// own code — CORS, rate limiting, JSON marshaling — everything except
/// the actual [`RouteHandler`] call), caught by the `catch_unwind`
/// around `dispatch` in [`handle_connection`], which lets this `Drop`
/// run normally on the way out. **A fault *inside* a route handler
/// itself is a different, harder case, verified directly (not assumed)
/// to behave differently**: `RouteHandler` is a plain `extern "C" fn`,
/// and calling through one that panics does not unwind at all — Rust
/// converts a panic crossing a non-`"C-unwind"` `extern "C"` boundary
/// into an immediate process `abort()` (confirmed by triggering one:
/// `thread caused non-unwinding panic. aborting.`), *before* the
/// `catch_unwind` around `dispatch` (which sits on the *safe* Rust side
/// of that boundary) ever gets a chance to intercept anything. This
/// `Drop` never runs in that case — but neither does anything else,
/// since the whole process is gone. This is not a gap to close: it's
/// the same "a compiled trap is an unconditional `abort()`" philosophy
/// `codegen.rs::emit_transact`'s own doc comment already establishes
/// for every other compiled Nirdosha fault — a real route handler is
/// eventually compiled LLVM code (`codegen.rs`'s own follow-up work,
/// this crate's module doc), which already can't unwind either way.
struct ServeHttpLease;

impl Drop for ServeHttpLease {
    fn drop(&mut self) {
        kernel::release(domain::serve_http());
    }
}

fn handle_connection(mut stream: TcpStream, routes: &[Route], config: &ServeConfig, limiter: &ratelimit::RateLimiter, readiness: &Readiness) {
    // `Listener::run` already called `kernel::acquire(domain::serve_http())`
    // for this connection before spawning the thread that's now running
    // this function (A11/A23 fix, see `run`'s own doc comment) — this
    // function's job is only to hold that lease for the connection's
    // whole lifetime and release it exactly once, on drop.
    let _lease = ServeHttpLease;
    let peer = stream.peer_addr().ok();

    for request_index in 0..config.max_requests_per_connection {
        let timeout = if request_index == 0 { config.header_timeout } else { config.keepalive_idle_timeout };
        let _ = stream.set_read_timeout(Some(timeout));
        let req = match http::read_request(&mut stream, config.body_timeout) {
            Ok(Some(r)) => r,
            Ok(None) => break, // clean connection close / idle timeout -- not an error
            Err(http::ReadError::TooLarge) => {
                let _ = http::write_response(&mut stream, 413, "text/plain", b"413 Payload Too Large", &[], None);
                break;
            }
            Err(_) => break, // malformed/timed-out request -- close the connection, same as any other unrecoverable transport error
        };

        let keep_alive = req.wants_keep_alive() && request_index + 1 < config.max_requests_per_connection;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| dispatch(&req, routes, config, limiter, peer, readiness)));
        let response = result.unwrap_or_else(|_| http::Response::error(500, "internal error"));
        let done = http::write_response(&mut stream, response.status, response.content_type, &response.body, &response.headers, response.cookie.as_deref());
        if done.is_err() || !keep_alive {
            break;
        }
    }
}

fn dispatch(req: &http::Request, routes: &[Route], config: &ServeConfig, limiter: &ratelimit::RateLimiter, peer: Option<SocketAddr>, readiness: &Readiness) -> http::Response {
    if req.method == "OPTIONS" {
        return cors_preflight_response(req, config);
    }
    if req.path == "/" && req.method == "GET" {
        return if config.ui_html.is_empty() { http::Response::error(404, "not found") } else { http::Response::ok_html(200, &config.ui_html) };
    }
    if req.path == "/healthz" {
        // Deliberately independent of `readiness` -- a bound, accepting
        // listener is "alive" regardless of whether e.g. crash replay
        // has finished yet (`docs/adr/0009`'s own bind-before-replay
        // ordering: this is exactly what lets a liveness probe keep
        // passing, uninterrupted, through a real replay backlog after a
        // restart, instead of k8s crash-looping the pod).
        return http::Response::ok_text(200, "ok");
    }
    if req.path == "/readyz" {
        return if readiness.is_ready() { http::Response::ok_text(200, "ready") } else { http::Response::error(503, "not ready") };
    }
    if req.path == "/metrics" {
        return metrics_response(req, config);
    }
    if req.path == "/api/_demo_login" {
        return with_cors(demo_login_response(req, config), req, config);
    }
    if req.path == "/api/_whoami" {
        return with_cors(whoami_response(req, config), req, config);
    }
    if config.rate_limited_paths.iter().any(|p| *p == req.path) {
        let key = client_ip_for_rate_limit(req, peer, config);
        if let Some(key) = key {
            if !limiter.check(key, config.rate_limit_max_per_window, config.rate_limit_window) {
                return http::Response::error(429, "rate limited");
            }
        }
    }
    let identity_json = match resolve_identity(req, config) {
        Ok(json) => json,
        Err(resp) => return with_cors(resp, req, config),
    };
    let Some(route) = routes.iter().find(|r| r.path == req.path) else {
        return with_cors(http::Response::error(404, "not found"), req, config);
    };
    let (status, body, cookie) = call_route(route.handler, &req.body, identity_json.as_deref());
    let mut resp = http::Response { status, content_type: "application/json", body, headers: Vec::new(), cookie };
    resp = with_cors(resp, req, config);
    resp
}

/// `Authorization` header → this request's `identity_json` (`Ok(None)`
/// for "no header at all," `Ok(Some(json))` for a real, verified
/// identity) — or the actual `401` response to send back immediately
/// when a header is present but doesn't verify. Checked once, ahead of
/// route lookup, so an invalid token 401s the same way regardless of
/// whether the path even exists — mirrors the deleted interpreted
/// `serve.rs::resolve_identity`'s own "present but invalid is always a
/// hard failure, never silently anonymous" behavior.
fn resolve_identity(req: &http::Request, config: &ServeConfig) -> Result<Option<String>, http::Response> {
    let Some(auth_header) = req.header("authorization") else { return Ok(None) };
    let Some(token) = auth_header.strip_prefix("Bearer ").or_else(|| auth_header.strip_prefix("bearer ")) else {
        return Err(http::Response::error(401, "Authorization header must be `Bearer <token>`"));
    };
    match identity::validate_token(token, &config.auth[..]) {
        Ok(claims) => Ok(Some(identity::identity_json(&claims))),
        Err(e) => Err(http::Response::error(401, &format!("invalid token: {e}"))),
    }
}

/// `POST /api/_demo_login` — demo mode only (`config.demo_mode`); `404`
/// on a real-identity server, the same way the deleted interpreted
/// `serve.rs` never registered this route at all outside demo mode.
/// Body: `{"subject": "...", "roles": [...], "claims": {...}}`, all
/// optional (default subject `"demo"`, empty roles/claims) — recovered
/// verbatim from `05a747c~1:crates/compiler/src/serve.rs`'s own
/// `handle_demo_login` as this port's ground truth.
fn demo_login_response(req: &http::Request, config: &ServeConfig) -> http::Response {
    if !config.demo_mode {
        return http::Response::error(404, "not found");
    }
    let body: serde_json::Value = if req.body.is_empty() {
        serde_json::Value::Object(Default::default())
    } else {
        match serde_json::from_slice(&req.body) {
            Ok(v) => v,
            Err(e) => return http::Response::error(400, &format!("invalid JSON body: {e}")),
        }
    };
    let subject = body.get("subject").and_then(serde_json::Value::as_str).unwrap_or("demo").to_string();
    let roles: Vec<String> = body.get("roles").and_then(serde_json::Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
    let claims: Vec<(String, String)> = body
        .get("claims")
        .and_then(serde_json::Value::as_object)
        .map(|m| m.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect())
        .unwrap_or_default();
    // `demo_mode` (already checked above) is only ever `true` alongside
    // a single-entry `auth` (`auth_providers_from_env`'s own contract) --
    // `.first()` over an index, so a hand-built `ServeConfig` that
    // somehow violates that (this crate's own tests never do) degrades
    // to a clean 500, not a panic.
    let Some(demo_auth) = config.auth.first() else {
        return http::Response::error(500, "demo mode is enabled but no identity provider is configured");
    };
    match identity::mock_issue_token(&subject, demo_auth, &roles, &claims) {
        Ok(token) => http::Response::ok_text(200, &serde_json::json!({"token": token}).to_string()),
        Err(e) => http::Response::error(500, &format!("failed to mint demo token: {e}")),
    }
}

/// `GET /api/_whoami` — always present, every mode: confirms a bearer
/// token is still valid against *this* running process, so a generated
/// UI can catch a `localStorage`-cached identity from a previous,
/// since-restarted demo-mode process whose ephemeral signing key no
/// longer matches (`ui_gen.rs`'s own doc comment on why this route
/// exists).
fn whoami_response(req: &http::Request, config: &ServeConfig) -> http::Response {
    match resolve_identity(req, config) {
        Ok(Some(json)) => http::Response { status: 200, content_type: "application/json", body: json.into_bytes(), headers: Vec::new(), cookie: None },
        Ok(None) => http::Response::error(401, "no Authorization header"),
        Err(resp) => resp,
    }
}

/// The one `unsafe` boundary in this crate — calling a real function
/// pointer with the fixed ABI [`RouteHandler`] documents, and reading
/// back its leaked (`Box::leak`-style, same convention `runtime-kernels`
/// already uses for cross-boundary string output) output buffers.
fn call_route(handler: RouteHandler, args_json: &[u8], identity_json: Option<&str>) -> (u16, Vec<u8>, Option<String>) {
    let (id_ptr, id_len) = match identity_json {
        Some(s) => (s.as_ptr(), s.len() as i64),
        None => (std::ptr::null(), 0),
    };
    let mut out_body_ptr: *mut u8 = std::ptr::null_mut();
    let mut out_body_len: i64 = 0;
    let mut out_cookie_ptr: *mut u8 = std::ptr::null_mut();
    let mut out_cookie_len: i64 = 0;
    // A plain `extern "C" fn(...)` value (unlike a raw function
    // pointer typed `unsafe extern "C" fn(...)`) is safe to *call* —
    // the `unsafe` this function actually needs is entirely in
    // dereferencing the raw output pointers below, which the handler
    // itself is trusted (by the ABI contract `RouteHandler`'s own doc
    // comment states) to have written validly.
    let code = handler(
        args_json.as_ptr(),
        args_json.len() as i64,
        id_ptr,
        id_len,
        &mut out_body_ptr,
        &mut out_body_len,
        &mut out_cookie_ptr,
        &mut out_cookie_len,
    );
    let body = if out_body_ptr.is_null() || out_body_len <= 0 {
        b"{}".to_vec()
    } else {
        unsafe { std::slice::from_raw_parts(out_body_ptr, out_body_len as usize) }.to_vec()
    };
    let cookie = if out_cookie_ptr.is_null() || out_cookie_len <= 0 {
        None
    } else {
        Some(String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(out_cookie_ptr, out_cookie_len as usize) }).into_owned())
    };
    let status = match code {
        0 => 200,
        1 => 200, // business-level Err -- still a real JSON payload, same Result(_,_)-over-JSON convention this project already uses
        2 => 401,
        _ => 500,
    };
    (status, body, cookie)
}

/// **Never** a wildcard on a credentialed response — reflects exactly
/// one configured origin, or none at all. A `*` here alongside
/// `Access-Control-Allow-Credentials: true` would let any origin's page
/// make a cookie-bearing request and read the response, exactly the
/// CSRF hole a custom-header requirement on mutating routes (real,
/// separate follow-up on the client-bundle side — `ui_gen.rs`'s own
/// fetch wrapper contract, not this crate) exists to close.
/// `(scheme, host, port)`, with an *implicit* default port
/// (`https` → 443, `http` → 80, anything else → `None`, treated as
/// "always distinct from any explicit port") filled in when the origin
/// string carries none — red-team report A21
/// (`scratch/red-team-report-main-d7fae42.md`): a browser normalizes
/// `https://app.example.com:443` and `https://app.example.com` to the
/// *same* origin for CORS purposes, but a bare `==` string comparison
/// (this function's old shape) treated them as different, so a
/// configured `allowed_origins` entry without an explicit port could
/// silently fail to match a request that carried one (or vice versa) —
/// a CORS failure for what both sides consider the same origin.
fn parse_origin(origin: &str) -> Option<(&str, &str, u16)> {
    let (scheme, rest) = origin.split_once("://")?;
    let default_port = match scheme {
        "https" => 443,
        "http" => 80,
        _ => 0, // no sensible default; an explicit port is required to match at all
    };
    match rest.rsplit_once(':') {
        Some((host, port_str)) => match port_str.parse::<u16>() {
            Ok(port) => Some((scheme, host, port)),
            Err(_) => Some((scheme, rest, default_port)), // not a real port (e.g. an IPv6 host's own ':') -- treat the whole thing as host
        },
        None => Some((scheme, rest, default_port)),
    }
}

fn origins_match(a: &str, b: &str) -> bool {
    match (parse_origin(a), parse_origin(b)) {
        (Some(pa), Some(pb)) => pa == pb,
        _ => a == b, // either side failed to parse -- fall back to the old literal comparison rather than silently matching nothing
    }
}

fn cors_headers_for(req: &http::Request, config: &ServeConfig) -> Vec<(String, String)> {
    let Some(origin) = req.header("origin") else { return Vec::new() };
    if !config.allowed_origins.iter().any(|o| origins_match(o, origin)) {
        return Vec::new();
    }
    vec![
        ("Access-Control-Allow-Origin".to_string(), origin.to_string()),
        ("Access-Control-Allow-Credentials".to_string(), "true".to_string()),
        ("Vary".to_string(), "Origin".to_string()),
    ]
}

fn with_cors(mut resp: http::Response, req: &http::Request, config: &ServeConfig) -> http::Response {
    resp.headers.extend(cors_headers_for(req, config));
    resp
}

fn cors_preflight_response(req: &http::Request, config: &ServeConfig) -> http::Response {
    let mut headers = cors_headers_for(req, config);
    if !headers.is_empty() {
        headers.push(("Access-Control-Allow-Methods".to_string(), "GET, POST, OPTIONS".to_string()));
        headers.push(("Access-Control-Allow-Headers".to_string(), "Content-Type, Authorization, X-Requested-With".to_string()));
    }
    http::Response { status: 204, content_type: "text/plain", body: Vec::new(), headers, cookie: None }
}

fn metrics_response(req: &http::Request, config: &ServeConfig) -> http::Response {
    match &config.metrics_token {
        Some(token) => {
            // Red-team report A20: plain `==` on the full `Bearer <token>`
            // string is a real timing oracle against `/metrics` for a
            // short or guessable token -- `constant_time_eq` is the same
            // function `kernel::identity`'s own API-key validation
            // already uses for exactly this reason.
            let expected = format!("Bearer {token}");
            let ok = req.header("authorization").map(|h| constant_time_eq(h.as_bytes(), expected.as_bytes())).unwrap_or(false);
            if !ok {
                return http::Response::error(401, "unauthorized");
            }
        }
        None => {}
    }
    http::Response::ok_text(200, &kernel::dump_report())
}

fn client_ip_for_rate_limit(req: &http::Request, peer: Option<SocketAddr>, config: &ServeConfig) -> Option<IpAddr> {
    let peer_ip = peer.map(|p| p.ip());
    if let (Some(peer_ip), Some(xff)) = (peer_ip, req.header("x-forwarded-for")) {
        if config.trusted_proxies.contains(&peer_ip) {
            if let Some(real) = xff.split(',').next().map(|s| s.trim()) {
                if let Ok(ip) = real.parse::<IpAddr>() {
                    return Some(ip);
                }
            }
        }
    }
    peer_ip
}

#[cfg(test)]
mod tests;
