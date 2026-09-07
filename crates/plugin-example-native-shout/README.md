# nirdosha-plugin-native-shout

The simplest possible reference for rfcs/0008-native-plugin-abi-widening.md
Phase 1's `str` crossing: one builtin, `plugin_shout(s: str) -> str`,
called directly from a compiled `.nir` binary's generated LLVM IR — no
interpreter anywhere in the path. If you want the stateful/`handle(Kind)`
half of the story instead, see `../plugin-example-native-kv`.

## Try it

```sh
cargo build --release -p nirdosha-plugin-native-shout
# produces target/release/libnirdosha_plugin_native_shout.a
```

`crates/compiler/tests/native_plugin_examples.rs` is the automated,
run-the-real-binary proof that this exact crate's compiled output,
linked into a real `nirdosha build`, produces the right answer —
`cargo test -p nirdosha --test native_plugin_examples`.

## How this crate is built (the plugin-author side)

One function, in [`src/lib.rs`](./src/lib.rs):

```rust
#[repr(C)]
pub struct NirStr { pub ptr: *const u8, pub len: i64 }

#[unsafe(no_mangle)]
pub extern "C" fn plugin_shout(s: NirStr) -> NirStr { /* ... */ }
```

Three rules this ABI actually enforces, not just documents:

1. **`NirStr`'s field order and widths must match `Ty::Str`'s own LLVM
   shape exactly** — `{ptr, i64}`, first the pointer, then the length,
   passed **by value** (not a pointer to the struct) — the same
   `#[repr(C)]`-struct-by-value convention `runtime-kernels/src/lib.rs`'s
   `Dec128Bits` already uses for a different two-word value. Get the
   field order wrong and you get silently wrong data, not a compile
   error — there's no cross-language shape-check here yet.
2. **Not NUL-terminated.** `len` is the only thing saying how many bytes
   are valid; a `.nir` `str` can contain an embedded `\0`.
3. **A returned string's memory is never freed by Nirdosha.** `Ty::Str`
   isn't affine (no scope-closing point to hook a free onto — the same
   reason `codegen.rs`'s own `sha256_hex` builtin leaks its output),
   so `plugin_shout` allocates via `Box::leak` and that memory is gone
   for the life of the process. A plugin returning many short-lived
   strings in a hot loop should design around this, not fight it.

`Cargo.toml`'s `[package.metadata.nirdosha]` block declares the same
signature in a statically-greppable form:

```toml
[package.metadata.nirdosha]
kind = "native-compiled"
builtins = [
    { name = "plugin_shout", params = ["str"], ret = "str" },
]
```

## How a project consumes it (the app-author side)

There's no `nirdosha build` CLI flag that finds this crate on its own
yet (rfcs/0008 Phase 3 — Cargo-driven auto-discovery — isn't built).
Today, consuming a native plugin means constructing a
`NativePluginBuiltin` by hand and calling
`codegen::build_with_native_plugins` instead of the plain `nirdosha`
CLI:

```rust
use nirdosha::ast::Ty;
use nirdosha::plugin::NativePluginBuiltin;

let lib_bytes: &'static [u8] = /* read target/release/libnirdosha_plugin_native_shout.a */;
let shout = NativePluginBuiltin {
    name: "plugin_shout".to_string(),
    params: vec![Ty::Str],
    ret: Ty::Str,
    static_lib: lib_bytes,
};
shout.validate().expect("signature must be a supported native-ABI value");
// typecheck_with_native_plugins / check_ownership / codegen::build_with_native_plugins
// -- see crates/compiler/tests/native_plugin_examples.rs for the real,
// working, end-to-end version of this.
```

## What you get for free once it's linked in

- **Static type checking.** `plugin_shout(42)` or wrong arity are real
  *type errors*, caught before `clang` ever runs — `typecheck_with_
  native_plugins` slots this signature into the exact same table an
  ordinary `fn` uses.
- **A named, actionable rejection**, not a confusing LLVM/clang
  failure, for a signature this ABI doesn't support yet (`validate()`).
- **Real native speed.** This is a direct `call` in generated LLVM IR
  into a statically-linked symbol — no interpreter, no dispatch table.

## Full design

rfcs/0008-native-plugin-abi-widening.md — what's built (this crate is
part of the Phase 1/4 evidence), what's still open (Phase 2's authoring
macro, Phase 3's Cargo-driven discovery), and why str/handle crossing
works the way it does.
