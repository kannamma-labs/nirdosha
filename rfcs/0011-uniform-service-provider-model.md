# RFC 0011: A uniform service-provider model — open admission domains, revived plugin dispatch, and proactive rehydration for any future backend

> **Status.** All 8 phases of the implementation checklist below have
> landed and pass real, compiled-and-run tests — `env(...)`, the opened
> `Domain` registry, eager startup registration, runtime scheme dispatch
> for `db_connect`/the new `call_via` (`mq_connect_via`'s own fallback is
> explicitly deferred, see that checklist item below, not silently
> unfinished), `PoolRegistry<PluginManagedConnection>` with
> credential-stripped keys and a per-provider pool-key cap, the reaper
> (panic containment, shutdown flag, interval validation), `Effect::Env`,
> the plugin ABI's panic/thread-safety contract, and — the closing item —
> one real, shipped `call`-shape plugin
> (`crates/plugin-example-native-authed-http`) built, linked, and run
> end to end against a real local server. Five review passes hardened
> the design before implementation began (correctness, then
> adversarial/security, then performance); the implementation itself
> was then verified phase by phase against real `cargo build`/
> `cargo test` runs, not just review. **Everything this RFC's scope
> actually covers is done; `mq_connect_via`'s fallback remains a named,
> pointed-at follow-up, not a gap in what was promised.**
>
> Implementation checklist (all required before this RFC can be called
> complete; not a mandated order):
> - **Done (Phase 1).** `env(name) -> Result(str, str)` — the `ENV_BUILTINS` builtin, its
>   `nir_env_get` extern, and `Effect::Env` added to `ast::Effect`'s
>   closed set (§6, "Effect on the permission model").
> - **Done (Phase 2a).** `Domain` opened to a runtime registry (`DOMAIN_SLOTS`, `NAME_INDEX`,
>   `register_domain`) with `acquire`/`release` unchanged and lock-free
>   (§3), plus the `kernel::recorder` discriminant-encoding update done
>   in the same change, not deferred.
> - **Done (Phase 4).** Eager, deterministic startup registration for the seven built-ins
>   and every `build_with_native_plugins`-supplied provider (§4).
> - **Done for `db_connect`/`call_via` (Phase 6); `mq_connect_via` explicitly
>   deferred, not forgotten.** Runtime scheme dispatch inside `nir_db_connect`/
>   the new `nir_call_via`, plus the plugin-provider table (`kernel::
>   plugin_provider`), replacing any compile-time scheme match (§2).
>   `nir_mq_connect_via`'s own fallback is deliberately out of scope for
>   this pass — Context (above) already named it "not 'existing' the way
>   §1 assumes" (no `nir_mq_connect_via` kernel fn exists at all yet,
>   `mq_connect_via` is typechecked/ownership-checked but never reaches
>   codegen). The follow-up that adds it should copy `kernel::
>   plugin_provider::connect_conn_shape` (used by `nir_db_connect`) as
>   its template verbatim — same `ProviderOp::ConnStream` variant, same
>   `HandleTable<PluginConn>`, same acquire-on-`Pooled`/acquire-inside-
>   `checkout_with_key_cap`-on-`Unpooled` split — rather than rederiving
>   the dispatch shape from scratch.
> - **Done (Phase 5).** `PoolRegistry<PluginManagedConnection>`, the credential-stripped
>   `(provider, identity)` pool-key rule, the per-provider pool-key cap
>   and its unpooled fallback (still `acquire`/`release`-bracketed), and
>   `has_broken`'s always-`false` simplification (§5).
> - **Done (Phase 7).** The reaper: a single self-looping `ThreadPool` job
>   (`kernel::reaper`, its own dedicated `ThreadPool` singleton, not
>   `lib.rs`'s user-`spawn` one), per-sweep panic containment (a
>   `reaper_panics` counter surfaced through `dump_report`, not the outer
>   `worker_loop` `catch_unwind` the RFC's own review pass flagged as
>   silently ending the loop on one bad sweep), a shutdown flag (checked
>   each wake, unused by anything today since `ThreadPool` still has no
>   join API), `NIRDOSHA_KERNEL_REAPER_INTERVAL_SECS` floor/ceiling
>   validation, and a rotating sweep offset. Every pool-backed registry
>   this RFC touches (`db.rs`'s sqlite/postgres, `http.rs`'s http/https,
>   `plugin_provider.rs`'s shared plugin registry) self-registers with it
>   on first use via a new type-erased `pool::Sweepable` trait +
>   `pool::register_for_reaping`/`sweep_all_registered` — a real
>   extension beyond §5's own text, which described the mechanism but not
>   how a `PoolRegistry<M>` actually reaches it; this closes that gap for
>   every registry this RFC's own phases created or already had, not just
>   the plugin one. One real gap fixed while wiring this: `pool.rs`'s
>   `PluginManagedConnection::is_valid` never called
>   `record_stale_rehydrated` (unlike `SqliteManager`/`PostgresManager`/
>   `HttpManager`'s own `is_valid` impls) — it had no `domain` field to
>   record against; Phase 7 adds one (§5's own text already assumed this
>   parity existed).
> - **Done (Phase 3).** The plugin ABI additions: `_is_valid`/`_close` on every shape
>   (`call` included, via its kernel-internal `_connect`), the
>   must-not-unwind and must-be-thread-safe contracts, and
>   `NativePluginBuiltin::validate`'s four-functions-or-none build-time
>   check (§2).
> - **Done (Phase 8).** At least one real plugin built and run against this ABI end to
>   end — RFC 0008 Phase 4's own still-open ask, which this RFC is — the
>   same bar RFC 0008 Phase 1 itself shipped with
>   `crates/plugin-example-native-shout`/`-native-kv`: a real,
>   separately-compiled `call`-shape crate
>   (`crates/plugin-example-native-authed-http`), linked into a real
>   `nirdosha build` output, making a genuine `TcpStream`-backed HTTP/1.1
>   request with an `Authorization: Bearer <token>` header sourced from
>   the process environment at connect time — the concrete gap this
>   RFC's own Motivation names (`http_get`/`http_post`/`call_via`'s two
>   core-served schemes have no way to set a custom header from `.nir`).
>   Proven by `crates/compiler/tests/native_plugin_examples.rs`'s
>   `the_shipped_authed_http_plugin_compiles_and_makes_a_real_authenticated_request`
>   (a real local `tiny_http` server on an ephemeral port observes the
>   real header) and, once more, by hand: the same compiled binary run
>   standalone against an independent local server process, watching the
>   request land live.
>
> All 8 phases (env, the open domain registry, eager registration, the
> plugin provider contract, `db_connect`/`call_via`'s runtime dispatch,
> pooling, the reaper, and one real end-to-end plugin) have landed and
> pass real, compiled-and-run tests
> (`crates/compiler/tests/native_plugin_codegen.rs`,
> `crates/compiler/tests/native_plugin_examples.rs`,
> `crates/runtime-kernels/src/kernel/pool.rs`'s and `reaper.rs`'s own
> test modules). `mq_connect_via`'s own runtime fallback is the one
> item explicitly out of scope for this RFC's phases — see that
> checklist item above for the template a follow-up should copy.

## Motivation

Every external service Nirdosha consumes today (`db`, `mq`, `http`/
`https`, the identity builtins) got there the same expensive way: a
hand-written `Ty`, a hand-written set of builtins in `ast::BUILTIN_NAMES`,
a hand-written `typeck.rs` signature, a hand-written `codegen.rs` `emit_*`
+ `declare`, and a hand-written `runtime-kernels` implementation wired
into a *closed* `Domain` enum (`kernel/mod.rs:141`, currently `Tcp`,
`File`, `Thread`, `Db`, `Mq`, `Http`, `ServeHttp`). That's a real,
deliberate design for the handful of backends that justify a compiler
change — but it doesn't scale to "whatever Nirdosha may be asked to
consume next" (object storage, a payment API, an external OIDC IdP,
anything), and nothing today lets a new backend show up without
touching the compiler.

This isn't a green-field problem. Three prior efforts in this repo
already converged on pieces of the answer, independently, and this RFC
is the work of finishing and connecting them, not inventing a fourth
mechanism:

- **ADR 0004** built exactly the right dispatch shape — a `scheme://`
  connection string routed to a plugin by naming convention
  (`db_provider_mysql_connect`, `mq_provider_stomp_connect`) — proved
  against real MySQL and ActiveMQ containers. It died the next day when
  the tree-walking interpreter it depended on (`PluginFn`, `self.plugins`)
  was deleted (`c82fa1f`). The ADR's own note says the shape is still
  right, "whenever `db`/`mq` gain compiled-path codegen and a revived...
  plugin mechanism." `db`/`mq`'s compiled-path codegen now exists
  (`emit_db_connect`/`emit_mq_connect`, real `nir_db_*`/`nir_mq_*`
  kernels). The revived plugin mechanism is what §2 below is.
- **RFC 0008** built the compiled-path plugin ABI itself —
  `crate::plugin::NativePluginBuiltin` — a third-party crate exports
  plain `#[no_mangle] extern "C"` functions over scalars/`str`/
  `handle(Kind)`, precompiled to a staticlib, and `codegen.rs`'s
  existing `sigs`-driven call dispatch links and calls it with zero
  special-casing. Its own status box names Phase 3 (Cargo-driven
  auto-discovery) and Phase 4 (a real external-I/O plugin ported onto
  this ABI) as open. This RFC is that Phase 4, with the admission/
  rehydration requirements below folded in.
- **RFC 0007** named, but never built, "the compiler-side manifest
  pass... declared bounds feed the kernel's ceilings instead of the
  current hardcoded default" (§10, row "Manifests"). It also already
  built — and left explicitly unwired — the two generic primitives a
  uniform model needs: `kernel::HandleTable<T>` and
  `kernel::pool::PoolRegistry<M: r2d2::ManageConnection>`, both
  documented as built "for the next resource domain... instead of
  inventing its own table," and `kernel::thread_pool::ThreadPool`,
  "the exact deadlock-avoidance design `rfcs/0006`'s spawn/join
  concerns need."

What's requested, restated precisely against that background:

1. One uniform way to add a future service, not a bespoke compiler
  change per vendor — matching JMS's "write once against the shape,
  swap the vendor" model (a `.nir` program calls `db_connect`, never a
  vendor-named function; the vendor lives only in the URL scheme).
2. That uniform way must never add a blocking wait anywhere in the
  admission path. [[nirdosha_admission_kernel_nonblocking]]:
  `kernel::acquire` is fail-fast CAS, never blocks. This is a hard
  invariant, not a starting point to negotiate from — see "Rejected
  alternatives" for why the closest Java precedent (JCA's blocking
  pool checkout) is explicitly not the model. **Scoped precisely,
  since a later review pass rightly pushed on this**: the invariant is
  about `kernel::acquire`'s own CAS decision, which this RFC keeps
  instantaneous for every domain it adds. It does *not* — today,
  already, in shipped `db.rs`/`http.rs` — extend to pool *checkout*:
  `pool.get()` blocks up to `connect_timeout` (default 5s) waiting for
  a free connection when a pool is exhausted, one layer below
  `acquire`. That's real, and this RFC's plugin providers inherit it
  unchanged rather than resolving the tension — see §5 for the fuller
  discussion and "Open questions" for whether checkout should also
  move to fail-fast.
3. Config (URLs, credentials) must be sourceable from the process
  environment, not hardcoded in `.nir` source — today neither exists;
  every `db_connect`/JWT secret in `examples/features/
  57_nirdosha_ops_console_server_v2.nir` is a literal.
4. The kernel should have a live catalog of every service a program
  depends on, and proactively keep pooled connections healthy — not
  just discover staleness reactively, the first time a caller happens
  to check one out. [[nirdosha_pool_rehydration_requirement]]'s
  original ask (db/http rehydration) already shipped
  (`kernel/db.rs`'s `SqliteManager`/`PostgresManager::is_valid`,
  `kernel/http.rs`'s `HttpManager`/`HttpsManager::is_valid`, both
  calling `record_stale_rehydrated`) — but strictly lazily, on
  checkout. Nothing watches a pool while it's idle, and nothing knows
  a provider exists before the first connect.

## Threat model

Added after a review pass pointed out that severity judgments about
this design are meaningless without stating who the adversaries are —
worth pinning down before the findings below, not after:

- **A `NativePluginBuiltin` plugin is build-time trusted, not an
  adversary.** It's native code, statically linked into the host binary
  by whoever built it — it can already do anything the host process
  can do, with or without this RFC. "Malicious plugin" isn't a useful
  threat to design against here; **buggy plugin** and **compromised
  build pipeline** are the real, in-scope concerns (a panic, a resource
  leak, a slow `is_valid` — not a plugin deliberately attacking its own
  host).
- **The `.nir` program is the real tenant adversary.** It may be
  third-party code running against operator-linked plugins and
  operator-set env vars — a multi-tenant `serve` deployment is the
  concrete case this matters for. Most of the findings below are
  **tenant → operator** attacks: a `.nir` program doing something
  within its type-checked rights that costs the operator more than
  intended (holding resources, reaching a provider it shouldn't,
  exhausting a shared pool).
- **The external endpoint** (a DB server, an S3 bucket, an IdP) may be
  hostile or compromised — already true today for `db`/`http`, not new
  to this RFC, but worth stating since a `call`-shape plugin's whole
  purpose is reaching arbitrary such endpoints.
- **Env/config knobs are operator-controlled**, not tenant-controlled —
  but this RFC adds several new ones, and "operator-controlled" isn't
  the same as "validated"; a typo'd operator value shouldn't degrade
  into something worse than the default.
- **One pre-existing gap worth naming so silence isn't mistaken for
  oversight**: the `sqlite` bare-path arm (§2 — `db_connect("/any/path")`,
  no `://` to sniff) is arbitrary filesystem read/write via SQLite,
  today, for any `.nir` program that can reach `db_connect` at all — the
  sharpest tenant→operator capability in the entire built-in set. It
  predates this RFC, and this RFC neither widens nor narrows it; it's
  named here only so a reader doesn't mistake this document's silence
  on it for having missed it.

## Design

### 1. Three resource shapes, not one builtin set per vendor

`db` (request/response), `mq` (streaming pub/sub), and `http`/`https`
(one-shot, no handle) are already, structurally, the three shapes ADR
0004 proved generalize across vendors (MySQL as a second `db`-shaped
backend, ActiveMQ/STOMP as a second `mq`-shaped backend). This RFC
keeps that as a closed set for now, on the same terms `Domain`'s own
doc comment already states for itself: *"Add one here... exactly when a
real resource needs it — not speculatively ahead of that."* A fourth
shape is a future RFC/ADR decision with a real backend behind it, not
something this document tries to pre-enumerate.

Every future service is one of these three shapes. A `.nir` program
never sees a vendor name in a builtin name — only in the connection
string it passes to `env(...)` (§6).

**Naming the `.nir`-facing entrypoint for each shape, pinned here since
an earlier draft left `call`'s unnamed** — a real gap, not a nit, found
by trying to write an actual example against this design and
discovering there was nothing to call. `conn` uses the existing
`db_connect(url)`; `stream` uses the existing `mq_connect_via(url)`
(ADR 0004's own precedent: a new, `_via`-suffixed entrypoint alongside
`mq_connect`'s original fixed `(host, port)` shape, rather than
overloading it). `call` gets the identical treatment, one new builtin,
named `call_via` rather than after one specific scheme — an earlier
draft called this `https_request_via`, which bakes an http-shaped bias
into what's actually the generic `call`-shape dispatch entrypoint (it
falls through to *any* registered `call`-shape plugin, not just
HTTP-flavored ones — an `s3://`, `authedhttp://`, or any other scheme a
plugin registers); `call_via` matches the `conn`/`stream`/`call` shape
vocabulary this section already uses, and the `_via` suffix precedent
`mq_connect_via` already sets for "a fixed-shape builtin plus a
scheme-dispatched `_via` sibling":
`call_via(url: str, path: str, body: str) -> Result(HttpResponse, str)`
— scheme-sniffed exactly like the other two (§2), falling through to a
registered `call`-shape plugin when the scheme isn't plain `http`/
`https`. Core `http_get`/`http_post`/`https_get`/`https_post` are
untouched and keep their existing fixed `(host, port, path[, body])`
shape — `call_via` is additive, the same relationship `mq_connect_via`
already has to `mq_connect`. Reusing `Result(HttpResponse, str)` as
`call_via`'s return type is a deliberate lowest-common-denominator
reuse of an existing shape (`ast.rs`'s `prelude_structs()` already
injects `HttpResponse { status: i64, body: str }` the same way
`http_get`/`http_post` return it today), not a claim that
`call_via("smtp://...")` is HTTP underneath — a non-HTTP provider (an
SMTP gateway, say) returning `{status, body}` through `call_via` is
just reusing the closest-shaped existing struct, so a `.nir` reader
should not read the return type as implying an HTTP response is always
what comes back.

### 2. Provider dispatch: runtime, not compile-time — the correction that matters most

An earlier draft of this section had `codegen.rs` scheme-sniffing a
`.nir` call site's `str` argument **at compile time** and emitting a
direct `call`/`declare` against whichever `NativePluginBuiltin` matched
— the same shape ADR 0004's interpreter-side `match` had. That's wrong
for exactly the reason §6 exists: `db_connect(env("DATABASE_URL"))`
means the scheme isn't known until the program *runs* and reads that
variable. `codegen.rs` sees a call to `db_connect` with an `env(...)`
expression as its argument — it cannot sniff `"mysql://..."` out of
that string, because that string doesn't exist until runtime. A review
pass caught this (framed as "`handle(Kind)` can't distinguish
providers," since the identical gap resurfaces one call later, at
`db_query` time — see below), and correctly enough — the fix goes
deeper than a patch: **all** scheme dispatch moves to runtime, inside
the kernel, not into `codegen.rs`'s call-site codegen at all.

**What stays exactly as it is today.** `db_connect`/`db_query`/
`db_execute`/`stop` (and `mq`'s/`http`'s equivalents) each compile to
one fixed extern call, unchanged from current `codegen.rs` —
`nir_db_connect(url_ptr, url_len, ...)`, `nir_db_query(handle, sql_ptr,
sql_len, ...)`, and so on. No new `match` in codegen, no per-scheme
`declare`. This is the real home of the "zero new dispatch machinery"
claim the earlier draft made in the wrong place — it's true of
`codegen.rs`, just not for the reason that draft gave.

**What's new: a runtime provider table, populated once at process
start (§4).** Every `nir_db_connect`/`nir_db_query`/... Rust
implementation, given a runtime string or handle id, does this, all
inside the kernel:

1. `nir_db_connect(url, ...)`: check built-in schemes first (SQLite
  path/`:memory:`, `postgres://`/`postgresql://`), exactly as today.
  If none match, normalize the scheme (below) and look it up in a
  process-wide `static PLUGIN_PROVIDERS: OnceLock<HashMap<&'static
  str, ProviderFns>>`, populated once at startup (§4; never from
  anything runtime-only). Found → **check out from
  `PoolRegistry<PluginManagedConnection>` (§5), keyed by the
  credential-stripped identity below — the same `get_or_create`-then-
  `pool.get()` shape core `db`'s own `postgres_pool_registry()...`
  (`db.rs`) already uses**; `connect_fn` runs only when that checkout
  grows the pool, never called directly by `nir_db_connect` itself.
  `kernel::acquire(domain)` brackets the checkout, not the pool's
  growth (§5's ceiling section states this precisely — an earlier
  draft said "on pool growth" here, which was wrong, and is corrected
  in both places). The checkout is wrapped in the kernel-owned
  `HandleTable<PluginConn>` (below); that table's id is returned as
  the `handle(Kind)` value. Not found → the real, disclosed "no
  provider registered for scheme X" error ADR 0004's own
  "Consequences" section already named as this convention's one honest
  cost — the error echoes only the **normalized scheme** (the part
  before `://`, never the full URL), so it can never leak a credential
  even though it does echo tenant-controlled input — now hit at
  runtime, on a real string, exactly the way the
  interpreter-side version of this mechanism always worked (the
  earlier draft's actual mistake: ADR 0004's own dispatch was never
  compile-time — the interpreter evaluated the argument to a
  `Value::Str` and pattern-matched its *content* during execution,
  which is runtime dispatch already; this RFC's compiled-path version
  needed to match that, not invent a static `match` in `codegen.rs`).
2. `nir_db_query(handle, sql, ...)`/`nir_db_execute`: look up `handle`
  — core `db_table()` first, then `HandleTable<PluginConn>` on a miss,
  the same priority built-in schemes already get in step 1; a miss in
  *both* is the same invalid-handle error every kernel boundary
  already returns for a bad id, not a new error kind. If found in
  `HandleTable<PluginConn>`, read *that entry's own recorded*
  `ProviderFns` and call its `op_fn` — never anything resolved from
  the call site. This is what makes the cross-provider mixup the
  review named structurally impossible, rather than merely checked: no
  code path calls a provider's `_op` based on anything but what that
  specific handle was given at `_connect` time. A "validate
  `PluginConn.provider == the op's provider`" check, as originally
  proposed as a patch on top of a compile-time-resolved call, isn't
  needed as a separate step, because there is no compile-time-resolved
  call to validate against — the handle's own table entry *is* the
  only source of which function pointer ever gets called for it.
  `stop` **returns the checkout to the pool** (removes the entry from
  `HandleTable<PluginConn>`, then drops the `PooledConnection` guard it
  held) rather than calling `close_fn` directly — `close_fn` only ever
  runs where r2d2 already runs it: `has_broken`-triggered eviction on
  put-back, r2d2's own internal idle/lifetime reaping, or this RFC's
  reaper sweep (§5). This mirrors exactly how core `db`'s pooled
  connections already behave, not a new lifecycle invented for
  plugins.

**Handle ownership is kernel-owned, not plugin-owned** (unchanged
conclusion from the previous draft, now more clearly load-bearing): the
kernel keeps one `HandleTable<PluginConn>` per shape (the same role
`db_table()` plays for core `db`), `PluginConn` pairing the plugin's
raw `i64` with the `ProviderFns` it was connected through. `.nir` code
only ever holds the kernel's table id; a plugin's four functions (below)
only ever receive the raw `i64` the kernel unwraps for them — on every
call into the plugin, not just `_connect`'s return.

**A stale handle id resolving to a different provider's connection —
checked against the real `HandleTable<T>`, not assumed safe.** A review
pass raised exactly the right concern: the "dispatch is structurally
safe" argument above only holds if a closed handle's id can never be
reassigned to a later, unrelated connection — otherwise a tenant
holding a stale id from a closed S3 handle could have that same id land
on a freshly opened MySQL handle's table slot, and the kernel would
dispatch the *right* function pointers for the *wrong* logical handle.
Checked against the actual, already-shipped `HandleTable<T>`
(`kernel/mod.rs`, not new machinery this RFC needs to invent): `insert`
mints an id via `next_id.fetch_add(1, Ordering::Relaxed)`, and `remove`
only deletes the map entry — `next_id` is never decremented, reset, or
returned to a freelist. Ids are monotonically unique for the life of
the process; a removed id is never handed out again, by construction,
not by convention or by a generational tag this RFC needs to add. This
makes the concern moot for both core and plugin handles as they already
exist, not something to newly design a fix for. One honest residual
note, for completeness rather than because it's a real risk: the id
space is a 63-bit positive `i64` range, and `fetch_add` never wraps in
practice — same "generous, not tuned" posture as `MAX_DOMAINS`
elsewhere in this document.

**Pool keys, and `PluginConn`, must carry a credential-stripped
identity — not the raw URL.** Two separate concerns land on the same
fix. First: `PoolRegistry` keyed by provider name alone (as an earlier
draft implied) means `db_connect("postgres://admin:pw@host/db1")` and a
later `db_connect("postgres://readonly:pw2@host/db2")` — same provider,
same pool — would silently reuse a physical connection opened under one
identity to serve queries intended for a different user or database: a
confused-deputy privilege escalation, worse for a plugin provider where
a URL is more likely to carry distinct credentials (an S3 key pair, an
OIDC client secret) than for core `db`. Second: a raw URL, password
included, living in a pool key or a `PluginConn` for the process's
whole lifetime is a real, if low-probability, exposure surface (core
dumps, `/proc` memory, an accidental future `dump_report` leak). One
rule fixes both: the pool key — and anything `PluginConn`/`HandleTable`
retains past the `_connect` call — is **`(provider, credential-stripped
identity)`**, not `(provider,)` and not the raw URL.
"Credential-stripped identity" means the parsed `scheme://user@host:port/path`
with the password component removed but the *username* kept (a
different user is a different privilege scope, so it must still key a
different pool). **Rescoped, since a review pass correctly caught
that the previous wording can't survive contact with pooling itself**:
the claim was "the raw URL is used once, at `_connect`, and never
retained afterward" — but r2d2 redials by calling
`ManageConnection::connect()` with no arguments, repeatedly, over the
pool's whole life (after an `is_valid` failure evicts a connection, on
ordinary pool growth, on a reaper-triggered top-up) — so the adapter
*must* hold the full URL, password included, for the pool's entire
lifetime to be able to redial at all. This doesn't sink the underlying
argument, it just scopes it correctly: the boundaries that actually
matter are that the raw URL never appears in a pool **key**, never in
`PluginConn`/`HandleTable`, and never in anything `dump_report` or an
error message prints (§6, §2's error-message rule) — not that it's
discarded outright. It necessarily lives inside the pool manager
itself (`PluginManagedConnection`, or core `PostgresManager` today,
which is equally unable to redial without retaining its own URL) for
as long as the pool exists — the same posture every shipped core
manager already has, not a new exposure this RFC introduces. The
core-dump/`/proc`-memory concern from the previous draft is still
real at *that* narrower scope (a live process's memory always holds
what it needs to redial) — worth stating honestly as a residual,
disclosed cost, not something this rule eliminates. One more
component needed pinning, not left implicit: the **query string is
excluded from the identity too**, not just the password — a query
string is exactly where SAS-token-style URLs (`?token=...`,
`?api_key=...`) carry credentials in real-world convention, and the
whole point of this rule is to never retain one past the connect call.
The disclosed cost, stated rather than hidden: two URLs differing only
in query parameters share a pool, including a parameter that's actually
connection-affecting (`?sslmode=require`) rather than secret — a
provider for which that distinction matters must fold the affecting
parameter into the host/path portion of its own identity computation
itself (a provider-specific normalization choice), rather than relying
on this kernel-generic default to separate them. This is a correction
to how `PoolRegistry::get_or_create`'s `key: &str` argument must be
constructed for every provider this RFC adds, core and plugin alike —
not a new data structure.

**A real operational trap this identity rule creates, worth one line
rather than discovering it during an incident**: rotating a password
without changing user/host/path is, correctly, *not* a new identity —
same pool, same physical connections. `db_connect("postgres://admin:pw1@h/db")`
then, after a rotation, `db_connect("postgres://admin:pw2@h/db")` share
a pool; the second caller silently gets connections still authenticated
as `pw1` until the pool evicts them (idle/lifetime timeout, an
`is_valid` failure) or the process restarts. This is the correct
consequence of "same user is the same privilege scope," not a bug in
it — but operators rotating a credential behind an unchanged
user/host/path should treat that rotation as requiring a rolling
restart, and a provider that needs rotation to take effect immediately
must fold a versioned credential tag into its own identity computation
instead of relying on this default rule.

**Scheme → identifier normalization, spelled out exactly** (schemes
aren't identifiers — they carry `+` suffixes, case variance, ports):
take the substring before `://`, lowercase it, map `+`/`.`/`-` to `_`.
`mysql+tls://...` → `mysql_tls`; `POSTGRESQL://...` → `postgresql`.
Built-in scheme identifiers, pinned exactly rather than left implicit
(this is the set a plugin may *not* register): `sqlite` (a bare path or
`:memory:` — no `://` to strip; a fallback rule, not a normalized
scheme), `postgres`, `postgresql`, `redis`, `http`, `https`. Two
collision rules, both caught at build time, both loud rather than
silent: a plugin scheme that normalizes to one of those six loses
outright; two plugins whose schemes normalize to the *same* identifier
as each other are equally ambiguous and both rejected —
`build_with_native_plugins` fails the build in either case, naming the
colliding identifier, never picking a winner silently.

**A shape's plugin contract is four functions or none, checked at
build time.** `NativePluginBuiltin::validate` (RFC 0008 Phase 1's real
gate, already running before a build links) gains one more rule:
registering `<shape>_provider_<scheme>_connect` without matching `_op`/
`_close`/`_is_valid` (§5) for the same prefix fails the build
immediately, naming the missing function — not a runtime surprise.

**`_op`'s bind-value narrowing is a runtime error today, not a
compile-time one — disclosed here, not silently left as a gap someone
has to rediscover.** `db_query`/`db_execute` against a plugin-routed
connection reject any non-empty bind-value array (`kernel::
plugin_provider::op`'s own `binds_present` check) with a named error at
request time, since `_op`'s pinned ABI (§5) is a single `str` in, `str`
out — there is no bind-value channel across the plugin boundary for
this phase to route through. A genuine static (typeck-time) diagnostic
for this would need either a refined `db` handle type distinguishing
plugin-routed handles from built-in ones (a real type-system addition,
out of scope here), or a narrower dataflow check tracing a `Ty::Db`
value back to the specific `db_connect(...)` call that produced it
(machinery `typeck.rs` doesn't have today) — both bigger than this
phase's scope, so the check stays runtime-only for now. **The blast
radius is smaller than it might look**: `docs/LANGUAGE.md` §2's `str`
has no concatenation, slicing beyond `str_slice`'s fixed bounds
primitive, or interpolation of any kind — there is no way to build a
dynamic SQL string in `.nir` source by splicing a runtime value into it
at all, with or without this restriction, which is precisely why binds
are "the *only* way to parameterize a query" in the first place (§5's
`_op` doc comment, `typeck.rs`'s own `db_query` entry). So a plugin
provider that rejects binds doesn't newly expose a concatenation-based
injection path — a caller who wants to react to it defensively today
still gets a clean, named `Err`, not a silent drop or a trap.

**The `call` shape's contract was incoherent in the previous draft, and
it's the shape most exposed to abuse — fixed, not just patched.** The
previous draft gave `call` three functions (`_request`/`_is_valid`/
`_close`, no `_connect`) while also claiming the kernel keeps an
"internal pooled connection keyed by host" for it — but
`PluginManagedConnection` (§5) needs a `connect_fn` to create anything
poolable, and a three-function contract doesn't supply one. Left as
written, that meant one of two bad outcomes: either the kernel can't
actually pool `call`-shape plugins at all (the pooling sentence was
dead spec), or `_request`'s own text — "the plugin either dials
per-request or manages its own keep-alive internally" — is literally
true, meaning a `call`-shape plugin's connection footprint is invisible
to the kernel: no `Domain` accounting, no ceiling, no reaper, no
rehydration. That's exactly backwards: `call` is the shape whose whole
purpose is reaching arbitrary tenant-specified external URLs (§ Threat
model), so it's the worst possible place for admission to go dark — a
tenant pointing an `http`-shaped plugin at a URL of its choosing could
hold unbounded connections with nothing to stop it.

`call`-shape plugins matter for a second, independent reason worth
naming, found while working through a concrete example (a payment
gateway): core `http_post`/`https_post` have **no way to set a request
header at all** — `typeck.rs`'s own signature is `(host, port, path,
body)`, nothing else, and `nir_http_post`'s `declare`d LLVM signature
matches. The underlying kernel (`http.rs::request_bytes`) *can* attach
`Authorization: Bearer <token>`, but only the notify subsystem's own
internal call path ever passes that argument — it's unreachable from
any public `.nir` builtin. So today, `.nir` code cannot make an
authenticated REST call at all, to anything, full stop — not a gap
`env(...)` closes by itself. A `call`-shape plugin's `_request`
function is native code with no such restriction: it can build whatever
headers it needs internally, with a credential sourced via `env(...)`
at connect time. This is the concrete reason a `call`-shape plugin, not
core `https_post`, is how something like a payment gateway integration
would actually have to be built under this design — noted here as a
real, disclosed limitation of core `http`, not something this RFC
claims to fix.

The fix: `call` gets the same four-function contract as `conn`/`stream`
after all, just not a `.nir`-visible one.
`<call>_provider_<scheme>_connect(host: str) -> i64` dials (or
otherwise prepares) one keep-alive session to a *host*, not a full
per-request URL — called only by the kernel's own
`PluginManagedConnection` adapter to populate
`PoolRegistry<PluginManagedConnection>`, exactly mirroring what
`HttpManager::connect` already does for core `http` (a real TCP dial,
keyed by host, that `.nir` code never sees or triggers directly).
`<call>_provider_<scheme>_request(raw_conn: i64, path, ...) -> str`
replaces `_op`, operating on that pooled connection. `_is_valid`/
`_close` are unchanged in shape from `conn`/`stream`. This restores
real admission: exactly like core `http` today (`Domain::Http`'s own
doc comment: "admission is held for the duration of one pooled
checkout"), `kernel::acquire(domain)` brackets *each `_request` call's*
own internal checkout — acquired when `_request` checks a connection
out of `PoolRegistry<PluginManagedConnection>`, released when that call
returns it — not held across multiple calls the way `conn`/`stream`'s
session-scoped bracketing is (step 1 above; an earlier draft said "on
pool growth" in both places, conflating session-scoped and
per-request-scoped admission — corrected in both now).`connect_fn`
still only runs when a checkout actually grows the pool. A `call`-shape
plugin's concurrent *requests* are therefore ceiling-bound exactly like
core `http`, and every pool it grows is reaper-swept exactly like
`db`/`mq` — the gap the review named is closed by giving `call` the
same admission path core `http` already has, not by declaring it out of
scope.

**A plugin function must never unwind across its own `extern "C"`
boundary — this is a hard contract, not a kernel-side safety net.** All
four ABI functions (`call`-shape included, now that its `_connect` is
kernel-internal rather than absent) are plain `extern "C"`,
not `extern "C-unwind"`. Per this crate's own already-established
understanding of that distinction (`thread_pool.rs`'s doc comment:
unwinding across a plain `extern "C"` boundary is a *defined abort at
that boundary* since Rust 1.71 — not UB, but also not something a
caller-side `catch_unwind` can intercept, because the abort happens
inside the plugin's own frame before control ever returns to the
kernel). That means a panic escaping a plugin's own Rust implementation
of `is_valid_fn`/`connect_fn`/etc. takes down the **entire process**,
not just the call — there is no kernel-side mitigation possible against
this ABI, the same way `NativePluginBuiltin::validate`'s scalar-only
restriction (RFC 0008 Phase 1) exists because some ABI questions are
hard, not because they're solved. So this is a stated plugin-author
contract, alongside `is_valid_fn`'s fail-fast requirement: a plugin
written in Rust must wrap its own function bodies in `catch_unwind`
internally and translate any panic into this ABI's existing error
conventions (the 0/1/negative sentinels every other kernel boundary
already uses) before returning across the FFI edge. Unenforceable by
the kernel, same as `is_valid_fn`'s fail-fast requirement — a
documented hazard, not a silent one. (§5 covers the separate, related
question of what happens if a panic reaches the *reaper's own* code,
not a plugin's.)

**A plugin's four functions must be safe under concurrent invocation on
the same raw `i64`.** `&handle(Kind)` is a real, already-allowed borrow
form (`plugin.rs`'s own doc comment: "a *read* of a stateful resource
must **borrow**, not consume"), and a borrowed handle can be captured
by more than one concurrently spawned `.nir` thread — meaning the
kernel can call a provider's `_op`/`_is_valid`/`_close` concurrently
against the same raw `i64` in a well-typed program. This isn't new to
plugins — the identical question already applies to any core `db`/`mq`
handle held across `spawn` — and this RFC deliberately doesn't add
kernel-side per-handle locking to answer it: a lock on the hot path
would reintroduce exactly the blocking-under-contention primitive
"Rejected alternatives" rules out for JCA, just relocated. So this is
also a stated contract, not a kernel guarantee: a provider's four
functions must be internally thread-safe (or the provider must document
that a handle from it isn't safe to share across spawned threads) —
consistent with `ownership.rs`'s general "the checker is the real gate"
posture (its own doc comment) rather than runtime locking.

**Where a plugin provider's admission ceiling and env var come from.**
`register_domain(name, env_var, default_max)` (§3) needs both, and the
compiler can't invent a plugin author's preferred values. Convention
plus override: absent anything more specific, a plugin provider gets
`NIRDOSHA_KERNEL_MAX_<SHAPE>_<NORMALIZED_SCHEME>` (uppercased) and the
same `default_max` (10,000) every built-in domain already uses. A
plugin author who wants something different attaches it as two more
fields alongside the existing `name`/`params`/`ret`/`static_lib` on
that provider's registration metadata — read once, at the single
startup registration point (§4), never guessed by the compiler and
never overridable from a second, independent code path (§4 explains
why there's only one such path now).

Cargo-driven auto-discovery (RFC 0008 Phase 3) is a real dependency for
this to feel uniform in day-to-day use — without it, wiring a new
plugin still means hand-editing the `build_with_native_plugins` call
site. This RFC doesn't attempt that work; see "Open questions."

### 3. Opening `Domain`: same non-blocking CAS, no longer a hardcoded match

`Domain` becomes a registry instead of a closed enum, **without
introducing a lock anywhere `acquire`/`release` touch** — an earlier
draft of this section sketched `counters_for`'s hardcoded `match`
becoming a `static REGISTRY: OnceLock<Mutex<Vec<(&'static str,
DomainCounters)>>>` indexed by `DomainId`. That's wrong: a `Mutex` on
the acquire path is exactly the blocking primitive "Rejected
alternatives" rules out for JCA, reintroduced quietly by the registry
mechanism itself rather than by anything provider-specific. The fix is
splitting the rare write path (registration) from the hot read path
(acquire/release):

```rust
const MAX_DOMAINS: usize = 4096; // generous, not tuned — same posture
                                  // Domain::default_max already has

static DOMAIN_SLOTS: [DomainCounters; MAX_DOMAINS] =
    [const { DomainCounters::new() }; MAX_DOMAINS];
static NEXT_DOMAIN: AtomicU32 = AtomicU32::new(0);
// Registration-time only — never touched by acquire/release/stats.
static NAME_INDEX: OnceLock<Mutex<HashMap<&'static str, DomainId>>> = OnceLock::new();

pub struct DomainId(u32); // opaque index into DOMAIN_SLOTS

/// Idempotent: a second call with the same `name` returns the same
/// `DomainId` — kept as a defensive property even though §4's single
/// eager registration pass is, in practice, the only caller. Takes
/// `NAME_INDEX`'s lock via `lock().unwrap_or_else(|e| e.into_inner())`,
/// never a bare `.unwrap()`: a panic while holding this lock (an
/// allocation failure inside the `HashMap`, say) would otherwise
/// poison it permanently, turning one transient fault into "no new
/// provider can ever register again" for the rest of the process — the
/// map it guards is just a name→id lookup, safe to keep using after a
/// panic. `NEXT_DOMAIN.fetch_add` happens *inside* this lock, after
/// checking whether `name` is already present — never before — so two
/// duplicate `register_domain("db", ...)` calls burn exactly one slot
/// between them, not one each.
pub fn register_domain(name: &'static str, env_var: &'static str, default_max: i64) -> DomainId;

pub fn acquire(domain: DomainId) -> bool {
    let counters = &DOMAIN_SLOTS[domain.0 as usize]; // plain array index, no lock
    // ... identical CAS loop to today's acquire, unchanged
}
pub fn release(domain: DomainId);                        // same array index, no lock
pub fn stats(domain: DomainId) -> (i64, u64, u64, u64);   // same, no lock
pub fn record_stale_rehydrated(domain: DomainId);         // same, no lock
```

**Decided: opaque `u32`, as sketched above — not an interned string
key.** An earlier draft left `DomainId`'s representation open against
an interned-string alternative; the string alternative buys nothing the
registry doesn't already provide via `NAME_INDEX` (name → id is already
a real, available lookup for anything that needs the name back), and a
`u32` is what makes `DOMAIN_SLOTS`'s plain array indexing possible at
all. This also settles `kernel::recorder`'s existing discriminant-based
domain encoding, which an earlier draft treated as separate, deferred
follow-up work: it isn't separate — it's a mechanical part of *this*
change, done in the same pass as the rest of §3, not after it, since
both of `dump_report`'s real consumers (§ Compatibility) are already
being updated alongside this RFC regardless. If a binary (non-text)
encoding of domain identity ever exists somewhere, it gets a version
bump and a string table the same way any other format change would;
today's flight-recorder text output already carries names, not raw
discriminants, so nothing downstream of *that* format needs to change
at all.

`acquire`/`release`/`stats`/`record_stale_rehydrated` index straight
into the static array — the exact same lock-free CAS-on-a-static shape
`TCP`/`FILE`/... already have today, just addressed by a `u32` instead
of hardcoded per-name. The seven existing domains are registered eagerly,
at fixed indices 0–6 in `main`'s preamble (§4 — not "at first use";
that phrasing here was never updated when §4 settled on eager
registration, and this is the correction), as `DomainId` constants
behind the same names (`domain::DB`, `domain::HTTP`, ...) so every
existing call site
(`kernel::acquire(Domain::Db)`) becomes `kernel::acquire(domain::DB)`
with identical behavior, identical env var names
(`NIRDOSHA_KERNEL_MAX_DB` unchanged), identical default ceiling. This is
additive to the CAS mechanism, not a rewrite of it — the non-blocking
guarantee is a property of `acquire`'s loop body, which stays untouched;
only the one-time registration path gains a lock, and nothing on the
`acquire`/`release` path ever waits on it. `MAX_DOMAINS` is a real,
disclosed limit (same posture as `Domain::default_max`'s own
"deliberately generous, not tuned") — registering past it is a loud
**startup-time** error. (Deliberately back to this wording: a previous
correction pass changed it to "registration-time" on the assumption
that registration was lazy and could happen at any point in a running
process; §4's own correction below removes that lazy path entirely —
every provider now registers during `main`'s preamble, so exceeding
`MAX_DOMAINS` is always caught before any `.nir` code runs, never
sprung on a connect months into a long-running process.)

`register_domain`'s `name` argument is the registry key, so every
caller must agree on its exact spelling or the same provider silently
burns two slots under two names. Canonical form, pinned here rather
than left implicit: for a built-in domain, the existing bare name
(`"db"`, `"mq"`, `"http"`, ...); for a plugin provider,
`<shape>_provider_<normalized_scheme>` — the identical string §2's
collision rule already normalizes and matches against built-in names.
§4's single eager registration pass is the only caller of
`register_domain` for either kind, using the exact same normalization
function §2's runtime dispatch also uses — one function, not two
independent implementations that could drift apart on spelling.

This is also the literal answer to "the kernel should know what's
what": `register_domain` **is** the catalog. A new
`kernel::registered_domains() -> Vec<(&'static str, DomainId)>` (reading
`NAME_INDEX`, itself only ever consulted off the hot path) gives
`dump_report`/the flight recorder/§5's reaper a live enumeration of
every service the running program actually depends on, core or
plugin-supplied, instead of the current fixed seven-line
`dump_report` loop.

### 4. Registration is eager, at startup, over a set that's always fully known at build time

An earlier draft of this section went through two designs: first, a
compiler pass walking `.nir` `<shape>_connect` call sites as the *sole*
registration mechanism (broke on `db_connect(env("DATABASE_URL"))` —
a call site's scheme can be runtime-only, so a pass over call sites
can't see it); then, in response, "lazy by default, manifest as
pre-warm" — registration deferred to each provider's actual first
connect. That second design is workable but turns out to be solving a
problem that §2's own correction just made disappear: **which
providers a binary could ever use is fully determined by what was
linked into it**, not by what any particular `.nir` call site's string
literal says. The seven built-in domains are always present. Every
plugin provider is named in the `&[NativePluginBuiltin]` list passed to
`build_with_native_plugins` — a build-time, Rust-side list the person
building the binary supplies, regardless of whether any `.nir` source
reaches it via a literal or an `env(...)`-sourced URL. What's genuinely
runtime-only is *which already-registered* provider a given
`db_connect(env(...))` call resolves to when it executes (§2's
dispatch) — not *whether that provider is known to the kernel at all*
(registration). Once that distinction is drawn, there's no reason for
registration to be lazy at all.

So registration is simply **eager, at process startup, over the full
enumerable set**: `codegen.rs` emits one `register_domain(...)` call
per built-in domain, in a fixed order (`domain::TCP` through
`domain::SERVE_HTTP`, indices 0–6 — deterministic, not "whichever
`acquire` happens to fire first," closing an ordering gap an earlier
draft's Compatibility section assumed without actually stating),
followed by one call per distinct provider in
`build_with_native_plugins`'s list, in the order given, into `main`'s
preamble, before any user code runs. Every provider's `env_var`/
`default_max` (§2) comes from exactly **one** place — this single
registration call, reading either the built-in `Domain` constants or
the plugin's own registration metadata — so there's no second code path
that could register the same provider under different admission
settings depending on how a `.nir` program happens to spell its URL.

This closes RFC 0007's "manifest pass... declared bounds feed the
kernel's ceilings instead of the current hardcoded default" more
narrowly than a literal AST-walking compiler pass would: the "manifest"
here *is* `build_with_native_plugins`'s own plugin list plus the fixed
built-in set — no new frontend pass over `.nir` call sites is needed at
all. RFC 0007's fuller vision (NFR bound-checking, composition checks
across actual call sites) stays future work, not claimed here.

`register_domain`'s idempotence (§3) is kept as a defensive property,
not a load-bearing one, now that there's a single eager registration
point — harmless, and worth keeping in case a future revision adds a
second call site.

### 5. Proactive rehydration: a bounded sweep that never touches the hot path

Every registered domain that's pool-backed also registers a
`PoolRegistry<M>` (§ `pool.rs`, unchanged) keyed by provider —
**lazily, on that pool's first real use, not eagerly at every domain's
registration time** (red-team report A29, `scratch/red-team-report-
main-d7fae42.md`: this sentence's own phrasing read as implying eager
registration; `pool::register_for_reaping`'s own `Once`-guarded call
site, next to each pool's own accessor function, is what actually makes
it lazy — an idle compiled program that never opens a `db`/`http`/
plugin connection registers zero pools with the reaper). A new
`kernel::reaper` module realizes its periodic wake using
`kernel::thread_pool::ThreadPool` **as it actually exists**, verified
against the real API (`submit(self: &Arc<Self>, job: Job) ->
Result<(), SpawnError>`, `thread_pool.rs:161`) rather than assumed to
have a timer/parked-worker feature it doesn't. `ThreadPool` only
dispatches jobs; it has no built-in notion of a periodic wake. The
reaper doesn't need one: it's realized as a **single job, submitted
once, whose body is its own infinite loop** —
`loop { sweep_all_pools(); thread::sleep(interval) }` — not a second
threading primitive alongside `ThreadPool`, just one ordinary job that
happens to never return. From `ThreadPool`'s own perspective this
permanently occupies one worker for the life of the process (it never
reaches `wait_for_job`'s idle state, so it's never a candidate for
`IDLE_TIMEOUT` retirement) — a disclosed, deliberate cost of exactly
one OS thread, not a new capability requirement on `thread_pool.rs`
itself. The interval is `NIRDOSHA_KERNEL_REAPER_INTERVAL_SECS`, default
30. A rehydration found this way calls the same
`record_stale_rehydrated(domain)` counter that reactive, checkout-time
rehydration already uses — proactive and reactive rehydration are the
same event, discovered at a different time, not two telemetry surfaces.

**Per-iteration panic containment, and a counter so a dead reaper isn't
invisible.** `thread_pool.rs`'s own `worker_loop` already wraps a
*whole job's* execution in `catch_unwind` — but the reaper's job body
is `loop { sweep_all_pools(); sleep(...) }`, so a panic anywhere inside
one `sweep_all_pools()` call (a kernel-side bug — a poisoned lock
`.unwrap()`, an indexing error — not a plugin panic, which is the
different, unrecoverable case §2's plugin contract already covers)
would be caught at the *outermost* `catch_unwind`, ending the whole
loop, not just that one sweep. The job "completes" from `ThreadPool`'s
perspective — no crash, nothing visibly wrong — and rehydration
silently stops forever while every other health signal stays green,
exactly the failure mode a review pass named. Fix: `catch_unwind` goes
*inside* the loop, around each `sweep_all_pools()` call individually,
so one bad sweep is contained and the loop continues to the next
`sleep`/wake; a caught panic increments a new
`reaper_panics: AtomicU64` counter, surfaced through `dump_report`/
`stats` (§3) — so a broken reaper is a visible number going up, not a
silent absence.

**Shutdown is out of scope for this RFC's own correctness claim, but
the loop must not make it worse.** `ThreadPool` (as read) has no
explicit shutdown/join API — a real compiled Nirdosha binary today has
no graceful-shutdown story that joins background workers at all, so an
abrupt process exit already kills every thread, reaper included,
unconditionally; this RFC doesn't need to solve graceful shutdown to be
correct. What it does need to not do is become a *new* obstacle if a
future RFC (RFC 0006's structured-concurrency territory) adds one: the
reaper's loop checks a shutdown flag (an `AtomicBool` or equivalent)
each time it wakes, so a future join-based shutdown has something to
signal. Stated plainly rather than hidden: a bare
`thread::sleep(interval)` is uninterruptible for the whole interval, so
even a signaled shutdown can take up to
`NIRDOSHA_KERNEL_REAPER_INTERVAL_SECS` (default 30s) to actually notice
and exit — acceptable for now; a `Condvar`-based sleep-with-interrupt
(parked on a condvar the shutdown flag also signals, instead of a bare
`sleep`) would cut that to near-zero if a future shutdown mechanism
ever needs it tighter than 30s.

**`NIRDOSHA_KERNEL_REAPER_INTERVAL_SECS` needs its own validation, not
just a default.** This is a genuinely new operator-facing knob (§ Threat
model's fourth bullet: operator-controlled but not automatically
validated) — set to `0`, it produces `sleep(Duration::ZERO)`,
effectively a busy loop pinning one core at 100% forever. Parsing
follows the exact convention `PoolConfig::from_env` already establishes
(§ `pool.rs`: a malformed value degrades to the default field-by-field,
never fails the whole program) — extended with a floor: parse failure
→ default (30s); parse success but below 1 second → clamp to 1 second;
`0` and negative values are never accepted silently. A soft ceiling
too, not just a floor: a value above 3600s (1 hour) is accepted but
logs a startup warning rather than silently degrading proactive
rehydration to near-never — an operator setting an enormous interval
(`NIRDOSHA_KERNEL_REAPER_INTERVAL_SECS=99999999`) is far more likely to
have made a typo than to have intended to disable proactive sweeping,
and reactive checkout-time rehydration still covers correctness either
way, so this is a warn, not a clamp.

Robustness properties, stated explicitly rather than left implicit:

- **Non-blocking checkout, not `pool.get()` — and, by this RFC's own
  defaults, not a connection factory either.** A review pass raised a real concern
  here: does `pool.try_get()` silently *grow* the pool (open a new
  connection) when nothing's idle but the pool is under `max_size`,
  which would turn the reaper into a background connection-opener —
  visible server-side, and reintroducing exactly the blocking-connect
  hazard `try_get` was chosen to avoid? Checked against this repo's
  actual vendored dependency, not general r2d2 knowledge: `Cargo.lock`
  pins `r2d2 0.8.10`, and its `try_get_inner`
  (`r2d2-0.8.10/src/lib.rs:456-486`) only ever pops from the
  already-idle list; on empty, it returns `Err` immediately without
  calling `add_connection` (the function that *does* open a new
  connection — called only from `get_timeout`'s blocking loop, never
  from `try_get`'s path). `try_get`'s own doc comment confirms this
  directly: "This method will not block waiting to establish a new
  connection." So the concern doesn't hold against the version this
  repo actually depends on — each wake, for every registered pool, the
  reaper does one opportunistic `pool.try_get()` (`None` if nothing
  idle, no side effect) + `ManageConnection::is_valid` + return. Plain
  `pool.get()` blocks up to `connect_timeout` waiting for a free
  connection — on a fully checked-out pool, that would stall the one
  reaper worker for every other registered pool behind it; `try_get`
  avoids that for the reason just cited, not by accident. One real
  caveat worth stating: a *successful* pop does call
  `establish_idle_connections` afterward, which — only if a provider's
  `PoolConfig.min_idle` is set above its current default of `Some(0)`
  (§ `pool.rs`) — can open more connections as a background top-up.
  Inert under this RFC's own defaults; a provider that overrides
  `min_idle` upward should expect the reaper's successful sweeps to
  contribute to that top-up, not just validate.
- **One opportunistic checkout per pool per wake, stated as the bound,
  not hidden.** A pool of size N takes up to N wakes to sweep in full —
  this is a deliberate ceiling on reaper work per wake, not an
  oversight; tightening it (checking more than one connection per pool
  per wake) is a tuning question for implementation, not a correctness
  one.
- **A per-`is_valid` timeout is a plugin contract, not a kernel
  guarantee — it can't be enforced.** An earlier draft of this bullet
  claimed "each `is_valid` call gets its own short timeout." That isn't
  implementable against this ABI: `is_valid_fn` is a synchronous
  `extern "C"` call, and Rust cannot interrupt a blocked C call running
  on another thread from the outside. The only two real options are
  blocking the one reaper worker on it (no timeout — exactly the hazard
  this bullet was trying to avoid) or spawning a thread per validation
  (destroys the "exactly one worker" design stated above). So this is a
  **plugin contract requirement, not a kernel mechanism**: `is_valid_fn`
  must be fail-fast (a cheap local probe — "is this socket still
  readable" — not a fresh reconnect carrying its own TCP timeout). A
  plugin that violates this stalls the sweep for every other registered
  provider on that wake — a real, disclosed hazard, the same category
  of cost ADR 0004's own "Consequences" section already accepts for a
  load-bearing naming convention with nothing on the kernel's side
  statically checking the other side's behavior. The per-wake pool cap
  above bounds *how often* a misbehaving plugin's stall recurs, not its
  duration once it happens.

**Coverage latency, stated as a formula, not left for an operator to
derive.** Worst-case time before the reaper proactively rehydrates a
given stale idle connection is roughly `interval × pool_size ×
ceil(#pools / per_wake_cap)` — with this RFC's defaults (30s interval,
a size-10 pool, few registered pools) that's on the order of five
minutes; with the pool-key growth this section discusses below and the
per-wake cap, it degrades roughly linearly — a stale connection in a
rarely-hit pool could sit unswept for hours. Acceptable for the stated
goal (reactive, checkout-time rehydration has no such latency and still
catches every case proactive sweeping might miss), but the formula
belongs here so an operator tuning
`NIRDOSHA_KERNEL_REAPER_INTERVAL_SECS` downward knows what they're
actually buying. One fairness note worth stating alongside it: the
per-wake cap makes the reaper work-bounded but not coverage-*fair* as
written — whichever pools registered last are swept last, every cycle,
forever. Starting each wake's sweep at a rotating offset into the pool
list (round-robin, not always index 0) fixes that for free — one
counter's worth of new state, worth specifying now rather than
discovering as a real staleness blind spot later.

**Hard constraint**: none of the above ever touches `kernel::acquire`'s
synchronous CAS path. The reaper runs entirely on its own worker,
against connections that are, by `PoolRegistry`/r2d2 construction,
currently idle (returned, not checked out) — a caller's `acquire` is
never blocked waiting on the reaper, and the reaper never blocks a
caller. This is RFC 0007 §5's "no plane synchronously depends on a
plane downstream" rule, applied to this specific mechanism, and it's
what keeps [[nirdosha_admission_kernel_nonblocking]] true for every
future provider, not just the ones that predate this RFC.

**This RFC's fail-fast philosophy stops at `kernel::acquire` — checkout
is a separate, bounded wait, already true today, not something this RFC
introduces.** `pool.get()` (used in shipped `db.rs`/`http.rs`'s
connect/query paths — confirmed by reading them, not assumed) blocks up
to `connect_timeout` when a pool is exhausted: a real wait, one layer
below `kernel::acquire`'s own instantaneous CAS, that a review pass
correctly flagged as sitting awkwardly next to "Rejected alternatives"'
rejection of JCA's blocking checkout. What keeps these from being the
same failure: JCA/HikariCP's actual incident pattern is *unbounded or
effectively-unbounded* waits compounding under load into
thread-starvation deadlock; a `connect_timeout`-bounded wait fails
loudly after a fixed, small bound — closer to an ordinary network
client's connect timeout than to a resource-exhaustion deadlock. This
RFC doesn't change that behavior for `db`/`http`, and gives plugin
providers the identical bounded-wait checkout for consistency, rather
than introducing a new inconsistency between built-in and plugin
providers. Whether *checkout* itself (not just `acquire`) should also
move to `try_get`-plus-immediate-error — eliminating this wait entirely,
at the cost of pushing "pool exhausted, retry" handling onto `.nir`
code — is a real, unresolved design fork; see "Open questions." This
RFC's position, stated rather than left to be inferred: keep bounded
blocking checkout for now, since a disclosed 5-second bound is a
categorically different risk than JCA's actual incident history, and
changing shipped `db`/`http` checkout semantics is a bigger, separate
diff than "add a uniform provider model."

**Caveat particular to a `max_size = 1` pool.** Even though the reaper
never blocks *itself* waiting for a connection (`try_get`), while it
holds the one connection it did get — for the duration of that one
`is_valid` call — a concurrent real caller's `pool.get()` has zero idle
connections to find and genuinely waits, up to `connect_timeout`, the
same as if an ordinary request were mid-flight instead. This isn't new
blocking the reaper introduces into `kernel::acquire`'s own path (the
"Hard constraint" above is unaffected) — it's a size-1 pool sharing its
one physical connection between a real caller and the sweeper the
ordinary way it always shares that one connection between any two
callers. Worth choosing `min_idle`/`max_size` ≥ 2 for a provider where
this matters, not a defect in the reaper's design specifically.

**What the ceiling actually bounds — pool growth must not leak past
it.** Left unstated otherwise, `Domain`'s admission ceiling and a
pool's own `PoolConfig::max_size` (§ `pool.rs`) are two independent
numbers: r2d2 grows a pool up to `max_size` internally, on demand,
inside `pool.get()`'s blocking path — code that never calls
`kernel::acquire` at all. If `max_size` exceeds the domain's ceiling,
the ceiling is decorative: physical connections can outgrow it without
ever being denied. This RFC's rule, stated explicitly rather than left
to an implementer's guess: **a provider's `PoolConfig::max_size` must
be set ≤ its domain's `default_max`/env-var ceiling** — enforceable
because §4's single eager registration point knows both numbers for
every provider at once, and can refuse to build a pool whose configured
`max_size` exceeds its own ceiling. The admission ceiling counts
concurrently-*held* checkouts (`conn`/`stream`: connect-to-stop; `call`:
one pooled checkout's duration, per `Domain::Http`'s existing doc
comment), and `kernel::acquire`/`release` bracket exactly that hold —
the pool's own `max_size` becomes an inner implementation detail a
caller never actually hits first, rather than an independent, silently
looser bound. One honest pre-existing gap this surfaces, not introduced
by this RFC: today's shipped `db`/`http` code already pairs
`Domain::default_max` (10,000) with `PoolConfig::default().max_size`
(10) — consistent today only because `max_size` happens to be the
tighter number, not because anything states or enforces that
relationship as a rule. This RFC is what turns "happens to be
consistent" into a stated, checked invariant.

**The ceiling bounds concurrency per pool, not distinct pools per
provider — a real, separate gap, sharpest for `call`.** §2's
acquire-per-checkout bracketing closes *concurrency*: every checkout of
every pool under one domain shares that domain's single CAS counter, so
total concurrently-held checkouts across however many distinct pools a
provider has is already bounded by the domain ceiling, regardless of
pool-key count. What it does *not* bound is the **number of distinct
pool keys itself** — `PoolRegistry::get_or_create` only ever inserts
into its `HashMap<String, Pool<M>>`, never evicts. A `call`-shape
plugin pools by host (§2); a tenant calling it against 10,000 distinct,
attacker-chosen hosts creates 10,000 `HashMap` entries — each
individually small (`min_idle: Some(0)`), but unbounded in aggregate:
registry memory and bookkeeping grow with tenant whim, a slow-growth
DoS distinct from a connection-exhaustion one. **This is pre-existing,
not introduced by this RFC**: core `http`'s own `http_pool_registry()`
(`http.rs:330`) is keyed by `pool_key(host, port)` today, with the
identical no-eviction `get_or_create` — `http_get` against enough
distinct tenant-chosen hosts already has this exact problem, in shipped
code, checked directly rather than assumed. This RFC is what's
generalizing the keyed-pool pattern to more providers, though, so it's
the right place to state the rule going forward: **cap the number of
distinct pool keys per provider** — one global
`NIRDOSHA_KERNEL_POOL_MAX_KEYS` (not a per-scheme/per-prefix knob:
pinned to exactly this one name here, since two implementers left to
invent it independently would spell it two different ways), a
generous, disclosed default — e.g. 256, the same "generous, not tuned"
posture as `MAX_DOMAINS`. Once at the cap, a *new* key falls back to
an unpooled, direct dial-per-request — but **`kernel::acquire(domain)`/
`release(domain)` still bracket that fallback request exactly as if it
were a checkout**, stated explicitly because the previous draft's "no
admission-ceiling growth beyond the one connection in flight" reads
ambiguously in exactly the dangerous direction: if the fallback path
skipped `acquire` entirely (a plausible misreading — it isn't a real
"checkout"), the cap would itself become a ceiling *bypass*, since a
tenant past 256 keys could hold connections with no admission at all,
reopening the exact exhaustion the ceiling exists to bound. So, pinned
per shape: no `HashMap` entry, no reaper coverage, no pooling either
way — but for `call`, the fallback is dial-per-request, released
immediately after; for `conn`/`stream` past the cap, it's a direct,
unpooled connection that's still wrapped in the handle table and still
session-bracketed (`acquire` at connect, `release` at `stop`), exactly
like a pooled one, just without a pool backing it. An LRU-eviction
alternative (close the least-recently-used existing pool to make room)
was considered and set aside: it needs bookkeeping `PoolRegistry`
doesn't have today, and evicting a pool with in-flight checkouts
against it needs its own correctness argument — degrading to unpooled
dialing past the cap is simpler and strictly safer to reason about.
This RFC applies the cap to every new keyed pool it adds (`call`-shape
plugins, and any `conn`/`stream` plugin whose identity is
host-cardinality-sensitive); retrofitting it onto core `http`'s
existing `http_pool_registry()` is a real, disclosed follow-up this RFC
doesn't itself attempt.

**Plugin providers need a real `ManageConnection` impl, and can't
supply one directly.** `PoolRegistry<M: r2d2::ManageConnection>` is
generic over a *Rust trait* — a `NativePluginBuiltin` plugin, being a
precompiled `extern "C"` staticlib, cannot implement a Rust trait
across that boundary any more than it can return a `Vector`/`Matrix`
(RFC 0008 Phase 1's own scalar-only ABI limit, for the identical
reason: "a Rust trait object has no meaning to LLVM IR generated by a
different compilation," `plugin.rs`'s own doc comment). This is why
§2's shape contract includes a fourth required function,
`<shape>_provider_<scheme>_is_valid(handle: i64) -> i64` (0/1, the same
boolean-as-`i64` convention every other kernel boundary already uses),
alongside `_connect`/`_op`/`_close`. The kernel supplies one generic
adapter — `struct PluginManagedConnection { connect_fn, is_valid_fn,
close_fn: extern "C" fn(...) }` — that implements `ManageConnection`
once by calling through whichever function pointers were resolved for
a given provider at registration time. Every plugin provider of a given
shape reuses this same adapter type, so `PoolRegistry<PluginManagedConnection>`
is the one pool type covering all of them; the reaper above and
reactive checkout-time rehydration both call `is_valid_fn` through it
exactly the way `db.rs`'s `SqliteManager::is_valid` calls into
`rusqlite` today. `ManageConnection::has_broken` — a required method
distinct from `is_valid` in r2d2's own trait definition, meant as a
*fast*, non-round-tripping check, not a network probe — always returns
`false` for `PluginManagedConnection`: a deliberate simplification, not
an oversight. A genuinely broken connection `has_broken` misses on
put-back is still caught by `is_valid` at its next checkout or the
reaper's next sweep — **which depends on `test_on_check_out` actually
being on for these pools**, stated explicitly rather than left as an
unstated assumption: `PoolConfig::apply` (§ `pool.rs`) never calls
r2d2's own `.test_on_check_out(...)` builder method, so every pool
built through it — plugin pools under this RFC included, not just
core `db`/`http` — inherits r2d2's own builder default, which is
`true` (`r2d2-0.8.10/src/lib.rs`'s `Config::default`). Plugin pools go
through the identical `PoolConfig::apply`/`get_or_create` path core
pools already use, so this holds for them without needing anything
new — but it's a real, checked fact about this repo's actual
dependency, not a "presumably" — and it avoids a fifth required extern
function in an ABI already trying to stay minimal.

The unwrap direction, stated explicitly since it's the kind of detail
an implementer could guess either way: `is_valid_fn`/`close_fn`, like
`_op`, take the plugin's **raw `i64`**, never the kernel's
`handle(Kind)` id. `.nir` code holds the kernel id → `PluginManagedConnection`
looks it up in the kernel-owned `HandleTable<PluginConn>` (§2) →
extracts the `PluginConn`'s raw `i64` → calls the plugin's extern
function with that. The plugin's four functions never see a kernel id
at all, on any of the four calls, not just `_connect`'s return — the
kernel unwraps on every call into the plugin, symmetrically with how it
wraps once on the way out of `_connect`.

A `NativePluginBuiltin`-registered S3 provider that
supplies all four functions gets the identical proactive sweep `db`/
`http` get — for free, no plugin-side scheduling code required — but
only because the kernel does the trait-adaptation work, not because the
plugin implements `ManageConnection` itself.

### 6. Uniform config resolution: `env(name)`

One new builtin: `env(name: str) -> Result(str, str)`, `Err` if unset.
A new `ENV_BUILTINS` list in `codegen.rs`, one `nir_env_get` extern
wrapping `std::env::var` (a new `kernel/env.rs` or a few lines in
`mod.rs` — no handle type, no pooling, no `Domain`, since reading the
process environment isn't a scarce external resource the way `tcp`/
`file`/`db` are). Every provider — core-shipped or plugin — takes its
URL/credential as an ordinary `str` argument; `env(...)` is how `.nir`
source populates it without a literal: `db_connect(env("DATABASE_URL"))`,
a plugin's `s3_provider_s3_connect(env("S3_URL"))`. No provider-specific
env-var sniffing anywhere in the compiler — where a secret comes from
stays visible in `.nir` source, the same explicitness precedent
`NIRDOSHA_TEST_POSTGRES_URL`'s existing fallback-with-override pattern
already sets one layer down.

**Decided: the real process environment only, never a `.env`-style
file.** An earlier draft left this open; it doesn't need to stay open.
`.env` convenience is a *launcher* concern (`direnv`, systemd's
`EnvironmentFile=`, `docker run --env-file`) — not a language-runtime
one. A runtime that silently reads a `.env` file breaks sealed
deployments and reproducibility in exactly the way that made this
worth asking about in the first place: a stray file changing behavior
with nothing in the invocation announcing it. If `.env` convenience
ever becomes a real need, it belongs at the CLI/compiler-driver level
behind an explicit flag, never implicit inside `env(...)` itself. This
is a firm "no," not a placeholder for later.

**Decided: richer config (keypairs, cert+key pairs, whole credentials
files) is out of scope for `env(...)`, and mostly doesn't need a new
mechanism at all.** A `NativePluginBuiltin` plugin is native code — it
can already read a file itself.
`s3_provider_s3_connect(env("AWS_SHARED_CREDENTIALS_FILE"))` passes a
*path*; the plugin opens it. No `file`-reading builtin is needed for
the plugin ecosystem specifically: multi-part config is multiple
`env(...)` calls combined by the caller; file-shaped config is a path
via `env(...)` that the provider itself reads. The one case this
doesn't cover is a **core builtin** needing file-shaped config —
concretely, an HS256→RS256/ES256 JWT migration needing a keypair rather
than a shared string — but that's core-compiler work either way (the
JWT signing builtins live in `IDENTITY_BUILTINS`, not anything this RFC
touches), so it's out of scope on those grounds regardless of this
decision. A general-purpose `file_read` builtin, if one is ever
justified beyond that narrow case, is its own RFC — reading arbitrary
files raises its own admission/permission questions (a
`Domain::File`-shaped ceiling already exists for `tcp`/`file` handles;
whether a *string-returning* file read reuses it or needs something
narrower isn't a question this document should answer in passing).

**Once `env(...)` returns a value, it's an ordinary `str` — leakage is
a real risk, not just unauthorized reads.** "Effect on the permission
model" covers who may call `env(...)` at all; it doesn't cover what
happens to the value afterward. A connection string returned by
`env("DATABASE_URL")` often carries a password inline
(`postgres://user:pass@host/db`) — once it's a plain `str`, every
string operation applies, including concatenation into a log line or an
error message a `serve` route echoes back to a client. This RFC doesn't
propose a redaction mechanism (a real one would need `Ty::Str` to carry
a taint bit, a bigger change than this document's scope), but example
code and core kernels built against this RFC must not echo a connect
error's URL verbatim, and any future example `.nir` program must not
log an `env(...)` result directly — cheap discipline, cited here so
it's a stated rule rather than something rediscovered as an incident
later.

**Illustrative examples only, not a spec.** `env(name)` takes an
arbitrary `str` — this RFC defines the mechanism, not a fixed catalog
of variable names, and nothing below is proposed for adoption as-is.
The names a `.nir` program actually passes to `env(...)` are chosen by
whoever writes the `<shape>_connect(env("..."))` call (core example
code, or a plugin's own README), same as any other string literal. The
list exists only to show the *range* of config a uniform `env(...)`
needs to cover across the ecosystem this RFC targets — ordinary
industry-convention names, not something Nirdosha defines or reserves:

- SMTP/email: `SMTP_HOST`/`SMTP_PORT`/`SMTP_USER`/`SMTP_PASSWORD`, or a provider API key (`SENDGRID_API_KEY`, `POSTMARK_TOKEN`)
- Object storage: `AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY`/`AWS_REGION`, `S3_BUCKET`/`S3_ENDPOINT` (MinIO/R2-style), or the GCS/Azure equivalents
- Message queues: `RABBITMQ_URL`/`AMQP_URL`, `KAFKA_BROKERS`, `NATS_URL`
- HTTP/external APIs: `API_BASE_URL`, a per-provider key by convention (`STRIPE_API_KEY`, `TWILIO_AUTH_TOKEN`)
- Observability: `NIRDOSHA_OBSERVABILITY_URL` (already exists — `nfr.rs`), `SENTRY_DSN`, `OTEL_EXPORTER_OTLP_ENDPOINT`
- Auth/secrets: `JWT_SECRET`, `SESSION_SECRET` — note a real HS256→RS256/ES256 migration needs a keypair, not a shared string; the "richer config" decision above covers why that's out of scope here
- Federated OIDC/OAuth2, if Nirdosha ever delegates identity instead of self-issuing (it doesn't today): `OIDC_ISSUER_URL`/`OIDC_CLIENT_ID`/`OIDC_CLIENT_SECRET`, or a named IdP's own convention (`AUTH0_DOMAIN`, `COGNITO_USER_POOL_ID`)
- SAML (unlikely to matter for the current demo): `SAML_IDP_METADATA_URL`, `SAML_SP_ENTITY_ID`

None of these are wired to anything by this RFC — the only concrete,
in-scope consumer today is `db_connect(env("DATABASE_URL"))`/the
hardcoded JWT secret in `examples/features/
57_nirdosha_ops_console_server_v2.nir`, both cited in the Motivation.
The rest is scope illustration, kept here so a future reader sizing
`env(...)`'s design doesn't under-estimate the variety of values it
needs to pass through cleanly (plain strings, URLs, and — per the
`JWT_SECRET` note above — at least one case, keypairs, it doesn't cover
at all).

## Effect on the permission model

No change to `requires(role/claim: ...)`, the identity-layer `check_role`/
`acquire` family (a different, unrelated "acquire" from `kernel::acquire`),
or `serve.rs`'s route-exposure *rules* — this RFC doesn't touch how
those are declared or checked. Two things below mean "no authorization
impact" isn't quite the right summary of the whole picture, though:
provider reachability, and what `dump_report` now reveals.

**Provider reachability is a new capability surface this RFC creates,
and it deserves stating rather than assuming away.** Before this RFC,
`.nir` code could only ever reach the handful of built-in backends.
After it, `db_connect("s3://...")` — no `env(...)` involved at all —
makes *every* plugin linked into the binary reachable from *every*
`.nir` program in it, with no declared, per-program scoping. This is
consistent with the reachability model every existing builtin already
has, though, not a regression from one: there is no `.nir`-level
allow-list for `db_connect`/`http_get` today either — any function
anywhere in a compiled program can already call them, and the actual
trust decision happens once, at build time, in what the operator
chooses to link (which `NativePluginBuiltin`s get passed to
`build_with_native_plugins`, exactly like today's decision to link a
`db`/`http`-capable binary at all). This RFC extends an already-zero-ACL
model to more providers; it doesn't introduce a new *kind* of unscoped
access. Whether a finer-grained, `requires(...)`-style per-program
provider allow-list is worth adding later — real defense-in-depth for a
multi-tenant `serve` deployment where different `.nir` programs in the
same binary shouldn't all reach the same plugin — is a genuine open
question, not resolved here.

**`dump_report`'s HTTP route makes "no authorization impact" not
literally true.** "Compatibility" already notes `compiled-serve` serves
`dump_report()`'s raw text over HTTP; this RFC makes that output more
informative than it was, since a plugin's normalized scheme name now
appears in it — a live catalog of which backends, vendors, and
integration points a deployment actually uses, exactly the
reconnaissance an attacker would want before, say, aiming an `env(...)`
exfiltration attempt at the right variable name. This route arguably
belongs on the list of things `serve.rs`'s exposure rules ought to
govern — it currently isn't gated by them at all. This RFC doesn't
resolve that, but operator documentation for anything built against it
should say plainly that this route isn't safe to expose unauthenticated
in a deployment where the provider catalog itself is sensitive.

`env(name)` lets `.nir` source read *any* process environment variable
— a real new capability, since a careless or compromised `.nir` program
with no legitimate reason to touch AWS could still read
`AWS_SECRET_ACCESS_KEY` if it happens to be set in the process
environment. **Decided: `Effect::Env`, added to `ast::Effect`'s closed
set, unscoped.** An earlier draft left the shape of this open between
three options; a compile-time allow-list is ruled out on the same
grounds §2's own runtime-dispatch correction already established for
provider reachability — the entire point of §6 is that variable names
are deliberately *not* known at compile time in an env-driven
deployment, so a compile-time list of "permitted names" would be
checking against something the compiler structurally cannot see, the
identical failure mode the original compile-time manifest pass had.
`Effect::Env` gives the real, available thing instead: an audit/
declaration point ("this program reads the environment") on the same
"notation with nothing to check, added when a real domain needs it"
terms `Domain`'s own doc comment already states for itself — consistent
with that precedent rather than inventing a new kind of gate. A finer
allow-list stays possible later, layered on top as a lint or a separate
capability check (§ "Open questions" on a per-program provider
allow-list sketches the same shape for provider reachability) — but
that's additive, not a prerequisite for shipping `Effect::Env` itself.

`NativePluginBuiltin`-registered providers inherit `Ty::Handle`'s
existing affine-ownership guarantees (RFC 0005 §1) unchanged — opening
`Domain` touches only the kernel-side counter lookup, not
`ownership.rs`'s checking.

## Compatibility

- `Domain` enum → registry is source-compatible for every existing call
  site: the seven built-in domains keep their existing names as
  pre-registered `DomainId` constants. Nothing external ever
  serialized `Domain as u8` — but `dump_report`'s **text output** is a
  real, textually unstable surface, not a cosmetic one: moving from a
  fixed seven-line loop to iterating `registered_domains()` changes
  line order/count once any plugin-backed domain registers (existing
  domains stay first if `registered_domains()` preserves registration
  order, but that's a property to guarantee, not assume). Two real
  consumers already parse this exact text and need updating alongside
  this change: `crates/compiler/tests/nirdosha_ops_console.rs` (filters
  stdout lines by the fixed `"nirdosha kernel flight recorder:"` header
  and per-domain indentation) and `crates/compiled-serve/src/lib.rs`
  (serves `dump_report()`'s raw text directly as an HTTP response body
  — any real client of that route inherits the same instability).
- No existing `.nir` program's behavior changes: `db_connect`'s
  scheme-sniff (now inside `nir_db_connect`'s own Rust implementation,
  §2 — not `codegen.rs`) gains a plugin-lookup fallback after its
  existing built-in checks; those built-in SQLite/Postgres code paths
  are untouched, and `codegen.rs`'s call-site codegen for `db_connect`
  doesn't change at all. **`mq_connect_via` does not get this fallback
  in this pass** (status box, above) — it stays exactly as it typechecks
  and ownership-checks today, with no live kernel implementation, so
  there is nothing to disclose a behavior change *against* for it yet.
  `call_via` is a wholly new, additive builtin (next bullet), so it has
  no prior behavior to preserve.
- `env(...)` is a new, additive builtin name — can't collide with a
  user identifier, since every name in `ast::BUILTIN_NAMES` is already
  reserved.
- `call_via(...)` is likewise new and additive (§1's rename note: it
  was never shipped under its earlier draft name `https_request_via`,
  so there is nothing to migrate callers away from). One behavior worth
  stating plainly rather than leaving implicit: a **plugin-routed**
  `call_via` call always reports `status: 200` on success — `_request`'s
  pinned ABI (§2, "the `call` shape's contract") is a single `str`
  return with no separate status-code channel, so there is no numeric
  status for the kernel to relay. `call_via` against `http://`/
  `https://` is unaffected — those go through the exact same
  `nir_http_get`/`nir_http_post`-style parsing as core `http`, real
  status code included.

- **JCA-style blocking pool checkout** (wait-with-timeout when a pool
  is exhausted): rejected outright. This is the textbook cause of a
  well-known class of Java production incidents — thread-starvation
  deadlock under connection-pool exhaustion, the entire reason
  HikariCP ships dedicated leak-detection diagnostics. It directly
  contradicts [[nirdosha_admission_kernel_nonblocking]].
  `kernel::acquire` stays fail-fast CAS for every domain this RFC adds,
  with no exception for plugin-registered ones.
- **Per-provider bespoke env-var sniffing** (teach `db_connect` to
  check `DATABASE_URL` automatically, a future `s3_connect` to check
  `S3_URL`, etc.): rejected. Implicit, not auditable from `.nir` source
  alone, and doesn't generalize to a provider the core compiler has
  never heard of — a plugin's integration would need its own bespoke
  sniffing, duplicated per provider. The explicit `env(...)` builtin
  (§6) is strictly more general and keeps the config source visible in
  source.
- **A brand-new "service" abstraction distinct from `NativePluginBuiltin`**
  (a dedicated registration API/trait, closer to what the proposal ADR
  0004 itself responded to originally asked for): rejected for the same
  reason ADR 0004 gave the first time — the representation (an opaque
  affine handle plus a scalar/`str` ABI) and the routing convention
  (scheme-sniff by naming convention) both already exist. A second,
  parallel mechanism would fragment the plugin surface RFC 0008 already
  built rather than complete it.
- **Resurrecting ADR 0004's dispatch exactly as written**: impossible —
  it targeted `self.plugins: HashMap<String, PluginFn>`, which no
  longer exists; the interpreter it depended on is gone. This RFC
  targets `NativePluginBuiltin`, the only live compiled-path plugin
  ABI, instead.

## Open questions

Four earlier open questions here are now decided — folded into the
Design/"Effect on the permission model" sections above rather than
left as open items (§6 covers richer config and `.env`-file loading;
"Effect on the permission model" covers the `Effect::Env` gate shape;
§3 covers `DomainId`'s representation). What's left is genuinely
undecided, with the criteria for resolving each stated rather than
left implicit:

- **Per-domain reaper interval**: global to start
  (`NIRDOSHA_KERNEL_REAPER_INTERVAL_SECS`, one value for every
  provider), not per-domain. The extension point already exists at
  zero extra design cost, though: §2's plugin-metadata
  convention-plus-override pattern (`env_var`/`default_max`)
  generalizes directly to an interval field the same way, whenever a
  real provider needs a different cadence than every other one.
  Revisit trigger: a real provider demonstrating that need, not
  speculatively ahead of it.
- **RFC 0008 Phase 3** (Cargo-driven plugin auto-discovery): genuinely
  separate work, correctly out of this RFC's scope. One warning worth
  carrying forward into whoever picks it up: auto-discovery widens the
  build-time-trusted set from "a list the operator hand-read and
  passed to `build_with_native_plugins`" to "whatever dependency
  resolution happens to pull in" — meaning §2's collision-detection
  rules become *more* load-bearing once auto-discovery lands, not
  less, since nobody is hand-reading the plugin list to catch a
  collision by eye anymore. Version-pinning discipline and §2's
  build-time collision validation are both prerequisites for Phase 3,
  not optional hardening on top of it.
- **A fourth resource shape** beyond `conn`/`stream`/`call`: still a
  judgment call for whenever it's actually needed, not designed
  speculatively here — but the bar is pinned now rather than left
  vague: a *working* plugin built against the existing three shapes
  that demonstrates the mismatch is a real ergonomics problem, not a
  hypothetical one, **and** its own ADR (not just an RFC) once
  accepted, since a new shape changes `Ty` the way `conn`/`stream`/
  `call` already do — matching `docs/adr/README.md`'s own line between
  a judgment call and a designed-up-front change. Pinning this
  explicitly now is what stops a future revision from sneaking a
  fourth shape in casually, under an RFC that treats it as a minor
  addition.
- **Checkout fail-fast (`try_get`) vs. today's bounded `pool.get()`**:
  the one question that genuinely needs field data rather than more
  argument — §5 already scoped it honestly rather than deciding it by
  philosophy, and that framing holds. Sharpened decision criteria:
  this should be resolved by **telemetry** — actual pool-exhaustion
  frequency at realistic ceiling/`max_size` ratios in a real
  deployment — not by which failure mode sounds worse in the
  abstract. And if it does flip, it should flip **per-provider, as
  opt-in metadata** (the same convention-plus-override shape as
  `env_var`/`default_max`/the reaper interval above), never as a
  global semantic change — shipped `db`/`http` checkout behavior
  should never be silently altered out from under an existing
  deployment.
- **A per-program provider allow-list**: kept open, and honestly, kept
  open partly because *the unit of scoping doesn't exist yet* —
  Nirdosha compiles one binary; "per-program" would mean per-route or
  per-module attribution the runtime doesn't currently track anywhere.
  §2's runtime-dispatch correction (a review pass's own finding, this
  document's biggest design change) also rules out the naive version
  of this: a compile-time scheme allow-list would break exactly the
  way the original compile-time manifest pass broke, for the identical
  reason — a scheme resolved from `env(...)` isn't known at compile
  time to check against a list. The coherent boundary today is the one
  this RFC already uses: the build-time linking decision. One thing
  worth stating so a future RFC doesn't start from zero, though: a
  *runtime* capability check is compatible with this design without
  any rework — `PLUGIN_PROVIDERS`' lookup and `HandleTable`'s
  per-entry dispatch (§2) don't preclude a capability/allow-list check
  being added inside `nir_db_connect` itself, gated on whatever
  scoping unit a future revision defines; nothing here forecloses
  that, it's just not designed here.
