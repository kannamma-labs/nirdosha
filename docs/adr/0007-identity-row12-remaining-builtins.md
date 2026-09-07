# 0007: The rest of Row 12 — dotted-path claims, sessions, refresh tokens, revocation, API keys

Date: 2026-09-07
Status: accepted

## Context

`check_role`/`oidc_validate_token`/`extract_claim`/`identity_expired`
already compiled for real; the rest of Row 12
(`docs/nirdosha_row12_functions_identity.md`) — `check_role_path`/
`extract_claim_path`, `create_application_session`/`session_cookie`,
`new_refresh_token`/`exchange_refresh_token`, `check_revocation`,
`validate_api_key` — were already declared and typechecked
(`typeck.rs`'s own fixed signatures) but had no codegen at all. Several
of these needed real design decisions their typeck signatures didn't
settle on their own.

## Decision

**`validate_api_key`'s identity is minimal, not a lookup result** — its
fixed `(key: str, expected_hash: str) -> Result(VerifiedIdentity, str)`
signature has no third parameter for a richer source of identity data,
so it doesn't pretend to have one. On a constant-time hash match, the
returned `VerifiedIdentity` carries `subject = sha256_hex(key)` (a
stable, non-reversible identifier derived from the key itself), a fixed
`issuer = "api-key"`, empty claims, and `expires_at = i64::MAX` (not
`0`, which would make `identity_expired` treat every API-key identity
as already expired). Looking `expected_hash` up from wherever it's
actually stored is the caller's own job.

**Refresh-token redemption is single-use twice over.** `RefreshTokenHandle`'s
`box i64` field already gives a well-typed `.nir` program a compile-time
single-use guarantee. This adds a real, independent server-side table
(`kernel::identity`'s `refresh_table`) marking a handle id used on its
first `exchange_refresh_token` call — a second attempt, however it
happened, is a genuine `Err`, not silently reissuing again. Defense in
depth across the FFI boundary, not redundant with the type system: the
affine check only holds for code that actually goes through `ownership.rs`'s
checker.

**Session ids are real, unpredictable identifiers, not derived from
subject/issuer** — per-process entropy (real-time nanoseconds + PID,
seeded once) folded with a monotonic counter through the same `sha256`
helper `sha256_hex` already uses. The recovered interpreter-era design
this ports named the predictable-session-id failure mode directly; this
keeps that fix.

**Revocation fails open on absence.** `check_revocation` reads a
top-level `"revoked"` boolean; a token with no such claim (issued
before the claim existed, or from an IdP that never sets it) reads as
*not* revoked. Fail-closed would silently revoke every token that
predates the claim.

## Consequences

All eight builtins are real, compiled, and verified end to end through
a real compiled binary (`crates/compiler/tests/codegen.rs`'s
`row12_remaining_identity_builtins_compile_and_run_for_real`), including
the single-use enforcement actually firing on a second redemption
attempt — not just asserted in isolation.

**A real, pre-existing `codegen.rs` gap found and disclosed, not
fixed here**: `match_expr` resolves a match's own result type from
`local_ty_of(&arms[0].body, ...)` *before* `match_enum` binds the arm's
own pattern variable into scope — so a match arm that field-accesses a
match-bound aggregate value directly (`Ok(c) => c.value`) infers the
wrong type and produces a real, clang-caught IR type mismatch. No
existing test in the repo exercised this pattern before this phase's
own test tried to. Worked around in the new test (routing through a
function call, the same shape every other aggregate-returning match arm
in this test suite already uses) rather than fixed here — a general
`match_expr`/`local_ty_of` ordering issue is bigger scope than this
phase's own item.

**Honest cost, not hidden**: `validate_api_key`'s expected-hash lookup
(wherever the caller sources it from) is necessarily read fresh on
every call by the caller's own code — this builtin itself has no
caching or refresh story, since it holds no state between calls at all.
