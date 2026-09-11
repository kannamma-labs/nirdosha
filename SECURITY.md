# Security Policy

## Red-team invitation

This is an active invitation, not just a passive reporting policy:
**try to break the guarantees this project actually claims, and tell
us what you find.** Certora's own audit ethos is the model here — a
formal-methods project's credibility comes from inviting scrutiny of
its specific, falsifiable claims, not from asserting safety and hoping
nobody checks. Two internal findings already show this isn't just
posture: a self-red-team pass against `docs/API_TRUST_MODEL.md` found
`serve.rs`'s dispatcher was default-*open*, not default-deny (79 of 246
functions in a shipped example were callable with no token at all,
mutating ones included), and that JWKS validation only ever checked a
symmetric key, accepting no mainstream IdP's real key material — both
root-caused, fixed, and left in `docs/ROADMAP.md` (search "A10"/"A11")
as the record, not quietly folded away. An external finding of the same
shape and severity is exactly what this invitation is for.

**What counts as a real finding:** a concrete `.nir` file (or, for the
compiled `serve` surface, a request) that violates a guarantee this
project actually makes today — see "Areas most worth scrutiny" below,
and `docs/LANGUAGE.md`/`docs/PUBLIC_ROADMAP.md` for what's claimed
`[DONE]` versus `[PARTIAL]`/`[OPEN]`. A report against a limitation this
project already discloses (this file's own "Scope" section, or a
`[PARTIAL]`/`[OPEN]` roadmap tag) is still worth filing — it's triaged
as confirming a known gap, not dismissed — but it's a different kind of
finding than a claimed `[DONE]` guarantee failing.

**How to submit:** the same private channel as any other vulnerability
report — see "Reporting a vulnerability," directly below. Include the
reproducing `.nir` file/request and which specific claim (cite the
doc and line, or the exact sentence) it violates. There is no bounty
program today (a small, self-funded team — see `MAINTAINERS.md`) — the
reward on offer is a real fix, credit in the fix's commit/changelog
unless you'd rather stay anonymous, and, for a marquee-severity find,
a public disclosure write-up once it's fixed (the same treatment the
two internal findings above got).

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
  `acquire`/`check_role` enforcement path (`codegen.rs`), `nirdosha
  build --serve`'s compiled HTTP server, or the `emit-ui`-generated
  static client). `sandbox` process isolation and the old,
  interpreter-backed `serve.rs` HTTP/auth boundary are historical — the
  interpreter and `serve.rs` were deleted entirely and neither exists
  on any current branch; see [`docs/API_TRUST_MODEL.md`](./docs/API_TRUST_MODEL.md) §4a.

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

**2026-09 — the interpreter and `serve.rs` were deleted entirely**
(`docs/API_TRUST_MODEL.md` §4a); there is no `nirdosha serve`
subcommand, and never will be again. `db`/`json`/`mq`/`transact`/
`sandbox` and most Row 12 identity builtins (`oidc_validate_token`,
`extract_claim`) don't run in any form today — `codegen.rs::
check_supported` rejects them and there's no interpreter left to fall
back to.

**A real compiled serving mode exists now, though, under a different
name: `nirdosha build --serve`** (`rfcs/0010-landing-and-serve-exposure.md`,
`docs/PUBLIC_ROADMAP.md`'s Track B "B8"). It compiles a real HTTP server
directly into the binary, deny-by-default on route exposure
(`typeck::check_serve_exposure` — a function is reachable only if it's
screen/dashboard-bound or named in an explicit `serve { expose ... }`
entry), enforcing the same compiled `requires(role/claim: ...)`/
`acquire`/`check_role`/field-masking this file's own "Areas most worth
scrutiny" section below already names, now reachable over a real
socket rather than only via a direct function call in the same binary.
This is new, real, unaudited-by-anyone-outside-this-project surface —
if you're looking for where to start, start here.

Areas most worth scrutiny:

- `nirdosha build --serve`'s compiled HTTP server — request parsing,
  routing, and `typeck::check_serve_exposure`'s deny-by-default
  exposure-set computation itself, now that it's a real listener
  reachable over a socket rather than only a direct function call
- `codegen.rs`'s compiled `requires`/`acquire`/`check_role`
  enforcement and field-masking lowering — the actual security
  boundary in a compiled binary today, now that it's the only path
  that runs at all
- `ownership.rs`/`typeck.rs` — the static guarantees the whole project
  is built around; a real counterexample to "the type checker accepts
  it, therefore it's memory/race-safe" is a serious finding
- `emit-ui`'s generated static client — its field-level `view`/`edit`
  gate rendering is cosmetic only; whether the compiled `serve` server
  it talks to actually enforces the same gate server-side (not just
  hides the control client-side) is exactly the kind of gap worth
  probing
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
