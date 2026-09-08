//! Real integration tests — a real `Listener` bound to `127.0.0.1:0`
//! (an OS-assigned free port), a real background thread running its
//! accept loop, and a real `TcpStream` client for every test, matching
//! this whole session's own "run the real thing, don't just reason
//! about the code" discipline. Routes are hand-written `extern "C" fn`s
//! matching `RouteHandler`'s real ABI, not compiled `.nir` code — the
//! `codegen.rs` wiring that makes a real `.nir` program's own exposed
//! functions reach this exact table is separate, later work (this
//! crate's own module doc).

use super::*;
use std::io::{Read, Write};
use std::net::TcpStream as ClientStream;

fn leak_bytes(v: Vec<u8>) -> (*mut u8, i64) {
    let boxed = v.into_boxed_slice();
    let len = boxed.len() as i64;
    (Box::leak(boxed).as_mut_ptr(), len)
}

unsafe fn write_out(ptr: *mut u8, len: i64, out_ptr: *mut *mut u8, out_len: *mut i64) {
    unsafe {
        *out_ptr = ptr;
        *out_len = len;
    }
}

extern "C" fn echo_route(
    args_ptr: *const u8,
    args_len: i64,
    _id_ptr: *const u8,
    _id_len: i64,
    out_body_ptr: *mut *mut u8,
    out_body_len: *mut i64,
    out_cookie_ptr: *mut *mut u8,
    out_cookie_len: *mut i64,
) -> i32 {
    let args = unsafe { std::slice::from_raw_parts(args_ptr, args_len as usize) }.to_vec();
    let (ptr, len) = leak_bytes(args);
    unsafe {
        write_out(ptr, len, out_body_ptr, out_body_len);
        *out_cookie_ptr = std::ptr::null_mut();
        *out_cookie_len = 0;
    }
    0
}

extern "C" fn unauthorized_route(
    _args_ptr: *const u8,
    _args_len: i64,
    _id_ptr: *const u8,
    _id_len: i64,
    out_body_ptr: *mut *mut u8,
    out_body_len: *mut i64,
    out_cookie_ptr: *mut *mut u8,
    out_cookie_len: *mut i64,
) -> i32 {
    unsafe {
        *out_body_ptr = std::ptr::null_mut();
        *out_body_len = 0;
        *out_cookie_ptr = std::ptr::null_mut();
        *out_cookie_len = 0;
    }
    2
}

extern "C" fn cookie_route(
    _args_ptr: *const u8,
    _args_len: i64,
    _id_ptr: *const u8,
    _id_len: i64,
    out_body_ptr: *mut *mut u8,
    out_body_len: *mut i64,
    out_cookie_ptr: *mut *mut u8,
    out_cookie_len: *mut i64,
) -> i32 {
    let (body_ptr, body_len) = leak_bytes(b"{\"ok\":true}".to_vec());
    // A complete, fully-attributed cookie string -- since A5's fix,
    // `write_response` writes this out verbatim and appends nothing of
    // its own, matching what `kernel::identity::nir_session_cookie`
    // itself actually hands a real handler.
    let (cookie_ptr, cookie_len) = leak_bytes(b"nirdosha_session=abc123; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=28800".to_vec());
    unsafe {
        write_out(body_ptr, body_len, out_body_ptr, out_body_len);
        write_out(cookie_ptr, cookie_len, out_cookie_ptr, out_cookie_len);
    }
    0
}

extern "C" fn identity_echo_route(
    _args_ptr: *const u8,
    _args_len: i64,
    id_ptr: *const u8,
    id_len: i64,
    out_body_ptr: *mut *mut u8,
    out_body_len: *mut i64,
    out_cookie_ptr: *mut *mut u8,
    out_cookie_len: *mut i64,
) -> i32 {
    let body = if id_ptr.is_null() || id_len == 0 {
        b"{\"identity\":null}".to_vec()
    } else {
        let id = unsafe { std::slice::from_raw_parts(id_ptr, id_len as usize) }.to_vec();
        let mut out = b"{\"identity\":".to_vec();
        out.extend_from_slice(&id);
        out.push(b'}');
        out
    };
    let (ptr, len) = leak_bytes(body);
    unsafe {
        write_out(ptr, len, out_body_ptr, out_body_len);
        *out_cookie_ptr = std::ptr::null_mut();
        *out_cookie_len = 0;
    }
    0
}

static TEST_ROUTES: &[Route] = &[
    Route { path: "/api/echo", handler: echo_route },
    Route { path: "/api/unauthorized", handler: unauthorized_route },
    Route { path: "/api/cookie", handler: cookie_route },
    Route { path: "/api/identity", handler: identity_echo_route },
];

fn start_test_server(config: ServeConfig) -> (SocketAddr, Readiness) {
    let listener = bind("127.0.0.1:0").expect("bind should succeed");
    let addr = listener.local_addr().expect("local_addr should succeed");
    let readiness = Readiness::new();
    let readiness_for_thread = readiness.clone();
    std::thread::spawn(move || {
        listener.run(TEST_ROUTES, config, readiness_for_thread).expect("run should not itself fail");
    });
    // Real, bounded polling for the listener to actually be accepting
    // connections yet (a fresh thread needs a moment to start its
    // accept loop) -- not a fixed sleep guess.
    for _ in 0..200 {
        if ClientStream::connect(addr).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    (addr, readiness)
}

fn raw_request(addr: SocketAddr, request: &str) -> String {
    let mut stream = ClientStream::connect(addr).expect("connect should succeed");
    stream.write_all(request.as_bytes()).expect("write should succeed");
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut buf = Vec::new();
    let _ = stream.read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).into_owned()
}

fn status_of(response: &str) -> u16 {
    response.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0)
}

fn body_of(response: &str) -> String {
    response.split("\r\n\r\n").nth(1).unwrap_or("").to_string()
}

#[test]
fn a_registered_route_echoes_its_real_json_body() {
    let (addr, _r) = start_test_server(ServeConfig::default());
    let body = "{\"x\":1}";
    let req = format!("POST /api/echo HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    let resp = raw_request(addr, &req);
    assert_eq!(status_of(&resp), 200);
    assert_eq!(body_of(&resp), body);
}

#[test]
fn an_unregistered_path_returns_404() {
    let (addr, _r) = start_test_server(ServeConfig::default());
    let req = "GET /api/no-such-route HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n";
    let resp = raw_request(addr, req);
    assert_eq!(status_of(&resp), 404);
}

#[test]
fn a_route_returning_code_2_maps_to_401() {
    let (addr, _r) = start_test_server(ServeConfig::default());
    let req = "GET /api/unauthorized HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n";
    let resp = raw_request(addr, req);
    assert_eq!(status_of(&resp), 401);
}

#[test]
fn healthz_answers_ok_regardless_of_readiness() {
    let (addr, _r) = start_test_server(ServeConfig::default());
    let req = "GET /healthz HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n";
    let resp = raw_request(addr, req);
    assert_eq!(status_of(&resp), 200);
}

#[test]
fn readyz_is_503_until_marked_ready_then_200() {
    let (addr, readiness) = start_test_server(ServeConfig::default());
    let req = "GET /readyz HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n";
    let resp = raw_request(addr, req);
    assert_eq!(status_of(&resp), 503);
    readiness.mark_ready();
    let resp = raw_request(addr, req);
    assert_eq!(status_of(&resp), 200);
}

#[test]
fn a_body_over_the_max_size_is_rejected_with_413() {
    let (addr, _r) = start_test_server(ServeConfig::default());
    let oversized = MAX_BODY_BYTES + 1;
    let req = format!("POST /api/echo HTTP/1.1\r\nHost: x\r\nContent-Length: {oversized}\r\nConnection: close\r\n\r\n");
    let mut stream = ClientStream::connect(addr).unwrap();
    stream.write_all(req.as_bytes()).unwrap();
    // The body is rejected from the `Content-Length` header alone,
    // before this crate ever reads (or needs) the oversized body itself
    // -- so the test client never has to actually send one.
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut buf = Vec::new();
    let _ = stream.read_to_end(&mut buf);
    let resp = String::from_utf8_lossy(&buf);
    assert_eq!(status_of(&resp), 413);
}

/// `ServeHttpLease`'s own doc comment has the real, verified-by-hand
/// limit on what it can guarantee: a fault *inside* a `RouteHandler`
/// itself (a plain `extern "C" fn`) doesn't unwind at all and aborts
/// the whole process immediately — there is no connection, no lease,
/// and no test process left to assert anything against at that point,
/// so that case isn't (and can't safely be) exercised here.
///
/// A tight, isolated unit test of the `Drop` impl itself, not an
/// end-to-end request through a real `Listener` — `domain::serve_http()`'s
/// held count is a single process-wide counter shared by every
/// concurrently-running test in this binary (`cargo test`'s default
/// parallelism), so a broader "run N real requests, compare the
/// counter before and after" version of this test is genuinely racy
/// against sibling tests' own in-flight connections (found by actually
/// hitting exactly that flake, not assumed) — a single `acquire`+`drop`
/// pair, checked immediately with no sleep/poll window for another
/// test to interleave in, keeps the same real guarantee without it.
#[test]
fn serve_http_lease_drop_releases_exactly_one_admission() {
    assert!(kernel::acquire(domain::serve_http()));
    let (held_before, _, _, _) = kernel::stats(domain::serve_http());
    drop(ServeHttpLease);
    let (held_after, _, _, _) = kernel::stats(domain::serve_http());
    assert_eq!(held_after, held_before - 1);
}

#[test]
fn cors_reflects_only_a_configured_origin_never_a_wildcard() {
    let mut config = ServeConfig::default();
    config.allowed_origins = vec!["https://allowed.example".to_string()];
    let (addr, _r) = start_test_server(config);

    let req_allowed = "GET /api/echo HTTP/1.1\r\nHost: x\r\nOrigin: https://allowed.example\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}";
    let resp = raw_request(addr, req_allowed);
    assert!(resp.contains("Access-Control-Allow-Origin: https://allowed.example"));
    assert!(!resp.contains("Access-Control-Allow-Origin: *"));

    let req_other = "GET /api/echo HTTP/1.1\r\nHost: x\r\nOrigin: https://evil.example\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}";
    let resp = raw_request(addr, req_other);
    assert!(!resp.contains("Access-Control-Allow-Origin"));
}

#[test]
fn options_preflight_from_an_allowed_origin_gets_a_real_204() {
    let mut config = ServeConfig::default();
    config.allowed_origins = vec!["https://allowed.example".to_string()];
    let (addr, _r) = start_test_server(config);
    let req = "OPTIONS /api/echo HTTP/1.1\r\nHost: x\r\nOrigin: https://allowed.example\r\nConnection: close\r\n\r\n";
    let resp = raw_request(addr, req);
    assert_eq!(status_of(&resp), 204);
    assert!(resp.contains("Access-Control-Allow-Methods"));
}

#[test]
fn rate_limiting_denies_past_the_configured_max_within_one_window() {
    let mut config = ServeConfig::default();
    config.rate_limited_paths = vec!["/api/echo"];
    config.rate_limit_max_per_window = 2;
    config.rate_limit_window = Duration::from_secs(60);
    let (addr, _r) = start_test_server(config);
    let req = "GET /api/echo HTTP/1.1\r\nHost: x\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}";
    assert_eq!(status_of(&raw_request(addr, req)), 200);
    assert_eq!(status_of(&raw_request(addr, req)), 200);
    assert_eq!(status_of(&raw_request(addr, req)), 429);
}

#[test]
fn a_cookie_route_gets_a_real_set_cookie_header_with_security_attributes() {
    let (addr, _r) = start_test_server(ServeConfig::default());
    let req = "GET /api/cookie HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n";
    let resp = raw_request(addr, req);
    assert!(
        resp.contains("Set-Cookie: nirdosha_session=abc123; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=28800"),
        "write_response must write the handler's cookie verbatim, appending nothing of its own (A5): {resp}"
    );
}

#[test]
fn a_bearer_token_reaches_the_route_as_an_unverified_json_blob() {
    let (addr, _r) = start_test_server(ServeConfig::default());
    let req = "GET /api/identity HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer sometoken\r\nConnection: close\r\n\r\n";
    let resp = raw_request(addr, req);
    assert_eq!(status_of(&resp), 200);
    assert!(body_of(&resp).contains("sometoken"), "body: {}", body_of(&resp));
}

#[test]
fn no_authorization_header_reaches_the_route_as_a_null_identity() {
    let (addr, _r) = start_test_server(ServeConfig::default());
    let req = "GET /api/identity HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n";
    let resp = raw_request(addr, req);
    assert_eq!(body_of(&resp), "{\"identity\":null}");
}

#[test]
fn keep_alive_serves_a_second_request_on_the_same_connection() {
    let (addr, _r) = start_test_server(ServeConfig::default());
    let mut stream = ClientStream::connect(addr).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let req1 = "GET /healthz HTTP/1.1\r\nHost: x\r\n\r\n";
    stream.write_all(req1.as_bytes()).unwrap();
    let mut buf = [0u8; 4096];
    let n = stream.read(&mut buf).unwrap();
    let resp1 = String::from_utf8_lossy(&buf[..n]);
    assert_eq!(status_of(&resp1), 200);
    assert!(!resp1.to_ascii_lowercase().contains("connection: close"));

    let req2 = "GET /healthz HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n";
    stream.write_all(req2.as_bytes()).unwrap();
    let mut buf2 = Vec::new();
    stream.read_to_end(&mut buf2).unwrap();
    let resp2 = String::from_utf8_lossy(&buf2);
    assert_eq!(status_of(&resp2), 200);
}

#[test]
fn max_requests_per_connection_closes_after_the_configured_count() {
    let mut config = ServeConfig::default();
    config.max_requests_per_connection = 1;
    let (addr, _r) = start_test_server(config);
    let mut stream = ClientStream::connect(addr).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let req = "GET /healthz HTTP/1.1\r\nHost: x\r\n\r\n";
    stream.write_all(req.as_bytes()).unwrap();
    let mut buf = Vec::new();
    // With `max_requests_per_connection == 1`, the server must close
    // after this one response -- `read_to_end` only returns once the
    // peer closes, so this would hang (and time out) if it kept the
    // connection open waiting for a second request.
    stream.read_to_end(&mut buf).unwrap();
    let resp = String::from_utf8_lossy(&buf);
    assert_eq!(status_of(&resp), 200);
}

#[test]
fn metrics_requires_the_configured_bearer_token() {
    let mut config = ServeConfig::default();
    config.metrics_token = Some("secret".to_string());
    let (addr, _r) = start_test_server(config);

    let unauthed = "GET /metrics HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n";
    assert_eq!(status_of(&raw_request(addr, unauthed)), 401);

    let authed = "GET /metrics HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer secret\r\nConnection: close\r\n\r\n";
    let resp = raw_request(addr, authed);
    assert_eq!(status_of(&resp), 200);
    assert!(body_of(&resp).contains("flight recorder"));
}

#[test]
fn metrics_answers_unauthenticated_when_no_token_is_configured() {
    let (addr, _r) = start_test_server(ServeConfig::default());
    let req = "GET /metrics HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n";
    assert_eq!(status_of(&raw_request(addr, req)), 200);
}

/// Red-team report A11/A23 (`scratch/red-team-report-main-d7fae42.md`):
/// `kernel::acquire(domain::serve_http())` is now checked in the accept
/// loop itself, before any thread is spawned for the connection (see
/// `Listener::run`'s own doc comment for the full "why," including why
/// a denied connection is dropped rather than answered with a `503`).
/// This test proves that end to end: with the ceiling saturated by one
/// held-open connection, a second connection attempt gets no response
/// at all -- the server closes it immediately, rather than spawning a
/// thread that (in the old design) would have written a `503` body.
///
/// `#[ignore]`d for the same reason every other test touching a
/// cached-forever built-in domain ceiling in this codebase already is
/// (`NIRDOSHA_KERNEL_MAX_DB`'s own regression test, `lib.rs`'s
/// `db_kernel_tests` module, has the full rationale): `domain::
/// serve_http()`'s ceiling is resolved once, lazily, and cached for the
/// rest of the process -- setting the env var here only has any effect
/// if this is the very first thing in the whole test binary to ever
/// call `kernel::acquire(domain::serve_http())`, which every other test
/// in this file's own `start_test_server` calls too. Safe only run
/// alone: `cargo test -- --ignored a_connection_past_the_serve_http_ceiling_is_dropped_not_answered`.
#[test]
#[ignore]
fn a_connection_past_the_serve_http_ceiling_is_dropped_not_answered() {
    unsafe { std::env::set_var("NIRDOSHA_KERNEL_MAX_SERVE_HTTP", "1") };
    let (addr, _r) = start_test_server(ServeConfig::default());

    // First connection: holds the ceiling's one slot open by never
    // completing its HTTP request (no full header block sent, so the
    // server's own `handle_connection` thread stays parked in
    // `read_headers` -- `header_timeout`'s default 10s is far longer
    // than this test needs).
    let _held = ClientStream::connect(addr).expect("first connection, under the ceiling of 1, must be accepted");

    // Second connection, past the now-saturated ceiling of 1: the
    // accept loop's own `kernel::acquire` must deny it and drop the
    // stream immediately -- `read_to_end` returning zero bytes (a clean
    // EOF with nothing written first) is exactly what a dropped-without-
    // a-response connection looks like from the client's side.
    let mut denied = ClientStream::connect(addr).expect("TCP accept itself still succeeds; kernel::acquire denies it after");
    denied.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut buf = Vec::new();
    let _ = denied.read_to_end(&mut buf);
    assert!(buf.is_empty(), "a connection past the serve_http ceiling must get no response at all (dropped, not a 503 body): {:?}", String::from_utf8_lossy(&buf));

    unsafe { std::env::remove_var("NIRDOSHA_KERNEL_MAX_SERVE_HTTP") };
}
