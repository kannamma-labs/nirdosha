# v1 → v2 capability port map

Started 2026-09-20. `crates/compiler` (the original interpreted/native
`.nir` compiler) was deleted from this repo the same day, deprecated in
favor of the v2 Rust dialect (`docs/V2_MIGRATION_ROADMAP.md`). Deleting
that crate did not delete the *need* for everything it did — some of its
capabilities have no v2-native equivalent yet and genuinely have to be
rebuilt, through v2's own architecture (plain Rust types → the
`#[nirdosha_rt::contract(...)]` proc-macro → the `nirdosha-driver`
MIR-level Stage 2 checker), not assumed away or quietly dropped.

**The rule this doc exists to enforce**: when closing a gap turns up
something that still needs porting from the old compiler to v2's
rustc + macro + driver architecture, that gets a row here, and the code/
crates the port would start from stay in the tree (even if currently
unused by anything) rather than being deleted. Deleting reference
material because "nothing consumes it *yet*" throws away the port's own
starting point — the crates below survive expressly to be that starting
point when someone gets to a row's `[OPEN]`.

Status values: `[OPEN]` — not started. `[PARTIAL]` — some real v2-native
piece exists; the row's own notes say what's still missing. `[DONE]` —
fully ported; kept here as a record of where the equivalent landed.

## Native / third-party plugin support

**Status: `[OPEN]`**

v1's compiled-path plugin ABI (rfcs/0008-native-plugin-abi-widening.md,
rfcs/0011-uniform-service-provider-model.md, rfcs/0009-ui-catalog-
extensibility.md): a plugin author ships a `crate-type = ["staticlib"]`
crate exporting `#[no_mangle] extern "C"` functions matching a fixed ABI
shape (`str`/`handle(Kind)` scalars only); `nirdosha build` links the
resulting `.a` into the compiled binary and `codegen.rs` emits calls to
the plugin's symbols directly, or (for a `call`-shape RFC 0011 provider)
emits a `nir_kernel_register_plugin_provider` call so the kernel's own
runtime dispatch table can route a URL scheme to it. Four reference
crates in this repo still implement the plugin-author side of this
exactly as designed, and are being kept for exactly this reason:

- `crates/plugin-example-native-shout` — the minimal `str`-crossing
  reference (rfcs/0008 Phase 1).
- `crates/plugin-example-native-kv` — the stateful `handle(Kind)`
  reference, `kv_open`/`kv_set`/`kv_get`/`kv_close` (rfcs/0008 Phase 1).
- `crates/plugin-example-native-authed-http` — a real `call`-shape
  provider for `call_via`'s `authedhttp://` scheme (rfcs/0011 Phase 8).
  Its own `extern "C"` functions have the exact shape
  `nirdosha_runtime_kernels::kernel::plugin_provider::ProviderFns`
  expects, and that module's own `register` function is `pub` and
  already reachable from v2 code today (`runtime-kernels` is in this
  repo's live dependency graph, unlike `crates/compiler` ever was) —
  the missing piece isn't the runtime dispatch, it's (a) a safe way to
  bridge this crate's own hand-rolled `#[repr(C)] struct NirStr` to
  `runtime-kernels::kernel::pool::NirStr` (same layout, different
  nominal Rust type — needs an explicit, documented, presumably
  `unsafe`-but-justified bridge, not a silent transmute buried in
  application code) and (b) no v2 example has ever actually called
  `plugin_provider::register` from a real `fn main()` to prove this
  works end to end.
- `crates/ui-plugin-example-sparkline` — rfcs/0009 Phase B's UI-catalog
  extension. Its own README's consumer-side example
  (`nirdosha::ui_plugin::NativeUiComponent`) named a type that lived in
  `crates/compiler` and is gone with it — the crate's own constants
  (`NAME`/`RENDER_JS`/`RENDER_FN`) are still real and still compile, but
  whatever v2's own UI-catalog-extension consumer looks like has to be
  designed from scratch; nothing today reads these constants at all.

**What a real v2 port needs to decide, not yet decided**: whether v2
even wants the staticlib+ABI-linking model at all, now that a v2 "plugin"
crate can simply be an ordinary Cargo dependency the app crate calls
directly (no `extern "C"`, no separate ABI, no link step) — the ABI
crossing existed in v1 specifically because the compiler produced its
own standalone binary and a plugin had to cross into *that*, a
constraint plain-Rust v2 mostly doesn't have. The `authedhttp` case
above is the one part of this that's *not* purely an ABI-crossing
question — `plugin_provider`'s runtime dispatch table (routing a URL
scheme to a provider chosen at runtime, not at compile time) is a real
capability an ordinary static Cargo dependency doesn't replace by
itself, and is worth porting on its own merits regardless of what
happens to the ABI-linking question above.

**Prior art in this repo**: `TRUSTED_PLUGINS.md` is v1's trust
convention for a listed plugin (self-declared, maintainer-reviewed) —
its own "Listed plugins" section already discloses today's status and
points back here.

## SMT/Z3 proof-discharge for Hoare contracts

**Status: `[OPEN]`**

v1's `nirdosha certify` ran a real Tier-1 Z3 encoding
(`contract_check.rs`) over a `validate fn { pre: ..., post: ... }`
block, proving the postcondition for *every* input satisfying the
precondition — a genuine formal proof, not a test. v2's comment-layer
equivalent (`/// nirdosha:validate {"pre":[...],"post":[...]}`) parses
and is inventoried by `cargo-nirdosha`'s scanner
(`nirdosha_hi::v2_verify::describe_v2_source` reports these as
`nirdosha_declarations`), but nothing checks whether the claim is
*true* — `v2_verify.rs`'s own doc comment calls this "inert" and the v2
corpus's own README status table agrees. `crates/bench`'s own 2026-09-20
repoint (this session) hit this directly: its `overflow_checked_multiply`
task used to get a real `PROVED` verdict from Z3; the v2-native
replacement can only report "builds, and the `effects(pure)` claim
passed Stage 2's real interprocedural check" — a different, weaker,
still-real guarantee, not a stand-in for the missing one. `crates/bench/
RESULTS.md` (historical, pre-repoint) and `crates/bench/src/main.rs`
(current) both disclose this gap in place; this row is the durable
tracking entry for actually closing it.

**What a real v2 port needs**: encoding v2's `nirdosha:validate` clauses
into the same kind of Z3 query `contract_check.rs` used to build, wired
either into `cargo-nirdosha verify` (Stage 1) or `nirdosha-driver`
(Stage 2, where it'd have real MIR to reason over rather than
doc-comment text) — not designed here, just named as the real remaining
work.

## Injection-immunity (v1's `str`-has-no-concatenation guarantee)

**Status: open design question, not a straightforward port**

v1's `str` type had no concatenation operator at all — an injectable
SQL/command string was inexpressible, a real by-construction guarantee
the interpreter/compiler's own typechecker enforced. v2 source is plain
Rust; `String` concatenation is ordinary and unrestricted, and
`nirdosha-contract-core/src/scan.rs`'s dialect-restriction table (the
same table that denies `unsafe`, raw locks, raw threads) has no
injection/SQL-building entry at all today. Unlike the two rows above,
this may not be a "port it" item so much as a "does v2 want this
restriction at all" design question — v1's restriction was a real cost
(ordinary string-building code that's obviously safe still doesn't
compile) that v2's "just write real Rust" positioning may or may not
want to reintroduce, even narrowed to query-building contexts. Noted
here so it isn't silently assumed to be equivalent to the other two rows
above, not because a port is definitely the right call.
