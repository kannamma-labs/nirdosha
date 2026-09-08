# nirdosha-compiled-serve

ROADMAP B8's compiled `serve` mode — the dispatch-table framework
`rfcs/0010-landing-and-serve-exposure.md`'s own Status box discloses as
additive alongside (never a replacement for)
`examples/features/51_compiled_serve.nir`'s primitives-first,
hand-written `tcp_listener`/`accept` style.

This crate owns the HTTP transport: a raw-`TcpListener` accept loop,
real per-connection socket timeouts, `Domain::ServeHttp` admission
(fail-fast `503`, never a silent stall), a `413` body-size cap, a
keep-alive/`max_requests_per_connection` policy, CORS (never a wildcard
on a credentialed response), a fixed-window per-IP rate limiter, and a
real `Set-Cookie`. It does **not** own RBAC or JWT verification — every
[`Route`](src/lib.rs)'s `handler` is a plain function pointer
(`RouteHandler`'s own doc comment has the exact ABI), and whatever
`requires(...)` check a route needs happens *inside* that function —
compiled Nirdosha LLVM code this crate only ever calls through a
pointer, never reimplements.

**Not yet wired to `codegen.rs`.** This crate is complete and tested on
its own, against hand-written `extern "C"` test routes
(`src/tests.rs`) — proving the HTTP engine for real without waiting on
the separate, larger effort of having `codegen.rs` emit real per-route
wrapper functions from a compiled program's own exposure set (RFC 0010)
and a `nirdosha build --serve` CLI flag to link this crate in.

Lives in `crates/runtime-kernels`'s own separate Cargo workspace, not
the repo root's — see `docs/adr/0010-runtime-kernels-rlib-for-compiled-serve.md`
for why. Build/test it with:

```sh
cd crates/runtime-kernels
cargo test -p nirdosha-compiled-serve
```
