# RFC 0008: Native plugin ABI widening and Cargo-driven discovery — a compile-time-only path to a real plugin ecosystem

> **Status.** Phase 1 (§Phase 1 below) is built and verified, on this
> branch, not a proposal: `crate::plugin::NativePluginBuiltin::validate`
> now accepts `str`, `handle(Kind)`, and `&handle(Kind)` in addition to
> plain scalars; `codegen.rs::llvm_ty` gives `Ty::Handle(_)` a real `i64`
> mapping (previously a hard `unsupported()` rejection). Two real
> reference crates exist and are exercised end to end by a real compiled
> binary, not just typechecked: `crates/plugin-example-native-shout`
> (`str` both directions) and `crates/plugin-example-native-kv`
> (`handle(Kind)` + `&handle(Kind)` borrow-to-read/move-to-close, `str`
> params/returns). `crates/compiler/tests/native_plugin_codegen.rs` (new:
> `a_str_typed_native_plugin_crosses_the_boundary_in_both_directions`,
> `a_handle_typed_native_plugin_crosses_the_boundary_as_a_plain_i64`,
> `a_native_plugin_handle_used_twice_is_a_compile_time_ownership_error`,
> `a_borrowed_native_plugin_handle_can_be_read_any_number_of_times_
> before_one_final_close`) and `crates/compiler/tests/
> native_plugin_examples.rs` (new file: builds both example crates for
> real via `cargo build --release`, links them into one compiled binary,
> runs it) are the evidence — `cargo test -p nirdosha --no-fail-fast`
> stays 100% green throughout, no existing behavior changed. Phases 2
> (authoring macro), 3 (Cargo-driven `nirdosha build` auto-discovery),
> and 4 (a real external-I/O plugin, e.g. `mysql`, ported onto this ABI
> against a live service) remain open — Phase 1 makes them possible, it
> doesn't do them. The rest of this document is the original design
> capture, kept as written below; this box is the only part updated
> after the fact.

> **Provenance note.** This RFC responds directly to an external
> assessment of this repo ("ecosystem is essentially zero outside the
> language itself... adoption will depend heavily on how quickly the
> missing I/O and data layers land") that turned out to be accurate,
> not stale: `crates/plugin-example-{mysql,activemq,cassandra,neo4j,
> hbase}/` and `crates/plugin-support/` — the only plugin ecosystem
> that ever existed, giving real I/O to SQL/MQ/Cassandra/HBase/Neo4j —
> were deleted in `c82fa1f` ("refactor: remove native plugin
> ecosystem") because they depended entirely on the tree-walking
> interpreter's `PluginBuiltin`/`PluginFn` dispatch, which no longer
> exists on `remove-interpreter`. What survived,
> `crates/compiler/src/plugin.rs`'s `NativePluginBuiltin`
> (rfcs/0005 §3), is a real, shipped, compile-time-only calling
> mechanism — but a scalar-only one, with no plugin built on it and no
> way to discover one automatically. This RFC is explicitly scoped to
> **not** reopen interpreter dispatch or any form of dynamic/WASM
> loading (rfcs/0004's Kind C) — every mechanism proposed here resolves
> at `nirdosha build` time and links statically, the same posture
> `NativePluginBuiltin` already has.

## Motivation

`NativePluginBuiltin` (`crates/compiler/src/plugin.rs`) proved the hard
part of the compiled-path plugin question in rfcs/0005 §3: a plugin
author's `#[no_mangle] extern "C"` symbol, precompiled into a
`staticlib`, can be declared and called directly from generated LLVM
IR (`emit_llvm_ir_impl`, `codegen.rs:1307`, inserts the plugin's
signature into the same `sigs` table an ordinary `fn` uses, so
`Codegen::call`'s existing dispatch needs zero changes) and linked into
the final `clang` invocation alongside `RUNTIME_KERNELS_LIB`
(`build_impl`, `codegen.rs:6771`). `crates/compiler/tests/
native_plugin_codegen.rs` proves this end to end: a real, separately
`rustc`-compiled function, called from a real compiled binary, no
interpreter anywhere in the path.

Two things stop that proof from being an ecosystem:

1. **`NativePluginBuiltin::validate()` accepted only scalars, as of the
   state this RFC opens against** — `i8..usize`/`f64`/`bool`/`void`.
   Every real I/O plugin the deleted gallery had (a SQL row, a
   Cassandra query result, a DB connection handle) needs at least a
   string or an opaque handle to cross the boundary; none of that
   compiled at the time this RFC was opened (see the Status box above
   for what §Phase 1 below has since changed).
2. **There is no discovery mechanism at all.** Every existing native
   plugin test hand-assembles a `&[NativePluginBuiltin]` slice in Rust
   source (`native_plugin_codegen.rs:88`) and passes it to
   `typecheck_with_native_plugins`/`build_with_native_plugins`
   directly. The bare `nirdosha build` CLI has no way to *find* a
   plugin crate on its own (`emit_llvm_ir_with_native_plugins`'s own
   doc comment, `codegen.rs:1281-1292`, says exactly this). The Cargo-
   based discovery design that once existed for the interpreter path
   (`docs/ECOSYSTEM.md` §G1, "Stage 1", built 2026-09-04) was deleted
   with the interpreter along with the plugins that used it.

Neither gap is a request to relax "compile-time only, nothing
interpreted" — both are asking that constraint to do more work than a
scalar-only ABI and a hand-written Rust slice currently let it.

## Design

### Phase 1 — Widen the native ABI past scalars

**1a. `Ty::Handle` costs nothing new and should be accepted immediately.**
A `Ty::Handle(String)` value's runtime representation is already a
plain `i64` (`HandleRegistry::insert`'s return, rfcs/0005 §1) with all
of its safety coming from `ownership.rs`'s affine tracking at the
*type* level, not from anything special in codegen. `codegen.rs`'s
`llvm_ty` currently rejects `Handle` for the compiled path only because
"plugins are already interpreter-only for the compiled path" (rfcs/0005
§1) — a precondition this RFC removes. Concretely:
`NativePluginBuiltin::validate`'s scalar check (`plugin.rs`) gains
`Ty::Handle(_)`, and `codegen.rs::llvm_ty` emits `i64` for it exactly as
it does for `Ty::Thread`/`Ty::Channel`/`Ty::File` today. **Built and
verified** (see this document's Status box) — this really was as cheap
as predicted, a one-line change in each of the two files.

**1b. `str` crossing, using the representation that already exists —
turned out to need even less new machinery than planned.** `codegen.rs`
already defines `Ty::Str => "{ptr, i64}"` — Nirdosha's compiled string
representation is *already* a pointer+length pair — and, load-bearing
for this RFC, `Ty::is_aggregate()` **deliberately excludes** `Ty::Str`:
a `str` value already passes and returns *by value*, in registers, the
same way `f64` and `runtime-kernels/src/lib.rs`'s own two-word
`Dec128Bits` do, never through the `sret`/by-pointer convention
`Vector`/`Matrix`/`Ty::Named` need. Practical effect: `Codegen::call`'s
existing generic `sigs`-driven dispatch already declares and calls a
`str`-typed native symbol correctly with **zero codegen changes** — the
original plan below (decompose into two separate scalar arguments, plus
an explicit `<name>_free` convention for owned returns) turned out to
be solving a problem codegen didn't have. What actually shipped is
simpler: a plugin author's own `#[repr(C)]` struct of exactly `(ptr:
*const u8, len: i64)`, passed **by value** (matching `Dec128Bits`'s own
precedent exactly), in both directions. A `str`-typed return is never
freed by Nirdosha at all — no `owns_returned_str` field, no free-symbol
convention — matching the *existing*, already-shipped posture
`codegen.rs`'s own `sha256_hex` builtin already has (`Ty::Str` isn't
affine, so there is no scope-closing point to hook a free onto); a
plugin author who wants a real free path can still add one to their own
public API, but nothing about this ABI requires or invokes it. The
original two-scalar-decomposition and `<name>_free` design below is
kept struck through in spirit (not deleted from this document — this
RFC's own convention, matching rfcs/0005's revision style, is to record
what was tried and superseded, not silently rewrite history) by this
paragraph; see `crates/plugin-example-native-shout` for the real,
shipped shape.

**1c. A real finding this RFC's original draft missed: reads must
borrow, not just widen the type.** Building `crates/plugin-example-
native-kv` (Phase 4-lite, done alongside Phase 1 — see Status box)
surfaced a genuine gap the plan below didn't anticipate: `Ty::Handle`
being affine (rfcs/0005 §1) means a bare `handle(Kind)`-typed parameter
is **consumed** on its first use, exactly like any other affine value —
so a native `kv_set`/`kv_get` declared as taking a plain `handle(Kind)`
would make the handle unusable after the very first call, permanently
blocking any real "connect once, query N times, close" plugin (which is
most of them). The deleted interpreter-path test suite had already
solved this once (`widget_query(&handle(Widget))`, rfcs/0005 §1's own
evidence) and this RFC's Phase 1 rediscovers and ports the identical
answer to the compiled path: `&handle(Kind)` — `Ty::Ref(Box::new(Ty::
Handle(_)))` — **borrows** rather than consumes, and needed no new
codegen either, because `codegen.rs::llvm_ty` already renders any
`Ty::Ref(_)` as a plain one-word `ptr` (the address of the caller's
`i64` handle slot). `NativePluginBuiltin::validate` accepts this one
specific shape (not `Ty::Ref` in general — a `&str`/`&i64` native param
would be redundant, since those already cross by value); the plugin's
own Rust signature takes a `*const i64` and dereferences it once. Only
the one real, final, resource-closing call (`kv_close(h)`, no `&`)
consumes the handle. `crates/compiler/tests/native_plugin_codegen.rs`'s
`a_borrowed_native_plugin_handle_can_be_read_any_number_of_times_
before_one_final_close` is the proof.

**1d. JSON as a str-boundary convenience, not a new ABI.** `Ty::Json`
does **not** gain a native representation of its own. A native plugin
that needs structured data declares its native signature as `str` in
and/or out; a thin `.nir`-side or generated-shim `json_encode`/
`json_decode` pair (already real builtins per `docs/ECOSYSTEM.md`'s
own builtin registry, `[DONE]`) sits on the `.nir` side of the call.
This is exactly what the deleted `cassandra_query` plugin did for row
results in the interpreter world (`cql_value_to_json`) — moved to the
`.nir` side of a str boundary instead of the Rust side, same shape.

**Deliberately not attempted here**: raw `struct`/`enum` crossing,
generics, trait objects, `async` — none of these have a compiled-path
answer this RFC proposes, and rfcs/0005 §3's own "harder, still-open
question" framing is correct that this is separate, larger design work
than an ABI widening. A plugin author works around this the same way
Rust FFI authors always have: flatten to scalars/strings/handles at
the boundary, keep the rich type on the Rust side of it.

### Phase 2 — An authoring convention (a macro + a manifest, not new tooling)

A plugin author applies a proc-macro, e.g. `#[nirdosha_native_fn]`, to
a function whose signature is already restricted to the Phase 1 surface
(scalar | `Ty::Handle`-shaped `i64` | `&handle(Kind)`-shaped `*const
i64` | `str`-shaped `#[repr(C)] { ptr, len }` by value). The macro:

- Generates the `#[unsafe(no_mangle)] extern "C"` shim performing the
  `NirStr`/handle-pointer marshaling (Rust `&str`/`String` in and out,
  a dereferenced `*const i64` for a borrowed handle) — the same
  mechanical translation `crates/plugin-example-native-shout` and
  `crates/plugin-example-native-kv` currently hand-write (see each
  crate's own `src/lib.rs`).
- Registers the function's Nirdosha-facing signature (name, `Ty` for
  each param, `Ty` for the return) into a `build.rs`-run collector.

The plugin crate's `build.rs` dumps that collected signature list as a
manifest file (plain TOML, one file per crate, sitting next to the
compiled `.a` the same way `NativePluginBuiltin::static_lib`'s own doc
comment already describes a plugin crate producing via
`include_bytes!`) — **data written to disk by a build script the
plugin author's own `cargo build` already runs**, not code the
Nirdosha compiler ever executes. This preserves "everything happens at
compile time, nothing interpreted" precisely: `nirdosha build` reads a
TOML file, it never loads or runs a byte of the plugin crate's own
logic to learn its shape.

### Phase 3 — `nirdosha build` does the discovery and linking automatically

Revives the `[package.metadata.nirdosha]` convention `docs/ECOSYSTEM.md`
§G1 designed and partially shipped for the interpreter path (Stage 1,
2026-09-04), re-targeted at `NativePluginBuiltin` instead of the
deleted `PluginBuiltin`/`NirdoshaPlugin` trait:

1. A project's own `Cargo.toml` lists native plugin crates as ordinary
   `[dependencies]`, tagged `kind = "nir-native"` in that crate's own
   `[package.metadata.nirdosha]` block so `nirdosha build` can tell a
   plugin crate from an ordinary Rust dependency without guessing.
2. `nirdosha build` (or a `nirdosha plugin fetch` prep step) shells out
   to `cargo build --release -p <plugin-crate>` for each declared
   plugin — a build-time subprocess invocation, the same posture
   `codegen.rs::build_impl` already takes toward `clang` (`codegen.
   rs:6815`), not runtime interpretation of anything.
3. Reads each plugin's Phase-2 manifest, constructs the
   `NativePluginBuiltin` roster in-process (replacing today's
   hand-written Rust slice), and passes it to
   `typecheck_with_native_plugins`/`build_with_native_plugins`/
   `emit_llvm_ir_with_native_plugins` exactly as `native_plugin_codegen.
   rs`'s test already does manually.
4. Reads the compiled `.a` bytes from each plugin crate's own target
   directory into `NativePluginBuiltin::static_lib`, replacing the
   test's `include_bytes!`/manual-read stand-in with a real discovered
   path.

None of steps 1-4 introduce a new registry, a new file format nobody
already reads (TOML via `[package.metadata.*]` is exactly what Cargo
itself already supports for third-party tool metadata), or any load of
the plugin's logic ahead of static linking.

### Phase 4 — Prove it on one real crate before calling it done

Port `mysql` — the most complete of the five deleted plugins — onto
this ABI end to end: `connect`/`query`/`execute`/`close` as
`Ty::Handle`+`Ty::Str` native functions, a real `nirdosha build`
producing a native binary, run against a live MySQL container, the
same live-verification bar `docs/ECOSYSTEM.md` §G1's Stage 1 gap-
closing pass and rfcs/0005 §3 both already held themselves to. Only
after this lands does "a real, compile-time plugin ecosystem" stop
being a claim about a mechanism and start being a claim about
something a `.nir` program can actually do.

## Effect on the permission model

`NativePluginBuiltin` today carries no effect classification at all —
unlike the interpreter's `PluginBuiltin`, which gained a required
`effects: BTreeSet<Effect>` field in rfcs/0003 specifically because an
untagged plugin call was invisible to `effects.rs`'s `Expr::Call`
walk and let a function declaring `effect(pure)` call a real
network-backed plugin undetected. The compiled path doesn't run
`effects::infer_effects` as part of `nirdosha build` today (only
`emit-ui`, `main.rs:475`, does, for its own unrelated reason) — so this
RFC introduces no *regression* — but the instant effect-purity checking
is extended to the compiled path (a real, separately-tracked need),
`NativePluginBuiltin` will need the identical `effects` field rfcs/0003
already added to its interpreter counterpart, or the exact
same unsoundness reappears on the compiled side. Tracked as an open
question below rather than solved here, since it's speculative against
work that hasn't landed.

`requires(role/claim: ...)`/`acquire`/`screen` gates are unaffected: a
native plugin call is an ordinary function call in `sigs` as far as
`typeck.rs` is concerned (`codegen.rs:1328`'s comment is explicit about
this), so anything gating a call today gates a native plugin call
identically.

## Compatibility

Fully additive, and turned out even more so than planned: `NativePluginBuiltin`'s
own shape (`name`/`params`/`ret`/`static_lib`) is unchanged — no new
field, since the simpler by-value `str`/leak-always convention (§Phase
1b) needed none. `validate()`'s internal check gained `Ty::Str`,
`Ty::Handle(_)`, and `Ty::Ref(Box<Ty::Handle(_)>)` as accepted shapes;
`codegen.rs::llvm_ty` gained one new match arm (`Ty::Handle(_) =>
"i64"`, previously a hard rejection). No `.nir` grammar changes — `&h`
and `handle(Kind)` were already real, parseable syntax (rfcs/0005).
No existing `.nir` program's compiled output changes, because none
shipped anywhere already calls a native plugin (every caller is a
Rust-side test harness — `native_plugin_codegen.rs`,
`native_plugin_examples.rs` — not a `.nir` program distributed on its
own). `cargo test -p nirdosha --no-fail-fast` stayed 100% green
throughout this change, confirming no existing behavior moved.

## Rejected alternatives

**WASM/dynamic loading (rfcs/0004's Kind C) as the ecosystem answer.**
Explicitly out of scope by the requirement that motivated this RFC:
"nothing should be interpreted, everything should happen at compile
time." `wasmtime`-hosted plugins, even AOT-compiled, still load and
instantiate a module at the target program's runtime, not at
`nirdosha build` time — a different trust/performance trade rfcs/0004
already spiked (real numbers, rot13-to-WASM) for a genuinely different
goal (isolating an *untrusted* plugin author). This RFC's plugins are
exactly as trusted as `RUNTIME_KERNELS_LIB` already is: statically
linked into the same binary, checked by the same `rustc`/`clang`.

**A bespoke Nirdosha plugin registry instead of Cargo.** Rejected for
the same reason `docs/ECOSYSTEM.md` §G1 already rejected it for the
interpreter path: hosting a service, a publish CLI, and its own
uptime/abuse-moderation is the expensive option for a project this RFC
itself is trying to make *more* self-sustaining, not less
(`docs/ECOSYSTEM.md` §G5's solo-maintainer note). Cargo/crates.io
already solve resolution, semver, lockfiles, and hosting; the only
missing piece is the metadata convention Phase 2/3 define.

**A generic (non-scalar) FFI ABI via serialization of arbitrary Rust
types (e.g. `serde`-derived binary encoding for any `T`).** Rejected as
solving a bigger problem than the evidence justifies: every real
plugin in the deleted gallery needed at most scalars, one connection
handle, and JSON-shaped query results — str+Handle+JSON-over-str covers
that whole surface without inventing a general cross-language type
encoding, which is genuinely open research (rfcs/0005 §3's own framing)
rather than a bounded engineering task.

## Open questions

- **Effect classification for native plugins**, deferred above pending
  compiled-path effect-purity checking actually existing to protect.
- **`str`-return memory growth is a real, disclosed, unsolved cost, not
  just a rot13-scale curiosity.** Every native-plugin-returned string is
  leaked for the life of the process (§Phase 1b) — fine for
  `plugin_shout`'s scale, a real concern for a plugin returning large or
  frequent query results (`kv_get` included) in a long-running server
  process. A real free-path convention (plugin-exported
  `<name>_free(ptr, len)`, called once Nirdosha's own use of the string
  provably ends) was in this RFC's first draft and was cut for being
  unjustified by the evidence *so far* — Phase 4's `mysql` port, under
  real sustained load, is what would actually justify designing it for
  real instead of guessing.
- **Should `&T` crossing generalize past `handle(Kind)`?** `Ty::Ref` is
  accepted narrowly (`Ty::Ref(Box<Ty::Handle(_)>)` only) because that's
  the one case with a proven need (§Phase 1c). `Ty::Db`/`Ty::Mq` are
  also affine (rfcs/0005 §1) and would hit the identical "reads must
  borrow" problem the moment either gets a compiled-path story — worth
  widening then, not speculatively now.
- **Versioning across `nirdosha` releases**: rfcs/0003 raised "what
  does plugin ABI compatibility mean given static linking" for the
  interpreter path and left it a policy question; static linking makes
  it arguably simpler here (a plugin is always rebuilt against the
  `nirdosha` version compiling it, never loaded against a mismatched
  one at runtime) but that argument hasn't been written down formally
  yet.
- **Windows.** `build_impl`'s existing native-plugin linking already
  has a real Unix/Windows split for system libraries (`codegen.
  rs:6829-6834`); a plugin crate's own staticlib crossing that same
  split (calling convention, `.lib` vs `.a`) isn't addressed here and
  needs its own pass before Phase 4's `mysql` port is called done on
  more than one platform.
