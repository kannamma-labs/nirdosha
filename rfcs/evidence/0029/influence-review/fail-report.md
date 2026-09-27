# RFC 0029 — Independent Review of Model Influence and Human Review Semantics

```
Reviews:       evidence/0029/influence-review/original-semantics.md
Status:        FAIL — blocking semantic corrections required
Reviewed:      2026-09-27
Method:        Independent read-only review against all five domain profiles
```

## 1. Verdict

The candidate semantics are directionally strong but do not yet guarantee
identical classification for semantically equivalent lineage graphs. A1
(material model influence) and A2 (observable human review) remain
`Ambiguous`. The Phase 0 no-ambiguity and independent-review gates do not pass.

No runtime or production capability is implied by this review.

## 2. Blocking findings

### R1 — representation-dependent influence classification

The material-path rule and terminal authority mapping do not define a
canonical fold over an edge sequence. Equivalent behavior can receive a
different level when neutral `PolicyRule`, `Decision`, or `Capability` nodes
are inserted or when overlapping edge labels are selected.

Required correction: define typed endpoint constraints and a canonical
path-folding function invariant under allowed neutral-node decomposition.

### R2 — overlapping node and edge semantics

`Input` overlaps `DerivedFeature` and other relations; `Approval` overlaps
`Authorization`; `DraftContent` overlaps `ReasonGeneration`. Endpoint kinds,
cardinality, temporal direction, exclusivity, precedence and illegal
combinations are unspecified.

Required correction: provide a normative edge table with exact source/target
kinds, meaning, exclusivity and composition. Structural edges must not
accidentally alter authority classification.

### R3 — undefined adjudication/authority transition

The semantic-break rule refers to an admitted adjudication that creates an
authoritative fact, but the graph has no adjudication node, transition record
or break predicate. Provenance retention, influence on adjudication, new fact
authority and downstream influence are conflated.

Required correction: add a universal typed authority-assertion/adjudication
transition or define it exactly from existing primitives. This is a newly
implied universal abstraction and reopens the “no new primitive” conclusion.

### R4 — non-deterministic `ObservationalOnly` proof

The separation evidence is illustrative, not necessary and sufficient. It
does not close over the system boundary, dependencies, shared state,
scheduling/resource channels, configuration or freshness/invalidation.

Required correction: define a closed non-interference claim containing the
boundary, dependency graph, trusted enforcement, allowed channels, proof
artifact, validity and conservative behavior for unmodeled channels.

### R5 — model influence conflated with final effect authority

The capability rule says reviewed authority cannot exceed the upstream model
influence ceiling. That incorrectly prevents an independently authorized
human from approving an effect after receiving a recommendation.

Required correction: carry independent fields for computed model-influence
level and final capability authority/authority chain. Model influence limits
what the model authorizes; it does not cap independent domain authority.

### R6 — human-review modes not deterministically evaluated

`IndependentDecision` includes the `BoundedApproval` requirements while modes
are described as non-ranked. Required predicates such as qualification,
independence, availability, capacity and sampling are not typed references.

Required correction: make review mode a declared contract that passes or
fails, not an inferred exclusive classification. Every profile predicate must
have identity, version, authority, evaluation time and failure behavior.

### R7 — incomplete invalidation semantics

“Expire or re-evaluate” leaves the action unspecified and does not cover
concurrent invalidation/consumption, post-consumption discovery, unknown
external acceptance or retrospective authority revocation.

Required correction: bind each dependency to `Deny`, `Reevaluate`, `Suspend`
or `Reconcile`; define atomic reservation/consumption and retrospective impact
behavior.

### R8 — golden cases are not canonical determinism tests

Several cases leave the exact level or response to downstream policy. There
are no canonical typed graphs, expected diagnostics, capability outcomes or
neutral-decomposition equivalence pairs. Banking lacks an explicit negative
model-influence case.

Required correction: encode canonical graphs/review records and exact outputs,
including equivalent graph pairs and every profile's `None`, admitted and next
prohibited levels.

## 3. Additional required corrections

- Rubber-stamp predicates need window identity, minimum sample, baseline,
  missing-data behavior and monitor authority.
- “Evidence made available” and “evidence accessed” remain different facts;
  access proves interaction, not comprehension.
- Status documents must not mark A1/A2 or the no-ambiguity gate complete until
  the corrected semantics pass another independent review.

## 4. Design choices that passed

- Human review does not erase upstream model provenance.
- Observable protocol compliance is separate from opaque cognition and
  professional correctness.
- Missing or untrusted lineage fails conservatively.
- Domain safety, legal, clinical and payment authority remains controlling.
- Model identity and inference receipts bind into affected capabilities.
- Model participation is per decision/effect rather than a binary domain
  property.

## 5. Re-review gate

Re-review requires a revision that addresses R1–R8 and supplies canonical
golden fixtures. The next reviewer must test representation invariance,
authority separation, non-interference proof validity, declared review-mode
evaluation, invalidation races and exact diagnostics. Only a passing re-review
may change A1/A2 to `CoreExact` and close the no-ambiguity gate.

Candidate corrections are recorded in
[Revision 1](../../../0029.a-model-assurance-port.md#model-influence-and-human-review-semantics-revision-1) and its
[golden-fixture specification](../../../0029-canonical-ir-and-conformance-vectors.md#canonical-influence-and-human-review-golden-fixtures).
Their presence does not change this report's FAIL verdict; re-review is still
required.
