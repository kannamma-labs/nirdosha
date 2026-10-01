# 0021: Gate-3 cross-crate registry totality — per-crate HIR fragments, workspace-wide merge

Date: 2026-09-21
Status: accepted

## Context

Today's registry totality (V1, and the coverage matrix generally) is
per-crate: `linkme` distributed slices only see what got linked into one
compiled binary. `crates/nirdosha-driver` already is a working
rustc-driver-based analysis tool (used for `#[contract(effects(pure))]`
interprocedural call-graph checking over MIR) — Plan Phase 17 extends that
same harness for cross-crate registry totality rather than building a new
tool.

**A real architectural constraint found while scoping this**: rustc
compiles one crate at a time. A single `rustc_driver::Callbacks` process
only ever has full HIR for the crate currently being compiled
(`LOCAL_CRATE`) — a dependency crate's HIR isn't available to it (only
already-lowered metadata/MIR-if-needed via cross-crate queries, not full
HIR trees). "Walk HIR across every crate in the workspace's dependency
graph" in one process, as the plan's own phrasing could be read, isn't how
rustc's per-crate compilation model works. The real, achievable version:
have the driver contribute one crate's worth of real, HIR-derived data
*every time it compiles a crate* (which cargo already does once per crate
in a workspace build), accumulating into per-crate fragment files an
independent merge step combines after the full build completes — the same
pattern other rustc-driver-based whole-workspace tools use.

## Decision

**`nirdosha-driver::gate3` (new module)**: when `NIRDOSHA_GATE3_DIR` is
set (opt-in — every other build through this driver is unaffected, the
same pass-through posture the existing contract-claims pass already has),
walks the local crate's `static` items via `tcx.mir_keys(())`
(statics have MIR-lowered initializer bodies, the same enumeration the
existing contract-claims pass already uses), and for any whose *type* is
exactly `PolicyRegistration` or `CatalogRegistration` (matched by
`tcx.def_path_str`, not by attribute or name pattern — see below),
extracts specific literal field values.

**HIR-syntactic extraction, not const-evaluation.** The macro-generated
statics this looks for are always, by construction
(`nirdosha-guard-macros`'s `policy_impl`/`attribute_impl`), simple struct
literals with string-literal fields — never a computed expression.
Matching that shape directly in HIR (`ExprKind::Struct` with
`ExprKind::Lit` field values) avoids decoding a
`mir::interpret::ConstAllocation`'s raw bytes/relocations to recover a
`&'static str`'s pointer+length, which would be the "proper" const-level
approach but is a materially larger, higher-risk undertaking for the same
result given every relevant static's initializer is already syntactically
trivial. A static of the matched type whose initializer isn't this exact
shape is silently skipped, not guessed at.

**Matched by type, not by `#[linkme::distributed_slice(...)]` attribute.**
That attribute is itself a proc-macro that consumes/transforms the item —
by `after_analysis` time (when this pass runs), it does not reliably
survive as an inspectable HIR attribute the way `#[doc]` (already read by
the existing contract-claims pass) does. The static's resolved *type*
(`PolicyRegistration`/`CatalogRegistration`) is stable and unambiguous
regardless of macro-expansion attribute details.

**A real, adjacent finding: `#[dataset(...)]` doesn't populate
`DATASETS`/`DatasetRecord` either** — it only registers into `CATALOG`
(`kind: "dataset"`, raw attribute-argument text), the same
"declared-but-never-populated" pattern `docs/adr/0019` diagnosed for
`ApprovalChainRecord`/`RoleRecord`/`PortRecord`/`ModelRecord`/
`WorkflowRecord`. Gate-3's dataset side therefore reads `CatalogRegistration`
entries with `kind == "dataset"` and parses `entity = "..."` out of their
raw `source` text (the same whitespace-tolerant string extraction
`nirdosha-guard-macros`'s `parse_quorum_clause`, `docs/adr/0019`, uses for
a different clause shape) rather than a `DatasetRecord` that nothing
produces.

**`cargo nirdosha verify --guard --workspace`** (`cargo-nirdosha`, new):
runs `cargo build --workspace` with `RUSTC_WORKSPACE_WRAPPER` set to the
driver and `NIRDOSHA_GATE3_DIR` pointing at a fresh (stale entries
cleared) fragment directory — the same `RUSTC_WORKSPACE_WRAPPER` mechanism
`cargo nirdosha build --deep` already uses — then merges every crate's
fragment and reports any policy `resource` no fragment's `dataset_entities`
names anywhere in the workspace. Scoped specifically to this cross-crate
check, not a re-run of the full V1–V8 pass suite across every package (a
distinct, larger undertaking of merging N per-package guard dumps this
mode doesn't attempt) — stated so the scope isn't misread as broader than
it is.

## Consequences

**What this makes possible.** `cargo test -p nirdosha-driver --test
gate3` builds a real, temporary two-crate Cargo workspace (`crate-a`
declaring a real `guard_policy!` referencing `orphan_resource`, `crate-b`
optionally declaring a real `#[dataset(entity = "orphan_resource", ...)]`)
through the real `nirdosha-driver` binary and proves, end to end: the gap
is caught and correctly attributed to `crate_a` when `crate-b`'s dataset
is absent; the gap disappears once it's present; and — the plan's own
specific point — `crate-a`'s fragment *alone* (what a single crate's
`linkme` dump would see) never contains any `dataset_entities` at all,
proving per-crate V1 structurally cannot see this class of gap regardless
of what a sibling crate declares.

**What this does *not* make possible, stated so it's never misread later.**
`cargo nirdosha verify --guard --workspace` was not additionally smoke-
tested against this repository's own real ~80-crate workspace (a full
rebuild through the driver wrapper, likely several minutes and meaningful
disk pressure on top of this session's own repeated `cargo clean` cycles)
— it shares the exact fragment JSON contract and merge algorithm the
isolated two-crate fixture test already proves end to end, but the CLI's
own subprocess/path-handling glue code has only that indirect coverage,
not a direct real-workspace run. `RoleRecord`/`PortRecord`/`ModelRecord`/
`WorkflowRecord`/`DatasetRecord` remain unpopulated by any macro (`docs/adr/0019`'s
finding, confirmed to extend to `#[dataset]` here) — Gate-3 works around
the dataset side of this via `CatalogRegistration`'s raw text, but doesn't
fix the underlying gap for any of these types.
