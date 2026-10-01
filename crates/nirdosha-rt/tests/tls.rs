//! `Router::with_tls`, proved end-to-end -- a real TLS handshake over a
//! real socket, against both transports, with a real (self-signed,
//! test-only) certificate the client actually validates (no disabled
//! verification), not just `web/tls.rs`'s own unit tests of
//! `TlsConfig::from_pem` in isolation.
#![cfg(feature = "tls")]

use nirdosha_rt::web::{Runtime, ServeConfig};
use nirdosha_rt::{Auth, Response, Router};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const TEST_CERT_PEM: &[u8] = include_bytes!("../src/web/testdata/tls_test_cert.pem");
const TEST_KEY_PEM: &[u8] = include_bytes!("../src/web/testdata/tls_test_key.pem");

struct Server {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<std::io::Result<()>>>,
}

impl Server {
    fn start(runtime: Runtime) -> Self {
        let router = Router::new(|_| Auth::login("anon", &[]))
            .with_runtime(runtime)
            .with_tls(TEST_CERT_PEM, TEST_KEY_PEM)
            .expect("the fixed test cert/key must build a real TLS config")
            .get("/", "root", |_, _| Response::text(200, "ok over tls"));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker = thread::spawn(move || router.serve_until(listener, &worker_stop, ServeConfig { max_connections: 8.try_into().unwrap(), io_timeout: Duration::from_millis(500) }));
        Self { addr, stop, worker: Some(worker) }
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

/// A real TLS client handshake against `addr`, trusting the *exact*
/// self-signed test certificate the server presents -- real chain
/// validation against a real root, not `dangerous()`-style disabled
/// verification standing in for "TLS happened."
fn https_get(addr: SocketAddr) -> String {
    let cert_der = rustls_pemfile::certs(&mut &*TEST_CERT_PEM).next().expect("a certificate in the test PEM").expect("a well-formed certificate");
    let mut root_store = rustls::RootCertStore::empty();
    root_store.add(cert_der).expect("the test cert must add as a trusted root");
    let config = rustls::ClientConfig::builder().with_root_certificates(root_store).with_no_client_auth();
    let server_name = rustls::pki_types::ServerName::try_from("localhost").expect("a valid DNS name").to_owned();
    let mut conn = rustls::ClientConnection::new(Arc::new(config), server_name).expect("a valid client connection");
    let mut sock = TcpStream::connect(addr).expect("connect should succeed");
    sock.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    sock.set_write_timeout(Some(Duration::from_secs(3))).unwrap();
    let mut tls = rustls::Stream::new(&mut conn, &mut sock);
    tls.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").expect("write should succeed");
    let mut response = String::new();
    let _ = tls.read_to_string(&mut response);
    response
}

#[test]
fn tls_termination_works_over_a_real_socket_sync_transport() {
    let server = Server::start(Runtime::Sync);
    let resp = https_get(server.addr);
    assert_eq!(status_of(&resp), 200, "response: {resp}");
    assert!(resp.contains("ok over tls"));
    assert!(
        resp.to_ascii_lowercase().contains("strict-transport-security"),
        "HSTS must be present once this process is actually terminating TLS itself: {resp}"
    );
}

#[test]
fn tls_termination_works_over_a_real_socket_async_transport() {
    let server = Server::start(Runtime::Async);
    let resp = https_get(server.addr);
    assert_eq!(status_of(&resp), 200, "response: {resp}");
    assert!(resp.contains("ok over tls"));
}

/// A plain (non-TLS) client speaking directly to a TLS-terminated
/// server never gets a parseable HTTP response — the handshake itself
/// fails, proving this doesn't silently fall back to plaintext.
#[test]
fn a_plain_http_client_cannot_talk_to_a_tls_terminated_server() {
    let server = Server::start(Runtime::Sync);
    let mut sock = TcpStream::connect(server.addr).unwrap();
    sock.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let _ = sock.write_all(b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
    let mut buf = Vec::new();
    let _ = sock.read_to_end(&mut buf);
    let response = String::from_utf8_lossy(&buf);
    assert!(!response.starts_with("HTTP/1.1 200"), "a plaintext request must never be parsed as a valid HTTP request by a TLS-terminated server: {response:?}");
}
