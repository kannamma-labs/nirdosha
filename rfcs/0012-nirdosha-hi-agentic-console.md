# RFC 0012: `nirdosha hi` — a native LLM console gated on provider credentials

> **Status: v1 `[DONE]`.** This RFC was originally drafted as
> `0010-nirdosha-hi-agentic-console.md` on an older, unmerged branch
> (`remove-interpreter`) before `0010` was taken on `main` by
> `0010-landing-and-serve-exposure.md` — renumbered to `0012` on merge,
> content otherwise unchanged from the original design below except
> where this status block and inline notes say a decision was actually
> made. **Shipped in v1:** the activation contract, a bespoke
> (not RFC-0011-call-shape) OpenAI-compatible client, NL-to-`.nir`
> generation with a bounded compiler-feedback self-repair loop, and an
> on-request diagnostic explainer (`crates/compiler/src/hi.rs`).
> **Still `[OPEN]`, deliberately not built in v1** (see "Console
> capability scope" below for the RFC's own priority ordering, and each
> item's line for why): model-driven dispatch to other subcommands,
> read-only Q&A over project files, read-only ingestion of other-
> language source, streaming responses, native Anthropic Messages API
> support. The "Provider-client fork point" section's own recommendation
> (reuse RFC 0011) turned out to hit a real, previously-unknown obstacle
> once actually attempted — see that section's added note.

## Motivation

`nirdosha` today is a hermetic, batch, compiled-only CLI: `init`,
`gen-crud`, `build`, `emit-llvm`, `emit-ast`, `emit-ui`
(`crates/compiler/src/main.rs`). It has no interactive mode — `run`/
`serve`/`--sandbox-worker` were deliberately removed along with the
tree-walking interpreter — and no notion of the tool itself calling an
LLM. Separately, the project already stakes a claim that nirdosha is a
language designed to be LLM-native (small, unambiguous grammar; GBNF-
constrained-decoding ambitions), and `docs/nirdosha-agent-api.md` Track C
("Agent-Facing API") is already roadmapped but, per `docs/ROADMAP.md`,
0% built.

`nirdosha hi` is proposed as the front door to that ambition: a
subcommand that, run with no other flags, checks whether it can find LLM
credentials and either welcomes the user into an agentic console or
states exactly which environment variables would activate it. This RFC
scopes the activation contract, the console's capability boundary, and
the protocol-level requirements a native LLM client actually implies —
it is not a full agent implementation, and several concrete pieces are
left as open questions rather than resolved here.

A prior, host-side LLM client existed once, for a different purpose:
`crates/bench/src/real_model.rs` (`reqwest::blocking`, targeting any
OpenAI-compatible `/chat/completions` endpoint, configured via
`NIRDOSHA_BENCH_API_KEY`/`_API_BASE`/`_MODEL`) benchmarked interpreted
execution. It was deleted along with the interpreter it benchmarked
(`05a747c refactor: remove tree-walking interpreter and its only
consumers`), but its wire-format choice and env-var-only configuration
are useful prior art, referenced throughout this RFC — and, v1 update:
recovered in full via `git show 05a747c^:crates/bench/src/real_model.rs`
and mirrored directly in `hi.rs`'s own client.

## Design

### CLI surface

A new arm in the existing hand-rolled dispatch — there is no clap/
structopt in this workspace, by explicit choice
(`crates/presence-gateway/src/main.rs:1-8`: "same 'no dependency this
repo doesn't already need elsewhere' posture") — and this RFC does not
propose introducing one:

```rust
// crates/compiler/src/main.rs
match first.as_str() {
    "init" => cmd_init(args),
    "gen-crud" => cmd_gen_crud(args),
    "build" => cmd_build(args),
    "emit-llvm" => cmd_emit_llvm(args),
    "emit-ast" => cmd_emit_ast(args),
    "emit-ui" => cmd_emit_ui(args),
    "emit-catalog" => cmd_emit_catalog(args),
    "hi" => cmd_hi(args),               // new
    other => { /* unknown subcommand */ }
}
```

`cmd_hi` follows the same per-command arg-loop style as `cmd_init`/
`cmd_build`, and `print_usage()` gains a usage line alongside the
others.

### Activation contract

**v1: implemented exactly as scoped below**, minus the "well-known
provider env var" step — v1 ships only the project's own explicit trio
plus `OPENAI_API_KEY` specifically (real OpenAI is OpenAI-compatible by
definition, so it needs no separate wire-format adapter); a broader
well-known-vendor list is left for a follow-up once it's clear which
vendors' users actually want the console's own OpenAI-compatible wire
format vs. a native adapter (Anthropic's real Messages API, notably,
is *not* OpenAI-compatible — deliberately out of scope for v1).

1. Check well-known provider environment variables in a fixed priority
   order. The exact list and ordering is an open question (below) — it
   needs to name real vendor env-var conventions (e.g.
   `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`) without this RFC committing to
   a specific frozen list that goes stale.
2. If none are set, check nirdosha's own explicit trio —
   `NIRDOSHA_LLM_PROVIDER`, `NIRDOSHA_LLM_PROVIDER_KEY`,
   `NIRDOSHA_LLM_PROVIDER_MODEL` — required together; a partial set is
   treated as "not configured," and the error names precisely which of
   the three is still missing rather than a generic failure. Naming
   follows the project's existing `NIRDOSHA_`-prefixed, `.ok()`-based
   convention (`crates/runtime-kernels/src/kernel/nfr.rs:183`,
   `recorder.rs:56`, `pool.rs:101`).
3. If neither resolves, print the exact three env vars needed and exit
   non-zero. No `.env` file support — deliberately, for consistency with
   RFC 0011's `env()` builtin, which made the same choice for compiled
   `.nir` programs.
4. If resolved, print a welcome banner ("Nirdosha Agentic Console") and
   enter the console loop.

### Console capability scope

Bounded deliberately, in priority order:

1. **NL → nirdosha generation** — the primary act. **`[DONE]`**
2. **Generate → `build` → feed diagnostic back → retry.** A self-repair
   loop, conceptually continuous with the deleted `crates/bench`
   pass@1/self-repair scaffold (`AREAS.md`'s "Real pass@1/self-repair
   scaffold, mock models only today" note) — but interactive instead of
   a benchmark harness, and driving the real compiler instead of mocks.
   **`[DONE]`, bounded at 3 attempts.**
3. **Explain a compiler diagnostic** in plain language, on request.
   **`[DONE]`** — `:explain`.
4. **Dispatch to existing subcommands** (`init`/`gen-crud`/`build`/etc.)
   chosen by the model — never arbitrary shell or file access. The model
   picks *which* nirdosha command to run and with what flags; it does not
   get a general execution surface. **`[OPEN]`** — deliberately deferred:
   this is a new host-level trust boundary (see "Effect on the
   permission model" below), and shipping it needs its own explicit
   decision, not a default that rides along with the rest of v1.
5. **Read-only Q&A** over the current project's own files. **`[OPEN]`**
6. **Read-only ingestion** of other-language source (Java, Node, …) as
   migration input only. The console never *emits* another language as
   an output target — see Rejected alternatives for why. **`[OPEN]`**

### Provider-client fork point

Two shapes were considered for how the LLM call itself is implemented,
and this RFC recommends one without treating the choice as closed:

- **RFC 0011 `call`-shape service provider** — reuse
  `PoolRegistry`/admission kernel (`kernel::acquire`/`release`, CAS,
  non-blocking) and the `env()` builtin, the same pattern
  `crates/plugin-example-native-authed-http` already demonstrates for
  injecting a bearer token into outbound HTTP. This is the extension
  point RFC 0011 was explicitly built for ("any future backend... via a
  native plugin without a compiler change"), and reusing it gets pooling,
  admission control, and reaper-driven connection rehydration "for free."
  **Recommended, but not what v1 actually built** — the original text
  flagged this as "currently blocked on sequencing" (RFC 0011 not yet on
  this branch); RFC 0011 has since landed on `main`, but attempting this
  path for real surfaced a second, previously-unknown blocker the
  original RFC didn't anticipate: `crates/runtime-kernels` (where
  `kernel::http`/`plugin_provider`/`PoolRegistry` live) is its own
  separate Cargo workspace (deadlock-avoidance reason, see its own
  `Cargo.toml`), while `crates/compiler` (this subcommand's home) is a
  member of the *root* workspace — the same "multiple workspace roots"
  error `docs/adr/0010-runtime-kernels-rlib-for-compiled-serve.md`
  already hit and solved for `compiled-serve` by moving it *into*
  `runtime-kernels`'s workspace. `crates/compiler` can't do the same
  without a larger restructuring. Real, separate follow-up work, not a
  v1 blocker.
- **Bespoke host-side client**, mirroring the deleted
  `crates/bench/src/real_model.rs` — a direct `reqwest`-based client with
  no provider abstraction. Simpler to ship immediately, independent of
  RFC 0011's landing state, but forgoes pooling/admission integration and
  sets a different precedent for how the *tool itself* (as opposed to
  generated programs) talks to external services. **What v1 actually
  ships** — `reqwest` (blocking, `rustls-tls`) was already a resolved
  workspace dependency via `crates/presence-gateway`, so this added zero
  new dependencies and zero cross-workspace plumbing.

### Protocol/transport requirements

These are named explicitly because "an LLM client" is not a single
uniform abstraction — it implies concrete transport and wire-format
commitments that don't yet exist anywhere in this codebase:

- **TLS/HTTPS client.** Cloud provider APIs (`api.anthropic.com`,
  `api.openai.com`, …) require TLS. **v1: `reqwest`'s `rustls-tls`
  feature** — confirmed working against a real `api.openai.com` request
  (a genuine structured 401 came back, not a TLS/connection failure).
  Separately, `crates/runtime-kernels/src/kernel/http.rs` also does real
  TLS (`native-tls`/vendored OpenSSL) for the *compiled `.nir` program's*
  own `https_get`/`https_post`, but that's a different client the
  console doesn't use — see "Provider-client fork point" above for why.
- **Streaming (SSE, `text/event-stream`).** Every major provider streams
  completions token-by-token this way. **v1: non-streaming only**, as
  recommended below.
- **Per-provider wire formats genuinely differ** — Anthropic's Messages
  API, OpenAI's Chat Completions, and Gemini's `generateContent` are
  different JSON shapes, not one schema with different URLs; auth
  conventions differ too (`x-api-key` vs. `Authorization: Bearer` vs.
  query param). **v1 scope: OpenAI-compatible only** (recommended v1
  scope-limiter, adopted as-is) — covers OpenAI, DeepSeek, and any
  OpenAI-compatible proxy/gateway, not native per-vendor adapters.
- **Rate limits/retries (429 backoff)** are an application-level concern
  the admission-kernel/pool model doesn't address today — that layer
  governs connection-count backpressure, not provider quota, and needs
  separate handling. **Still `[OPEN]`** — v1's client surfaces a 429 as
  an ordinary error, no automatic retry-with-backoff.

## Effect on the permission model

None, to the compiled `.nir` program's permission model —
`requires(role/claim: ...)`, `acquire`, a `screen`'s view/edit gates, and
`serve.rs`'s server-side enforcement are all unaffected; this feature is
host-tooling only and touches nothing a compiled program's runtime
checks.

Separately — not the same thing, but worth naming rather than omitting —
if the console is ever allowed to *act* on the user's behalf (write
files, invoke `build`/`init` without an explicit confirmation step),
that introduces a new *host-level* trust boundary. It has no bearing on
`.nir`'s permission model but is real scope this RFC flags without
resolving. **This is exactly why "dispatch to existing subcommands
chosen by the model" (capability 4 above) stayed `[OPEN]` in v1** —
shipping it needs its own explicit decision about what "an LLM can run
`nirdosha build`/`init` on your behalf without asking" actually means,
not a default that rides along with NL-generation.

## Compatibility

Purely additive. `hi` is a new subcommand name outside today's closed
set (`init`/`gen-crud`/`build`/`emit-llvm`/`emit-ast`/`emit-ui`/
`emit-catalog`) — no existing `.nir` program, build script, or CI
invocation of `nirdosha` changes behavior, because nothing currently
parses or depends on `hi` as anything other than an unrecognized-
subcommand error.

## Rejected alternatives

- **Multi-language code generation (Java, Node, etc.) as an output
  target.** Nirdosha's differentiation is a grammar small and
  unambiguous enough for GBNF-constrained decoding — a structural
  guarantee that the model *cannot* emit syntactically invalid nirdosha,
  which no general-purpose language's grammar supports in the same way.
  The self-repair loop's power similarly comes from verifying output
  against nirdosha's own compiler; that doesn't generalize to other
  toolchains without new dependencies (`javac`, `node`) the project
  doesn't otherwise carry, and without them the loop degrades to
  unverified generation — losing the one property that makes this
  console trustworthy rather than "another chatbot that writes code."
  Read-only ingestion of other languages (as migration input into
  nirdosha) is kept, since it avoids both problems and only adds
  adoption value.
- **Reintroducing a general interpreter/`run`/`serve` mode to support
  this.** The console is a distinct interactive surface — talking to an
  LLM — from interpreting `.nir` programs, and doesn't require undoing
  the prior, deliberate removal of that mode.
- **`.env` file support for credentials.** Rejected for consistency with
  RFC 0011's `env()` builtin, which made the same choice for the same
  reason (no implicit file-based config surface).

## Open questions

- Exact well-known-provider env var list and priority order, and how new
  providers get added over time without this RFC (or its successor)
  needing a rewrite each time a vendor ships a new key convention. **v1
  ships only `OPENAI_API_KEY` as a well-known fallback — the broader
  list stays open.**
- ~~RFC-0011 `call`-shape provider vs. bespoke client: which ships
  first~~ **Resolved: bespoke client shipped in v1** — see
  "Provider-client fork point" above.
- TLS client choice: reuse/extend the kernel's HTTP layer, or bring in a
  TLS-capable client crate. **Resolved: `reqwest`'s `rustls-tls`.**
- Streaming vs. non-streaming-only for v1, and whether that forces RFC
  0011's `call`-shape to grow a new variant. **Resolved: non-streaming
  only for v1.**
- Whether v1 scopes "well-known providers" to OpenAI-compatible
  endpoints only (as the deleted bench crate did) or commits to native
  per-vendor wire-format adapters immediately. **Resolved: OpenAI-
  compatible only.**
- Keeping secrets out of the flight recorder or any other logging
  (`crates/runtime-kernels/src/kernel/recorder.rs`) — no existing
  convention in the tool itself to reuse; this needs one. **Partially
  addressed: `hi::Activation` has a hand-written `Debug` impl that
  redacts the API key unconditionally, so an accidental `{:?}` can't
  leak it — no `kernel::recorder` integration exists in v1 at all, so
  there's nothing there yet to redact.**
- Testing/CI strategy without live provider keys — a mock-provider or
  recorded-fixture approach, echoing the deleted bench crate's "mock
  models only" posture, to keep CI hermetic. **Partially addressed: the
  activation contract (`resolve_activation`) is a pure, injected-env
  function with full unit-test coverage, no live keys needed. The LLM
  client itself has no CI-run test yet — only manually exercised against
  a real `api.openai.com` endpoint (confirmed a real TLS connection and
  well-formed request, via a live 401 with an invalid key).**
- Whether `nirdosha hi` stays a mode of the single `nirdosha` binary or
  eventually warrants a separate binary, mirroring how
  `presence-gateway` is already a second binary in this workspace.
  **Still open — v1 stays a subcommand of the existing binary.**
