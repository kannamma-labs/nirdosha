# RFCs

Design documents for decisions that are cross-cutting, breaking, or
shape the language surface / a public interface. No state machine, no
shepherd assignment, no formal gate — a document here is a design
capture at whatever stage it's actually at, not a claim that it's been
reviewed or approved by anyone. Read each file's own text to know how
settled (or not) it actually is; don't infer that from its presence in
this list.

## Documents

| # | Title |
|---|---|
| [0001](./0001-package-manifest-format.md) | Package manifest format (Cargo-based package manager) |
| [0002](./0002-editor-tooling-lsp-tree-sitter.md) | Editor/tooling ecosystem: tree-sitter grammar + minimal LSP |
| [0003](./0003-plugin-abi-v2.md) | Plugin ABI v2 — effect declarations, async/sync policy, versioning |
| [0004](./0004-native-plugin-sandboxing.md) | Trust model for native (Kind A) plugins |
| [0005](./0005-plugin-boundary-safety-and-performance.md) | The Nirdosha↔Rust plugin boundary — safety and performance |
| [0006](./0006-structured-concurrency.md) | Structured concurrency for native threads — Pillars 1-4 |
| [0007](./0007-apm-runtime-kernel.md) | A compiled-path resource-control kernel — boundary-leased admission, fail-open telemetry, and NFRs-as-language |
| [0008](./0008-native-plugin-abi-widening.md) | Native plugin ABI widening and Cargo-driven discovery — a compile-time-only path to a real plugin ecosystem |
| [0009](./0009-ui-catalog-extensibility.md) | Grammar-of-graphics charts and compile-time UI-plugin components — escaping the closed 4-chart/7-control catalog without opening a runtime hole. Track C (appended): the v2 Rust dialect's own UI engine — `dashboard!`/`crud_screens!`, nav bar + session login, built and proven |
| [0010](./0010-landing-and-serve-exposure.md) | Per-role/claim `landing` screens, and the compiled-`serve` route-exposure model (deny-by-default on mutating exposure) |
| [0011](./0011-uniform-service-provider-model.md) | A uniform service-provider model — open admission domains, revived plugin dispatch, and proactive rehydration for any future backend |
| [0012](./0012-nirdosha-hi-agentic-console.md) | `nirdosha hi` — a native LLM console gated on provider credentials |
| [0013](./0013-nirdosha-realm.md) | Nirdosha Realm — a local-first project knowledge graph with bidirectional `.nir` traceability |
| [0014](./0014-generative-build-console.md) | The generative build console — prompt → build → generate → publish, over an interactive Realm graph (speculative, nothing built) |
| [0015](./0015-keyed-guard-external-state.md) | `guard(keys) { ... }` — keyed mutual exclusion for multi-statement invariants over external (`db`+`http`) state (speculative, nothing built) |
| [0016](./0016-domain-packs-and-whose-job-domain-correctness-is.md) | Sealed domain plugins — pre-baked, non-waivable, cryptographically attested invariants (and compliance profiles) |
| [0017](./0017-security-guarantee-manifest.md) | Security guarantee manifests — per-module policy contracts checked at compile time and enforced by the APM kernel |
| [0018](./0018-llm-reflex-syntax-try-operator-string-quoting-or-patterns.md) | LLM-reflex syntax — a real `?` try-operator, single-quoted strings, and `\|` or-patterns in `match` (speculative, nothing built) |
| [0019](./0019-omniscope-unified-static-analysis-engine.md) | OMNISCOPE — a unified static analysis engine synthesizing mypy/Pyright/ESLint/SonarQube/Semgrep/CodeQL/Snyk/Coverity/Dafny/Prusti/Kani into one product-lattice kernel (speculative, nothing built) |
| [0020](./0020-v2-entity-policy-and-encryption-annotations.md) | Compile-time policy compliance (`policy!`/`crud(..)`), categorical field actions (`categorical_actions!`, Road 1 authoritative / Road 2 derived), an HTTP layer with OpenAPI on by default, and encryption requirements for the v2 Rust dialect — everything but encryption enforcement is built and proven |
| [0021](./0021-typed-hi-graph-mcp-and-incremental-authoring.md) | Typed Hi graph, project-scoped MCP reads/writes, resumable incremental authoring, and deterministic graph-to-v2-`.nir` emission — substantially implemented (`crates/nirdosha-graph`); the deterministic emitter and spec schema versioning remain real gaps |
| [0021.a](./0021.a-workflow-authoring-graph.md) | Non-executable workflow/state/transition and approval-policy authoring schemas — substantially implemented, including all five derived edges |
| [0021.b](./0021.b-approval-runtime.md) | Multi-person approval runtime, person-equivalence trust contract, durable ledger and outbox — substantially implemented and now wired in (`nirdosha-hi workflow ...`, `crates/nirdosha-workflow`) |
| [0021.c](./0021.c-graph-analysis.md) | Analysis findings, checker integration, coverage and evidence freshness — tool surface implemented; real structural/reachability/predicate adapters remain open |
| [0022](./0022-web-layer-hardening-and-switchable-transport.md) | Web-layer hardening for `nirdosha-rt`'s live `Router` — `Secure`/`SameSite` cookies, real non-wildcard CORS, per-IP and Redis-backed fleet-wide rate limiting, a switchable async/sync `serve_until` transport (async default), dialect-wide claims-based authorization (`Claim`/`ClaimProof`, `requires(claim = ..)`), RFC 9449 DPoP (`with_sender_constrained_tokens`), real TLS termination (`with_tls`), and security headers on every response — shipped |
| [0023](./0023-data-guard.md) | Store-agnostic access control — one decision surface, store-native enforcement, typed IR, Cedar front-end, relation lowering, federated reads, governed reference data, and a first-class data-guard MCP for LLM agents — proposed, revised |
| [0024](./0024-hi-and-guard-mcp-integration.md) | `nirdosha-hi` integration with the data-guard MCP — safe composition of graph authoring and live data access, with delegation tokens, evaluate-then-act, and maker-checker policy mutations — proposed |
| [0025](./0025-nirdosha-rtm-ecosystem.md) | Nirdosha RTM Ecosystem — vendor-neutral modules on the guard kernel, MIC, rustc + macros + drivers architecture, and conformance suite — draft |
| [0026](./0026-nirdosha-metadata-plane.md) | The Nirdosha Metadata Plane — governed lineage over the guard kernel, declared/observed graph delta, V10, and lineage query/composition macros — draft |
| [0027](./0027-verified-stream-compute-layer.md) | A verified stream-compute layer (`dataflow!`) for `nirdosha-rt` — Flink-style operators as contract-checked fns, driver-verified determinism and exactly-once commit order, checkpoint/replay runtime; per-capability adopt/reject map against Flink's distributed model; web-centric fan-out, projections, and audit taps — draft |
| [0027 implementation plan](./0027-implementation-plan.md) | File-level implementation plan for RFC 0027 — per-crate change list, new files, phase gates, verification commands — draft |

A decision made in the course of implementing something, not designed
up front, goes in [`docs/adr/`](../docs/adr/README.md) instead — a
plain record of what was decided and why, not a proposal awaiting
review.
