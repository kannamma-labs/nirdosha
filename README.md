# Nirdosha — निर्दोष

<p align="center">
<b>A systems language your AI agent can write — that humans can trust.</b><br/>
No GC. No data races. No deadlocks. No buffer overflow. <i>Proven at build time, not promised.</i>
</p>

<p align="center">
<a href="https://github.com/kannamma-labs/nirdosha/actions/workflows/build.yml"><img src="https://github.com/kannamma-labs/nirdosha/actions/workflows/build.yml/badge.svg" alt="build"/></a>
<a href="./LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="license: MIT"/></a>
<a href="https://github.com/kannamma-labs/nirdosha/wiki"><img src="https://img.shields.io/badge/docs-wiki-blue" alt="Wiki"/></a>
<a href="./docs/PUBLIC_ROADMAP.md"><img src="https://img.shields.io/badge/ROADMAP-view-purple" alt="Roadmap"/></a>
<a href="./MAINTAINERS.md"><img src="https://img.shields.io/badge/maintainers-5-green" alt="Maintainers"/></a>
<a href="https://github.com/sponsors/arunsoman"><img src="https://img.shields.io/badge/%E2%9D%A4-Sponsor-ea4aaa" alt="Sponsor"/></a>
</p>

---

**Existing languages assume a human author.** Nirdosha assumes the author might be an LLM — and constrains the language so entire classes of bugs are *unexpressible*, not just discouraged. The same constraints give humans unusually strong static guarantees for high-assurance backend code.

> 💡 **Why this works at all: [A Language Is Only as Good as Its Ecosystem](https://github.com/kannamma-labs/nirdosha/wiki/A-Language-Is-Only-as-Good-as-Its-Ecosystem)** — Nirdosha's capability ceiling is Rust's, because every capability underneath is an ordinary Rust crate.

## See it

![A themed dashboard with live SQLite data, a sortable/searchable table, and a role-gated approval action — the same screen under a lower-privileged identity, with a field dropped and an action disabled](./demo.gif)

## Try it in 30 seconds

1. **🍴 [Fork this repo](https://github.com/kannamma-labs/nirdosha/fork)** (top-right corner — takes 5 seconds, no local setup needed)
2. **[Open in GitHub Codespaces](https://codespaces.new/kannamma-labs/nirdosha?quickstart=1)** — the fork builds in your browser in about a minute
3. Run it:

```sh
cargo run -p nirdosha --release -- build examples/syntax/hello_nir.nir -o hello && ./hello
```

Prefer installing a binary? Linux, Windows, and Apple Silicon macOS builds ship on every [release](https://github.com/kannamma-labs/nirdosha/releases). No compiler needed.

---

<details>
<summary><b>📄 One function, five guarantees the compiler itself checks</b></summary>

Not comments. Not conventions. Not runtime middleware — the build fails if any of these is violated:

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

**Two independent gates, not one mechanism wearing two hats.** `requires(role: "hr_staff")` on the function decides *who can call it at all* — `get_employee`'s name has no direct-call path once gated; the only way to obtain a callable value is `acquire get_employee(proof)`, and it demands a real `RoleView` proving `"hr_staff"`. `requires(role: "admin")` on the field separately decides *what a successful caller sees back* — `salary` masks itself to zero on every `return` unless the value passed as `caller` proves `"admin"` specifically.

Neither `RoleView` can be forged (`RoleView("admin")` is a compile-time error) or fabricated by this function's own logic — the only way to get a real one is `check_role(identity, role)` succeeding against a real `VerifiedIdentity`, itself compiled, not interpreted.

The result: an HR staffer who isn't also an admin can call the function and get real employee records back with salary redacted; someone who's neither can't obtain a callable `get_employee` in the first place. Two different failure points, two different roles, checked independently, with no `if` anywhere in `get_employee`'s own body deciding either one.

`effect(pure)` is checked against what the function *actually does* — annotate a function that performs I/O as `pure` and `nirdosha build` rejects it, naming the real effect it found. `nfr(latency_ms: 50, concurrency_max: 1000)` wires real per-call tracking into the APM kernel with zero code at any call site, and escalates to `NIRDOSHA_OBSERVABILITY_URL` automatically if one is configured. The full file also carries a `validate` block with a genuine Z3-proven post-condition — `tenure_bonus_pct`'s `result` provably stays in `[0, 20]` for *every* possible input, not just the ones a test tries. Flip that bound to something false and `nirdosha build` fails outright, naming a real counterexample. Full runnable source: [`examples/features/50_field_masking_and_check_role.nir`](./examples/features/50_field_masking_and_check_role.nir).

</details>

<details>
<summary><b>🖥️ From two <code>struct</code>s to a generated UI — and past the "four chart shapes" wall</b></summary>

Every closed UI vocabulary eventually runs into the same wall: your dashboard needs a scatter plot, a gauge, a funnel — and the usual answers are fork the tool, or give up and let something emit raw markup (quietly reopening the exact XSS surface a closed vocabulary was buying you out of).

Nirdosha's third option: `render: "chart"` turns "pick one of four fixed shapes" into a small, composable config — a mark crossed with encoding channels — staying exactly as closed and typechecked as the shapes it sits alongside:

```nirdosha
dashboard {
    visual "Revenue by month" -> chart_revenue_by_month {
        render: "chart"
        mark: "bar"
        encode x { field: "month" type: "temporal" }
        encode y { field: "amount" type: "quantitative" aggregate: "sum" }
    }
}
```

Need a genuinely custom widget? Add a Rust crate to your project's `Cargo.toml` — `nirdosha emit-ui` finds and links it automatically:

```nirdosha
layout {
    sparkline { source: recent_sales_totals field: "amount" }
}
```

Both are additive — every existing `bar_chart`/`graph`/`heatmap`/`timeline`/`divider`/`card` program keeps working exactly as it did.

**Why this door doesn't undo agent safety:** other generative-UI specs (OpenUI, Vercel's json-render, Google's A2UI) resolve their component catalog *inside the running app process* — editable by the same process that's talking to the agent. Here, your catalog is fully resolved and typechecked at `nirdosha build`/`emit-ui` time, before the binary the agent talks to exists. No admin API, no hot-reload, no session negotiation. Growing the vocabulary is something *you* do once, by adding a Cargo dependency you reviewed — never something a chat turn can do. Full argument: [`rfcs/0009`](./rfcs/0009-ui-catalog-extensibility.md) §"Effect on the permission model".
</details>

<details>
<summary><b>🤖 Why this exists — the one-paragraph version</b></summary>

Nirdosha targets one specific problem: **a backend service written and maintained by an AI coding agent, with no human reviewing every line before it runs.** An LL(1) grammar exported to GBNF lets a sampler force every token an agent emits to stay syntactically valid; `nirdosha emit-ast` gives a self-repair loop a structured, typed AST to work from instead of a paragraph to guess at; and there is no mutex in the language at all, so an agent literally cannot generate a lock-ordering deadlock — a guarantee about what the *language* can express, true regardless of which constructs are compiled yet. It isn't trying to be a better Rust — see the [wiki](https://github.com/kannamma-labs/nirdosha/wiki) for the full case.

**Two ways in:** don't write code? Paste [`agent-skills/nirdosha/paste-anywhere-prompt.md`](./agent-skills/nirdosha/paste-anywhere-prompt.md) into any LLM chat (ChatGPT, Claude, Gemini) — this exact prompt has produced a working e-commerce store, a food-delivery platform, a telecom revenue-assurance system, and an online trading platform, each from an LLM with zero prior Nirdosha exposure. Do write code? [`examples/syntax/`](./examples/syntax/) walks `hello_nir.nir` → multi-module enterprise app; [`examples/features/`](./examples/features/) is the full reference (50 files, one per feature) — `docs/LANGUAGE.md` §10 says exactly which compile today.
</details>

<details>
<summary><b>📊 Nirdosha vs. Rust, Go, Mojo</b></summary>

|  | **Nirdosha** | **Rust** | **Go** | **Mojo** |
| --- | --- | --- | --- | --- |
| Target use case | LLM-written backend services, compliance CRUD | General-purpose systems | Cloud-native services | AI/ML-first, Python-compatible |
| Data-race freedom | Static | Static | Dynamic only | Not yet fully guaranteed |
| Deadlock freedom | No mutex primitive exists at all | Possible | Possible | Not a current guarantee |
| LLM writability | LL(1) grammar exported to GBNF for constrained decoding | LLMs default to Python 90–97% of the time | No constrained decoding built in | No published GBNF integration |

Full comparison in the [wiki](https://github.com/kannamma-labs/nirdosha/wiki/Nirdosha-vs-Alternatives).
</details>

<details>
<summary><b>📦 Install (binaries, from source, or Codespaces)</b></summary>

Prebuilt binaries for **Linux, Windows, and Apple Silicon macOS** on every [release](https://github.com/kannamma-labs/nirdosha/releases):

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

Prefer not to pipe a script? `.../releases/latest/download/<asset>` always resolves to the newest release:

```sh
# Linux x86_64
curl -fsSL https://github.com/kannamma-labs/nirdosha/releases/latest/download/nirdosha-x86_64-unknown-linux-gnu.tar.gz | tar xz

# macOS, Apple Silicon
curl -fsSL https://github.com/kannamma-labs/nirdosha/releases/latest/download/nirdosha-aarch64-apple-darwin.tar.gz | tar xz

./nirdosha build examples/syntax/hello_nir.nir -o hello && ./hello
```

(Intel Mac: build from source for now — see below.)

Building from source needs `clang` and `z3` (`apt install clang libz3-dev` / `brew install llvm z3` / `pacman -S clang z3`). Full matrix, scaffolding, and UI generation: [Getting Started](https://github.com/kannamma-labs/nirdosha/wiki/Getting-Started).

**Four things that will trip up your *first original line*** — parse/type errors, not style nits:

- **Enum variants are calls, always with `()`.** `Some(5)`, `None()`, `Circle(r)` — a zero-payload variant still needs the parens. `Color::Red` also works, but a bare variant name never takes the place of a call.
- **`str` can't be a function's parameter or return type.** Use an `enum` for categorical data, or `struct Text { value: str }` to pass free text.
- **No string concatenation or formatting.** A `str` value only ever comes from a source literal or a builtin.
- **No statement separators.** `return x` then `-y` on the next line parses as `return (x - y)`, not two statements.

Full rationale: [`AGENTS.md`](./AGENTS.md).
</details>

<details>
<summary><b>✅ What's shipped vs. what's designed</b></summary>

**Real, compiled, and running today** — the highlights:

- **Language core** — LL(1) grammar cross-verified against an independent LALR(1) generator, a static type checker, ownership/affine types (`box`/`&`/`froze`, no GC, no manual `free`), `spawn`/`thread`/`chan` with no mutex in the language, generics, `Option`/`Result`, SMT-backed (Z3) integer/buffer-overflow proofs, and `validate { pre:/post: }` Hoare contracts (Z3-proven at build time for Tier-1 integer functions).
- **Native codegen** — LLVM `-O2` compilation for the numeric/control-flow/`box`/`froze`/`str`/`tcp`/`file`/concurrency subset, within 1.4× of `gcc -O2` on the operations it covers.
- **Identity and data protection** — `check_role` against a real `VerifiedIdentity`, producing an unforgeable `RoleView`; field-level `requires(role/claim: ...)` masking; function-level `requires(role/claim: ...)` + `acquire` gating callability — all three compiled, all enforced in the binary itself, no server process involved.
- **`nfr(...)`** — non-functional requirements as a first-class, compiled fn annotation: automatic latency/error-rate/throughput/concurrency tracking via the APM kernel, with async escalation to an observability endpoint on a crossed threshold.
- **UI engine (static)** — `nirdosha emit-ui` derives a Material-styled page from `struct`/`screen`/`dashboard` conventions; `render: "chart"` adds a bounded grammar-of-graphics config, and a Rust crate can contribute a custom `layout` widget — hand-assembled or auto-discovered from the app's own `Cargo.toml` — with no runtime code-execution hole ([`rfcs/0009`](./rfcs/0009-ui-catalog-extensibility.md)).
- **Cross-platform CI** — Linux, macOS, and Windows all build and run their full test suite on every push, not just at release time.

**Not currently running in any form** — designed, documented, and (for most of these) previously interpreter-backed, but the interpreter is removed and native codegen doesn't reach them yet: `db`, `json`, `http`/`https`, `mq`, `transact`, `workflow`, `sandbox`, and the live (server-backed) half of the UI engine. This is the deliberate, disclosed trade the interpreter removal made — see [Track B](./docs/PUBLIC_ROADMAP.md) for what's landed since and what's next; treat the roadmap's older entries with the same caution, since large parts of it still describe the pre-removal world.
</details>

<details>
<summary><b>🙋 How to help</b></summary>

Small team — high-context contributions matter more than volume. See [`MAINTAINERS.md`](./MAINTAINERS.md) for who has write access. Issues are labeled `good first issue` / `help wanted` / `compiler` / `llm` / `infra` / `documentation` (full set: [`.github/labels.yml`](./.github/labels.yml)) — `good first issue` tickets don't need a "may I?" comment first, just send the PR. Your first issue or PR won't land in silence: [`welcome.yml`](./.github/workflows/welcome.yml) posts a real, specific reply.

| If you care about | Try |
| --- | --- |
| Ownership/concurrency, PL theory | A `Track B` codegen gap, or an SMT/typeck edge case |
| Constrained decoding, agent repair loops | `crates/bench/`'s pass@1 harness — the scaffold's real, it just hasn't been pointed at a live model yet |
| Real backends, CRUD, sandboxing | A `Track B` codegen gap for `db`/`json`/`http`/`mq`/`sandbox` — none of them run today in any form |
| Docs / DX | Error-message clarity, Getting Started walkthroughs, missing examples |

[`AREAS.md`](./AREAS.md) lists subsystem owners; cross-cutting or breaking changes go through the [RFC process](./rfcs/README.md) first — see [`GOVERNANCE.md`](./GOVERNANCE.md) and [`CONTRIBUTING.md`](./CONTRIBUTING.md).
</details>

<details>
<summary><b>📚 Documentation lives in the wiki</b></summary>

This README is the pitch and the five-minute quick start. Everything else lives in the **[Nirdosha Wiki](https://github.com/kannamma-labs/nirdosha/wiki)**:

- [Design Philosophy](https://github.com/kannamma-labs/nirdosha/wiki/Design-Philosophy) — the twelve requirements, and the Rice's-theorem constraint that shapes everything
- [Who It's For](https://github.com/kannamma-labs/nirdosha/wiki/Who-Its-For) — the honest fit
- [Nirdosha vs. Rust, Go, Mojo](https://github.com/kannamma-labs/nirdosha/wiki/Nirdosha-vs-Alternatives)
- [Architecture](https://github.com/kannamma-labs/nirdosha/wiki/Architecture) — the real compiler pipeline, the LL(1) grammar, independent cross-checks
- [Language Features](https://github.com/kannamma-labs/nirdosha/wiki/Language-Features) — the full feature set
- [The UI Engine](https://github.com/kannamma-labs/nirdosha/wiki/UI-Engine) — zero-syntax CRUD/dashboard generation
- [Benchmarks](https://github.com/kannamma-labs/nirdosha/wiki/Benchmarks) — compiled-vs-compiled numbers, methodology and caveats included
- [**LLM Integration**](https://github.com/kannamma-labs/nirdosha/wiki/LLM-Integration) — the flagship page: what each mechanism solves for an agent, and the evidence it's real
- [Getting Started](https://github.com/kannamma-labs/nirdosha/wiki/Getting-Started) — full install/build/run/scaffold
- [Honest Scope & Roadmap](https://github.com/kannamma-labs/nirdosha/wiki/Honest-Scope-and-Roadmap) — shipped vs. next
- [FAQ](https://github.com/kannamma-labs/nirdosha/wiki/FAQ)
</details>

<details>
<summary><b>❓ FAQ (short version)</b></summary>

**Is it production-ready?** No — it's pre-1.0 and moving fast. The compiled path covers a real, growing subset; most backend-service capabilities (`db`/`json`/`http`/`mq`/`transact`/`workflow`) don't run in any form right now. See the [Public Roadmap](./docs/PUBLIC_ROADMAP.md).

**Why not just use Rust?** Rust already solves memory safety for teams that can invest in its learning curve. Nirdosha targets a narrower problem — AI agents writing backend code unsupervised. [Full answer](https://github.com/kannamma-labs/nirdosha/wiki/Nirdosha-vs-Alternatives).

**Found a bug?** Open an issue with the `nirdosha build`/`emit-llvm` error message and the `.nir` source. Security issue? See [SECURITY.md](./SECURITY.md).

**Want to contribute?** See [CONTRIBUTING.md](./CONTRIBUTING.md). More in the [full FAQ](https://github.com/kannamma-labs/nirdosha/wiki/FAQ).
</details>

---

<p align="center">
<i>निर्दोष — designed so that what the compiler accepts is, provably, without fault.</i>
</p>
