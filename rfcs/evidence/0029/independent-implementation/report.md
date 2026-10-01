# Independent RFC 0029 / 0029.a conformance check (Ruby, from scratch)

```
Status:      Independent freeze-gate evidence; not itself a certification
Implements:  A second, from-scratch evaluator against the same 50 symbolic
             fixtures the existing Rust crate (crates/rfc0029-conformance)
             evaluates, built without reading that crate, its generator, or
             any evidence document that narrates expected results.
Location:    independent-implementations/rfc0029-ruby/
Entry point: ruby verify.rb   (Ruby 3.x, standard library only)
```

## 1. What was built and how

A self-contained Ruby project, standard-library only (`yaml`, `json`,
`digest`), with no gems and no `Gemfile`:

- `lib/jcs.rb` — a hand-written RFC 8785 (JSON Canonicalization Scheme)
  encoder: UTF-16-code-unit object-key ordering, minimal string escaping,
  integer-only number serialization (this project's fixture data is
  integers-only; RFC 8785's own I-JSON profile excludes non-interoperable
  numbers, so restricting to integers is a spec-derived choice). No
  canonical-JSON/JCS library was used or consulted.
- `lib/expand.rb` — parses `rfcs/fixtures/0029-influence-review-symbolic.yaml`
  (via Ruby's stdlib `yaml`/Psych) and expands each of the 50 symbolic
  fixtures into the canonical shape described by
  `rfcs/fixtures/0029-canonical/schema.json`, following only that schema's
  structure and the symbolic source's own `compact_grammar` /
  `lossless_expansion` / `defaults` header.
- `lib/model_graph.rb` — a plain local graph view (nodes/edges) used by the
  evaluators below.
- `lib/influence.rb` — the model-influence tier evaluator: the
  representation-invariant fold from RFC 0029.a's "Model Influence and Human
  Review Semantics Revision 1" §§2–4, the closed edge-contribution table,
  the §2.4 consequential-path-completeness check, and the §6 `ObserveOnly`
  certificate-validity gate.
- `lib/review.rb` — the review-contract evaluator: §7's declared-mode,
  conjunctive-predicate, `Deny > Indeterminate > Escalate` semantics, plus a
  self-certification detector.
- `lib/fact_provenance.rb` — an evaluator, deliberately separate from
  `influence.rb` per the symbolic source's own instruction ("checked … by a
  dedicated evaluator, never by compute_influence"), for the
  `fact_requirement`/`fact_provenance` lineage checks (L8–L13).
- `lib/admission.rb` — combines the three evaluators above into an
  admission/capability verdict (`MandatoryDeny`, `InfluenceLevelExceedsProfile`,
  `ReviewSelfCertification`, `ReviewProtocolFailed`, the four `Fact*`
  diagnostics, or `accepted` + capability issuance).
- `lib/expected_shape.rb` — encodes each fixture's own shorthand `expected:`
  block into the strict schema shape (used later only as the oracle
  encoding, never consulted while designing the evaluators above).
- `verify.rb` — runs all 50 fixtures, independently computes canonical
  bytes/hashes and semantic verdicts, and *only then* reads
  `rfcs/fixtures/0029-canonical/{manifest.json,vectors/*.json,jcs/*.jcs.json}`
  to compare.

Every evaluator was derived from `rfcs/0029-domain-neutral-policy-enforcement.md`,
`rfcs/0029.a-model-assurance-port.md`, and cross-fixture pattern study of the
symbolic source's 50 inputs+expected blocks (explicitly licensed by the task
brief for resolving underdetermined rules) — never by reading
`crates/rfc0029-conformance/`, `generate.py`, or the evidence/tabletop prose.

## 2. Canonicalization results

**28/50 fixtures produce byte-identical RFC 8785 JCS output and an identical
SHA-256 hash to the existing corpus's `manifest.json`/`jcs/*.jcs.json`.**

Match list (28): `B2_STALE, EQ1A, EQ1B, EQ2A, EQ2B, EQ3A, EQ3B, EQ4A, EQ4B,
H3, I2, INV_EXTERNAL_UNKNOWN, INV_PRECOMMIT, K1, K1_STRUCTURAL_ONLY, K3, L5,
L6, L7, R1_ACK, R2_ACK_FOR_APPROVAL, R3_NOT_ACCESSED, R4_CONFLICT, R5_PASS,
R6_STALE, R7_CAPACITY, R8_MONITOR_MISSING, R9_EVALUATION_MISSING`.

Non-match list (22): `B1, H1, H1_ORDER_REUSE, H2, I1, I3, I3_PAYMENT_REUSE,
K4, L1, L2, L3, L4, L8, L9, L10, L11, L12, L13, M1, M2, M3, M4`.

This did not come for free — the first pass matched 0/50. Getting to 28/50
required several rounds of comparing my own independently-derived structure
against the oracle at the very end (never before deriving it) and fixing
real bugs in my own encoding, most notably:

- `nodes`/`edges` arrays are sorted by `kind` ascending (RFC 8785 orders
  object keys, not array elements, so this ordering is the existing corpus's
  own convention, not something JCS dictates) — confirmed against `B1`,
  `L1`, `EQ1A`, `EQ2A`.
- The four equivalence-group `normalized_graph.value` blocks
  (EQ1–EQ4) are the group's own **bare compact tokens verbatim**
  (`{"nodes": ["m:ModelInvocation", ...], "edges": ["m-Recommend-d"]}`), not
  expanded node/edge objects. Recomputing SHA-256 of that exact value
  independently produced
  `sha256:3337df3702a67458de9e95c232e09a927ca90214586bee05b6f800f6e9ab030e`
  for group `g-recommend-1` — an exact byte-for-byte match with the oracle's
  own stated hash for that block. This is a clean, direct confirmation that
  the hand-written JCS encoder in `lib/jcs.rb` is correct, independent of any
  other structural guess.
- A `Review` node's attributes are just `{"review_contract": <catalog key or
  null>}`; an `AuthorityAssertion` node's attributes are the fixture's own
  `assertion:` block copied verbatim (local ids, not stable ids), or just
  `{"id": <local id>}` when no `assertion:` block exists at all.
- `capability.status`/`invalidation`/`review.status` (in the *oracle's own*
  `expected` encoding) default to `"not_evaluated"` when the fixture's
  shorthand omits the key — not the symbolic source's own literal
  `defaults:` header values (`not_issued`/`none`/`not_required`), which
  apply only once the key is actually written. This is a real inconsistency
  between the symbolic source's stated defaults and what every fixture
  actually resolves to; see §4, "ambiguous specification" / "schema defect".
- The `evaluation_fixture` catalog indirection (e.g. `rc-independent-pass` →
  `R5_PASS`'s evaluations) is followed into `input.parameters.evaluations`
  only when the contract was reached via an `AuthorityAssertion`'s
  `contract:` field, not via a bare `review_contract:` field with no
  assertion at all (`K3`/`L6`, whose `self-review` contract also names an
  `evaluation_fixture` but whose oracle input carries no `evaluations` key).

**Remaining 22 disagreements, fully diagnosed, not merely unexplained:**

- **21 of the 22** (every non-match except `M4`) differ from the oracle in
  exactly one place: the oracle's `input.parameters` carries an
  `admission_policy: {effect_class, maximum_model_level}` object that has
  **no textual basis anywhere in the symbolic YAML source** — not in any
  fixture's own fields, not in `compact_grammar`/`lossless_expansion`, not in
  `defaults`. Every other byte of these 21 fixtures' canonical encoding
  (nodes, edges, attributes, catalog entries, every other parameter, and the
  entire `expected` block including all semantic fields) matches the oracle
  exactly. Reproducing `admission_policy` correctly would require guessing
  or reverse-engineering a per-effect-class tier-cap table from the oracle
  output itself, which the task's independence rule forbids ("never
  consulting [the oracle] to reverse-engineer your algorithm before you've
  derived it yourself from the spec"); its only textual source appears to be
  the excluded `generate.py`. **Classification: schema defect / ambiguous
  specification** — the symbolic source calls itself "lossless_expansion"
  but is not actually self-contained: at least one piece of canonical data
  (the per-effect-class model-authority ceiling that RFC 0029.a §13.2
  requires to exist) is missing from it entirely. This is a real freeze
  blocker for the symbolic source's own completeness claim, independent of
  whether my semantic verdicts happen to agree with the oracle's on this
  corpus's 50 cases (they do — see §3).
- **`M4`** differs in one place: `catalog_entries`. The fixture's own
  `expected.final_authority` shorthand is the literal string `"supervisor"`,
  which is not a key in `authority_catalog` (the closest real entry is
  `maintenance-supervisor`). My independently-derived `final_authority`
  computation (profile → reviewer-authority mapping, §3) correctly resolves
  to `maintenance-supervisor` and adds that authority's catalog entry. The
  oracle's own generated input, however, has **no authority catalog entry
  at all** for `M4` — consistent with its generator also being unable to
  resolve the literal (broken) key `"supervisor"`. Both implementations
  agree the input is broken; they simply fail differently.
  **Classification: non-canonical input** (the fixture's own data, not
  either implementation).

## 3. Semantic results

Comparing my independently computed `provenance` / `model_level` /
`model_authorized` / `final_authority` / `admission.{status,diagnostic}` /
`review.{status,detail}` / `capability.{status,effect_class}` /
`invalidation` against each fixture's own `expected` block (read from
`rfcs/fixtures/0029-canonical/vectors/*.json` strictly as the oracle, and
only after every evaluator above was finished):

- **40/50 fixtures agree on every checked field.**
- **392/403 checked fields agree (97.3%)** — fields the oracle itself marks
  `not_evaluated` for a given fixture (e.g. `admission.status` on the
  EQ1–EQ4 equivalence-group fixtures, which only assert an influence-level
  verdict) are excluded from both the numerator and denominator, since there
  is no oracle claim there to agree or disagree with.
- **Admission (`accepted`/`rejected`/`indeterminate` + diagnostic),
  `capability`, fact-provenance (`FactStale`/`FactRevoked`/`FactWrongModel`/
  `FactWrongDeployment`/`FactUnauthorizedConsumer`, `L8`–`L13`), and
  `invalidation` agree on 100% of the fields the oracle asserts**, across all
  50 fixtures. All 11 remaining field disagreements are in `review.detail`
  (10) and `final_authority` (1, `M4`, already covered in §2).

**The 10 `review.detail` disagreements** (`EQ3A, EQ3B, K4, H2, M1, M4, I1,
R6_STALE, L3, L5`) are all the same pattern: my evaluator always reports the
review contract's mode name as `detail` on a pass (e.g. `"IndependentDecision"`)
and the specific finding code on a fail/indeterminate (confirmed correct
against `R4_CONFLICT` → `IndependenceConflict`, `R7_CAPACITY` →
`CapacityExceeded`, `R9_EVALUATION_MISSING` → `ReviewPredicateMissing`,
`R2_ACK_FOR_APPROVAL` → `ReviewModeInsufficient`, etc. — 0 disagreements on
any of those). The oracle's own `expected.review.detail` is `null` on these
10 specific fixtures because their symbolic shorthand happens to write bare
`review: pass` (no `:detail` suffix), whereas `R1_ACK`/`R5_PASS` write
`review: pass:Acknowledgement`/`pass:IndependentDecision` and the oracle's
detail is populated to match. Both readings compute the identical `status`;
they differ only in whether a redundant, always-derivable mode name is
echoed into `detail`. **Classification: unstable expected result** — the
symbolic source is internally inconsistent about whether a passing review's
`detail` is populated, with no substantive difference between the
underlying cases (nothing about `EQ3A` makes its passing `IndependentDecision`
review less determinable than `R5_PASS`'s).

## 4. Disagreement classification summary

| # | Fixtures | Field(s) | Classification | Which side is right |
|---|---|---|---|---|
| 1 | 21 (all Capability/Effect-bearing fixtures except M4) | `input.parameters.admission_policy` (canonicalization only; 0 semantic impact) | **Schema defect / ambiguous specification** | Neither — the symbolic source's own completeness claim is false; the value cannot be independently re-derived from any allowed input |
| 2 | `M4` | `input.catalog_entries` (canonicalization); `expected.final_authority` already matches (both encode the literal broken shorthand) | **Non-canonical input** | Neither implementation is "wrong"; the fixture's `final_authority: supervisor` shorthand names a nonexistent authority |
| 3 | `EQ3A, EQ3B, K4, H2, M1, M4, I1, R6_STALE, L3, L5` | `expected.review.detail` | **Unstable expected result** | Arguably mine — a real implementation can always derive the mode name on pass; the oracle's `null` reflects fixture-authoring terseness, not a different semantic answer |

No disagreement in this run was classified as an **implementation defect**
in this Ruby project once the above were understood and fixed (earlier,
pre-fix iterations did surface real defects on this side — see §5 — all
resolved before this final run).

## 5. Real ambiguities resolved during derivation (disclosed, not hidden)

Two points in RFC 0029.a Revision 1's text were genuinely ambiguous and had
to be resolved by cross-checking candidate readings against all 50 fixtures'
own inputs+expected blocks (explicitly licensed by the task brief), not by
picking whichever answer looked more "correct" in isolation:

1. **When does reaching an `AuthorityAssertion` end active model authority?**
   §4 rule 4 reads literally as "the assertion's *outgoing*
   `AssertFact`/`AssertDecision`/`Authorize` edges" deactivate authority —
   but fixture `I2` reaches an `AuthorityAssertion` via `HumanInput` with **no
   outgoing edge at all** (the assertion is never completed), and its own
   expected result is still `model_authorized: false`. That is only
   reachable if the `HumanInput` hand-off itself ends active model
   authority, before any assertion output edge exists. Adopted; verified to
   not regress any of the other seven `AuthorityAssertion`-bearing fixtures.
2. **`AssertFact` sourced directly from a `ModelInvocation`.** Revision 1
   §2.2's closed edge table licenses `AssertFact` only as
   `AuthorityAssertion → Fact`. Six fixtures (`L8`–`L13`) instead write
   `m-AssertFact-f` directly from the `ModelInvocation` node — an endpoint
   the closed table does not license (strictly, this should reject with
   `InvalidLineageEdge`). Their own expected results are a normal
   `Classify`-level, model-authorized answer, as if the model's raw output
   simply *is* the asserted fact. Treated as an ordinary `Classify`
   contribution (not an authority transition) when sourced this way, and
   flagged here rather than silently rejecting six fixtures the spec's
   literal text would reject and the corpus's own oracle does not.
   **Classification: non-canonical input** (an edge shape the RFC's own
   closed table disallows) coinciding with **ambiguous specification** (the
   RFC never says what a disallowed-but-present edge shape should compute,
   only that it "rejects" — which the fixture's own expected result does not
   do).
3. **Two fixtures (`M4`, `L5`) assert `review: pass` with no resolvable
   review contract in the input at all** (no `review_contract:` field, no
   `assertion:` block). The oracle's own generated *input* has the identical
   gap (`Review` node attributes read `{"review_contract": null}` — see §2).
   Semantically I still predict `pass` (defaulting to the corpus's universal
   `rc-independent-pass` contract) so that `final_authority` resolves per
   the profile mapping, since that is the only reading under which `L5`'s
   expected `final_authority: customs-officer` is reachable from the input
   at all; this is disclosed as a genuine input-completeness gap, not a
   silent guess.

## 6. Deviations from strict independence (disclosed)

- `rfcs/fixtures/0029-canonical/README.md` (an explicitly allowed file)
  describes the existing generator's *own* validation behavior in one
  sentence ("rejects duplicate YAML keys, floats, non-string JSON keys,
  naive timestamps and integers outside the exact I-JSON range"). This
  implementation's integer-only restriction (`lib/jcs.rb`) matches that
  description, but it was derived independently from RFC 8785's own I-JSON
  interoperability profile (§3.2.2.3 discussion of the safe integer range),
  which independently mandates the same restriction — not copied from the
  README's description of the generator's checks. No other content from
  excluded files was read, glimpsed, or otherwise consulted.
- No other deviation occurred. `crates/rfc0029-conformance/`, `generate.py`,
  `0029-canonical-ir-and-conformance-vectors.md`, `rfcs/evidence/0029/`
  (other than this new directory), `0029-phase0-readiness-and-domain-matrix.md`,
  and `docs/ROADMAP.md` were not opened.

## 7. What was not done

- The task's schema.json's `normalized_graph` "asserted" commitment was
  only computed for the 4 equivalence groups the symbolic source itself
  marks as carrying one (`EQ1`–`EQ4`, 8 fixtures); all other 42 fixtures
  correctly carry `normalized_graph.status = "not_asserted"`, matching the
  README's own statement that only those four groups have a reference
  normalizer commitment.
- `admission_policy`'s per-effect-class tier-cap table (§2) was not
  reconstructed. Doing so would require either consulting the excluded
  generator or reverse-engineering values from the oracle's own output
  before deriving the rule from the spec — both excluded by the task's
  independence rule. My admission evaluator instead uses a more
  conservative, self-derived rule (§13.2's absolute prohibition on
  `ExecuteIrreversible` via direct model trigger, plus a same-fixture
  authorized-level-vs-required-effect-tier comparison) that happens to reach
  the same verdict on every one of this corpus's 50 fixtures, but is not
  claimed to be the general per-effect-class cap mechanism RFC 0029.a §13.2
  actually calls for — that would need the missing table.
- No timing/performance testing; this is a one-shot batch conformance run
  (50 fixtures execute in well under a second).

## 8. Update (2026-09-27, same day): corpus fixes applied, re-run

After this report's findings were verified and acted on: the
`effect_authority_ceilings` table was added to the symbolic YAML itself
(closing finding #1 in §2), `M4`'s `final_authority` typo was fixed, and
`L8`–`L13` were rebuilt on RFC 0029.a's actual closed edge-table shape
(finding in §5, item 2) — which also turned up a second edge-kind violation
in that rebuild (`Eligibility` sourced from `Fact`, also disallowed;
corrected to `DataFlow`) once the crate gained a dedicated edge-kind
validator. `lib/expand.rb` was then updated to read
`effect_authority_ceilings` from the now-normative YAML (a mechanical
catch-up to newly-available allowed-file data, not a re-derivation of any
evaluator logic) and `verify.rb` was re-run:

- **Canonicalization: 50/50 fixtures now byte-identical and hash-identical.**
- **Semantics: 34/50 fully agree; the remaining 16/403 field disagreements
  are exactly the `review.detail` naming inconsistency already classified
  in §3/§4 as "unstable expected result"** — now also visible on `L8`–`L13`
  simply because their rebuild gave them a review contract for the first
  time (they had none before). No new disagreement category appeared.

This does not change any classification in §4 — it closes finding #1 and
the `M4` half of finding #2's canonicalization effect, and confirms the
`L8`–`L13` rebuild didn't introduce any semantic regression relative to
this implementation's own independent computation.

## 9. Update (2026-09-27, same day): CI differential gate, L8-L14 rebuild's second edge-kind bug, and a real Ruby-side defect found on re-run

Three further things happened, in this order:

1. `crates/rfc0029-conformance` gained a dedicated edge source/target-*kind*
   validator (`validate_graph`, RFC 0029.a §2.2's closed table) plus an
   exhaustive negative test per `EdgeKind` variant. Applying it to the
   already-rebuilt `L8`-`L13` found a **second** edge-kind violation this
   report's finding #2 (§5) didn't catch: `Eligibility`'s allowed sources
   are `ModelInvocation`/`Transform`/`Rule`/`CandidateSet` — not `Fact`,
   which `L8`-`L13`'s `f-Eligibility-d` edge used. Corrected to `DataFlow`
   (which does license `Fact -> Decision`); traced by hand to produce an
   identical semantic result, since the model's contribution was already
   locked in by an earlier edge in the same fold. This implementation was
   not asked to re-check that specific edge and did not independently
   catch it — recorded here for completeness of the corpus's history, not
   as this implementation's own finding.
2. `verify.rb` gained a permanent CI differential gate (exit 1 on any
   canonicalization mismatch, or any semantic mismatch outside the
   `review.detail` exception documented in §3/§4/§8), wired into
   `.github/workflows/rfc0029-differential.yml`. `lib/expand.rb` was
   updated to read the now-normative `effect_authority_ceilings` table
   (mechanical catch-up to newly-available allowed-file data, not new
   algorithm derivation — see §8).
3. A new fixture, `L14`, was added to close a real coverage gap a new
   "every diagnostic must be reached by some fixture" test found on the
   Rust side: no fixture demonstrated `FactProvenanceMissing` (a
   `fact_requirement` declared with no satisfying `fact_provenance`
   anywhere). Re-running this implementation against it surfaced a **real
   defect in `lib/fact_provenance.rb`**: `evaluate` returned `nil` ("not
   applicable") whenever `fact_provenance` was absent from the fixture,
   silently treating a *declared but unmet* requirement the same as "no
   requirement exists" — this implementation had never implemented
   `FactProvenanceMissing` at all. Confirmed via the differential gate
   itself: it failed with exactly this one fixture's four field
   disagreements (`admission.status`, `admission.diagnostic`,
   `capability.status`, `capability.effect_class`), all traceable to the
   same root cause. **Fixed**: `evaluate` now returns `"FactProvenanceMissing"`
   when a declared requirement has no corresponding provenance data at
   all, checked before any of the specific mismatch conditions. Re-run
   confirms this closes the gap: canonicalization 51/51, semantics agree on
   every field except the pre-existing, disclosed `review.detail`
   terseness gap (now 17 fixtures, `L14` included, since it also gained a
   review contract in its own construction).

**Classification of the `L14` finding**: implementation defect in this
Ruby project, not an RFC ambiguity — `fact_requirement`/`fact_provenance`
are this project's own fixture-level convention (documented in the
symbolic source's `lossless_expansion` header, not literally specified by
either RFC), so there is no independent spec text either side could have
misread; this implementation simply never handled the "requirement
declared, nothing satisfies it" case.

## 10. Update (2026-09-27, same day): five new cross-domain fixtures, one more real defect

Five fixtures were added, one per remaining domain profile (`B3` banking,
`K5` KYC, `H4` healthcare, `M5` manufacturing, `I4` insurance), each the
same RFC-sanctioned fact-provenance shape `L8` already exercises for
logistics — proving that shape, and this project's fact-provenance
evaluator, generalize across domains rather than being a logistics-specific
artifact. Corpus is now 56 fixtures.

Re-running this implementation found one more real, genuine defect: `B3`
disagreed on both canonicalization and `final_authority`. Root cause:
`lib/admission.rb`'s `REVIEWER_AUTHORITY_BY_PROFILE` table — itself derived,
as disclosed in §5 item, from cross-fixture pattern study — had no
`"banking"` entry, because no banking fixture before `B3` ever had a
*passing* human review to learn that mapping from (`B1` has no model at
all; `B2_STALE` is a shadow/observe-only case). This is not a coincidence
worth just patching silently: banking's `authority_catalog` defines no
human-reviewer role distinct from the institutional `"bank"` entry that
`INSTITUTIONAL_AUTHORITY_BY_PROFILE` already uses for `B1`'s no-human case
— every *other* profile has two separate authorities (an institution and a
named human role); banking has only one. **Fixed** by adding `"banking" =>
"bank"` with a comment recording exactly this, not by inventing a
distinct-but-arbitrary human-role authority that doesn't exist in the
catalog. Re-run: canonicalization 56/56, semantics agree on every field
except the pre-existing `review.detail` terseness gap (now 22 fixtures).

**Classification**: implementation defect in this Ruby project's derived
lookup table (an incomplete pattern-derivation, not wrong once the missing
data point existed) — not an RFC ambiguity, and not a defect in the new
fixtures, which correctly modeled banking's actual authority structure (one
authority entity serving both roles) rather than inventing a distinction
the domain doesn't have.

## 11. How to reproduce

```sh
cd independent-implementations/rfc0029-ruby
ruby verify.rb                 # requires Ruby 3.x; no gems, no Gemfile
```
(This session had no system Ruby available and ran it via
`docker run --rm -v "$PWD":/work -w /work/independent-implementations/rfc0029-ruby ruby:3-alpine ruby verify.rb`
— Docker was used purely as an execution environment, not part of the
deliverable.) A machine-readable `run_output.json` is written alongside
`verify.rb` on every run.
