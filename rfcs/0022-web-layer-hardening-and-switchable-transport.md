# RFC 0022: Web-layer hardening for `nirdosha-rt` — cookies, CORS, rate limiting, and a switchable async/sync transport

> Status: **shipped, 2026-09-17**. `crates/nirdosha-rt/src/web.rs` (the
> live `Router`, not `crates/compiler`'s deprecated `nirdosha build
> --serve` / `crates/compiled-serve`) now has: `Secure`/`SameSite=Strict`
> session cookies, real non-wildcard CORS (`with_cors`), fixed-window
> per-peer-IP rate limiting (`with_rate_limit`), and a switchable
> `serve_until` transport (`Runtime::Async` default, `Runtime::Sync`
> opt-out via `with_runtime`). All four are real, tested end-to-end over
> real sockets (`crates/nirdosha-rt/tests/router_concurrency.rs`), not
> just unit-tested in isolation. Claims-based authorization (a
> `ClaimProof` sibling to `role.rs`'s `RoleProof<R>`) and a real DPoP/
> RFC 9449 port onto this router are explicitly **not** part of this
> RFC — see "Follow-up work" below.
>
> Cross-references:
> - `docs/ROADMAP.md`'s 2026-09-17 deprecation note — `crates/compiler`
>   and everything downstream of it (`crates/compiled-serve`,
>   `crates/runtime-kernels`'s DPoP/OIDC wiring as *that* crate's own
>   caller) is deprecated in favor of the v2 Rust dialect
>   (`docs/nirdosha-rt-dialect.md`). This RFC's own work happened on the
>   live path for exactly that reason — see "Why not `compiled-serve`"
>   below for how that shaped the design.
> - `gaps.md` — the v2-dialect gap inventory this RFC closes four rows
>   of (§3.1 "Real authority boundary," `web.rs`'s previously-missing
>   CORS/rate-limit/TLS-adjacent hardening) and leaves two open (claims-
>   based identity, TLS termination).
> - `crates/compiled-serve/src/ratelimit.rs`,
>   `crates/compiled-serve/src/lib.rs`'s CORS functions — the reference
>   implementations this RFC ports logic from (not code — see below).

## Motivation

A real pentest against any app built on `nirdosha-rt`'s `web::Router`
would have found, before this RFC: a session cookie with no `Secure`/
`SameSite` attributes (session-fixation/CSRF-adjacent, the kind of
finding an automated scanner flags in the first minute); no CORS
handling at all, meaning any cross-origin `fetch()` targeting a
deployment behind a permissive reverse proxy had no origin check to
rely on from the app itself; no rate limiting, so `with_login`'s own
login endpoint had no bound on brute-force throughput; and no way to
run the accept loop on anything but one OS thread per connection.

None of this was a deliberate, disclosed design limit the way (for
example) this router's plain-HTTP-only posture is (TLS termination is
still, and remains, a deployer's reverse proxy's job — see "Follow-up
work"). It was simply never built, because the router's own
development had been focused on route dispatch, sessions, and RBAC
(`role.rs`'s `RoleProof<R>`), not the transport-adjacent hardening a
real deployment needs. `crates/compiled-serve` (the deprecated
`.nir`-compiler path) already had working, tested versions of the
cookie/CORS/rate-limit logic — this RFC ports that *logic*, rewritten
against `web.rs`'s own `Request`/`Response` types, onto the router that
is actually still being built on.

The fourth piece — a switchable transport — came out of a direct
disagreement mid-session: `web.rs`'s synchronous, thread-per-connection
`serve_until` was initially described as an intentional design choice
(paralleling the dialect's own real, disclosed rejection of `async fn`
in *application* code, since the driver can't yet verify contracts over
async control flow). The correction: it wasn't a decision, it was just
what got built first. Most apps this dialect targets are I/O-bound
(waiting on a database, an upstream HTTP call, a long-poll like
`communication_feed!`'s own), which is exactly the shape where a
thread-per-connection model has real, avoidable per-connection
overhead — and none of that requires touching the dialect's own
async-verification gap at all, because a route handler can stay an
ordinary sync closure regardless of what accept loop calls it.

## Why not `compiled-serve`

The obvious shortcut — reuse `compiled-serve`'s own already-tested
`RateLimiter`/CORS code directly, maybe even move it into a shared
crate both paths depend on — was considered and rejected mid-session,
after DPoP/FAPI hardening work was mistakenly done *in*
`compiled-serve` first and had to be reverted. `crates/compiler` (and
everything downstream: `compiled-serve`, and `runtime-kernels`'s own
role as `compiled-serve`'s FFI kernel) was marked deprecated in
`docs/ROADMAP.md` on the same day this work happened, in favor of the
v2 Rust dialect. Building anything new on a deprecated path, even by
reusing its code as a dependency, would mean the work ships on a
component with no future. Every piece in this RFC instead re-implements
the same *logic* natively in `web.rs`'s own module tree
(`crates/nirdosha-rt/src/web/ratelimit.rs`, `Router`'s own CORS
methods), reusing `compiled-serve`'s design (the fixed-window-per-IP
shape, the non-wildcard origin-reflection shape) as prior art, not as a
dependency.

`runtime-kernels` itself is not deprecated — `nirdosha-rt`'s own
`native` feature still depends on it for real FFI kernels (`db`,
`transact`, `nir_oidc_validate_token`, `nir_dpop_verify`, etc.), and
that dependency is unaffected by this RFC or by `compiled-serve`'s
deprecation. Only `compiled-serve` itself (the HTTP server built
*around* those kernels for the deprecated compiler's `nirdosha build
--serve`) has no future — see "Follow-up work" for what that means for
a future DPoP port.

## What shipped

### Session cookie hardening

`SESSION_COOKIE_ATTRS = "HttpOnly; Secure; SameSite=Strict; Path=/"`
replaces the previous bare `HttpOnly; Path=/` on both the login-minted
cookie and the logout-clearing one. `Secure` assumes the same disclosed
deployment model the rest of this router already has: `web.rs` speaks
plain HTTP itself; TLS, if any, terminates at a reverse proxy in front
of it. A browser talking to this process directly over plain HTTP
would never see the cookie sent back at all — a real, disclosed limit
of a from-scratch HTTP/1.1 listener with no TLS of its own, not new to
this RFC.

Tested: `session_cookie_carries_secure_and_samesite_attributes` (both
the login-minted and logout-clearing cookie strings).

### CORS

`Router::with_cors(origins: Vec<&'static str>)` — empty (never called)
means no CORS headers are ever emitted, the same as before this RFC.
Configured, a request from one of `origins` gets that exact origin
reflected back plus `Access-Control-Allow-Credentials: true`; any other
origin gets no CORS headers at all, which every browser treats as a
hard deny. Never a wildcard — a credentialed response (this router
always sends one, since every gated route relies on the session
cookie) reflecting `*` would let any origin ride an authenticated
user's session. `OPTIONS` preflights are answered before route
dispatch; origin matching normalizes each side's own default port
(`https://x` ≡ `https://x:443`).

Tested: `cors_reflects_only_a_configured_origin_never_a_wildcard`,
`no_cors_configured_means_no_cors_headers_at_all`.

### Rate limiting

`Router::with_rate_limit(paths, max_per_window, window)` — a fixed-
window, per-peer-IP limiter (`web/ratelimit.rs`, ported from
`compiled-serve/src/ratelimit.rs`), most relevant to `with_login`'s own
login path but takes an arbitrary path list. **Scope, disclosed, not a
gap**: this only ever triggers through `serve_until`'s real accept
loop, which is the one source of a real peer IP — `Router::dispatch`
called directly (this crate's own unit tests, or an app embedding
`Router` in its own server loop) never rate-limits. It is also, like
`compiled-serve`'s own limiter, per-process only; a fleet-wide limiter
would need a shared store and is real, separate follow-up work, not
silently assumed away.

Tested end-to-end over real sockets:
`rate_limit_denies_past_the_configured_max_from_the_same_peer_and_leaves_other_paths_untouched`
(`router_concurrency.rs`), plus the ported unit tests in
`web/ratelimit.rs` itself (window reset, per-IP independence, the
bounded-tracked-IPs eviction property).

### Switchable transport (`Runtime::Async` / `Runtime::Sync`)

`Router::with_runtime(Runtime)`; `Router::new` defaults to
`Runtime::Async`. Neither variant changes what a route handler looks
like — still an ordinary sync `Fn(&Request, &PathParams) -> Response`
either way, and the dialect's own driver keeps rejecting `async fn` in
application code regardless of which transport a given `Router` runs
under. Only the accept loop underneath differs:

- **`Runtime::Sync`** (`serve_until_sync`): the original implementation,
  unchanged — one OS thread per accepted connection, bounded by
  `ServeConfig::max_connections`.
- **`Runtime::Async`** (`serve_until_async`, new): a tokio-based accept
  loop. A `tokio::sync::Semaphore` sized to `max_connections` replaces
  the thread-count check; each accepted connection becomes a `tokio`
  task; the one handler call per request goes through
  `tokio::task::spawn_blocking` rather than being awaited inline —
  necessary because handlers can legitimately block for a real amount
  of time (`communication_feed!`'s long-poll wait is exactly that
  shape), and awaiting one inline would stall that reactor thread's
  other work. `Response::write_to`'s wire-format logic was extracted
  into `Response::wire_string()` so both transports serialize a
  response identically from one definition.

  Contract parity with the sync path, both disclosed and tested: an
  over-capacity connection is accepted then closed without being
  handled (never left sitting unaccepted in the kernel backlog until
  some unrelated timeout — a caller depending on a prompt, explicit
  close needs exactly this, and the existing
  `slow_handler_does_not_block_other_routes_and_capacity_is_bounded`
  test enforces it); shutdown stops accepting first, then every
  in-flight task is awaited before `serve_until` returns; a panicking
  handler closes the connection with no response and never takes the
  server down (a panicking `spawn_blocking` task surfaces as an `Err`
  on its `JoinHandle`, the same isolation `catch_unwind` gives the sync
  path).

  `tokio` sits behind this crate's own `async-runtime` feature,
  default-on (mirroring the precedent `nirdosha-hi`'s `native-window`
  feature already set for `wry`/`tao`: a real, usually-wanted
  dependency, but not a mandatory one for a build with no use for it).
  A `--no-default-features` build that still requests `Runtime::Async`
  (including via `Router::new`'s own default) gets a real, honest `Err`
  naming the missing feature — never a silent downgrade to `Sync`, and
  never a panic.

Tested: every pre-existing test in `router_concurrency.rs` (seven
tests spanning worker isolation, panic recovery, bounded admission,
concurrent sessions, and `communication_feed!`'s long-poll) now runs
against the async transport by default, unmodified, and passes — the
real proof that the two transports are behaviorally equivalent from a
caller's perspective, not just independently plausible. Plus:
`router_defaults_to_the_async_runtime_and_with_runtime_overrides_it`
(unit), `with_runtime_sync_opt_out_still_serves_real_requests_over_real_sockets`
(integration, real sockets), and
`async_runtime_without_the_feature_is_a_clear_error_not_a_panic` (only
compiled under `--no-default-features`).

## Follow-up work (real gaps, disclosed, not started)

- **Claims-based authorization.** `gaps.md` §3.1: `role.rs`'s
  `RoleProof<R>` has no claims-based sibling (`ClaimProof`) at all — a
  gated route can require a role, never an arbitrary claim. Next work.
- **DPoP / RFC 9449 on this router.** The FAPI 2.0 sender-constrained-
  token work from earlier in this session's history was built and
  proven against `compiled-serve` (real end-to-end tests: DPoP-bound
  demo login, replay rejection, wrong-key rejection), then reverted
  when `crates/compiler`'s deprecation surfaced — none of that logic
  has been ported to `web.rs` yet. A real port needs `nir_dpop_verify`/
  `nir_oidc_validate_token` (`runtime-kernels`, not deprecated) wired
  into `web.rs`'s own `Auth`/session model, which has a materially
  different shape (cookie-based sessions plus an app-supplied
  `authenticate` closure) than `compiled-serve`'s bearer-token-only
  model — not a drop-in port.
- **TLS termination.** Unchanged, disclosed limit: `web.rs` speaks
  plain HTTP only; a deployer's reverse proxy is assumed for TLS, the
  same posture `compiled-serve` always had.
- **Fleet-wide rate limiting.** The shipped limiter is per-process, by
  design, same as `compiled-serve`'s own. A shared-store (e.g. Redis)
  version for multi-instance deployments is real, separate work.
- **Security headers** (`Content-Security-Policy`, `X-Frame-Options`,
  `Strict-Transport-Security`, etc.) on `web.rs`'s HTML responses were
  out of this RFC's scope entirely — not evaluated, not disclosed as
  "considered and deferred," genuinely not looked at yet.
