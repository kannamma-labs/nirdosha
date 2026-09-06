# Nirdosha — Public Roadmap

A scannable, external-facing summary of what's shipped and what's next.
This is a distillation for readers deciding whether to try Nirdosha or
contribute — the full internal tracker, with verification detail and
session-by-session notes, is [`docs/ROADMAP.md`](./docs/ROADMAP.md).

Status tags: `[DONE]` (verified — tests pass or run end-to-end),
`[PARTIAL]` (real progress, gap named), `[OPEN]` (scoped, not started),
`[NOT RUNNABLE]` (real, working code as of when it was built and
verified — against the now-deleted interpreter — but not reachable in
any form today; added 2026-09, see the callout just below).

> **2026-09 — read every "interpreter-only"/"interpreted path" note
> below as historical, not current.** The tree-walking interpreter
> (`run`/`serve`) was removed entirely in a separate pass this session —
> there is no interpreted fallback left at all. A `[DONE]` item below
> tagged "interpreter-only" was real and verified *when it was written*,
> but isn't runnable in *any* form today, compiled or otherwise, until
> Track B's native codegen actually reaches it — a strictly worse
> statement than "falls back to the interpreter." Track A's own framing
> ("gates building critical apps on the interpreted path") is fully
> moot for the same reason. What changed this session, fully compiled
> and verified end to end: `check_role` (real identity, an unforgeable
> `RoleView`), field-level `requires(role/claim: ...)` masking, function-
> level `requires(role/claim: ...)` + `acquire` (first-class/privileged
> functions), and `nfr(...)` (non-functional requirements as a compiled
> fn annotation with real APM-kernel tracking + escalation) — see
> `docs/LANGUAGE.md` §6a/§6e/§6f/§10 and `docs/PHASE0.md`'s "Twentieth"/
> "Twenty-first" updates for the full detail this list doesn't yet
> reflect below.

---

## Shipped

**Language core**
- [DONE] LL(1) grammar — hand-written parser, cross-verified against an
  independent LALR(1) generator (`crates/grammar_check/`)
- [DONE] Static type checker
- [DONE] Ownership/affine types (`box`/`&`) — no GC, no manual `free()`
- [DONE] Concurrency primitives (`spawn`/`thread`, `chan`) — compiled to
  native code (2026-09), backed by a real admission-controlled kernel;
  no mutex in the language, so a lock-order deadlock isn't expressible,
  and the one deadlock class that is (a global `chan`/`thread` stall) is
  caught by a dynamic detector and aborted, not left to hang
- [DONE] `froze` — RFC 0006 Pillar 1's `Froze<T>`, an immutable,
  freely-shareable heap handle (`box` already satisfied Pillar 1's
  `Iso<T>`)
- [DONE] `struct`/`enum`/`match`, generics, `Option(T)`/`Result(T, E)`
- [DONE] SMT-backed integer/buffer-overflow proofs (Z3), tiered with a
  runtime-guard fallback
- [DONE] Native codegen via LLVM (`-O2`) for the compiled subset —
  within 1.4× of `gcc -O2` on scalar benchmarks
- [DONE] `validate <fn_name> { pre: ... post: ... }` — real Hoare
  contracts on a function: a Z3-backed static proof that hard-fails the
  build on a genuine counterexample where it can reach one, for
  Tier-1-provable (integer-only) functions. The dynamic runtime-check
  backstop this bullet used to describe for everything Tier-1 can't
  prove no longer exists (it lived in the now-deleted interpreter) —
  see `docs/LANGUAGE.md` §16 for the current, honest split.

**Identity, data protection, and non-functional requirements** (2026-09,
compiled, no interpreter involved at any point)
- [DONE] `check_role(identity, role)` against a real `VerifiedIdentity`,
  producing a genuine, unforgeable `RoleView` — `RoleView`/`ClaimView`
  can't be directly constructed by a `.nir` program
- [DONE] Field-level `requires(role/claim: ...)` masking — a struct
  field zeroes itself on every `return` unless the returning function's
  own `RoleView`/`ClaimView` parameter proves it, fail-closed
- [DONE] Function-level `requires(role/claim: ...)` + `acquire` —
  first-class/privileged functions: a gated `fn`'s value is obtainable
  only via `acquire name(proof)`, a real `Result(fn(..)->.., str)`
  checked against a real proof; calling any `fn(..)->..`-typed value
  (gated or not) is a real indirect call
- [DONE] `nfr(latency_ms:/error_rate_max:/throughput_min_per_sec:/
  concurrency_max:)` — non-functional requirements as a first-class fn
  annotation, tracked automatically via the APM kernel with async
  escalation to `NIRDOSHA_OBSERVABILITY_URL` on a crossed threshold

**Backend/services**
- [NOT RUNNABLE] `db` (SQLite + Postgres), `json`, `http`/`https`, `mq`
  (Redis) — no codegen yet; see Track B below
- [NOT RUNNABLE] Identity — OIDC/JWT validation (`oidc_validate_token`),
  claims (`extract_claim`), an admin-editable role-mapping cache (IdP
  role names → app role names). `check_role` +
  `requires(role/claim:...)`/`acquire` are the exception, now fully
  compiled — see the identity section above.
- [NOT RUNNABLE] `transact` — durable transactions (WAL, crash replay,
  retry/timeout, idempotency)
- [NOT RUNNABLE] `workflow` — durable state machines with email/SMS/push
  notification actions
- [NOT RUNNABLE] Auto-generated, additive-only DB schema migrations

**UI engine** — the `nirdosha emit-ui` half (static HTML derived from
`struct`/`screen`/`dashboard` conventions, no live backend) is real and
runs today; everything below tagged `[NOT RUNNABLE]` depended on the
now-deleted `nirdosha serve` for its *live*, server-enforced half —
`emit-ui` still generates the corresponding markup/hints, but nothing
runs behind it.
- [DONE] Zero-syntax CRUD + dashboard inference from `struct`/fn naming
  conventions, via `emit-ui` — static markup, no UI code needed for the
  common case
- [DONE] `screen`/`dashboard`/`module` DSL for the cases naming
  conventions can't express — `emit-ui` reads these into the same
  static markup
- [NOT RUNNABLE] Field-level RBAC (`view`/`edit` role/claim gates) and
  format validation (`pattern`/`format`/`min`/`max`) *enforced
  server-side* — `emit-ui` still emits the client-side hide/disable
  hints, but there's no server left to enforce anything behind them.
  Field-level `requires(role/claim:...)` masking (identity section
  above) is a different, newer, compiled mechanism that *does* enforce
  for real today, just not through this UI-layer gate.
- [DONE] Design-token theming (`--theme`) with live reload — color
  ramps, motion, dark-mode strategy, layout shell, all optional (a
  static-generation-time concern, unaffected by `serve`'s removal)
- [NOT RUNNABLE] `workspace`/`panel` — composite multi-pane screens
  composing fields/lists from several structs onto one page
  (`docs/LANGUAGE.md` §15) — needs the live multi-source data `serve`
  provided
- [NOT RUNNABLE] `visual`/`render` — graph, heatmap, and timeline views
  on a dashboard or inside a panel, on top of the existing bar-chart-only
  `chart` (`docs/LANGUAGE.md` §11c) — needs live query data
- [NOT RUNNABLE] `field { render: "countdown" }` — a live SLA countdown
  chip on a table field, ticking client-side with zero added network
  traffic (`docs/LANGUAGE.md` §11) — needs a live table row to attach to
- [NOT RUNNABLE] `action { show_result: true }` — a "Simulate"/"Preview"
  action shows its own JSON return value in a modal instead of just
  refreshing the row (`docs/LANGUAGE.md` §11) — needs a live action call
- [NOT RUNNABLE] A workflow stage stepper — a real `●━●━○━○` progress
  stepper on a workflow queue row instead of a bare state-name badge, no
  syntax change (`docs/LANGUAGE.md` §14) — needs a live workflow queue
- [NOT RUNNABLE] `examples/ctms/ctms.nir` — all of the above proven
  together against a real 89-screen enterprise app spec (a
  Counter-Terrorism Financing & Transaction Monitoring System), not just
  in isolation — see `docs/ROADMAP.md` Track E6; the static markup still
  generates via `emit-ui`, the live proof no longer runs

**LLM integration**
- [DONE] LL(1) grammar exported to GBNF for constrained decoding
  (`crates/compiler/nirdosha.gbnf`)
- [NOT RUNNABLE] Structured `Diagnostic` JSON on every error
  (`--format=json`) — that flag was interpreter-mode-only and no longer
  exists in the compiled-only CLI (`nirdosha build`/`emit-llvm` print
  plain-text errors); `emit-ast`'s own JSON output, listed separately
  below, is unaffected
- [DONE] `emit-ast`/`validate_fragment` for typed AST/fragment tooling
- [PARTIAL] `crates/bench/` pass@1 + self-repair-rate harness — scaffold,
  corpus, and a real `Model` (`--mode real`, any OpenAI-compatible
  `/chat/completions` endpoint) all exist; not yet run against a live
  provider for lack of an API key in this environment

---

## In progress / next

**Track A — Production readiness** (2026-09: this track's own "gates
building critical apps on the interpreted path" framing is moot — the
interpreter is gone, so there's no interpreted path left to gate
anything on. Kept for now as a record of open production-hardening
work that would matter again if/when a compiled `serve` (Track B8)
exists to need it.)
- [OPEN] `transact` durability under real kill-mid-transaction conditions
- [OPEN] A deployment story for a *compiled* `serve` (containerization,
  secrets/JWKS handling) — `nirdosha serve` itself no longer exists
- [PARTIAL] Observability — a local OTel-shaped tracer exists; wiring
  to a real collector (OTLP) is open
- [OPEN] A compatibility/versioning policy before the next breaking
  language change
- [PARTIAL] Identity admin console — role-mapping cache is done;
  multi-IdP registry and a roles→functions/fields report are open
- [OPEN] Real Windows verification — the compiled `tcp`/`tcp_listener`
  runtime was ported to Windows' `RawSocket` API (v0.1.0-alpha.3) but
  has never run on a real Windows machine; needs an actual test pass
- [OPEN] macOS binaries link system Z3 instead of vendoring it —
  `z3-src` 416.0.2 fails to build against current AppleClang (a real
  upstream incompatibility); revisit once a fixed `z3`/`z3-src` release
  ships

**Track B — Full compilation** (`json`/`http`/`mq`/`transact`/
sandboxing/most of identity have no codegen yet and, with the
interpreter gone, don't run in any form; native codegen covers the
numeric/control-flow subset, `tcp`/`tcp_listener`, `file`, scalar-only
native plugin calls, `dec128` arithmetic, basic concurrency —
`thread`/`spawn`/`join`, `chan`/`send`/`recv`, `froze` — and, as of
2026-09, `check_role`, field- and function-level `requires(...)`/
`acquire`, and `nfr(...)` — see the identity section under "Shipped"
above)
- [DONE] `file` (`open`/`send`/`recv`/`stop`) — linked `nir_file_*`
  kernels, the same "declare + link a staticlib" pattern `tcp` already
  used; `examples/file_io.nir` compiles and runs as a native binary
  unchanged, verified against the interpreter's own output
  (`crates/compiler/tests/codegen.rs`)
- [DONE] Basic concurrency — `thread`/`spawn`/`join`, `chan`/`send`/
  `recv` (2026-09), plus RFC 0006 Pillar 1's `froze`. Contrary to this
  section's own earlier framing below ("not a kernel to link"), it
  shipped *as* a kernel to link: `runtime-kernels`' `nir_thread_spawn`/
  `nir_thread_join`/`nir_chan_*`, a real admission ceiling
  (`Domain::Thread`), and a dynamic global-stall deadlock detector
  (`docs/LANGUAGE.md` §7/§10, `rfcs/0007-apm-runtime-kernel.md` §8).
  Word-sized payloads/arguments/results only; `sandbox` is unaffected,
  still open below.
- [DONE] Scalar-only native plugin calls (Kind A plugins) —
  `plugin::NativePluginBuiltin`/`codegen::build_with_native_plugins`
  (`rfcs/0005-plugin-boundary-safety-and-performance.md` §3), ~250x
  faster than interpreted plugin dispatch for this subset; `str`/
  aggregate-typed plugin builtins still interpreter-only
- [DONE] The compiled-path runtime kernels (`det`/`inv`/`tcp`/`file`/...)
  moved from a bare, dependency-free `rustc` invocation to a real Cargo
  package (`crates/runtime-kernels/`, `docs/adr/0003-runtime-kernels-
  cargo-dependency.md`) that can depend on crates.io crates — the
  actual, previously-invisible reason `dec128` stayed interpreter-only
  this long: `rust_decimal` was simply unreachable from the old build.
- [PARTIAL] `dec128` — `dec_from_i64`/`dec_to_str`/`dec_round`/
  `dec_scale`, `+`/`-`/`*`/`/`, and all six comparisons compile to real
  `rust_decimal`-backed native code (`nir_dec128_*` kernels), verified
  against the interpreter byte for byte, including the division-by-zero
  trap and a `dec128` field inside a real `struct`. Only `dec_from_str`
  remains — its `.nir`-visible return type is `Result(dec128, str)`,
  and no existing compiled builtin actually constructs a real
  `Result(_, _)` enum value as its return yet (`inv`/`solve`, this
  codebase's other fallible builtins, present failure a different way)
  — a real, deliberately deferred design question, not a shortcut;
  cleanly rejected in the meantime.
- [OPEN] `transact` → `db`/`json` → `mq` → identity → `http`/`https` →
  sandboxing → first-class functions → compiled `serve` mode, roughly in
  that order. `db`/`json`/`mq` share `file`'s "linked handle-based
  kernel" shape but need a real dynamically-typed value representation
  for query results first (`Ty::Handle`'s own affine fix, `rfcs/0005`
  §1, generalizes to a `db`/`mq` connection handle for free once that
  representation exists). `sandbox` (a real, separate OS *process*, not
  a thread) remains a materially harder, separate design question from
  the concurrency work above — see `rfcs/0005` §0's own difficulty
  ranking for the fuller breakdown.

**Track C — Agent-facing HTTP API** (the spec exists —
[`docs/nirdosha-agent-api.md`](./docs/nirdosha-agent-api.md) — the `/v1/*`
server itself is 0% built, and its "about half the underlying
capability already ships" premise needs re-checking post-interpreter-
removal: much of what it counted on shipping was interpreter-backed and
isn't currently runnable — see the "Shipped" callout above)
- [OPEN] The HTTP server and its 20 endpoints across code generation,
  execution, introspection, benchmarking, and provenance

**Track D — Mobile app generation** (a second renderer of the existing
UI manifest, independent of Tracks A–C — see [`docs/MOBILE.md`](./docs/MOBILE.md))
- [OPEN] `emit-mobile` codegen scaffold — native iOS/Android from the
  same `struct`/`screen` declarations that drive the web UI today

**Track F — Next-generation language & UI architecture** (design
discussion, independent of every track above — see
[`docs/NEXT_GEN.md`](./docs/NEXT_GEN.md))
- [OPEN] A target-independent UI manifest with multiple renderers
  (web/TUI/mobile), not just today's one fixed web template
- [DONE] A real module/package system — `module Ident { ... }`
  namespacing, `pub` visibility, and `use "path.nir"` splitting a
  program across files, all real and tested — see `docs/ROADMAP.md` Track
  F, F2. The legacy `module "Display Name" { ... }` nav-label form
  (still just a nav label, no scoping) is untouched and still works.
- [OPEN] A composable UI layout system — Phase A [DONE]: `screen
  <Struct> { layout { row { column { group "..." { field x } } } } }`,
  real containers (row/column/grid/group/tabs), plus a searchable +
  scroll-paginated dropdown, a live timeline widget, and colored status
  badges — see `docs/ROADMAP.md` Track F, F4. A per-element `css: "..."`
  styling override and the rest of the widget catalog are still open.

---

## How to help

Pick an `[OPEN]` item above, comment on its GitHub issue (or open one
if it doesn't exist yet), and say what you're picking up before
starting on anything non-trivial. See
[CONTRIBUTING.md](./CONTRIBUTING.md).

Docs, examples, and `.nir` test cases are just as valuable as compiler
work and are the fastest way to make a first contribution.
