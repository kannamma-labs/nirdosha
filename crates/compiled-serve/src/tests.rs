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

/// `GET /` serves the baked-in UI (if any) as real HTML, real content
/// type included — not the `application/json` every other response on
/// this server correctly still uses.
#[test]
fn root_serves_the_baked_in_ui_as_real_html() {
    let mut config = ServeConfig::default();
    config.ui_html = b"<!doctype html><title>t</title>".to_vec();
    let (addr, _r) = start_test_server(config);
    let req = "GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n";
    let resp = raw_request(addr, req);
    assert_eq!(status_of(&resp), 200);
    assert!(resp.to_ascii_lowercase().contains("content-type: text/html"), "response: {resp}");
    assert!(body_of(&resp).contains("<!doctype html>"), "body: {}", body_of(&resp));
}

/// No `ui_html` baked in (this crate's own `ServeConfig::default()`,
/// and every test above that doesn't set it) — `GET /` 404s rather
/// than serving an empty page silently.
#[test]
fn root_with_no_ui_baked_in_is_404() {
    let (addr, _r) = start_test_server(ServeConfig::default());
    let req = "GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n";
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

/// Red-team report A21 (`scratch/red-team-report-main-d7fae42.md`): a
/// browser normalizes `https://allowed.example:443` and
/// `https://allowed.example` to the same origin for CORS purposes -- a
/// request carrying the explicit default port must still match a
/// configured origin without one.
#[test]
fn cors_matches_an_explicit_default_port_against_a_configured_origin_without_one() {
    let mut config = ServeConfig::default();
    config.allowed_origins = vec!["https://allowed.example".to_string()];
    let (addr, _r) = start_test_server(config);

    let req = "GET /api/echo HTTP/1.1\r\nHost: x\r\nOrigin: https://allowed.example:443\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}";
    let resp = raw_request(addr, req);
    assert!(
        resp.contains("Access-Control-Allow-Origin: https://allowed.example:443"),
        "an explicit :443 must still match the same https origin configured without a port: {resp}"
    );
}

#[test]
fn origins_match_normalizes_explicit_default_ports() {
    assert!(origins_match("https://a.example", "https://a.example:443"));
    assert!(origins_match("http://a.example", "http://a.example:80"));
    assert!(!origins_match("https://a.example", "https://a.example:8443"), "a non-default explicit port must NOT match a bare origin");
    assert!(!origins_match("https://a.example", "http://a.example"), "scheme must still be part of the match");
    assert!(!origins_match("https://a.example", "https://b.example"), "host must still be part of the match");
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

/// Real verification, not the old placeholder: a bearer token only
/// reaches a route as `identity_json` when it actually verifies against
/// the server's own `AuthConfig` — minting one via
/// `identity::mock_issue_token` (the same path `/api/_demo_login`
/// itself calls) rather than an arbitrary opaque string, since an
/// arbitrary string is exactly what real verification now correctly
/// rejects (see `a_garbled_bearer_token_is_rejected_before_reaching_the_route`
/// below).
#[test]
fn a_bearer_token_reaches_the_route_as_identity_json() {
    let config = ServeConfig::default();
    let token = identity::mock_issue_token("alice", &config.auth[0], &["admin".to_string()], &[("dept".to_string(), "eng".to_string())]).expect("minting a demo token should succeed");
    let (addr, _r) = start_test_server(config);
    let req = format!("GET /api/identity HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n");
    let resp = raw_request(addr, &req);
    assert_eq!(status_of(&resp), 200, "body: {}", body_of(&resp));
    let body = body_of(&resp);
    assert!(body.contains("\"subject\":\"alice\""), "body: {body}");
    assert!(body.contains("roles") && body.contains("admin"), "body: {body}");
    assert!(body.contains("eng"), "body: {body}");
}

/// A real, distinct `AuthConfig` for multi-provider tests — same
/// ephemeral-`/dev/urandom`-secret construction `AuthConfig::demo()`
/// uses, just with a caller-chosen issuer/audience instead of the fixed
/// `"nirdosha-demo"` pair, so two calls never collide.
fn make_test_auth_config(issuer: &str, audience: &str) -> AuthConfig {
    use base64::Engine as _;
    let mut buf = [0u8; 32];
    std::fs::File::open("/dev/urandom").and_then(|mut f| Read::read_exact(&mut f, &mut buf)).expect("OS entropy source");
    let secret = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf);
    let jwks_json = serde_json::json!({"keys": [{"kid": "test", "kty": "oct", "k": secret}]}).to_string();
    AuthConfig { jwks_json, issuer: issuer.to_string(), audience: audience.to_string() }
}

/// ROADMAP.md A6's "Multi-IdP registry" — `identity::validate_token`
/// dispatches by the token's own (unverified) issuer claim, then runs
/// the real signature check against the *matching* provider's own JWKS,
/// never a different one's.
#[test]
fn multiple_providers_dispatch_by_the_tokens_own_issuer_claim() {
    let provider_a = make_test_auth_config("issuer-a", "aud-a");
    let provider_b = make_test_auth_config("issuer-b", "aud-b");
    let mut config = ServeConfig::default();
    config.auth = vec![provider_a.clone(), provider_b.clone()];
    config.demo_mode = false;

    let token_a = identity::mock_issue_token("alice", &provider_a, &["admin".to_string()], &[]).expect("mint against provider a");
    let token_b = identity::mock_issue_token("bob", &provider_b, &[], &[]).expect("mint against provider b");
    let (addr, _r) = start_test_server(config);

    let req_a = format!("GET /api/identity HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {token_a}\r\nConnection: close\r\n\r\n");
    let resp_a = raw_request(addr, &req_a);
    assert_eq!(status_of(&resp_a), 200, "a token from provider a must verify against provider a's own JWKS: {}", body_of(&resp_a));
    assert!(body_of(&resp_a).contains("\"subject\":\"alice\""), "body: {}", body_of(&resp_a));

    let req_b = format!("GET /api/identity HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {token_b}\r\nConnection: close\r\n\r\n");
    let resp_b = raw_request(addr, &req_b);
    assert_eq!(status_of(&resp_b), 200, "a token from provider b must verify against provider b's own JWKS, not provider a's: {}", body_of(&resp_b));
    assert!(body_of(&resp_b).contains("\"subject\":\"bob\""), "body: {}", body_of(&resp_b));
}

/// A token whose issuer matches none of the configured providers is a
/// real 401, never a silent fallback to whichever provider happens to
/// be first in the list.
#[test]
fn an_unrecognized_issuer_401s_rather_than_falling_back() {
    let provider_a = make_test_auth_config("issuer-a", "aud-a");
    let provider_b = make_test_auth_config("issuer-b", "aud-b");
    let provider_unconfigured = make_test_auth_config("issuer-unconfigured", "aud-unconfigured");
    let mut config = ServeConfig::default();
    config.auth = vec![provider_a, provider_b];
    config.demo_mode = false;

    let token = identity::mock_issue_token("eve", &provider_unconfigured, &[], &[]).expect("mint against the unconfigured provider");
    let (addr, _r) = start_test_server(config);
    let req = format!("GET /api/identity HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n");
    let resp = raw_request(addr, &req);
    assert_eq!(status_of(&resp), 401, "body: {}", body_of(&resp));
}

#[test]
fn no_authorization_header_reaches_the_route_as_a_null_identity() {
    let (addr, _r) = start_test_server(ServeConfig::default());
    let req = "GET /api/identity HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n";
    let resp = raw_request(addr, req);
    assert_eq!(body_of(&resp), "{\"identity\":null}");
}

/// A present-but-invalid `Authorization` header is a hard `401` before
/// the route is ever reached — never silently treated as "no identity"
/// (that would let a caller shrug off a bad token and get an anonymous
/// response from a route that might otherwise have required one).
#[test]
fn a_garbled_bearer_token_is_rejected_before_reaching_the_route() {
    let (addr, _r) = start_test_server(ServeConfig::default());
    let req = "GET /api/identity HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer not-a-real-jwt\r\nConnection: close\r\n\r\n";
    let resp = raw_request(addr, req);
    assert_eq!(status_of(&resp), 401, "body: {}", body_of(&resp));
}

/// `/api/_demo_login` mints a real token; feeding it straight back in
/// as a bearer token on an ordinary route proves the whole loop is
/// real, not just that minting alone succeeds.
#[test]
fn demo_login_mints_a_token_that_a_real_route_accepts() {
    let (addr, _r) = start_test_server(ServeConfig::default());
    let body = r#"{"subject":"bob","roles":["editor"],"claims":{"team":"docs"}}"#;
    let login_req = format!("POST /api/_demo_login HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    let login_resp = raw_request(addr, &login_req);
    assert_eq!(status_of(&login_resp), 200, "body: {}", body_of(&login_resp));
    let login_body: serde_json::Value = serde_json::from_str(&body_of(&login_resp)).expect("demo login body should be real JSON");
    let token = login_body["token"].as_str().expect("demo login response should carry a token");

    let whoami_req = format!("GET /api/_whoami HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n");
    let whoami_resp = raw_request(addr, &whoami_req);
    assert_eq!(status_of(&whoami_resp), 200, "body: {}", body_of(&whoami_resp));
    assert!(body_of(&whoami_resp).contains("\"subject\":\"bob\""), "body: {}", body_of(&whoami_resp));

    let route_req = format!("GET /api/identity HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n");
    let route_resp = raw_request(addr, &route_req);
    assert_eq!(status_of(&route_resp), 200);
    assert!(body_of(&route_resp).contains("editor"), "body: {}", body_of(&route_resp));
}

/// `/api/_demo_login` doesn't exist at all outside demo mode — a real
/// identity server has no self-service login.
#[test]
fn demo_login_is_not_available_outside_demo_mode() {
    let mut config = ServeConfig::default();
    config.demo_mode = false;
    let (addr, _r) = start_test_server(config);
    let req = "POST /api/_demo_login HTTP/1.1\r\nHost: x\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}";
    let resp = raw_request(addr, req);
    assert_eq!(status_of(&resp), 404);
}

/// `/api/_whoami` with no `Authorization` header at all is a plain
/// `401` (there's no identity to confirm), distinct from the
/// invalid-token case above only in its message, not its status.
#[test]
fn whoami_with_no_header_is_401() {
    let (addr, _r) = start_test_server(ServeConfig::default());
    let req = "GET /api/_whoami HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n";
    let resp = raw_request(addr, req);
    assert_eq!(status_of(&resp), 401);
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

/// Red-team report A16. `#[ignore]`d: `NIRDOSHA_SERVE_TRUSTED_PROXIES`
/// is a real process-wide env var, and every other test in this file
/// calls `ServeConfig::default()` too -- setting it here while other
/// tests run in parallel (`cargo test`'s default) would leak into their
/// own `default()` calls. Safe only run alone.
#[test]
#[ignore]
fn trusted_proxies_from_env_parses_a_comma_separated_list_and_skips_invalid_entries() {
    unsafe { std::env::set_var("NIRDOSHA_SERVE_TRUSTED_PROXIES", "10.0.0.1, 10.0.0.2,not-an-ip,192.168.1.1") };
    let proxies = trusted_proxies_from_env();
    let expected: Vec<std::net::IpAddr> = vec!["10.0.0.1".parse().unwrap(), "10.0.0.2".parse().unwrap(), "192.168.1.1".parse().unwrap()];
    assert_eq!(proxies, expected, "valid entries (whitespace-trimmed) must parse in order; the invalid one must be skipped, not abort the whole list");
    unsafe { std::env::remove_var("NIRDOSHA_SERVE_TRUSTED_PROXIES") };
}

#[test]
fn trusted_proxies_from_env_is_empty_when_unset() {
    unsafe { std::env::remove_var("NIRDOSHA_SERVE_TRUSTED_PROXIES") };
    assert!(trusted_proxies_from_env().is_empty());
}

#[test]
fn auth_providers_from_env_is_demo_mode_when_unset() {
    unsafe {
        std::env::remove_var("NIRDOSHA_JWKS_FILE");
        std::env::remove_var("NIRDOSHA_ISSUER");
        std::env::remove_var("NIRDOSHA_AUDIENCE");
        std::env::remove_var("NIRDOSHA_IDENTITY_PROVIDERS");
    }
    let (auth, demo_mode) = auth_providers_from_env();
    assert!(demo_mode);
    assert_eq!(auth.len(), 1);
    assert_eq!(auth[0].issuer, "nirdosha-demo");
}

/// Same "process-wide env var, safe only run alone" reasoning as
/// `trusted_proxies_from_env_parses_a_comma_separated_list...` right
/// above — every other test in this file calls `ServeConfig::default()`
/// too.
#[test]
#[ignore]
fn auth_providers_from_env_reads_a_real_single_provider() {
    let jwks_path = std::env::temp_dir().join(format!("nirdosha_test_jwks_{}.json", std::process::id()));
    std::fs::write(&jwks_path, r#"{"keys":[]}"#).expect("scratch jwks file should write");
    unsafe {
        std::env::set_var("NIRDOSHA_JWKS_FILE", &jwks_path);
        std::env::set_var("NIRDOSHA_ISSUER", "https://issuer.example");
        std::env::set_var("NIRDOSHA_AUDIENCE", "my-app");
        std::env::remove_var("NIRDOSHA_IDENTITY_PROVIDERS");
    }
    let (auth, demo_mode) = auth_providers_from_env();
    assert!(!demo_mode);
    assert_eq!(auth.len(), 1);
    assert_eq!(auth[0].issuer, "https://issuer.example");
    assert_eq!(auth[0].audience, "my-app");
    assert_eq!(auth[0].jwks_json, r#"{"keys":[]}"#);
    unsafe {
        std::env::remove_var("NIRDOSHA_JWKS_FILE");
        std::env::remove_var("NIRDOSHA_ISSUER");
        std::env::remove_var("NIRDOSHA_AUDIENCE");
    }
    let _ = std::fs::remove_file(&jwks_path);
}

#[test]
#[ignore]
fn auth_providers_from_env_reads_a_real_provider_list_file() {
    let jwks_path_a = std::env::temp_dir().join(format!("nirdosha_test_jwks_a_{}.json", std::process::id()));
    let jwks_path_b = std::env::temp_dir().join(format!("nirdosha_test_jwks_b_{}.json", std::process::id()));
    std::fs::write(&jwks_path_a, r#"{"keys":["a"]}"#).expect("scratch jwks file should write");
    std::fs::write(&jwks_path_b, r#"{"keys":["b"]}"#).expect("scratch jwks file should write");
    let providers_json = serde_json::json!([
        {"jwks_file": jwks_path_a.to_str().unwrap(), "issuer": "issuer-a", "audience": "aud-a"},
        {"jwks_file": jwks_path_b.to_str().unwrap(), "issuer": "issuer-b", "audience": "aud-b"},
    ]);
    let providers_path = std::env::temp_dir().join(format!("nirdosha_test_providers_{}.json", std::process::id()));
    std::fs::write(&providers_path, providers_json.to_string()).expect("scratch providers file should write");
    unsafe {
        std::env::remove_var("NIRDOSHA_JWKS_FILE");
        std::env::set_var("NIRDOSHA_IDENTITY_PROVIDERS", &providers_path);
    }
    let (auth, demo_mode) = auth_providers_from_env();
    assert!(!demo_mode);
    assert_eq!(auth.len(), 2, "auth: {auth:?}");
    assert_eq!(auth[0].issuer, "issuer-a");
    assert_eq!(auth[0].jwks_json, r#"{"keys":["a"]}"#);
    assert_eq!(auth[1].issuer, "issuer-b");
    assert_eq!(auth[1].jwks_json, r#"{"keys":["b"]}"#);
    unsafe { std::env::remove_var("NIRDOSHA_IDENTITY_PROVIDERS") };
    let _ = std::fs::remove_file(&jwks_path_a);
    let _ = std::fs::remove_file(&jwks_path_b);
    let _ = std::fs::remove_file(&providers_path);
}

/// A malformed/unreadable `NIRDOSHA_IDENTITY_PROVIDERS` degrades to demo
/// mode with a loud `eprintln!`, the same non-fatal posture every other
/// env-config parse failure in this crate already has — never a process
/// crash over a deployment mistake.
#[test]
#[ignore]
fn auth_providers_from_env_degrades_to_demo_mode_on_a_malformed_providers_file() {
    let providers_path = std::env::temp_dir().join(format!("nirdosha_test_providers_bad_{}.json", std::process::id()));
    std::fs::write(&providers_path, "not valid json").expect("scratch providers file should write");
    unsafe {
        std::env::remove_var("NIRDOSHA_JWKS_FILE");
        std::env::set_var("NIRDOSHA_IDENTITY_PROVIDERS", &providers_path);
    }
    let (auth, demo_mode) = auth_providers_from_env();
    assert!(demo_mode);
    assert_eq!(auth.len(), 1);
    assert_eq!(auth[0].issuer, "nirdosha-demo");
    unsafe { std::env::remove_var("NIRDOSHA_IDENTITY_PROVIDERS") };
    let _ = std::fs::remove_file(&providers_path);
}
