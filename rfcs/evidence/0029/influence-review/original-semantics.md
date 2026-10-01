# RFC 0029 — Model Influence and Human Review Semantics

```
Extends:       RFC 0029.a model-influence overlay
Status:        Candidate semantics; independent review failed, revision required
Created:       2026-09-27
Targets:       Field-classification ledger A1 and A2
```

> **Revision 1:** The candidate corrections for review findings R1–R8 are in
> [Model Influence and Human Review Semantics, Revision 1](../../../0029.a-model-assurance-port.md#model-influence-and-human-review-semantics-revision-1),
> with canonical cases specified in the
> [golden-fixture companion](../../../0029-canonical-ir-and-conformance-vectors.md#canonical-influence-and-human-review-golden-fixtures).
> This original document remains the reviewed baseline; Revision 1 controls
> where the two differ and awaits independent re-review.

## 1. Purpose

This document defines interoperable semantics for:

1. deciding whether and how a model materially influences a protected
   decision or effect; and
2. deciding whether an observable human-review protocol satisfies a declared
   review mode.

It does not claim philosophical causality, human cognition, professional
correctness, or absence of undisclosed influence. It defines typed lineage,
observable protocol evidence, conservative unknown behavior and exact
authority consequences.

## 2. Decision-lineage graph

A protected decision carries a finite directed acyclic graph:

```text
G = (N, E)
```

Every node has a stable ID, kind, authority, version or commitment, and
classification. Every edge has a kind and source/target node.

### 2.1 Node kinds

```text
AuthoritativeFact
AssertedFact
ModelInput
ModelOutput
DerivedValue
PolicyRule
CandidateSet
Presentation
HumanReview
Decision
Obligation
Capability
Effect
Evidence
```

`ModelOutput` includes a model-generated feature consumed by deterministic
code. Relabeling it `DerivedValue` does not remove its model provenance.

### 2.2 Edge kinds

```text
Input
DerivedFeature
CandidateGeneration
CandidateSuppression
Eligibility
Ranking
AttentionAllocation
DefaultSelection
Recommendation
DraftContent
ReasonGeneration
Approval
EffectParameter
EffectTarget
EffectTiming
EffectTrigger
Authorization
EvidenceOf
ObservationalOnly
```

All kinds except `EvidenceOf` and `ObservationalOnly` are material-influence
edges. `EvidenceOf` records provenance after or about a decision and does not
by itself influence that decision.

## 3. Graph well-formedness

Admission requires:

- a finite acyclic graph for every protected decision/effect instance or
  replay-equivalent graph template;
- every model-derived value to retain a path from its `ModelOutput`;
- every `Decision`, `Capability` and `Effect` to be reachable from its actual
  inputs, rules, presentation and review nodes;
- every model-influenced capability to bind the relevant model/deployment and
  inference receipt;
- every omitted/filtered candidate, hidden fact, ranking/default and generated
  reason that can affect the result to appear as lineage;
- no edge kind unknown to the active schema/profile;
- no authority level inferred from display names or implementation type.

Missing, cyclic, contradictory or untrusted lineage produces
`InfluenceUnknown` and an `Indeterminate` result for claims requiring `None`
or a bounded influence level.

## 4. Material influence algorithm

For a protected target `t` (`Decision`, `Obligation`, `Capability` or
`Effect`) and model-output node `m`:

```text
material_path(m, t) =
    exists directed path m → t containing at least one material edge
    and containing no semantic break that converts the value into an
    independently authoritative fact through an admitted adjudication.
```

An ordinary transformation, deterministic rule, threshold, human display or
human approval is not a semantic break. An admitted adjudication creates a new
authoritative fact only when the domain profile names its authority, evidence,
correction and independence semantics; the original model lineage remains in
the adjudication evidence.

The binding is:

```text
if lineage invalid/incomplete/untrusted:
    InfluenceUnknown
else if no ModelOutput reaches t through a material edge:
    None
else:
    strongest authority level among all material paths to t
```

The strongest level is computed after mapping each terminal path to the exact
decision/effect authority in §5. Multiple model paths take the maximum; a
domain prohibition overrides the computed level with rejection.

## 5. Terminal authority mapping

| Material terminal relationship | Minimum binding |
| --- | --- |
| Observation displayed without domain classification, ranking or decision use | `Observe` |
| Label/category/derived feature used by target | `Classify` |
| Queue/order/urgency/attention changed | `Prioritize` |
| Action suggested without creation of an actionable artifact | `Recommend` |
| Draft action, notice, order, work item or reason created | `Draft` |
| Output counts as an approval for the target effect | `Approve` |
| Output directly authorizes/triggers a proven reversible effect | `ExecuteReversible` |
| Output directly authorizes/triggers a compensatable effect | `ExecuteCompensatable` |
| Output directly authorizes/triggers an irreversible effect | `ExecuteIrreversible` |

When a path has several relationships, use the strongest. For example, a
classification that changes queue priority is at least `Prioritize`; a
recommendation that automatically triggers quarantine is
`ExecuteReversible`.

## 6. `ObservationalOnly`

`ObservationalOnly` is admissible only when enforced separation proves the
model output cannot affect candidate generation/suppression, eligibility,
ranking, attention, presented information, defaults, reasons, decision
inputs, obligations, capability fields, effect parameters/target/timing or
whether the effect occurs.

Acceptable evidence includes an isolated shadow sink, one-way audit channel,
separate credentials, no readable return path, and coverage showing no shared
mutable decision state. Merely saying “for analytics” or hiding the output
from the final approver is insufficient.

If the proof is absent, the edge cannot be `ObservationalOnly` and the result
is `InfluenceUnknown` or the strongest evidenced material level.

## 7. Human-review modes

```text
Acknowledgement
BoundedApproval
IndependentDecision
```

The modes are not a quality ranking and never prove cognition.

### 7.1 Acknowledgement

The reviewer confirms receipt, awareness or completion of a procedural step.
It requires authenticated identity, timestamp and acknowledged artifact. It
does not satisfy a policy requiring approval, adjudication or independent
review.

### 7.2 BoundedApproval

The reviewer accepts, rejects, modifies or escalates a proposal inside an
exact authority/effect boundary. Required fields are:

- qualified reviewer and current delegated authority;
- independence constraints declared by the profile;
- source evidence and model limitations made available;
- explicit action rather than passive timeout/default;
- structured reason bound to source evidence and applicable rule;
- disagreement/override/escalation paths;
- current capacity and response-bound evidence;
- sampling/quality-control contract.

The resulting capability cannot exceed the bounded proposal/effect class.

### 7.3 IndependentDecision

The reviewer selects the decision under an independent domain authority.
`BoundedApproval` requirements apply, plus:

- the reviewer can reach every admitted outcome without model permission;
- authoritative source evidence is available separately from model summary;
- the reviewer records an affirmative evidence-bound reason;
- the model/provider/originating authority cannot be the sole reviewer where
  separation is required;
- the decision is represented as the reviewer's decision while preserving all
  upstream model lineage.

Independent review does not erase influence. If the model selected evidence,
ranked attention, supplied defaults or drafted reasons, the decision remains
model-influenced at the computed level.

## 8. Review evidence and validation

A review record binds:

- review ID, mode, decision/effect and bundle;
- reviewer identity, qualification, role and authority version;
- independence/equivalence checks;
- source-evidence IDs/versions made available and accessed as required;
- model outputs, uncertainty, limitations and alternatives shown;
- permitted actions and actual action;
- structured reason code and evidence/rule references;
- override/disagreement/escalation availability and use;
- start/finish times and applicable response bound;
- queue/load/capacity state;
- quality-control/sampling assignment;
- invalidation set and review-result capability.

Validation is exact over these observable fields. The quality or sincerity of
human reasoning remains `Opaque`.

## 9. Review findings

Stable findings are:

```text
ReviewerUnauthorized
ReviewerQualificationExpired
IndependenceConflict
RequiredEvidenceUnavailable
RequiredEvidenceNotAccessed
ModelLimitationsHidden
ActionOutsideReviewAuthority
ReasonMissing
ReasonNotEvidenceBound
OverrideUnavailable
EscalationUnavailable
ReviewDeadlineMissed
CapacityExceeded
RubberStampPattern
SelfCertification
ReviewEvidenceInvalidated
```

The first twelve are instance/protocol failures where their predicate holds.
`RubberStampPattern` is a continuous/retrospective finding and does not by
itself prove a specific past decision wrong. Policy declares whether a finding
denies the current effect, adds a reviewer, narrows/suspends dependent
automation, opens investigation or triggers re-review.

## 10. Capacity and rubber-stamp contract

The profile declares reviewer qualification/independence, maximum queue/load,
response bounds, evidence-access requirements, override authority,
disagreement path and sampling. Observable indicators may include:

- review queue/load beyond the declared capacity;
- required evidence never accessed;
- repeated identical reasons inconsistent with case evidence;
- response time outside a profile-declared plausible protocol interval;
- near-zero disagreement or override over a bounded window;
- acceptance despite contradictory authoritative evidence;
- the same model/authority creating and closing a case where independence is
  mandatory.

No individual indicator proves cognition. The admitted policy combines them
as bounded monitor predicates and names the response. Unknown monitor health
suspends the continuous review-quality claim.

## 11. Capability and invalidation semantics

A capability produced after review binds the review record and cannot exceed
the reviewer's authority or the upstream model-influence ceiling. It expires
or re-evaluates when required source evidence, resource version, reviewer
authority, model identity/effective state, policy bundle, applicability or
capacity dependency changes.

Changed evidence with the same review/capability ID is rejected. Idempotent
retry returns the prior result and receipt.

## 12. Admission and runtime algorithm

For every protected decision/effect:

1. validate the lineage graph;
2. find all model-output paths to the target;
3. classify `None`, `InfluenceUnknown`, or strongest influence level;
4. compare the level with the declared binding and domain maximum;
5. validate required MAP identity, receipt, promises and effective state;
6. if review is required, validate the declared review mode and record;
7. apply review findings and capacity/monitor state;
8. bind the computed influence and review evidence into the capability;
9. reject consumption for a stronger/different effect or changed input;
10. emit decision, review and consumption evidence.

Under-declaration is rejected. Over-declaration may be accepted conservatively
if its stronger assurance requirements and domain authority limits pass; it
does not grant stronger effect authority.

## 13. Stable diagnostics

```text
InfluenceLineageMissing
InfluenceLineageInvalid
InfluenceUnknown
InfluenceUnderDeclared
InfluenceLevelExceedsProfile
ObservationalSeparationUnproved
ModelReceiptMissingOrMismatched
ReviewModeInsufficient
ReviewProtocolFailed
ReviewCapacityUnhealthy
ReviewSelfCertification
ReviewEvidenceInvalidated
```

Diagnostics include graph/review paths, target decision/effect, declared and
computed levels, failed predicates and affected capability.

## 14. Cross-domain golden cases — influence

| ID | Domain case | Required result |
| --- | --- | --- |
| I1 | Model writes to isolated shadow log with no return path | `None`; `ObservationalOnly` proven |
| I2 | KYC model-derived feature enters deterministic acceptance rule | At least `Classify`; not `None` |
| I3 | KYC model changes analyst queue order | `Prioritize` |
| I4 | Healthcare model prioritizes worklist and emits advisory alert | `Recommend` (stronger terminal relation) |
| I5 | Healthcare model creates non-executable order draft | `Draft` |
| I6 | Manufacturing model recommends maintenance only | `Recommend` |
| I7 | Manufacturing recommendation automatically triggers bounded quarantine | `ExecuteReversible` |
| I8 | Insurance model drafts adverse notice reason | `Draft` plus explanation assurance |
| I9 | Insurance model output counts as bounded low-value approval | `Approve` |
| I10 | Model selects evidence hidden from reviewer | Material; level derived from downstream target |
| I11 | Model supplies default accepted by human | At least `Draft`; review does not erase lineage |
| I12 | Model path or derived-feature provenance is missing | `InfluenceUnknown` and `Indeterminate` |
| I13 | Recommendation capability presented for payment/restart/treatment | Reject stronger-effect consumption |
| I14 | Admitted adjudicator creates authoritative fact from model-assisted case | Original influence retained in evidence; downstream fact uses adjudicator authority |

## 15. Cross-domain golden cases — human review

| ID | Domain case | Required result |
| --- | --- | --- |
| H1 | Reviewer merely clicks acknowledge | Passes `Acknowledgement`; fails approval/independent-review requirement |
| H2 | KYC analyst lacks source identity evidence | `RequiredEvidenceUnavailable`; review fails |
| H3 | KYC model opens and closes its own alert | `SelfCertification`; reject |
| H4 | Clinician sees only generated summary, not required observations | Review protocol fails |
| H5 | Clinician independently signs treatment after source review | Observable `IndependentDecision` may pass; clinical correctness remains `Opaque` |
| H6 | Manufacturing supervisor approves while personal lock remains | Review cannot override safety deny; reject effect |
| H7 | Insurance adjuster provides evidence-bound disagreement | Independent protocol passes; model lineage retained |
| H8 | Insurance reason is fluent but not bound to actual clause/rule | `ReasonNotEvidenceBound`; reject notice/effect |
| H9 | Reviewer role expired before effect | `ReviewerQualificationExpired`; invalidate/re-review |
| H10 | Evidence changes after review | `ReviewEvidenceInvalidated`; re-review |
| H11 | Queue exceeds declared capacity | `CapacityExceeded`; narrow/suspend dependent automation |
| H12 | Repeated click-through pattern while monitor healthy | `RubberStampPattern`; apply declared bounded response, not automatic cognition claim |
| H13 | Review-quality monitor is blind | Suspend continuous review-quality claim and follow declared posture |
| H14 | Qualified reviewer reasons incorrectly despite valid protocol | Protocol may pass; substantive judgment remains `Opaque` |

## 16. Properties required of implementations

- Determinism: equivalent canonical graphs/review records produce the same
  classification and diagnostics.
- Monotonicity: adding a material path cannot lower influence.
- Conservation: transformations and human review do not erase provenance.
- Authority safety: computed influence cannot grant effect authority.
- Conservative unknown: missing/untrusted lineage never becomes `None`.
- Review honesty: protocol conformance never becomes a cognition/correctness
  claim.
- Invalidation: changed bound input/authority/model/evidence prevents reuse.
- Domain floor: safety, legal, professional and payment authorities remain
  controlling.

## 17. Resolution and remaining gate

These semantics propose to resolve ledger A1 by a finite typed reachability
calculation and A2 by separating observable review protocol from opaque
reasoning quality. The independent
[review](./fail-report.md) found eight blocking issues, so
the following target classifications are not yet accepted:

| Concept | Classification |
| --- | --- |
| Decision-lineage graph and materiality algorithm | `CoreExact` |
| Domain edge/resource/effect vocabulary | `ProfileSpecialization` |
| Observable review modes/protocol | `CoreExact` |
| Human reasoning/professional correctness | `Opaque` |
| Missing/untrusted lineage or review evidence | `Indeterminate` |
| Authority escalation/self-certification | `Unsupported` |

Phase 0 schema freeze requires R1–R8 to be corrected and a passing independent
re-review. This document alone implements no runtime behavior.
