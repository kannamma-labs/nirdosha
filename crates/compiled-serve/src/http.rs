//! Minimal, hand-rolled HTTP/1.1 request parsing over a raw
//! [`TcpStream`] — deliberately not `tiny_http` or any other server
//! crate: this crate's whole point is real per-connection socket
//! timeouts (`set_read_timeout` on the stream directly) and a real
//! `413` on an over-large body *before* it's ever fully buffered, both
//! of which need direct control over the raw stream this file already
//! has. `GET`/`POST` only, `Content-Length` bodies only (no chunked
//! transfer-encoding) — the same scope
//! `examples/features/51_compiled_serve.nir`'s own hand-written parser
//! already has, named here rather than silently assumed complete.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// A real `1 MiB` cap, reused verbatim from the deleted interpreter-era
/// `serve.rs`'s own working default (`MAX_BODY_BYTES`) rather than
/// inventing a new number — `rfcs/0010`'s own plan named this as the
/// one existing number worth carrying forward.
pub const MAX_BODY_BYTES: usize = 1024 * 1024;

const MAX_HEADER_BYTES: usize = 16 * 1024;

pub struct Request {
    pub method: String,
    pub path: String,
    headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// HTTP/1.1 defaults to keep-alive unless `Connection: close` is
    /// explicit — this project's HTTP/1.0 support is nonexistent (every
    /// example/test speaks 1.1), so this doesn't special-case 1.0's
    /// opposite default.
    pub fn wants_keep_alive(&self) -> bool {
        !self.header("connection").map(|v| v.eq_ignore_ascii_case("close")).unwrap_or(false)
    }

}

pub enum ReadError {
    TooLarge,
    Timeout,
    Malformed,
    Closed,
}

/// Reads one HTTP/1.1 request off `stream`. `Ok(None)` means the
/// connection closed cleanly (EOF right at a request boundary, or a
/// keep-alive idle timeout — both are "nothing more to read here," not
/// errors) — the caller closes the connection either way, same as any
/// other terminal case.
pub fn read_request(stream: &mut TcpStream, body_timeout: Duration) -> Result<Option<Request>, ReadError> {
    let mut buf = Vec::new();
    let header_end = read_headers(stream, &mut buf)?;
    let Some(header_end) = header_end else { return Ok(None) };

    let head = String::from_utf8_lossy(&buf[..header_end]);
    let mut lines = head.split("\r\n");
    let request_line = lines.next().ok_or(ReadError::Malformed)?;
    let mut parts = request_line.split(' ');
    let method = parts.next().ok_or(ReadError::Malformed)?.to_string();
    let path = parts.next().ok_or(ReadError::Malformed)?.to_string();

    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }

    let content_length: usize = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    if content_length > MAX_BODY_BYTES {
        return Err(ReadError::TooLarge);
    }

    let already_read = buf.len() - (header_end + 4);
    let mut body = buf[header_end + 4..].to_vec();
    if content_length > already_read {
        let _ = stream.set_read_timeout(Some(body_timeout));
        let mut remaining = vec![0u8; content_length - already_read];
        stream.read_exact(&mut remaining).map_err(|_| ReadError::Timeout)?;
        body.extend_from_slice(&remaining);
    } else {
        body.truncate(content_length);
    }

    Ok(Some(Request { method, path, headers, body }))
}

/// Reads until `\r\n\r\n` (the header/body boundary), capping total
/// header bytes at [`MAX_HEADER_BYTES`] — a malicious or broken client
/// sending an unbounded header block must not grow this buffer forever.
/// Returns the byte offset of the `\r\n\r\n` marker's own start.
fn read_headers(stream: &mut TcpStream, buf: &mut Vec<u8>) -> Result<Option<usize>, ReadError> {
    let mut chunk = [0u8; 512];
    loop {
        if let Some(pos) = find_header_end(buf) {
            return Ok(Some(pos));
        }
        if buf.len() >= MAX_HEADER_BYTES {
            return Err(ReadError::TooLarge);
        }
        match stream.read(&mut chunk) {
            Ok(0) => return if buf.is_empty() { Ok(None) } else { Err(ReadError::Malformed) },
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {
                return if buf.is_empty() { Ok(None) } else { Err(ReadError::Timeout) };
            }
            Err(_) => return Err(ReadError::Closed),
        }
    }
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

pub struct Response {
    pub status: u16,
    /// Was hardcoded to `application/json` at the one call site that
    /// actually writes a response (`lib.rs::handle_connection`) —
    /// harmless sloppiness for every route this crate served before
    /// (JSON API responses, and plain-text `/healthz`/`/readyz`/
    /// `/metrics` bodies browsers/`curl` don't care about the label
    /// on), but a real bug for `GET /`'s HTML page (Stage 3 of
    /// reviving compiled `serve`): a browser served real HTML labeled
    /// `application/json` may refuse to render it as a page at all.
    /// Real per-response content type, not a second hardcoded guess.
    pub content_type: &'static str,
    pub body: Vec<u8>,
    pub headers: Vec<(String, String)>,
    pub cookie: Option<String>,
}

impl Response {
    pub fn ok_text(status: u16, text: &str) -> Response {
        Response { status, content_type: "text/plain", body: text.as_bytes().to_vec(), headers: Vec::new(), cookie: None }
    }
    pub fn ok_html(status: u16, html: &[u8]) -> Response {
        Response { status, content_type: "text/html; charset=utf-8", body: html.to_vec(), headers: Vec::new(), cookie: None }
    }
    pub fn error(status: u16, message: &str) -> Response {
        let body = serde_json::json!({"err": message});
        Response { status, content_type: "application/json", body: serde_json::to_vec(&body).unwrap_or_default(), headers: Vec::new(), cookie: None }
    }
}

fn status_text(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Unknown",
    }
}

/// Writes one real HTTP/1.1 response, `Set-Cookie` (`HttpOnly; Secure;
/// SameSite=Lax; Path=/` unconditionally, `rfcs/0010`'s own "Set-Cookie
/// gets actually wired" requirement) included whenever `cookie` is
/// `Some`.
pub fn write_response(stream: &mut TcpStream, status: u16, content_type: &str, body: &[u8], extra_headers: &[(String, String)], cookie: Option<&str>) -> std::io::Result<()> {
    let mut out = format!("HTTP/1.1 {status} {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n", status_text(status), body.len());
    for (k, v) in extra_headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    if let Some(cookie) = cookie {
        out.push_str(&format!("Set-Cookie: {cookie}; HttpOnly; Secure; SameSite=Lax; Path=/\r\n"));
    }
    out.push_str("\r\n");
    stream.write_all(out.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()?;
    Ok(())
}
