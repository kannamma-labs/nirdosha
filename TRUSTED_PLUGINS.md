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
(the compiled-path plugin ABI these examples never targeted, since
widened past scalars to `str`/`handle(Kind)` by
rfcs/0008-native-plugin-abi-widening.md Phase 1) has two in-repo
reference crates implementing it end to end
(`crates/plugin-example-native-shout`, `crates/plugin-example-native-kv`)
plus a real `call`-shape provider (`crates/plugin-example-native-authed-http`,
rfcs/0011 Phase 8) and a UI-catalog extension
(`crates/ui-plugin-example-sparkline`, rfcs/0009 Phase B) — but nothing
in this repo can currently link their staticlib output into a real
binary (`crates/compiler`, the native AOT compiler that could, was
retired in favor of the v2 Rust dialect) and `nirdosha build`'s own CLI
still has no flag to load a native plugin at all, so none of these are
candidates for this table yet — there's no app-facing way to depend on
one. Wiring a v2-native equivalent (an ordinary Rust dependency plus
whatever trust/discovery story v2 needs) is tracked as an open item in
`docs/V1_CAPABILITY_PORT_MAP.md`, not done. This table's real purpose is
unchanged — the template a genuine plugin's listing follows, and the
day-one gate a future auto-discovery step (RFC 0001) requires — it just
currently has zero rows to show for it.

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
