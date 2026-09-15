# The Nirdosha Rust Dialect — Design Document (Stage 1)

**One sentence:** a Nirdosha program is a strict subset of Rust — it
compiles and runs under a stock toolchain like any Rust program, and
under `cargo nirdosha` the same source is suddenly *verified* Nirdosha
code: contract claims become checks, lies become build errors, and the
build earns a certificate.

This is the "CHECKED tier" of the master plan promoted to the core
design. It removes the adoption trapdoor ("guarantees require `.nir`"):
the verification layer now runs on the language AI agents already
emit natively. The `.nir` language stays the reference implementation of
the spec; this dialect is the adoption wedge.

---

## 1. The promise, precisely

| | plain `cargo` | `cargo nirdosha` |
|---|---|---|
| Compiles? | ✅ always — it *is* a Rust program | ✅ after verification |
| Runs? | ✅ | ✅ (same binary semantics + guards) |
| `effects(pure)` | fast local scan in the macro: an obvious lie is a `compile_error!` even here | full source verification of every claim, incl. hand-written doc contracts |
| `requires(role)` | **enforced by types** — unforgeable proof token, uncallable without a minted proof | same enforcement + the claim is verified and recorded |
| `nfr(..)` | enforced by an injected runtime guard (latency recorded, concurrency gated) | same + SLA lands in the certificate |
| dialect rules (`unsafe`, raw threads) | not enforced (plain Rust) | **build errors** |
| Certificate | none | `<target>/nirdosha/contract-report-<pkg>.json` |
| Lying program | builds and runs | **refused, with exact lines named** |

The demo pair in `examples/` is the whole story:

```
cargo run -p rt-payroll-lying          # plain cargo: runs happily
cargo nirdosha build                   # (in that package) REFUSED: 3 violations
```

## 2. The three-layer enforcement ladder

1. **Types** — `requires(role = "…")` expands to a leading parameter
   `&nirdosha_rt::RoleProof<crate::nirdosha_roles::HrStaff>`. The token
   has a private constructor; only `Auth::prove` mints it after a real
   role check. *Uncallable* is a property of types, not tooling: it
   holds under every compiler, forever. This is the strongest clause
   and needs no special compiler at all.

2. **Attribute macro** — `#[nirdosha_rt::contract(..)]`
   (`crates/nirdosha-macros`): parses the contract (unknown clauses are
   hard errors), scans pure-claiming bodies for impure operations
   (`compile_error!` — under plain cargo too), injects the proof
   parameter and the NFR guard, and emits the portable doc encoding.

3. **The Nirdosha compiler** — `cargo nirdosha`
   (`crates/cargo-nirdosha`), Stage 1: a source-scanning verifier that
   runs *in front of* cargo. Verifies both authoring forms (attribute
   and hand-written doc), enforces dialect-wide restrictions, refuses
   to delegate on any violation, and writes the certificate.

Stage 2 replaces the path-based scan with a rustc driver (HIR/MIR):
interprocedural effects, real name resolution, Z3 discharge of numeric
bounds, proof elision ("verified code runs faster"), and binding of the
NFR record into bench gates. The CLI and certificate artifact stay
identical, so Stage 2 is an upgrade, not a rewrite.

## 3. Two authoring forms, one encoding

**Attribute form** (ergonomic, does codegen):

```rust
#[nirdosha_rt::contract(effects(pure), requires(role = "hr_staff"),
                       nfr(latency_ms = 50, concurrency_max = 1000))]
fn compute_payroll(records: &[PayRecord]) -> u64 { ... }
```

**Doc form** (zero-dependency, verification-only):

```rust
/// nirdosha:contract {"effects":["pure"],"requires":{"role":"hr_staff"},"nfr":{"latency_ms":50,"concurrency_max":1000}}
fn compute_payroll(records: &[PayRecord]) -> u64 { ... }
```

Both compile to the same thing; the macro emits the doc encoding so the
contract travels with the compiled crate into rustdoc JSON, readable by
any agent or auditor with no Nirdosha tooling. Unknown keys are
`deny_unknown_fields` errors — a typo can never degrade into a silent
no-op. Role names in contracts resolve to same-named types declared by
`nirdosha_rt::roles!`; an undeclared role fails to compile because the
injected proof parameter does not resolve.

## 4. Honesty notes (what Stage 1 deliberately is not)

- **`effects(pure)` checking is over-approximate and local.** It is
  path-based (`std::fs`, `Instant`, `.spawn()`, `unwrap`…) without full
  name resolution, and it does not follow calls across functions. Stage
  2's driver closes both gaps; until then, false positives are possible
  and preferable to false negatives.
- **NFRs are enforced, not proven.** `latency_ms` is measured per call
  (flight recorder, `NIRDOSHA_NFR_LOG=1` for JSON lines on stderr);
  `concurrency_max` is a blocking gate. No solver proves wall-clock
  performance, and we never claim otherwise.
- **`Auth` is demo-grade in Stage 1** — the session asserts its roles.
  The enforcement invariant that matters is unchanged: only `Auth`
  mints proofs, every mint consults the session, and the proof is a
  type. Row-12 identity (`check_role` → unforgeable views) wires under
  this same interface in Stage 2.
- **Async fns and generic-heavy code** are not yet contract-verified
  (macro refuses async; generics pass through). Trait-decl contracts are
  recorded with a warning; impl-block checking is Stage 2.

## 5. Crate map

| Crate | Job |
|---|---|
| `crates/nirdosha-contract-core` | contract model + JSON encoding, attribute parser, impure/dialect scanners (shared by macro and compiler) |
| `crates/nirdosha-macros` | `#[contract]`: parse, honesty-check locally, inject proof param + NFR guard, emit doc encoding |
| `crates/nirdosha-rt` | runtime: `roles!`, `RoleProof`, `Auth`, NFR guard + flight recorder; re-exports `contract` |
| `crates/cargo-nirdosha` | the Nirdosha compiler CLI: verify → refuse-or-delegate → certificate |
| `examples/rt-payroll` | compliant program; all three forms on display |
| `examples/rt-payroll-lying` | zero-dependency lying program; plain cargo runs it, nirdosha refuses it |

Usage:

```
cargo build && cargo install --path crates/cargo-nirdosha   # or PATH=target/debug
cargo nirdosha build | check | run | test | verify          # per-package
NIRDOSHA_STRICT=1 cargo nirdosha build                      # every pub fn must carry a contract
```

## 6. What this borrows from the `.nir` compiler (and what replaces it)

`effects.rs` semantics (the effect vocabulary and the idea of
declared-vs-actual checking) move onto Rust syntax;
`smt.rs`/Z3, contract checking, and identity land in Stage 2 as HIR/MIR
passes. The `.nir` frontend (token/parser/ast) is *replaced by rustc
itself* — that is the point: we stop maintaining a grammar and start
inheriting the entire Rust ecosystem, LLM training priors included.

## 7. Relationship to the strategy

- **The category claim, sharpened:** "guarantees about the language, not
  the model" now applies to the language agents already write. The
  VeraBench-style benchmark for us becomes: *constrained-Rust beats
  constrained-new-language* for agent correctness, because the priors
  are free.
- **The certificate** is the commercial artifact: per-package JSON now,
  proof-carrying and hash-bound in Stage 2.
- **The demo** (`rt-payroll-lying`) is the visceral one from the action
  plan: same source, two compilers, refusal with exact lines — 30
  seconds long, no setup beyond a `cargo install`.