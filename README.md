# Nirdosha — निर्दोष ("without fault")

[![build](https://github.com/kannamma-labs/nirdosha/actions/workflows/build.yml/badge.svg)](https://github.com/kannamma-labs/nirdosha/actions/workflows/build.yml)
[![license: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE)
[![Wiki](https://img.shields.io/badge/docs-wiki-blue)](https://github.com/kannamma-labs/nirdosha/wiki)
[![Contributing](https://img.shields.io/badge/CONTRIBUTING-read-blue)](./CONTRIBUTING.md)
[![Governance](https://img.shields.io/badge/GOVERNANCE-read-blue)](./GOVERNANCE.md)
[![Roadmap](https://img.shields.io/badge/ROADMAP-view-purple)](./docs/PUBLIC_ROADMAP.md)
[![Maintainers](https://img.shields.io/badge/maintainers-5-green)](./MAINTAINERS.md)
[![Sponsor](https://img.shields.io/badge/%E2%9D%A4-Sponsor-ea4aaa)](https://github.com/sponsors/arunsoman)
[![Open in GitHub Codespaces](https://github.com/codespaces/badge.svg)](https://codespaces.new/kannamma-labs/nirdosha?quickstart=1)

> **A systems language built so an AI agent can write and run backend
> code with no human reviewing every line first.** No garbage
> collector, no data races, no deadlocks, no integer/buffer overflow —
> proven in the compiler today, not promised for later. Linux, macOS,
> and Windows binaries ship on every release, verified by CI on all
> three on every push.

Real, working software today: a real compiler (`crates/compiler/`) —
**no interpreter fallback anymore**, removed entirely and deliberately
so the compiled path can't quietly stay second-class. What that
compiler proves and runs, natively, right now: an LL(1) grammar
exported to GBNF for constrained decoding, SMT-backed (Z3) integer/
buffer-overflow proofs, ownership/affine types with no GC, real
identity (`check_role` producing an unforgeable `RoleView`) driving
automatic field-level data masking, and `nirdosha emit-ui`'s static UI
derivation from `struct`/`screen` conventions. `db`/`json`/`http`/`mq`/
`transact`/`workflow`/`sandbox` are fully designed and documented but
don't run in any form right now, compiled or otherwise, until each
gets real codegen — see [Track B](./docs/PUBLIC_ROADMAP.md) for
exactly what's landed and what's next.

## Two ways in

**Don't write code? You don't need to.** Paste
[`agent-skills/nirdosha/paste-anywhere-prompt.md`](./agent-skills/nirdosha/paste-anywhere-prompt.md)
into any LLM chat (ChatGPT, Claude, Gemini) and describe what you want
in plain English — it writes working `.nir` code for you. This exact
prompt has already produced a working e-commerce store, a food-delivery
platform, a telecom revenue-assurance system, and an online trading
platform, each hundreds of lines, each from an LLM with zero prior
Nirdosha exposure. See [LLM Integration](https://github.com/kannamma-labs/nirdosha/wiki/LLM-Integration)
for the full mechanism and evidence.

**Write code? Start with the language itself.**
[`examples/syntax/`](./examples/syntax/) is a progressive walkthrough —
`hello_nir.nir` to a multi-module enterprise app — one level at a
time. [`examples/features/`](./examples/features/) is the complete
reference: 50 files, one per language feature, from scalar types
through identity/masking to the declarative UI layer — see
`docs/LANGUAGE.md` §10 for exactly which of them `nirdosha build`
compiles today versus which are still waiting on Track B (per the
paragraph above, a real fraction of the catalogue doesn't run in any
form yet). Or skip both and jump straight into
[**GitHub Codespaces**](https://codespaces.new/kannamma-labs/nirdosha?quickstart=1) —
zero local setup, building in about a minute.

## What Nirdosha code looks like

**One function. Five independent guarantees the compiler itself checks
— not comments, not conventions, not a framework's runtime middleware.**
(Excerpt — `Text`/`main` omitted here, full runnable source linked below.)

```nirdosha
struct Employee {
    name: str,
    department: str,
    salary: f64 requires(role: "admin"),   // ← masked on return, not just "hidden in the UI"
}

fn get_employee(caller: RoleView, name: Text, department: Text, salary: f64) -> Employee
    effect(pure)                             // lying here is a build error, not a comment
    requires(role: "hr_staff")               // uncallable at all without a real proof of this role
    nfr(latency_ms: 50, concurrency_max: 1000)   // real APM tracking, zero code at the call site
{
    return Employee(name.value, department.value, salary)
}
```

```sh
git clone https://github.com/kannamma-labs/nirdosha.git && cd nirdosha
cargo run -p nirdosha --release -- build examples/features/50_field_masking_and_check_role.nir -o employee && ./employee
# 10           <- a Z3-proven Hoare contract elsewhere in the same file (see below)
# 150000.000000
# 0.000000     <- masked
# Ada Lovelace
# -3.000000    <- never even reached get_employee's body
# no_access
```

**Two independent gates, not one mechanism wearing two hats.**
`requires(role: "hr_staff")` on the function decides *who can call it
at all* — `get_employee`'s name has no direct-call path once gated;
the only way to obtain a callable value is `acquire
get_employee(proof)`, and it demands a real `RoleView` proving
`"hr_staff"`. `requires(role: "admin")` on the field separately decides
*what a successful caller sees back* — `salary` masks itself to zero on
every `return` unless the value passed as `caller` proves `"admin"`
specifically. Neither `RoleView` can be forged
(`RoleView("admin")` is a compile-time error) or fabricated by this
function's own logic — the only way to get a real one is
`check_role(identity, role)` succeeding against a real
`VerifiedIdentity`, itself compiled, not interpreted. The result: an
HR staffer who isn't also an admin can call the function and get real
employee records back with salary redacted; someone who's neither
can't obtain a callable `get_employee` in the first place — two
different failure points, two different roles, checked independently,
with no `if` anywhere in `get_employee`'s own body deciding either one.

`effect(pure)` is checked against what the function *actually does* —
annotate a function that performs I/O as `pure` and `nirdosha build`
rejects it, naming the real effect it found. `nfr(latency_ms: 50,
concurrency_max: 1000)` wires real per-call tracking into the APM kernel
with zero code at any call site, and escalates to
`NIRDOSHA_OBSERVABILITY_URL` automatically if one is configured. The
full file also carries a `validate` block with a genuine Z3-proven
post-condition (`tenure_bonus_pct`'s `result` provably stays in `[0,
20]` for *every* possible input, not just the ones a test tries) —
flip that bound to something false and `nirdosha build` fails outright,
naming a real counterexample. Full runnable source:
[`examples/features/50_field_masking_and_check_role.nir`](./examples/features/50_field_masking_and_check_role.nir).

## From two `struct`s to a generated UI

![Historical, pre-2026-09 evidence — captured before this repo's interpreter (`run`/`serve`) was removed. A themed dashboard with live SQLite data, a sortable/searchable table, and a role-gated approval action — the same screen under a lower-privileged identity, with a field dropped and an action disabled by the server](./demo.gif)

*Historical, kept for the record, not a claim about what runs today:
this GIF predates the interpreter removal above — the live,
DB-backed `nirdosha serve` it shows no longer exists in this tree, and
nothing currently replaces that *live* half of it.* What's real and
current instead: `nirdosha emit-ui <file.nir> -o out.html` derives a
static Material-styled page from the same `struct`/`screen`
conventions — no live backend, no server process — and the field
masking demonstrated above runs *underneath* any UI, in the compiled
binary itself, whether or not one is ever generated. See the
[UI Engine](https://github.com/kannamma-labs/nirdosha/wiki/UI-Engine)
wiki page for the full, current picture.

## Why this exists, in one paragraph

Nirdosha targets one specific problem: **a backend service written and
maintained by an AI coding agent, with no human reviewing every line
before it runs.** An LL(1) grammar exported to GBNF lets a sampler
force every token an agent emits to stay syntactically valid;
`nirdosha emit-ast` gives a self-repair loop a structured, typed AST to
work from instead of a paragraph to guess at; there is no mutex in the
language at all, so an agent literally cannot generate a lock-ordering
deadlock — a guarantee about what the *language* can express, true
regardless of which constructs are compiled yet. It isn't trying to be
a better Rust — see the
[wiki](https://github.com/kannamma-labs/nirdosha/wiki) for the full case.

## Nirdosha vs. Rust, Go, Mojo — the one-line version

| | **Nirdosha** | **Rust** | **Go** | **Mojo** |
|---|---|---|---|---|
| Target use case | LLM-written backend services, compliance CRUD | General-purpose systems | Cloud-native services | AI/ML-first, Python-compatible |
| Data-race freedom | Static | Static | Dynamic only | Not yet fully guaranteed |
| Deadlock freedom | No mutex primitive exists at all | Possible | Possible | Not a current guarantee |
| LLM writability | LL(1) grammar exported to GBNF for constrained decoding | LLMs default to Python 90–97% of the time | No constrained decoding built in | No published GBNF integration |

Full comparison in the
[wiki](https://github.com/kannamma-labs/nirdosha/wiki/Nirdosha-vs-Alternatives).

## Install

No compiler needed — prebuilt binaries are published for **Linux,
Windows, and Apple Silicon macOS** on every
[release](https://github.com/kannamma-labs/nirdosha/releases):

```sh
# macOS / Linux — installer script, auto-detects your platform
curl --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/kannamma-labs/nirdosha/main/scripts/install.sh | sh

nirdosha build examples/syntax/hello_nir.nir -o hello && ./hello
nirdosha emit-ui examples/features/39_screen_ui.nir -o ui.html   # static UI derived from a struct/screen block
```

```powershell
# Windows — PowerShell
irm https://raw.githubusercontent.com/kannamma-labs/nirdosha/main/scripts/install.ps1 | iex
```

Prefer not to pipe a script? Download the binary straight from the
release instead — `.../releases/latest/download/<asset>` always
resolves to the newest release:

```sh
# Linux x86_64
curl -fsSL https://github.com/kannamma-labs/nirdosha/releases/latest/download/nirdosha-x86_64-unknown-linux-gnu.tar.gz | tar xz

# macOS, Apple Silicon
curl -fsSL https://github.com/kannamma-labs/nirdosha/releases/latest/download/nirdosha-aarch64-apple-darwin.tar.gz | tar xz

./nirdosha build examples/syntax/hello_nir.nir -o hello && ./hello
```

(Intel Mac: build from source for now — see below.)

Building from source needs `clang` and `z3` (`apt install clang
libz3-dev` / `brew install llvm z3` / `pacman -S clang z3`) — or skip
the setup entirely with [Codespaces](https://codespaces.new/kannamma-labs/nirdosha?quickstart=1).
Full install matrix, scaffolding a new project, and generating a UI:
see [Getting Started](https://github.com/kannamma-labs/nirdosha/wiki/Getting-Started)
in the wiki.

### Before you write your own program

The examples above run as-is, but these four things will trip up your
*first original line* — they're parse/type errors, not style nits:

- **Enum variants are calls, always with `()`.** `Some(5)`, `None()`,
  `Circle(r)` — a zero-payload variant still needs the parens.
  `Color::Red` also works (optional disambiguation sugar), but a bare
  variant name never takes the place of a call.
- **`str` can't be a function's parameter or return type.** Use an
  `enum` for categorical data, or `struct Text { value: str }` to pass
  free text.
- **No string concatenation or formatting.** A `str` value only ever
  comes from a source literal or a builtin — there's no `+` or format
  string to build one at runtime.
- **No statement separators.** The parser always extends the current
  expression across a newline — `return x` then `-y` on the next line
  parses as `return (x - y)`, not two statements.

Full rationale and the complete list: [`AGENTS.md`](./AGENTS.md).

## What's shipped

Real, compiled, and running today — the highlights:

- **Language core** — LL(1) grammar cross-verified against an
  independent LALR(1) generator, a static type checker,
  ownership/affine types (`box`/`&`/`froze`, no GC, no manual `free`),
  `spawn`/`thread`/`chan` with no mutex in the language, generics,
  `Option`/`Result`, SMT-backed (Z3) integer/buffer-overflow proofs,
  and `validate { pre:/post: }` Hoare contracts (Z3-proven at build
  time for Tier-1 integer functions).
- **Native codegen** — LLVM `-O2` compilation for the numeric/
  control-flow/`box`/`froze`/`str`/`tcp`/`file`/concurrency subset,
  within 1.4× of `gcc -O2` on the operations it covers.
- **Identity and data protection** — `check_role` against a real
  `VerifiedIdentity`, producing an unforgeable `RoleView`; field-level
  `requires(role/claim: ...)` masking that zeroes a struct field on
  return unless the caller's own `RoleView`/`ClaimView` proves it;
  function-level `requires(role/claim: ...)` + `acquire`, gating
  whether a function is even callable at all — all three compiled, all
  enforced in the binary itself, no server process involved.
- **`nfr(...)`** — non-functional requirements as a first-class,
  compiled fn annotation: automatic latency/error-rate/throughput/
  concurrency tracking via the APM kernel, with async escalation to an
  observability endpoint on a crossed threshold.
- **UI engine (static)** — `nirdosha emit-ui` derives a Material-styled
  page from `struct`/`screen`/`dashboard` naming conventions — no
  hand-written frontend code, no live backend.
- **Cross-platform CI** — Linux, macOS, and Windows all build and run
  their full test suite on every push, not just at release time.

**Not currently running in any form** — designed, documented, and (for
most of these) previously interpreter-backed, but with the interpreter
removed and native codegen not yet reaching them: `db`, `json`,
`http`/`https`, `mq`, `transact`, `workflow`, `sandbox`, and the live
(server-backed) half of the UI engine. This is the deliberate, disclosed
trade the interpreter removal made — see
[Track B](./docs/PUBLIC_ROADMAP.md) for what's landed since and what's
next; treat `docs/PUBLIC_ROADMAP.md`'s older entries with the same
caution, since large parts of it still describe the pre-removal,
interpreter-parity world.

## How to help

Small team, high-context contributions matter more than volume — see
[`MAINTAINERS.md`](./MAINTAINERS.md) for who has write access and how
active each is. Issues are labeled `good first issue` / `help wanted` /
`compiler` / `llm` / `infra` / `documentation` (full set:
[`.github/labels.yml`](./.github/labels.yml)) — `good first issue`
tickets don't need a "may I?" comment first, just send the PR. Your
first issue or PR here won't land in silence:
[`welcome.yml`](./.github/workflows/welcome.yml) posts a real, specific
reply, not boilerplate.

| If you care about | Try |
|---|---|
| Ownership/concurrency, PL theory | A `Track B` codegen gap, or an SMT/typeck edge case |
| Constrained decoding, agent repair loops | `crates/bench/`'s pass@1 harness — the scaffold's real, it just hasn't been pointed at a live model yet |
| Real backends, CRUD, sandboxing | A `Track B` codegen gap for `db`/`json`/`http`/`mq`/`sandbox` — none of them run today in any form |
| Docs / DX | Error-message clarity, Getting Started walkthroughs, missing examples |

[`AREAS.md`](./AREAS.md) lists who owns which subsystem; a cross-cutting
or breaking change goes through the [RFC process](./rfcs/README.md)
first — see [`GOVERNANCE.md`](./GOVERNANCE.md) and
[`CONTRIBUTING.md`](./CONTRIBUTING.md).

## 📚 Documentation lives in the wiki

This README is the pitch and the five-minute quick start. Everything
else — the full design philosophy, the compiler architecture, the
complete feature and grammar reference, benchmarks with methodology, and
the LLM-integration mechanism with evidence — lives in the
**[Nirdosha Wiki](https://github.com/kannamma-labs/nirdosha/wiki)**:

- [Design Philosophy](https://github.com/kannamma-labs/nirdosha/wiki/Design-Philosophy) — the twelve requirements, and the Rice's-theorem constraint that shapes everything
- [Who It's For](https://github.com/kannamma-labs/nirdosha/wiki/Who-Its-For) — the honest fit
- [Nirdosha vs. Rust, Go, Mojo](https://github.com/kannamma-labs/nirdosha/wiki/Nirdosha-vs-Alternatives)
- [Architecture](https://github.com/kannamma-labs/nirdosha/wiki/Architecture) — the real compiler pipeline, the LL(1) grammar, independent cross-checks
- [Language Features](https://github.com/kannamma-labs/nirdosha/wiki/Language-Features) — the full feature set
- [The UI Engine](https://github.com/kannamma-labs/nirdosha/wiki/UI-Engine) — zero-syntax CRUD/dashboard generation
- [Benchmarks](https://github.com/kannamma-labs/nirdosha/wiki/Benchmarks) — compiled-vs-compiled numbers, methodology and caveats included
- [**LLM Integration**](https://github.com/kannamma-labs/nirdosha/wiki/LLM-Integration) — the flagship page: what each mechanism solves for an agent, and the evidence it's real
- [Getting Started](https://github.com/kannamma-labs/nirdosha/wiki/Getting-Started) — full install/build/run/scaffold
- [Honest Scope & Roadmap](https://github.com/kannamma-labs/nirdosha/wiki/Honest-Scope-and-Roadmap) — shipped vs. interpreter-only vs. next
- [FAQ](https://github.com/kannamma-labs/nirdosha/wiki/FAQ)

## FAQ (short version)

**Is it production-ready?** No — it's pre-1.0 and moving fast. The
compiled path covers a real, growing subset (see
[what's shipped](#whats-shipped) above); most backend-service
capabilities (`db`/`json`/`http`/`mq`/`transact`/`workflow`) don't run
in any form right now. See the full
[Public Roadmap](./docs/PUBLIC_ROADMAP.md) for what's next.

**Why not just use Rust?** Rust already solves memory safety for teams
that can invest in its learning curve. Nirdosha targets a narrower
problem — AI agents writing backend code unsupervised. See the
[full answer](https://github.com/kannamma-labs/nirdosha/wiki/Nirdosha-vs-Alternatives).

**Found a bug?** Open a GitHub issue with the `nirdosha build`/
`emit-llvm` error message and the `.nir` source that produced it.
Security issue? See [SECURITY.md](./SECURITY.md) instead.

**Want to contribute?** See [CONTRIBUTING.md](./CONTRIBUTING.md). More in
the [full FAQ](https://github.com/kannamma-labs/nirdosha/wiki/FAQ).

---

*निर्दोष — designed so that what the compiler accepts is, provably, without
fault.*
