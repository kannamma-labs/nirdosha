# 0014: `ListProvider`/`Matcher` for screening — fixture-backed driver, digest-pinned integrity, real Jaro-Winkler

Date: 2026-09-21
Status: accepted

## Context

`rfcs/0025-nirdosha-rtm-ecosystem.md` §8.4 declares real trait shapes for
sanctions/PEP screening — `ListProvider::fetch(list_id) -> SignedList` and
`Matcher::screen(name, lists, cfg) -> Vec<Hit>` — and the `matcher!` macro's
declared defaults (`algorithm = fuzzy_jaro_winkler; threshold = 0.92`,
`lists = [ofac_sdn, un_consolidated, eu_fsf]`). Neither trait had an
implementation anywhere in the workspace before this: the RFC's own code
block is illustrative (`OsanctionsProvider fetches and verifies signed
snapshots` — no such type exists). `matcher!` itself is not yet a callable
macro (a grammar mismatch already flagged separately in
`crates/nirdosha-rt/tests/rtm_policy_corpus.rs`, out of this phase's scope)
— this ADR is about the driver side of the port, not the macro front-end.

This workspace has no live OFAC/UN/EU vendor feed credentials, and Plan
Phase 13's own scope note (in the approved execution plan) commits to
building each missing driver "to the same standard the repo already uses
for Postgres" rather than inventing fake vendor credentials.

## Decision

**New crate `crates/nirdosha-screening-list-fixture`**, following the
established `nirdosha-<domain>-<subject>-<vendor>` naming
(`docs/adr/0013-postgres-store-driver-pooling-and-rls.md`'s own precedent).

**`FixtureListProvider`**: reads `<root>/<list_id>.json` (a small,
synthetic test list — `tests/fixtures/test_sanctions_list.json`, fictional
names only, not real sanctions data) and verifies its SHA-256 digest
against a pinned `<root>/<list_id>.json.sha256` sidecar before returning
it, failing closed (`FetchError::IntegrityMismatch`) on drift.

**"Signed" is honestly scoped to content-addressed integrity, not a vendor
signature.** RFC 0025's `SignedList` name implies cryptographic vendor
signing this workspace has no vendor keypair to check. Inventing a fake
signature scheme would be exactly the kind of aspirational-not-honest claim
this repo's own convention (`docker-compose.dev.yml`'s image pinned by
digest, not a floating tag) exists to avoid. `FixtureListProvider` applies
that same "pinned by digest" pattern to a list file instead of a container
image. A real vendor driver implementing `ListProvider` with an actual
cryptographic signature check is a strict superset of this contract and can
replace `FixtureListProvider` without changing the trait or any caller.

**`FuzzyMatcher` implements real Jaro-Winkler similarity directly** (not a
stub returning a fixed score, and not a new external string-distance
dependency) — the algorithm is small, fixed, and worth keeping inline and
auditable given how close it sits to a compliance decision. Reference-
checked against the standard `"martha"`/`"marhta"` ≈ 0.961 test vector.
Screening is case-insensitive and checks every entry's aliases, not just
its primary name, keeping the single best-scoring match per entry.

## Consequences

**What this makes possible.** `cargo test -p nirdosha-screening-list-fixture`
proves, without any vendor credentials: a list loads and its integrity is
checked before use; a tampered/stale list is refused, not silently served;
and name screening finds real fuzzy matches above threshold while excluding
unrelated names, using the exact algorithm `matcher!`'s declared default
names.

**What this does *not* make possible, stated so it's never misread later.**
`matcher!` is still not a callable macro — a `guard_policy!`-declared
screening check has nothing to wire this driver into yet; that is the
separate, already-flagged grammar-mismatch gap, not something this ADR
closes. No live vendor list (OFAC SDN, UN Consolidated, EU FSF) is fetched
by anything here — `FixtureListProvider` is the one honest implementation
this workspace can produce; a real vendor `ListProvider` is a distinct
future driver satisfying the same trait, the same "vendor swap" pattern the
RFC itself describes (§8.4: "New `ListProvider` driver; `matcher!` block
unchanged").
