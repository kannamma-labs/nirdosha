# nirdosha-plugin-native-kv

The stateful reference for rfcs/0008-native-plugin-abi-widening.md
Phase 1's `handle(Kind)` crossing, combined with `str` (see
`../plugin-example-native-shout` for `str` on its own): `kv_open`/
`kv_set`/`kv_get`/`kv_close`, an in-memory key-value store. This is the
*shape* every real stateful I/O plugin needs — `connect`, use the
connection any number of times, `close` it — the same pattern the
(deleted, interpreter-only) `mysql`/`cassandra`/`activemq` reference
plugins had. It's in-memory only so this example needs no Docker
container to try; porting one of those onto this same ABI is rfcs/0008
Phase 4's real remaining proof.

## Try it

```sh
cargo build --release -p nirdosha-plugin-native-kv
# produces target/release/libnirdosha_plugin_native_kv.a
```

`crates/compiler/tests/native_plugin_examples.rs` builds this exact
crate, links it into a real `nirdosha build` output alongside
`plugin-example-native-shout`, and runs the resulting binary —
`cargo test -p nirdosha --test native_plugin_examples`.

## The one rule this crate exists to demonstrate: borrow to read, move to close

```nir
fn main() -> i64 {
    let h: handle(KvStore) = kv_open()
    let ok: i64 = kv_set(&h, "greeting", "hello")   // &h -- borrows
    let v: str = kv_get(&h, "greeting")              // &h -- borrows
    return kv_close(h)                               // h  -- consumes
}
```

`Ty::Handle` is affine (`ownership.rs`, rfcs/0005 §1): a bare
`handle(KvStore)` value can be used **exactly once** before it's
"moved." If `kv_set`/`kv_get` took a bare `handle(KvStore)` instead of
`&handle(KvStore)`, the first call would consume `h` and every
following call would be a compile-time `UseAfterMove` error — a real
plugin could never be queried more than once. Passing `&h` instead
(`Ty::Ref(Box::new(Ty::Handle(...)))`) **borrows** it: `codegen.rs`
already renders any `Ty::Ref(_)` as a plain one-word `ptr` (the address
of the caller's own `i64` handle slot), so the underlying value is read
through the pointer, never moved. `kv_close(h)` — no `&` — is the one
real, final, consuming use.

On the Rust side, that means `kv_set`/`kv_get` take `*const i64` and
dereference it once ([`src/lib.rs`](./src/lib.rs)); `kv_open`/
`kv_close` take/return a plain `i64`, exactly like
`plugin-example-native-shout`'s scalars.

```toml
[package.metadata.nirdosha]
kind = "native-compiled"
builtins = [
    { name = "kv_open", params = [], ret = "handle(KvStore)" },
    { name = "kv_set", params = ["&handle(KvStore)", "str", "str"], ret = "i64" },
    { name = "kv_get", params = ["&handle(KvStore)", "str"], ret = "str" },
    { name = "kv_close", params = ["handle(KvStore)"], ret = "i64" },
]
```

`crates/compiler/tests/native_plugin_codegen.rs`'s
`a_native_plugin_handle_used_twice_is_a_compile_time_ownership_error`
and `a_borrowed_native_plugin_handle_can_be_read_any_number_of_times_
before_one_final_close` are the two proofs of this rule, using a
smaller inline plugin than this crate.

## The one thing a plugin author has to hand-roll today

The deleted, interpreter-only `nirdosha-plugin-support::HandleRegistry<T>`
gave every stateful plugin a shared, generic `Mutex<HashMap<u64, T>>`
for free. No compiled-path equivalent exists yet — this crate's
`stores()`/`next_handle()` (~15 lines) are what a plugin author writes
by hand until rfcs/0008 Phase 2's authoring-convention crate exists.
`ownership.rs`'s affine tracking is what actually prevents a `.nir`
program from double-closing or leaking a handle — this crate's own Rust
code does not, and does not need to, defend against that itself (see
`kv_close`'s doc comment).

## What's honestly not covered

- **No `Result`/error-detail crossing.** `kv_set`/`kv_close` return a
  plain `1`/`0` sentinel; `kv_get` returns an empty string on a miss —
  a real richer error story is rfcs/0005 §3's "harder, still-open
  question" (no aggregate/`Result` crossing yet).
- **No Cargo-driven auto-discovery.** Consuming this crate still means
  hand-constructing `NativePluginBuiltin`s and calling
  `codegen::build_with_native_plugins` — see
  `plugin-example-native-shout/README.md`'s "app-author side" section,
  identical here.

## Full design

rfcs/0008-native-plugin-abi-widening.md.
