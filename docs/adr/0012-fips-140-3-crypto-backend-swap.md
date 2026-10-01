# 0012: A CMVP-validatable crypto backend behind a `fips` feature flag

Date: 2026-09-15
Status: accepted

## Context

`SECURITY.md`/`ROADMAP.md` already disclosed the gap honestly:
`ring`/RustCrypto `sha2` back this project's real cryptographic
operations, and neither is a NIST CMVP-validated module — a government
or regulated deployment requiring FIPS-140-3-validated crypto could not
use Nirdosha's Ed25519 signing or `sha256_hex`/JWT builtins as-is
(`ROADMAP.md`'s "Government software" row: "Nirdosha's cryptography ...
are standard RustCrypto software implementations, not a NIST
CMVP-validated cryptographic module").

Real usage was confirmed before any fix was designed, not assumed:
`ring::signature::Ed25519KeyPair` signs/verifies pack manifests and
certificates (`hi_plugin.rs`, `mcp_tools.rs`, `main.rs`'s
`keygen`/`sign`/`verify` commands); `jsonwebtoken` (which defaults to
`ring` internally) backs JWT issuance/validation and the RFC 7638 DPoP
JWK thumbprint in `runtime-kernels`.

**A real, unplanned finding surfaced while tracing every crypto call
site, not part of the original scope:** `nir_sha256_hex` — the actual
`.nir`-facing `sha256_hex` builtin backing pack/content integrity
hashing — was a from-scratch, hand-rolled SHA-256 implementation, not
the `sha2` crate, on a premise that had gone stale. Its own old doc
comment said this file "is compiled as an isolated `rustc
--crate-type staticlib` invocation with no `--extern` flags... so it
has no access to Cargo dependencies at all" — true of a build path
`crates/compiler/build.rs` stopped using once it switched to `cargo
rustc` (full Cargo dependency resolution), a fact this exact file's own
neighboring `dpop_jwk_thumbprint` already disproved by calling
`sha2::Sha256::digest` successfully in the identical compiled
staticlib. An unaudited hand-rolled primitive on the language's most
load-bearing crypto builtin was never something a real dependency
constraint justified — it just never got revisited after the
constraint disappeared.

## Decision

**Add a `fips` Cargo feature to both `crates/compiler` and
`crates/runtime-kernels`, off by default, that swaps the crypto
implementation behind a small re-export shim (`crypto_backend.rs` in
each crate) rather than touching call sites throughout the codebase.**

- **Compiler crate** (`crates/compiler/src/crypto_backend.rs`):
  re-exports `ring::{rand, signature}` (default) or
  `aws_lc_rs::{rand, signature}` (`fips`) under one path; a `sha256`
  function backed by `sha2::Sha256` (default) or `aws_lc_rs::digest`
  (`fips`). Every real `ring::`/direct-`sha2` call site now goes
  through it: `hi_plugin.rs`, `mcp_tools.rs`, `main.rs`'s
  `keygen`/`sign`/`verify` commands, `hi_graph::sha256_hex`, and this
  session's own `audit_chain.rs`.
- **Runtime-kernels crate** (`crates/runtime-kernels/src/kernel/
  crypto_backend.rs`): re-exports `jsonwebtoken` (default) or
  `jsonwebtoken-aws-lc` (`fips`, a maintained fork at the same 9.x
  version — confirmed a real drop-in via a scratch build: identical
  `encode`/`decode`/`Header`/`Algorithm`/`DecodingKey` API, zero
  call-site changes needed) under one name; a `sha256_concat` function
  with the same `sha2`-vs-`aws_lc_rs::digest` split. Every JWT/DPoP call
  in `lib.rs` goes through it, and `nir_sha256_hex`'s hand-rolled
  implementation (~80 lines: `SHA256_H0`/`SHA256_K`/`sha256_compress`)
  was deleted and replaced with a single call through this shim.

**Why `aws-lc-rs`, checked before committing, not assumed compatible:**
its API was built to mirror `ring`'s own `rand`/`signature`/`digest`
surface closely enough that the swap is a pure re-export, not a
rewrite. Before touching any real file: a scratch build of `aws-lc-rs =
{ features = ["fips"] }` was run in this environment to confirm its
real build-time requirement (`cmake`/`go`/`clang`, for the vendored
AWS-LC FIPS module's own C/Go build) is actually satisfiable here, and
a real Ed25519 sign/verify + SHA-256 digest round-trip against it was
checked against known-correct values before any production code
changed.

**Why a re-export shim, not a runtime-selectable backend.** The two
implementations are selected at compile time, not by a config value a
deployment could flip without rebuilding — matching how every other
build-time posture in this codebase works (`z3/vendored`, `native-tls`
vs `openssl` vendoring) and avoiding the far larger surface area of
making cryptographic backend selection a runtime decision.

## Consequences

**What this makes possible**: a regulated deployment builds with
`cargo build --features fips` (compiler) and `cargo build
--no-default-features --features fips` (runtime-kernels; mutually
exclusive with the plain `jsonwebtoken` dependency, hence
`--no-default-features`) to get every real cryptographic operation
(Ed25519 pack/certificate signing, JWT issuance/validation, SHA-256
hashing) running through `aws-lc-rs`'s CMVP-validatable module instead
of `ring`/RustCrypto `sha2`. Confirmed by building and testing both
crates' `fips` feature for real in this environment — not just the
scratch harness — including that `hmac_sha256_tests`' RFC 4231 vectors
and the new digest tests produce byte-identical output to the default
backend.

**What this does *not* make possible, stated so it's never misread
later.** "Links against a CMVP-validated module" is a
**compliant-module** claim about the algorithm implementation this
binary calls into. It is **not** a validation claim about *Nirdosha's
own compiled binary* — CMVP validation certifies one specific frozen
artifact its vendor submitted (AWS's own AWS-LC-FIPS builds carry real
certificate numbers on NIST's site); it does not automatically extend
to every downstream program that happens to statically link against
it. A deployment wanting to say "this binary is FIPS 140-3 validated"
still needs its own build to go through CMVP itself, or to resolve
whatever boundary rules apply to a statically-linked consumer of an
already-validated module — a real question for whoever owns that
compliance claim, not one this ADR answers. What this decision does
close is the real, necessary precondition: the cryptographic
*operations themselves* no longer run through an implementation that
could never be part of such a claim in the first place.

**A quality fix that rode along, not scope creep** — the hand-rolled
`sha256_compress` deletion removes ~80 lines of unaudited crypto code
from the language's most load-bearing crypto builtin regardless of
which feature flag is active, since the default backend now calls the
real `sha2` crate too. This was found by tracing every real crypto call
site for the `fips` swap, not searched for independently, and is
exactly the kind of gap that stays invisible until someone has a reason
to look at every single one.

**What's still open, not folded into this claim**: FIPS 140-3 covers
far more than "which module hashes/signs" — self-tests, key-management
*procedure*, physical security (N/A for pure software, but still a
checklist line to mark N/A explicitly), and the CMVP submission process
itself are none of them things source code can self-attest to. See
`docs/research/2026-09-proof-tier-and-scope-gap-tracker.md` for the
fuller disclosed-gap tracking this ADR's decision closes one entry of,
and the same day's "Standard/Requirements/Evidence" compliance-graph
work (RFC 0016's compliance profiles, wired for the first time the same
session) for the general mechanism a fuller FIPS-140-3 requirement
graph would eventually hang off of.
