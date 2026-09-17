//! RFC 9449 (DPoP), proved end-to-end -- a real server, a real
//! `with_sender_constrained_tokens` config, and real ES256-signed DPoP
//! proofs over the wire, not just `web/dpop.rs`'s own unit tests of the
//! pure `verify_proof` function in isolation.
#![cfg(feature = "dpop")]

use nirdosha_rt::web::{Runtime, ServeConfig};
use nirdosha_rt::{Auth, Response, Router};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

struct Server {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<std::io::Result<()>>>,
}

impl Server {
    fn start(router: Router) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker = thread::spawn(move || {
            router.serve_until(listener, &worker_stop, ServeConfig { max_connections: 8.try_into().unwrap(), io_timeout: Duration::from_millis(250) })
        });
        Self { addr, stop, worker: Some(worker) }
    }

    fn request(&self, raw: &str) -> String {
        let mut stream = TcpStream::connect(self.addr).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        stream.set_write_timeout(Some(Duration::from_secs(3))).unwrap();
        stream.write_all(raw.as_bytes()).unwrap();
        let mut response = String::new();
        let _ = stream.read_to_string(&mut response);
        response
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.worker.take().unwrap().join().unwrap().unwrap();
    }
}

fn status_of(response: &str) -> u16 {
    response.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0)
}

// A fixed, checked-in P-256 test keypair -- the same one `web/dpop.rs`'s
// own unit tests and `runtime-kernels`' original DPoP tests use,
// test-only, never used for anything real. `KEY_B` exists solely to
// prove a proof signed by the *wrong* key is rejected.
const KEY_A_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgzvUAj7DFAlncvF+5\nKh1PaOnplTGaH4VKUbad2SJZRc2hRANCAAT4Fgvuc92G/Tlx3tdInAnryMO+cPO4\nZ77MvnaJskfNgdVa75Dkb9ta42OPIVpSYfDdWMIEg01aGaWsmssB/6vQ\n-----END PRIVATE KEY-----\n";
const KEY_A_X: &str = "-BYL7nPdhv05cd7XSJwJ68jDvnDzuGe-zL52ibJHzYE";
const KEY_A_Y: &str = "1VrvkORv21rjY48hWlJh8N1YwgSDTVoZpayaywH_q9A";
const KEY_A_JKT: &str = "m4TkNpMqi-3VybpMJQzhinaLaT3W4Gt9I7JNoKjF9Y8";

const KEY_B_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg/flRaWiG+0UCUPUk\nwYANdoO+1FUZYnrji82OiEhLS2GhRANCAAQcXG156IF16KjypyaCRsVZ12Rkqx8K\nJJTuJQ16JSpdxBDa2jOdwsvREKYrtwT7V/xWJRCkxKSYWKHbvFr9Zfm9\n-----END PRIVATE KEY-----\n";
const KEY_B_X: &str = "HFxteeiBdeio8qcmgkbFWddkZKsfCiSU7iUNeiUqXcQ";
const KEY_B_Y: &str = "ENraM53Cy9EQpiu3BPtX_FYlEKTEpJhYodu8Wv1l-b0";

fn make_proof(key_pem: &str, x: &str, y: &str, claims: serde_json::Value) -> String {
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
    header.typ = Some("dpop+jwt".to_string());
    header.jwk = Some(jsonwebtoken::jwk::Jwk {
        common: jsonwebtoken::jwk::CommonParameters::default(),
        algorithm: jsonwebtoken::jwk::AlgorithmParameters::EllipticCurve(jsonwebtoken::jwk::EllipticCurveKeyParameters {
            key_type: jsonwebtoken::jwk::EllipticCurveKeyType::EC,
            curve: jsonwebtoken::jwk::EllipticCurve::P256,
            x: x.to_string(),
            y: y.to_string(),
        }),
    });
    let encoding_key = jsonwebtoken::EncodingKey::from_ec_pem(key_pem.as_bytes()).expect("fixed test key must parse");
    jsonwebtoken::encode(&header, &claims, &encoding_key).expect("signing a well-formed proof must succeed")
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
}

fn dpop_claims(htm: &str, htu: &str, jti: &str, ath: Option<&str>) -> serde_json::Value {
    let mut c = serde_json::json!({ "htm": htm, "htu": htu, "iat": now(), "jti": jti });
    if let Some(ath) = ath {
        c["ath"] = serde_json::json!(ath);
    }
    c
}

fn access_token_hash(token: &str) -> String {
    use base64::Engine as _;
    use sha2::Digest;
    let digest = sha2::Sha256::digest(token.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

/// A fixed access token string -- this router's own `authenticate`
/// closure below decides what `Auth` a caller gets purely by whether
/// the bearer token matches this literal, and always binds it to
/// `KEY_A_JKT` when it does. Standing in for the real thing (an
/// `authenticate` closure that actually verifies a JWT's signature and
/// reads its `cnf.jkt` claim) -- the DPoP check downstream of
/// `authenticate` doesn't care how `Auth::cnf_jkt` got set, only that
/// it did.
const BOUND_TOKEN: &str = "bound-access-token-xyz";

fn router() -> Router {
    Router::new(|req| {
        let bound = req.header("authorization").map(|h| h.trim_start_matches("Bearer ")) == Some(BOUND_TOKEN);
        if bound {
            Auth::login("carol", &[]).with_cnf_jkt(KEY_A_JKT)
        } else {
            Auth::login("carol", &[])
        }
    })
    .with_sender_constrained_tokens()
    .with_runtime(Runtime::Sync)
    .get("/api/report", "report", |_, _| Response::text(200, "ok"))
}

/// The full round trip: a correctly bound token plus a fresh, correctly
/// signed proof reaches the route; the *same* proof replayed is
/// rejected; no `DPoP` header at all is rejected; a proof signed by a
/// different key entirely is rejected even though the token itself is
/// genuinely valid; and a token with no `cnf.jkt` binding at all
/// (`Auth::login` with no `with_cnf_jkt`) is rejected even with a
/// perfect proof attached.
#[test]
fn dpop_round_trip_over_a_real_server() {
    let server = Server::start(router());
    let ath = access_token_hash(BOUND_TOKEN);
    let host = format!("127.0.0.1:{}", server.addr.port());
    let url = format!("http://{host}/api/report");

    let proof = make_proof(KEY_A_PEM, KEY_A_X, KEY_A_Y, dpop_claims("GET", &url, "proof-1", Some(&ath)));
    let ok = server.request(&format!("GET /api/report HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {BOUND_TOKEN}\r\nDPoP: {proof}\r\nConnection: close\r\n\r\n"));
    assert_eq!(status_of(&ok), 200, "body: {ok}");

    let replay = server.request(&format!("GET /api/report HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {BOUND_TOKEN}\r\nDPoP: {proof}\r\nConnection: close\r\n\r\n"));
    assert_eq!(status_of(&replay), 401, "the same proof presented twice must be rejected as a replay: {replay}");

    let no_proof = server.request(&format!("GET /api/report HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {BOUND_TOKEN}\r\nConnection: close\r\n\r\n"));
    assert_eq!(status_of(&no_proof), 401, "no DPoP header at all must be rejected: {no_proof}");

    let wrong_key_proof = make_proof(KEY_B_PEM, KEY_B_X, KEY_B_Y, dpop_claims("GET", &url, "proof-2", Some(&ath)));
    let wrong_key = server.request(&format!("GET /api/report HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {BOUND_TOKEN}\r\nDPoP: {wrong_key_proof}\r\nConnection: close\r\n\r\n"));
    assert_eq!(status_of(&wrong_key), 401, "a proof signed by a key other than the one the token was bound to must be rejected: {wrong_key}");

    let unbound_proof = make_proof(KEY_A_PEM, KEY_A_X, KEY_A_Y, dpop_claims("GET", &url, "proof-3", Some(&access_token_hash("some-other-unbound-token"))));
    let unbound = server.request(&format!("GET /api/report HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer some-other-unbound-token\r\nDPoP: {unbound_proof}\r\nConnection: close\r\n\r\n"));
    assert_eq!(status_of(&unbound), 401, "a token with no cnf.jkt binding at all must be rejected even with a valid proof attached: {unbound}");
}

/// An ordinary anonymous request (no `Authorization` header) is
/// untouched by DPoP enforcement entirely -- a different, unrelated
/// gate (role/claim proofs, or simply an ungated route as used here)
/// decides whether it's allowed, not this check.
#[test]
fn requests_with_no_bearer_token_at_all_are_unaffected_by_dpop() {
    let server = Server::start(router());
    let host = format!("127.0.0.1:{}", server.addr.port());
    let resp = server.request(&format!("GET /api/report HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"));
    assert_eq!(status_of(&resp), 200, "body: {resp}");
}
