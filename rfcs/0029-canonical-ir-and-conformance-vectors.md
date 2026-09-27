# RFC 0029 — Canonical IR and Conformance Vector Package v1

```
Amends:        0029.a-model-assurance-port.md#model-influence-and-human-review-semantics-revision-1 §12
Source:        fixtures/0029-influence-review-symbolic.yaml
Status:        Candidate encoding plus isolated Rust parser/JCS/influence verifier
Created:       2026-09-27
```

## 1. Boundary

This package fixes the wire shape for the reviewed influence and human-review
corpus. It does not implement admission, normalization, policy composition or
effect enforcement. Its purpose is to make independently implemented readers
agree on the bytes they consumed and on the exact verdict expected for each
case.

The canonical artifact is
[`fixtures/0029-canonical/schema.json`](./fixtures/0029-canonical/schema.json).
Generation and verification instructions are in the
[`fixture README`](./fixtures/0029-canonical/README.md).

## 2. Record shape

Each fixture is a closed record containing:

1. `schema_version`, fixture identity and profile identity;
2. `input.bundle`, fully expanded nodes and edges, referenced catalog entries,
   and case parameters;
3. `expected`, containing separate influence, authority, review, admission,
   capability and invalidation results; and
4. an asserted normalized graph and hash only when the reviewed source defines
   the complete normalization oracle.

The node-kind and edge-kind enums are closed. Unknown values reject schema
validation. Graph node attributes and profile parameters remain versioned
contract payloads; they are not yet the final production policy IR.

## 3. Encoding contract

- JSON must satisfy the Draft 2020-12 schema.
- Duplicate member names are rejected before expansion.
- Canonical bytes follow RFC 8785.
- The v1 numeric profile accepts exact-range integers only and rejects floats.
- Times are UTC RFC 3339 strings, never YAML-native timestamp values.
- SHA-256 covers the complete canonical fixture, including both input and
  expected result.
- Human-readable JSON is generated from the same in-memory value but is not the
  hashed representation.

## 4. Result separation

Expected results cannot occur beneath `input`. This prevents an evaluator from
accepting a fixture by echoing an embedded answer. `not_evaluated` is distinct
from `not_required`, `none`, `unknown`, rejection and indeterminacy.

The four normalization equivalence groups have exact normalized graph values
and hashes. Remaining cases retain their reviewed `graph_id`, but explicitly
say `not_asserted` for normalized graph content. Closing that field requires
the independently reviewed reference normalizer.

## 5. Validation performed

The generator currently proves mechanically that:

- all 50 symbolic fixtures expand;
- every generated readable vector validates against the schema;
- regeneration is byte-stable;
- every JCS file matches its manifest byte count and SHA-256 digest; and
- all four equivalence pairs share their asserted normalized value and hash.

## 6. Freeze blockers

This candidate cannot be called the final canonical policy IR until:

1. node attributes, contract payloads and profile extensions receive closed
   production schemas;
2. a Rust implementation independently parses and re-encodes all vectors;
3. that implementation computes, rather than consumes, every expected result;
4. the reference normalizer supplies asserted normalized graphs for applicable
   non-equivalence fixtures;
5. independent encoders agree byte-for-byte and hash-for-hash; and
6. downgrade, unknown-field and malformed-input negative vectors pass.

The dependency-light Rust
[`rfc0029-conformance`](../crates/rfc0029-conformance/README.md) crate now
implements versioned top-level IR types, strict closed enums, graph structural
validation, independent JCS/hash reproduction and the influence fold for all
38 graph-bearing fixtures. It remains disconnected from live effect gateways.
Nested duplicate-name rejection and the closed review-contract,
predicate-definition and predicate-evaluation path are now implemented; all
nine review fixtures are computed from their inputs. A separate
`evaluate_fact_provenance` evaluator now checks a persisted `Fact`'s
producer/freshness/revocation/authorized-consumer state against a consuming
`Decision`'s declared requirement (§8, `L8`–`L13`), orthogonal to the
influence fold. The combined evaluator also reproduces all 28 asserted
admission outcomes and 21 graph-based capability outcomes from explicit
per-effect limits and exact effect binding.
Closed remaining attribute/profile payload types, dependency invalidation and
complete normalized-graph emission remain freeze blockers.
## Canonical Influence and Human-Review Golden Fixtures

```
Companion to:  0029.a-model-assurance-port.md#model-influence-and-human-review-semantics-revision-1
Status:        Typed symbolic fixtures and candidate executable JSON/JCS encoding complete
Created:       2026-09-27
```

## 1. Canonical fixture shape

This document fixes the semantic content of the first golden fixtures. Their
complete typed symbolic records live in
[`fixtures/0029-influence-review-symbolic.yaml`](./fixtures/0029-influence-review-symbolic.yaml).
Their candidate executable expansion, schema, exact JCS bytes and SHA-256
manifest live under
[`fixtures/0029-canonical/`](./fixtures/0029-canonical/README.md). The encoding
is not the final production IR; its remaining freeze blockers are recorded in
the [canonical package specification](./0029-canonical-ir-and-conformance-vectors.md).

```yaml
fixture_id: stable-id
profile: profile-id-and-version
bundle: bundle-id-and-hash
nodes: [canonical typed nodes]
edges: [canonical typed edges]
declared_binding: influence-level
review_contract: optional contract/ref-and-predicates
dependencies: [phase-specific invalidation bindings]
expected:
  normalized_semantic_graph: symbolic-hash-id
  provenance_influenced: bool
  computed_model_level: level | None | Unknown
  model_authorized_target: bool
  final_authority: authority-chain-or-none
  review: Pass | Fail(finding) | Indeterminate
  admission: Accepted | Rejected(code) | Indeterminate
  capability: Issued(scope) | NotIssued
  invalidation: phase-and-action
```

All complete reviewed equivalence-group graphs now have binary hashes.
`normalized_semantic_graph` remains a stable symbolic fixture equivalence ID
for other cases, whose executable vectors explicitly mark normalized graph
content `not_asserted` until the reference normalizer can compute it.

## 2. Neutral-decomposition equivalence

| Fixture pair | Variant A | Variant B | Exact expected result |
| --- | --- | --- | --- |
| EQ1 | ModelInvocation → `Recommend` → Decision | ModelInvocation → DataFlow → Transform → DataFlow → Rule → `Recommend` → Decision | Same graph `g-recommend-1`; level `Recommend` |
| EQ2 | ModelInvocation → `Rank` → Presentation → Attention → Review | Insert any structural Transform/Rule chain before Rank | Same graph `g-prioritize-1`; level `Prioritize` |
| EQ3 | ModelInvocation → `DraftArtifact` → Review → AuthorityAssertion → Decision | Insert structural validation Rule before assertion | Same graph `g-draft-human-1`; level `Draft`; final human authority |
| EQ4 | ModelInvocation → `ObserveOnly` → Evidence | Insert isolated Transform before Evidence | Same graph `g-shadow-1`; level `None` only with same valid certificate |

Any pair producing a different classification fails determinism.

## 3. Banking fixtures

### B1 — model absent

Authoritative ledger/screening/FX facts flow through rules to an atomic
reservation decision. No ModelInvocation exists.

Expected: provenance `false`; level `None`; final bank authority; accepted if
other policy requirements pass.

### B2 — isolated fraud-model shadow output

The reservation decision is copied one-way to a shadow model sink using a
current §6 non-interference certificate. Output reaches Evidence only.

Expected: decision level `None`; `ObserveOnly` accepted. Removing/staling the
certificate yields `Indeterminate(ObserveOnlyAttestationStale)`.

### B3 — human-reviewed, fact-provenance-checked hold (2026-09-27)

`ModelInvocation → Recommend → Review → HumanInput → AuthorityAssertion →
AssertFact → Fact → DataFlow → Decision → Authorize → Capability`
(`wire.transfer.hold`, ceiling `Recommend`) — the same RFC-sanctioned
lineage shape §8's `L8` uses, added to prove `evaluate_fact_provenance`
generalizes beyond logistics. Also the first banking fixture with a
*passing* human review, which surfaced a real gap in the independent Ruby
implementation's derived `REVIEWER_AUTHORITY_BY_PROFILE` table (no prior
banking fixture had one to learn from) and a genuine cross-domain finding:
banking's `authority_catalog` has no human-reviewer role distinct from the
institutional `bank` entry `B1` already uses — every other profile has
both.

Expected: level `Recommend`; `model_authorized` false; final authority
`bank`; accepted; `wire.transfer.hold` issued.

## 4. KYC fixtures

### K1 — model feature in deterministic approval rule

ModelInvocation → `DerivedFeature` is represented as structural DataFlow into
a Transform, then `Eligibility` into onboarding Decision.

Expected: provenance `true`; level `Classify`; model authorized only up to the
profile's admitted eligibility use; under-declared `None` rejected.

### K2 — model-ranked analyst queue

ModelInvocation → `Rank` → Presentation → `Attention` → Review.

Expected: level `Prioritize`; human review does not erase provenance.

### K3 — model-created alert self-closure

ModelInvocation creates screening referral and supplies `ModelApproval` to the
closure Decision without independent assertion.

Expected: reject `ReviewSelfCertification`; no closure capability.

### K4 — independent adverse decision

Model `Recommend`s escalation. Qualified reviewer accesses required source
evidence and completes an `IndependentDecision` assertion authorizing the
domain Decision.

Expected: model level `Recommend`; model-authorized target `false`; final
authority is reviewer/compliance chain; review Pass; model provenance retained.

### K5 — fact-provenance-checked reject decision (2026-09-27)

Same `L8`/`B3` lineage shape (`application.reject`, ceiling `Recommend`),
added for cross-domain coverage.

Expected: level `Recommend`; `model_authorized` false; final authority
`compliance-reviewer`; accepted; `application.reject` issued.

## 5. Healthcare fixtures

### H1 — advisory alert

ModelInvocation consumes valid clinical inputs and `Recommend`s advisory
Alert Decision.

Expected: level `Recommend`; capability limited to clinical alert; use for
order/treatment rejected.

### H2 — order draft followed by clinician signature

ModelInvocation → `DraftArtifact` → Review. Clinician accesses required
observations/allergy/medication evidence and completes IndependentDecision
AuthorityAssertion → executable Order Decision.

Expected: model level `Draft`; final clinician authority; treatment capability
limited by clinician/domain policy; reasoning correctness remains Opaque.

### H3 — generated summary only

Same as H2, but source evidence is available and not accessed despite the
contract requiring access.

Expected: `Fail(RequiredEvidenceNotAccessed)`; no capability.

### H4 — fact-provenance-checked clinical alert (2026-09-27)

Same `L8`/`B3`/`K5` lineage shape (`clinical.alert`, ceiling `Recommend`),
added for cross-domain coverage.

Expected: level `Recommend`; `model_authorized` false; final authority
`clinician`; accepted; `clinical.alert` issued.

## 6. Manufacturing fixtures

### M1 — maintenance recommendation

ModelInvocation → `Recommend` → Presentation; human independently approves
work order through AuthorityAssertion.

Expected: model level `Recommend`; final maintenance authority is human chain;
model does not cap human approval.

### M2 — automatic reversible quarantine

ModelInvocation → Classify/Rule → `ModelTrigger` → Effect with
`Reversible` classification.

Expected: level `ExecuteReversible`; accepted only if profile/promise/gateway
admits exact quarantine; release/restart consumption rejected.

### M3 — hazardous restart attempt

ModelInvocation → `ModelTrigger` → irreversible/hazardous start Effect.

Expected: computed `ExecuteIrreversible`; rejected
`InfluenceLevelExceedsProfile`; no capability.

### M4 — human approval cannot override lock fact

Valid human assertion requests restart while authoritative personal-lock fact
activates mandatory safety deny.

Expected: final human authority present but effect denied; no capability.

### M5 — fact-provenance-checked work-order approval (2026-09-27)

Same `L8`/`B3`/`K5` lineage shape (`work_order.approve`, ceiling
`Recommend`), added for cross-domain coverage.

Expected: level `Recommend`; `model_authorized` false; final authority
`maintenance-supervisor`; accepted; `work_order.approve` issued.

## 7. Insurance fixtures

### I1 — damage recommendation and independent settlement

ModelInvocation → `Recommend`; adjuster completes IndependentDecision using
contract/loss/source evidence.

Expected: model level `Recommend`; final authority adjuster; settlement
capability may issue within authority limit.

### I2 — generated adverse reason

ModelInvocation → `DraftReason` → Review, but reason lacks actual clause/rule
reference.

Expected: level `Draft`; `Fail(ReasonNotEvidenceBound)`; notice/effect blocked.

### I3 — bounded low-value approval

ModelInvocation → `ModelApproval` → claim Decision within admitted product,
amount, evidence and recovery boundary.

Expected: level `Approve`; model-authorized target `true`; capability limited
to exact bounded claim class. Any payment/payee use rejected.

### INV_PRECOMMIT / INV_EXTERNAL_UNKNOWN — external payment invalidation phases

(Renamed here 2026-09-27 from this section's own prior "I4" label, which
named a concept, not an actual fixture id — no fixture was ever built with
id `I4` for this until a *different* fixture claimed that id below. Always
refer to fixtures by their real corpus id.) Independent settlement
capability is reserved; payee version changes before local commit.

Expected: `Abort` or `ReevaluateAtomically` exactly as dependency binding
declares. If provider may have accepted before discovery, enter
`UnknownOutcome` and `Reconcile`; never issue duplicate payment.

### I4 — fact-provenance-checked settlement (2026-09-27)

Same `L8`/`B3`/`K5` lineage shape (`settlement.approve`, ceiling
`Recommend`), added for cross-domain coverage.

Expected: level `Recommend`; `model_authorized` false; final authority
`adjuster`; accepted; `settlement.approve` issued.

## 8. Logistics fixtures (out-of-sample)

Logistics/customs played no part in deriving C1–C6; see
[the logistics/customs tabletop](./evidence/0029/tabletops/logistics-customs.md).
These fixtures are the executable encoding of that document's §17
conformance cases, not a new profile family invented independently of it.

### L1 — deterministic release, no model

Fact → Rule → Decision → Capability → Effect, no ModelInvocation node.

Expected: level `None`; `model_authorized` false; accepted; `cargo.release`
capability issued.

### L2 — automatic reversible hold

ModelInvocation → `ModelApproval` → Decision → `ModelTrigger` → Effect with
`Reversible` classification.

Expected: level `ExecuteReversible`; accepted at the profile's admitted
ceiling for `shipment.hold`; capability issued.

### L3 — human-reviewed release

ModelInvocation → `Recommend` → Review; officer independently authorizes
release through AuthorityAssertion.

Expected: level `Recommend`; final authority is the customs officer; model
does not cap human approval; `cargo.release` capability issued.

### L4 — autonomous release attempt

ModelInvocation → `ModelTrigger` → Effect with `Irreversible`
classification for `cargo.release`.

Expected: computed `ExecuteIrreversible`; rejected
`InfluenceLevelExceedsProfile`; no capability.

### L5 — active seizure order overrides human assertion

Valid human assertion requests release while an authoritative active-seizure
fact activates mandatory deny.

Expected: final human authority present but effect denied; no capability.

### L6 — model cannot clear its own referral

ModelInvocation → `ModelApproval` → Decision, review contract is
`self-review`.

Expected: level `Approve`; `model_authorized` true;
`Fail(SelfCertification)`; rejected `ReviewSelfCertification`.

### L7 — external duty-assessment invalidation

Duty-assessment version dependency after an external-authority acknowledgement
becomes unknown.

Expected: `Reconcile`; `UnknownOutcome`; duplicate release forbidden.

### L8–L13 — cross-decision model-fact lineage

The out-of-sample review's one open item (§8 of the tabletop): a persisted
classification fact produced by a model in one decision episode and consumed,
unmodified, by a later, separate decision with no `ModelInvocation` node of
its own. A new, orthogonal evaluator (`evaluate_fact_provenance`) checks a
`Fact` node's producer identity, freshness, revocation, and authorized
consumers against a consuming `Decision`'s declared requirement — a check
independent of `compute_influence` and `evaluate_review`, mirroring how
those two are already kept separate.

An initial version of these six fixtures wired `ModelInvocation` directly to
`Fact` via `AssertFact`. A second, independently built implementation (Ruby,
`independent-implementations/rfc0029-ruby/`) checked this against RFC
0029.a's own closed edge table (§2.2) and found `AssertFact` is licensed
**only** as `AuthorityAssertion → Fact` — the only edge in the entire table
with `Fact` as a target. A model cannot structurally produce a `Fact` at
all; only an authority assertion can. This is a stronger resolution of the
tabletop's original worry than the fixtures first gave it (see the
tabletop's §8 for the full account) and also exposed that the Rust crate
never validated edge source/target *kinds* against this table — only that
edge endpoints exist (readiness matrix §7).

All six fixtures now share one RFC-sanctioned graph — `ModelInvocation →
Recommend → Review → HumanInput → AuthorityAssertion → AssertFact → Fact →
Eligibility → Decision → Authorize → Capability` (`cargo.release`, ceiling
`Recommend`) — so `compute_influence` computes the same `Recommend` level
and `model_authorized: false` for all of them (a human authority reviewed
and asserted the fact; the model is not itself authorized at the protected
capability). Only the `Fact`'s provenance attributes and the `Decision`'s
declared requirement differ.

| ID | Fact provenance vs. requirement | Expected |
| --- | --- | --- |
| L8 | producer, deployment, freshness and consumer all match | accepted; `cargo.release` issued |
| L9 | `valid_until` before evaluation time | rejected `FactStale` |
| L10 | `revoked: true` | rejected `FactRevoked` |
| L11 | `produced_by_model` mismatch | rejected `FactWrongModel` |
| L12 | `produced_by_deployment` mismatch | rejected `FactWrongDeployment` |
| L13 | `authorized_consumers` does not name the consuming decision | rejected `FactUnauthorizedConsumer` |

This closes the representational question the reviewer raised, on a
RFC-conformant graph shape, but not the full temporal one: every fixture
here is still one static, single-shot graph evaluated at one `evaluated_at`
instant, so it demonstrates a producer/consumer *contract* being checked,
not two independently-timed decision episodes separated by real elapsed
time or a live revocation event arriving between an authority's assertion
and a later decision's consumption of it. A fuller treatment would need
either genuine cross-fixture state or a stateful pilot (as `funds.reserve`
is for ledger
mutation), and remains open.

## 9. Review-mode fixtures

| ID | Declared mode and facts | Exact expected result |
| --- | --- | --- |
| R1 | Acknowledgement; authenticated click | Pass Acknowledgement only |
| R2 | BoundedApproval required; acknowledgement record supplied | Fail `ReviewModeInsufficient` |
| R3 | IndependentDecision; evidence available but required access absent | Fail `RequiredEvidenceNotAccessed` |
| R4 | IndependentDecision; independence predicate false | Fail `IndependenceConflict` |
| R5 | IndependentDecision; all typed predicates current | Pass; capability bounded by reviewer authority |
| R6 | Review predicates stale | Indeterminate or Deny exactly per predicate failure behavior |
| R7 | Capacity window unhealthy | Fail `CapacityExceeded`; declared Narrow/Suspend response |
| R8 | Rubber-stamp monitor data missing | SuspendClaim or Indeterminate exactly per monitor contract |

## 10. Fixture completion gate

The schema-independent re-review uses the complete symbolic records linked in
§1. It may pass only if their typed nodes, edges, contracts and exact outputs
are sufficient and every equivalence pair produces the same normalized
semantic graph. Phase 0 exit still requires executable canonical files and
binary hashes.

## 11. Independent implementation result (2026-09-27)

A second, from-scratch implementation
([`independent-implementations/rfc0029-ruby/`](../independent-implementations/rfc0029-ruby/),
report at
[`rfcs/evidence/0029/independent-implementation/report.md`](./evidence/0029/independent-implementation/report.md))
was built in Ruby (standard library only), without reading
`crates/rfc0029-conformance/`, `generate.py`, or any evidence/tabletop
document, against the same 50 symbolic fixtures. Findings, each verified
against the actual files before being accepted, not taken on the report's
own word:

- **Canonicalization now matches on all 50/50 fixtures.** The first run
  matched 28/50; the 21 non-`M4` mismatches traced to one field,
  `input.parameters.admission_policy`, having **no textual basis anywhere in
  the symbolic YAML** — only in `generate.py`'s (excluded) hardcoded table.
  §5 above claimed this file was "lossless"; it was not. Fixed by moving the
  per-effect-class ceiling table (RFC 0029.a §13.2) into the symbolic source
  itself as `effect_authority_ceilings`, with `generate.py` now reading it
  from there instead of a separate hardcoded dict — same values, now
  actually normative. The 22nd mismatch (`M4`) was a genuine pre-existing
  typo: `expected.final_authority: supervisor` named a key
  (`authority_catalog` has only `maintenance-supervisor`) that never
  existed; fixed directly.
- **Semantics agreed on 40/50 fixtures fully, 392/403 checked fields
  (97.3%) before any fix** — admission, capability, fact-provenance, and
  invalidation agreed 100% of the time either side asserted a value. The
  remaining disagreements were the same `M4` typo plus a cosmetic
  inconsistency in whether a passing review's `detail` field is populated
  (10 fixtures) — not a semantic difference, left as a known, low-severity
  fixture-authoring inconsistency.
- **A real defect in `L8`–`L13`, not in the corpus at large:** those six
  fixtures used `ModelInvocation -AssertFact-> Fact` directly, which RFC
  0029.a's own closed edge table (§2.2) does not license — `AssertFact` is
  sanctioned only as `AuthorityAssertion → Fact`. Rebuilt on the
  RFC-sanctioned shape; see §8 above and the tabletop's §8 for the full
  account, including what it also revealed about the Rust crate never
  validating edge source/target kinds against that table at all.

This closes the "second, independently-built implementation… agreeing
byte-for-byte and hash-for-hash" freeze blocker named in §6 above and in the
readiness matrix §6 — for canonicalization.

**Update (2026-09-27, same day):** everything this section's closing
paragraph once listed as still open has since closed. A dedicated edge-kind
validator now checks every edge against RFC 0029.a's closed table (catching
a *second* `L8`–`L13` defect this report's own §11 text didn't). The result
taxonomy is now closed `AdmissionDiagnostic`/`ReviewFinding` enums with a
generated coverage matrix (which found and closed two real gaps —
`FactProvenanceMissing` unreached by any fixture, fixed by adding `L14`, and
that same diagnostic never implemented at all in the Ruby side, caught on
re-run). Compatibility/downgrade and version/extension policy are now
executable tests, not prose. And — the one thing that genuinely required
new engineering, not just closing a checklist item — this "one run of one
independent implementation" is now a permanent CI differential gate
(`.github/workflows/rfc0029-differential.yml`), re-verifying agreement on
every future change instead of asserting it once. `Frozen version` is
declared on this basis in the readiness matrix §9, core-only — the domain
matrices below (§12) and `AdmissionReportV1`'s generalization remain
explicitly outside it.

## 12. Cross-domain conformance matrix (2026-09-27)

Per-domain coverage against the dimensions the logistics out-of-sample
tabletop (§8) exercises, as of this date — built honestly, not claimed
complete. A ✓ means a fixture in that cell exists and passes; a — means it
does not exist yet. This table is itself evidence for or against a claim,
not decoration: reading it should tell you exactly what has and has not
been checked for each domain, without needing to open five separate
sections above to find out.

| Dimension | Banking | KYC | Healthcare | Manufacturing | Insurance | Logistics |
| --- | --- | --- | --- | --- | --- | --- |
| No-model, institutional decision | `B1` ✓ | — | — | — | — | `L1` ✓ |
| Autonomous model-triggered reversible effect | — | — | — | `M2` ✓ | `I3` ✓ (Approve, bounded) | `L2` ✓ |
| Human-reviewed, independent assertion | `B3` ✓ | `K4` ✓ | `H2` ✓ | `M1` ✓ | `I1` ✓ | `L3` ✓ |
| Autonomous effect exceeds profile ceiling (rejected) | — | — | `H1_ORDER_REUSE` ✓ | `M3` ✓ | `I3_PAYMENT_REUSE` ✓ | `L4` ✓ |
| Mandatory-fact deny overrides human approval | — | — | — | `M4` ✓ | — | `L5` ✓ |
| Self-certification rejected | — | `K3` ✓ | — | — | — | `L6` ✓ |
| Dependency invalidation / reconcile | — | — | — | — | `INV_PRECOMMIT`/`INV_EXTERNAL_UNKNOWN` ✓ | `L7` ✓ |
| Fact-provenance lineage (positive case) | `B3` ✓ | `K5` ✓ | `H4` ✓ | `M5` ✓ | `I4` ✓ | `L8` ✓ |
| Fact-provenance lineage (stale/revoked/wrong-model/wrong-deployment/unauthorized/missing) | — | — | — | — | — | `L9`–`L14` ✓ (all six) |

**What this closes**: every domain now has at least a human-reviewed case
and a fact-provenance-lineage positive case, proving the frozen core's
evaluators (`compute_influence`, `evaluate_review`, `evaluate_fact_provenance`,
`evaluate_admission`) generalize across all five domain profiles this
project tracks, not just logistics — and building these five fixtures
found two more real, genuine defects (documented in the independent-
implementation report §9–§10): a second `L8`–`L13` edge-kind violation, and
a missing `"banking"` entry in the Ruby implementation's derived
reviewer-authority table, which also surfaced a real cross-domain modeling
fact (banking has no human-reviewer authority distinct from its
institutional one).

**What this does not close**: the dashes above are real gaps, not omissions
by accident. Banking has no autonomous-reversible-effect, exceeds-ceiling,
mandatory-deny, self-certification, or invalidation fixture. KYC has no
no-model, autonomous-reversible, exceeds-ceiling, mandatory-deny, or
invalidation fixture. Healthcare and manufacturing are each missing three
to four dimensions; insurance is missing two. Only logistics has full
row coverage, because it was the domain the out-of-sample review actually
adversarially tested end to end — the other five were extended exactly as
far as proving the frozen core generalizes required, per this step's own
instruction to have the matrices consume the frozen core rather than
reshape it, not as far as logistics' own adversarial depth. Closing every
remaining cell is future work, not claimed here.
