# Nirdosha — निर्दोष ("without fault")

[![build](https://github.com/arunsoman/nirdosha/actions/workflows/build.yml/badge.svg)](https://github.com/arunsoman/nirdosha/actions/workflows/build.yml)
[![license: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE)
[![Wiki](https://img.shields.io/badge/docs-wiki-blue)](https://github.com/arunsoman/nirdosha/wiki)
[![Contributing](https://img.shields.io/badge/CONTRIBUTING-read-blue)](./CONTRIBUTING.md)
[![Governance](https://img.shields.io/badge/GOVERNANCE-read-blue)](./GOVERNANCE.md)
[![Roadmap](https://img.shields.io/badge/ROADMAP-view-purple)](./docs/PUBLIC_ROADMAP.md)
[![Maintainers](https://img.shields.io/badge/maintainers-5-green)](./MAINTAINERS.md)
[![Sponsor](https://img.shields.io/badge/%E2%9D%A4-Sponsor-ea4aaa)](https://github.com/sponsors/arunsoman)

> **A systems language designed for LLMs to write, with a grammar so
> constrained the model can't emit invalid syntax.** No garbage collector,
> no data races, no deadlocks, no integer/buffer overflow — those aren't
> the pitch, they're the proof that a language built for an AI agent to
> write unsupervised can also be trusted to run.

Status: a real, runnable Rust compiler (`crates/compiler/`) under
active development — many safety properties are *proven* today, and
where one is still *aspirational* the [wiki](https://github.com/arunsoman/nirdosha/wiki/Honest-Scope-and-Roadmap)
says so plainly. Source files use the `.nir` extension.

**One function. Four independent guarantees the compiler itself checks
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
    nfr(latency_ms: 50, concurrency_max: 1000)   // real APM tracking, zero code at the call site
{
    return Employee(name.value, department.value, salary)
}
```

```sh
cargo run -p nirdosha --release -- build examples/features/50_field_masking_and_check_role.nir -o employee && ./employee
# 10          <- a Z3-proven Hoare contract elsewhere in the same file (see below)
# 150000.000000
# 0.000000    <- masked
# Ada Lovelace
```

`caller: RoleView` is the *only* thing standing between a request and a
real salary — and a `RoleView` cannot be forged (`RoleView("admin")` is a
compile-time error) or fabricated by this function's own logic; it can
only come from `check_role(identity, "admin")` succeeding against a real
`VerifiedIdentity`, itself compiled, not interpreted. Call
`get_employee` with a `RoleView` that proves `"admin"` and `salary`
passes through untouched; call it with any other proof — or none — and
`salary` comes back `0.0`, unconditionally, while every other field
still passes through. No `if` in `get_employee`'s own body decides
that — the masking is a property of the field declaration, checked at
every `return`, so there's no per-handler `if user.role == "admin"`
check to forget writing, the way a hand-rolled equivalent scattered
across every handler that touches `Employee` always eventually gets
forgotten in one of them.

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
naming a real counterexample. Full runnable source, plus what each
guarantee doesn't (yet) cover:
[`examples/features/50_field_masking_and_check_role.nir`](./examples/features/50_field_masking_and_check_role.nir).

![Historical, pre-2026-09 evidence — captured before this repo's interpreter (`run`/`serve`) was removed. 334 lines of Nirdosha producing a themed dashboard with live SQLite data, a sortable/searchable vendor table, and a role-gated payout-approval action — then the same screens under a lower-privileged identity, with a field dropped and an action disabled by the server](./demo.gif)

*Historical evidence, kept for the record, not a claim about what runs
today: this GIF predates the interpreter removal described in "Current
focus" below — the `nirdosha serve`/`examples/vendor_ops.nir` it shows
no longer exist in this tree, and nothing currently replaces the *live*,
DB-backed half of it. What the hero example above demonstrates instead
is real and current: field-level masking, driven by a real `RoleView`,
enforced in the compiled binary itself, no server process involved. The
UI-generation half that survives today is `nirdosha emit-ui` (static
HTML derived from `struct`/`screen` conventions, no live backend) — see
[Try it](#try-it-in-under-a-minute) below, and the
[UI Engine](https://github.com/arunsoman/nirdosha/wiki/UI-Engine) wiki
page for the full, current picture.

---

## Current focus / how to help

Small team, high-context contributions matter more than volume — see
[`MAINTAINERS.md`](./MAINTAINERS.md) for who has write access and how
active each is. Right now:

- **Track B (full compilation) — now the only path to running any of
  this, not a latency/throughput nice-to-have.** The interpreter (`run`/
  `serve`) has been removed entirely, by deliberate decision, so that
  the compiled path can't quietly stay second-class. Native codegen only
  covers the numeric/control-flow/`tcp`/`file` subset today —
  `db`/`json`/`http`/`mq`/identity/`transact`/concurrency don't run at
  all right now, in any form, until each gets real codegen (see
  [ROADMAP](./docs/PUBLIC_ROADMAP.md)). First gap to close: `transact` →
  `db`/`json`.
- **macOS verification** — release binaries link system Z3 because
  `z3-src` doesn't build against current AppleClang ([ADR
  0001](./docs/adr/0001-vendor-z3-except-macos.md), tracked as
  [issue #5](https://github.com/arunsoman/nirdosha/issues/5)) — that part
  is still open. What's now closed: macOS gets CI verification on every
  push/PR, not just at release time. `build-macos` in
  [`.github/workflows/build.yml`](./.github/workflows/build.yml) mirrors
  release.yml's proven `brew install z3` + system-Z3 build on a real
  `macos-14` runner. Windows is CI-verified too: `build-windows` in the
  same file builds and runs the `tcp` suite plus the compiled-native-
  codegen TCP tests on a real `windows-latest` runner on every push/PR
  (see [ROADMAP A7](./docs/PUBLIC_ROADMAP.md)).

Full list with status tags: [`docs/PUBLIC_ROADMAP.md`](./docs/PUBLIC_ROADMAP.md).
Issues are labeled `good first issue` / `help wanted` / `compiler` /
`llm` / `infra` / `documentation` (full set: [`.github/labels.yml`](./.github/labels.yml)).
Pick one, comment before starting on anything non-trivial — see
[`CONTRIBUTING.md`](./CONTRIBUTING.md). [`AREAS.md`](./AREAS.md) lists
who owns which subsystem; a cross-cutting or breaking change goes
through the [RFC process](./rfcs/README.md) first — see
[`GOVERNANCE.md`](./GOVERNANCE.md). Your first issue or PR here won't
land in silence: [`welcome.yml`](./.github/workflows/welcome.yml) posts
a real, specific reply (not boilerplate) and
[`label-first-contribution.yml`](./.github/workflows/label-first-contribution.yml)
tags it `first-time contributor` so it's visible at a glance — it
doesn't solve cold-start on its own, but it's a real signal in place of
nothing.

| If you care about | Try | 
|---|---|
| Ownership/concurrency, PL theory | A `Track B` codegen gap, or an SMT/typeck edge case |
| Constrained decoding, agent repair loops | `crates/bench/` harness + real models, `crates/grammar_export/` corpus entries |
| Real backends, CRUD, sandboxing | A new `examples/*.nir` service, or Track A production-readiness items |
| Docs / DX | Error-message clarity, Getting Started walkthroughs, missing examples |

## Why this exists, in one paragraph

Nirdosha targets one specific, currently-unsolved problem: **a backend
service written and maintained by an AI coding agent, with no human
reviewing every line before it runs.** An LL(1) grammar exported to GBNF
lets a sampler force every token an agent emits to stay syntactically
valid; `--format=json` gives a self-repair loop a structured proof
obligation instead of a paragraph to guess at; `sandbox` is a real OS
process and a language primitive, not a bolted-on Docker wrapper; there is
no mutex in the language, so an agent literally cannot generate a
lock-ordering deadlock — and the one deadlock class that *is* still
expressible (every thread mutually blocked on `chan`/`thread`, with no
lock involved at all) is caught at runtime and aborted with a diagnostic
naming the stuck handles, not left to hang forever. It isn't trying to
be a better Rust — see the
[wiki](https://github.com/arunsoman/nirdosha/wiki) for the full case,
including where the design is still evolving, not a finished product.

## Who this is for — and who it isn't

**This is for you if:**
- You're building an agent (or agent framework) that writes and runs
  backend code with no human reviewing it before it executes, and you
  need the *language* to make bug classes unrepresentable rather than
  catching them in review.
- You want to try the constrained-decoding mechanism on a real compiler
  today, not a whitepaper — the GBNF grammar and `emit-ast`'s structured
  diagnostics run now, against the compiled path.
- You're fine filing issues against a fast-moving pre-1.0 project, not
  pulling a finished 1.0 into a production stack.

**This isn't for you if:**
- You want a general-purpose systems language for humans to write —
  that's Rust, and Rust is the honest answer (see the FAQ below).
- You need `db`/`json`/`http`/`mq` at all — there is no interpreter
  fallback anymore, and none of these compile to native code yet either,
  so none of it runs in any form until Track B lands (see
  [ROADMAP](./docs/PUBLIC_ROADMAP.md)). Basic concurrency (`spawn`/
  `join`/`thread`, `chan`/`send`/`recv`) does compile now — `sandbox`
  still doesn't.
- You need something production-ready this quarter — nothing here
  claims that.

Full picture: [Who It's For](https://github.com/arunsoman/nirdosha/wiki/Who-Its-For) in the wiki.

## Nirdosha vs. Rust, Go, Mojo — the one-line version

| | **Nirdosha** | **Rust** | **Go** | **Mojo** |
|---|---|---|---|---|
| Target use case | LLM-written backend services, compliance CRUD | General-purpose systems | Cloud-native services | AI/ML-first, Python-compatible |
| Data-race freedom | Static | Static | Dynamic only | Not yet fully guaranteed |
| Deadlock freedom | No mutex primitive exists at all (lock-order deadlocks unrepresentable); a `chan`/`thread` global stall is dynamically detected and aborted, not left to hang | Possible | Possible | Not a current guarantee |
| LLM writability | LL(1) grammar exported to GBNF for constrained decoding | LLMs default to Python 90–97% of the time | No constrained decoding built in | No published GBNF integration |

Full comparison, plus the honest "why not just use Rust" answer, in the
[wiki](https://github.com/arunsoman/nirdosha/wiki/Nirdosha-vs-Alternatives).

## Try it in under a minute

**Don't want to learn the syntax first?** Paste
[`agent-skills/nirdosha/paste-anywhere-prompt.md`](./agent-skills/nirdosha/paste-anywhere-prompt.md)
into any LLM chat and describe what you want in plain English — it writes
the `.nir` code for you. This prompt has already been used, unmodified, to
generate a working e-commerce store, a food-delivery platform, a telecom
revenue-assurance system, and an online trading platform, each hundreds of
lines, each by an LLM with no prior Nirdosha exposure. See
[LLM Integration](https://github.com/arunsoman/nirdosha/wiki/LLM-Integration)
for the full mechanism and evidence.

**Install and run it yourself — no compiler needed, prebuilt binaries are
published on every [release](https://github.com/arunsoman/nirdosha/releases):**

```sh
git clone https://github.com/arunsoman/nirdosha.git && cd nirdosha

# macOS / Linux — installer script, auto-detects your platform
curl --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/arunsoman/nirdosha/main/scripts/install.sh | sh

nirdosha build examples/syntax/hello_nir.nir -o hello && ./hello
nirdosha emit-ui examples/features/39_screen_ui.nir -o ui.html   # static UI derived from a struct/screen block
```

Prefer not to pipe a script into `sh`? Download the binary straight from
the release instead — `.../releases/latest/download/<asset>` always
resolves to the newest release, so this stays correct with no version
number to update. Run this from inside the `nirdosha` clone from above:

```sh
# Linux x86_64
curl -fsSL https://github.com/arunsoman/nirdosha/releases/latest/download/nirdosha-x86_64-unknown-linux-gnu.tar.gz | tar xz

# macOS, Apple Silicon
curl -fsSL https://github.com/arunsoman/nirdosha/releases/latest/download/nirdosha-aarch64-apple-darwin.tar.gz | tar xz

./nirdosha build examples/syntax/hello_nir.nir -o hello && ./hello
```

These two targets are what's currently published; Windows and Intel
Mac binaries aren't up yet (build from source below in the meantime) —
the [releases page](https://github.com/arunsoman/nirdosha/releases/latest)
has the current full asset list.

Full install (Windows, building from source, toolchain requirements),
scaffolding a new project, and generating a UI: see
[Getting Started](https://github.com/arunsoman/nirdosha/wiki/Getting-Started)
in the wiki.

### Before you write your own program

The example above runs as-is, but these four things will trip up your
*first original line* — they're parse/type errors, not style nits:

- **Enum variants are calls, always with `()`.** `Some(5)`, `None()`,
  `Circle(r)` — a zero-payload variant still needs the parens.
  `Color::Red` also works (optional disambiguation sugar), but a bare
  variant name never takes the place of a call.
- **`str` can't be a function's parameter or return type.** Use an
  `enum` for categorical data, or `struct Text { value: str }` to pass
  free text.
- **No string concatenation or formatting.** A `str` value only ever
  comes from a source literal or a builtin (`json_get_str`,
  `db_query`, ...) — there's no `+` or format string to build one at
  runtime.
- **No statement separators.** The parser always extends the current
  expression across a newline — `return x` then `-y` on the next line
  parses as `return (x - y)`, not two statements.

Full rationale and the complete list: [`AGENTS.md`](./AGENTS.md).

## 📚 Documentation lives in the wiki

This README is the pitch and the five-minute quick start. Everything
else — the full design philosophy, the compiler architecture, the
complete feature and grammar reference, benchmarks with methodology, and
the LLM-integration mechanism with evidence — lives in the
**[Nirdosha Wiki](https://github.com/arunsoman/nirdosha/wiki)**:

- [Design Philosophy](https://github.com/arunsoman/nirdosha/wiki/Design-Philosophy) — the twelve requirements, and the Rice's-theorem constraint that shapes everything
- [Who It's For](https://github.com/arunsoman/nirdosha/wiki/Who-Its-For) — the honest fit
- [Nirdosha vs. Rust, Go, Mojo](https://github.com/arunsoman/nirdosha/wiki/Nirdosha-vs-Alternatives)
- [Architecture](https://github.com/arunsoman/nirdosha/wiki/Architecture) — the real compiler pipeline, the LL(1) grammar, independent cross-checks
- [Language Features](https://github.com/arunsoman/nirdosha/wiki/Language-Features) — the full feature set
- [The UI Engine](https://github.com/arunsoman/nirdosha/wiki/UI-Engine) — zero-syntax CRUD/dashboard generation
- [Benchmarks](https://github.com/arunsoman/nirdosha/wiki/Benchmarks) — compiled-vs-compiled numbers, methodology and caveats included
- [**LLM Integration**](https://github.com/arunsoman/nirdosha/wiki/LLM-Integration) — the flagship page: what each mechanism solves for an agent, and the evidence it's real
- [Getting Started](https://github.com/arunsoman/nirdosha/wiki/Getting-Started) — full install/build/run/scaffold
- [Honest Scope & Roadmap](https://github.com/arunsoman/nirdosha/wiki/Honest-Scope-and-Roadmap) — shipped vs. interpreter-only vs. aspirational
- [FAQ](https://github.com/arunsoman/nirdosha/wiki/FAQ)

## FAQ (short version)

**Is it production-ready?** Not yet — it's under active development,
with real guarantees proven today and the rest tracked openly, not
hand-waved. See [Honest Scope & Roadmap](https://github.com/arunsoman/nirdosha/wiki/Honest-Scope-and-Roadmap).

**Why not just use Rust?** Rust already solves memory safety for teams
that can invest in its learning curve. Nirdosha targets a narrower
problem — AI agents writing backend code unsupervised. See the
[full answer](https://github.com/arunsoman/nirdosha/wiki/Nirdosha-vs-Alternatives).

**Found a bug?** Run `nirdosha <file.nir> --format=json` and paste the
`Diagnostic` JSON into a GitHub issue. Security issue? See
[SECURITY.md](./SECURITY.md) instead.

**Want to contribute?** See [CONTRIBUTING.md](./CONTRIBUTING.md). More in
the [full FAQ](https://github.com/arunsoman/nirdosha/wiki/FAQ).

---

*निर्दोष — designed so that what the compiler accepts is, provably, without
fault.*
