# Evidence: panic containment survives the real production link path

`crates/runtime-kernels/src/kernel/thread_pool.rs` (ported from the
now-removed interpreter's `thread_pool.rs`) relies on
`std::panic::catch_unwind` to stop a panicking spawned job from
poisoning the whole pool. The crate's `[profile.release]` originally
set `panic = "abort"`, on the reasoning that a panic reaching this
crate's own `extern "C"` boundary with unwinding enabled would be
undefined behavior. That reasoning predates Rust 1.71 — since then,
unwinding across a plain (non-`"C-unwind"`) `extern "C"` boundary is
*defined*: a safe abort at that exact boundary, not UB. That's what
makes `panic = "unwind"` (the current setting) strictly safer here than
`abort`, not riskier: `catch_unwind` can now actually contain a
panicking job, and an uncaught panic that somehow still reached one of
this crate's `extern "C"` kernels still aborts at that boundary either
way — the same safe-failure behavior `abort` gave everywhere, with
`catch_unwind` now able to intercept it before that point.

`cargo test`'s own passing suite for `thread_pool` does **not** prove
this, because `cargo test` builds in the `dev`/`test` profile, which
already unwinds by default — it proves the scheduling/reuse logic is
correct, nothing about the real shipped artifact. This directory is the
real test: does containment survive when the code is called via
`extern "C"` from a host with **zero Rust runtime of its own** — the
exact shape of a real compiled `.nir` binary (raw LLVM IR + this
staticlib, linked by a bare `clang` invocation, `codegen.rs::build`'s
own convention)?

## What's here

- `main.c` — a plain C program, no Rust anywhere in it, declaring and
  calling `nir_kernel_self_test_panic_containment()` (defined in
  `crates/runtime-kernels/src/lib.rs`, `#[unsafe(no_mangle)] pub extern
  "C" fn`, not a real language builtin — no `.nir` program can reach
  it). That function submits a panicking job to a `ThreadPool`, waits
  for it to run, then submits a normal job and returns `1` only if the
  pool survived and ran it.

## Running it

```sh
cd crates/runtime-kernels && cargo build --release
cd ../../rfcs/evidence/0007-apm-runtime-kernel/panic_containment
clang main.c ../../../../crates/runtime-kernels/target/release/libnirdosha_runtime_kernels.a -lm -o panic_test
./panic_test
```

This is the *exact* link command `codegen.rs::build()` issues for a
real `nirdosha build` (see its own `clang_cmd.arg(&ll_path).arg(&runtime_lib_path)...arg("-lm")`),
minus the generated `.ll` file, which `main.c` stands in for here —
same staticlib, same bare-`clang` linking, same "no Rust runtime except
what's statically in the `.a`" environment.

## Result

Linked clean (no missing-symbol errors — the concern that `panic =
"unwind"` might need unwind-runtime symbols `codegen.rs::build()`
doesn't forward on Unix did not materialize on this toolchain/platform).
Run 5/5 clean:

```
thread '<unnamed>' panicked at src/lib.rs:69:32:
nir_kernel_self_test_panic_containment: expected panic, containment under test
nir_kernel_self_test_panic_containment() = 1
```

The panic fired, was printed by Rust's own panic hook (proving the
panic/unwind machinery is genuinely active, not silently disabled), was
caught by `catch_unwind` inside `worker_loop`, the pool survived, ran
the job submitted after it, and the whole C-hosted process exited `0`.

`kernel_bench` (`../kernel_bench/`) re-run against this same build
confirms zero measurable overhead on any `nir_tcp_*`/`nir_file_*` call
from enabling unwinding — unwind tables are metadata, not a runtime
cost, when no panic occurs, consistent with Rust's (and C++'s)
zero-cost-exceptions design.
