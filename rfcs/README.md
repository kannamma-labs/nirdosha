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

A decision made in the course of implementing something, not designed
up front, goes in [`docs/adr/`](../docs/adr/README.md) instead — a
plain record of what was decided and why, not a proposal awaiting
review.
