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
> functions), `nfr(...)` (non-functional requirements as a compiled
> fn annotation with real APM-kernel tracking + escalation), a minimal
> compiled `serve` mode (real HTTP traffic routed to compiled functions,
> no interpreter), real `oidc_validate_token`/`extract_claim`/
> `identity_expired` (genuine JWT/JWKS signature verification, closing
> the "authentication, not just authorization" gap the identity work
> above used to name), real `db`/`json` (SQLite via `rusqlite`,
> `examples/features/27_database.nir` verified end to end against a
> real database), `transact` Layer 1 (real precheck/network/verify/
> commit/compensate/log control flow — `retry`/`timeout` architecturally
> blocked by the compiled trap model, not just deferred), and real
> `mq`/`http`/`https` (Redis, and an HTTP(S) client with vendored TLS
> and chunked-transfer-encoding decoding, both verified against real
> servers) — see `docs/LANGUAGE.md` §5/§6a/§6e/§6f/§10, `docs/TRANSACT.md`,
> and `docs/PHASE0.md`'s "Twentieth" through "Twenty-sixth" updates for
> the full detail this list doesn't yet reflect below.

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
- [DONE] `nirdosha verify <file.nir>` (2026-09) — a standalone,
  machine-readable verdict over typecheck/ownership/`validate`-contract
  results plus Z3 Tier-1 proof-obligation counts: JSON on stdout, no
  LLVM/clang toolchain and no binary produced. The first concrete step
  of the "sell the verdict, not the syntax" trust-layer direction —
  turns the compiler into something CI or an agent's own repair loop can
  call directly. See `docs/LANGUAGE.md` §1.
- [DONE] **Three-valued verdict** (2026-09, master plan Part 3 Sprint
  0) — `verify`'s top-level `verdict` field and exit code are now
  `PROVED`/`DISPROVED`/`UNKNOWN` (exit `0`/`1`/`2`), never a binary
  pass/fail: a `validate` obligation Z3 can't model (a non-integer
  parameter, today) reports `UNKNOWN`, not a silent `PROVED` the way the
  first cut of `verify` reported it. Parity target: Velvet, C Proof.
  Z3 counterexamples were already embedded in the verdict's
  `contracts.obligations[].detail` from `verify`'s first cut (parity
  target: Imandra, Theorem) — both Sprint 0 items now closed, alongside
  the JSON verdict + exit code item above.
- [PARTIAL] `nirdosha fix` (2026-09, master plan Part 3 Sprint 1) —
  byte-offset `FixPatch`es (`token::Span::byte`) plus three fixability
  classes modeled on `rustc`'s own `Applicability`
  (`auto`/`assisted`/`manual`, `main.rs`'s `Applicability` doc comment),
  `--apply` writes every `Auto` patch to disk (highest byte offset
  first, so earlier patches' ranges never shift under a later one) and
  re-verifies afterward. Parity target: Kōdo. v1's one real fixability
  analysis: unknown-identifier typo correction by edit distance
  (`fix_unbound_identifier`) against names in scope — an unambiguous
  single closest candidate is `auto`, a tie is `assisted` (names both,
  picks neither), nothing close enough is `manual`. `[PARTIAL]` because
  every other diagnostic kind (parse errors, ownership violations,
  counterexamples, unsupported obligations) still reports `fix: null`,
  honestly, rather than a fabricated `manual` implying analysis that
  hasn't been built yet. Tests:
  `crates/compiler/tests/fix_command.rs` (CLI-level, including a
  regression test for a real bug caught by hand-running this end to
  end: an earlier revision's `--apply` only scanned
  `contracts.obligations` for `Auto` patches, silently dropping every
  one attached to a `load`/`typecheck`/`ownership` diagnostic instead).
- [PARTIAL] `nirdosha explain [<code>]` (2026-09, master plan Part 3
  Sprint 1) — a curated, hand-written index (`explain::REGISTRY`),
  `NIR0001`-`NIR0012` mapped 1:1 to `agent-skills/nirdosha/AGENTS.md`'s
  twelve numbered "rules that will break your output," plus `NIR0013`
  for the unbound-identifier/typo case `fix` already automates. Parity
  target: Kōdo, Midspiral. Modeled on `rustc --explain`'s real
  precedent (a curated subset of diagnostics get long-form docs, most
  don't) rather than a mechanical dump of every internal error variant.
  `[PARTIAL]` because only three diagnostic sites auto-attach a `code`
  to `verify`/`fix`'s own JSON output today — `NIR0002` (`str` in a
  `fn` signature), `NIR0012` (reserved word as identifier, recovered
  from `loader::load_program`'s formatted error string via a narrowly-
  scoped substring match, since `ParseError` doesn't carry a structured
  code field yet), `NIR0013` (unbound identifier) — every other
  registry entry is real, browsable reference material
  (`nirdosha explain NIR0009` works today) that nothing auto-tags onto
  a live diagnostic yet. Tests: `crates/compiler/tests/explain_command.rs`.
- [DONE] `nirdosha mcp` (2026-09, master plan Part 3 Sprint 1) — an MCP
  server on the stdio transport (JSON-RPC 2.0, newline-delimited),
  parity target: Acutis, Imandra, Kōdo. Four tools, all `source`-based
  (never a filesystem path, matching how an MCP client actually calls
  a tool): `verify_code`, `get_grammar` (the real `nirdosha.gbnf`),
  `fix` (with `apply: true` returning `patched_source` instead of
  writing a file — there's no file in an MCP call to write back to),
  and `describe` (a curated fn/struct/enum/`validate` structural
  summary, parse-only, modeled on Kōdo's own `kodo.describe` tool —
  distinct from `emit-ast`'s full span-carrying AST). Every tool
  reuses the exact `run_verify_pipeline`/`write_auto_patches` the CLI
  commands run, including a shared fix applied via the same helper
  `nirdosha fix --apply` uses (extracted, not duplicated, specifically
  to avoid a second copy of the byte-offset-ordering bug `nirdosha
  fix`'s own changelog entry above already names once). Tests:
  `crates/compiler/tests/mcp_server.rs` — spawns the real binary and
  speaks the stdio transport directly (initialize/tools list/tools
  call/notifications/malformed input/unknown method).
- [DONE] `nirdosha certify <file.nir>` (2026-09, master plan Part 3
  Sprint 1) — Certificate v0, parity target: Velvet, Kōdo. A
  deterministic JSON attestation over the same `run_verify_pipeline`
  result `verify`/`fix`/`mcp` share: real SHA-256 `source_hash`/
  `grammar_hash` (reproducible by a third party, never a timestamp or
  the caller's own file path), `toolchain_version`, an `evidence_tier`
  field (`proved`/`checked`/`sampled`/`unknown` — only the first and
  last are reachable today; `checked`/`sampled` are reserved so the
  schema doesn't have to break when the CHECKED tier lands post-seed),
  and a compact `verdict_summary`. Issues a certificate for every
  verdict including `DISPROVED` — a conclusive counterexample is real,
  conclusive evidence, not withheld until the code passes. Tests:
  `crates/compiler/tests/certify_command.rs` (all three verdicts'
  evidence tiers, real-hash verification against the shipped
  `nirdosha.gbnf`, byte-for-byte determinism across two runs, and that
  the caller's own file path never leaks into the certificate).
- [PARTIAL] PyPI thin client `nirdosha-verify` (2026-09, master plan
  Part 3 Sprint 1, parity target: dottxt/Outlines' distribution model)
  — `clients/python/nirdosha-verify/`: a real, tested, buildable Python
  package (`pip install -e .` and `python -m build` both verified —
  `pyproject.toml`/hatchling, a wheel + sdist actually built) wrapping
  `nirdosha verify`/`fix`/`certify` as `nirdosha_verify.verify()`/
  `fix()`/`certify()` plus a `nirdosha-verify` console-script
  passthrough. `[PARTIAL]`, honestly: it is not yet published to PyPI
  (a real, external, one-way action nobody has asked for yet) and does
  not bundle/download a prebuilt binary — v0 expects `nirdosha` already
  on `PATH` or pointed to via `NIRDOSHA_BIN`; auto-fetching a release
  binary needs nirdosha to actually publish tagged release binaries
  first (`docs/STABILITY_AND_RELEASES.md`'s monthly cadence). Tests:
  `clients/python/nirdosha-verify/tests/test_verify.py`, run against
  the real locally-built binary (9/9 passing, including the console
  script and a real `python -m build` producing an installable wheel).

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
- [DONE] `oidc_validate_token`/`extract_claim`/`identity_expired` —
  real `jsonwebtoken`-backed JWT/JWKS signature verification against a
  static JWKS (RSA/EC-P256/oct, one key per `kid`, algorithm locked by
  the JWK's own `kty` — closes the classic algorithm-confusion attack),
  real JSON claim extraction, a real `VerifiedIdentity` driving the
  `check_role`/`acquire` machinery above end to end from a genuine
  signed token, not just a hand-built `VerifiedIdentity`. `check_role`
  itself upgraded alongside this from a comma-separated-list
  simplification to real JSON-array parsing (falling back to the
  comma-separated form for backward compatibility).
- [DONE] A minimal compiled `serve` mode — `str_index_of`/`str_slice`,
  `len(str)` (three new string primitives) hand-parse an HTTP request
  line over a real `tcp_listener`/`accept` loop, routing `/api/<fn>` to
  distinct compiled functions by a plain `.nir` `if`/`else if` chain, no
  new language construct. GET-only, no `Content-Length`/POST body
  support — see `examples/features/51_compiled_serve.nir`, real-`curl`-
  verified. This was the first cut, kept as a worked example, not the
  final word: a full production HTTP engine (real `POST`/body parsing,
  CORS, keep-alive, rate limiting) landed as a 2026-09 follow-up behind
  `nirdosha build --serve` — see B8 under "In progress / next" below.

**Backend/services**
- [PARTIAL] `db` (SQLite + Postgres) + `json` — 2026-09: real, both
  compiled — `db_connect`/`db_query`/`db_execute` and all 9 `json_*`
  accessors, `examples/features/27_database.nir` verified end to end
  against a real SQLite database. Postgres landed as a 2026-09
  follow-up too — real, pooled (`kernel::pool::PoolRegistry`, `:memory:`
  deliberately unpooled), vendored-TLS connections (`docs/adr/0005`).
  Still open: a zero-payload `enum` bind value isn't compiled yet, and
  `BLOB` columns come back as JSON `null` (no `bytes` type to carry
  them).
- [DONE] `http`/`https` + `mq` (Redis) — 2026-09: real, both compiled —
  `http_get`/`http_post`/`https_get`/`https_post` (vendored OpenSSL,
  chunked-transfer-encoding decoding, real keep-alive + pooling,
  `docs/adr/0006`) and `mq_connect`/`mq_publish`/`mq_consume`, both
  verified end to end against real servers.
- [DONE] Identity — 2026-09: OIDC/JWT validation (`oidc_validate_token`)
  and claim extraction (`extract_claim`) are real — `jsonwebtoken`-
  backed signature verification against a static JWKS (live
  rotation/refresh still open), same `check_role` +
  `requires(role/claim:...)`/`acquire` machinery already compiled. The
  rest of Row 12 shipped as a 2026-09 follow-up (`docs/adr/0007`): real
  dotted-path claim lookup (`check_role_path`/`extract_claim_path`),
  real unpredictable session ids, real server-side single-use
  refresh-token redemption, fail-open revocation, and constant-time
  API-key validation — see the identity section above. Still open: an
  admin-editable role-mapping cache (IdP role names → app role names).
- [DONE] `transact` — 2026-09: Layer 1 real and compiled
  (`precheck?/network/verify/commit/compensate?/log?`, a real `bool`
  result), plus a real fsync'd WAL-mode SQLite durability log, compiler-
  synthesized crash replay dispatched from generated `main`'s own
  prologue, and bounded retry-with-backoff for `commit`/`compensate`
  when their return type is `Result(_, _)` — verified against real
  compiled binaries, including running the same binary twice as two
  separate OS processes sharing one durability log to prove replay
  actually recovers a row a prior process left `commit_pending` after
  exhausting its own retry budget (`docs/adr/0009`). `network`'s own
  `retry`/`timeout` stay a real architectural gap in the compiled trap
  model, not a deferred nicety (a compiled trap is an unconditional
  abort — see the identity section's own note on why compiled traps
  can't be caught). Two disclosed, deliberate narrowings, not silent
  gaps: replay dispatch has no build-version fingerprint guard yet, and
  the durability log is local-SQLite only, unsafe under this repo's own
  multi-replica deployment overlay.
- [PARTIAL] `workflow` — 2026-09: Layer 1 real and compiled — durable
  state machines (`start_*`/`advance_*`, `on_entry`/`on_exit`, ordinary
  transitions, `terminal` states), `send_email`/`send_sms`/`send_push`/
  `notify` (real authenticated HTTPS POSTs against a live provider row),
  and `state { sla_seconds: N }` + `list_<workflow>_overdue()` SLA/
  escalation detection (an external scheduler still has to poll it and
  fire the escalation itself — no scheduling/cron primitive exists in
  the language). Still [NOT RUNNABLE]: a non-empty `data { ... }` block,
  magic-link (`link`-marked) transitions, `owner:` state-ownership
  enforcement, and the `pending_for_me`/`submitted_by_me`/`history`
  queue-UI read fns (compiled, but always a real `Err` — no durable
  storage for any of these in this Layer 1 runtime yet).
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
anything on. A compiled `serve` (Track B8) now exists, so the items
below are live production-hardening work again, not a record kept for
a hypothetical future.)
- [DONE] `transact` durability under a real crash-shaped condition —
  `crates/compiler/tests/codegen.rs`'s
  `transact_replay_finishes_a_commit_pending_row_left_by_a_prior_process`
  runs the same compiled binary twice as two separate OS processes
  sharing one durability log: the first exhausts its retry budget and
  leaves a row `commit_pending`, the second's own `main` prologue
  replays and finishes it before that second process's own body ever
  runs (`docs/adr/0009`). Not a literal `SIGKILL` mid-write — a real,
  disclosed narrower verification, not overclaimed as one.
- [OPEN] A deployment story for a *compiled* `serve` (containerization,
  secrets/JWKS handling) — `nirdosha serve` itself no longer exists
- [PARTIAL] Observability — a local OTel-shaped tracer exists; wiring
  to a real collector (OTLP) is open
- [DONE] A compatibility/versioning policy before the next breaking
  language change — `docs/STABILITY_AND_RELEASES.md` (2026-09):
  monthly tagged releases starting 2026-10-01, a named checklist for
  what `v1.0` requires, and a breaking-change policy that already
  covers `nirdosha verify`'s JSON verdict schema
- [PARTIAL] Identity admin console — role-mapping cache is done;
  multi-IdP registry and a roles→functions/fields report are open
- [OPEN] Real Windows verification — the compiled `tcp`/`tcp_listener`
  runtime was ported to Windows' `RawSocket` API (v0.1.0-alpha.3) but
  has never run on a real Windows machine; needs an actual test pass
- [OPEN] macOS binaries link system Z3 instead of vendoring it —
  `z3-src` 416.0.2 fails to build against current AppleClang (a real
  upstream incompatibility); revisit once a fixed `z3`/`z3-src` release
  ships

**Track B — Full compilation** (sandboxing (B6) is the one item left
with no codegen at all, and with the interpreter gone doesn't run in
any form — everything else this track originally scoped now has real
codegen: the numeric/control-flow subset, `tcp`/`tcp_listener`, `file`,
scalar-only native plugin calls, `dec128` arithmetic, basic concurrency
— `thread`/`spawn`/`join`, `chan`/`send`/`recv`, `froze` — `check_role`,
field- and function-level `requires(...)`/`acquire`, `nfr(...)`, a real
compiled `serve` engine (RFC 0010's route-exposure model wired to a
full HTTP server, not just the minimal hand-rolled one), the three
string primitives that minimal server first needed
(`str_index_of`/`str_slice`, `len(str)`), real JWT/OIDC identity
verification plus the rest of Row 12 (sessions/refresh/revocation/API
keys), `db` (SQLite + Postgres)/`json`, `mq`, `http`/`https`, and
`transact` including its durability log and crash replay — see the
identity section under "Shipped" above and B1/B2/B3/B5/B8 below)
- [DONE] `file` (`open`/`send`/`recv`/`stop`) — linked `nir_file_*`
  kernels, the same "declare + link a staticlib" pattern `tcp` already
  used; `examples/features/24_file_io.nir` compiles and runs as a real
  native binary (real writes, real append, real read-to-EOF). Shipped
  2026-09-05 with no automated regression test at all — closed this
  session: `crates/compiler/tests/codegen.rs`'s
  `compiled_file_open_write_append_read_round_trips_real_bytes_on_disk`
  compiles and runs the same program, then reads the file back off disk
  independently to confirm the compiled binary actually wrote it, not
  just that it exited `0`.
- [DONE] Basic concurrency — `thread`/`spawn`/`join`, `chan`/`send`/
  `recv` (2026-09), plus RFC 0006 Pillar 1's `froze`. Contrary to this
  section's own earlier framing below ("not a kernel to link"), it
  shipped *as* a kernel to link: `runtime-kernels`' `nir_thread_spawn`/
  `nir_thread_join`/`nir_chan_*`, a real admission ceiling
  (`Domain::Thread`), and a dynamic global-stall deadlock detector
  (`docs/LANGUAGE.md` §7/§10, `rfcs/0007-apm-runtime-kernel.md` §8).
  Word-sized payloads/arguments/results only; `sandbox` is unaffected,
  still open below.
- [DONE] Native plugin calls (Kind A plugins) —
  `plugin::NativePluginBuiltin`/`codegen::build_with_native_plugins`
  (`rfcs/0005-plugin-boundary-safety-and-performance.md` §3), ~250x
  faster than interpreted plugin dispatch ever was for the scalar
  subset; widened past scalars to `str`, `handle(Kind)`, and
  `&handle(Kind)` (`rfcs/0008-native-plugin-abi-widening.md` Phase 1,
  two real reference crates: `crates/plugin-example-native-{shout,kv}`)
  — there is no interpreter left for anything to fall back to.
  Aggregate (`Vector`/`Matrix`)-typed plugin builtins remain
  unsupported; Cargo-driven auto-discovery of a plugin crate is still
  open (`rfcs/0008` Phase 3).
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
- `[DONE]` **B1. `transact` codegen** (2026-09) — Layer 1 real:
  `precheck?/network/verify/commit/compensate?/log?`, a real `bool`
  result, `examples/features/36_transact.nir` unmodified and verified.
  `network`'s `retry`/`timeout` are architecturally blocked in the
  compiled trap model (a compiled trap is an unconditional abort, and
  `network`'s return type can never be `Result(_, _)`) — rejected
  explicitly. A real fsync'd WAL-mode SQLite durability log, compiler-
  synthesized crash replay from generated `main`'s own prologue, and
  bounded retry-with-backoff for `commit`/`compensate` when their
  return type is `Result(_, _)` shipped as a 2026-09 follow-up
  (`docs/adr/0009`) — see the "Shipped" section's own `transact` entry
  above for the verification detail. Two disclosed, deliberate
  narrowings, not silent gaps: no build-version fingerprint guard on
  replay dispatch yet, and the durability log is local-SQLite only,
  unsafe under this repo's own multi-replica deployment overlay.
- `[DONE]` **B9. `sleep_ms` codegen** (2026-09) — a real wall-clock
  sleep; needed for `transact`'s own future backoff work.
- `[DONE]` **B3. `mq` codegen** (2026-09) — `mq_connect`/`mq_publish`/
  `mq_consume` (Redis, `LPUSH`/`BLPOP`), verified against a real local
  Redis instance.
- `[DONE]` **B5. `http`/`https` codegen** (2026-09) — real client, both
  plain and TLS (vendored OpenSSL — found necessary by an actual link
  failure against system OpenSSL, decided deliberately not left to
  deploy time), chunked-transfer-encoding decoding.
- [PARTIAL] **B2. `db` + `json` codegen** (2026-09) — `db_connect`/
  `db_query`/`db_execute` (SQLite via `rusqlite`'s `bundled` feature) and
  all 9 `json_*` accessors, real and verified: `examples/features/
  27_database.nir`, unmodified, compiles and runs against a real
  in-memory SQLite database (schema creation, parameterized insert/
  update, a filtered `SELECT`, a connection failure as a real `Err`).
  `Ty::Json` compiles as raw text, re-parsed by each accessor — this
  item's own design note ("`str` + shims, not a new runtime value type")
  landed as planned. Postgres landed as a real 2026-09 follow-up too —
  pooled (`kernel::pool::PoolRegistry`), vendored-TLS (`postgres`/
  `postgres-native-tls`, verify-by-default off-`localhost`,
  `docs/adr/0005`). Named gaps still open: a zero-payload `enum` bind
  value not yet compiled; `BLOB` columns represented as JSON `null` (no
  `bytes` type to carry them).
- [DONE] **B8. Compiled `serve` mode** (2026-09) — this was never
  actually gated on the rest of Track B (that was a sequencing choice,
  not a technical one), and shipped in two real layers, both still
  present and both real, not one superseding the other:
  - The original minimal layer: `/api/<fn>` routing over a real
    `tcp_listener`/`accept` loop, request-line parsing via three new
    string primitives (`str_index_of`/`str_slice`, `len(str)`), routing
    to distinct compiled functions via a plain `.nir` `if`/`else if`
    chain — no new language construct, real-`curl`-verified
    (`examples/features/51_compiled_serve.nir`). Still GET-only,
    sequential, no string concatenation — a deliberately-kept
    primitives-first worked example, not the production path.
  - The production path, `nirdosha build --serve`: RFC 0010's
    `serve { expose ... }` exposure model (deny-by-default on an
    ungated mutating route, `typeck::exposed_fn_names`) wired to a real
    HTTP engine (`crates/compiled-serve`) — real `POST`/`Content-Length`
    body parsing, CORS (never a wildcard on a credentialed response),
    keep-alive with a `max_requests_per_connection` policy, a per-IP
    rate limiter, a `413` body-size cap, `Domain::ServeHttp` admission
    (fail-fast `503`, never a silent stall), and a real `Set-Cookie` —
    the exact gaps the minimal layer above named are closed here, not
    fixed in place. `transact` durability log init and crash replay run
    from this mode's own generated `main` before the listener ever
    binds, ahead of any request. Doesn't own RBAC/JWT verification
    itself — each route's `requires(...)` check runs inside the
    compiled function it dispatches to, same as everywhere else.
    Verified end to end (real `nirdosha build --serve`, a real `GET`
    and a real `POST` carrying a `Content-Length` body both reaching
    the same exposed `fn`, an unexposed path real-404ing):
    `crates/compiler/tests/codegen.rs`'s
    `compiled_serve_production_path_exposes_a_route_via_a_real_http_post_with_a_body`
    — added this session; before it, only the minimal layer above had
    automated coverage (`compiled_serve_routes_by_path_to_two_compiled_functions`,
    which exercises `51_compiled_serve.nir`'s own hand-rolled style, not
    this production path).
- [PARTIAL] **B10. `workflow` codegen** (2026-09) — Layer 1 real:
  `start_*`/`advance_*`, `on_entry`/`on_exit`, ordinary transitions,
  `terminal` states; `send_email`/`send_sms`/`send_push`/`notify` (real
  authenticated HTTPS POSTs against a live provider row); `state {
  sla_seconds: N }` + `list_<workflow>_overdue()` SLA/escalation
  detection (an external scheduler still has to poll it and fire the
  escalation itself). Verified: `examples/features/52`–`54`. Named
  gaps: a non-empty `data { ... }` block and `link`-marked (magic-link)
  transitions are explicitly rejected (no durable storage for either in
  this Layer 1 runtime); `pending_for_me`/`submitted_by_me`/`history`
  compile but are always a real `Err` (same gap); `owner:` state
  ownership isn't enforced at runtime.
- [DONE] Identity's own remainder (dotted-path claim lookup, sessions/
  refresh/revocation/API-key validation, `docs/adr/0007`) and
  `transact`'s own durability remainder (durability log, crash replay,
  `commit`/`compensate` retry-with-backoff, `docs/adr/0009`) — this
  note originally flagged them as unordered, open work; both shipped in
  2026-09, and neither needed the sequencing this note originally
  proposed, same as `db`/`json` (B2), compiled `serve` (B8), `transact`
  Layer 1 (B1), `mq` (B3), `http`/`https` (B5), and `workflow` Layer 1
  (B10) before them.
- [OPEN, DESCOPED FROM v1] `sandbox` (a real, separate OS *process*, not
  a thread) — 2026-09 decision: explicitly out of scope for the first
  production release, not merely unstarted. It remains a materially
  harder, separate design question from the concurrency work already
  shipped (see `rfcs/0005` §0's own difficulty ranking), and unlike
  every other item above, it has zero existing compiled-backend
  scaffolding — the interpreter-era implementation that made
  `sandbox`/`stop`/cross-process `chan` real was deleted along with the
  rest of `interpreter.rs`, so this is new kernel work (process spawn/
  kill/reap, a re-exec worker protocol), not a port of an
  already-proven design the way every other Track B item was. Nothing
  else in the compiled backend depends on it, so shipping without it
  costs nothing beyond the two catalog examples that need it staying in
  their already-disclosed "not runnable" state — tracked as future,
  post-v1 work.

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
