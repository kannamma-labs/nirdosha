# RFC 0029 — Independent Out-of-Sample Review: Logistics/Customs

Reviews:       rfcs/evidence/0029/tabletops/logistics-customs.md
Status:        PASS
Reviewed:      2026-09-27
Method:        Independent read-only review against RFC 0029.a §13 (C1-C6)
               and RFC 0029 §36, cross-checked against the executable
               fixtures L1-L7 in rfcs/fixtures/0029-canonical/vectors/
               and the compute_influence/evaluate_admission logic in
               crates/rfc0029-conformance/src/lib.rs.

## 1. Verdict

No genuine seventh universal primitive is required to represent
logistics/customs import release. I tried, in good faith, to break C1-C6 and
RFC 0029 §36 against six categories of hard cases (sovereign jurisdiction
conflict with no supranational tie-breaker, seizure/forfeiture as a distinct
effect from a hold, bonded-warehouse many-to-one consolidation, free-trade
supply-chain audit trails the model cannot itself observe, an AI
classification output laundered into a "fact" that a duty rule keys off, and
storage-duration-triggered forfeiture) and in every case the existing node/
edge vocabulary, the six MAP contracts, and the twelve §36 hardening
requirements were sufficient once applied honestly. None of these needed a
new node kind, edge kind, decision class, evidence-finality mode, safety
posture, or effect-authority tier.

That said, this is not a spotless document, and I do not think the author's
own §19 claim ("no case in this domain-language pass required proposing a new
universal primitive") is fully earned by what the tabletop and its fixtures
actually test. The single most consequential representational claim in the
whole document — that an AI model's HS-code classification safely reduces to
`Classify` tier because it "feeds a deterministic tariff lookup rule, not a
direct decision — mirrors the KYC `K1` pattern" (§8) — is asserted by analogy
and never tested by any case in §17 or any fixture in L1-L7. When I built out
that case by hand and compared it structurally to the actual K1 fixture, I
found the analogy is looser than the tabletop implies (K1 is a single-hop,
single-graph pattern; customs' classify → assess-duty → release is a
multi-stage pattern where the model's causal contribution is most likely
already crystallized into an ordinary persisted `Fact` by the time a release
decision is evaluated, possibly much later, by a different gateway instance).
I ultimately convinced myself this composition *is* representable without a
new primitive — via C1's transformation-lineage requirement plus RFC 0029
§36.8's dependency/re-evaluation mechanism, exactly as row 12 of the same
table already demonstrates for tariff-table corrections — but the tabletop
never demonstrates this itself, and the executable evaluator (`lib.rs`) has
no fixture proving that lineage is actually carried forward across a
multi-decision boundary. That is a real gap in what has been tested, even
though it did not turn into an actual counterexample. See Finding 1.

I also found a concrete, checkable inconsistency in the executed fixture data
(Finding 2) and two narrower completeness gaps in the tabletop's own effect
and policy-kind taxonomy (Findings 3-4). None of these force a new primitive
either, but an independent reviewer should not wave them through silently.

## 2. Case-by-case scoring

Note: the tabletop's §17 table actually contains **12** rows, not the 10
implied in the task brief. All 12 are scored below.

| # | Case | Score | Reason |
| --- | --- | --- | --- |
| 1 | Deterministic declaration, no model, duty paid, no hold (`L1`) | PASS | Plain `Fact→Rule→Decision→Capability→Effect` composition with no model involved; verified against the canonical fixture, which admits and issues `cargo.release` exactly as claimed. |
| 2 | AI model places a reversible container hold pending review (`L2`) | PASS | `ModelApproval→Decision→Authorize→Capability→Issue` plus `ModelTrigger→Effect(Reversible)` correctly resolves to `ExecuteReversible`, at exactly the admitted ceiling for `shipment.hold`; matches the manufacturing quarantine pattern. |
| 3 | AI recommends release; officer independently authorizes (`L3`) | PASS | The fixture is unusually thorough: it actually evaluates all fifteen C5-style independence/capacity predicates (qualification, independence, evidence access/accessed, capacity, escalation, override, sampling, reason-bound, etc.) rather than asserting review in the abstract. This is a genuine, non-rubber-stamp independent-review demonstration. |
| 4 | AI model attempts to trigger release directly (`L4`) | PASS, with caveat | Correctly rejected (`InfluenceLevelExceedsProfile`) against the `Recommend` ceiling for `cargo.release`. Caveat: the fixture labels this same effect `cargo.release` as `reversibility_class: Irreversible`, while `L1` labels the identical `effect_id` `Compensatable` — see Finding 2. Doesn't change the outcome here (both tiers exceed `Recommend`), so it doesn't meet the bar for a FAIL, but it is a real inconsistency. |
| 5 | Active seizure order exists (`L5`) | PASS | Unconditional `MandatoryDeny` short-circuit ahead of influence/review evaluation, exactly mirroring the manufacturing `M4` mandatory-deny pattern and RFC 0029 §36.6/§36.7. |
| 6 | Model that flagged a shipment also tries to clear its own referral (`L6`) | PASS | `review_contract: self-review` correctly fails via `evaluate_review`'s `SelfCertification` path; structurally identical to KYC `K3`. |
| 7 | Customs submission accepted locally, external ack times out (`L7`) | PASS | Directly mirrors insurance `INV_EXTERNAL_UNKNOWN`'s dependency/phase/action shape (`after_external_unknown` → `Reconcile` → `UnknownOutcome`, duplicate effect forbidden). |
| 8 | Carrier manifest and broker declaration disagree on weight | PASS | Correctly deferred to domain policy precedence/quorum/corroboration rules per §36.7 rather than the architecture asserting a specific winner; this is the right posture, not a gap. |
| 9 | Origin-country export permit conflicts with destination import restriction | PASS | Squarely §36.12's conflict-of-law handling: `Indeterminate` plus manual escalation, no silent selection of the more convenient jurisdiction. I looked hard for a case where two sovereign authorities could each claim final (not just evidentiary) authority over the *same* effect with no tie-breaker, but physical custody always resolves which authority's decision is actually enforceable next, so this reduces to the same applicability-conflict pattern already covered. |
| 10 | Warehouse system releases a container bypassing the declaration gateway | PASS | The strongest example of intellectual honesty in the document: named an explicit, uncovered bypass path (§18) left `Uncontrolled`/`AcceptedRisk` rather than falsely claimed as covered — exactly what §36.2's closure-honesty requirement demands. |
| 11 | Inspection-priority model underperforms for one origin-country cohort | PASS | Same subgroup-assurance promise family already used for KYC/lending fairness monitoring. Residual note: "origin country" as a protected subgroup axis carries WTO most-favored-nation/non-discrimination baggage that ordinary demographic fairness monitoring doesn't — but that's a policy-content/legal question the document correctly disclaims (§18), not an architecture gap. |
| 12 | Duty assessed retroactively changes after a tariff-table correction | PASS | Direct, uncomplicated application of §36.8 dependency/transition revalidation; same shape as rate-table corrections elsewhere. Notably, this is the *same* mechanism that would have to carry a corrected HS classification too — see Finding 1. |

## 3. Findings

**Finding 1 (primary) — The classify→duty→release composition is asserted, not tested, and the K1 analogy is looser than claimed.**
§8 states the classification model's HS-code suggestion "feeds a deterministic
tariff lookup rule, not a direct decision — mirrors the KYC `K1` pattern," and
sets the admitted ceiling for `line_item.classify` at `Classify`. But:
- No row in §17 and no fixture in L1-L7 actually builds this chain (classify
  → duty assessed → "duty paid" satisfies the release-eligibility predicate
  → release). `L1` explicitly excludes the model ("no model"); nothing else
  touches it.
- The actual `K1` fixture (`rfcs/fixtures/0029-canonical/vectors/K1.json`) is
  `ModelInvocation --DataFlow--> Transform --Eligibility--> Decision`: one
  hop, one connected graph, and the `Eligibility` edge lands directly on the
  *same* Decision node being admitted. Customs' real shape is multi-stage:
  the classify decision and the release decision are different decisions,
  almost certainly evaluated at different times by different gateway calls,
  with the classification's result persisted as an ordinary `Fact` by the
  time the release decision's graph is built.
- `compute_influence` (lib.rs) computes influence purely from the graph it is
  handed; if that graph doesn't include the `ModelInvocation` node (because
  the classification already concluded earlier and only its output survived
  as a Fact), `models.is_empty()` returns `{level: None, model_authorized:
  false}` — i.e., the model's real causal contribution to the release
  decision becomes structurally invisible to the admission check, unless
  graph-construction practice deliberately carries the model's lineage
  forward.
- The tabletop's own §5 (Facts and authorities) table lists eight fact
  sources but has no row for "line-item classification when produced by a
  model." Compare the manufacturing profile (`0029-domain-profiles.md`),
  which the tabletop repeatedly claims to mirror, and which explicitly states:
  "Model output remains inference until the admitted adjudication step." The
  customs tabletop never states the equivalent safeguard for its own
  classification pathway.

On reflection I do not think this forces a new primitive: C1's
transformation-lineage requirement plus §36.8's dependency-invalidation
mechanism (the same mechanism row 12 already exercises for tariff-table
corrections) is sufficient in principle — a corrected/audited misclassification
is just another fact correction that must trigger re-evaluation of the
affected release decision, model-derived or not. But "sufficient in
principle, never demonstrated" is exactly the gap an out-of-sample review
exists to catch. **Recommendation:** add an explicit fixture (e.g. `L8`)
that builds the connected classify→assess→release graph with the
`ModelInvocation` node carried into the release decision's graph, and add a
row to §5 stating that model-produced classification remains inference until
an admitted adjudication/correction step, mirroring the manufacturing
sentence verbatim.

**Finding 2 (minor, verified in executed fixtures) — `cargo.release`'s `reversibility_class` is inconsistent between `L1` and `L4`.**
`rfcs/fixtures/0029-canonical/vectors/L1.json` labels effect `cargo.release`
as `reversibility_class: Compensatable`; `L4.json` labels the *identical*
`effect_id` `Irreversible`. `contribution()` in lib.rs derives a
`ModelTrigger` edge's tier directly from this per-instance attribute, with no
cross-check that a given `effect_id` carries one canonical classification
(as RFC 0029 §36.2's effect-taxonomy-closure language implies it should). In
this fixture set the inconsistency is inert — both `ExecuteCompensatable` and
`ExecuteIrreversible` exceed the `Recommend` ceiling, so `L4`'s rejection
outcome doesn't change — but nothing in `validate_graph` or
`evaluate_admission` would catch a case where mislabeling *did* change the
outcome (e.g., a mislabeled effect sliding under a ceiling it should have
exceeded). This is a fixture/harness rigor gap shared across all domains
using this evaluator, not something specific to logistics, so it isn't
out-of-sample evidence of anything domain-specific — but it surfaced here and
is worth fixing.

**Finding 3 (minor) — Seizure/forfeiture is used but never declared as a distinct effect.**
§17 row 5 and §5 treat "active seizure order" purely as an *incoming* fact
from customs enforcement/judicial authority that triggers mandatory deny of
release. §3's effect list never names an internal "seize"/"forfeit" effect
distinct from "place a hold," even though real customs regimes have
administrative forfeiture processes initiated by the customs officer, not
only exogenous judicial orders. This is trivially representable with the
existing `Effect`/`Decision`/`Capability` primitives and the existing
`Irreversible` finality class (it needs no new kind), but §3's effect closure
is incomplete relative to §36.2's closure requirement as currently written.

**Finding 4 (minor) — "Bounded temporal requirement" policy kind is used implicitly (§12's perishable-goods carve-out) but never listed in §7.**
The manufacturing profile explicitly lists "bounded temporal requirement" as
one of its policy kinds. Bonded-warehouse storage-duration limits
(demurrage, "goods deemed abandoned" after N days) are a natural fit for this
existing policy kind but §7 omits it from its enumeration. Cosmetic, but
worth tidying since it's the kind of omission that later gets mistaken for
"customs doesn't need this."

No finding above required a new node kind, edge kind, decision class,
evidence-finality mode, safety posture, or effect-authority tier.

## 4. Seventh-primitive assessment

No. After actively trying to construct a counterexample across sovereign
jurisdiction conflicts, seizure/forfeiture, bonded-warehouse consolidation,
free-trade audit trails, model-classification laundering, and
storage-duration forfeiture, I could not find a logistics/customs requirement
that forces a new node kind, edge kind, C7 contract, or extension to RFC 0029
§36. Every case in §17, including the two I had to build out myself because
the tabletop's own table doesn't cover them, resolves using C1-C6 and the
existing candidate schema.

My residual doubt is narrow and specific: the classify→duty→release
composition (Finding 1) is the one case in this domain where a wrong
conclusion would be easy to miss, because the danger isn't a missing
primitive — it's a plausible-looking analogy (to KYC's `K1`) that turns out,
on close structural comparison, not to match as tightly as claimed, papering
over a multi-stage lineage-carrying requirement that is never actually
exercised. I'm confident this is fixable within the existing architecture
(an added fixture plus a one-sentence addition to §5, per the
recommendation above), and confident enough after working through it myself
that it does not amount to a missing primitive. But "confident after
independently reconstructing the missing test case" is a different, weaker
claim than "the document proved it," and I want that distinction on the
record rather than quietly rounding it up to a clean pass.
