# Nirdosha — निर्दोष

<p align="center">
  <b>Write plain Rust. Get compiler-checked contracts.</b><br/>
  A Nirdosha program is a strict subset of Rust — it compiles and runs under a stock <code>cargo</code> toolchain like any Rust program, and under <code>cargo nirdosha</code> the same source is suddenly <i>verified</i>: contract claims become checks, and lying becomes a build error.<br/>
  <i>No bespoke language. No bespoke parser. Guarantees about the code, not a promise about the model that wrote it.</i>
</p>

[![build](https://github.com/kannamma-labs/nirdosha/actions/workflows/build.yml/badge.svg)](https://github.com/kannamma-labs/nirdosha/actions/workflows/build.yml)
[![license: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](./LICENSE)
[![Wiki](https://img.shields.io/badge/docs-wiki-blue)](https://github.com/kannamma-labs/nirdosha/wiki)
[![Contributing](https://img.shields.io/badge/CONTRIBUTING-read-blue)](./CONTRIBUTING.md)
[![Governance](https://img.shields.io/badge/GOVERNANCE-read-blue)](./GOVERNANCE.md)
[![Roadmap](https://img.shields.io/badge/ROADMAP-view-purple)](./docs/PUBLIC_ROADMAP.md)
[![Maintainers](https://img.shields.io/badge/maintainers-5-green)](./MAINTAINERS.md)
[![Sponsor](https://img.shields.io/badge/%E2%9D%A4-Sponsor-ea4aaa)](https://github.com/sponsors/arunsoman)
[![Open in GitHub Codespaces](https://github.com/codespaces/badge.svg)](https://codespaces.new/kannamma-labs/nirdosha?quickstart=1)

## Why this exists, precisely

Verification tools that require a bespoke language put a trapdoor in front of adoption: the guarantees only apply once you've already rewritten your code into something unfamiliar. Nirdosha removes that trapdoor. There is no separate grammar, no custom parser, no interpreter to install — a Nirdosha program *is* a Rust program. What changes is what happens when `cargo nirdosha` looks at it instead of plain `cargo`: contract claims written as attribute macros or doc comments stop being decoration and become checked facts.

*(This repo used to also ship a separate, from-scratch interpreted language — `crates/compiler`, retired 2026-09-20. That work continues elsewhere; what lives here is the free, Rust-native lane described above — see [`docs/nirdosha-rt-dialect.md`](./docs/nirdosha-rt-dialect.md) for the full design and the reasoning behind dropping a bespoke parser entirely.)*

## The promise, precisely

| | plain `cargo` | `cargo nirdosha` |
|---|---|---|
| Compiles? | ✅ always — it *is* a Rust program | ✅ after verification |
| Runs? | ✅ | ✅ (same binary, same semantics) |
| `effects(pure)` claim | not checked | verified against the function's real, resolved effects — an obvious lie is a build error even under plain `cargo`'s own macro expansion |
| `requires(role = "…")` | **enforced by types** — an unforgeable proof token, uncallable without one, under *any* compiler | same enforcement, plus the claim is recorded in the certificate |
| `nfr(latency_ms/…)` | enforced by an injected runtime guard | same, plus the SLA lands in the certificate |
| `logging(domain, country)` | policy resolved and guard injected; metadata recorded | same, plus the resolved policy hash lands in the certificate |
| Lying program | builds and runs | **refused, with the exact lines named** |

## See it refuse a lie — 30 seconds, no setup beyond `cargo`

```sh
git clone https://github.com/kannamma-labs/nirdosha.git && cd nirdosha
cargo run -p rt-payroll-lying          # plain cargo: runs happily
```
```
payroll auditor — audited "" in 16.874µs
```
```sh
cd examples/rt-payroll-lying && cargo nirdosha build   # same source, the real compiler
# nirdosha:expect-exit 1 -- this command is SUPPOSED to fail; that's the demonstration
```
```
nirdosha: error .../main.rs:32 fn `snapshot_audit` — claims effects(pure) but the body performs std::time::Instant::now (clock access) — a contract is a checked declaration, not a comment
nirdosha: error .../main.rs:33 fn `snapshot_audit` — claims effects(pure) but the body performs std::fs::read_to_string (file system access) — a contract is a checked declaration, not a comment
nirdosha: error .../main.rs:38 fn `mislabeled` — malformed `nirdosha:contract` JSON: unknown field `effekts`, expected one of `effects`, `requires`, `ensures`, `nfr`, `crud`, `resource`, `sequence`, `logging` at line 1 column 10
nirdosha: rt-payroll-lying — source_scan: 1 files, 2 contracts inspected, 3 violations
nirdosha: refusing to build — plain `cargo build` would have accepted this code; that difference is the product
```

Both transcripts above are real, unedited output from this repo, not illustrative text. [`examples/rt-payroll-lying/`](./examples/rt-payroll-lying/) is a 44-line file with three tiny functions, each annotated with its own doc comment explaining exactly what it's demonstrating.

The lie doesn't have to be in the same function to get caught, either. [`examples/rt-payroll-pure-chain/`](./examples/rt-payroll-pure-chain/) claims purity on a function whose own body is clean — the file I/O happens two calls deeper in the graph. `cargo nirdosha build`'s default (Stage 2, a real MIR call-graph pass) finds it and names the chain; `cargo nirdosha build --fast` (Stage 1, a body-local scan, for sub-second IDE feedback) doesn't, and says so honestly rather than pretending to be as strong as the default. See that file's own doc comment for exactly why.

## The compliant side — real contracts, real enforcement

[`examples/rt-payroll/`](./examples/rt-payroll/) is the "everything works" counterpart, three contract forms on display in one short file:

```rust
#[contract(
    effects(pure),
    requires(role = "hr_staff"),
    nfr(latency_ms = 250, concurrency_max = 4)
)]
fn compute_payroll(records: &[PayRecord]) -> u64 {
    records.iter().map(|r| r.gross_cents - r.gross_cents / 10).sum()
}
```

`requires(role = "hr_staff")` isn't a runtime `if` — the macro injects a leading parameter (`&RoleProof<HrStaff>`) with a private constructor. The only way to obtain one is `session.prove::<HrStaff>()` succeeding against a real login. Delete the `.prove()` call anywhere it's needed and the program **stops compiling** — uncallable is a property of the type system, true under any compiler, not a check that only fires when you remember to run one.

```sh
cargo run -p rt-payroll
# payroll: employees [1, 2, 3], net 271350 cents (gross 301500 cents)
# manager view refused: session does not hold role `manager`
# executive view: 60300 cents

cd examples/rt-payroll && cargo nirdosha verify
# nirdosha: rt-payroll — source_scan: 1 files, 3 contracts inspected, 0 violations
```

## The trust pipeline today — honest about what exists

`cargo nirdosha`'s real subcommands, verified against this repo, not aspirational:

| Command | What it does |
|---|---|
| `cargo nirdosha build` | Stage 1 scan + Stage 2 (MIR effect lattice over the real call graph), then `cargo build` |
| `cargo nirdosha build --fast` / `--shallow` | Stage 1 only — sub-second, no nightly/`rustc-dev` toolchain needed |
| `cargo nirdosha check` / `run` / `test` | pass-through `cargo` subcommands, contract-scanned first |
| `cargo nirdosha verify [--workspace]` | verify every contract + emit a certificate, no `cargo` delegation; `--provenance` binds `Cargo.lock` + toolchain, `--sign key.pk8` signs it |
| `cargo nirdosha bench` | gates `nfr(latency_ms: …)` against your own test suite as the real workload — no synthetic benchmark to maintain separately |
| `cargo nirdosha check-certificate <path> --require <guarantee>` | policy gate: does this certificate actually cover the guarantee you need? |
| `cargo nirdosha keygen` / `verify-certificate` | Ed25519 keypair generation and signed-certificate verification |

**What doesn't exist yet, said plainly rather than implied:** the retired interpreter's `fix`/`explain`/`certify`/`mcp`/`equivalence`/`attest`/`audit`/`suggest-contracts` subcommands have no v2-dialect equivalent today. `verify`'s certificate + `check-certificate`'s policy gate cover the "prove it, then let CI check the proof" loop; the rest is real, disclosed, unbuilt follow-on work, not a silent gap.

## Who this is for

- **Agent builders** — point your agent at plain Rust; the contracts it writes as doc comments or macros are checked, not trusted.
- **Platform & safety teams** — gate what agents can merge: `cargo nirdosha verify --workspace` in CI, a certificate on every build, `check-certificate --require <guarantee>` as the actual policy gate.
- **Auditors and compliance** — a certificate names its own evidence: which contracts were checked, against which source, with `--provenance` binding the toolchain and lockfile too.

## Why you won't be waiting on us

Every capability a Nirdosha program has is an ordinary Rust crate underneath — because it *is* Rust. Databases, TLS, JWT, HTTP servers, message queues: reachable the same day you add the dependency, not gated behind this project shipping a builtin for it. This is actually a stronger version of the old pitch, not a weaker one: there's no longer a separate interpreter's own feature surface standing between "the language supports it" and "the Rust ecosystem already has ten crates for it."

## What's real here today

Verified directly in this repo, not from a roadmap:

- **The contract macro/doc-comment forms** above — `#[contract(...)]` and the zero-dependency `nirdosha:contract {...}` doc-comment form, both checked by real `rustc`-driven passes (a proc macro for the attribute form, a MIR-level driver for the deep call-graph check), never a hand-rolled scanner reading JSON that a separate optional tool might forget to run.
- **A real guard/policy system** ([`crates/nirdosha-guard-core`](./crates/nirdosha-guard-core/) + friends) — role-based access control, field masking, audit chains with hash-chain integrity, lineage tracking, and a real, TLS-pooled Postgres store driver ([`crates/nirdosha-guard-store-postgres`](./crates/nirdosha-guard-store-postgres/)) with RLS enforced via session variables, not generated per-tenant views.
- **A worked, deep case study** — [`examples/rtm/`](./examples/rtm/) maps a full real-time transaction monitoring system (~150 screens, dozens of guard policies) onto this platform, cross-referenced against the RFCs that specify it, gaps disclosed inline rather than glossed over.
- **A real workflow engine, lineage plane, and MCP integration** — see [`crates/nirdosha-workflow`](./crates/nirdosha-workflow/), [`crates/nirdosha-lineage`](./crates/nirdosha-lineage/), [`crates/nirdosha-guard-mcp`](./crates/nirdosha-guard-mcp/).
- **RFC- and ADR-driven development** — every non-trivial design decision has a written record: [`rfcs/`](./rfcs/README.md) for proposals discussed up front, [`docs/adr/`](./docs/adr/README.md) for judgment calls made while building, including the honest costs and open questions, not just the wins.

**Not yet built, disclosed rather than silently absent:** live driver-capability attestation (a manifest's claims aren't cross-checked against reality yet), read-path query execution against a real store (only the write path has a production driver today), and the old interpreter's richer trust-pipeline subcommands named above. See [`docs/V2_GUARANTEES.md`](./docs/V2_GUARANTEES.md) and [`docs/V2_IMPLEMENTATION_BOOK.md`](./docs/V2_IMPLEMENTATION_BOOK.md) for the full, current guarantee profile and status.

## How to help

Small team — high-context contributions matter more than volume. See [`MAINTAINERS.md`](./MAINTAINERS.md) for who has write access. [`AREAS.md`](./AREAS.md) lists subsystem owners; cross-cutting or breaking changes go through the [RFC process](./rfcs/README.md) first — see [`GOVERNANCE.md`](./GOVERNANCE.md) and [`CONTRIBUTING.md`](./CONTRIBUTING.md).

| If you care about | Try |
| --- | --- |
| The contract macros, effect verification, PL theory | `crates/nirdosha-macros`, `crates/nirdosha-driver`'s MIR effect lattice |
| Real backends, data guards, storage | `crates/nirdosha-guard-*` — a second production store driver, or the read-path execution gap named above |
| Case studies, worked examples | `examples/rtm/` and the pattern it follows for a new vertical |
| Docs / DX | Error-message clarity, the FAQ below, missing examples |

## Documentation

- [`docs/nirdosha-rt-dialect.md`](./docs/nirdosha-rt-dialect.md) — the full design: the three-layer enforcement ladder (types → macro → MIR driver), and why no guarantee here is allowed to be a hand-rolled scanner.
- [`docs/V2_GUARANTEES.md`](./docs/V2_GUARANTEES.md) — the target guarantee profile.
- [`docs/V2_IMPLEMENTATION_BOOK.md`](./docs/V2_IMPLEMENTATION_BOOK.md) — the real, dated work log.
- [`rfcs/README.md`](./rfcs/README.md) / [`docs/adr/README.md`](./docs/adr/README.md) — proposals and judgment calls, respectively.
- [SECURITY.md](./SECURITY.md) — an open invitation to find a way past a guarantee this project claims, not just a reporting form.

## FAQ (short version)

**Do I have to rewrite my code?** No — that's the whole point. If it's valid Rust, `cargo nirdosha` can look at it; contracts are additive annotations, not a rewrite.

**Is it production-ready?** Pre-1.0 and moving fast. The guard/policy system, the contract macros, and the workflow/lineage layers are real and tested; the trust-pipeline subcommand gap above is real too. Read [`docs/V2_GUARANTEES.md`](./docs/V2_GUARANTEES.md) before betting production traffic on any specific guarantee.

**Why not just use Rust's own type system?** You are — Nirdosha adds a small set of additional, compiler-checked claims (effect purity, role-gated callability, NFR tracking) on top of it, not a replacement for it.

**What does a certificate actually prove?** Which contracts were checked, against which source files, with how many violations found — replayable by re-running `cargo nirdosha verify` yourself. `--provenance` extends that to the toolchain and lockfile.

**Found a bug, or a way past a guarantee this project claims?** [SECURITY.md](./SECURITY.md) is the place — two internal findings of exactly that shape are already fixed and on the record there.

**Want to contribute?** See [CONTRIBUTING.md](./CONTRIBUTING.md).

---

<p align="center">
  <b>दोष</b> = fault. <b>निर्</b> = without.<br/>
  <i>निर्दोष — the language named after its own guarantee: what the compiler accepts is, provably, without fault.</i>
</p>
