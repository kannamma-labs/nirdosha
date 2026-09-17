# Nirdosha — gaps between documentation and code

Generated 2026-09-17 against `/home/arun/nirdosha-rt` (`rt-dialect/stage-1` free lane).

## Scope and ground rules

- **System under evaluation:** the v2 Rust dialect (`nirdosha-rt`, `nirdosha-macros`, `nirdosha-driver`, `cargo-nirdosha`). The standalone `.nir` compiler in `crates/compiler` is treated as **deprecated** per `docs/ROADMAP.md`'s deprecation note (2026-09-17), but its code is referenced when it still implements something the v2 lane lacks.
- **Assumption per request:** "compilers are no more within the system — we have `rustc` + macro + driver." A gap that can be closed by writing a proc-macro / attribute macro / rustc-driven analysis (without inventing a new runtime kernel or network service) is marked **ACHIEVABLE VIA RUSTC/MACRO/DRIVER**.
- **Excluded per request:** `sandbox` / process isolation is **not** audited here.
- **Method:** every row was checked against real source or a concrete absence, not inferred from a doc's own status tag.

## Legend

| Tag | Meaning |
|-----|---------|
| `IMPLEMENTED` | Real code exists in the v2 lane and is exercised by tests/examples. |
| `DEPRECATED-ONLY` | Real code exists, but only in the deprecated `crates/compiler` `.nir` compiler. Not available in the v2 Rust dialect. |
| `MISSING` | Documented/required, no code anywhere, and not automatically supplied by `rustc`/macro/driver. |
| `PARTIAL` | Some code exists, but the documented end-to-end requirement is not fully met. |
| `ACHIEVABLE VIA RUSTC/MACRO/DRIVER` | No code yet, but can be implemented without a new runtime service — by a proc-macro, rustc name-resolution/dataflow, or cargo-metadata integration. |

---

## 1. V2 dialect language surface

### 1.1 Contract clauses (`#[contract(...)]`)

| Requirement | Doc claim | Code truth | Tag | Notes / evidence |
|-------------|-----------|------------|-----|----------------|
| `effects(pure)` Stage-1 body scan | Macro catches obvious lies as `compile_error!` even under plain `cargo`. | `nirdosha-macros/src/lib.rs:174-188` runs a path-based body scan and emits `compile_error!`. | `IMPLEMENTED` | |
| `effects(pure)` Stage-2 interprocedural check | Driver resolves transitive call graph and names the full chain. | `nirdosha-driver/src/main.rs` does local call-graph traversal only; no transitive dependency summaries; external calls default-deny unless in `std_effects.rs`. | `PARTIAL` | Real dependency effect summaries are the missing piece. The local check is implemented. |
| `requires(role = "...")` unforgeable proof | Type-level `RoleProof<R>` injected by macro; only `Auth::prove` mints it. | `nirdosha-macros/src/lib.rs:199-209`, `nirdosha-rt/src/role.rs:74-91`. | `IMPLEMENTED` | |
| `requires(claim = "...", "...")` / claim proofs | Doc form grammar supports `claim`; `.nir` had `ClaimView`. | 2026-09-17: `nirdosha-contract-core::claim`/`naming` (name/value validation, mechanical `PascalCase(name)++PascalCase(value)` type-ident derivation), `parse.rs`'s `requires(claim = "..", "..")` clause, `nirdosha-macros`' `&ClaimProof<C>` injection, `nirdosha-rt::role::{Claim, ClaimProof}` + `Auth::with_claim`/`prove_claim`, the `claims!` macro, and `web::Router::{get,post,put,delete}_gated_claim`. Proven end-to-end: `crates/nirdosha-rt/tests/claims.rs` (a real `#[contract(requires(claim = ..))]`-gated fn, minted/missing/wrong-value proofs) and `web.rs`'s own `claim_gated_route_enforces_the_real_claim_proof`. | `IMPLEMENTED` | See `rfcs/0022-web-layer-hardening-and-switchable-transport.md`'s follow-up-work note (now closed) for the design. |
| `requires(expr)` / `ensures(expr)` | Hoare pre/post over MIR, issue #68. | `nirdosha-contract-core/src/model.rs` has `Requires`/`Ensures` with expression form. Macro emits a dead sibling function for type checking (`nirdosha-macros/src/lib.rs:210+`). Driver discharges over MIR (`nirdosha-driver/src/numeric.rs`, `tests/contract_predicates.rs`). | `IMPLEMENTED` | Field/index/method access in predicates still unsupported (documented). |
| `nfr(latency_ms/error_rate_max/throughput_min_per_sec/concurrency_max)` | Runtime guard + flight recorder. | `nirdosha-rt/src/nfr.rs`, `nirdosha-macros/src/lib.rs` injects `nir_nfr_call_begin/end`. `cargo nirdosha bench` evaluates p95. | `IMPLEMENTED` | |
| `resource(kind = "...")` dataflow check | `rustc_mir_dataflow` forward analysis proving acquire/release pairing. | `nirdosha-driver/src/dataflow.rs`, `nirdosha-driver/tests/resource_dataflow.rs`. | `IMPLEMENTED` | |

### 1.2 Dialect-wide restrictions

| Requirement | Doc claim | Code truth | Tag | Notes / evidence |
|-------------|-----------|------------|-----|----------------|
| Reject `unsafe`, raw threads, raw locks, static state, reference writes | Driver conservative local subset rejects these. | `nirdosha-driver/src/main.rs:446-540` rejects unsafe, async, destructors, statics, writes through references, unresolved dispatch. | `IMPLEMENTED` | |
| Reject destructors / drop glue | "Any value needing drop glue is rejected outright." | `nirdosha-driver/src/main.rs:516-540` rejects `TerminatorKind::Drop`. | `IMPLEMENTED` | |
| Expanded macros and build-generated code checked | "Macros are checked after expansion." | Driver sees expanded HIR/MIR, but Stage-1 source scanner (`nirdosha-contract-core/src/scan.rs`) does **not** expand macros. | `PARTIAL` | Stage-1 cannot certify generated bodies. |
| Async fns contract-verified | Documented as **not yet** supported; macro refuses them. | `nirdosha-driver/src/main.rs:446` rejects async functions in the pure subset. | `IMPLEMENTED` (as documented limitation) | |

---

## 2. `cargo nirdosha` tooling and certificates

### 2.1 Commands

| Requirement | Doc claim | Code truth | Tag | Notes / evidence |
|-------------|-----------|------------|-----|----------------|
| `cargo nirdosha build \| check \| run \| test \| verify` | Per-package verification. | `crates/cargo-nirdosha/src/main.rs:51-56`. | `IMPLEMENTED` | |
| `cargo nirdosha verify --workspace` | Strict gate over all in-dialect crates + aggregate certificate. | `crates/cargo-nirdosha/src/main.rs:536-590` and `crates/cargo-nirdosha/src/lib.rs` implement workspace scan. | `IMPLEMENTED` | |
| `cargo nirdosha verify --audit` | Re-check certificate source hashes/binding. | `crates/cargo-nirdosha/src/main.rs:403-460`. | `IMPLEMENTED` | |
| `cargo nirdosha bench` | NFR `latency_ms` CI gate using package test suite as workload. | `crates/cargo-nirdosha/src/main.rs:592-670`. | `IMPLEMENTED` | |
| `cargo nirdosha check-certificate --require <guarantee>` | Consumer policy gate; rejects unsupported guarantees. | `crates/cargo-nirdosha/src/main.rs:155-195`, `crates/nirdosha-contract-core/src/evidence.rs`. | `IMPLEMENTED` | |
| `cargo nirdosha keygen` / `verify-certificate` | Ed25519 certificate signing. | `crates/cargo-nirdosha/src/main.rs:198-295` uses `nirdosha_audit::signing`. | `IMPLEMENTED` | |
| Signed trust chain / issuer authentication policy (issue #13) | "Entry #13's signed-plugin trust chain — reserved, always `None` in v1." | `nirdosha-contract-core/src/certificate.rs:95`, `crates/cargo-nirdosha/src/lib.rs:322-324`. | `MISSING` | Needs a trust-anchor format + signed-plugin verification path. |

### 2.2 Evidence and provenance

| Requirement | Doc claim | Code truth | Tag | Notes / evidence |
|-------------|-----------|------------|-----|----------------|
| Source hash binding | Certificate binds SHA-256 of every source file. | `nirdosha-contract-core/src/certificate.rs`, `crates/cargo-nirdosha/src/lib.rs`. | `IMPLEMENTED` | |
| Dependency-closure + toolchain binding | `--provenance` hashes `Cargo.lock` and records `rustc --version`. | `nirdosha-contract-core/src/provenance.rs`, `crates/cargo-nirdosha/src/lib.rs`. | `IMPLEMENTED` | |
| Built executable bytes binding | G6: "artifact-bytes + cfg binding … pending." | Not implemented. | `MISSING` | Requires post-link artifact hash + target/feature/cfg capture. |
| Authenticated issuer policy | G6: "authenticated issuer identity is entry #13's signed-plugin trust chain and remains pending." | Not implemented. | `MISSING` | |
| Deterministic, timestamp-free certificates | Same sources + tool → byte-identical certificate. | Implemented; no timestamps in `certificate.rs` schema. | `IMPLEMENTED` | |
| Bound per-assertion MIR proof records | `proofs` array with backend name. | `nirdosha-driver/src/proof_certificate.rs`, used when `--deep` emits driver certificates. | `IMPLEMENTED` | |
| Guarantee bundle (`guarantees-<pkg>.json`) | Artifact traveling with the build. | Mentioned in `docs/nirdosha-rt-dialect.md`; no code found generating a separate `guarantees-*.json` file. | `MISSING` | The coverage is inside the certificate, but the standalone bundle file is not produced. |

---

## 3. Runtime (`nirdosha-rt`) and native adapters

### 3.1 Identity and authorization

| Requirement | Doc claim | Code truth | Tag | Notes / evidence |
|-------------|-----------|------------|-----|----------------|
| `Auth::login` caller-asserted roles for demo | `Auth::login(user, roles)` mints an `Auth` that can prove roles. | `nirdosha-rt/src/role.rs:74-91`. | `IMPLEMENTED` (demo-grade, as documented) | |
| Real authority boundary / trusted host identity | `native::Authority` verifies JWT signatures, issuer/audience, time validity before authorizing. | `nirdosha-rt/src/native.rs:11-21` exposes `AuthorizedSaga` wrapping `DurableSaga`. | `IMPLEMENTED` (native feature) | |
| Role denial without side effects | Authorization rechecks before any log insert or external call. | Claimed in `docs/V2_GUARANTEES.md`; tests in `crates/nirdosha-rt/tests/native_guarantees.rs`. | `IMPLEMENTED` | |
| Claims-based identity (`extract_claim`, `check_claim`) | `.nir` Row-12 had `ClaimView`; dialect doc mentions claim keys. | 2026-09-17: `nirdosha-rt/src/role.rs`'s `Claim`/`ClaimProof`/`Auth::prove_claim`, `nirdosha_rt::claims!`, and macro injection — see the `requires(claim = ...)` row above for the full file list. | `IMPLEMENTED` | |
| Multi-IdP registry | Old compiled-serve read `NIRDOSHA_IDENTITY_PROVIDERS` JSON. | No equivalent in `nirdosha-rt`. | `MISSING` | Runtime config loading; achievable with ordinary Rust + env var, but not a macro-only job. |

### 3.2 Transactions and isolation

| Requirement | Doc claim | Code truth | Tag | Notes / evidence |
|-------------|-----------|------------|-----|----------------|
| `DurableSaga` native WAL + crash replay | Native runtime-kernels adapter; kill/restart tests. | `nirdosha-rt/src/native.rs:5`, `crates/nirdosha-rt/tests/native_guarantees.rs`. | `IMPLEMENTED` (native feature) | |
| `WorkerProcess` owned Unix process lifecycle | Launch explicit executable, stop/Drop kill process group. | `nirdosha-rt/src/native.rs:6`, tests verify stop/Drop release durable-log lock. | `IMPLEMENTED` (native feature) | |
| Non-Unix launch | "Non-Unix launch is unsupported." | No Windows/JobObject implementation. | `MISSING` | Needs OS-specific runtime kernel work. |
| Process filesystem/network confinement | "supplies process separation, not a permissions sandbox." | Not implemented (and `sandbox` is excluded from this audit). | `MISSING` | |

### 3.3 UI / web runtime

| Requirement | Doc claim | Code truth | Tag | Notes / evidence |
|-------------|-----------|------------|-----|----------------|
| `Router::serve_until` with worker pool + shutdown | Concurrent HTTP, 64 workers, 5s timeouts, drain on stop. | `nirdosha-rt/src/web.rs:767+`. | `IMPLEMENTED` | |
| `communication_feed!` long-poll | Waits for generated POST or timeout; revision cursor. | `crates/nirdosha-macros/src/communication_feed.rs`, `nirdosha-rt/src/feed.rs`. | `IMPLEMENTED` | |
| Server-rendered screens (CRUD, dashboard, wizard, kanban, settings, landing) | Six archetypes. | `crates/nirdosha-macros/src/crud_screens.rs`, `dashboard.rs`, `wizard.rs`, `kanban_board.rs`, `settings_screen.rs`, `landing.rs`. | `IMPLEMENTED` | |
| Theme system + design tokens | `nirdosha-rt/src/theme.rs` + `resolve_design_tokens`. | Implemented; live reload via `ThemeCache` was old compiler, not in v2. | `IMPLEMENTED` (base) | |
| TLS termination for `nirdosha serve` itself | Old compiler's `serve.rs` used plain HTTP; doc says production needs reverse proxy. | `nirdosha-rt/src/web.rs` also serves plain HTTP. | `MISSING` | Could be achieved with `rustls` + `hyper`/`axum` integration, but is runtime work, not macro-only. |

---

## 4. Top-level `.nir` declarative forms → dialect macros

From `docs/DIALECT_FORM_COVERAGE.md`.

| `.nir` form | Dialect substitute | Code truth | Tag | Notes |
|-------------|-------------------|------------|-----|-------|
| `screen <Struct> { .. }` | Archetypes (`crud_screens!`, `settings_screen!`, `kanban_board!`, `wizard!`, `communication_feed!`). | Archetypes exist. No generic `screen!` mirroring `ScreenDecl` field-for-field. | `IMPLEMENTED` (archetype coverage) | |
| `dashboard { .. }` | `dashboard!` | `nirdosha-macros/src/dashboard.rs`. | `IMPLEMENTED` | |
| `workflow Name { .. }` | `wizard!` + `workflow!` | `nirdosha-macros/src/workflow.rs` generates state enum + `advance_*`; `wizard!` is server-side per-run state. | `IMPLEMENTED` | |
| `landing { .. }` | `landing!` | `nirdosha-macros/src/landing.rs`; role-only, claim-based rules blocked on missing claim primitive. | `PARTIAL` | Claim rules: `MISSING` / `ACHIEVABLE VIA RUSTC/MACRO/DRIVER`. |
| `serve { expose fn_a, fn_b }` | None — deliberate out of scope. | No `expose!` macro or centralized allow-list in `nirdosha-rt`. | `MISSING` / `ACHIEVABLE VIA RUSTC/MACRO/DRIVER` | Could be a proc-macro that registers routes on `Router` and validates gates at compile time. |
| `workspace Name { .. }` | None — blocked on screen registry. | `nirdosha-rt/src/showcase_screens.rs` has a `render_workspace` helper but no `workspace!` macro. | `MISSING` / `ACHIEVABLE VIA RUSTC/MACRO/DRIVER` | Needs a name-addressable screen registry first. |
| `validate <fn> { pre: .. post: .. }` | `#[contract(requires(expr), ensures(expr))]` | Covered, different syntax, Z3-over-MIR. | `IMPLEMENTED` | |

---

## 5. UI DSL / next-generation UI

From `docs/NEXT_GEN.md`, `docs/MOBILE.md`, `crates/compiler/UI_DSL_TODO.md`.

| Requirement | Doc claim | Code truth | Tag | Notes |
|-------------|-----------|------------|-----|-------|
| Target-independent UI manifest | `ui_gen.rs` already emits target-agnostic JSON. | In deprecated compiler only; v2 has no manifest emitter. | `DEPRECATED-ONLY` / `MISSING` | Could be revived in v2 as a macro/codegen helper (`ACHIEVABLE VIA RUSTC/MACRO/DRIVER` for the manifest builder). |
| Second renderer (TUI) | Proposed as cheapest proof-of-concept. | No TUI renderer in v2. | `MISSING` | |
| Multiple renderers + manifest versioning | R1: no schema version; becomes compatibility problem once a compiled consumer exists. | No manifest versioning code. | `MISSING` / `ACHIEVABLE VIA RUSTC/MACRO/DRIVER` | Add a schema-version field to macro-emitted manifest. |
| Bounded interaction-verb vocabulary (`call -> navigate`, `call -> confirm`, etc.) | Proposed to replace fixed 6 `Action.kind`s. | v2 archetypes have fixed reaction shapes. | `MISSING` / `ACHIEVABLE VIA RUSTC/MACRO/DRIVER` | Macro-time code generation can switch on a small verb enum. |
| Per-element style/token layer beyond app-wide theme | F1 proposed style/token layer; F4 Phase C `css:` escape hatch not built. | No per-element scoped CSS class generation in v2. | `MISSING` / `ACHIEVABLE VIA RUSTC/MACRO/DRIVER` | Can generate scoped classes + sanitized CSS at macro expansion time. |
| Phase B widget catalog (`progress`, `multi_select`, `tag_input`, date pickers, `toggle`, `slider`, etc.) | Listed as open. | Not in v2. | `MISSING` / `ACHIEVABLE VIA RUSTC/MACRO/DRIVER` | Template + macro expansion; no runtime kernel needed. |
| File/blob/attachment type (R5) | Blocks export/upload/camera capture. | No `blob`/`bytes`/`Attachment` type in `nirdosha-rt`. | `MISSING` | Needs runtime type + storage primitive. Not macro-only. |
| `target:` key for web/mobile exclusion | Proposed in `docs/MOBILE.md` as a `screen`/`dashboard` kv. | Not in `nirdosha-rt` or macros. | `MISSING` / `ACHIEVABLE VIA RUSTC/MACRO/DRIVER` | Add a `target` field to screen/dashboard macro attrs and filter during render. |

---

## 6. Mobile native app generation

From `docs/MOBILE.md`.

| Requirement | Doc claim | Code truth | Tag | Notes |
|-------------|-----------|------------|-----|-------|
| `nirdosha emit-mobile` CLI verb | Mirroring `emit-ui` for iOS/Android. | No `mobile_gen.rs`, no `emit-mobile` command. | `MISSING` | |
| Standard profile (SwiftUI/Compose CRUD/dashboard/login) | Reuses manifest IR. | No code. | `MISSING` | Could be built as macro-driven project generator. |
| Rich profile D2: biometric step-up | Needs device-bound artifact + `step_up: biometric` action key. | No code. | `MISSING` | Needs runtime + platform-specific secure enclave/keystore integration. |
| Rich profile D3: camera/document capture | Blocked on file/blob type. | No code. | `MISSING` | |
| Rich profile D4: push notifications (APNs/FCM) | Needs real provider adapter + device-token registration. | No code. | `MISSING` | |
| Rich profile D5: offline action queue with idempotency key on `POST /api/<fn>` | Needs RPC-layer idempotency table. | No code. | `MISSING` | |
| Manifest IR reuse | `ui_gen.rs` manifest is target-agnostic. | Only in deprecated compiler. | `DEPRECATED-ONLY` / `MISSING` | |

---

## 7. Native product compiler (`/home/arun/nirdosha` `codegen-refactor`) — relevant because docs still reference it

From `docs/V2_MIGRATION_ROADMAP.md`, `docs/ROADMAP.md`.

| Requirement | Doc claim | Code truth in this repo | Tag | Notes |
|-------------|-----------|-------------------------|-----|-------|
| Conditional service embedding (W1) | Split runtime kernel into link-time-selectable modules driven by `effects.rs`. | `crates/runtime-kernels/src/kernel/` is monolithic in `nirdosha-rt`; no link-time module selection. | `MISSING` | Needs build.rs / cargo-feature / linker-section work. |
| UI inside the binary (W2) | `emit-ui` assets linked into the native binary; single artifact serves itself. | `nirdosha-rt` serves server-rendered HTML, not embedded binary assets. | `MISSING` | |
| High-performance HTTP path (W3) | Vetted server stack behind kernel surface. | `nirdosha-rt/src/web.rs` uses a minimal HTTP stack, not a vetted high-performance server. | `MISSING` | |
| APM egress / OTLP export (W4) | Recorder's own doc names OTLP, sampling, real sink as missing. | `nirdosha-rt` has NFR logging only; no OTLP exporter. | `MISSING` | |
| FIPS 140-3 module path end-to-end (W5) | ADR 0012 backend swap. | `nirdosha-audit` has `fips` feature using `aws-lc-rs`; `nirdosha-rt` does not expose or enforce a compliance profile. | `PARTIAL` | Crypto backend exists in audit crate; not wired into v2 runtime as a compliance profile. |
| Artifact evidence / guarantee manifest in binary (W6) | `verify-artifact` reproduces evidence offline. | `nirdosha-hi` has pack signing (`crates/nirdosha-contract-core/src/pack.rs`), but no `verify-artifact` command or embedded manifest in `nirdosha-rt`. | `MISSING` | |
| `nirdosha hi` as front door | Console runs today on `codegen-refactor`; becomes generate→verify→publish front door at M4. | `crates/nirdosha-hi/` exists, but it is not integrated with `nirdosha-rt` build/publish. | `MISSING` (integration) | |

---

## 8. Standards / compliance posture

From `docs/ROADMAP.md` "Standards & compliance posture" table.

| Area / Standard | Doc status | Code truth | Tag | Notes |
|-----------------|------------|------------|-----|-------|
| OWASP Top 10 (rate limiting) | `[PARTIAL]` — no rate limiting. | No rate-limiting middleware in `nirdosha-rt/src/web.rs`. | `MISSING` | Achievable with a simple token-bucket middleware in `Router`, no compiler work. |
| OpenID Connect (discovery, `/userinfo`, multi-IdP) | `[PARTIAL]` — missing discovery, `/userinfo`, multi-IdP registry. | v2 `nirdosha-rt` does not implement OIDC discovery or `/userinfo`; multi-IdP not present. | `MISSING` | |
| SAML 2.0 | `[OPEN]` | No code. | `MISSING` | |
| SCIM | `[OPEN]` | No code. | `MISSING` | |
| FIDO2/WebAuthn | `[OPEN]` | No code. | `MISSING` | |
| TLS for `nirdosha serve` itself | `[OPEN]` | Plain HTTP only. | `MISSING` | |
| mTLS | `[OPEN]` | No code. | `MISSING` | |
| OpenAPI spec generation | `[OPEN]` | No code. | `MISSING` / `ACHIEVABLE VIA RUSTC/MACRO/DRIVER` | Could generate OpenAPI from macro-registered route signatures. |
| Syslog | `[OPEN]` | No code. | `MISSING` | |
| OpenTelemetry (OTLP export, metrics) | `[PARTIAL]` — real collector export not built. | No OTLP exporter in `nirdosha-rt`. | `MISSING` | |
| CEF / ECS logging | `[OPEN]` | No code. | `MISSING` | |
| Backup tooling | `[OPEN]` | No code. | `MISSING` | |
| WCAG 2.2 / accessibility | `[OPEN]` — zero `aria-` attributes. | `nirdosha-rt/src/web.rs` templates have no ARIA. | `MISSING` / `ACHIEVABLE VIA RUSTC/MACRO/DRIVER` | Add ARIA attributes in macro-generated HTML. |
| Audit-trail screen/API archetype | Not in standards table, but recurring in regulated domains (`rfcs/0009`). | No `audit_trail!` macro. | `MISSING` / `ACHIEVABLE VIA RUSTC/MACRO/DRIVER` | Macro over history table + read-only screen. |
| Approval chain with quorum + delegation | Not in standards table. | No `approval_chain!` macro; `workflow!` lacks quorum/delegation. | `MISSING` / `ACHIEVABLE VIA RUSTC/MACRO/DRIVER` once delegation table exists | Generates state machine + decision-count table. |
| Notification inbox / bell-icon convention | Not in standards table. | `docs/WORKFLOW.md` says possible but no macro. | `MISSING` / `ACHIEVABLE VIA RUSTC/MACRO/DRIVER` | Macro over `Notification` struct + badge in nav. |
| RBAC admin screens | Not in standards table. | No `rbac_admin!` macro; claim primitive missing. | `MISSING` / `ACHIEVABLE VIA RUSTC/MACRO/DRIVER` once claims exist | Generates CRUD screens for roles/claims/mappings. |
| Scheduler / recurring job | Not in standards table. | No cron primitive; no `scheduler!` macro. | `MISSING` | Needs external scheduler contract or runtime timer. |
| Faceted search screen | Not in standards table. | No `search_screen!` macro. | `MISSING` / `ACHIEVABLE VIA RUSTC/MACRO/DRIVER` for UI half | Needs backend search primitive. |
| Import/export job | Not in standards table. | No `import_job!`/`export_job!` macro. | `MISSING` | Blocked on file/blob/attachment type. |

---

## 9. `.nir` language features not ported to v2

These are documented in `docs/LANGUAGE.md` / `docs/TRANSACT.md` / `docs/WORKFLOW.md` and exist in the deprecated compiler, but have **no equivalent** in the v2 Rust dialect.

| Feature | Old compiler status | V2 status | Tag | Notes |
|---------|---------------------|-----------|-----|-------|
| `transact { ... }` durability + crash replay | Compiled backend landed 2026-09-08. | No dialect macro/runtime equivalent. | `DEPRECATED-ONLY` / `MISSING` | Could be built as a `#[nirdosha_rt::saga]` proc-macro over `DurableSaga` (runtime exists). |
| `workflow { ... }` full feature: `data {}`, `link` tokens, state ownership, queue UI | Compiled Layer 1 subset exists; many features rejected by `check_supported`. | No dialect equivalent beyond `workflow!` state machine (no durable store/HTTP routes). | `DEPRECATED-ONLY` / `MISSING` | |
| `send_email`/`send_push`/`notify` notification builtins | Compiled in old compiler. | No equivalent in `nirdosha-rt`. | `MISSING` | Needs runtime adapters + provider config store. |
| Presence gateway / real-time push bridge | Built as separate `crates/presence-gateway/` in this repo. | Not integrated with `nirdosha-rt` at all. | `MISSING` (integration) | |
| Auto-generated DB migrations (`nirdosha serve --db`) | SQLite-only, additive-only, in `migrate.rs`. | No equivalent in v2. | `MISSING` | |
| Generic table-browser route `/_nirdosha/table/<name>` | In old `serve.rs`. | No equivalent in v2. | `MISSING` | |
| `nirdosha serve` as a subcommand | Removed with interpreter. | v2 `nirdosha-rt::web::Router` can serve, but there is no `nirdosha serve` CLI in `cargo-nirdosha`. | `MISSING` | |
| `dec128` compiled backend | Old compiler rejects it (interpreter-only). | No `dec128` type in v2 at all. | `MISSING` | Could reuse `rust_decimal` in v2 as ordinary Rust dependency. |
| `sandbox` / `stop` | Explicitly excluded from this audit. | — | `EXCLUDED` | |

---

## 10. Items that are documented as deliberate non-goals or limitations

These are **not gaps**; they are honest, documented boundaries.

| Item | Doc reference | Why it is not a gap |
|------|---------------|---------------------|
| No WebSocket termination in core repo; presence gateway is separate. | `docs/WORKFLOW.md` "Deliberate non-goals" | By design. |
| At-least-once notification delivery, never exactly-once. | `docs/WORKFLOW.md` | Fundamental distributed-systems limit, acknowledged. |
| No provider-specific email/SMS/push API schemas. | `docs/WORKFLOW.md` | By design; generic POST transport only. |
| `payload` not threaded into `on_entry`/`on_exit` bindings. | `docs/WORKFLOW.md` | Documented open item, not silently dropped. |
| `dec128` interpreter-only in old compiler. | `docs/LANGUAGE.md` §2, §10 | Architectural blocker documented. |
| Stage 2 driver nightly + `rustc-dev` only; `stable_mir` blocked. | `docs/nirdosha-rt-dialect.md` §4 | External toolchain limitation, acknowledged. |
| Source-scan Stage 1 cannot see through calls / macro expansion. | `docs/nirdosha-rt-dialect.md` §4 | Honest limitation. |
| Async contracts not verified. | `docs/nirdosha-rt-dialect.md` §4 | Macro refuses them; documented. |

---

## A. Documentation relevance in the v2 era

Every `.md` under `docs/` classified by whether it still describes the current system (the v2 Rust dialect + `rustc`/macro/driver) or has become historical/speculative because the standalone `.nir` compiler / interpreter was deprecated.

| Document | Relevance today | Why |
|----------|-----------------|-----|
| `docs/nirdosha-rt-dialect.md` | **CURRENT — free lane** | The authoritative design doc for the v2 Rust dialect. |
| `docs/V2_GUARANTEES.md` | **CURRENT — free lane** | Guarantee profile for the v2 verifier/driver. |
| `docs/V2_IMPLEMENTATION_BOOK.md` | **CURRENT — free lane** | Running implementation log for v2. |
| `docs/DIALECT_FORM_COVERAGE.md` | **CURRENT — free lane** | Maps old `.nir` top-level forms to v2 macros. |
| `docs/V2_MIGRATION_ROADMAP.md` | **CURRENT — product lane** | Native-first product roadmap; describes work mostly in `/home/arun/nirdosha` `codegen-refactor`. |
| `docs/ROADMAP.md` | **CURRENT — mixed / status tracker** | Still the single source of truth for status, but large sections describe the deprecated `.nir` compiler. Read with the 2026-09-17 deprecation note in mind. |
| `docs/PUBLIC_ROADMAP.md` | **CURRENT — mixed** | External-facing summary; explicitly updated in 2026-09 to mark interpreter-era items as `[NOT RUNNABLE]`. |
| `docs/STABILITY_AND_RELEASES.md` | **CURRENT — policy** | Compatibility/versioning policy; acknowledges the interpreter deletion. |
| `docs/FEATURE_WORKFLOW.md` | **CURRENT — process** | Governs how features/bugs are tracked in this repo. |
| `docs/VALUES.md` | **CURRENT — philosophy** | Why the interpreter was removed; still load-bearing. |
| `docs/API_TRUST_MODEL.md` | **CURRENT — design** | Trust model for generated apps; applies equally to old `.nir` and v2 `nirdosha-rt::web`. |
| `docs/SPEC_V1.md` | **CURRENT — certificates** | in-toto predicate schema used by `cargo nirdosha verify/certify/fix`. |
| `docs/MIR_NUMERIC_PROOFS.md` | **CURRENT — driver** | Describes the implemented Z3-over-MIR numeric proof pass. |
| `docs/goal.md` | **MOSTLY CURRENT — philosophy** | High-level design rows; some examples reference old interpreter-only features, but the goals themselves still guide v2. |
| `docs/LANGUAGE.md` | **DEPRECATED / HISTORICAL** | A practical reference to the old standalone `.nir` language, interpreter, and `nirdosha serve`. The v2 dialect replaces this; it is kept because it is cited by source comments and `README.md`. |
| `docs/GRAMMAR.md` | **DEPRECATED / HISTORICAL** | EBNF for the old `.nir` parser. v2 intentionally has "no bespoke parser, ever." |
| `docs/TRANSACT.md` | **DEPRECATED / HISTORICAL** | Target design for `.nir` `transact`. The old compiler now has a compiled backend, but v2 has no equivalent. Kept as the intended shape if/when ported. |
| `docs/WORKFLOW.md` | **DEPRECATED / HISTORICAL** | Target design for `.nir` `workflow`. Same status as `TRANSACT.md`. |
| `docs/SANDBOXING.md` | **DEPRECATED / HISTORICAL** | Describes sandboxing in the deleted interpreter; explicitly descoped and not runnable today. |
| `docs/nirdosha_row11_amendment.md` | **DEPRECATED / HISTORICAL** | Rationale for Row 11 (`struct`/`enum`/`match`) in the old language. Rust supersedes it. |
| `docs/nirdosha_row12_functions_identity.md` | **DEPRECATED / HISTORICAL** | Row 12 identity design in the old language. Concepts migrated to v2 (`RoleProof`), but the doc itself is old-language-centric. |
| `docs/PHASE0.md` | **DEPRECATED / HISTORICAL** | Historical build journal. |
| `docs/Nirdosha_Unified_Plan.md` | **DEPRECATED / HISTORICAL** | Phase 0.5→5 plan scoped to numerics + agent surface; predates and does not cover v2. |
| `docs/PROTOLANG_PORT.md` | **DEPRECATED / HISTORICAL** | Describes how the old language departed from earlier aspirational specs. |
| `docs/protolang_reference_specification.md` | **NOT RELEVANT TO NIRDOSHA** | Earlier aspirational spec, not the current language. |
| `docs/protolang_std_io_specification.md` | **NOT RELEVANT TO NIRDOSHA** | Earlier aspirational I/O spec, not the current language. |
| `docs/NEXT_GEN.md` | **MIXED — mostly historical / design** | F2 namespacing/F3 contracts/F4 Phase A shipped in the old compiler, but the doc reasons about `interpreter.rs`/`serve.rs` that no longer exist. F1 (multi-renderer) is still design-only. |
| `docs/MOBILE.md` | **DESIGN-ONLY** | Spec for native iOS/Android generation; nothing implemented. |
| `docs/ECOSYSTEM.md` | **DESIGN-ONLY** | Adoption infrastructure; "nothing in this document is implemented." |
| `docs/PLUGIN_AUTHORING_FOR_LLMS.md` | **MOSTLY HISTORICAL** | Plugin recipe referenced old plugin examples (`mysql`/`activemq`/`cassandra`/`neo4j`/`hbase`) that do not exist in this repo today. The current examples are `plugin-example-native-{authed-http,kv,shout}` and `ui-plugin-example-sparkline`. |
| `docs/KUBERNETES.md` | **PRODUCT-LANE — historical specifics** | Deployment guide for the deprecated `.nir` `nirdosha serve`; concepts still valid but command-line details are stale. |
| `docs/KUBERNETES_ADVANTAGE.md` | **PRODUCT-LANE — positioning** | Positioning doc referencing the old compiler's capabilities. |
| `docs/PROTOBOX_INTEGRATION.md` | **PRODUCT-LANE — historical specifics** | Interface contract with protobox; references `.nir` output and old `nirdosha serve` behavior. |
| `docs/nirdosha-agent-api.md` | **DESIGN-ONLY / historical** | Spec for an agent-facing HTTP API; server not built. Updated in 2026-09 to note interpreter deletion. |
| `docs/nirdosha-v2-comment-layer.md` | **DESIGN-ONLY / deferred** | Accepted proposal for a comment-encoded declarative layer on `.nir`; recorded as deferred work, not implemented. |
| `docs/llm-ops-api-spec.md` / `docs/llm-ops-api-spec-v2.md` | **NOT RELEVANT TO NIRDOSHA** | Generic multi-backend LLM training/serving specs; zero Nirdosha-specific content per project instructions. |
| `docs/CONCURRENT_STATE_PATTERNS.md` | **MIXED** | Concurrency design notes; useful background, but examples may reference old interpreter paths. |

### ADRs and research notes

| Path | Relevance today | Why |
|------|-----------------|-----|
| `docs/adr/README.md` | **CURRENT** | Index of architectural decisions. |
| `docs/adr/0003-runtime-kernels-cargo-dependency.md` | **CURRENT** | Governs how `runtime-kernels` is consumed by both lanes. |
| `docs/adr/0005-postgres-pooling-and-tls.md` | **MIXED** | Describes the old compiler's DB backend; the kernel design is reused in v2 native adapters but the `.nir`-specific wiring is gone. |
| `docs/adr/0006-http-keepalive-pooling.md` | **MIXED** | Same pattern lives in `runtime-kernels/src/kernel/http.rs`; the old compiler framing is historical. |
| `docs/adr/0008-build-rs-watch-full-kernel-src-tree.md` | **CURRENT** | Still applies to `runtime-kernels/build.rs`. |
| `docs/adr/0010-runtime-kernels-rlib-for-compiled-serve.md` | **PRODUCT-LANE — current** | Relevant to the native `.nir` compiled-serve work. |
| `docs/adr/0012-fips-140-3-crypto-backend-swap.md` | **CURRENT** | Underpins `nirdosha-audit` FIPS feature. |
| `docs/adr/0001-vendor-z3-except-macos.md` | **DEPRECATED / HISTORICAL** | Old `.nir` compiler build concern; v2 driver uses the nightly toolchain's `rustc-dev` component, not vendored Z3. |
| `docs/adr/0002-ban-str-in-fn-signatures.md` | **DEPRECATED / HISTORICAL** | Old `.nir` language decision; Rust's type system already handles this differently. |
| `docs/adr/0007-identity-row12-remaining-builtins.md` | **DEPRECATED / HISTORICAL** | Old compiler identity builtins; v2 uses `RoleProof` + runtime-kernels JWT. |
| `docs/adr/0004-external-data-service-boundary.md` | **DEPRECATED / HISTORICAL** | Old compiler plugin boundary design. |
| `docs/adr/0009-transact-durability-and-replay.md` | **DEPRECATED / HISTORICAL (design)** | Durability protocol target for `.nir`; no v2 equivalent yet, but the design is still the intended shape. |
| `docs/adr/0011-keep-readable-reserved-words-reject-dunder-prefix.md` | **DEPRECATED / HISTORICAL** | Old `.nir` lexer policy. |
| `docs/research/*.md` | **CURRENT** | 2026-09 research notes on verification, signing, and guarantee differentiation; directly supports v2 design decisions. |

### How to read the docs now

- For **v2 dialect guarantees and commands**: read `nirdosha-rt-dialect.md`, `V2_GUARANTEES.md`, `V2_IMPLEMENTATION_BOOK.md`, `MIR_NUMERIC_PROOFS.md`.
- For **status of the whole project**: read `ROADMAP.md`, but treat any claim about `interpreter.rs`, `serve.rs`, or `nirdosha serve` as historical unless it explicitly references `nirdosha-rt::web::Router` or `cargo nirdosha`.
- For **old `.nir` syntax/semantics**: read `LANGUAGE.md`, `GRAMMAR.md`, `TRANSACT.md`, `WORKFLOW.md` only to understand what the deprecated compiler did or what a future port might look like.
- For **unbuilt features**: read `MOBILE.md`, `ECOSYSTEM.md`, `nirdosha-agent-api.md` as specs, not current capabilities.

## Summary matrix

| Category | Implemented | Deprecated-only | Missing | Partial | Achievable via rustc/macro/driver |
|----------|-------------|-----------------|---------|---------|-----------------------------------|
| V2 contracts / driver | 7 | 0 | 2 | 2 | 2 |
| `cargo nirdosha` / certificates | 6 | 0 | 3 | 0 | 0 |
| `nirdosha-rt` runtime / native | 5 | 0 | 4 | 1 | 1 |
| `.nir` form → dialect macro | 3 | 0 | 0 | 1 | 3 |
| UI DSL / next-gen UI | 0 | 1 | 5 | 0 | 5 |
| Mobile | 0 | 1 | 7 | 0 | 0 |
| Native product compiler integration | 0 | 0 | 6 | 1 | 0 |
| Standards / compliance | 0 | 0 | 11 | 1 | 2 |
| `.nir` features not ported | 0 | 6 | 7 | 0 | 1 |
| **Total** | **21** | **8** | **46** | **6** | **15** |

*(Totals are rough counts of discrete rows; some rows carry a secondary "achievable" note in addition to a primary tag.)*

---

## Highest-impact gaps to close next

### Existing gaps already tracked
1. **Claim-based identity (`ClaimProof`)** — unblocks claim-based `landing!` rules and restores parity with `.nir` Row 12. Purely macro/runtime-type work.
2. **`serve { expose }` / centralized route allow-list** — the access-control-relevant UI form gap. Can be a proc-macro over `Router`.
3. **`workspace!` macro** — needs a screen registry; then pure macro work.
4. **OpenAPI generation** — strong candidate for a proc-macro that inspects registered routes.
5. **ARIA / accessibility** — low-risk macro/template change with large compliance payoff.
6. **Rate limiting** — simple middleware, high security value.
7. **OTLP APM export / TLS server** — runtime work, needed for production deployments.
8. **Signed trust chain / artifact-byte binding / `verify-artifact`** — closes the certificate-to-binary provenance story.

### New archetype gaps to consider adding to the roadmap
These are recurring domain patterns that can mostly be macro-driven once a convention is fixed:

9. **`audit_trail!`** — read-only, time-ordered audit screen + API for any struct (SOX, HIPAA, PCI). Macro over history table.
10. **`approval_chain!`** — sequential/quorum approval with delegation and SLA escalation. Extends `workflow!` with decision-count table.
11. **`notification_inbox!`** — persisted bell-icon notification feed with read/unread + dismissal. Macro over a `Notification` struct.
12. **`rbac_admin!`** — auto-generated admin screens for roles, claims, role mappings. Blocked on `ClaimProof`; otherwise macro-only.
13. **`search_screen!`** — faceted search UI with pagination and saved queries. UI half is macro-only; needs backend search primitive.
14. **`scheduler!`** / **`cron_job!`** — recurring durable job definition + execution log. Needs external scheduler contract or runtime timer.
15. **`export_job!`** — select columns/filters → downloadable bundle + audit record. Blocked on file/blob type.
16. **`import_job!`** — CSV/Excel upload → validation → preview → apply. Blocked on file/blob type.
17. **`ledger!`** — double-entry ledger with chart of accounts, journal entries, balance queries. Builds on `DurableSaga` + `dec128` conventions.
18. **`invoice!`** / **`payment!`** — invoice/payment intent/capture/refund lifecycle with idempotency.
19. **`inventory!`** / **`shipment!`** — stock tracking and shipment state tracking for supply-chain apps.
20. **`sla_monitor!`** / **`alert_manager!`** — threshold-based alerts with routing and acknowledgement.
21. **`knowledge_base!`** — hierarchical articles with review workflow and role-gated editing.
22. **`broadcast!`** — admin announcements with audience targeting and acknowledgement tracking.
