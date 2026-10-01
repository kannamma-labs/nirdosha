# Nirdosha Spec v1 — attestation formats as in-toto predicates

`nirdosha-master-plan.md` Part 3 Q1 2027: *"Spec v1 published — verdict
schema + certificate format + repair protocol as an in-toto predicate
(compose with SLSA, don't compete)."* This document is that spec,
published separately from the compiler so an alternative implementation
(a different language's toolchain, a third-party verifier) can produce
or consume these exact JSON shapes without depending on
`crates/compiler` at all — the same reason `docs/PUBLIC_ROADMAP.md`'s
"language-agnostic ladder" names Spec v1 as the point where "backends
become pluggable."

## Why in-toto, and what that buys you

[in-toto](https://in-toto.io)'s [attestation
framework](https://github.com/in-toto/attestation) defines one generic
envelope — a **Statement** — for "here is a claim about a software
artifact," with the claim's own shape left open as a **Predicate**.
SLSA provenance, SBOM attestations, and Sigstore's own signing
metadata all reuse this same envelope; adopting it here means a
Nirdosha verdict/certificate/repair-report sits in the *identical* slot
in any in-toto-aware pipeline (`cosign attest`/`cosign verify-
attestation`, GitHub's own attestation store, an SLSA verifier) instead
of inventing a second, incompatible wrapper format. **Compose with
SLSA, don't compete**: Nirdosha's predicates describe *language-level
correctness claims* (this specific `.nir` file's contracts are proved,
this exact artifact's hash is what a reviewer signed) — a genuinely
different claim from SLSA provenance's *build-process* claims (what
built this, from what source, on what infrastructure). A real
supply-chain trust story wants both, attached to the same artifact,
never merged into one predicate type.

### The Statement envelope (verified against the published spec, not guessed)

```json
{
  "_type": "https://in-toto.io/Statement/v1",
  "subject": [
    { "name": "<path or logical name>", "digest": { "sha256": "<hex, no prefix>" } }
  ],
  "predicateType": "<one of the three URIs below>",
  "predicate": { "...": "one of the three predicate bodies below" }
}
```

`subject[0].digest.sha256` is always the real SHA-256 of the exact
source bytes the predicate is about — reproducible by anyone holding
the file, the same way `docs/LANGUAGE.md`'s Certificate v0 section
already requires for `source_hash`/`grammar_hash`. `subject[0].name` is
the path `nirdosha` was invoked with; it identifies the file for a
human reading the statement, and is not itself part of the trust
claim — only the digest is.

## Producing a Statement today

`nirdosha verify`/`fix`/`certify` all take an `--in-toto` flag: the
command's existing plain JSON output becomes the `predicate` value,
unchanged, wrapped in the envelope above (`main.rs::wrap_in_toto`).
Nothing about the underlying JSON schemas below changes when `--in-toto`
is added or omitted — the flag is purely a presentation choice over the
same data.

```sh
nirdosha verify path/to/file.nir --in-toto
nirdosha certify path/to/file.nir --sign key.pk8 --in-toto   # composes with signing
nirdosha fix path/to/file.nir --in-toto
```

## Predicate 1 — `https://nirdosha.dev/attestations/verify/v1`

The verdict schema. Exactly `nirdosha verify`'s own JSON
(`main.rs::VerifyVerdict`), covered by
`docs/STABILITY_AND_RELEASES.md`'s breaking-change policy as an
external contract from its first release.

```jsonc
{
  "source": "<string, the path verify was run against>",
  "verdict": "PROVED" | "DISPROVED" | "UNKNOWN",
  "load":       { "status": "passed" | "failed" | "skipped", "errors": [ /* Diagnostic, below */ ] },
  "typecheck":  { "status": "passed" | "failed" | "skipped", "errors": [ /* Diagnostic, below */ ] },
  "ownership":  { "status": "passed" | "failed" | "skipped", "errors": [ /* Diagnostic, below */ ] },
  "contracts": {
    "status": "passed" | "failed" | "skipped",
    "verdict": "PROVED" | "DISPROVED" | "UNKNOWN",
    "proved": "<integer>",
    "unsupported": "<integer>",
    "failed": "<integer>",
    "obligations": [
      {
        "fn_name": "<string>",
        "status": "proved" | "unsupported" | "counterexample" | "unbound_identifier" | "<other, additive>",
        "detail": "<string or null>",
        "fix": null | { /* Fix, below */ }
      }
    ]
  },
  "proof_obligations": {
    "proven_in_range": "<integer>",
    "proven_nonzero_divisor": "<integer>",
    "proven_index_bounds": "<integer>"
  }
}
```

**Diagnostic** (one entry of `load`/`typecheck`/`ownership`'s own
`errors` array):

```jsonc
{
  "line": "<integer>",
  "col": "<integer>",
  "message": "<string>",
  "fix": null | { /* Fix, below */ },
  "code": "<string, one of explain::REGISTRY's NIR-codes, or null>"
}
```

**Fix** (`nirdosha fix`'s own per-diagnostic analysis, additive to
`verify`'s own JSON since it's cheap to compute either way):

```jsonc
{
  "applicability": "auto" | "assisted" | "manual",
  "patch": null | { "start_byte": "<integer>", "end_byte": "<integer>", "replacement": "<string>" },
  "rationale": "<string>"
}
```

Stage `status` values are three-valued for the *pipeline mechanics*
question ("did this stage run and complete") — genuinely binary in
practice (typecheck either finds no error or it does), distinct from
the top-level/`contracts` `verdict` field's own three-valued *epistemic*
question ("what do we know about correctness"). Exit code mirrors
`verdict` exactly: `0`/`1`/`2` for `PROVED`/`DISPROVED`/`UNKNOWN`.

## Predicate 2 — `https://nirdosha.dev/attestations/certificate/v1`

The certificate format (Certificate v0, plus v1's optional signature
fields). `main.rs::Certificate`/`SignedCertificate`.

```jsonc
{
  "certificate_version": "0",
  "source_hash": "sha256:<hex>",
  "grammar_hash": "sha256:<hex>",
  "toolchain_version": "<string, e.g. \"0.1.0\">",
  "evidence_tier": "proved" | "checked" | "sampled" | "unknown",
  "verdict_summary": {
    "verdict": "PROVED" | "DISPROVED" | "UNKNOWN",
    "contracts_proved": "<integer>",
    "contracts_unsupported": "<integer>",
    "contracts_failed": "<integer>"
  },
  "proof_obligations": {
    "proven_in_range": "<integer>",
    "proven_nonzero_divisor": "<integer>",
    "proven_index_bounds": "<integer>"
  },

  // v1 only, present together or not at all (nirdosha certify --sign):
  "signature_algorithm": "ed25519",
  "public_key": "<base64, raw 32-byte Ed25519 public key>",
  "signature": "<base64, Ed25519 signature over the canonical JSON
                 serialization of every field above this comment,
                 field order exactly as declared here, compact
                 (no whitespace) -- see 'Canonical serialization' below>"
}
```

`certificate_version`/`source_hash`/`grammar_hash`/`toolchain_version`
are always strings, even though an earlier revision briefly used a
`&'static str`-backed enum-like type for `evidence_tier`/
`certificate_version`/`toolchain_version` internally — a real
implementation detail that forced a schema-compatible fix (see
`docs/PUBLIC_ROADMAP.md`'s signed-certificates entry) but never changed
the wire format itself. `evidence_tier`'s value set is deliberately
wider than what this compiler can produce today (only `proved`/
`unknown` are reachable, `checked`/`sampled` are reserved for the
future CHECKED tier) specifically so this schema never needs a
breaking change when that tier ships.

**"Key-pinned" is a verifier policy, not a format feature.** This
predicate carries the public key that signed it so a verifier never
needs external key discovery — but it says nothing about whether that
key *should* be trusted. An implementation that wants that guarantee
maintains its own list of accepted public keys (per `SECURITY.md`'s own
framing) and checks the embedded key against it after signature
verification, exactly the model TLS certificate pinning uses.

## Predicate 3 — `https://nirdosha.dev/attestations/fix/v1`

The repair protocol. `main.rs::FixReport`.

```jsonc
{
  "before": { /* Predicate 1 (verdict schema), the state before any patch */ },
  "applied": [
    {
      "site": "<string, a function name or \"<line>:<col>\">",
      "detail": "<string, the obligation's status tag or diagnostic message>",
      "start_byte": "<integer>",
      "end_byte": "<integer>",
      "replacement": "<string>"
    }
  ],
  "after": null | { /* Predicate 1 (verdict schema), the state after every applied patch, only present with --apply */ }
}
```

`applied` is always `[]` without `--apply` (a report-only run) and
`after` is always `null` in that case too — `--apply`'s own documented
contract (`main.rs::cmd_fix`) is that nothing is written to disk, and
therefore nothing changes, unless it's explicitly requested. Byte
offsets in `applied[].start_byte`/`end_byte` are `[start, end)` into
the *original* file (the one described by `before`'s own implicit
state), consumed highest-offset-first so applying one patch never
shifts another's own range out from under it — see
`main.rs::write_auto_patches`'s own doc comment for the full algorithm.

## Canonical serialization (for verifying a Certificate v1 signature)

A signature in predicate 2 covers the JSON produced by serializing a
struct with exactly the fields listed above the signature fields, in
that declaration order, with no extra whitespace (Rust's
`serde_json::to_vec` on the equivalent typed struct — never a generic
JSON value re-serialized after alphabetizing or otherwise reordering
keys, which would produce different bytes than what was actually
signed). A conforming verifier in another language must either
preserve field order through its own JSON library when re-serializing,
or maintain its own fixed field-order list matching this document — the
reference implementation (`main.rs::cmd_verify_certificate`) does the
former, by round-tripping through a typed struct with the same field
order declared here.

## Versioning

Each predicate type's URI ends in `/v1`; a future breaking change to
any of the three bumps that one predicate's own version segment
(`/v2`, ...) without forcing the other two to move — the same
independent-versioning principle `docs/STABILITY_AND_RELEASES.md`
already applies to `nirdosha verify`'s JSON schema as a standalone
contract. Additive fields (a new optional key, a new enum value) never
require a version bump, per that same policy; removing or renaming a
field does.
