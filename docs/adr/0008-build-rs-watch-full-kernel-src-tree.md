# 0008: `crates/compiler/build.rs` must watch all of `runtime-kernels/src`, not just `lib.rs`

Date: 2026-09-07
Status: accepted

## Context

A real, reproduced bug, not a hypothetical one: adding
`crates/runtime-kernels/src/kernel/identity.rs` (a new file) and wiring
it into `kernel/mod.rs` (`pub mod identity;`) — without touching
`runtime-kernels/src/lib.rs` itself — left `nirdosha` linking a *stale*
embedded archive missing the new `nir_*` symbols, surfacing as real
`undefined reference` linker errors when running the full test suite,
even though a build immediately after adding the code had succeeded.
Root cause, confirmed by reading `build.rs` directly:
`cargo::rerun-if-changed` was declared only for
`runtime-kernels/src/lib.rs` and its `Cargo.toml` — never the rest of
`src/`. Cargo has no visibility into this build script's own nested
`cargo rustc` shell-out, so it only re-runs the script (and, in turn,
only then recompiles `nirdosha` against a fresh `include_bytes!`-ed
archive) when a path named in `rerun-if-changed` actually changes.
Editing only a file under `kernel/` left `lib.rs` byte-for-byte
unchanged, so the script never re-ran and the compiler kept linking
whatever archive it had embedded last time.

This had gone unnoticed because every *previous* addition to
`runtime-kernels` in this same work also happened to touch `lib.rs`
directly (`kernel::db`/`kernel::http`'s own wiring both edited `lib.rs`
to add or change `nir_db_*`/`nir_http_*` re-exports), incidentally
triggering a rebuild each time and masking the gap. `kernel::identity`
was the first addition confined entirely to a new file plus a one-line
`kernel/mod.rs` change, and hit it immediately.

## Decision

`build.rs` now walks the entire `runtime-kernels/src` directory tree
recursively and emits `cargo::rerun-if-changed` for every file (and
every subdirectory, so a newly-added file is also caught) — not just
`lib.rs`. No external crate needed; a small recursive `std::fs::read_dir`
walk is enough.

## Consequences

Any future change anywhere under `runtime-kernels/src` — a new kernel
module, an edit to an existing one, anything — now reliably triggers a
real rebuild of the embedded archive and a recompile of `nirdosha`
against it. Verified directly, not assumed: appended a trivial comment
to `kernel/identity.rs` and confirmed a plain `cargo build --release`
(no `cargo clean`) picked it up and recompiled `nirdosha`, where before
this fix an equivalent change silently would not have.

**Real cost, worth taking**: `build.rs` itself now does a directory
walk on every invocation, even when nothing changed — a few filesystem
stats, not a measurable build-time cost, and Cargo still only re-runs
the expensive part (the nested `cargo rustc` shell-out) when one of the
watched paths' mtimes actually changed.
