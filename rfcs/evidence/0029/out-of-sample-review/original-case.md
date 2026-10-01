# RFC 0029 — Logistics/Customs Out-of-Sample Tabletop

```
Companion to:  RFC 0029 — Domain-neutral policy admission, enforcement,
               and evidence
               RFC 0029.a — Provider-neutral Model Assurance Port
Status:        Out-of-sample falsification exercise; no implementation
Created:       2026-09-27
Scenario:      Bonded-shipment customs import release
Purpose:       Test C1–C6 and the candidate policy IR against a domain that
               played no part in deriving them
Gate:          rfcs/0029-phase0-readiness-and-domain-matrix.md
               §"Convergence and Independent Validation Plan" §2
```

## 0. Why this domain

The MAP core (C1–C6) was derived from, and then re-tested against, exactly
the four domains that motivated it: KYC, medical, manufacturing, and
insurance. Finding no seventh universal primitive across those four is
in-sample evidence, not independent validation. Logistics/customs is chosen
here because RFC 0029 §6's own domain-neutral vocabulary table already names
it (`Logistics | Carrier | Handover | Shipment`) — so the core claims to
cover it — but no tabletop, profile, or fixture for it existed before this
document, and it did not inform C1–C6's design. This document is written in
domain language first; the executable encoding in
`rfcs/fixtures/0029-influence-review-symbolic.yaml` (fixtures `L1`–`L7`) is a
separate, later mapping step, exactly as required by the readiness matrix's
candidate-schema validation protocol (§8).

## 1. Boundary

An importer's customs broker files an import declaration for a bonded
shipment. Systems in scope: broker filing system, carrier manifest system,
bonded-warehouse inventory system, national customs authority's declaration
and risk-engine system, an AI-assisted goods-classification and
inspection-priority model, a customs officer's case-review terminal, a duty/
tax assessment and payment system, and a post-release audit and appeal
system. The deployment is Tier 2: gateways and deployment controls are
expected to mediate declaration submission, hold placement, duty assessment,
and physical release notice; static no-bypass coverage is not claimed, and
physical possession of the cargo remains outside Nirdosha's guarantee in the
same sense RFC 0029 §30 and the manufacturing tabletop already draw for
machine actuation — a release notice is a software effect; a forklift moving
a container is not.

Actors and authorities: importer, customs broker, carrier, bonded-warehouse
operator, customs officer (case authority), customs risk-and-classification
model, national customs authority (institutional authority for duty and
release), destination-country and origin-country/transit sovereign
authorities where they disagree, and an appeals/audit authority.

## 2. Resources

Canonical resource: one `shipment` (a bonded consignment identified by a bill
of lading / manifest reference), with derived resources:

- `declaration` — one customs declaration version per shipment, effective-
  dated, amendable, superseded by correction;
- `line item` — one tariff/HS-code classification unit within a declaration;
- `container`/`package` — physical unit(s) comprising the shipment, which may
  split (partial release) or merge (consolidated manifest);
- `duty assessment` — one computed liability record per declaration version.

Aliases and equivalence: the same shipment may be referenced by carrier
bill-of-lading number, warehouse lot number, and customs entry number.
Split/merge of containers is a declared derivation relation, not a new
canonical identity. A mismatched alias (e.g., broker files against the wrong
manifest reference) produces `Indeterminate`, exactly per RFC 0029 §36.1 —
this is the same pattern already required for banking beneficiary aliases and
healthcare patient merges.

## 3. Effects

Distinct protected effects, deliberately not collapsed into one
`customs.decide`:

- accept/reject a declaration for processing;
- classify a line item (HS code / tariff category);
- assess duty/tax liability;
- place a shipment/container hold;
- release a shipment/container (full or partial);
- flag or clear an inspection referral;
- request additional documentation;
- correct a declaration or assessment;
- open/close an appeal;
- notify a sovereign authority of a discrepancy;
- record a post-release audit finding.

Closure includes: administrative re-assessment, batch/backfill correction of
duty tables, EDI message-based declaration submission (not only the broker
UI), and warehouse-system-driven release triggers that bypass the customs
declaration path entirely — this last one is a named exclusion below (§18),
not a silently covered path.

## 4. Vocabulary and predicates

Units and bounded predicates: currency-denominated duty amount (minor units,
integer), tariff/HS code (closed code list per customs authority, versioned),
country-of-origin code, weight/volume/quantity per line item, container
count, declared value, exchange rate (effective-dated), inspection-priority
score (model output, bounded `[0,1]`, never itself a fact), hold reason code
(closed enum), release-eligibility predicate (deterministic: declaration
accepted, duty paid or bonded, no active hold, no active seizure order).

## 5. Facts and authorities

Authoritative fact sets and their sources:

- carrier manifest (authority: carrier system);
- broker declaration (authority: broker, attested by broker credential);
- bonded-warehouse inventory state (authority: warehouse operator system);
- tariff/HS classification tables and duty rates (authority: national
  customs authority, effective-dated);
- exchange rate (authority: designated rate provider, freshness-bound);
- sanctions/denied-party and prohibited-goods lists (authority: national and
  multilateral sanctions authorities);
- active seizure/hold orders (authority: customs enforcement or judicial
  order, exact per RFC 0029 §36.6/§36.7 mandatory-deny semantics);
- prior payment/bond record (authority: customs payment system).

Disagreement: carrier manifest, broker declaration, and warehouse inventory
routinely disagree on weight, count, or contents. RFC 0029 §36.7's
per-fact-type authority/precedence/quorum contract applies directly —
customs policy, not Nirdosha, decides whether declaration governs subject to
manifest corroboration, or vice versa. This is `CoreExact`, specialized by
the profile (§7).

## 6. Applicability

Jurisdiction/applicability facts: origin country, transit country/countries,
destination country, product category (some categories route to a different
regulatory authority — e.g. agricultural or pharmaceutical goods), and
free-trade-agreement eligibility. Conflicting-sovereign-authority cases
(origin-country export permit vs. destination-country import restriction)
produce `Indeterminate` and manual escalation exactly per RFC 0029 §36.12 —
no silent selection of the more convenient jurisdiction.

## 7. Policy kinds and composition

- **Authorization**: only an authenticated broker/importer of record may
  file or amend a declaration for their own shipment.
- **Numeric invariant**: duty assessed is a deterministic function of
  classification, value, and rate; assessed duty cannot go negative.
- **State invariant**: a shipment under active hold or seizure order cannot
  be released; a released shipment cannot be re-declared without an
  amendment workflow.
- **Transition**: `Filed → Classified → Assessed → (Held | Cleared) →
  Released → (Audited | Appealed)`.
- **Separation of duty**: the officer who places an inspection hold and the
  officer/system that clears it are independently authenticated where policy
  requires it (this is exactly the model-self-certification question tested
  in fixture `L6`).
- **External evidence**: sanctions/denied-party match and free-trade
  eligibility are external-evidence policy kinds.
- **Aggregate**: cumulative importer risk score, repeat-violation counters.

Composition follows RFC 0029 §19's normative defaults: a mandatory deny
(active seizure order) dominates any permit regardless of classification or
duty status.

## 8. Decisions, axes and model influence

The AI-assisted classification/inspection-priority model may:

- **Classify** a line item's candidate HS code (feeds a deterministic tariff
  lookup rule, not a direct decision — mirrors the KYC `K1` pattern);
- **Prioritize/Rank** shipments into an inspection queue;
- **Recommend** a hold or release to a customs officer, who independently
  reviews and authorizes (mirrors `M1`/`H2`/`I1` — fixture `L3`);
- autonomously place a **reversible** shipment/container hold within a
  bounded policy (mirrors the manufacturing quarantine pattern — fixture
  `L2`; a hold is reversible because it stops movement without disposing of
  goods);
- never autonomously **release** a shipment (fixture `L4`: a model-triggered
  release is classified `ExecuteIrreversible` — goods that have left bonded
  custody cannot be un-released — and is rejected as exceeding the profile's
  admitted ceiling of `Recommend` for that effect class);
- never **close its own inspection referral** (fixture `L6`: a model that
  flagged a shipment cannot also be the sole authority that clears it —
  self-certification is rejected exactly as in KYC's `K3`).

| Effect class | Admitted maximum model-influence tier |
| --- | --- |
| `line_item.classify` | `Classify` |
| `inspection.rank` | `Prioritize` |
| `shipment.hold` | `ExecuteReversible` |
| `cargo.release` | `Recommend` |
| `inspection.clear` | `Recommend`, and never by the model that raised the referral |

Axis A: `Runtime` for declaration/hold/release decisions, `Continuous` for
post-release audit sampling. Axis B: `Guarded` for declaration acceptance and
duty assessment, `DurableWorkflow` for the filed→released transition
sequence, `Atomic` only for the local duty-ledger posting once assessed.
Axis C: `ExternallyAttested` for sanctions/denied-party and free-trade
facts; `Local` for internally computed duty arithmetic.

## 9. Decision capability

A release capability binds: shipment/declaration identity and version,
authenticated officer (or the admitted autonomous-hold policy for `L2`-class
effects), duty-paid or bonded status, absence of active hold/seizure at
issuance, and single consumption at the physical/administrative release
trigger. Reuse after a declaration amendment or a new hold is rejected —
this is the exact `funds.reserve`-style capability-binding proof obligation
from RFC 0029 §36.3, applied to a release instead of a debit.

## 10. Obligations

Blocking: duty payment or bond sufficiency check before release capability
issuance; sanctions/denied-party check before declaration acceptance.
Post-commit: notify the bonded warehouse of release, schedule post-release
audit sampling, record classification for statistical reporting.

## 11. Finality, reversibility and compensation

Declaration acceptance and duty assessment are locally reversible by
correction/amendment (with history preserved, never overwritten). A hold is
reversible. **Release is irreversible** at the software-decision layer —
compensation after an erroneous release is a new enforcement/recovery action
(seizure, penalty, re-import demand), never a claim that the release was
undone. This mirrors the medical tabletop's "medication administered is
irreversible" finding and the insurance tabletop's F5 distributed-finality
requirement exactly.

## 12. Safety posture

Default: `FailClosed` for release (no release without a current, unexpired,
unrevoked capability). A `FailOperationalBounded` posture may be
meta-policy-authorized for perishable-goods spoilage risk, structurally
identical to the healthcare emergency-access posture — narrow scope, hard
expiry, mandatory post-hoc review — never an implicit exception.

## 13. Evidence

`AtomicEvidence` for the local duty-ledger posting; `DurableOutboxEvidence`
for the declaration/hold/release workflow record; `SynchronousExternalEvidence`
for sanctions-check acknowledgement where the sanctions authority's ack is
itself blocking. Forensic replay stores classification inputs and rule
matches, not full manifest contents beyond what the declared replay class
needs.

## 14. Continuous monitoring

Monitors: classification-model drift against a reference tariff table,
inspection-priority subgroup performance (e.g., disproportionate hold rates
by origin country — a fairness question structurally identical to KYC's A3),
sanctions-list freshness, and duty-assessment reconciliation against
payments received. Monitor blindness suspends the continuous claim per
RFC 0029 §36.10, not silently.

## 15. Confidentiality

Declaration contents, valuation, and importer identity are commercially
sensitive; sanctions-match evidence may be classified. Policy-plane
minimisation applies to the classification model's inputs exactly as
RFC 0029 §36.11 and RFC 0029.a §21 already require.

## 16. Budget

Declaration/hold/release decisions are low-latency, low-fan-out. Aggregate
importer-risk scoring is a bounded nightly/hourly window, not unbounded
history.

## 17. Conformance cases

| Test | Required response | Architecture result |
| --- | --- | --- |
| Deterministic declaration, no model, duty paid, no hold | Accept and release | `CoreExact` — fixture `L1`, mirrors banking `B1` |
| AI model places a reversible container hold pending review | Accepted at `ExecuteReversible`; officer may release | `CoreExact` — fixture `L2`, mirrors manufacturing `M2` |
| AI recommends release; officer independently authorizes | Accepted at `Recommend` with independent human authority | `CoreExact` — fixture `L3`, mirrors `M1`/`H2`/`I1` |
| AI model attempts to trigger release directly | Rejected: exceeds admitted ceiling for an irreversible effect | `CoreExact` — fixture `L4`, mirrors manufacturing `M3` |
| Active seizure order exists | Mandatory deny regardless of other facts | `CoreExact` — fixture `L5`, mirrors manufacturing `M4` |
| Model that flagged a shipment also tries to clear its own referral | Rejected: self-certification | `CoreExact` — fixture `L6`, mirrors KYC `K3` |
| Customs authority submission accepted locally, external ack times out | `UnknownOutcome`, reconcile, no duplicate release | `CoreExact` — fixture `L7`, mirrors insurance `INV_EXTERNAL_UNKNOWN` |
| Carrier manifest and broker declaration disagree on weight | Declared disagreement/precedence result, not incidental order | `ProfileSpecialization` — RFC 0029 §36.7, no new primitive found |
| Origin-country export permit conflicts with destination import restriction | `Indeterminate`, manual escalation | `ProfileSpecialization` — RFC 0029 §36.12, no new primitive found |
| Warehouse system releases a container without going through the declaration gateway | Coverage/bypass classification, not a silent pass | `ProfileSpecialization`/named exclusion — see §18; effect-taxonomy closure (§36.2) already anticipates administrative bypass paths |
| Inspection-priority model underperforms for one origin-country cohort | Detect, narrow/suspend, impact review | `ProfileSpecialization` via MAP subgroup assurance (§18 of RFC 0029.a); no new primitive found |
| Duty assessed retroactively changes after a tariff-table correction | Dependency invalidation and re-evaluation at the affected transition | `CoreExact` — RFC 0029 §36.8, no new primitive found |

No case in this table required a node kind, edge kind, decision class,
evidence-finality mode, safety posture, or effect-authority tier beyond what
already exists in the candidate schema and RFC 0029.a's C1–C6. That is a
result for this document's independent reviewer to verify, not a conclusion
this document is entitled to assert on its own (see §19).

## 18. Exclusions and unsupported guarantees

- Nirdosha does not determine whether a customs classification is legally
  correct, whether a sanctions match is substantively accurate, or whether an
  import restriction is lawful — those remain domain-authority judgments,
  exactly as RFC 0029 §1 and §4 already draw the line for every other domain.
- The warehouse-system direct-release bypass path named in §17 is not claimed
  covered by this document; it is named as a required entry in the effect
  closure and left as `Uncontrolled` or `AcceptedRisk` until a deployment's
  gateway inventory proves otherwise. This is intentional honesty, not a gap
  hidden by scope-narrowing.
- Physical custody and physical movement of cargo remain outside any
  software guarantee, structurally identical to the manufacturing tabletop's
  refusal to let a database row claim it isolated hazardous energy.
- This document does not certify any real customs authority's system.

## 19. Current representation verdict

Every conformance case in §17 was representable using only node kinds,
edge kinds, decision classes, evidence-finality modes, safety postures, and
effect-authority tiers that already exist for banking, KYC, healthcare,
manufacturing, and insurance. No case in this domain-language pass required
proposing a new universal primitive. This is this document's author's
finding, produced *without* consulting an independent reviewer, and is
therefore in-sample-style evidence about a genuinely out-of-sample domain —
useful, but not sufficient on its own. The independent review commissioned
under the readiness matrix's out-of-sample gate (§2) must reach this
conclusion on its own, actively trying to find a counterexample, before it
carries any weight toward promoting the MAP core from `Candidate baseline`
to `Validated core`.
