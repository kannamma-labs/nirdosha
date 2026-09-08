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
//! **Not yet wired to `codegen.rs`.** This first cut proves the real
//! HTTP engine (admission, timeouts, dispatch, sessions, CORS, rate
//! limiting) against hand-written `extern "C"` test routes — the
//! matching `codegen.rs` work to emit real per-route wrapper functions
//! from a compiled program's own exposure set (RFC 0010) and a
//! `nirdosha build --serve` CLI flag to link this crate in is real,
//! separate follow-up work, the same "build the mechanism, prove it,
//! then wire it to codegen" order this session's own `transact`
//! durability work (`docs/adr/0009`) already followed.

use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use nirdosha_runtime_kernels::kernel::{self, domain};

mod http;
mod ratelimit;

pub use http::MAX_BODY_BYTES;

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
/// - `unverified_bearer_token_json_{ptr,len}`: `""`/zero-length when
///   the request carried no bearer token, otherwise `{"token":"<raw>"}`
///   — the raw token text, **not verified in any way** by this crate
///   (no signature check, no issuer/audience check, no expiry check).
///   This is NOT the "already resolved and verified upstream" identity
///   object an earlier draft of this doc comment claimed — that claim
///   was false; see `Request::unverified_bearer_token_json`'s own doc
///   comment (`http.rs`) for why. A handler that needs a real,
///   verified principal must call `oidc_validate_token`/
///   `nir_oidc_validate_token` (or, for a session cookie instead of a
///   bearer token, `verify_session`) itself — this crate hands over the
///   raw material, never a verified result.
/// - `out_body_{ptr,len}`: the handler writes a heap-allocated (leaked,
///   same disclosed-not-hidden convention `nir_transact_decode_args`'s
///   own string output already uses) UTF-8 JSON response body here.
/// - `out_cookie_{ptr,len}`: `null`/zero-length for "no `Set-Cookie`
///   this response", otherwise a heap-allocated (leaked) cookie
///   *value* string (e.g. `nirdosha_session=<id>`) — this crate attaches
///   the real `HttpOnly; Secure; SameSite=Lax; Path=/` attributes and
///   writes the header; a handler never builds the header line itself.
/// - Return: `0` = success (`out_body` is the real JSON result, HTTP
///   200), `1` = business-level error (`out_body` is still a real JSON
///   error payload, HTTP 200 — matching this project's existing
///   `Result(_, _)`-over-JSON convention elsewhere), `2` = unauthorized/
///   forbidden (this crate writes a bare 401/403, `out_body` ignored).
pub type RouteHandler = extern "C" fn(
    args_json_ptr: *const u8,
    args_json_len: i64,
    unverified_bearer_token_json_ptr: *const u8,
    unverified_bearer_token_json_len: i64,
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
    pub trusted_proxies: Vec<IpAddr>,
    /// A bearer token `/metrics` requires — `None` means `/metrics`
    /// answers unauthenticated, which is fine only when the listener
    /// itself is bound to localhost; `Some` requires
    /// `Authorization: Bearer <token>` to match exactly.
    pub metrics_token: Option<String>,
}

impl Default for ServeConfig {
    fn default() -> Self {
        ServeConfig {
            header_timeout: Duration::from_secs(10),
            body_timeout: Duration::from_secs(30),
            keepalive_idle_timeout: Duration::from_secs(60),
            max_requests_per_connection: 1000,
            allowed_origins: Vec::new(),
            rate_limited_paths: Vec::new(),
            rate_limit_max_per_window: 20,
            rate_limit_window: Duration::from_secs(60),
            trusted_proxies: Vec::new(),
            metrics_token: None,
        }
    }
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
    /// Runs the accept loop forever (or until the process exits) —
    /// spawns a thread per accepted connection, unconditionally; the
    /// **thread**, not this accept loop, checks `domain::serve_http()`
    /// admission and fails fast with a real `503` if denied, so a
    /// saturated ceiling never makes the accept loop itself stall or
    /// queue (`rfcs/0010`'s own "admission failure is a fast, visible
    /// 503, not a silent stall" design).
    pub fn run(self, routes: &'static [Route], config: ServeConfig, readiness: Readiness) -> std::io::Result<()> {
        let config = Arc::new(config);
        let limiter = Arc::new(ratelimit::RateLimiter::new());
        for stream in self.tcp.incoming() {
            let stream = match stream {
                Ok(s) => s,
                Err(_) => continue,
            };
            let config = Arc::clone(&config);
            let limiter = Arc::clone(&limiter);
            let readiness = readiness.clone();
            std::thread::spawn(move || handle_connection(stream, routes, &config, &limiter, &readiness));
        }
        Ok(())
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
    if !kernel::acquire(domain::serve_http()) {
        let _ = http::write_response(&mut stream, 503, "text/plain", b"503 Service Unavailable -- server at capacity", &[], None);
        return;
    }
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
        let done = http::write_response(&mut stream, response.status, "application/json", &response.body, &response.headers, response.cookie.as_deref());
        if done.is_err() || !keep_alive {
            break;
        }
    }
}

fn dispatch(req: &http::Request, routes: &[Route], config: &ServeConfig, limiter: &ratelimit::RateLimiter, peer: Option<SocketAddr>, readiness: &Readiness) -> http::Response {
    if req.method == "OPTIONS" {
        return cors_preflight_response(req, config);
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
    if config.rate_limited_paths.iter().any(|p| *p == req.path) {
        let key = client_ip_for_rate_limit(req, peer, config);
        if let Some(key) = key {
            if !limiter.check(key, config.rate_limit_max_per_window, config.rate_limit_window) {
                return http::Response::error(429, "rate limited");
            }
        }
    }
    let Some(route) = routes.iter().find(|r| r.path == req.path) else {
        return with_cors(http::Response::error(404, "not found"), req, config);
    };
    let unverified_bearer_token_json = req.unverified_bearer_token_json();
    let (status, body, cookie) = call_route(route.handler, &req.body, unverified_bearer_token_json.as_deref());
    let mut resp = http::Response { status, body, headers: Vec::new(), cookie };
    resp = with_cors(resp, req, config);
    resp
}

/// The one `unsafe` boundary in this crate — calling a real function
/// pointer with the fixed ABI [`RouteHandler`] documents, and reading
/// back its leaked (`Box::leak`-style, same convention `runtime-kernels`
/// already uses for cross-boundary string output) output buffers.
fn call_route(handler: RouteHandler, args_json: &[u8], unverified_bearer_token_json: Option<&str>) -> (u16, Vec<u8>, Option<String>) {
    let (id_ptr, id_len) = match unverified_bearer_token_json {
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
fn cors_headers_for(req: &http::Request, config: &ServeConfig) -> Vec<(String, String)> {
    let Some(origin) = req.header("origin") else { return Vec::new() };
    if !config.allowed_origins.iter().any(|o| o == origin) {
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
    http::Response { status: 204, body: Vec::new(), headers, cookie: None }
}

fn metrics_response(req: &http::Request, config: &ServeConfig) -> http::Response {
    match &config.metrics_token {
        Some(token) => {
            let ok = req.header("authorization").map(|h| h == format!("Bearer {token}")).unwrap_or(false);
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
