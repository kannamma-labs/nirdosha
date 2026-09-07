# Trusted plugins

A lightweight, self-declared trust convention for Kind-A Nirdosha
plugins (`docs/ECOSYSTEM.md` §G1 / rfcs/0004-native-plugin-sandboxing.md)
— explicitly **not** new registry infrastructure (§G1 already rejects
building a bespoke registry/hosting service), just a repo file a
maintainer reviews before adding a row, the same spirit as GitHub's
Verified Publisher badge.

## What this is, and isn't

A Kind-A plugin is arbitrary native Rust code, statically linked
directly into the compiled binary via `codegen::build_with_native_plugins`
(`crates/compiler/src/codegen.rs`, `NativePluginBuiltin`) — full process
trust, no sandbox (rfcs/0004-native-plugin-sandboxing.md's own
open-question section explains why that's a deliberate, disclosed gap,
not solved by this file). Appearing on this list means:

- A maintainer has read the plugin's source at the version listed.
- Its declared `[package.metadata.nirdosha]` builtins and their
  `effects` (rfcs/0003-plugin-abi-v2.md) match what the code actually
  does, as best a human review can tell.
- It has no obvious malicious behavior or gratuitous unsafe code.

Appearing on this list does **not** mean:

- The plugin is free of bugs, or its declared effects can't be wrong —
  nothing *enforces* the declaration is true (rfcs/0004's own honest
  limitation).
- Nirdosha's maintainers audit every future version — a listing
  reflects the version pinned below; re-review on update is manual,
  not automatic.
- Any runtime isolation exists. A trusted plugin can still corrupt the
  host process's memory the same as any other native Rust dependency
  with a genuine bug.

## Listed plugins

**None, as of 2026-09.** The six reference plugins this table used to
list (`nirdosha-plugin-rot13`/`-mysql`/`-activemq`/`-cassandra`/
`-neo4j`/`-hbase`, `crates/plugin-example-*/`) were removed entirely in
`refactor: remove native plugin ecosystem` — every one of them
depended on the tree-walking interpreter's own `PluginBuiltin`/
`PluginFn` dispatch, which no longer exists (the interpreter was
deleted in a separate pass the same session). `NativePluginBuiltin`
(the compiled-path plugin ABI these examples never targeted, a
different and narrower scalar-only shape) survives and is real —
`crates/compiler/tests/native_plugin_codegen.rs` exercises it directly
— but nothing currently plugs into it: no reference plugin crate
implements it, and `nirdosha build`'s own CLI has no flag to load a
native plugin at all yet (`build_with_native_plugins` is a library
entry point with no wired-up caller). This table's real purpose is
unchanged — the template a genuine plugin's listing follows, and the
day-one gate a future auto-discovery step (RFC 0001) requires — it
just currently has zero rows to show for it.

## Requesting a listing

Open a PR adding a row above, with:

1. The crate name, exact version (a git tag/commit for an unpublished
   crate), and a link to its source.
2. Its full `[package.metadata.nirdosha]` builtins list, including
   `effects`.
3. A one-line description of what it does and what external system (if
   any) it talks to.

A maintainer reviews the source before merging — see
[`GOVERNANCE.md`](./GOVERNANCE.md) for who that is today. Expect this
to take real review time, not to be a formality; that's the entire
point of the list existing.
