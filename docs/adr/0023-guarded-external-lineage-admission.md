# 0023: Guarded external lineage admission — separate quarantine store, structural exclusion, explicit audited promotion

Date: 2026-09-21
Status: accepted

## Context

`Authority::ExternalClaimed` already existed as a real enum variant in
`nirdosha-lineage` — the classification was declared, but there was no
admission API and no quarantine state; an external claim had nowhere to
enter the graph.

## Decision

**New module `nirdosha-lineage::external_admission`**, built directly
against `nirdosha-guard-core`/`nirdosha-audit` (both already real
dependencies of `nirdosha-lineage`) rather than depending on
`nirdosha-guard-mic`'s `GuardClient` — `nirdosha-guard-mic` itself depends
on `nirdosha-lineage`, so the reverse dependency would be circular.
`link_external_lineage` is guarded the same way `GuardClient::guarded_apply`
is: evaluated against real policies via the same `evaluator::evaluate`
every other guard client uses, and every attempt (allowed or denied) is
audited before anything is admitted.

**Quarantine is a genuinely separate store, not a flag on a trusted-graph
row.** A claim `link_external_lineage` admits is written to a
`QuarantineStore`, never directly to a `GraphStore`. This makes "a
quarantined claim can't silently pass as verified lineage" true by
construction rather than by a filter every query has to remember to
apply: `provenance_of`/`downstream_of`/`upstream_of`-style queries read
from `GraphStore`, and a quarantined claim simply isn't there yet.
`promote` is the one, explicit, audited action that moves a claim from
quarantine into the real store — proven end to end against the real,
working `nirdosha-lineage-store-embedded` driver (`GraphStore::upsert_edge`/
`get_edges` are real and tested today), rather than waiting on
`nirdosha-lineage`'s own separate, larger, not-yet-built Phase 2 query DSL
(`query.rs`'s own doc comment: "no query actually runs here yet" —
`lineage_query_plan`/`LineageQueryRunner` are still contract-only types).

**`Authority` is always forced, never taken from the caller's claim.**
`ExternalLineageClaim` deliberately carries no `authority` field of its
own — `link_external_lineage` always stamps the resulting edge
`Authority::ExternalClaimed`. The entire point of this classification is
the kernel's own judgment about a claim's trust level, not the claimant's
word for it.

**Promotion is its own, independently-gated decision**, not something
admission approval also grants — `promote` runs its own real
`evaluator::evaluate` against the *approver's* context, separate from the
subject that originally submitted the claim, mirroring the "V10
issued-unconsumed" finding pattern's own posture (`examples/rtm`'s Part 6:
dormant/pending states need an explicit, auditable transition, not an
implicit one). Fails closed on: an unknown claim id, a claim that isn't
`Pending` (no double-promotion, no resurrecting a rejected claim), a
denied approval decision, or a real `GraphStore` write failure (the claim
stays `Pending`, not silently marked promoted on a failed write).

**Test split across two crates, for a real reason found while building
this.** `nirdosha-lineage`'s own test module uses a minimal local
`TestGraphStore` mock; the proof against the real
`nirdosha-lineage-store-embedded` driver lives in that crate's own new
`tests/external_admission.rs`. A dev-dependency from `nirdosha-lineage` on
`nirdosha-lineage-store-embedded` was tried first and reverted — confirmed
by direct compilation to create a Cargo dev-dependency cycle that
duplicates `nirdosha-lineage`'s own type identity across two separate
builds (`EmbeddedGraphStore`'s `impl GraphStore` binds to one `nirdosha_lineage`
crate instance, the test-under-compilation's `GraphStore` trait to
another), breaking the trait bound entirely. `nirdosha-lineage-store-embedded`
already depends on `nirdosha-lineage` normally, so hosting the real-driver
test there has no such cycle.

## Consequences

**What this makes possible.** `cargo test -p nirdosha-lineage
external_admission` proves the admission/promotion state machine itself:
a claim is denied without a real matching policy (never reaching
quarantine), refused for empty evidence even when otherwise allowed,
admitted with `Authority::ExternalClaimed` always forced regardless of
what the claim would imply, and promotion independently fails closed on a
missing approver policy, a not-pending claim, or (proven against the real
embedded driver in `nirdosha-lineage-store-embedded`'s own test) never
reaching the store at all on a denied promotion. The real-driver test
additionally proves the structural-exclusion property directly: a real
`get_edges` query against the real store returns nothing for a
quarantined-but-unpromoted claim, and returns exactly the promoted edge —
with its forced `Authority::ExternalClaimed`, correct `src`/`dst` — once
promoted.

**What this does *not* make possible, stated so it's never misread later.**
No `link_external_lineage!`/`promote_external_lineage!` macro or catalog
registration exists — this is a real, callable Rust API, not something a
`guard_policy!`-adjacent DSL block can declare yet. `nirdosha-lineage`'s
own Phase 2 query engine (`lineage_query_plan`/`LineageQueryRunner`,
RFC 0026 §9) still doesn't exist — this phase's structural-exclusion
property holds for the real `GraphStore::get_edges` today, and will
continue to hold once that query engine is built on top of the same
`GraphStore`, but this phase doesn't build that engine itself.
`QuarantineStore` has no persistent driver of its own yet (only
`InMemoryQuarantineStore`) — a real deployment needing quarantine state
to survive a restart would need one, the same "one honest implementation,
trait stays open" pattern this session's other new drivers already
follow.
