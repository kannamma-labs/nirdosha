# Security Policy

## Reporting a vulnerability

Please report security issues privately, not as a public GitHub issue —
use [GitHub's private vulnerability reporting](https://github.com/kannamma-labs/nirdosha/security/advisories/new)
for this repository (the "Security" tab → "Report a vulnerability").
This opens a private advisory only the maintainer can see until it's
resolved.

Please include:

- What you found and why it's a security issue, not just a bug
- A minimal `.nir` file that reproduces it
- The affected area if you know it (parser/typeck/ownership, `smt.rs`
  refinement checking, the compiled `requires(role/claim:...)`/
  `acquire`/`check_role` enforcement path (`codegen.rs`), or the
  `emit-ui`-generated static client). `sandbox` process isolation and a
  `serve.rs` HTTP/auth boundary are historical — the interpreter and
  `serve.rs` were deleted entirely and neither exists on any current
  branch; see [`docs/API_TRUST_MODEL.md`](./docs/API_TRUST_MODEL.md) §4a.

You should get an initial response within a week. This is a small team
(see [`MAINTAINERS.md`](./MAINTAINERS.md)), not a funded security team —
response time is best-effort, not SLA-backed.

## Scope

Nirdosha is under active development. Some safety properties are proven
today (ownership/affine types, SMT-discharged overflow bounds, the
concurrency model); others are explicitly aspirational and documented
as such — see [Honest Scope & Roadmap](https://github.com/kannamma-labs/nirdosha/wiki/Honest-Scope-and-Roadmap).
A report that a *documented, disclosed* limitation is exploitable is
still useful — please file it — but it's triaged differently from a
violation of a claim the project actually makes.

Three specific things the project does **not** currently claim, named
here so a report against them is triaged as a known gap rather than a
new finding: `&` references have no `&mut`-style liveness/exclusivity
enforcement beyond "the referent isn't already moved" (only `box`'s
affine tracking is fully enforced — see `ownership.rs`'s own doc
comment); there is no built-in audit-trail feature; and the built-in
crypto (`hmac`/`sha2`/`ring`) is standard RustCrypto, not a NIST
CMVP-validated module, so FIPS 140-3 is not met.

**2026-09 — there is no compiled serving mode.** The interpreter and
`serve.rs` were deleted entirely (`docs/API_TRUST_MODEL.md` §4a); there
is no `nirdosha serve` subcommand. `db`/`json`/`mq`/`transact`/`sandbox`
and most Row 12 identity builtins (`oidc_validate_token`,
`extract_claim`) don't run in any form today — `codegen.rs::
check_supported` rejects them and there's no interpreter left to fall
back to. What does compile and run for real, and is the actual
security boundary in a compiled binary today: `requires(role/claim:
...)`/`acquire`/`check_role` enforcement (`docs/API_TRUST_MODEL.md`
§4a's own 2026-09 update).

Areas most worth scrutiny:

- `codegen.rs`'s compiled `requires`/`acquire`/`check_role`
  enforcement and field-masking lowering — the actual security
  boundary in a compiled binary today, now that it's the only path
  that runs at all
- `ownership.rs`/`typeck.rs` — the static guarantees the whole project
  is built around; a real counterexample to "the type checker accepts
  it, therefore it's memory/race-safe" is a serious finding
- `emit-ui`'s generated static client — its field-level `view`/`edit`
  gate rendering is cosmetic only, with no server left to enforce it
  independently, since none exists
- Anything that would let untrusted `.nir` source (e.g. LLM-generated
  code fed through the agent-facing tooling) bypass a `requires(role:
  ...)`/`acquire` gate in a compiled binary

Out of scope: findings that require local code execution with the same
privileges as the `nirdosha` process itself, or that target the
deleted interpreter/`serve.rs`/`sandbox` runtime path (historical, not
present on any current branch).

## Supported versions

This project is pre-1.0 and does not yet have a formal support-window
policy — see Track A ("Compatibility/versioning policy") in
[`docs/PUBLIC_ROADMAP.md`](./docs/PUBLIC_ROADMAP.md). Until that lands, the only
supported version is the latest commit on `main`.
