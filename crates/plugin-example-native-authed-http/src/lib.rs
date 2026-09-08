//! `authedhttp_provider_authedhttp_{connect,request,is_valid,close}` —
//! rfcs/0011-uniform-service-provider-model.md Phase 8's real, shipped
//! `call`-shape plugin. Demonstrates the concrete gap the RFC's own
//! Motivation names: `call_via`'s two core-HTTP-served schemes
//! (`http://`/`https://`) have no way to attach a custom `Authorization`
//! header from `.nir` code today — this plugin fills that gap for one
//! specific scheme (`authedhttp://`) by sourcing a bearer token from the
//! process environment at connect time and injecting it into every
//! request it makes on that connection's behalf.
//!
//! ## Same hand-rolled state story as `plugin-example-native-kv`
//!
//! See that crate's own module doc for why: no compiled-path
//! `HandleRegistry<T>` equivalent exists yet, so `CONNS`/`next_handle`
//! below are the same ~15-line pattern every stateful compiled plugin
//! currently writes by hand.
//!
//! ## Hand-rolled HTTP/1.1 client, not a dependency
//!
//! This crate cannot depend on `runtime-kernels` (no shared crate exists
//! between it, `compiler`, and a plugin crate — this RFC's own Context
//! section) or on any external HTTP client crate (the same "zero
//! dependency beyond `std`" convention `plugin-example-native-shout`/
//! `-kv` already establish, this crate's own `Cargo.toml` comment). The
//! request/response framing below is the same minimal
//! `std::net::TcpStream` + hand-written `Connection: close` request
//! `runtime-kernels::kernel::http`'s own client path uses internally —
//! this plugin doesn't need pooling or keep-alive itself, since that's
//! already the kernel's job one layer up (`PoolRegistry<
//! PluginManagedConnection>`, Phase 5): one real TCP connection per
//! `_request` call is the correct scope here.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, OnceLock};

/// Must match every other plugin crate's own copy field-for-field (RFC
/// 0011 §2, `kernel::pool::NirStr`'s own doc comment) — each plugin
/// crate declares its own rather than sharing one, same as
/// `plugin-example-native-kv`'s identical doc note.
#[repr(C)]
pub struct NirStr {
    pub ptr: *const u8,
    pub len: i64,
}

unsafe fn nir_str_to_string(s: &NirStr) -> String {
    let bytes = unsafe { std::slice::from_raw_parts(s.ptr, s.len as usize) };
    String::from_utf8_lossy(bytes).into_owned()
}

fn string_to_nir_str(s: String) -> NirStr {
    let leaked: &'static mut [u8] = Box::leak(s.into_bytes().into_boxed_slice());
    NirStr { ptr: leaked.as_ptr(), len: leaked.len() as i64 }
}

/// One live "connection" — really just the target `host:port` and the
/// bearer token resolved once, at `_connect` time, per RFC 0011 §2's
/// "the plugin's four functions never see a kernel id, only ever the
/// raw handle it was given" contract. No actual TCP socket is held open
/// between calls — see this module's own doc comment for why one fresh
/// `TcpStream` per `_request` is the right scope here, not a gap.
struct ConnState {
    addr: String,
    token: String,
}

fn conns() -> &'static Mutex<HashMap<i64, ConnState>> {
    static CONNS: OnceLock<Mutex<HashMap<i64, ConnState>>> = OnceLock::new();
    CONNS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_handle() -> i64 {
    static NEXT: AtomicI64 = AtomicI64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// `host` is the *whole* dial string `call_via`'s plugin fallback passes
/// through unmodified (`kernel::plugin_provider::call_via`'s own `host =
/// url.to_string()`), e.g. `authedhttp://127.0.0.1:54321` — this
/// function strips the `authedhttp://` prefix itself rather than
/// expecting it pre-stripped.
///
/// Reads the bearer token from `AUTHEDHTTP_BEARER_TOKEN` in the process
/// environment once, here, at connect time — exactly the credential-at-
/// connect-time pattern RFC 0011's own Phase 8 description names (the
/// same env-var-sourced-credential shape `.nir`'s own `env(...)` builtin,
/// Phase 1, exists to support — a real plugin reads its config via the
/// process environment directly in its own Rust code, the same way any
/// other native process would, rather than calling back into a `.nir`
/// builtin).
///
/// Returns a negative `i64` on failure (RFC 0011 §2's own "0/1/negative
/// sentinels every other kernel boundary already uses" convention) — no
/// token set is the only failure mode this function has.
#[unsafe(no_mangle)]
pub extern "C" fn authedhttp_provider_authedhttp_connect(host: NirStr) -> i64 {
    let url = unsafe { nir_str_to_string(&host) };
    let addr = url.strip_prefix("authedhttp://").unwrap_or(url.as_str()).to_string();
    let Ok(token) = std::env::var("AUTHEDHTTP_BEARER_TOKEN") else {
        return -1;
    };
    let h = next_handle();
    conns().lock().unwrap().insert(h, ConnState { addr, token });
    h
}

/// One real HTTP/1.1 request per call, `Connection: close` (no keep-
/// alive on this side — see this module's own doc comment), with
/// `Authorization: Bearer <token>` set from the token stashed at
/// `_connect` time. Returns the response body as plain `str` — RFC 0011
/// §2's `_request` ABI has no separate status-code channel
/// (`nir_call_via`'s own doc comment discloses this cost already), so a
/// transport-level failure (host unreachable, connection refused) is
/// reported by returning an `"ERR: ..."`-prefixed body rather than
/// panicking or aborting the process; `nir_call_via` always reports
/// `status: 200` for any plugin-routed response body regardless of this
/// prefix, so a real caller has to check for it — the same lowest-
/// common-denominator cost every `call_via` consumer already accepts.
#[unsafe(no_mangle)]
pub extern "C" fn authedhttp_provider_authedhttp_request(raw_conn: i64, path: NirStr, body: NirStr) -> NirStr {
    let path = unsafe { nir_str_to_string(&path) };
    let body = unsafe { nir_str_to_string(&body) };

    let (addr, token) = {
        let guard = conns().lock().unwrap();
        match guard.get(&raw_conn) {
            Some(c) => (c.addr.clone(), c.token.clone()),
            None => return string_to_nir_str("ERR: connection not open".to_string()),
        }
    };

    match do_request(&addr, &path, &token, &body) {
        Ok(response_body) => string_to_nir_str(response_body),
        Err(e) => string_to_nir_str(format!("ERR: {e}")),
    }
}

fn do_request(addr: &str, path: &str, token: &str, body: &str) -> Result<String, String> {
    let mut stream = TcpStream::connect(addr).map_err(|e| e.to_string())?;
    let host_header = addr.split(':').next().unwrap_or(addr);
    let request = format!(
        "POST {path} HTTP/1.1\r\n\
         Host: {host_header}\r\n\
         Authorization: Bearer {token}\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).map_err(|e| e.to_string())?;
    let raw = String::from_utf8_lossy(&raw);
    match raw.split_once("\r\n\r\n") {
        Some((_headers, body)) => Ok(body.to_string()),
        None => Ok(String::new()),
    }
}

/// `1` if `raw_conn` still names a stashed connection, `0` otherwise —
/// r2d2's own `test_on_check_out` default (`true`, never overridden by
/// `PoolConfig::apply`, `pool.rs`'s own doc comment) means this runs on
/// every checkout; always-valid here since no real socket is held open
/// to go stale (this module's own doc comment) — the only way this
/// returns `0` is a handle that's already been `_close`d.
#[unsafe(no_mangle)]
pub extern "C" fn authedhttp_provider_authedhttp_is_valid(raw_conn: i64) -> i64 {
    if conns().lock().unwrap().contains_key(&raw_conn) {
        1
    } else {
        0
    }
}

/// Returns `1` on success, `0` if `raw_conn` was already closed (or
/// never opened) — same double-close-is-defense-in-depth-only story as
/// `plugin-example-native-kv::kv_close`'s own doc comment (a well-typed
/// `.nir` program can never trigger the `0` case; `ownership.rs`'s
/// affine tracking already rejects that at compile time).
#[unsafe(no_mangle)]
pub extern "C" fn authedhttp_provider_authedhttp_close(raw_conn: i64) -> i64 {
    match conns().lock().unwrap().remove(&raw_conn) {
        Some(_) => 1,
        None => 0,
    }
}
