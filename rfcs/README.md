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

A decision made in the course of implementing something, not designed
up front, goes in [`docs/adr/`](../docs/adr/README.md) instead — a
plain record of what was decided and why, not a proposal awaiting
review.
