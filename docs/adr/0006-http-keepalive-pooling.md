# 0006: `http`/`https` — real keep-alive, pooling, and admission control

Date: 2026-09-07
Status: accepted

## Context

`http_get`/`http_post`/`https_get`/`https_post` already compiled for
real, but opened a fresh `TcpStream`/`TlsStream` per call, sent
`Connection: close`, and read to EOF as its end-of-body signal — a
correct, deliberate cut for its own scope, but two real gaps followed
from it directly: `http_get`/`post` went through **zero admission
control** (no `Domain::Http` existed at all — unlike `tcp`/`file`/`db`),
and the connection-per-call design meant there was nothing a pool could
usefully reuse. Adding real pooling — the explicit ask this phase
started from — turned out to require fixing both, and they're not
independent: a pooled connection only has value if it survives past one
request, and under `Connection: close` the server tears the socket down
the instant the response finishes, so every pooled "reuse" would
immediately fail validation and rehydrate. A pool that always misses
isn't a real pool.

## Decision

**Real HTTP/1.1 keep-alive**, not just a pool bolted onto the old
design: requests now send `Connection: keep-alive`, and the response
reader (`crates/runtime-kernels/src/kernel/http.rs::read_http_response`)
is a real, incremental, `Content-Length`/chunked-aware parser that
never reads a single byte past the end of *one* response — replacing
"read until the peer closes the socket," which cannot work once the
peer might not close it. The server keeps the final say: if a response
declares `Connection: close`, that connection is marked and never
returned to the idle pool (`ManageConnection::has_broken`), regardless
of what the request asked for.

**Pooling**, same shape `kernel::db` already established: one
`PoolRegistry` per transport (`HttpManager`/`HttpsManager` — different
Rust types, `TcpStream` vs. `native_tls::TlsStream<TcpStream>`), keyed
by `host:port`. Liveness validation differs by transport, disclosed
rather than assumed uniform: plain HTTP gets a real non-blocking peek
(`Ok(0)` = peer closed, `WouldBlock` = healthy and idle); HTTPS has no
safe way to peek through an active TLS session without disrupting its
framing, so it relies on the server's declared close (above) plus
at-most-once retry semantics on an actual send failure as its real
defense, not a peek that doesn't exist for this transport.

**Admission control**: a new `Domain::Http` (previously absent
entirely), held for the duration of one request/response — unlike
`db`'s affine handle, an HTTP client call has no caller-visible session
to hold it open across.

## Consequences

Real pooled keep-alive, real admission control, real telemetry
(`stale_rehydrated` for the plain-HTTP peek path) — `http_get`/`post`
now behave like a production HTTP client, not "open a socket and hope,"
the same upgrade Postgres got in `docs/adr/0005`.

**A real bug this design's own tests caught before shipping, not
after**: a single underlying `read()` can return more bytes than one
response needs (the start of the next response, already in the same
TCP segment) — an early version of the reader dropped those bytes when
its local buffer went out of scope, silently corrupting the next
response's framing the moment two responses arrived close together.
Fixed by threading a `leftover: Vec<u8>` buffer through each pooled
connection (`HttpConn`/`HttpsConn`), carried from one request's read
into the next's. The regression tests for this
(`read_http_response_stops_exactly_at_the_boundary_so_a_second_response_parses_cleanly`
and its chunked twin) failed against the first version of this code and
pass now — kept as the permanent guard against this exact class of bug
recurring.

**Honest costs, not hidden**: HTTPS's `is_valid` cannot verify liveness
before sending — a stale TLS connection's staleness is only discovered
by a real send failure, at which point the request has already
consumed one retry attempt (at-most-once, never re-sent once bytes are
actually on the wire, to avoid double-executing a non-idempotent
`POST`). `pool.max_size` must stay `≥` `Domain::Http`'s own ceiling,
same invariant `docs/adr/0005` already states for `db` and for the
identical reason (admission never blocks, `Pool::get()` does).
