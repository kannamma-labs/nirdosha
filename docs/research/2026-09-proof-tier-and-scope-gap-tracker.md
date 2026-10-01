# Pending work: proof-tier gaps and disclosed scope caveats (2026-09)

> Tracking doc for a set of real gaps surfaced in one review pass over
> `crates/bench/RESULTS.md`, `contract_check.rs`/`smt.rs`, and the
> scope-caveat language already in `README.md`/`SECURITY.md`/`ROADMAP.md`.
> Companion to
> [`2026-09-pending-verification-differentiation-work.md`](./2026-09-pending-verification-differentiation-work.md).
> Exists so this survives past one conversation. **Deliberately excludes**
> the demo-baseline critique (prompt-injection and race demos beating
> naive baselines rather than a competent engineer's real defense) —
> parked separately, not forgotten, not written up here yet.

## 1. Tier-1 can't model division's result — closes the `type_confusion` gap

`crates/bench/RESULTS.md` documents `average_no_float_confusion`
returning `UNKNOWN` (`contracts_unsupported: 1`) on both providers
tested, and a plain-LLM Rust baseline beating Nirdosha's own verdict on
the identical task purely because `i64` return type forces integer
division by construction (RESULTS.md:229-241). Root cause, confirmed by
reading the code: `smt.rs:527-544`'s `BinOp::Div | BinOp::Rem` arm
deliberately returns an unconstrained `Int::fresh_const("div_result")`
— division's result is asserted nowhere, on purpose (module doc,
`smt.rs:28-31`). `contract_check.rs:1348-1349` mirrors this as an
`Unsupported` bailout the moment a predicate walk hits a `Div`.

**Status:** ✅ Done, 2026-09-15. `smt.rs::div_rem` (`pub(crate)`, shared
by `contract_check.rs`) introduces the fresh `q`/`r` pair below and
asserts it permanently, exactly as specified — including the honest
soundness note this doc didn't originally spell out: when `b == 0` the
relation is unsatisfiable by construction, which is the *correct*
partial-correctness reading (a real zero divisor traps at runtime
before any postcondition check is reached), not an accidental "assume
nonzero" cheat. Both `smt.rs`'s own module doc and `contract_check.rs`'s
have been corrected to stop claiming division is unmodeled. New tests
pin the exact documented gap closing
(`average_no_float_confusion_now_proves_instead_of_unsupported`) and
that the encoding is really truncating, not Euclidean
(`division_result_is_truncating_not_euclidean`,
`division_result_rejects_the_euclidean_answer_as_a_real_counterexample`
— `crates/compiler/tests/contract_check.rs`). Full `smt`/`contract_check`
suites plus the whole compiler integration suite (`--tests --no-fail-fast`)
pass; the one pre-existing failure (`mq_publish_and_consume_round_trip_a_
real_message`, needs a live broker) is unrelated and unchanged.

**The real fix** (not the cheap partial-axiom shortcut — that only ever
proves the narrow bound shapes someone thought to axiomatize ahead of
time, capping what can ever be proved about division): model Rust's
actual truncating-division semantics as a real relation. Introduce
fresh `q`/`r`, assert:
- `a == b*q + r`
- `|r| < |b|`
- `r == 0 || sign(r) == sign(a)` (truncation toward zero, not SMT-LIB's
  native Euclidean `div`/`mod`, which the `z3` crate exposes but which
  don't match Rust's actual runtime behavior)

This is nonlinear integer arithmetic (`b*q`, both symbolic) — real risk
of incompleteness/timeouts on some inputs, disclosed honestly in the
same certificate rather than silently downgraded. Touches `smt.rs:527-544`
and the mirrored bailout in `contract_check.rs:1348-1349`.

## 2. Tier-1's scope rules out most of AlgoVeri by construction

`RESULTS.md`'s own "what this is not" section names this directly:
AlgoVeri's `binary_search` postcondition
(`forall i :: 0 <= i < result ==> s[i] < target`) needs loop-invariant
and quantifier reasoning; `contract_check.rs`'s module doc
(`contract_check.rs:13-18`) confirms the scope is deliberately integer
params/return only — no loops, no calls, no division, no quantifiers.

**Status:** ❌ Not started. This is a new, honestly-named tier, not a
Tier-1 extension.

**The real fix**, three pieces, needed together:

1. **Loop invariants, verified the standard Hoare way.** `While`
   already exists in the AST (`ast.rs:1309`) but Tier-1 bails on sight
   of one. Add an `invariant:` clause, parsed through the same
   predicate-string machinery `pre_logic`/`post_logic` already use.
   Generate three real obligations: invariant holds on entry; assuming
   invariant ∧ loop-condition, one arbitrary pass (loop-body-assigned
   variables havoc'd to fresh symbolic values first) preserves it;
   invariant ∧ ¬condition implies the postcondition. Same design
   Dafny/Boogie/Why3 already use.
2. **Quantifiers.** No `forall`/`exists` anywhere in `smt.rs` or
   `contract_check.rs` today. Z3's `forall_const`/`exists_const` are
   already reachable — confirmed present and unused in the vendored
   `z3` crate (`ast/mod.rs`, `pattern.rs`). Extend the Hoare-predicate
   parser to accept `forall i in a..b { ... }`, translate straight to
   Z3's native quantifier.
3. **Arrays as real SMT terms.** `.nir` already has fixed-length
   `Vector` array literals (`ast.rs:60`, `Expr::ArrayLit`) but `smt.rs`
   never lifts them into Z3's Array theory — required for any
   `forall i :: 0<=i<len ==> a[i]...` postcondition, the exact shape
   AlgoVeri's specs use.

## 3. Five disclosed scope caveats

| # | Caveat | Where disclosed | Real fix |
|---|---|---|---|
| 3a | Borrow checker is lexical, not true NLL — no tracking through reassignment, a field/index, or a call boundary | `ownership.rs`'s "Borrow liveness (v1, lexical)"; `README.md:140`; `docs/research/2026-09-pending-verification-differentiation-work.md:275-289` (previously reviewed and deliberately left alone) | A real CFG-based liveness analysis: compute each reference's true live range (creation → last dereference), same as `rustc` NLL. Track *places* (not just variable names) so `x.field`/`x[i]` are tracked independently. Give calls a conservative interprocedural summary (a reference passed into a call is "used" there; dead after, unless the callee's return type hands it back). |
| 3b | Sandboxed process isolation "not currently running in any form" | `README.md:278`; full design already written in `docs/SANDBOXING.md` | Build the already-designed spec: real OS-process `--sandbox-worker` re-exec, `chan`-typed sandbox arguments extended to a cross-process socket transport, sandbox lifecycle compiler-enforced. Design isn't the gap — the build is. |
| 3c | Race-freedom guarantee doesn't cover `db` — `killer_demo` corrupts the same ledger on both Nirdosha and naive Python | `README.md:84`; `docs/CONCURRENT_STATE_PATTERNS.md:7` | Three parts, see below: (i) ✅ done; (ii) the typed-capability structural fix — ❌ **investigated and NOT proceeding, on this project's own real evidence** (`rfcs/0015`); (iii) ✅ **a real, different, currently-open bug found and fixed instead** — see below. |
| 3d | No FIPS-140-3-validated crypto — `ring`/`sha2`/`jsonwebtoken` (RustCrypto/`ring`) back Ed25519 pack/certificate signing and JWT, not a NIST CMVP module | `SECURITY.md:105`; `ROADMAP.md:694`; real usage confirmed at `hi_plugin.rs`/`mcp_tools.rs` (`ring::signature::Ed25519KeyPair`) and `runtime-kernels`'s `jsonwebtoken` | 🟡 **in progress 2026-09-15** — see below. |
| 3e | No general-purpose audit trail — `docs/API_TRUST_MODEL.md`/`docs/goal.md` describe a `finish_with_audit`/`ledger.rs`/`capability.rs` hash-chain mechanism and claim it's "already in this codebase," but **neither file exists in this checkout** (confirmed by search, 2026-09-15) — only `workflow`'s `get_<workflow>_history` is real. Code-generation events aren't hash-chained anywhere. | `README.md:140`; `docs/API_TRUST_MODEL.md` T14 `[OPEN]`; `docs/goal.md` row 10 | ✅ **real minimal version done 2026-09-15** — see below. Generalizing to an `audited`-style attribute any type/mutation can opt into is still real, separate follow-up. |

**Status:** 3a and 3b (typed-`db`-capability half of 3c too) ❌ not
started — each is a structural, multi-subsystem rewrite this session's
own prior review already scoped correctly, not something that shrinks
under closer reading. 3c(i) ✅, 3e ✅ (real minimal version), 3d 🟡, all
2026-09-15 -- details below.

### 3c(ii), investigated — a typed `db` capability is NOT the next move

Asked directly: "can't we just implement the needed things and get
`db` race-freedom over with once and for all?" The obvious next step
(a `guard(keys) { ... }` keyed-mutual-exclusion primitive, giving `db`
critical sections the same by-construction guarantee `chan` has) turns
out to be something this project's own prior work already investigated
exhaustively — `rfcs/0015-keyed-guard-external-state.md`, nine real
review rounds plus three real experiments run against live SQLite/
Postgres, not a paper design. **Its own load-bearing finding: every
real motivating example (balance transfer, payment capture, low-stock
alert, room booking, two-warehouse transfer) collapses into a
single-statement SQL operation the database already serializes
correctly on its own** (`race_probe_atomic.nir`'s conditional `UPDATE`
for the exact `killer_demo` shape; `EXCLUDE USING gist` for the
booking case, 3/3 real runs each, `examples/killer_demo/
RESULTS_BOOKING.md`). The RFC's own decision rule: if the collapse
holds, `guard` does not proceed — and it holds. Building `guard` now
would mean building a mechanism this project's own real evidence
already shows doesn't address the actual problem (the race isn't
"no lock exists," it's "the naive code didn't use the atomic
single-statement form the database already provides") — not a matter
of effort, a matter of the evidence pointing somewhere else. Status
stays ❌, correctly, not because it's hard but because it's the wrong
target.

### 3c(iii), done instead — a real, different, currently-open bug found and fixed

RFC 0015's own experiment A3, run against real Postgres, surfaced a
real bug independent of `guard` entirely: `crates/runtime-kernels/src/
kernel/mod.rs`'s `HandleTable<T>::with` held **one process-wide mutex**
across the full duration of the blocking I/O call inside it — meaning
*every* `db`/`mq`/plugin-connection call in the whole process
serialized on one lock for its own network round-trip, and, worse, two
threads on *different* handles could genuinely deadlock the instant
one blocked on a server-side dependency the other would resolve (A3's
own reproduction: thread A blocks waiting on a Postgres lock while
holding the table mutex; thread B can never even start the unrelated
query — different handle, different connection — that would let it
release what A is waiting on, because it can't get past the same
table-wide mutex to look its own handle up). Confirmed general, not
advisory-lock-specific, with a plain `SELECT ... FOR UPDATE` control.

**Fixed 2026-09-15**: `HandleTable<T>`'s internal representation
changed from `Mutex<HashMap<i64, T>>` to `Mutex<HashMap<i64,
Arc<Mutex<T>>>>` — the table-wide lock is now held only long enough to
clone one `Arc` (never blocking); the actual blocking work runs under
just that one handle's own lock, so two threads on different handles
never contend at all, and two threads on the same handle still
correctly serialize (the right granularity — one handle is one
physical connection nothing could use from two threads at once
anyway). `remove` now uses `Arc::try_unwrap`, turning "every affine
handle type is single-owner by construction" from an assumed invariant
into a checked one (a genuine violation panics with a clear message
instead of silently doing the wrong thing). New regression test,
`with_on_two_different_handles_runs_concurrently_not_serialized`:
blocks a `with` call on handle A for up to 5s, asserts a concurrent
`with` on handle B completes in under 500ms — fails against the old
representation, passes against the new one. Full `runtime-kernels` lib
suite (both existing `HandleTable` tests plus the new one) and the
whole compiler crate's build/test suite confirmed green afterward
(only the pre-existing, unrelated `recorder.rs` ordering flake
remains). This affects every `HandleTable` consumer — `db`, `mq`,
plugin connections, channels, threads — not just the Postgres
advisory-lock path A3 happened to be testing.

### 3c(i), done — untracked `db` writes are now observable, not silent

`crates/runtime-kernels/src/kernel/isolation_check.rs::record_and_check`
used to return immediately, with zero trace, whenever a `db` op had no
active `transact` on its thread (`txn: None`). Now a **write** in that
state fires `escalate_untracked_write` — the identical
`NIRDOSHA_OBSERVABILITY_URL` fire-and-forget channel a real anomaly
uses, tagged `"untracked_db_write"` so a consumer can tell "no guarantee
was even possible here" apart from "a guarantee was checked and
violated." A bare **read** stays silent on purpose (can't itself
participate in a lost-update cycle). New tests: the wire-format pure
function (`untracked_write_body_shape_and_escaping`, mirroring
`nfr.rs`'s own `json_escape`/`parse_http_url` testing pattern since the
real HTTP path is untestable across a `cargo test`-cached env var) and a
read-stays-silent symmetry test. **Bonus, found while adding these
tests**: `isolation_check.rs`'s test module shared one process-global
`Mutex<Checker>` with no serialization between tests touching it — a
real, pre-existing flake (`record_and_check_is_a_noop_with_no_active_txn`
failed under parallel `cargo test`), fixed with a `TEST_LOCK` guard on
every test that touches `shared()`. Confirmed stable across repeated
runs; full compiler integration suite still green (only the pre-existing
unrelated MQ-broker test fails).

### 3e, real minimal version done — hash-chained audit log, `audit_chain.rs`

New module `crates/compiler/src/audit_chain.rs`: `AuditEntry { seq,
timestamp, content, prev_hash, hash }`, `hash = sha256_hex(prev_hash ++
canonical_json(seq, timestamp, content))`, `GENESIS_HASH` (64 `'0'`s)
for the first entry — the same byte-concatenation-then-hash scheme
`runtime-kernels`'s own `sha256_hex(prev_hash, payload)` 2-arg chained
form already implements for `.nir` programs, independently reimplemented
on the tooling side. `append_entry`/`verify_chain` are the two public
entry points; `verify_chain` walks the whole log and returns exactly
which entry broke (`HashMismatch` vs `ChainBroken`) rather than a bare
bool. Wired into the two real self-modifying-event logs that already
existed and were flat/non-tamper-evident before this session:
`hint_cache.rs`'s `HintCache::record_success` (a self-repair hint
auto-promoted after clearing a real diagnostic on the next attempt —
exactly T14's own worked example shape) and `RuntimeLessons::record` (a
human-confirmed `--teach` override). Five new tests in `audit_chain.rs`
itself (fresh-chain, tampered-content detection, spliced-entry
detection, missing-file-is-empty-not-an-error, genesis-length pin), all
passing. **Bonus, found while wiring this in**: `hint_cache.rs`'s own
test module mutates process-wide env vars
(`NIRDOSHA_RUNTIME_LESSONS_PATH`) with no serialization between tests —
a pre-existing flake that this session's added file I/O (the audit
read-before-append) widened enough to actually manifest; fixed with the
same `TEST_ENV_LOCK`-guard pattern used for 3c(i)'s discovery. Confirmed
stable across 8 repeated runs.

**What this is not**: the pack-signing log (`hi_plugin.rs::
append_pack_signing_log`) is the same shape and a real candidate for
the same treatment, deliberately not touched this session to keep this
change's blast radius honest (named as follow-up, not silently
skipped). And this closes T14 for the two events this codebase already
logged — it doesn't yet generalize to an `audited` attribute any
arbitrary mutation could opt into (the bigger version 3e's own "real
fix" column above describes).

### 3d, done — CMVP-validatable crypto backend, `cargo build --features fips`

**What this does and doesn't claim, stated explicitly so it's never
misread later:** "links against `aws-lc-rs`'s FIPS build" is a
**compliant-module** claim (the algorithm implementation this binary
calls into is one NIST's CMVP has validated a build of). It is **not**
a validation claim about *Nirdosha's own binary* — CMVP validation
certifies one specific, frozen compiled artifact submitted by its
vendor (AWS's own AWS-LC-FIPS builds carry real cert numbers on NIST's
site), not every downstream program that happens to link against it.
A deployment wanting to say "this binary is FIPS 140-3 validated" still
needs its own build to go through CMVP (or reuse AWS's certificate
under whatever boundary rules apply to a statically-linked consumer --
a real question for whoever owns that compliance claim, not one this
session answers). What this session's fix does close: the actual
cryptographic *operations* (Ed25519 sign/verify, SHA-256, JWT) no
longer run through an unvalidated implementation when the deployment
needs them not to. That's a real, necessary precondition for a FIPS
claim -- not the claim itself.

Real usage confirmed first, not assumed: `ring::signature::Ed25519KeyPair`
signs/verifies pack manifests and certificates (`hi_plugin.rs`,
`mcp_tools.rs`, `main.rs`'s `keygen`/`sign`/`verify` commands);
`jsonwebtoken` (defaults to `ring` internally) backs JWT issuance/
validation in `runtime-kernels`. Environment checked before committing
to the approach, not assumed: `cmake`/`go`/`clang` (aws-lc-fips-sys's
real build-time requirement) are present in this sandbox; a scratch
build of `aws-lc-rs = { features = ["fips"] }` compiled clean (~3 min)
and a real Ed25519 sign/verify + SHA-256 round trip against it matched
known-correct values before any real file was touched.

**Landed**: `crypto_backend.rs` in both `crates/compiler` (re-exports
`ring::{rand, signature}` or `aws_lc_rs::{rand, signature}`) and
`crates/runtime-kernels` (`kernel/crypto_backend.rs`, same idea for
`jsonwebtoken`/`jsonwebtoken-aws-lc` — confirmed by a real scratch build
to be a 9.x-API-compatible drop-in fork, zero call-site changes needed
beyond the re-export itself — and a `sha256_concat` covering both
`sha2::Sha256` and `aws_lc_rs::digest`). Every real `ring::`/
`jsonwebtoken::` call site now goes through the shim (`hi_plugin.rs`,
`mcp_tools.rs`, `main.rs` in `compiler`; every JWT/DPoP call in
`runtime-kernels/src/lib.rs`). Both `fips` features are optional and off
by default — `cargo build`/`cargo test` with no flags is byte-for-byte
unaffected; `cargo build --features fips` (compiler) and `cargo build
--no-default-features --features fips` (runtime-kernels) opt a
regulated deployment in. Both default-feature builds verified: full
signing test suite (`signed_certificate.rs`, `trust_audit.rs`,
`in_toto.rs`) and the full `runtime-kernels` lib suite pass (only the
pre-existing, confirmed-unrelated `recorder.rs` ordering flake remains).
Both `--features fips` builds confirmed real, not just the scratch
harness: `crates/runtime-kernels` (`cargo build --no-default-features
--features fips`, 7m05s, real AWS-LC-FIPS C/Go build) compiles clean
and its full lib test suite passes under it -- including
`hmac_sha256_tests`'s RFC 4231 vectors and `crypto_backend::tests`'s own
digest tests, byte-identical to the default backend (only the
pre-existing `recorder.rs` flake remains, present under both feature
sets, unrelated). `crates/compiler` (`cargo build --features fips`,
~6-7 min) compiles clean too. Every direct `sha2::`/`Sha256::` call in
both crates now goes through `crypto_backend::sha256`/`sha256_concat` --
swept for stragglers after the first pass and found two more real ones,
fixed the same way: `mcp_tools::sha256_hex` (used throughout `main.rs`
for source/bundle hashing) and `runtime-kernels`'s `dpop_jwk_thumbprint`
(RFC 7638 JWK thumbprint) each had their own direct `sha2` call still
un-swapped after the first `crypto_backend.rs` pass; both now route
through the shared function, `grep`-confirmed zero remaining direct
`sha2::`/`Sha256::` usage outside `crypto_backend.rs` itself in either
crate.

**Real, unplanned finding surfaced while doing this**: `nir_sha256_hex`
— the actual `.nir`-facing `sha256_hex` builtin backing pack/content
integrity hashing and this session's own `audit_chain.rs` — was a
**from-scratch, hand-rolled SHA-256 implementation**, not the `sha2`
crate, on a documented premise that turned out to be stale: its own old
doc comment said this file "has no access to Cargo dependencies at all"
because of a since-changed build path (`crates/compiler/build.rs` now
uses `cargo rustc`, full dependency resolution — proven by this exact
file's neighboring `dpop_jwk_thumbprint` already calling `sha2::` in
the same compiled staticlib). An unaudited hand-rolled primitive on the
language's most load-bearing crypto builtin was never justified once
that premise stopped holding; replaced with a real call through
`crypto_backend::sha256_concat` (~80 lines of hand-rolled compression-
function code removed). Verified byte-identical via the existing RFC
4231 HMAC-SHA256 test vectors (`hmac_sha256_tests`, which build on the
same `sha256` function) plus two new tests pinning the standard empty-
string vector and the streamed-concatenation property directly.

## Explicitly parked, not lost

**Demo-baseline fairness** (prompt-injection demo vs. a no-autoescaping
Python agent; race demo vs. raw unlocked threads) — real critique, real
fixes discussed (rebuild both baselines to actual best practice: field
masking + real sanitization for injection; a DB's default isolation
level or a correctly-used mutex for the race, targeting write-skew or
lock-ordering deadlocks instead of no-lock-at-all). Not written up here
per instruction — pick this up as its own item when asked.
