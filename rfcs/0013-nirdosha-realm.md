# RFC 0013: Nirdosha Realm — a local-first project knowledge graph with bidirectional `.nir` traceability

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
main.rs:450`, shipped). Nirdosha Realm is the piece that connects
them: a bounded, local-first knowledge graph, keyed by content hash,
with `.nir` code units as first-class nodes in the same graph as
requirements and decisions — not a bolt-on RAG index over raw text.

## Design

### Non-negotiable framing, stated up front

Every design choice below follows from one constraint this project
already enforces everywhere else it touches external resources
(RFC 0007's admission kernel, RFC 0011's pooling/reaper, RFC 0012's
"same 'no dependency this repo doesn't already need elsewhere'
posture," `crates/presence-gateway/src/main.rs:1-8`): **Realm must run
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
3. **Explicit and inferred knowledge stay separate.** Something Realm
   infers (a `possibly_stale` flag, a suggested link) is tagged as
   inferred and never silently promoted to authoritative. This is
   also why Realm never auto-edits `.nir` source or requirement text
   on a detected drift (see Rejected alternatives) — it flags, a
   human or an explicit `hi` action decides.
4. **Every expensive operation is bounded.** Graph traversal, FTS
   candidate sets, and ingestion batches all take an explicit
   `max_depth`/`max_nodes`/`max_time` and return `partial: true` on
   exhaustion rather than degrading unboundedly.
5. **Local-first, scale-out by plugin, never by requirement.**

### Physical layout

```text
my-project/
├── .realm/
│   ├── realm.db          # SQLite (rusqlite, "bundled" feature —
│   │                      # already a workspace dependency:
│   │                      # crates/compiler/Cargo.toml:35,
│   │                      # crates/runtime-kernels/Cargo.toml:111)
│   └── content/           # sha256-addressed blobs (source docs,
│                           # not .nir — see "What's ingested" below)
├── src/*.nir
└── nirdosha.toml
```

Zero new dependencies for the local default: `rusqlite` (bundled) is
already vendored and linked into `crates/compiler`. This is a strictly
smaller ask than RFC 0011's HTTP client story — Realm's SQLite handle
is in-process, host-side tooling exactly like `nirdosha hi`'s bespoke
LLM client (RFC 0012's "Provider-client fork point"), not a pooled
service connection, so RFC 0011's `PoolRegistry`/reaper machinery
(built for the *compiled program's* runtime, in the separate
`runtime-kernels` workspace) does not apply here for the same
cross-workspace reason `hi` itself couldn't reuse it. One real, confirmed gap: `Cargo.lock` pins `libsqlite3-sys 0.28.0`
via the workspace's existing `rusqlite = { version = "0.31", features
= ["bundled"] }`, and `"bundled"` alone does not compile SQLite with
`SQLITE_ENABLE_FTS5` — that needs `rusqlite`'s separate `"fts5"`
cargo feature, not currently enabled anywhere in this workspace. Not
a blocker (adding a feature flag to an already-vendored dependency is
cheap and adds no new dependency), but real work this RFC's own
estimate above ("zero new dependencies") should not be read to
include for free — tracked as an open question below.

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
`realm.db` itself small even on a large project — the actual document
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

1. `realm sync` calls the same AST-serialization path `emit-ast`
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

This means the entire code-ingestion half of Realm is new glue code
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
  that lives in `.realm/realm.db`, never in the `.nir` file itself.
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
- A manual link (`nirdosha realm link R17 ledger::transfer_funds`) is
  in scope for v1 regardless of generation path, for code that
  predates Realm or was written by hand — see CLI surface below.

### Bidirectional impact, concretely

Both directions are the same bounded graph walk, just starting from a
different node kind, and both only ever *flag*, never rewrite (per
principle 3 above):

**Code → knowledge** (`realm sync` after any `.nir` edit, or on
demand):

1. Re-run the per-item `ast_hash` walk above for changed files.
2. For each `CodeUnit` whose `ast_hash` no longer matches the hash
   recorded on its `IMPLEMENTS` edge, walk that edge to the
   Requirement/Decision node(s) it implements and set
   `possibly_stale = true` on the node (a flag with a `reason` and the
   old/new hash, not a deletion or a rewrite).
3. `nirdosha hi :impact ledger::transfer_funds` (or
   `nirdosha realm impact ledger::transfer_funds` non-interactively,
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

Both commands are read-only with respect to `.nir` source and
requirement text — they only ever write flags into `realm.db`. Fixing
the drift (editing code, editing the requirement, or explicitly
clearing the flag once reviewed) stays a human or an explicit,
separate `hi` action, never something `sync`/`impact` does on its own.
This is the direct, load-bearing application of principle 3 — a
`possibly_stale` flag is Realm's inference; the requirement text and
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
nirdosha realm ingest <doc.md>        content-address, chunk, FTS-index
                                        a requirement/decision/design doc
nirdosha realm sync [<file.nir> ...]   re-run emit-ast, diff ast_hash
                                        per CodeUnit, flag stale links
nirdosha realm link <req-id> <qualified-name>
                                        record a manual IMPLEMENTS edge
nirdosha realm impact <target>         non-interactive impact report
                                        (CI-friendly; target is a
                                        requirement/decision id or a
                                        qualified CodeUnit name)
```

plus two new `hi` console verbs, alongside the existing `:explain`
(RFC 0012):

```text
:ask <question>       FTS-backed Q&A over ingested requirements/
                        decisions/code (closes RFC 0012 capability 5)
:impact <target>       same report as `realm impact`, inline
```

`realm sync`/`realm impact` are ordinary batch subcommands, following
the same per-command arg-loop dispatch style as `init`/`build`
(`crates/compiler/src/main.rs`'s `match first.as_str()`) — no new CLI
framework, consistent with RFC 0012's explicit choice not to introduce
one.

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
`realm sync`/`impact`/`:ask` touch nothing a compiled program's
runtime checks.

One nuance worth naming rather than omitting: Realm's `CodeUnit`
ingestion reads `requires(role: ...)`/`requires(claim: ...)`
annotations already present in `.nir` source as *data* — e.g. to
populate `Actor`/`Capability` nodes and `CONSTRAINS` edges so a
question like "what implements the admin-only ledger correction
capability" is answerable — but Realm never adds, removes, or
reinterprets what those annotations enforce at runtime. Reading a
`requires(role: "admin")` clause into the graph is not the same as
checking it; `serve.rs` and the codegen'd enforcement path remain the
only place that decision is made.

## Compatibility

Purely additive, same bar RFC 0012 met for `hi`. `.realm/` is a new,
optional directory; no existing `.nir` program, `build`, `emit-ast`,
or CI invocation changes behavior, because nothing in `nirdosha`
today reads or depends on `.realm/`'s existence. `emit-ast` itself is
called exactly as it already exists — this RFC adds a caller, not a
new output shape, so its existing `--emit-ast` CLI behavior and
`docs/nirdosha-agent-api.md`'s C4 spec are both unchanged.

## Rejected alternatives

- **Any mandatory external service** (Neo4j, Elasticsearch, Kafka,
  Redis, a vector DB, a mandatory embedding model or LLM call). Every
  one of these was considered and rejected for the same reason this
  RFC opens with: they'd make Realm unusable on an ordinary laptop and
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
- **Event-sourcing the entire Realm** (every state transition as an
  append-only event, full CQRS). Considered for the provenance layer
  specifically and adopted there in a narrow form (supersession is
  append-only, per principle 1); rejected as the *general* storage
  model because it adds overhead to every write for a guarantee only
  the requirement/decision lifecycle actually needs.

## Open questions

- Adding `rusqlite`'s `"fts5"` feature to `crates/compiler/Cargo.toml`
  (confirmed needed, not just possibly needed — see "Physical layout"
  above) — whether it goes on the existing `rusqlite` dependency
  workspace-wide or is scoped to wherever Realm's crate/module ends up
  living.
- Exact `CodeUnit` qualified-name scheme across multiple `.nir` files/
  modules once `use "..."` (loader-resolved imports,
  `crates/compiler/src/main.rs:455`'s own comment on `loader::
  load_program`) is in play — collision handling isn't designed here.
- Whether `.realm/realm.db` is meant to be committed to version
  control (so a team shares one traceability graph) or is per-checkout
  local state (so every clone re-ingests) — this changes whether
  `content/` blobs need dedup-friendly `.gitattributes` handling.
  Leans toward committed, given the whole point is *shared*
  traceability, but not decided here.
- Concrete default values for `max_depth`/`max_nodes`/`max_time_ms` —
  the numbers above are placeholders pending a real project exercising
  them, not benchmarked.
- Whether `realm link`/the `hi`-recorded `IMPLEMENTS` edge needs its
  own confirmation step before being treated as authoritative (echoing
  RFC 0012's still-open host-level-trust-boundary question for
  capability 4 — "dispatch to subcommands chosen by the model") — an
  agent-recorded link is itself a claim, not automatically ground
  truth.
- The vector-search plugin boundary's actual shape (provider trait,
  `Domain` reuse or a new one) — intentionally left to whichever RFC
  proposes the first real vector-store plugin, not pre-designed here.
