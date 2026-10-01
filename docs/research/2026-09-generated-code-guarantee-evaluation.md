# Evaluation: can this branch say "give me the generated code and I'll tell you whether it satisfies these guarantees"?

> Branch under evaluation: `codegen-refactor` in `nirdosha-hi-dogfood`  
> Evaluated against commit: `b1b3173` ("hi: certificate-mandatory publish, db-isolation checker, Sigstore-pattern pack signing, primitive_exclusivity, and a real use-after-free fix")

## One-sentence answer

**No — not yet.** This branch can verify, certify, repair, and attest the **source `.nir` file**. It cannot yet take a **generated binary or IR** and tell you whether that artifact satisfies a declared set of guarantees.

The gap is not in ambition or analysis infrastructure; it is in the **artifact boundary**. Every current pipeline (`verify`, `certify`, `fix`, `attest`, `audit`) starts from `.nir` source and produces a verdict about the source. None of them starts from the generated code.

---

## What this branch already does (source-level)

| Command / feature | What it verifies | Artifact it consumes | Artifact it produces | Evidence in tree |
|---|---|---|---|---|
| `nirdosha verify <file.nir>` | load, typecheck, ownership, Z3 contracts, proof obligations | `.nir` source | `VerifyVerdict` JSON with `PROVED / DISPROVED / UNKNOWN` | `crates/compiler/src/mcp_tools.rs::run_verify_pipeline` |
| `nirdosha certify <file.nir>` | same gates as `verify`, wrapped in a deterministic, hash-pinned certificate | `.nir` source | `Certificate` / `SignedCertificate` JSON | `crates/compiler/src/mcp_tools.rs::build_certificate` |
| `nirdosha fix <file.nir>` | same gates as `verify`, plus byte-offset `FixPatch` suggestions | `.nir` source | `FixReport` JSON | `crates/compiler/src/mcp_tools.rs::write_auto_patches` |
| `nirdosha attest / audit` | review attestation chain, trust config, signature verification | `.nir` source + attestation JSON | trust/audit report | `crates/compiler/src/mcp_tools.rs` (attestation types) |
| `nirdosha build --serve` | compiles a real HTTP server with deny-by-default route exposure | `.nir` source | native binary | `crates/compiled-serve/`, `crates/compiler/src/codegen.rs` |
| Domain packs + signing | pack manifest + invariants, primitive exclusivity, Sigstore-pattern signing | pack source/manifest | signed `pack.json` + SHA-256 pinned install record | `crates/compiler/src/hi_plugin.rs` |
| Isolation checker | detects db-serializability anomalies at runtime | runtime trace / compiled binary | async escalation via `NIRDOSHA_OBSERVABILITY_URL` | `crates/runtime-kernels/src/kernel/isolation_check.rs` |

The `verify` pipeline is genuinely three-valued (`PROVED`, `DISPROVED`, `UNKNOWN`) and emits structured JSON. Running it against `examples/syntax/hello_nir.nir` on this branch produces:

```json
{
  "source": "examples/syntax/hello_nir.nir",
  "verdict": "PROVED",
  "load": { "status": "passed" },
  "typecheck": { "status": "passed" },
  "ownership": { "status": "passed" },
  "contracts": { "status": "passed", "verdict": "PROVED" },
  "proof_obligations": { "proven_in_range": 1, ... }
}
```

The certificate pins the **source hash**, **grammar hash**, **toolchain version**, and (if signed) an Ed25519 signature. That is a real, reusable attestation about the source.

---

## What "generated code" means in this context

The statement has three possible readings. This branch satisfies the first one only partially and the other two not at all.

### Reading 1: "Give me the `.nir` source and I'll verify the guarantees the compiler will enforce."

**Status: yes, mostly.** `nirdosha verify` and `nirdosha certify` do exactly this for source-level properties: type safety, ownership, effects, role/claim gates, field masks, overflow/division bounds where Z3 can decide, and declared NFRs as runtime commitments.

Caveats:
- `UNKNOWN` verdicts exist and are honest; the pipeline does not collapse them into passes.
- Row-level / tenant isolation, full Rust-style borrow checker, and audit trails are still `[OPEN]` / partial.

### Reading 2: "Give me the compiler's emitted IR/binary and I'll check that the IR/binary still carries the guarantees."

**Status: no.** There is no emitted guarantee bundle alongside the binary. `nirdosha build` produces an ELF (or platform binary) and nothing else. There is no `.guarantees.json`, no embedded manifest section, no verifiable IR annotation.

The closest things are:
- `nirdosha emit-ast` / `nirdosha emit-llvm` — frontend dumps, not verification artifacts.
- `Certificate::nfr_commitments` — these are copied from the source AST at publish time, not extracted from generated code.

### Reading 3: "Give me an arbitrary binary/artifact and an external policy, and I'll tell you whether the artifact satisfies that policy."

**Status: no, and this is the strongest reading of the statement.** There is no `nirdosha verify-binary`, no `nirdosha check --against policy.json`, and no binary-level capability manifest enforcement.

The `runtime-kernels` admission mechanism (`kernel::acquire` / `kernel::release`) does enforce per-domain ceilings at runtime, but those ceilings are either hard-coded defaults or env-var overrides, not policy declarations carried with the binary.

---

## Why the gap matters for the "AI-generated code" promise

This branch's README says: "Agents write it; Nirdosha proves it — constrained at generation, verified by construction, repaired in a closed loop, certified for auditors."

That is true **if the artifact under review is the `.nir` source**. But in a real deployment, what runs is the compiled binary. An operator who receives only the binary currently has no Nirdosha-provided way to ask:

- "Was this binary produced from a certified `.nir` source?"
- "Does this binary only connect to the hosts my policy allows?"
- "Does this binary declare the same role gates the source certificate claims?"
- "What NFRs will this binary monitor at runtime?"

The `Certificate` contains a `source_hash`, so a third party can re-run `nirdosha verify` on the source and check hash equality. But that requires the source, not the generated code. The statement we are evaluating is stronger: it asks about the generated code itself.

---

## What would need to be built to satisfy the statement

To reach "give me the generated code and I'll tell you whether it satisfies these guarantees," this branch needs approximately the following, most of which are natural extensions of code that already exists:

1. **Emitted guarantee bundle at build time.**
   `nirdosha build app.nir -o app` should also emit `app.guarantees.json` (or embed a section in the binary) containing:
   - inferred effects per function
   - `requires(...)` / `nfr(...)` / field-mask metadata for exposed functions
   - static resource high-water marks where provable
   - network/file allowlists derived from literal arguments
   - source hash and certificate reference

2. **Binary/IR-level verifier.**
   A new command, e.g. `nirdosha verify-binary app --against policy.json`, that reads the guarantee bundle and checks it against an operator-supplied policy without recompiling. This is the dual of `nirdosha verify`: verify checks source against language rules; verify-binary checks generated artifact against deployment policy.

3. **Capability manifest enforced by `runtime-kernels`.**
   The runtime-kernel admission mechanism already exists. What is missing is a declared, build-time policy that the kernel consults as the source of truth for ceilings and allowlists, with env vars as overrides rather than the only mechanism.

4. **Provenance link from binary back to source certificate.**
   The guarantee bundle should reference the `Certificate` issued for the source, so an auditor can follow: binary → guarantee bundle → source certificate → source hash → re-verifiable source.

5. **Runtime evidence folded back into the certificate.**
   The branch already has `nfr_commitments` with `evidence_tier: "monitored"` and the isolation checker escalates anomalies. The next step is to let a deployed binary produce a runtime attestation (`isolation_violations`, NFR violation counts, resource admission denials) that can be attached to or compared with the original certificate. This is exactly the "Phase 4 — close the APM loop" work already described in `docs/research/2026-09-pending-verification-differentiation-work.md`.

---

## How close this branch is

This branch is **substantially closer** than the `main` branch of the parent `nirdosha` repo:

- `main` has no `verify`/`certify` commands at all, no compiled `serve`, no isolation checker, no pack signing, and no three-valued verdict pipeline.
- `codegen-refactor` has all of those and a clear research doc (`2026-09-pending-verification-differentiation-work.md`) that names the remaining APM-loop gap as next work.

The remaining distance is mostly **artifact-boundary engineering**, not fundamental research. The analysis passes exist. The runtime enforcement exists. What is missing is the bridge: an emitted, checkable guarantee bundle that travels with the generated code.

---

## Conclusion

For this branch:

- **"Give me the `.nir` source and I'll tell you whether it satisfies these guarantees"** — **yes**, with honest `UNKNOWN` limits.
- **"Give me the generated code and I'll tell you whether it satisfies these guarantees"** — **not yet**. The generated binary does not currently carry a checkable guarantee artifact, and there is no command that verifies a binary against an external policy.

The shortest credible path to "yes" is to make `nirdosha build` emit a guarantee bundle and add a `verify-binary` / `check --against-policy` mode that reuses the existing `Certificate` and `runtime-kernels` policy hooks.
