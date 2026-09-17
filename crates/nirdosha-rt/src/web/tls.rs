//! Optional TLS termination for `serve_until` — `rustls` for the sync
//! transport (`handle_sync_connection`, below) and `tokio-rustls` for
//! the async one (`acceptor`, used from `serve_until_async`'s own TLS
//! branch alongside the shared `serve_one_async` generic over any
//! `AsyncRead + AsyncWrite` stream). `Router::tls: None` (the default)
//! means this process speaks plain HTTP only and TLS, if any, is a
//! deployer's reverse proxy's job — unchanged default behavior;
//! `Router::with_tls` opts a given router into terminating TLS itself
//! instead.

use std::io::{Read, Write};
use std::sync::Arc;

pub struct TlsConfig {
    server_config: Arc<rustls::ServerConfig>,
}

/// `rustls` 0.23's process-level `CryptoProvider` has to be selected
/// exactly once — auto-detection only works when exactly one
/// crypto-backend feature (`ring`, here) is active across the *whole*
/// build's dependency graph, which this crate can't guarantee: another
/// dependency (e.g. a Postgres client pulled in by `nirdosha-rt`'s own
/// `native` feature) may also depend on `rustls` with a different
/// backend feature enabled, and Cargo unifies features across the
/// build. Installing explicitly, once, removes the ambiguity outright
/// rather than hoping feature unification never adds a second backend.
fn ensure_crypto_provider() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

impl TlsConfig {
    /// Builds a real `rustls::ServerConfig` from a PEM-encoded
    /// certificate chain and private key — the two byte strings a real
    /// deployment's cert/key files already contain, read by the caller
    /// (this crate has no opinion on where they live — ACME, a mounted
    /// secret, a local file — matching `Router`'s own "no opinion on
    /// token formats" posture for identity).
    pub fn from_pem(cert_pem: &[u8], key_pem: &[u8]) -> Result<Self, String> {
        ensure_crypto_provider();
        let certs: Vec<rustls::pki_types::CertificateDer<'static>> =
            rustls_pemfile::certs(&mut &*cert_pem).collect::<Result<_, _>>().map_err(|e| format!("invalid TLS certificate PEM: {e}"))?;
        if certs.is_empty() {
            return Err("no certificate found in the supplied PEM".to_string());
        }
        let key = rustls_pemfile::private_key(&mut &*key_pem)
            .map_err(|e| format!("invalid TLS private key PEM: {e}"))?
            .ok_or_else(|| "no private key found in the supplied PEM".to_string())?;
        let server_config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .map_err(|e| format!("invalid TLS certificate/key pair: {e}"))?;
        Ok(TlsConfig { server_config: Arc::new(server_config) })
    }
}

/// One TLS-terminated request/response cycle over `stream`, the sync
/// transport's own shape: a real handshake (driven by the first
/// read/write against `rustls::Stream`), one read's worth of request
/// bytes (matching the plain-HTTP sync path's own single-read
/// semantics), `dispatch` to get a response, one write, then the
/// connection closes on drop. `dispatch` is a closure rather than this
/// module reaching back into `Router` directly, so this module never
/// needs to know `Router`'s own internals — the same reason `check_dpop`
/// hands `dpop.rs` plain strings instead of a `Request`.
pub fn handle_sync_connection(tls: &TlsConfig, mut stream: std::net::TcpStream, dispatch: impl FnOnce(&super::Request) -> super::Response) {
    let mut conn = match rustls::ServerConnection::new(tls.server_config.clone()) {
        Ok(c) => c,
        Err(_) => return, // malformed server config -- can't happen once `TlsConfig::from_pem` has already succeeded, but never a panic either way
    };
    let mut tls_stream = rustls::Stream::new(&mut conn, &mut stream);
    let mut buf = [0u8; 65536];
    let n = match tls_stream.read(&mut buf) {
        Ok(n) if n > 0 => n,
        _ => return, // handshake failed, or the peer closed before sending a request -- close with no response
    };
    let raw = String::from_utf8_lossy(&buf[..n]).into_owned();
    let response = match super::Request::parse(&raw) {
        Some(req) => dispatch(&req),
        None => super::Response::bad_request("malformed request"),
    };
    let _ = tls_stream.write_all(response.wire_string().as_bytes());
}

#[cfg(feature = "async-runtime")]
pub fn acceptor(tls: &TlsConfig) -> tokio_rustls::TlsAcceptor {
    tokio_rustls::TlsAcceptor::from(tls.server_config.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    // A fixed, checked-in, self-signed test certificate/key pair —
    // test-only, never used for anything real. Generated with:
    // `openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1
    // -keyout key.pem -out cert.pem -days 3650 -nodes -subj "/CN=localhost"`
    const TEST_CERT_PEM: &[u8] = include_bytes!("testdata/tls_test_cert.pem");
    const TEST_KEY_PEM: &[u8] = include_bytes!("testdata/tls_test_key.pem");

    #[test]
    fn a_well_formed_cert_and_key_build_a_real_server_config() {
        assert!(TlsConfig::from_pem(TEST_CERT_PEM, TEST_KEY_PEM).is_ok());
    }

    #[test]
    fn a_malformed_cert_is_a_real_error_not_a_panic() {
        assert!(TlsConfig::from_pem(b"not a certificate", TEST_KEY_PEM).is_err());
    }

    #[test]
    fn a_malformed_key_is_a_real_error_not_a_panic() {
        assert!(TlsConfig::from_pem(TEST_CERT_PEM, b"not a key").is_err());
    }

    #[test]
    fn an_empty_cert_is_a_real_error_not_a_panic() {
        assert!(TlsConfig::from_pem(b"", TEST_KEY_PEM).is_err());
    }
}
