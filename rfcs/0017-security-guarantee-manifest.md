# RFC 0017: Security Guarantee Manifests — per-module policy contracts checked at compile time and enforced by the APM kernel

> Status: **real minimal version shipped, 2026-09-15** (Phase 5 of
> `docs/research/2026-09-pending-verification-differentiation-work.md`).
> `crates/compiler/src/guarantee_manifest.rs` implements §3 items 1, 3,
> 5 (capability ceiling, exported contract, role/claim vocabulary) and
> §4/§6 (the emitted bundle, `nirdosha verify-binary`); wired into
> `nirdosha build` (hard failure on a real violation when a manifest is
> present; the bundle is always emitted) and a new `nirdosha check-
> guarantees` command. §3 items 2/4 (resource-budget static bounding,
> network/file literal allowlists), §5 (runtime enforcement baked into
> `runtime-kernels`), and §7 (import-boundary checking) remain real,
> disclosed follow-up work — this is the design's real minimal slice,
> not its full scope. See the tracking doc's Phase 5 section for the
> end-to-end proof (13 unit tests, 9 CLI integration tests, a manual
> build→bundle→verify-binary smoke run against real compiled output).
>
> Below is the original design document, unchanged except for this
> status header — it still describes the RFC's full intended scope, not
> only what shipped.
> Cross-references:
> - `rfcs/0007-apm-runtime-kernel.md` — NFRs-as-language and the resource-control kernel
> - `rfcs/0010-landing-and-serve-exposure.md` — compiled `serve` and deny-by-default route exposure
> - `rfcs/0011-uniform-service-provider-model.md` — opened admission domains, runtime `Domain` registry, plugin-provider pools
> - `rfcs/0016-domain-packs-and-whose-job-domain-correctness-is.md` — sealed domain plugins, non-waivable invariants, attestation sidecars
> - `docs/API_TRUST_MODEL.md` — identity, RBAC, field masking, row-level gaps
> - `docs/LANGUAGE.md` §6a/§6e/§6f — `requires(...)`, field-level `requires(...)`, `nfr(...)`
> - `docs/research/2026-09-generated-code-guarantee-evaluation.md` — the evaluation that prompted this RFC
> - `crates/compiler/src/extraction_schema.rs` — typed PRD-extraction shape
> - `crates/compiler/src/mcp_tools.rs` — `VerifyVerdict`, `Certificate`, `SignedCertificate`, attestation machinery
> - `crates/runtime-kernels/src/kernel/mod.rs` — the `Domain` registry and `acquire`/`release` admission plane
> - `crates/runtime-kernels/src/kernel/nfr.rs` — runtime NFR tracking and escalation

## Motivation

Nirdosha already splits security work between the compiler and the runtime:

- The compiler **proves** effects, role/claim gates, field masks, overflow bounds, and `validate` contracts.
- `runtime-kernels` **enforces** resource admission and `nfr(...)` SLOs at process runtime.

This is the right shape, but the guarantee is invisible to anyone who does not read the source. There is no emitted artifact a reviewer, operator, or downstream tool can hand to Nirdosha and ask: *"Does this generated code still satisfy these guarantees?"* The guarantees live in the `.nir` source, in compiler internals, and in runtime counters, but they do not live anywhere as a **checkable contract attached to the built artifact**.

This is precisely the gap identified in `docs/research/2026-09-generated-code-guarantee-evaluation.md`: this branch can verify and certify the `.nir` source, but it cannot yet take a generated binary or IR and verify it against a declared policy. This RFC proposes the missing bridge.

A **security guarantee manifest** is a per-module JSON policy file that:

1. The compiler checks against the source.
2. The compiler merges with inline annotations.
3. The compiler emits as a guarantee bundle alongside the binary.
4. `runtime-kernels` enforces at runtime.

The runtime kernel already has the enforcement hooks; the manifest just makes those hooks explicit, version-controlled, reviewable, and portable with the artifact.

### Why now

- The admission/NFR kernel in `runtime-kernels/src/kernel/mod.rs` is real but configured only by environment variables (`NIRDOSHA_KERNEL_MAX_TCP`, etc.). A manifest turns those knobs into policy.
- `requires(role/claim: ...)` + `acquire` and field-level `requires(...)` are compiled now, so compile-time gate checking is no longer blocked by the interpreter-only path.
- `nirdosha build --serve` produces a real compiled HTTP server with deny-by-default route exposure, so per-deployment route and resource policy is now a real question.
- Multi-file programs via `use "..."` exist, and imported modules need a capability boundary.
- Domain packs (RFC 0016) introduce sealed, signed, non-waivable invariants and an attestation sidecar. A per-module security manifest is the natural complement: it constrains what ordinary, non-domain code is allowed to do, while the domain pack provides pre-certified primitives it must use for sensitive operations.
- The LLM-first author story needs a way for a human reviewer to constrain what an LLM-generated module is allowed to do without reading every line.

## Design

### 1. Artifact: the guarantee manifest

A manifest file lives next to a `.nir` source file or module root. By convention:

```text
src/payments.nir
src/payments.nir.guarantees.json
```

For a project root, a single `nirdosha.guarantees.json` is also accepted.

The manifest is JSON. Unknown keys are ignored so older tools can read newer manifests.

```json
{
  "manifest_version": "1",
  "module": "payments",
  "capabilities": {
    "allowed_effects": ["io", "network", "db"],
    "forbidden_effects": ["concurrent", "rng"]
  },
  "resource_budgets": {
    "tcp": 100,
    "file": 20,
    "thread": 0,
    "db": 50,
    "mq": 0
  },
  "network_policy": {
    "hosts": ["api.stripe.com:443", "ledger.internal:5432"],
    "default": "deny"
  },
  "file_policy": {
    "paths": ["/var/log/nirdosha/*.log"],
    "default": "deny"
  },
  "default_nfr": {
    "latency_ms": 200,
    "error_rate_max": 0.01,
    "throughput_min_per_sec": 50,
    "concurrency_max": 100
  },
  "role_vocabulary": ["admin", "hr_staff", "treasury_user", "compliance_officer"],
  "authorization_bearing_claims": ["department", "counterparty_id"],
  "exports": {
    "get_employee": {
      "requires": "role:hr_staff",
      "nfr": { "latency_ms": 50, "concurrency_max": 1000 }
    },
    "public_status": {
      "public": true
    }
  },
  "imports": {
    "ledger": {
      "capabilities": { "allowed_effects": ["io", "db"], "forbidden_effects": ["network"] }
    }
  }
}
```

### 2. What the manifest expresses vs. what stays inline

The manifest holds **module-level, cross-cutting, deployment-reviewable, or import-constraining** policy. Inline annotations in `.nir` stay as the source of truth for function-level behavior.

| Policy | Inline `.nir` | Manifest | Reason |
|---|---|---|---|
| Per-function `effect(...)` | ✓ | — | Compiler already infers and checks the real set. |
| Per-function `requires(role/claim: ...)` | ✓ | exported contract only | Moving all gates to a file creates drift. The manifest may *require* that exported `fn` X has gate Y. |
| Per-function `nfr(...)` | ✓ | defaults + exported override | Defaults reduce repetition; exported contract ensures API SLO. |
| Module capability ceiling (`allowed_effects`) | — | ✓ | Cannot be inferred from a single function. |
| Resource budgets (max concurrent tcp/file/thread/db/mq) | — | ✓ | Deployment policy; today only env vars or hard-coded defaults. |
| Host/file whitelist | — | ✓ | External policy the compiler cannot know. |
| Role/claim vocabulary | — | ✓ | Prevents arbitrary JWT claims from becoming auth primitives. |
| Import constraints | — | ✓ | Capability boundary for `use "..."`. |

### 3. Compiler pass: `guarantee_check`

A new compiler pass runs after `loader` and before `typeck`/`ownership`/`contract_check`:

```text
load_program(path)
  -> load_guarantee_manifest(path)      // NEW
  -> typeck / ownership / contract_check
  -> guarantee_check(program, manifest)  // NEW
  -> codegen::build / emit_llvm_ir
```

`guarantee_check` performs at least these checks:

1. **Capability ceiling.** For every function in the module, compare `effects::infer_effects(program)` against `allowed_effects`/`forbidden_effects`. Any function whose inferred effect set is not a subset of `allowed_effects`, or intersects `forbidden_effects`, is a **hard error**.
2. **Resource budget static check.** Where the static call graph and resource-creation sites can be bounded, prove the high-water concurrent usage is ≤ the budget. Where it cannot be bounded (dynamic loops, recursion), emit a **warning** and rely on runtime admission.
3. **Exported contract check.** For each `exports.<fn>` entry, verify the source `FnDecl` matches: `requires` string must match the inline `Requirement`; `public` must match `FnDecl::explicit_public`; `nfr` must be compatible with the inline `NfrSpec` (manifest may tighten but not loosen).
4. **Network/file policy.** For every `connect(host, port)` or `open(path, mode)` where the host/path is a compile-time literal, verify it matches the allowlist. Dynamic values are runtime-denied by the kernel.
5. **Vocabulary check.** Every `requires(role: ...)` / `requires(claim: ..., ...)` / field-level `requires(...)` must name a role or claim listed in `role_vocabulary` / `authorization_bearing_claims`.
6. **Import constraint check.** For each `imports.<module>` entry, the imported module's own manifest (if present) must have effects that are a subset of the importer's declared `allowed_effects` for that module.

Error/warning kinds are added to a new `guarantee_check::GuaranteeError` enum so tooling can consume them as structured diagnostics distinct from `typeck` errors.

### 4. Emitted guarantee bundle

`nirdosha build app.nir -o app` also emits `app.guarantees.json` next to the binary. It contains the **proven and enforced** properties:

```json
{
  "binary": "app",
  "manifest_version": "1",
  "module": "app",
  "source_hash": "sha256:...",
  "certificate_version": "0",
  "inferred_effects": {
    "main": ["io"],
    "get_employee": ["io"]
  },
  "proven_contracts": ["tenure_bonus_pct"],
  "unproven_contracts": ["get_employee"],
  "gated_exports": {
    "get_employee": "role:hr_staff"
  },
  "public_exports": ["public_status"],
  "nfr_tracked": {
    "get_employee": { "latency_ms": 50, "concurrency_max": 1000 }
  },
  "resource_budgets": {
    "tcp": 100, "file": 20, "thread": 0, "db": 50, "mq": 0
  },
  "network_policy": { "hosts": [...], "default": "deny" },
  "file_policy": { "paths": [...], "default": "deny" },
  "governing_packs": ["banking-v1"]
}
```

The bundle is the artifact an operator or auditor can check against their own policy. It references the `Certificate` issued for the source via `source_hash` and `certificate_version` so the source-side attestation chain is recoverable from the binary-side bundle.

### 5. Runtime enforcement

`runtime-kernels` already enforces the runtime half:

- Resource admission via `kernel::acquire` / `kernel::release` per `Domain`, with env-var override.
- NFR tracking via `kernel::nfr::register` / `call_begin` / `call_end`.
- Flight-recorder evidence via `kernel::recorder`.

RFC 0011 opened the `Domain` registry to runtime-discovered providers, so new domains can appear via plugins. The manifest must be able to reference arbitrary `Domain` names without the compiler hard-coding the closed set.

The manifest changes three things at runtime:

1. The manifest's `resource_budgets` become the **default ceilings** baked into the binary at build time, instead of the hard-coded generous defaults. Env vars remain as an operator override.
2. The manifest's `network_policy` / `file_policy` are enforced at runtime: a dynamic host or path that is not in the allowlist is denied by the kernel even if admission is otherwise available.
3. The manifest's `default_nfr` is applied to any function that does not declare its own `nfr(...)`.

### 6. CLI surface

```sh
# Build and emit guarantee bundle
nirdosha build app.nir -o app
# emits app and app.guarantees.json

# Verify a built binary/artifact against a policy
nirdosha verify-binary app.guarantees.json --against operator-policy.json

# Typecheck with manifest checking but do not build
nirdosha check app.nir --guarantees app.nir.guarantees.json

# Include the bundle in a signed certificate / in-toto attestation
nirdosha certify app.nir --sign key.pk8
# emitted Certificate gains a `guarantee_bundle` field
```

### 7. Multi-file / import boundaries

When `use "payments.nir"` is resolved, the loader also loads `payments.nir.guarantees.json`. The importer's manifest may constrain the imported module:

```json
{
  "imports": {
    "payments": {
      "capabilities": { "allowed_effects": ["io", "db"], "forbidden_effects": ["network"] }
    }
  }
}
```

If the imported module's own manifest declares `network`, the importer gets a hard error at build time.

### 8. Relationship to domain packs (RFC 0016)

A domain pack is a **certified primitive library** — sealed, signed, non-waivable invariants for a specific domain (e.g., banking). A security guarantee manifest is a **module capability contract** — it constrains what ordinary code in a module is allowed to do and what resources it may consume.

The two compose as follows:

- The manifest can declare that a module relies on a domain pack via `governing_packs`.
- `nirdosha certify` already records `governing_packs` in the `Certificate`. The guarantee bundle copies that field so the binary-side artifact names the same governing set.
- If a sensitive operation (e.g., balance update) must use a pack-provided primitive, the manifest can forbid the module from using raw `db` effects for that purpose, forcing the code through the certified primitive. This is a compile-time check: any function that both touches `db` and belongs to a module whose manifest forbids raw `db` for that concern must instead call a function from the declared pack.

This is the concrete mechanism that prevents "PROVED 0/0" for domain code: the pack supplies the contracts, the manifest forces their use, and the compiler proves they are called on the right paths.

## Effect on the permission model

Yes, this changes the permission model, but only by making existing boundaries explicit and reviewable.

- `requires(role/claim: ...)` and `acquire` keep their current compile-time meaning. The manifest does not replace them; it audits exported functions for them.
- Field-level `requires(...)` stays a source-level mask. The manifest's `role_vocabulary` / `authorization_bearing_claims` prevent silent expansion of what counts as a role or claim.
- The compiled `serve` mode (RFC 0010) is already deny-by-default on route exposure via `typeck::check_serve_exposure`. The manifest's `exports` section can simulate a project-level default-deny posture for exported functions: any exported function not listed in `exports` is a build error.
- Row-level / tenant isolation is still an open language feature. The manifest does not attempt to express it.

## Compatibility

This is an additive change. A `.nir` program with no manifest compiles exactly as before. A manifest is only loaded if the file exists; no new syntax is added to `.nir` source.

Breaking consideration: if a project later enables a project-wide `nirdosha.guarantees.json` with `default_deny_exports: true`, unlisted exported functions become errors. That is an opt-in policy, not a silent behavior change.

## Rejected alternatives

1. **Put all guarantees inline in `.nir`.** Already the status quo for function-level annotations. It does not solve module/deployment policy review and cannot express host/file allowlists.
2. **Put all guarantees in the manifest.** Rejected because it duplicates function-level `nfr(...)` / `requires(...)` and creates a drift hazard. The manifest audits and caps; it does not restate every function contract.
3. **Emit the guarantee bundle as an LLVM metadata section.** Considered for tamper resistance, but JSON is easier for operators and CI pipelines. A signed bundle can be layered later.
4. **Use the PRD extraction schema directly as the manifest.** The extraction schema (`crates/compiler/src/extraction_schema.rs`) is designed for PRD-to-code conformance checking, not for runtime policy. It is complementary, not a substitute.
5. **Make the manifest enforce default-deny at the route level.** Rejected for v1 because RFC 0010 already provides deny-by-default route exposure through `serve { expose ... }`. The manifest audits exports; it does not replace the serve-exposure mechanism.

## Open questions

1. **Dynamic host/path enforcement.** Should the runtime kernel maintain a trie/wildcard matcher for host/file patterns, or is exact/literal matching sufficient for a first slice? Wildcards are useful for log rotation paths.
2. **Recursive import checking.** If module A imports B which imports C, do we check transitive capability intersections? A first slice may check only direct imports.
3. **Signed manifests.** Is cryptographic signing of the manifest and emitted bundle a requirement for the first version, or a follow-up? The `Certificate` already has signing, so the bundle can be signed as part of `nirdosha certify`.
4. **Admission denial surface.** Today `kernel::acquire` returns `false` and callers fail with the same `-1` as other errors. If the manifest tightens budgets, should `AdmissionDenied` become a distinct surfaced error kind?
5. **Interaction with `audited "..." { }`.** An `audited` block suppresses runtime guard emission in codegen. Should a manifest `forbidden_effects` still be allowed to reject a function whose body is inside an `audited` block? Probably yes: `audited` is an escape hatch, not a waiver from policy.
6. **Domain pack enforcement precision.** How does the compiler prove that a forbidden raw `db` effect was avoided by routing through a pack primitive? This may require a new `Effect::DomainPack(...)` tag or call-graph attribution, not just effect-set intersection.
