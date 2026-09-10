# RFC 0013: Nirdosha Hi's knowledge graph — a local-first project knowledge graph with bidirectional `.nir` traceability

> **Naming note.** This RFC and its filename still say "Realm" — its
> original name when written. The feature itself has since been
> renamed: the module is `hi_graph.rs` (not `realm.rs`), the database
> file is `.nir/hi.db` (not `.nir/realm.db`), the CLI surface is
> `nirdosha hi <ingest|sync|link|impact|serve>` (not `nirdosha realm
> ...`), and there is no separate `nirdosha realm` subcommand anymore —
> `hi` with no arguments opens the build-mode window directly (see RFC
> 0014's own status box). The RFC number and filename are left as-is
> per this repo's convention of not renumbering/renaming historical
> RFCs after the fact; read "Realm"/`realm` below as this graph.

> **Status.** v1 slice **`[DONE]`**: schema (`nodes`/`edges`/
> `provenance`/`chunks`+FTS5) and auto-scaffold under `.nir/`
> (`hi_graph::open`); per-item code hashing reusing the same lex/parse
> primitives `emit-ast` is built from (`hi_graph::code_units_in_file`,
> deliberately *not* `loader::load_program` — see that function's own
> doc comment for why); bounded bidirectional impact queries
> (`hi_graph::impact`, depth/node-capped, `partial: true` on exhaustion);
> manual `hi link`; a minimal `hi ingest`/FTS5 path; the
> `nirdosha hi <ingest|sync|link|impact>` CLI surface
> (`main.rs::cmd_hi`); and `hi`'s own auto-scaffold-on-startup
> (`main.rs::cmd_hi_window`, gated on `NIRDOSHA_HI_DISABLE`). All
> covered by real, passing unit tests (`hi_graph.rs`'s own `#[cfg(test)]`
> module) and exercised end-to-end by hand against a real `.nir` file
> (`nirdosha hi sync`/`link`/`impact`/`ingest` run standalone) — not
> just compiled. `hi`'s interactive console verbs, `:ask`/`:impact`
> (`hi.rs`'s `Command::Ask`/`Command::Impact`), were shipped in this
> same v1 pass and call the query functions this RFC owns directly —
> but they, and every other way a human ends up looking at the graph,
> are RFC 0014's surface to describe, not this one's: this RFC's own
> scope is the graph itself and a programmatic way to query it, full
> stop.
>
> One correction the implementation surfaced against this RFC's own
> earlier text: the "Adding `rusqlite`'s `fts5` feature" open question
> below was wrong as stated — `rusqlite 0.31` has no such cargo feature
> at all, and doesn't need one. `libsqlite3-sys`'s own bundled build
> script passes `-DSQLITE_ENABLE_FTS5` unconditionally, so the
> `"bundled"` feature this workspace already had was sufficient by
> itself. `Cargo.toml` is unchanged from before this RFC — genuinely
> zero new dependencies, confirmed rather than assumed.
>
> **Still `[OPEN]`, deliberately not built in v1**: the vector-search
> plugin boundary, the in-source annotation (v2 code↔knowledge link),
> the full adaptive resource governor, and a confirmed
> committed-vs-per-checkout answer for `.nir/hi.db` — see Open
> Questions below, each one unchanged by the implementation pass.
> `CodeUnit` qualified-name collision handling across multiple files
> (an open question below) is a real, still-unaddressed gap: the
> current implementation scopes each file's own declarations
> independently (no cross-file `use` merging, precisely to avoid
> double-counting shared imports — see the `hi_graph.rs` module doc
> comment), but two *different* files each declaring an `fn` with the
> same name collide on the same `CodeUnit` node today, silently.

## Motivation

`nirdosha hi` (RFC 0012) ships NL-to-`.nir` generation, a compiler-
feedback self-repair loop, and `:explain`. Two capabilities RFC 0012
scoped but deliberately left `[OPEN]` are the actual gap this RFC
closes:

- **Capability 5 — read-only Q&A over the current project's own
  files.** RFC 0012 named it and stopped there. The naive
  implementation — dump every file in the project into the prompt —
  doesn't scale past a handful of files and re-reads everything on
  every question. There is no retrieval layer today.
- **Capability 4 — dispatch to existing subcommands chosen by the
  model.** Partly gated on the same problem: dispatching well requires
  the console to know *what a request is actually about* first.

Separately, nothing in the project today answers two questions that
matter the moment a `.nir` codebase and its requirements/decisions
both live past a few files:

1. **Code changed — what knowledge does that touch?** If
   `transfer_funds` in `ledger.nir` changes, is there a requirement or
   decision that now needs re-review? Nothing today tracks that link,
   so the answer is "grep and hope."
2. **A requirement changed — what code does that touch?** If a PRD
   line changes from "ledger records are immutable" to "administrators
   may modify ledger entries," nothing points at
   `fn correct_entry(...)` or the `screen`s that assumed immutability.

Both gaps are the same underlying missing piece: a durable,
queryable record of *what the project's knowledge is* and *which
`.nir` code units realize which piece of it*, kept current as both
sides change. `docs/nirdosha-agent-api.md` already specs the
content-addressing half of this (`source_hash`/`ast_hash`, E1)
without a consumer; `nirdosha emit-ast` already produces the
structured AST such a system would parse (`crates/compiler/src/
main.rs:450`, shipped). Nirdosha Hi's knowledge graph is the piece that connects
them: a bounded, local-first knowledge graph, keyed by content hash,
with `.nir` code units as first-class nodes in the same graph as
requirements and decisions — not a bolt-on RAG index over raw text.

## Design

### Non-negotiable framing, stated up front

Every design choice below follows from one constraint this project
already enforces everywhere else it touches external resources
(RFC 0007's admission kernel, RFC 0011's pooling/reaper, RFC 0012's
"same 'no dependency this repo doesn't already need elsewhere'
posture," `crates/presence-gateway/src/main.rs:1-8`): **this graph must run
on an ordinary developer laptop with zero mandatory external
services, and every expensive operation must be bounded.** No
Neo4j, no Elasticsearch, no Kafka, no vector database, no embedding
model, no mandatory LLM call. Those are optional accelerators a
project can add later through a plugin boundary (mirroring RFC 0011's
`Domain`/provider pattern), never load-bearing for correctness.

Five non-negotiable principles, referenced by name throughout:

1. **Source is immutable.** Ingested requirement/decision text is
   never rewritten in place — only superseded, with the supersession
   recorded as a new node and an edge, never a mutation.
2. **The graph is the knowledge backbone.** FTS and (later, optional)
   vector search are indexes *into* the graph, not a parallel source
   of truth.
3. **Explicit and inferred knowledge stay separate.** Something this graph
   infers (a `possibly_stale` flag, a suggested link) is tagged as
   inferred and never silently promoted to authoritative. This is
   also why the graph never auto-edits `.nir` source or requirement text
   on a detected drift (see Rejected alternatives) — it flags, a
   human or an explicit `hi` action decides.
4. **Every expensive operation is bounded.** Graph traversal, FTS
   candidate sets, and ingestion batches all take an explicit
   `max_depth`/`max_nodes`/`max_time` and return `partial: true` on
   exhaustion rather than degrading unboundedly.
5. **Local-first, scale-out by plugin, never by requirement.**

### Physical layout, and when it gets created

```text
my-project/
├── .nir/
│   ├── hi.db           # SQLite (rusqlite, "bundled" feature —
│   │                       # already a workspace dependency:
│   │                       # crates/compiler/Cargo.toml:35,
│   │                       # crates/runtime-kernels/Cargo.toml:111)
│   └── content/            # sha256-addressed blobs (source docs,
│                            # not .nir source — see "What's ingested")
├── src/*.nir
└── nirdosha.toml
```

`.nir/` is scaffolded automatically — not via a separate `init`-style
step a user has to remember to run. **Every successful `nirdosha hi`
invocation** (i.e. after RFC 0012's activation contract resolves
credentials and is about to enter the console loop — never on a
failed activation, which shouldn't leave stray directories behind for
someone who was only checking whether `hi` was configured) does, in
order:

1. If `.nir/` doesn't exist under the current working directory,
   create it (`.nir/hi.db`, `.nir/content/`).
2. Run the code half of `hi sync` (below) over every `.nir` file
   in the project, incrementally — content-addressing means a
   second, third, hundredth invocation with no code changes touches
   zero rows and costs a stat + hash comparison per file, not a
   re-parse.
3. Enter the console loop as today, unaffected otherwise.

This means the `CodeUnit` graph is always current the moment a
question or a generation request can be asked, with no separate
"remember to sync" step — the same "don't make the user remember to
maintain it" property that made the "recorded by the tool" choice
right for code↔knowledge links (below). Document ingestion
(`hi ingest`) stays explicit and out of this auto-scaffold: `hi`
has no way to guess which of a project's Markdown files are
requirements/decisions worth chunking and indexing versus a README or
a changelog, so that step is opt-in, never inferred from file
presence alone.

Zero new dependencies for the local default: `rusqlite` (bundled) is
already vendored and linked into `crates/compiler`. This is a strictly
smaller ask than RFC 0011's HTTP client story — this graph's SQLite handle
is in-process, host-side tooling exactly like `nirdosha hi`'s bespoke
LLM client (RFC 0012's "Provider-client fork point"), not a pooled
service connection, so RFC 0011's `PoolRegistry`/reaper machinery
(built for the *compiled program's* runtime, in the separate
`runtime-kernels` workspace) does not apply here for the same
cross-workspace reason `hi` itself couldn't reuse it. Confirmed against
the actual vendored build (`libsqlite3-sys 0.28.0`, per `Cargo.lock`,
via the workspace's existing `rusqlite = { version = "0.31", features =
["bundled"] }`): its bundled build script passes
`-DSQLITE_ENABLE_FTS5` unconditionally, so `"bundled"` alone is
already enough — no separate `rusqlite` cargo feature exists at this
version to add. `Cargo.toml` needed no change for FTS5 at all.

### Schema (v1 — deliberately small)

```text
Node kinds:   Source, Document, Chunk, CodeUnit, Requirement,
              Decision, Actor, Capability, Test, Job
Edge kinds:   CONTAINS, DERIVED_FROM, REFERENCES, SUPPORTS,
              CONTRADICTS, SUPERSEDES, IMPLEMENTS, IMPLEMENTED_BY,
              VERIFIES, CONSTRAINS
```

Nodes carry only small scalar fields (`id`, `kind`, `title`, `status`,
`content_hash`, `source_ref`) — never a large embedded JSON blob per
node. Provenance (`created_by`, `model`, `prompt`, `confidence`,
`timestamp`) lives in a separate `provenance` table keyed by edge id,
fetched only when asked, not carried on every edge. This keeps
`hi.db` itself small even on a large project — the actual document
and code bytes never live in SQLite rows.

### What's ingested, and how each side gets a content hash

**Requirements/decisions/documents** (PRDs, ADRs, design notes) are
ingested via a new `Chunk` pipeline: `sha256(content)` per chunk,
same identity-independent-of-document-version rule the RFC's own
prior art already uses for provenance (`docs/nirdosha-agent-api.md`
E1's `source_hash`/`ast_hash` split — this RFC generalizes that same
two-hash idea from "one compiled program" to "every chunk of every
ingested document"). Unchanged chunks between document versions are
never re-parsed, re-extracted, or re-embedded — only the diff is.

**`.nir` code** does *not* get a new parser, a new hash function, or
a new grammar production. It reuses `nirdosha emit-ast` verbatim
(`crates/compiler/src/main.rs:450`, shipped, unchanged):

1. `hi sync` calls the same AST-serialization path `emit-ast`
   already exposes (as a library call, not a subprocess — `cmd_emit_ast`
   already separates "load the AST" from "print it").
2. Walk the JSON AST's top-level items (`Function`, `Struct`, `Enum`,
   `Screen`/UI declarations — whatever the file actually declares at
   module scope).
3. For each item, `sha256` its own JSON subtree. This is exactly the
   `ast_hash` `docs/nirdosha-agent-api.md` already specs at the
   whole-program level, applied per top-level item instead. Because
   it's computed from the parsed AST, not source bytes, it is
   whitespace/comment-insensitive by construction — the same property
   E1 already documents.
4. `CodeUnit` node identity is `(qualified_name, kind)` — a
   module-qualified `fn`/`struct`/`screen` name, deliberately *not*
   line/col (which shifts on unrelated edits) and *not* a hash (which
   changes on every edit) — the same "chunk identity independent of
   version" rule as the document side. `content_hash` (the per-item
   `ast_hash`) is the *version* of that identity, stored as history,
   not the identity itself.

This means the entire code-ingestion half of this feature is new glue code
around an existing, shipped compiler entry point — no compiler-side
work, no grammar change, no new parsing surface to keep correct.

### How a code↔knowledge link actually gets created

This is the crux of "deep integration," and the RFC deliberately
picks the lowest-risk of two options:

- **v1 (this RFC): recorded by the tool, not the grammar.** When
  `nirdosha hi`'s existing NL→`.nir` generation flow (RFC 0012,
  `[DONE]`) is invoked *from* a Requirement/Decision node — e.g. `hi`
  is asked "implement R17" rather than a bare prompt — it already
  knows, in-process, which Requirement/Decision motivated the
  generated code. `hi` records `IMPLEMENTS`/`IMPLEMENTED_BY` edges
  between the new `CodeUnit`(s) and that node at the moment of
  generation, tagged with the generation's own provenance (model,
  prompt, timestamp — the same fields RFC 0012's `hi::Activation`
  already redacts carefully in `Debug`). No grammar change, no new
  `.nir` syntax, zero compatibility risk — purely additive metadata
  that lives in `.nir/hi.db`, never in the `.nir` *source* file
  itself.
- **v2 (explicitly deferred, not designed here): an in-source
  annotation** (a doc-comment convention or a new attribute
  production) so links survive code written outside `hi` entirely —
  hand-written or migrated. Left open deliberately: this project's
  grammar is hand-rolled LL(1) by explicit design choice
  (`docs/GRAMMAR.md` row 7's "verified unambiguous" bar), and a new
  production is exactly the kind of change the RFC template's own
  Design section says "grammar changes... belong here, not
  hand-waved" — it deserves its own RFC once the metadata-only
  approach in v1 has real usage to learn from, not a grammar addition
  speculatively bundled into this one.
- A manual link (`nirdosha hi link R17 ledger::transfer_funds`) is
  in scope for v1 regardless of generation path, for code that
  predates this graph or was written by hand — see CLI surface below.

### Bidirectional impact, concretely

Both directions are the same bounded graph walk, just starting from a
different node kind, and both only ever *flag*, never rewrite (per
principle 3 above):

**Code → knowledge** (`hi sync`'s code half — run automatically on
every `hi` startup per "Physical layout" above, or standalone via
`nirdosha hi sync` for CI/non-interactive use):

1. Re-run the per-item `ast_hash` walk above for changed files.
2. For each `CodeUnit` whose `ast_hash` no longer matches the hash
   recorded on its `IMPLEMENTS` edge, walk that edge to the
   Requirement/Decision node(s) it implements and set
   `possibly_stale = true` on the node (a flag with a `reason` and the
   old/new hash, not a deletion or a rewrite).
3. `nirdosha hi :impact ledger::transfer_funds` (or
   `nirdosha hi impact ledger::transfer_funds` non-interactively,
   for CI) prints every Requirement/Decision/Test reachable from that
   `CodeUnit`, bounded (`max_depth`, default 5; `max_nodes`, default
   500 — principle 4), with `possibly_stale` nodes called out first.

**Requirement → code** (the reverse walk, triggered by re-ingesting a
changed document — new chunk `content_hash` under the same
`Requirement.source_ref`):

1. The changed Requirement/Decision node's `IMPLEMENTED_BY` edges are
   walked to every `CodeUnit` that implements it, transitively bounded
   the same way.
2. Each reachable `CodeUnit` is flagged `impacted_by_requirement_change`
   (same flag-not-rewrite rule).
3. `nirdosha hi :impact R17` prints the same report from the other
   end: every `CodeUnit`/`Test` this requirement change touches.

Both directions are read-only with respect to `.nir` source and
requirement text — they only ever write flags into `.nir/hi.db`.
Fixing
the drift (editing code, editing the requirement, or explicitly
clearing the flag once reviewed) stays a human or an explicit,
separate `hi` action, never something `sync`/`impact` does on its own.
This is the direct, load-bearing application of principle 3 — a
`possibly_stale` flag is the graph's inference; the requirement text and
the code stay exactly as a human or a prior `hi` generation left them
until someone acts on the flag.

### Retrieval (serves RFC 0012 capability 5)

```text
query → exact lookup (id/hash/qualified-name) → hit? return
      → graph scope (bounded traversal from any named
        Requirement/Decision/CodeUnit in the query)
      → FTS5 over chunks in scope
      → (optional, v2+) vector search in scope, run concurrently
        with FTS, fused by reciprocal-rank
      → bounded result set → nirdosha hi's existing prompt assembly
```

FTS-only (no vector search, no embeddings, no LLM call) is the v1
floor — it must answer capability-5 questions ("what does R17 say,"
"what implements ledger immutability") without any optional piece
present. Vector search is an explicit, later, optional plugin behind
the same kind of provider boundary RFC 0011 opened for `db`/`mq`/
`http`/`call` — not designed in this RFC, just kept structurally
possible by not baking FTS-only assumptions into the graph schema.

### CLI surface

```text
nirdosha hi ingest <doc.md>        content-address, chunk, FTS-index
                                        a requirement/decision/design doc
nirdosha hi sync [<file.nir> ...]   the same code-sync step `hi` runs
                                        automatically on startup, exposed
                                        standalone (CI/non-interactive;
                                        does NOT scaffold .nir/ document
                                        ingestion, only the code half)
nirdosha hi link <req-id> <qualified-name>
                                        record a manual IMPLEMENTS edge
nirdosha hi impact <target>         non-interactive impact report
                                        (CI-friendly; target is a
                                        requirement/decision id or a
                                        qualified CodeUnit name)
```

This is the whole of this RFC's own UI surface — a scriptable,
non-interactive CLI, deliberately: its remit is the graph itself
and a programmatic way to query it, not how a human ends up looking at
the results. `hi`'s own interactive console verbs (`:ask`/`:impact`,
`hi.rs`'s `Command::Ask`/`Command::Impact`) call the exact same
`hi_graph::ask`/`hi_graph::impact` functions this CLI does — but describing
that console surface, and every richer way of viewing the same query
results (RFC 0014's 3D view included), belongs to RFC 0014.

`hi sync`/`hi impact` are ordinary batch subcommands, following
the same per-command arg-loop dispatch style as `init`/`build`
(`crates/compiler/src/main.rs`'s `match first.as_str()`) — no new CLI
framework, consistent with RFC 0012's explicit choice not to introduce
one. `nirdosha hi sync` as a standalone subcommand exists mainly
for CI (a merge/PR check can run `nirdosha hi sync && nirdosha
hi impact <target>` without ever entering the interactive console)
— for everyday use the point of "Physical layout"'s auto-scaffold is
that a person never needs to type `hi sync` themselves.

One escape hatch, following the project's existing `NIRDOSHA_`-
prefixed, `.ok()`-based env-var convention (`crates/runtime-kernels/
src/kernel/nfr.rs:183`, `pool.rs:101`, and RFC 0012's own trio):
`NIRDOSHA_HI_DISABLE=1` skips both the auto-scaffold and the
auto-sync step on `hi` startup entirely — for a project that never
wants a `.nir/` directory materializing on disk, or a CI image running
`hi` read-only.

### Bounding and resource posture

Every traversal in "Bidirectional impact" and "Retrieval" above takes
explicit `max_depth`/`max_nodes`/`max_time_ms` and returns
`partial: true` rather than running unbounded — the same posture RFC
0007 established for compiled-program resource control (boundary-
leased admission, fail-open telemetry), applied here to host-tooling
graph queries instead of runtime kernel calls. v1 ships one profile
(conservative defaults: depth 5, 500 nodes, single-threaded ingestion)
rather than the full adaptive resource-governor described in early
drafts of this proposal — that's real scope, deliberately deferred
(see Rejected alternatives) until there's a real project large enough
to need it.

## Effect on the permission model

None to the compiled `.nir` program's permission model —
`requires(role/claim: ...)`, `acquire`, a `screen`'s view/edit gates,
and `serve.rs`'s server-side enforcement are all unaffected, exactly
as RFC 0012 states for `hi` itself: this is host-tooling, and
`hi sync`/`impact`/`:ask` touch nothing a compiled program's
runtime checks.

One nuance worth naming rather than omitting: the graph's `CodeUnit`
ingestion reads `requires(role: ...)`/`requires(claim: ...)`
annotations already present in `.nir` source as *data* — e.g. to
populate `Actor`/`Capability` nodes and `CONSTRAINS` edges so a
question like "what implements the admin-only ledger correction
capability" is answerable — but the graph never adds, removes, or
reinterprets what those annotations enforce at runtime. Reading a
`requires(role: "admin")` clause into the graph is not the same as
checking it; `serve.rs` and the codegen'd enforcement path remain the
only place that decision is made.

## Compatibility

Additive to every existing subcommand: no existing `.nir` program,
`build`, `emit-ast`, or CI invocation of those changes behavior,
because nothing in `nirdosha` today reads or depends on `.nir/`'s
(the directory's) existence, and `emit-ast` itself is called exactly
as it already exists — this RFC adds a caller, not a new output
shape, so its existing `--emit-ast` CLI behavior and
`docs/nirdosha-agent-api.md`'s C4 spec are both unchanged.

**`hi` itself is the one exception, and it's worth being honest about
rather than filing under "purely additive."** Before this RFC, running
`nirdosha hi` had no filesystem side effect beyond the console session
itself. After this RFC, a successful activation now creates `.nir/` on
disk the first time it runs in a project — a real, visible change to
what invoking `hi` does, gated only by the `NIRDOSHA_HI_DISABLE`
escape hatch above. This is called out explicitly rather than folded
into "purely additive" language, because a new default write-on-
startup is exactly the kind of thing RFC 0012's own template bar
("does an existing program's behavior change? ... say why") asks to
be named, not assumed harmless.

## Rejected alternatives

- **Any mandatory external service** (Neo4j, Elasticsearch, Kafka,
  Redis, a vector DB, a mandatory embedding model or LLM call). Every
  one of these was considered and rejected for the same reason this
  RFC opens with: they'd make this graph unusable on an ordinary laptop and
  contradict the "no dependency this repo doesn't already need"
  posture RFC 0012 already committed to for `hi`. Each stays available
  later as an optional plugin behind a provider boundary shaped like
  RFC 0011's, never a v1 requirement.
- **A grammar-level `.nir` annotation for code↔requirement links, in
  v1.** Considered and deferred, not rejected outright — see "How a
  code↔knowledge link actually gets created" above. Shipping a new
  production before the metadata-only approach has real usage risks
  designing the wrong syntax for a problem not yet well-understood
  from experience.
- **Auto-rewriting requirement text or `.nir` source when drift is
  detected.** Rejected on principle 3 (explicit vs. inferred knowledge
  must stay separate) — an agent silently "fixing" a requirement to
  match code it just changed (or vice versa) destroys the exact
  signal `:impact` exists to surface. Flagging and stopping is the
  entire value of the feature; auto-resolution would remove it.
- **A full adaptive resource governor (CPU/RAM-sensing profile
  switching) in v1.** Real scope from the architecture this RFC is
  based on, but no project in this repo is yet large enough to need
  more than one conservative default profile — premature to build
  ahead of a concrete case that hits the current bounds. Left as a
  named follow-up, not silently dropped.
- **Event-sourcing the entire graph** (every state transition as an
  append-only event, full CQRS). Considered for the provenance layer
  specifically and adopted there in a narrow form (supersession is
  append-only, per principle 1); rejected as the *general* storage
  model because it adds overhead to every write for a guarantee only
  the requirement/decision lifecycle actually needs.

## Open questions

- ~~Adding `rusqlite`'s `"fts5"` feature.~~ **Resolved: not needed.**
  See the status block at the top — the bundled build already compiles
  FTS5 in unconditionally.
- Exact `CodeUnit` qualified-name scheme across multiple `.nir` files/
  modules once `use "..."` (loader-resolved imports,
  `crates/compiler/src/main.rs:455`'s own comment on `loader::
  load_program`) is in play — collision handling isn't designed here.
- Whether `.nir/hi.db` is meant to be committed to version
  control (so a team shares one traceability graph) or is per-checkout
  local state (so every clone re-ingests) — this changes whether
  `content/` blobs need dedup-friendly `.gitattributes` handling.
  Leans toward committed, given the whole point is *shared*
  traceability, but not decided here.
- Concrete default values for `max_depth`/`max_nodes`/`max_time_ms` —
  the numbers above are placeholders pending a real project exercising
  them, not benchmarked.
- Whether `hi link`/the `hi`-recorded `IMPLEMENTS` edge needs its
  own confirmation step before being treated as authoritative (echoing
  RFC 0012's still-open host-level-trust-boundary question for
  capability 4 — "dispatch to subcommands chosen by the model") — an
  agent-recorded link is itself a claim, not automatically ground
  truth.
- The vector-search plugin boundary's actual shape (provider trait,
  `Domain` reuse or a new one) — intentionally left to whichever RFC
  proposes the first real vector-store plugin, not pre-designed here.
- **`.nir/` the hidden directory vs. `.nir` the source-file
  extension.** Named this way deliberately (parallels `.git/` sitting
  next to the files it tracks), but it does mean `ls -a` in a project
  root shows both `.nir/` and `ledger.nir` side by side, and any doc
  or tooling that greps for "`.nir`" to mean "a Nirdosha source file"
  now needs to be a little more careful. Not reconsidered here since
  it was requested as the concrete directory name for this RFC, but
  worth a second look if that confusion turns out to bite in practice.
- Whether the auto-scaffold-on-`hi`-startup step should also print
  something the first time it creates `.nir/` in a project (so the
  new on-disk artifact isn't a silent surprise the first time), or
  stay silent every time the way a fast no-op sync does on every
  subsequent run.
