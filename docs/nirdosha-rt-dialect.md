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

**Standing design rule: no bespoke parser, ever.** Every guarantee this
dialect makes — `requires(role)`, `nfr(..)`, and any future clause
(e.g. policy/`crud_op` gating) — must be real, stock `rustc` doing the
work: the type system (an unforgeable proof type), ordinary proc-macro
expansion (`syn`/`quote`, checked by `rustc` for every build, not an
opt-in tool), or `const`-evaluation (a `const _: () = assert!(..)` that
`rustc` itself refuses to compile past). It must **never** be a
hand-rolled scanner reading JSON out of a doc comment and checked only
by a separate, optional tool someone has to remember to run — that
demotes a compiler guarantee back to a lint one person forgot to invoke.
If a new mechanism can't be expressed as one of the three real
`rustc`-checked forms above, it isn't ready to ship as a guarantee yet.

---

## 1. The promise, precisely

| | plain `cargo` | `cargo nirdosha` |
|---|---|---|
| Compiles? | ✅ always — it *is* a Rust program | ✅ after verification |
| Runs? | ✅ | ✅ (same binary semantics + guards) |
| `effects(pure)` | fast local scan in the macro: an obvious lie is a `compile_error!` even here | full source verification of every claim, incl. hand-written doc contracts |
| `requires(role)` | **enforced by types** — unforgeable proof token, uncallable without a minted proof | same enforcement + the claim is verified and recorded |
| `nfr(..)` | enforced by an injected runtime guard (latency recorded, concurrency gated) | same + SLA lands in the certificate |
| dialect rules (`unsafe`, raw threads, raw locks) | not enforced (plain Rust) | **build errors** |
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
   to delegate on any violation, and writes the certificate. Stage 1.5
   adds the workspace gate: `cargo nirdosha verify --workspace` runs
   strict verification over every *in-dialect* crate (explicit opt-in:
   a dependency on `nirdosha-rt`, or
   `[package.metadata.nirdosha-rt] dialect = true`; the dialect's own
   toolchain crates are exempt via `toolchain = true`) and emits the
   aggregate certificate
   `target/nirdosha/contract-report-workspace.json`.

4. **The rustc driver** — Stage 2 (`crates/nirdosha-driver`), the
   certifying default since issue #74 (`--fast`/`--shallow` opts back
   into Stage-1-only): the Clippy architecture. The full rustc pipeline
   runs, and
   afterwards contract claims are verified over **MIR**:

   - **Interprocedural purity** — `effects(pure)` is checked against
     the transitive closure of the call graph, with the full chain
     named in the error (`net_total -> ledger_overrides -> read_ledger
     -> std::fs::read_to_string`).
   - **Real name resolution** — `std::fs::read_to_string` is recognized
     because it resolves to that `DefId`, not because its text looks
     like a path. Stage 1's over-approximate string matching is
     retired for anything compiled through the driver.
   - **Conservative local subset** — expanded HIR and pre-optimization MIR
     reject unsafe/static boundaries, reference writes, destructors and
     unresolved function/trait calls. Scalar arithmetic and local recursion
     are supported; this does not prove termination or panic freedom.
   - **No crate-wide trust** — external calls, including std/core/alloc and
     nirdosha_rt, require explicit effect summaries. Those summaries are not
     implemented yet, so iterator-heavy code and injected NFR guards are
     currently unsupported by deep checking. Plain Cargo remains usable.
   - **Pass-through for non-contract crates** — the driver adds no effect
     restrictions to crates without contracts.

   The prior runtime-wide NFR exemption was removed: it also admitted
   arbitrary runtime effects and callbacks. See [V2 guarantees](V2_GUARANTEES.md)
   for the supported subset and the [running book](V2_IMPLEMENTATION_BOOK.md)
   for migration evidence.

   Stage 2 is nightly + `rustc-dev` only (it links `rustc_private`);
   Stage 1 verification is stable and works everywhere, which is why it
   stays available as `cargo nirdosha build --fast`/`--shallow` for
   IDE-time feedback. `cargo nirdosha build` (no flag) wires the driver
   in as `RUSTC_WORKSPACE_WRAPPER` by default. Stage 2.5 now discharges numeric MIR
   assertions with shared Z3 integer semantics (or an explicit interval
   fallback) and emits bound proof records. Guarded division/indexing can
   pass purity checking. Actual MIR proof-elision remains follow-on work.
   See [MIR numeric proofs](MIR_NUMERIC_PROOFS.md) for scope and examples.

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

## 4. Honesty notes (what each stage deliberately is not)

- **Stage 1's `effects(pure)` checking is over-approximate and
  body-local.** It is path-based (`std::fs`, `Instant`, `.spawn()`,
  `unwrap`…) without full name resolution, and it cannot see through
  calls — `rt-payroll-pure-chain` passes Stage 1 (`--fast`/`--shallow`)
  and is refused by Stage 2, on purpose, in the repo. False positives
  are possible in Stage 1 and preferable to false negatives; Stage 2,
  the certifying default since issue #74, replaces the guessing with
  real resolution.
- **NFRs are enforced, not proven.** `latency_ms` is measured per call
  (flight recorder, `NIRDOSHA_NFR_LOG=1` for JSON lines on stderr,
  `NIRDOSHA_NFR_LOG_FILE=…` as the harness sink); `concurrency_max`
  is a blocking gate; `cargo nirdosha bench` gates the p95 under the
  package's own test workload. No solver proves wall-clock
  performance, and we never claim otherwise.
- **`Auth` is demo-grade** — the session asserts its roles. The
  enforcement invariant that matters is unchanged: only `Auth` mints
  proofs, every mint consults the session, and the proof is a type.
  Row-12 identity (`check_role` → unforgeable views) wires under this
  same interface in Stage 2.5.
- **Async fns are not yet contract-verified** (the macro refuses them;
  a hand-written doc contract on an async fn sees only the
  coroutine-construction body — the deep analysis of async work is
  Stage 2.5). Generics pass through both stages.
- **The driver is nightly + `rustc-dev` only** (it links
  `rustc_private`, pinned to the toolchain it was built against — the
  Clippy/Miri maintenance model). Stage 1 verification and everything
  the runtime enforces by types are stable and work everywhere. A
  version-check guard (issue #65 item 9 / #72) fails `--deep` closed
  with an actionable "rebuild against this toolchain" message on a
  mismatch, instead of the undefined behavior a symbol/ABI mismatch
  would otherwise risk; `deep-effects-next-nightly`
  (`.github/workflows/v2-guarantees.yml`) builds against a floating
  `nightly` daily so a breaking `rustc_private` change is caught the
  day it lands, not discovered cold on the next deliberate pin bump.
  Migrating the MIR walk itself onto `stable_mir`/`rustc_smir` (issue
  #72's other ask, to shrink how much of this churn surface exists at
  all) is blocked in this environment: neither crate ships in the
  pinned nightly's `rustc-dev` component today (confirmed by searching
  the installed sysroot — no `stable_mir`/`rustc_smir` `.rlib` or
  vendored source anywhere in it), so there is currently nothing to
  migrate onto.

## 5. What each surface catches (the honest matrix)

| Lie | plain cargo | `cargo nirdosha build --fast` (Stage 1) | `cargo nirdosha build` (Stage 2, default) |
|---|---|---|---|
| pure claim, direct `std::fs` call | runs | **compile_error** (macro scan) + verify refusal | **rustc error** |
| pure claim, file I/O 3 calls away | runs | passes (out of sight) | **rustc error, chain named** |
| `unwrap()` inside a pure fn | runs | flagged | flagged (exact, via MIR asserts) |
| typo'd contract key | runs | **refused** | **refused** |
| undeclared role in `requires` | fails to compile (type resolves nowhere) | same | same |
| unverified third-party call in a pure fn | runs | passes (string scan can't know) | **refused (default-deny)** |
| recursion + checked arithmetic in a pure fn | runs | passes | **passes** (back-edges contribute nothing; overflow is the documented guard) |
| unsafe / raw threads / raw locks | runs | **refused** | refused (same rules, spans from MIR) |
| SLA breach (`nfr(latency_ms)`) | invisible | `cargo nirdosha bench` gates p95 under the real workload | same (Stage 2.5 binds Z3-guided synthetic benches) |

## 6. Crate map

| Crate | Job |
|---|---|
| `crates/nirdosha-contract-core` | contract model + JSON encoding, attribute parser, impure/dialect scanners (shared by macro, compiler, driver) |
| `crates/nirdosha-macros` | `#[contract]`: parse, honesty-check locally, inject proof param + NFR guard, emit doc encoding |
| `crates/nirdosha-rt` | runtime: `roles!`, `RoleProof`, `Auth`, NFR guard + flight recorder; re-exports `contract` |
| `crates/cargo-nirdosha` | the Nirdosha compiler CLI: verify → refuse-or-delegate → certificates; `--workspace` strict gate; `bench` SLA gate; wires in the driver by default, `--fast`/`--shallow` opts out |
| `crates/nirdosha-driver` | Stage 2 rustc driver: MIR interprocedural effects, real name resolution, totality checks, default-deny third party |
| `examples/rt-payroll` | compliant program; all contract forms on display; passes the default (Stage 2) gate |
| `examples/rt-payroll-lying` | zero-dependency lying program; plain cargo runs it, nirdosha refuses it |
| `examples/rt-payroll-pure-chain` | the *indirect* lie: Stage 1 (`--fast`) passes it, the Stage 2 default refuses with the chain |

### Certificates: `nirdosha.certificate/v1`

Every verification mints a certificate at
`target/nirdosha/contract-report-<package>.json` (workspace gate also
writes `contract-report-workspace.json`). The envelope is shared by
design with the proprietary tier: `subject` (package), `tool` (which
surface verified — `source_scan` never invokes rustc and says so),
`sources` (every verified file, package-relative, SHA-256), `proofs`
(per-assertion MIR discharge records with an explicit backend; source scans
leave this empty), `signature` (reserved for the signed-plugin
trust chain), and `binding` — SHA-256 over all bound content.

Two deliberate properties: **determinism** (no timestamps, no ambient
state — same sources + tool give byte-identical certificates, so
re-verification is a diff, and zero diff means zero drift; bounded MIR
solver outcomes can still vary with resource availability), and
**auditable claims**:

```
$ cargo nirdosha verify --audit
nirdosha: audit binding OK — content matches its stored hash (5a6067…)
nirdosha: audit sources OK — 1 file(s) match their verified hashes
nirdosha: listed source hashes and certificate binding match; this does not authenticate the issuer or establish build provenance
```

Changing a listed source or editing claims without recomputing the binding
fails the integrity audit. An attacker can recompute an unsigned binding;
this is not issuer authentication. New source reports include bound
`verification.coverage` and a consuming policy command that refuses stronger
guarantees unsupported by the source scanner. See
[V2 guarantees](V2_GUARANTEES.md) for commands and limitations, and the
[running implementation book](V2_IMPLEMENTATION_BOOK.md) for migration gates.

Usage:

```
cargo build && cargo install --path crates/cargo-nirdosha   # or PATH=target/debug
cargo nirdosha build | check | run | test | verify          # per-package verification
cargo nirdosha verify --audit                               # listed-source and binding integrity only
NIRDOSHA_STRICT=1 cargo nirdosha build                       # strict: every pub fn carries a contract
cargo nirdosha verify --workspace                           # strict gate over all in-dialect crates
cargo nirdosha bench                                        # nfr(latency_ms) CI gate, real workload
cargo build -p nirdosha-driver && cargo nirdosha build          # Stage 2 (nightly + rustc-dev), default since #74
cargo nirdosha build --fast                                     # Stage 1 only, opt out of the driver
```

## 7. What this borrows from the `.nir` compiler (and what replaces it)

`effects.rs` semantics (the effect vocabulary, declared-vs-actual
checking) now live over Rust syntax as the MIR effect lattice.
The shared `nirdosha-smt-core` encoder now supplies numeric MIR assertion
proofs; [MIR numeric proofs](MIR_NUMERIC_PROOFS.md) describes the supported
subset. Contract checking on impl blocks and Row-12 identity remain future
work on the driver's MIR pass.
The `.nir` frontend (token/parser/ast) is *replaced by rustc itself* —
that is the point: we stop maintaining a grammar and start inheriting
the entire Rust ecosystem, LLM training priors included.

## 8. Relationship to the strategy

- **The category claim, sharpened:** "guarantees about the language, not
  the model" now applies to the language agents already write. The
  VeraBench-style benchmark for us becomes: *constrained-Rust beats
  constrained-new-language* for agent correctness, because the priors
  are free.
- **The certificate** is the commercial artifact: per-package and
  workspace JSON today, proof-carrying and hash-bound in Stage 2.5.
- **The demo** (`scripts/rt-dialect-demo.sh`) is the visceral one from
  the action plan: same source, two compilers, refusal with exact
  lines — 30 seconds, no setup beyond a `cargo build`.
