# 0010: `runtime-kernels` also builds as an `rlib`, and shares its workspace with `compiled-serve`

Date: 2026-09-08
Status: accepted

## Context

`crates/compiled-serve` (ROADMAP B8, `rfcs/0010-landing-and-serve-exposure.md`)
needs real Rust-level access to `runtime-kernels`'s admission kernel —
`Domain::ServeHttp`, `acquire`/`release`, `dump_report` — as an ordinary
Cargo dependency, not through the `extern "C"`/staticlib-embedding path
every compiled `.nir` binary uses (`crates/compiler/build.rs`'s
`include_bytes!`).

Three things this phase found while wiring that up, none assumed in
advance:

1. `crates/runtime-kernels/Cargo.toml`'s `[lib] crate-type` was
   `["staticlib"]` only. A `staticlib`-only crate produces a plain `.a`
   archive with C-ABI symbols and no Rust crate metadata — nothing in
   that artifact another Rust crate's `cargo build` could actually link
   against as a dependency.
2. `mod kernel;` in `crates/runtime-kernels/src/lib.rs` was private —
   `Domain`/`acquire`/`release`/`dump_report` were all already `pub` at
   the item level, but unreachable from outside the crate because the
   module itself wasn't re-exported.
3. **The first fix tried for "where does `compiled-serve` live," and
   why it failed, not silently abandoned**: adding `crates/compiled-serve`
   as a member of the repo's *root* workspace, with a plain
   `nirdosha-runtime-kernels = { path = "../runtime-kernels" }`
   dependency, produced a real `cargo` error — `multiple workspace
   roots found in the same workspace`. Cargo does not allow a member of
   one workspace to path-depend on a package that is itself a
   *different* workspace's root (`runtime-kernels`'s own `[workspace]`
   table, ADR 0003's own deliberate isolation from the root workspace).
   `compiled-serve` needed a real place to live that could depend on
   `runtime-kernels` without recreating exactly the lock-contention risk
   ADR 0003 exists to avoid.

## Decision

**`crate-type = ["staticlib", "rlib"]`** — purely additive.
`crates/compiler/build.rs`'s own `cargo rustc ... --manifest-path
crates/runtime-kernels/Cargo.toml` invocation passes no explicit
`--crate-type` flag (confirmed by reading it, not assumed), so it
already builds whatever `[lib] crate-type` names; adding `rlib` makes
it emit both artifacts from the same one invocation, and `build.rs`'s
own artifact lookup targets a fixed filename
(`libnirdosha_runtime_kernels.a`) unaffected by an `.rlib` appearing
alongside it. Verified directly: a clean `cargo build --release` in
`crates/runtime-kernels` on its own produces both
`libnirdosha_runtime_kernels.a` and `libnirdosha_runtime_kernels.rlib`;
`cargo build --release -p nirdosha` (the embedding path) still
succeeds; the full `transact_*` codegen test suite (real compiled
binaries, real linking against the embedded staticlib) still passes
unchanged.

**`mod kernel;` → `pub mod kernel;`** in `lib.rs` — the module is now
part of this crate's public Rust API surface, alongside its unchanged
`#[no_mangle] extern "C"` surface every compiled `.nir` binary already
uses. No behavior change for that FFI surface; this only adds a second,
parallel way to reach the same code from ordinary Rust.

**`compiled-serve` joins `runtime-kernels`'s own separate workspace, not
the root one** — `crates/runtime-kernels/Cargo.toml`'s `[workspace]`
table gains `members = [".", "../compiled-serve"]`, and
`crates/compiled-serve/Cargo.toml` pins itself there explicitly with
`package.workspace = "../runtime-kernels"` (required: Cargo's default
workspace-root discovery walks *upward* from a manifest with no
`package.workspace` key looking for the nearest ancestor `[workspace]`,
which finds the repo root first by pure directory proximity regardless
of what `runtime-kernels`'s own `members` list says — confirmed
directly, `package ... is a member of the wrong workspace`, before
adding this key). Both crates now share one workspace, isolated from
the root one exactly as `runtime-kernels` alone already was — a
principled choice, not an accident of where the error message pushed
things: `compiled-serve` will eventually need the *same* "`crates/compiler/build.rs`
shells out to `cargo rustc` on it" embedding treatment `runtime-kernels`
already gets, to conditionally link it into a `nirdosha build --serve`d
binary — putting it in the root workspace would just relocate today's
exact lock-contention problem there instead of avoiding it.

## Consequences

**`compiled-serve` depends on `runtime-kernels` as an ordinary,
same-workspace path dependency** — no lock-contention risk, since both
builds happen under the one `cargo build` invoked against their shared
workspace; there is no nested `cargo` invocation on this edge at all.

**The root workspace still has no idea either crate exists** — `cargo
build`/`cargo test` from the repo root never sees `runtime-kernels` or
`compiled-serve` as members, matching ADR 0003's original isolation
exactly. Building/testing either one directly means `cd
crates/runtime-kernels` (or passing `--manifest-path`) first, the same
as `runtime-kernels` alone already required.

**A real, disclosed limit, not solved here**: this crate's own two
`extern "C"` FFI-boundary panic-safety properties (`panic = "unwind"`,
`kernel::thread_pool`'s `catch_unwind` containment) were verified
specifically for the staticlib-embedded, cross-*language*-boundary case
(`rfcs/evidence/0007-apm-runtime-kernel/panic_containment/`). Calling
straight into `kernel::acquire`/`release`/`dump_report` as plain Rust
functions from `compiled-serve` crosses no FFI boundary at all — an
ordinary Rust panic there unwinds exactly like any other Rust panic
would. What does *not* unwind, found while building `compiled-serve`
itself (not assumed): a panic inside a `RouteHandler` — a plain `extern
"C" fn`, the ABI a real dispatch-table entry needs — aborts the whole
process immediately, before any `catch_unwind` on the safe-Rust caller
side ever sees it (`crates/compiled-serve/src/lib.rs`'s own
`ServeHttpLease` doc comment has the full detail, and the specific
error reproduced: `thread caused non-unwinding panic. aborting.`). This
is consistent with, not a departure from, this project's existing
"a compiled trap is an unconditional `abort()`" philosophy
(`codegen.rs::emit_transact`'s own doc comment) — a real route handler
is eventually compiled LLVM code, which already can't unwind either
way.
