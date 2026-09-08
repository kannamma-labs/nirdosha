//! Real connection pooling for `http_get`/`http_post`/`https_get`/
//! `https_post`, replacing the previous connection-per-call,
//! `Connection: close` + read-to-EOF design (`lib.rs`'s original
//! `do_http`/`do_https`) with real HTTP/1.1 keep-alive.
//!
//! **This is a genuine protocol rewrite, not just "add a pool in
//! front of the old code" — the two are inseparable.** Pooling only has
//! real value if a connection survives past one request; under
//! `Connection: close`, the server tears the socket down the instant
//! the response finishes, so every pooled "reuse" would immediately
//! fail validation and rehydrate — a pool that always misses, which is
//! not a real pool. Making pooling real required replacing
//! "read until the peer closes the socket" (which *cannot* work under
//! keep-alive — there is no close to read until) with a real,
//! incremental reader that knows the response is complete from
//! `Content-Length`/chunked framing alone, exactly the way every real
//! HTTP client has to.
//!
//! Two independent managers/registries, same reason `kernel::db` has
//! two — `TcpStream` and `native_tls::TlsStream<TcpStream>` are
//! different Rust types, so `r2d2::ManageConnection`'s one associated
//! `Connection` type per manager needs one manager each.
//!
//! **The server can still say `Connection: close` even though we asked
//! for keep-alive** — a real HTTP/1.1 server's prerogative, common
//! after N requests or under load-shedding. `HttpConn`/`HttpsConn`
//! carry a `should_close` flag, set from the *response's* own headers
//! (not assumed from the request), read by `ManageConnection::has_broken`
//! — r2d2 checks that on checkin and drops the connection instead of
//! returning it to the idle pool, so a server-initiated close is
//! honored, never silently ignored.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Once, OnceLock};

use r2d2::ManageConnection;

use super::pool::{self, PoolConfig, PoolRegistry};

pub struct HttpParsed {
    pub status: i64,
    pub body: String,
}

/// `r2d2::ManageConnection::Error` requires a real `std::error::Error`
/// impl -- a bare `String` doesn't have one, so this is the smallest
/// possible wrapper, not a richer error type this module needs.
#[derive(Debug)]
pub struct HttpError(String);

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for HttpError {}

impl From<String> for HttpError {
    fn from(s: String) -> Self {
        HttpError(s)
    }
}

// ---- a minimal incremental reader over "whatever's already buffered,
// then the live stream" — the piece that makes Content-Length/chunked
// framing possible without ever needing to read to EOF. ----------------

struct StreamReader<'a, S: Read> {
    stream: &'a mut S,
    // Bytes already pulled off the wire while scanning for the header
    // terminator, not yet consumed by the caller -- read from here
    // first, before touching the stream again.
    buffered: Vec<u8>,
    pos: usize,
}

impl<'a, S: Read> StreamReader<'a, S> {
    fn new(stream: &'a mut S, prefix: Vec<u8>) -> Self {
        StreamReader { stream, buffered: prefix, pos: 0 }
    }

    fn fill_more(&mut self) -> Result<usize, String> {
        let mut chunk = [0u8; 8192];
        let n = self.stream.read(&mut chunk).map_err(|e| e.to_string())?;
        if n > 0 {
            self.buffered.extend_from_slice(&chunk[..n]);
        }
        Ok(n)
    }

    /// Reads and consumes one `\r\n`-terminated line (without the
    /// terminator) -- a chunk-size line or a trailer header line.
    fn read_line(&mut self) -> Result<String, String> {
        loop {
            if let Some(rel) = self.buffered[self.pos..].windows(2).position(|w| w == b"\r\n") {
                let line = String::from_utf8_lossy(&self.buffered[self.pos..self.pos + rel]).to_string();
                self.pos += rel + 2;
                return Ok(line);
            }
            if self.fill_more()? == 0 {
                return Err("connection closed before a line terminator arrived".to_string());
            }
        }
    }

    /// Reads and consumes exactly `n` bytes.
    fn read_exact_n(&mut self, n: usize) -> Result<Vec<u8>, String> {
        while self.buffered.len() - self.pos < n {
            if self.fill_more()? == 0 {
                return Err("connection closed before the expected number of bytes arrived".to_string());
            }
        }
        let out = self.buffered[self.pos..self.pos + n].to_vec();
        self.pos += n;
        Ok(out)
    }

    /// Reads until the peer closes the connection -- the one case
    /// where that's actually correct: no `Content-Length`, not
    /// chunked, and the response headers themselves said `Connection:
    /// close`, so there is no "next response" this could ever
    /// over-read into.
    fn read_to_close(&mut self) -> Result<Vec<u8>, String> {
        loop {
            if self.fill_more()? == 0 {
                break;
            }
        }
        Ok(self.buffered[self.pos..].to_vec())
    }

    /// Whatever's left in the internal buffer past everything actually
    /// consumed -- a single `read()` off a real socket (or, in a test,
    /// a `Cursor`) can and does return more bytes than one response
    /// needs, e.g. the start of the *next* response already sitting in
    /// the same TCP segment. Those bytes must be handed back to the
    /// caller and threaded into the next `read_http_response` call on
    /// this same connection, not silently dropped when this
    /// `StreamReader` goes out of scope -- exactly the bug this method
    /// exists to prevent (found by this module's own boundary tests,
    /// not designed in advance).
    fn take_leftover(self) -> Vec<u8> {
        self.buffered[self.pos..].to_vec()
    }
}

/// Same framing `lib.rs`'s original `decode_chunked_body` decoded from
/// an already-fully-buffered blob (kept for that non-streaming use, if
/// any remains) -- this is the streaming twin, reading chunk-size
/// lines and chunk bodies directly off `StreamReader` so it stops
/// exactly at the terminating chunk + trailer blank line, never
/// touching a byte that belongs to the *next* response on a
/// keep-alive connection.
fn read_chunked_body<S: Read>(r: &mut StreamReader<S>) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        let size_line = r.read_line()?;
        let size_hex = size_line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_hex, 16).map_err(|_| format!("malformed chunked body: bad chunk size `{size_hex}`"))?;
        if size == 0 {
            // Trailer headers, terminated by a blank line -- ignored,
            // same "don't need them" scope the original decoder had,
            // but they still must be *consumed* so the stream is left
            // positioned at the start of the next response.
            loop {
                let line = r.read_line()?;
                if line.is_empty() {
                    break;
                }
            }
            break;
        }
        let chunk = r.read_exact_n(size)?;
        out.extend_from_slice(&chunk);
        let crlf = r.read_exact_n(2)?;
        if crlf != b"\r\n" {
            return Err("malformed chunked body: missing CRLF after chunk data".to_string());
        }
    }
    Ok(out)
}

fn parse_headers(head_str: &str) -> Result<(i64, bool, Option<usize>, bool), String> {
    let mut lines = head_str.lines();
    let status_line = lines.next().ok_or_else(|| "malformed HTTP response: empty status line".to_string())?;
    let mut parts = status_line.splitn(3, ' ');
    let version = parts.next().unwrap_or("");
    let status_str = parts.next().ok_or_else(|| "malformed HTTP response: no status code in status line".to_string())?;
    let status: i64 = status_str.parse().map_err(|_| format!("malformed HTTP response: bad status code `{status_str}`"))?;

    let mut content_length: Option<usize> = None;
    let mut is_chunked = false;
    let mut connection_header: Option<String> = None;
    for line in lines {
        let Some((k, v)) = line.split_once(':') else { continue };
        let k = k.trim().to_ascii_lowercase();
        let v = v.trim();
        match k.as_str() {
            "content-length" => content_length = v.parse().ok(),
            "transfer-encoding" if v.eq_ignore_ascii_case("chunked") => is_chunked = true,
            "connection" => connection_header = Some(v.to_ascii_lowercase()),
            _ => {}
        }
    }
    // HTTP/1.1 defaults to persistent; HTTP/1.0 defaults to close.
    let peer_wants_close = match connection_header.as_deref() {
        Some("close") => true,
        Some("keep-alive") => false,
        _ => !version.contains("1.1"),
    };
    Ok((status, is_chunked, content_length, peer_wants_close))
}

/// Reads one complete HTTP response from `stream`, real
/// `Content-Length`/chunked-aware framing — never *processes* a single
/// byte past the end of *this* response as part of it. `prefix` is any
/// bytes already pulled off this same connection by a previous call
/// (see `take_leftover`'s own doc comment for why this exists: a single
/// underlying `read()` can return more than one response's worth of
/// bytes at once, e.g. the next response arriving in the same TCP
/// segment). Returns the parsed response, whether the peer wants this
/// connection closed, and any leftover bytes the *caller* must carry
/// into the next call on this connection — silently discarding them
/// (as an early version of this function did) corrupts the next
/// response's framing.
fn read_http_response(stream: &mut impl Read, prefix: Vec<u8>) -> Result<(HttpParsed, bool, Vec<u8>), String> {
    let mut header_buf = prefix;
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        if let Some(pos) = header_buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos;
        }
        let n = stream.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("connection closed before headers completed".to_string());
        }
        header_buf.extend_from_slice(&chunk[..n]);
        if header_buf.len() > 64 * 1024 {
            return Err("response headers exceeded 64KiB".to_string());
        }
    };
    let head_str = std::str::from_utf8(&header_buf[..header_end]).map_err(|_| "malformed HTTP response: headers are not valid UTF-8".to_string())?.to_string();
    let (status, is_chunked, content_length, peer_wants_close) = parse_headers(&head_str)?;
    let already_read_body = header_buf[header_end + 4..].to_vec();

    let mut r = StreamReader::new(stream, already_read_body);
    let body_bytes = if is_chunked {
        read_chunked_body(&mut r)?
    } else if let Some(len) = content_length {
        r.read_exact_n(len)?
    } else if peer_wants_close {
        r.read_to_close()?
    } else {
        // No Content-Length, not chunked, peer isn't closing --
        // the standard shape of a body-less response (204/304/HEAD)
        // under a persistent connection.
        Vec::new()
    };
    let leftover = r.take_leftover();
    let body = String::from_utf8(body_bytes).map_err(|_| "HTTP response body is not valid UTF-8".to_string())?;
    Ok((HttpParsed { status, body }, peer_wants_close, leftover))
}

fn request_bytes(method: &str, host: &str, path: &str, body: Option<&str>, bearer_token: Option<&str>) -> Vec<u8> {
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: keep-alive\r\n");
    if let Some(token) = bearer_token {
        req.push_str(&format!("Authorization: Bearer {token}\r\n"));
    }
    match body {
        Some(b) => req.push_str(&format!("Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{b}", b.len())),
        None => req.push_str("\r\n"),
    }
    req.into_bytes()
}

// ---- plain HTTP ---------------------------------------------------------

pub struct HttpConn {
    stream: TcpStream,
    should_close: bool,
    /// Bytes over-read past the previous response's boundary on this
    /// same connection (`read_http_response`'s own doc comment) --
    /// threaded into the next request's response read instead of being
    /// dropped.
    leftover: Vec<u8>,
}

pub struct HttpManager {
    host: String,
    port: u16,
}

impl ManageConnection for HttpManager {
    type Connection = HttpConn;
    type Error = HttpError;

    fn connect(&self) -> Result<Self::Connection, Self::Error> {
        let stream = TcpStream::connect((self.host.as_str(), self.port)).map_err(|e| e.to_string())?;
        Ok(HttpConn { stream, should_close: false, leftover: Vec::new() })
    }

    /// A real liveness check for plain TCP -- a non-blocking peek. An
    /// idle, healthy connection has nothing to read yet (`WouldBlock`);
    /// `Ok(0)` means the peer already closed it (the exact staleness
    /// case this exists to catch); any actual data on a connection
    /// that should be idle is unexpected enough to treat as broken too.
    fn is_valid(&self, conn: &mut Self::Connection) -> Result<(), Self::Error> {
        conn.stream.set_nonblocking(true).map_err(|e| e.to_string())?;
        let mut buf = [0u8; 1];
        let result = match conn.stream.peek(&mut buf) {
            Ok(0) => Err("connection closed by peer".to_string()),
            Ok(_) => Err("unexpected data on an idle connection".to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(()),
            Err(e) => Err(e.to_string()),
        };
        let _ = conn.stream.set_nonblocking(false);
        if result.is_err() {
            super::record_stale_rehydrated(super::domain::http());
        }
        result.map_err(HttpError)
    }

    fn has_broken(&self, conn: &mut Self::Connection) -> bool {
        conn.should_close
    }
}

fn http_pool_registry() -> &'static PoolRegistry<HttpManager> {
    static REGISTRY: OnceLock<PoolRegistry<HttpManager>> = OnceLock::new();
    static REGISTERED: Once = Once::new();
    let registry = REGISTRY.get_or_init(PoolRegistry::new);
    // RFC 0011 §5: every pool-backed registry registers itself with the
    // reaper once, lazily, the first time it's reached.
    REGISTERED.call_once(|| pool::register_for_reaping(registry));
    registry
}

// ---- HTTPS ---------------------------------------------------------------

pub struct HttpsConn {
    stream: native_tls::TlsStream<TcpStream>,
    should_close: bool,
    leftover: Vec<u8>,
}

pub struct HttpsManager {
    host: String,
    port: u16,
}

impl ManageConnection for HttpsManager {
    type Connection = HttpsConn;
    type Error = HttpError;

    fn connect(&self) -> Result<Self::Connection, Self::Error> {
        let tcp = TcpStream::connect((self.host.as_str(), self.port)).map_err(|e| e.to_string())?;
        let connector = native_tls::TlsConnector::new().map_err(|e| e.to_string())?;
        let stream = connector.connect(&self.host, tcp).map_err(|e| e.to_string())?;
        Ok(HttpsConn { stream, should_close: false, leftover: Vec::new() })
    }

    /// No pre-send liveness check for TLS -- a raw peek at the
    /// underlying socket would read encrypted record bytes without
    /// going through the TLS session, corrupting it; `native_tls`
    /// exposes no peek-without-consuming operation through an active
    /// session. The real defense here is `has_broken` (server-declared
    /// close, honored below) plus the caller's own at-most-once,
    /// retry-only-if-provably-unsent rule on an actual send failure --
    /// disclosed as the real mechanism, not silently assumed covered
    /// by a check that doesn't exist for this transport.
    fn is_valid(&self, _conn: &mut Self::Connection) -> Result<(), Self::Error> {
        Ok(())
    }

    fn has_broken(&self, conn: &mut Self::Connection) -> bool {
        conn.should_close
    }
}

fn https_pool_registry() -> &'static PoolRegistry<HttpsManager> {
    static REGISTRY: OnceLock<PoolRegistry<HttpsManager>> = OnceLock::new();
    static REGISTERED: Once = Once::new();
    let registry = REGISTRY.get_or_init(PoolRegistry::new);
    REGISTERED.call_once(|| pool::register_for_reaping(registry));
    registry
}

/// Same reasoning as `kernel::db::db_pool_config` -- `pool.max_size`
/// must stay `≥` the `http` domain's own admission ceiling, so the
/// ceiling (not a smaller pool) stays the one real, observable choke
/// point.
fn http_pool_config() -> PoolConfig {
    let mut cfg = PoolConfig::from_env("HTTP");
    if cfg.max_size < 1000 {
        cfg.max_size = 1000;
    }
    cfg
}

fn pool_key(host: &str, port: i64) -> String {
    format!("{host}:{port}")
}

/// Whether a failed attempt is safe to retry: the write itself failing
/// means the request bytes provably never reached the server (nothing
/// was sent, so nothing could have been processed) -- safe to retry
/// once against a fresh connection. A failure while *reading* the
/// response is ambiguous (the server may already have received and
/// even fully processed a non-idempotent request) and is never retried
/// -- retrying that could double-execute a real `POST`. This is the
/// actual, disclosed at-most-once mechanism `HttpsManager::is_valid`'s
/// own doc comment refers to (no pre-send liveness check for TLS, so
/// this is the real defense for a connection that went stale between
/// checkout and use).
enum SendError {
    WriteFailed(String),
    ReadFailed(String),
}

fn send_and_read(stream: &mut (impl Read + Write), req_bytes: &[u8], leftover: Vec<u8>) -> Result<(HttpParsed, bool, Vec<u8>), SendError> {
    stream.write_all(req_bytes).map_err(|e| SendError::WriteFailed(e.to_string()))?;
    read_http_response(stream, leftover).map_err(SendError::ReadFailed)
}

/// One request, over a pooled, keep-alive connection. Admission
/// (`kernel::acquire(domain::http())`/`release`) is held for exactly the
/// duration of this one call (both attempts, if a retry happens) --
/// unlike `db`'s affine handle, an HTTP client call has no
/// caller-visible session to hold it open across.
pub fn request_http(host: &str, port: i64, path: &str, method: &str, body: Option<&str>, bearer_token: Option<&str>) -> Result<HttpParsed, String> {
    if !super::acquire(super::domain::http()) {
        return Err("too many open http connections".to_string());
    }
    let result = (|| {
        let pool = http_pool_registry().get_or_create(&pool_key(host, port), http_pool_config(), || Ok(HttpManager { host: host.to_string(), port: port as u16 }))?;
        let req_bytes = request_bytes(method, host, path, body, bearer_token);

        let mut conn = pool.get().map_err(|e| e.to_string())?;
        let leftover = std::mem::take(&mut conn.leftover);
        match send_and_read(&mut conn.stream, &req_bytes, leftover) {
            Ok((parsed, peer_wants_close, leftover)) => {
                conn.should_close = peer_wants_close;
                conn.leftover = leftover;
                Ok(parsed)
            }
            Err(SendError::ReadFailed(e)) => Err(e),
            Err(SendError::WriteFailed(_)) => {
                // Provably unsent -- evict this connection (never
                // return a write-failed one to the pool) and retry
                // exactly once against a fresh checkout.
                conn.should_close = true;
                drop(conn);
                let mut conn = pool.get().map_err(|e| e.to_string())?;
                let leftover = std::mem::take(&mut conn.leftover);
                let (parsed, peer_wants_close, leftover) = send_and_read(&mut conn.stream, &req_bytes, leftover).map_err(|e| match e {
                    SendError::WriteFailed(e) | SendError::ReadFailed(e) => e,
                })?;
                conn.should_close = peer_wants_close;
                conn.leftover = leftover;
                Ok(parsed)
            }
        }
    })();
    super::release(super::domain::http());
    result
}

pub fn request_https(host: &str, port: i64, path: &str, method: &str, body: Option<&str>) -> Result<HttpParsed, String> {
    if !super::acquire(super::domain::http()) {
        return Err("too many open http connections".to_string());
    }
    let result = (|| {
        let pool = https_pool_registry().get_or_create(&pool_key(host, port), http_pool_config(), || Ok(HttpsManager { host: host.to_string(), port: port as u16 }))?;
        let req_bytes = request_bytes(method, host, path, body, None);

        let mut conn = pool.get().map_err(|e| e.to_string())?;
        let leftover = std::mem::take(&mut conn.leftover);
        match send_and_read(&mut conn.stream, &req_bytes, leftover) {
            Ok((parsed, peer_wants_close, leftover)) => {
                conn.should_close = peer_wants_close;
                conn.leftover = leftover;
                Ok(parsed)
            }
            Err(SendError::ReadFailed(e)) => Err(e),
            Err(SendError::WriteFailed(_)) => {
                conn.should_close = true;
                drop(conn);
                let mut conn = pool.get().map_err(|e| e.to_string())?;
                let leftover = std::mem::take(&mut conn.leftover);
                let (parsed, peer_wants_close, leftover) = send_and_read(&mut conn.stream, &req_bytes, leftover).map_err(|e| match e {
                    SendError::WriteFailed(e) | SendError::ReadFailed(e) => e,
                })?;
                conn.should_close = peer_wants_close;
                conn.leftover = leftover;
                Ok(parsed)
            }
        }
    })();
    super::release(super::domain::http());
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    // ---- pure parsing, no socket needed -- moved here from lib.rs's
    // old `http_kernel_tests` (they tested the now-deleted buffer-based
    // `parse_http_response`/`decode_chunked_body`; these test the real
    // streaming reader that replaced them, `Cursor` standing in for a
    // live stream since `read_http_response` only needs `impl Read`).

    #[test]
    fn read_http_response_extracts_status_and_body() {
        let mut c = Cursor::new(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 11\r\n\r\nhello world".to_vec());
        let (parsed, close, _) = read_http_response(&mut c, Vec::new()).unwrap();
        assert_eq!(parsed.status, 200);
        assert_eq!(parsed.body, "hello world");
        assert!(!close, "HTTP/1.1 with no explicit Connection: close must default to keep-alive");
    }

    #[test]
    fn read_http_response_handles_empty_body_with_no_content_length() {
        let mut c = Cursor::new(b"HTTP/1.1 204 No Content\r\n\r\n".to_vec());
        let (parsed, _, _) = read_http_response(&mut c, Vec::new()).unwrap();
        assert_eq!(parsed.status, 204);
        assert_eq!(parsed.body, "");
    }

    #[test]
    fn read_http_response_rejects_malformed_input() {
        let mut c = Cursor::new(b"not an http response at all".to_vec());
        assert!(read_http_response(&mut c, Vec::new()).is_err());
    }

    #[test]
    fn read_http_response_decodes_a_real_chunked_body() {
        let mut c = Cursor::new(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n7\r\nMozilla\r\n9\r\nDeveloper\r\n0\r\n\r\n".to_vec());
        let (parsed, _, _) = read_http_response(&mut c, Vec::new()).unwrap();
        assert_eq!(parsed.status, 200);
        assert_eq!(parsed.body, "MozillaDeveloper");
    }

    #[test]
    fn read_http_response_leaves_a_content_length_body_untouched() {
        let mut c = Cursor::new(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello".to_vec());
        assert_eq!(read_http_response(&mut c, Vec::new()).unwrap().0.body, "hello");
    }

    #[test]
    fn read_http_response_reports_explicit_connection_close() {
        let mut c = Cursor::new(b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 2\r\n\r\nok".to_vec());
        let (_, close, _) = read_http_response(&mut c, Vec::new()).unwrap();
        assert!(close, "an explicit Connection: close header must be honored");
    }

    #[test]
    fn read_http_response_defaults_to_close_under_http_1_0() {
        let mut c = Cursor::new(b"HTTP/1.0 200 OK\r\nContent-Length: 2\r\n\r\nok".to_vec());
        let (_, close, _) = read_http_response(&mut c, Vec::new()).unwrap();
        assert!(close, "HTTP/1.0 with no Connection header must default to close");
    }

    /// The actual point of this whole rewrite: reading one
    /// `Content-Length`-framed response must consume *exactly* that
    /// response and not one byte more. A single `Cursor`/socket `read()`
    /// can and does pull back more than one response's worth of bytes
    /// at once (this test's own two responses are small enough to
    /// arrive in one read) -- the leftover bytes from the first call
    /// must be threaded into the second as its `prefix`, exactly as
    /// `request_http`/`request_https` do via `HttpConn::leftover`, or
    /// the second response's framing is silently corrupted. This test
    /// caught exactly that bug (leftover bytes dropped instead of
    /// carried forward) before this fix.
    #[test]
    fn read_http_response_stops_exactly_at_the_boundary_so_a_second_response_parses_cleanly() {
        let mut c = Cursor::new(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nfirstHTTP/1.1 201 Created\r\nContent-Length: 6\r\n\r\nsecond".to_vec());
        let (first, _, leftover) = read_http_response(&mut c, Vec::new()).unwrap();
        assert_eq!(first.status, 200);
        assert_eq!(first.body, "first");
        let (second, _, _) = read_http_response(&mut c, leftover).unwrap();
        assert_eq!(second.status, 201);
        assert_eq!(second.body, "second");
    }

    /// Same boundary correctness, chunked framing this time -- trailer
    /// headers (even the empty case, just the terminating blank line)
    /// must be consumed too, or the next response would start reading
    /// from the middle of a leftover `\r\n`.
    #[test]
    fn read_http_response_stops_exactly_at_the_boundary_for_chunked_bodies_too() {
        let mut c = Cursor::new(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nfirst\r\n0\r\n\r\nHTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nsecond".to_vec());
        let (first, _, leftover) = read_http_response(&mut c, Vec::new()).unwrap();
        assert_eq!(first.body, "first");
        let (second, _, _) = read_http_response(&mut c, leftover).unwrap();
        assert_eq!(second.body, "second");
    }

    #[test]
    fn decode_chunked_body_rejects_truncated_input() {
        let mut c = Cursor::new(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nabc".to_vec());
        assert!(read_http_response(&mut c, Vec::new()).is_err());
    }

    // ---- real sockets: pooling, keep-alive reuse, and server-initiated
    // close, all against a real local TCP server. ----------------------

    /// Proves actual connection *reuse*, not just "no error" -- a
    /// server that counts its own `accept()` calls sees exactly one for
    /// three requests, which is only possible if the second and third
    /// requests really did reuse the first request's still-open
    /// connection rather than opening a fresh one each time.
    #[test]
    fn three_requests_to_the_same_server_reuse_one_connection() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            for i in 0..3 {
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0, "request {i} should have arrived on the reused connection");
                let body = format!("resp{i}");
                stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}", body.len()).as_bytes()).unwrap();
            }
        });
        for i in 0..3 {
            let result = request_http("127.0.0.1", port as i64, "/", "GET", None, None).unwrap();
            assert_eq!(result.status, 200);
            assert_eq!(result.body, format!("resp{i}"));
        }
        server.join().unwrap();
    }

    /// The server's own prerogative: even though every request asks for
    /// keep-alive, a server saying `Connection: close` must not have
    /// its connection reused for the next request -- proven by a fresh
    /// `accept()` for the second request.
    #[test]
    fn a_server_initiated_close_is_honored_not_reused() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            for i in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0, "request {i} should have arrived on a fresh connection");
                let body = format!("resp{i}");
                stream.write_all(format!("HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}", body.len()).as_bytes()).unwrap();
            }
        });
        for i in 0..2 {
            let result = request_http("127.0.0.1", port as i64, "/", "GET", None, None).unwrap();
            assert_eq!(result.body, format!("resp{i}"));
        }
        server.join().unwrap();
    }

    #[test]
    fn http_get_connection_refused_is_a_real_err_not_a_panic() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        assert!(request_http("127.0.0.1", port as i64, "/", "GET", None, None).is_err());
    }

    #[test]
    fn http_post_sends_a_real_body_with_content_length_and_keep_alive_header() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap();
            let received = String::from_utf8_lossy(&buf[..n]).into_owned();
            assert!(received.contains("Content-Length: 11"));
            assert!(received.contains("Connection: keep-alive"));
            assert!(received.ends_with("hello world"));
            stream.write_all(b"HTTP/1.1 201 Created\r\nContent-Length: 7\r\n\r\ncreated").unwrap();
        });
        let result = request_http("127.0.0.1", port as i64, "/submit", "POST", Some("hello world"), None).unwrap();
        server.join().unwrap();
        assert_eq!(result.status, 201);
        assert_eq!(result.body, "created");
    }

    /// `send_and_read`'s actual retry-eligibility decision, tested
    /// directly and deterministically rather than via a real socket
    /// race (a write failing mid-flight vs. a read failing after a
    /// successful write is otherwise hard to force reliably in an
    /// integration test). A write failure must classify as
    /// `WriteFailed` (retry-eligible -- provably unsent); a read
    /// failure after a successful write must classify as `ReadFailed`
    /// (never retried -- the request may already have been processed).
    struct FailingWrite;
    impl Read for FailingWrite {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            Ok(0)
        }
    }
    impl Write for FailingWrite {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "simulated broken pipe"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct FailingReadAfterWrite;
    impl Read for FailingReadAfterWrite {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(std::io::ErrorKind::ConnectionReset, "simulated reset mid-response"))
        }
    }
    impl Write for FailingReadAfterWrite {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_write_failure_classifies_as_retry_eligible() {
        let mut s = FailingWrite;
        match send_and_read(&mut s, b"GET / HTTP/1.1\r\n\r\n", Vec::new()) {
            Err(SendError::WriteFailed(_)) => {}
            other => panic!("expected WriteFailed, got {}", match other { Ok(_) => "Ok".to_string(), Err(SendError::ReadFailed(e)) => format!("ReadFailed({e})"), _ => unreachable!() }),
        }
    }

    #[test]
    fn a_read_failure_after_a_successful_write_is_never_retry_eligible() {
        let mut s = FailingReadAfterWrite;
        match send_and_read(&mut s, b"GET / HTTP/1.1\r\n\r\n", Vec::new()) {
            Err(SendError::ReadFailed(_)) => {}
            other => panic!("expected ReadFailed, got {}", match other { Ok(_) => "Ok".to_string(), Err(SendError::WriteFailed(e)) => format!("WriteFailed({e})"), _ => unreachable!() }),
        }
    }

    /// End-to-end resilience: the server accepts one request, responds
    /// *without* `Connection: close` (so the client believes the
    /// connection is reusable), then closes its side entirely before a
    /// second request ever arrives -- simulating a real, common failure
    /// mode (a load balancer or the peer itself silently dropping an
    /// idle keep-alive connection). The second `request_http` call must
    /// still succeed transparently, whether that's `is_valid`'s
    /// pre-send peek catching it (the common case for plain HTTP) or
    /// the write-retry path -- the caller-visible guarantee under test
    /// is "the second request just works," not which internal path
    /// got there.
    #[test]
    fn a_silently_closed_pooled_connection_does_not_break_the_next_request() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf).unwrap();
                stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nfirst").unwrap();
                // `stream` drops here -- a real close, no Connection:
                // close header was ever sent, so the client had every
                // reason to believe this connection was still good.
            }
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf).unwrap();
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nsecond").unwrap();
        });
        let first = request_http("127.0.0.1", port as i64, "/", "GET", None, None).unwrap();
        assert_eq!(first.body, "first");
        // Give the OS a moment to actually deliver the close before the
        // next checkout's liveness check runs against it.
        std::thread::sleep(std::time::Duration::from_millis(50));
        let second = request_http("127.0.0.1", port as i64, "/", "GET", None, None).unwrap();
        assert_eq!(second.body, "second");
        server.join().unwrap();
    }
}
