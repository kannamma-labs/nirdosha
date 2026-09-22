# RFC 0028 — Generated Native Mobile Clients (Kotlin/Android, Swift/iOS) from `.nir` Screen Macros

```
RFC:           0028
Title:         Generated native mobile clients (Kotlin/Android, Swift/iOS)
               from .nir screen macros
Status:        Draft — architecture complete, prototype pending
Depends:       RFC 0009 Track C (dashboard!/crud_screens!, nav bar + session login — built)
               RFC 0022 (web-layer hardening: Router, CORS, DPoP primitives — shipped)
               RFC 0022.b (login!/app_shell! archetypes, screen-map graph — shipped)
Supersedes:    docs/MOBILE.md's Track D plan (targeted the retired crates/compiler;
               retired 2026-09-20; D1–D5 were never built — this RFC re-does the
               same idea against the shipping v2 dialect)
Related:       RFC 0025 (rustc + proc-macros + drivers compilation architecture)
```

---

## 1. Motivation

The v2 dialect already produces a real backend for free: `crud_screens!` and friends (`crates/nirdosha-macros/`) expand into a `nirdosha-rt::Router` that serves both server-rendered HTML and — for `crud_screens!` specifically — a fully symmetric JSON `/api/...` surface. What's missing is a native mobile front end for that same backend.

`docs/MOBILE.md`'s Track D designed this once, targeting `crates/compiler`, which was retired 2026-09-20. This RFC re-derives the same idea against the dialect that actually ships today.

**Goal:** `.nir` source that already declares `crud_screens!`/`login!`/`app_shell!` should produce a real, buildable, native Android app and a real, buildable, native iOS app — generated, not hand-written — that talk to the same backend that same source already produces. Server and mobile client must not silently drift apart.

**Architecture in one sentence:** A derive macro on entity structs (`#[derive(NirdoshaSchema)]`) and a link-time registry (`inventory` / `linkme`) share the macro grammar at compile time, eliminating the need for any external static-analysis subsystem.

---

## 2. What already exists (evidence)

- **`.nir` is plain Rust.** `syn::parse_file` needs no preprocessing — confirmed by `cargo-nirdosha`'s `collect_dialect_files` and `nirdosha-hi`'s `code_units_in_file`.
- **`crud_screens!` is the dominant, already-symmetric archetype.** 38 uses across 17 files in `examples/rtm`. Per entity it registers matching HTML and JSON routes for List/Detail/Create/Update/Delete, plus a `guard:` mode via `GuardedTable` and compile-time `RoleProof<R>` gating per verb.
- **Other archetypes are not yet mobile-ready.** `dashboard!` has a JSON route. `wizard!`, `kanban_board!`, `workspace!` are HTML-only. `communication_feed!`/`approval_inbox!` are asymmetric. Each needs its own JSON-parity change — disclosed follow-on work.
- **Auth is cookie-session only, but bearer/DPoP primitives exist underneath.** `Router::with_login`'s `POST` handler always 302-redirects and sets `Set-Cookie: nirdosha_session=...; HttpOnly; Secure; SameSite=Strict`. `Auth`/`RoleProof`/`ClaimProof` already model bearer-friendly sessions, including `Auth::with_cnf_jkt` and `web/dpop.rs::verify_proof` — but no token-issuance endpoint exists.
- **`Router::openapi_document()` is intentionally schema-free.** This RFC generates Kotlin/Swift directly from its own extracted descriptor rather than routing through OpenAPI tooling.
- **`cargo-nirdosha`'s subcommand dispatch is a flat `match sub.as_str()`** in `main.rs`, already depends on `syn`, and has a clean precedent to copy (`keygen_cli`).

---

## 3. Proposal

### 3.1 Architecture: derive + registry, not symtab

The entire `nirdosha-symtab` subsystem proposed in earlier drafts — HIR→`syn` bridge, `ra_ap_load_cargo` integration, `cfg-expr` extraction, Tier 0/Tier 1 split — is **removed**. It was the largest and least-verified component, and it was solving a problem the compiler already solves.

In its place:

1. **`#[derive(NirdoshaSchema)]`** on entity structs. One line per struct, same as `#[derive(Serialize)]`. The derive reads the struct's own tokens (fields, syntactic types, `#[serde(...)]` attributes) via `serde_derive_internals`, and emits:

   ```rust
   impl NirdoshaSchema for Model {
       const DESCRIPTOR: EntityDescriptor = EntityDescriptor { /* ... */ };
   }
   ```

2. **`crud_screens!`** emits, in addition to its routes, a registration:

   ```rust
   inventory::submit! {
       ScreenRegistration {
           route: "/models",
           entity: <Model as NirdoshaSchema>::DESCRIPTOR,
           verbs: &[Read, Create, Update, Delete],
           access: &[...],
       }
   }
   ```

3. **`emit-mobile-schema`** is a small generated binary that iterates the registry and dumps JSON. No rust-analyzer. No `syn::parse_file`. No HIR bridge. The compiler already did all of that.

4. **`nirdosha-mobile-gen`** consumes that JSON and emits Kotlin and Swift projects.

**Anti-drift claim:** Route paths, field ordering, and wire shape are shared by construction for the supported subset. The semantic layer is not "detected" by a separate conformance pass — it is emitted by the derive from the same tokens the macro expansion sees.

### 3.2 The shared types — `crates/nirdosha-screen-ir`

A pure data crate. No parsing logic. No spans. No `proc-macro2`.

Types:

```rust
pub struct EntityDescriptor {
    pub name: String,
    pub wire_name: String,
    pub fields: Vec<FieldDescriptor>,
}

pub struct FieldDescriptor {
    pub name: String,
    pub wire_name: String,
    pub ty: TypeDescriptor,
    pub presence: WirePresence,
    pub role: FieldRole,
    pub relation: Option<RelationDescriptor>,
}

pub enum WirePresence {
    AlwaysPresent,
    AbsentWhenNone,   // serde(default) or Option<T> without skip_serializing_if
    NullWhenNone,     // Option<T> with explicit null semantics
}

pub enum FieldRole {
    ReadOnly,
    RequiredOnCreate,
    OptionalOnCreate,
    RequiredOnUpdate,
    OptionalOnUpdate,
    ResponseOnly,
}

pub struct ScreenRegistration {
    pub route: String,
    pub mount: String,
    pub entity: EntityDescriptor,
    pub verbs: &'static [CrudVerb],
    pub access: &'static [AccessRule],
}
```

Sidecar schema types (`SchemaConfig`, `RelationConfig`, `FixtureOverride`, `FixturesDependencies`) also live here.

Dependencies: `serde`, `serde_derive`. Nothing else.

### 3.3 Backend JSON-login gap — `crates/nirdosha-rt/src/web.rs`

`Router::with_login`'s `POST` handler gains content negotiation:

- `Content-Type: application/json` → parse `{ username, password }`.
- `Accept: application/json` → respond `200 { ok: true, landing, user: { id, roles } }` with `Set-Cookie`, instead of a redirect.
- On failure → `401 { ok: false, error }` when JSON is requested.
- HTML path unchanged.

**Cookie policy:** `Secure` is set only when the request is HTTPS or `NIRDOSHA_SECURE_COOKIES=1`. This is required for local dev over `http://` and for the E2E acceptance test.

**`/api/session`:** New endpoint. Returns `{ user: { id, roles } }`. Roles are resolved from the session.

**Session format:** Signed cookie with `{ sub, roles, exp, jti }`, HMAC key from env. Stateless by default. `RevocationStore` trait with `InMemory` / `Sql` / `File` impls; default is `InMemory` (correct for single-process `cargo run -p rtm`). `/api/session/refresh` re-issues.

**Pagination:** List JSON route gains `?page=&per_page=&sort=&q=&filter[field]=`, returns `{ items, total, page, per_page }`. Versioned via `Accept: application/vnd.nirdosha.v2+json`; the legacy bare array remains under `Accept: application/json`.

### 3.4 New subcommand — `cargo nirdosha emit-mobile <path>`

Wired into `crates/cargo-nirdosha/src/main.rs`'s dispatch, same shape as `keygen_cli`.

1. Reuse `collect_dialect_files` to walk the target app's `.nir`/`.rs` tree.
2. `syn::parse_file` each file; walk `syn::Item::Macro` nodes matching `crud_screens!`, `login!`, `app_shell!`, `app_shell_from_toml!` — including alias resolution via `use` items, with last-segment fallback and parse-success as the discriminator.
3. Evaluate `#[cfg]` with `cfg-expr` against `--features` and `--target`.
4. Generate a `Cargo.toml` for `emit-mobile-schema` that re-exports the user crate's features (see §3.6).
5. Run `emit-mobile-schema --release` (or debug) to dump the registry as JSON.
6. Pass the JSON to `nirdosha-mobile-gen`.

**`--parse-only` mode:** Developer diagnostic. Skips the schema binary. Emits models with a syntactic type rule: primitives + `Uuid` + `chrono` types typed; `Decimal` and all other paths → `RawJson`. Not a production path. Not in acceptance criteria.

### 3.5 Codegen backends — `crates/nirdosha-mobile-gen`

Two backends over the same JSON:

- **Kotlin** (`generate_kotlin`): kotlinx.serialization `@Serializable data class` per entity, Retrofit interface with one method per route, generic `ListScreen<T>`/`DetailScreen<T>`/`FormScreen<T>` Jetpack Compose composables driven by `FieldDescriptor`. Checked-in Kotlin runtime module (networking, cookie jar, theme) embedded via `include_str!`. Generates a complete Gradle project from a checked-in template.
- **Swift** (`generate_swift`): mirrors the above — `Codable` structs, `URLSession` API client, generic SwiftUI views, checked-in Swift runtime package. Generates a complete `.xcodeproj` from a template (pbxproj, scheme, Info.plist, assets, App.swift).

**Null/absent handling:**
- Kotlin: `Json { explicitNulls = false }` for `AbsentWhenNone`; class-level custom `KSerializer` for entities in the transitive closure of `NullWhenNone`; `RawJson` wrapper for fallback fields.
- Swift: `decodeIfPresent` + `contains` per field.
- Recursive entities use `SerialDescriptor` holder + function-based dispatch (`decodeSerializableElement`); Tarjan SCC with `has_self_edge` check for self-loops.

**Registry backend per target:**
- ELF, PE → `inventory`.
- Mach-O (macOS/iOS) → `linkme`.
- `--registry=inventory|linkme` overrides.

This requires **two registry codegen paths** and **two CI legs** (3–5 days implementation cost).

### 3.6 Feature re-export mechanism

`emit-mobile-schema`'s generated `Cargo.toml` re-exports the user crate's features:

```toml
[dependencies]
user-crate = { path = "../..", default-features = false }

[features]
default = ["user-crate/default"]
experimental = ["user-crate/experimental"]
# one line per feature, including implicit optional-dependency features
```

Regenerated on every `emit-mobile` run. If the user adds a feature and does not regenerate, the next run detects the mismatch and errors with a clear message.

**Cost:** regeneration required after each feature addition. Stated as a one-time cost per feature, not per regeneration.

### 3.7 Sidecar — `nirdosha-mobile.toml` + `.lock`

For external entities, relation overrides, fixture overrides, and server feature declaration:

```toml
[server]
features = ["default", "experimental"]

[relations."/models"]
author_id = { entity = "Author", display = "name" }

[schemas."/external"]
entity = "external_crate::Model"
fields = [...]
fixture_expr = "external_crate::Model::sample()"   # or fixture_skip / fixture_default

[fixtures_dependencies]
serde_json = "1"

[fixture_overrides."/models"]
opaque_payload = "serde_json::json!({ \"stub\": true })"
```

**`--init` behavior:** Generates sidecar skeleton; ambiguous relations commented with candidates. Re-running is additive; merges user edits; `--force` regenerates.

**Lock file (`nirdosha-mobile.lock`):** Per-entry `{ inference_source, last_written_by_tool, current_toml_hash, state }`. States: `resolved`, `user_edited`, `user_deleted`, `user_authored`, `needs_review`, `stale`. GC on `(route, field)`, not route alone. `last_written_by_tool` updated only on tool writes; never on user edits.

### 3.8 Prototype target

`examples/rtm`. Starting with a single `crud_screens!` entity (`m09_ml_models.nir`) as the smoke test.

---

## 4. Acceptance criteria

```
0a. cargo nirdosha emit-mobile examples/rtm --parse-only
    → completes successfully; elapsed time recorded in the report.
    → emits every crud_screens! invocation; marks cfg status "unknown".
    → syntactic type rule applied.
    → not a production path; not part of any correctness criterion.

1.  cargo nirdosha emit-mobile --init examples/rtm
    → creates nirdosha-mobile.toml + .lock.
    → re-running is additive; user edits preserved; deletions not re-added
      without --force.
    → GC marks stale entries by (route, field).

2.  cargo nirdosha emit-mobile examples/rtm --target android --dev
    → writes .kt + build.gradle.kts + WireConformanceTest.kt
    → writes gradle.properties with a projectDir-relative path.
    → writes .gitignore entry for local.properties.

3.  cd examples/rtm/mobile/android && ./gradlew test compileDebugKotlin
    → both pass; copyTestFixtures resolves wire.fixtures.root via project.file().

4.  cargo nirdosha emit-mobile examples/rtm --target ios --dev
    → writes .swift + full Xcode project.

5.  xcodebuild -list -json -project examples/rtm/mobile/ios/GeneratedClient.xcodeproj
      → contains "GeneratedClient"
    xcodebuild -showBuildSettings \
      -project examples/rtm/mobile/ios/GeneratedClient.xcodeproj \
      -scheme GeneratedClient -sdk iphonesimulator | grep SUPPORTED_PLATFORMS
      → contains "iphonesimulator"
    xcodebuild build \
      -project examples/rtm/mobile/ios/GeneratedClient.xcodeproj \
      -scheme GeneratedClient -destination 'generic/platform=iOS Simulator' \
      CODE_SIGNING_ALLOWED=NO
      → passes on Xcode 15+ (macos-14 CI)
    ./scripts/run-ios-tests.sh
      → accepts license if needed; downloads runtime if missing;
        resolves a concrete simulator via simctl JSON; runs xcodebuild test;
        passes (CI macos-14).

6.  just wire-fixtures
    → cargo run -p rtm-wire-fixtures -- --out target/nirdosha-wire-fixtures
    → fixtures crate generated under examples/rtm/mobile/fixtures/;
      user's Cargo.toml not edited.
    → uses atomicwrites; idempotent; cross-platform.

7.  End-to-end: login → /api/session → list → detail → create.
    Android: local + CI. iOS: CI (macos-14).

8.  nirdosha-macros test suite green after CanonicalScreen refactor.

9.  nirdosha-symtab does not exist. `#[derive(NirdoshaSchema)]` resolves
    all types reachable from crud_screens! entities in examples/rtm.
    Unresolved → warning + fallback.

10. Wire conformance:
      10a. Rust: serde output ↔ WireSchema; covers AlwaysPresent,
           AbsentWhenNone, NullWhenNone.
      10b. Kotlin: class-level custom KSerializer for entities in the
           transitive closure of NullWhenNone; descriptor isOptional matches
           runtime presence check; numeric accessors throw on type mismatch;
           RawJson serializer for fallback fields.
      10c. Swift: fixtures ↔ Codable; decodeIfPresent per WirePresence.
      10d. Coverage: baseline + pairwise + enum-exhaustive + all-absent.
           Higher-order combinations disclosed as not covered.
      10e. CI gate on report.json; default zero fallbacks.
      Runs via `just ci`. Failure blocks merge.
```

---

## 5. Non-goals

- **JSON-route parity for `dashboard!`, `wizard!`, `kanban_board!`, `workspace!`, `communication_feed!`, `approval_inbox!`, `settings_screen!`.** Each needs its own change in `nirdosha-macros`/`nirdosha-rt`.
- **Bearer/DPoP token issuance.** A real `/token` endpoint using `Auth::with_cnf_jkt`/`web/dpop.rs` is the natural v2 hardening. Not part of this RFC.
- **Push notifications, offline action queueing, device-bound biometric step-up.**
- **Non-`crud_screens!` UI fidelity** in the generated mobile app.
- **Entity structs from third-party crates without a sidecar descriptor.** The derive requires the struct to be local or the sidecar to declare its shape.
- **`Decimal` and other feature-dependent types in `--parse-only`.** Falls to `RawJson`; full symtab (via the derive) resolves it.

---

## 6. Files changed

**New crates:**
- `crates/nirdosha-screen-ir/` — pure data types, no parsing, no `proc-macro2`
- `crates/nirdosha-mobile-gen/` — Kotlin and Swift codegen backends, templates

**Modified crates:**
- `crates/nirdosha-macros/src/crud_screens.rs`, `login.rs`, `app_shell.rs`, `app_shell_from_toml.rs` — reference `NirdoshaSchema`
- `crates/nirdosha-macros/` — new `#[derive(NirdoshaSchema)]` and `#[derive(RoleName)]`
- `crates/nirdosha-rt/src/web.rs` — JSON login, `/api/session`, pagination
- `crates/nirdosha-rt/src/role.rs` — `RoleName` trait, `RoleProof::ROLE`
- `crates/nirdosha-rt/src/session.rs` — `RevocationStore` trait + impls (new)
- `crates/cargo-nirdosha/src/main.rs`, `lib.rs` — `emit-mobile` subcommand

**Generated (not checked in):**
- `examples/rtm/mobile/android/`
- `examples/rtm/mobile/ios/`
- `examples/rtm/mobile/fixtures/`
- `examples/rtm/nirdosha-mobile.toml`
- `examples/rtm/nirdosha-mobile.lock`

**Not part of this design:**
- `crates/nirdosha-symtab/` — does not exist
- HIR→`syn` bridge, `ra_ap_load_cargo` integration, cfg extraction — removed

---

## 7. Prototype plan

**Purpose:** Falsify the load-bearing assumption (`inventory` on Mach-O under LTO and `-dead_strip`) and validate the Rust-side infrastructure.

**Scope:**
- **In scope:** derive + registry + `serde_derive_internals` + linker behavior + a hand-written Kotlin model deserializing a hand-written fixture.
- **Out of scope:** codegen backends (`generate_kotlin`, `generate_swift`), Gradle/Xcode emitters, class-level custom serializers, Swift validation, `--init` UX, conformance infrastructure.

**Plan:**

| Day | Task | Falsification target |
|---|---|---|
| 1a | Derive attached to `struct Model`; emits hardcoded JSON literal | Proc macro can be attached |
| 1b | Derive calls `serde_derive_internals::attr::Container::from_ast`; emits real `EntityDescriptor` | `serde_derive_internals` on real structs |
| 2 | `crud_screens!` emits `inventory::submit!`; registry contains one entry | `inventory` across crates |
| 3 | `emit-mobile-schema` dumps JSON | Registry → JSON pipe |
| 4 | Hand-write Kotlin model + fixture | Kotlin deserialization |
| 5a | Debug-mode Kotlin test passes | Debug correctness |
| 5b-i | `emit-mobile-schema` with `lto = "fat"`; registry non-empty | LTO internalization |
| 5b-ii | `emit-mobile-schema` with `-Wl,-dead_strip`; registry non-empty | Linker GC of registry sections |
| 5b-iii | Both LTO and `-dead_strip`; registry non-empty | Combined stress |
| 5c | Derive on + macro off → no registration; derive off + macro on → clear error | Registry behavior + diagnostics |

**Estimate:** 6–8 working days if `inventory` works on Mach-O. **+3–5 days** if `linkme` is required. Total: **6–13 days**.

**If 5b-i/ii/iii fails:** add `-C link-dead-code`, or switch to `linkme`, or (if both fail) an explicit `register_screens!` macro called in `main`.

---

## 8. Open items — prototype targets

These are stated as targets, not as settled facts. The prototype determines each.

| Item | Status |
|---|---|
| `inventory` on Mach-O under LTO | **Prototype target** — day 5b-i |
| `inventory` on Mach-O under `-Wl,-dead_strip` | **Prototype target** — day 5b-ii |
| `inventory` under both | **Prototype target** — day 5b-iii |
| Specific linker flag for Mach-O preservation | **Prototype target** — `-no_dead_strip_inits_and_terms` is obsolete; the prototype determines the correct flag |
| `serde_derive_internals` API stability | **Pinned** to `=0.29.1`; vendored fallback parser |
| `linkme` Mach-O sufficiency | **Prototype target** — `S_ATTR_NO_DEAD_STRIP` is documented as a GC root for ld64; sufficiency under all configurations is a prototype target |

**Maintenance costs, disclosed:**

- `ra_ap_*`: 0 (subsystem removed)
- `serde_derive_internals`: 1–2 days/year (pinned + vendored fallback)
- `inventory` / `linkme`: 0–1 day/year (mature crates)
- Kotlin/Swift template correctness: ongoing (tested by conformance)
- Conformance testing bounds drift but does not eliminate it: known, bounded residual risk

---

## 9. Status

**The design phase is over.** Eight rounds of critique produced:

- A coherent architecture: derive + registry replaces the symtab subsystem.
- One load-bearing unverified assumption: `inventory` on Mach-O under LTO and `-dead_strip`, testable in 6–13 days.
- Declared scope: Rust infrastructure + Kotlin-only in the prototype; codegen templates and Swift explicitly deferred.
- Residual risks named, not hidden.

The remaining bugs live in codegen templates. They are found by running code, not by more review. Continuing to argue the design would reproduce the exact convergence failure this debate diagnosed.

**Next artifacts, in order:**

1. The prototype — 6–13 days. Day 5b-i/ii/iii is the falsification gate.
2. Either "it works, proceed to codegen" or "it failed on 5b, here is what changes."
3. The codegen phase, budgeted for the same bug-discovery pattern as the Rust side.

Build the week.