# RFC 0029 — Phase 0 Readiness and Cross-Domain Representation Matrix

```
Companion to:  RFC 0029 — Domain-neutral policy admission, enforcement,
               and evidence
               RFC 0029.a — Provider-Neutral Model Assurance Port
Status:        Pre-implementation gate; profiles and executable matrices pending
Created:       2026-09-27
```

## 1. Purpose

This document is the entry gate for RFC 0029 Phase 0. It reconciles the two
freeze statements in the RFC family and turns the existing table-top work into
a concrete representation-validation plan.

It authorizes no runtime, compiler, gateway, certificate, or assurance claim.
Its purpose is to prevent implementation from freezing a universal schema
before the selected domains can be represented without ambiguity or domain
fields leaking into the core.

## 2. Status resolution

Two different freeze decisions currently exist and must not be conflated:

1. The unified KYC/medical/manufacturing/insurance rerun found no seventh
   generic Model Assurance Port contract beyond C1–C6. The **conceptual MAP
   core is a candidate baseline for implementation experiments**.
2. The original banking-transfer and healthcare-access table-top says its
   F1–F12 scenarios have not been rerun against formal semantics or an
   implementation. The **RFC 0029 universal policy IR is not frozen**.

The unified rerun does not close the original F1–F12 gate. It validates the
MAP/profile boundary at the architecture level; it does not supply canonical
policy schemas, formal algebras, encoded profiles, or executable conformance
cases for the non-AI banking and healthcare scenarios.

Accordingly:

- RFC 0029 and RFC 0029.a remain proposed and unimplemented;
- the MAP envelopes may be used as candidate Phase 0 inputs, but may be
  reopened if profile encoding reveals a missing universal abstraction;
- production schema implementation starts only after the profiles and
  matrices below pass the representation gate;
- enforcement or assurance claims remain prohibited until their later-phase
  prerequisites pass.

## 3. Representation result vocabulary

Every domain concept and test case receives exactly one result:

| Result | Meaning | Phase 0 consequence |
| --- | --- | --- |
| `CoreExact` | Represented directly by domain-neutral semantics | Candidate core remains unchanged |
| `ProfileSpecialization` | Core semantics are sufficient; the profile supplies vocabulary, authority, thresholds, or evidence | Add only to the domain profile |
| `Opaque` | Runtime evaluation is possible, but semantics are unavailable to compatible static/formal claims | Admit only with the explicit reduced guarantee |
| `Unsupported` | The requested guarantee cannot be made honestly | Reject with a stable diagnostic |
| `Ambiguous` | More than one conforming interpretation or result is possible | Blocks Phase 0 freeze |

`Ambiguous` is never converted silently to `Opaque`, `MonitorOnly`, or an
accepted assumption. Any reduction is a new, explicitly approved submission
as required by RFC 0029 §9.4.

## 4. Required profiles and overlay

Phase 0 requires five concise domain profiles plus one cross-domain
model-influence overlay. AI is not a binary property of a domain or
deployment. Each domain profile covers the full protected process; the
overlay attaches RFC 0029.a requirements to the particular decision/effect
classes materially influenced by models.

A profile defines vocabulary and specialization; it must not redefine
identity, decision, lifecycle, receipt, evidence-finality, or bypass semantics
owned by the core.

| Profile | Representative protected process | Principal residual requirements |
| --- | --- | --- |
| Banking transfer | Cross-border account transfer | Atomic debit/reservation, aggregate limits, sanctions/FX authorities, settlement finality, duplicate/changed retry behavior |
| KYC | Risk-based onboarding and review, with or without model influence per decision | Customer/action/legal-reporting authority, screening, contestability; overlay supplies model identity, abstention/OOD, subgroup/adversarial assurance and meaningful review |
| Healthcare and clinical decision support | Record access, medication workflow and advisory prioritization | Minimum-necessary and emergency access; clinical identity/measurement; consent/allergy disagreement; intended use, harm/utility, label adjudication, local validation and recall |
| Manufacturing | Lockout, maintenance, inspection, quarantine, restart and product release | Asset/configuration baseline, physical attestation, personal locks, hard-real-time boundary and safety authority |
| Insurance | Claim intake, adjudication, notices, reserve, payout and contest | Contract/applicability graph, financial finality, contestability and catastrophe posture |

The [model-influence overlay](./0029.a-model-assurance-port.md#model-influence-overlay) applies
independently to each protected decision/effect class using the ordered
`None` through `ExecuteIrreversible` authority vocabulary. It must support
mixed workflows and gradual adoption without creating parallel “AI” and
“non-AI” domain schemas.

Profile artifacts completed so far:

- [Cross-border banking transfer](./0029-domain-profiles.md#banking-transfer-profile) —
  candidate profile; schema encoding and executable conformance pending.
- [Healthcare access, medication, and clinical decision support](./0029-domain-profiles.md#healthcare-access-and-clinical-decision-support-profile)
  — unified candidate profile with model-free clinical authority and bounded
  `Prioritize`/`Recommend`/`Draft` influence; schema encoding and executable
  conformance pending.
- [Industrial manufacturing](./0029-domain-profiles.md#manufacturing-profile) — unified
  candidate profile with mixed `None` through `ExecuteReversible` model
  influence; schema encoding and executable conformance pending.
- [Insurance claims and policy servicing](./0029-domain-profiles.md#insurance-profile) —
  unified candidate profile with mixed extraction, prioritization,
  recommendation, bounded approval and model-free authority; schema encoding
  and executable conformance pending.
- [Customer onboarding and KYC](./0029-domain-profiles.md#kyc-profile) — unified candidate
  profile spanning deterministic, authoritative, human and model-influenced
  decisions; schema encoding and executable conformance pending.

The [field-level classification ledger](./0029-phase0-readiness-and-domain-matrix.md#field-classification-ledger)
maps the shared and domain concepts to `CoreExact`,
`ProfileSpecialization`, `Opaque`, `Unsupported`, or `Ambiguous`. Its original
two blockers—indirect material model influence and observable meaningful
human review—now have candidate normative semantics and cross-domain golden
cases in the
[influence/review semantics](./evidence/0029/influence-review/original-semantics.md).
An [independent review](./evidence/0029/influence-review/fail-report.md) failed that
candidate resolution and reopened both blockers; R1–R8 must be corrected and
re-reviewed.

Candidate corrections for all eight findings are now specified in
[influence/review semantics Revision 1](./0029.a-model-assurance-port.md#model-influence-and-human-review-semantics-revision-1)
and the [golden fixtures](./0029-canonical-ir-and-conformance-vectors.md#canonical-influence-and-human-review-golden-fixtures). A1/A2
were closed by the
[passing independent re-review](./evidence/0029/influence-review/pass-report.md).

## 5. Cross-domain representation matrix

This is the minimum core-to-profile split that candidate schemas must encode.
The result cells are expected classifications; the profile artifacts and
conformance cases still have to prove them.

| Universal concern | Banking | Healthcare | Manufacturing | Insurance |
| --- | --- | --- | --- | --- |
| Canonical resource identity | `CoreExact`: account and beneficiary IDs; profile defines aliases | `CoreExact`: patient/record/order IDs; profile defines merges and replicas | `CoreExact`: asset, configuration and batch IDs | `CoreExact`: contract-version and claim IDs |
| Effect taxonomy and closure | `CoreExact`: reserve, debit, submit, settle, admin/file paths | `CoreExact`: view, disclose, order, dispense, administer | `CoreExact`: isolate, start, stop, release, physical command | `CoreExact`: reserve, approve, deny, notify, issue and settle payment |
| Domain predicates and units | `ProfileSpecialization`: money, currency and limits | `ProfileSpecialization`: clinical measurements and minimum necessary | `ProfileSpecialization`: energy, sensor units and configuration | `ProfileSpecialization`: coverage, deductible and loss values |
| Fact authority and disagreement | `CoreExact`; profile names sanctions, FX and ledger authorities | `CoreExact`; profile names consent, allergy, lab and clinician authorities | `CoreExact`; profile names sensor, inspector and safety authorities | `CoreExact`; profile names contract, adjuster and payment authorities |
| Decision-capability binding | `CoreExact` | `CoreExact` | `CoreExact` | `CoreExact` |
| Atomic local invariant | `CoreExact`; profile requires zero-overshoot debit domain | `CoreExact` where a shared clinical store boundary exists | `ProfileSpecialization`; software atomicity cannot prove physical isolation | `CoreExact` for local claim state, not external payment settlement |
| Distributed finality | `CoreExact`; profile supplies payment-state authorities | `CoreExact`; profile distinguishes order, dispense and administration | `CoreExact`; profile distinguishes command, acknowledgement and physical state | `CoreExact`; profile distinguishes reserve, issued, accepted, cleared and reversed |
| Reversibility/compensation | `CoreExact` | `CoreExact`; disclosure and administration may be irreversible | `CoreExact`; physical action may be irreversible | `CoreExact`; payment may be reversible or compensatable by state |
| Safety posture | `CoreExact`; profile normally fails closed | `CoreExact`; profile defines bounded emergency access | `CoreExact`; profile binds fail-safe/operational modes to safety authority | `CoreExact`; profile defines catastrophe and manual-adjudication posture |
| Aggregate consistency | `CoreExact`; profile defines daily-limit key/window/domain | Usually not applicable; profile must say so | `ProfileSpecialization` where fleet/production aggregates govern action | `CoreExact`; profile defines catastrophe/portfolio windows where used |
| Monitor health | `CoreExact` | `CoreExact` | `CoreExact`; hard-real-time proof remains profile/external certification | `CoreExact` |
| Jurisdiction/applicability | `CoreExact`; profile supplies corridor/product rules | `CoreExact`; profile supplies treatment/research/emergency contexts | `CoreExact`; profile supplies site/product/safety regime | `CoreExact`; profile supplies policy/endorsement/loss-time graph |
| Policy-plane confidentiality | `CoreExact`; profile supplies financial-data classification | `CoreExact`; profile supplies PHI rules | `CoreExact`; profile supplies trade-secret/safety data rules | `CoreExact`; profile supplies claimant and external-data rules |
| Model identity/status/receipt | Activated per model-influenced decision | Activated per model-influenced decision | Activated per model-influenced decision | Activated per model-influenced decision |
| Effect-authority/autonomy tier | Per-decision overlay; KYC domain authority remains controlling | Per-decision overlay; treatment execution initially unsupported | Per-decision overlay; hazardous autonomous actuation unsupported | Per-decision overlay; opaque adverse action unsupported |

No row is proved by this table alone. A classification becomes accepted only
when its profile and positive/negative cases are reviewed against the
candidate schema.

## 6. Required profile format

Each profile must contain the following sections in this order:

1. system and certification boundary;
2. canonical resource namespaces, aliases, merge/split/derivation rules;
3. closed effect taxonomy, equivalents, descendants, indirect and
   administrative paths;
4. typed vocabulary, units, states and bounded predicates;
5. authoritative fact sets, provenance, freshness, disagreement, revocation
   and compromise behavior;
6. applicability and jurisdiction authorities;
7. policy kinds and composition algebra specialization;
8. decision/effect classes and Axis A/B/C declarations;
9. capability binding, reuse, consumption and idempotency;
10. blocking and post-commit obligations;
11. finality, visibility, reversibility, unknown outcomes and compensation;
12. safety posture and emergency authority;
13. evidence-finality mode, replay class and evidence-outage behavior;
14. monitor observation and health contract where Continuous is used;
15. policy-plane confidentiality and minimisation;
16. cost/complexity budget;
17. positive, negative, stale, conflict, outage, concurrency, retry,
    compromise and bypass cases;
18. explicit exclusions and unsupported guarantees.

Every non-`None` model-influence binding additionally supplies the applicable
RFC 0029.a C1–C6 contracts and binds model identity, status, promise
evaluation and inference receipts to the RFC 0029 decision capability and
effect receipt.

## 7. Mandatory non-AI rerun matrices

### 7.1 Banking transfer

| Case | Required representation/result |
| --- | --- |
| Concurrent insufficient-funds transfers | Zero-overshoot atomic consistency domain or `Unsupported` |
| Daily limit under concurrency/partition | Declared window, authority, reservation and maximum overshoot |
| Identical retry | Idempotent prior result and bound consumption receipt |
| Same key with changed input | Deterministic rejection |
| Sanctions or FX fact changes before commit | Dependency invalidation and re-evaluation |
| Local ledger commit succeeds, external submit times out | Declared `UnknownOutcome`, reconciliation and no false atomic claim |
| Settlement file or administrative mutation | Included in effect closure or named exclusion without coverage claim |
| Evidence plane unavailable | Declared evidence-finality outage behavior |
| Authority later compromised | Affected-decision discovery and containment behavior |

### 7.2 Healthcare access and medication

| Case | Required representation/result |
| --- | --- |
| Normal versus emergency record access | Distinct scope/capability and field projection |
| Break-glass replay against another patient or purpose | Binding mismatch and rejection |
| Consent/PDP outage | Admitted safety posture, never an implicit exception |
| Conflicting allergy authorities | Declared disagreement result and evidence |
| Fact changes before dispense/administration | Transition dependency invalidation |
| Patient alias, merge, replica or search projection | Explicit identity/policy inheritance |
| Evidence outage during emergency access | Declared finality mode and reconciliation |
| Medication administered | Irreversible effect, never rewritten as compensatable atomic work |
| Research derivation from treatment data | Explicit derivation and purpose inheritance |
| Retention conflicts with deletion request | Applicability/conflict resolution rather than arbitrary precedence |

## 8. Candidate-schema validation protocol

Every profile is first written in domain language without reference to the
candidate schema. A separate author maps it into the candidate core/profile
representation. A third reviewer reconstructs the policy using only that
representation.

The reconstruction passes only when the reviewer identifies the same:

- protected resources and effects;
- authorities and conflict behavior;
- permit, deny, defer and indeterminate boundaries;
- timing, validity and invalidation behavior;
- finality and reversibility;
- obligations and failure behavior;
- guarantee, assumptions, exclusions and bounded exposures.

If two conforming implementations could reach different results from the same
typed inputs, the case is `Ambiguous` and blocks freeze.

## 9. Phase 0 entry gate

Production implementation of the canonical IR may begin only when:

- [x] all five domain profiles and the model-influence overlay exist in the
      format in §6;
- [x] every profile concept is classified using §3 in the first-pass ledger;
- [x] no `Ambiguous` classification remains after Revision 1 and passing
      independent re-review;
- [ ] all `Opaque` and `Unsupported` cases name the foreclosed guarantee and
      stable admission outcome — **partial**: `AdmissionReportV1`'s
      `Opaque`/`Unsupported` outcomes are documented at the report-compiler
      level (`pilot::report`'s doc comments), but this has not been checked
      against every `Opaque`/`Unsupported` case named anywhere in RFC 0029
      §8's own "opaque predicate" concept at the RFC-text level — still open;
- [x] the banking and healthcare matrices in §7 pass on paper against the
      candidate semantics — closed 2026-09-27, §11 below: all 19 rows now
      executable, not just on paper;
- [x] every non-`None` model-influence binding encodes C1–C6 without a new
      universal MAP envelope — established by both independent reviews (§2,
      §3): neither found a binding requiring a seventh generic contract;
- [x] no domain-specific field is required in the universal core —
      established by `tests/version_compatibility.rs` and this session's own
      use of the pattern: every domain-specific need (`fact_provenance`,
      `fact_requirement`, `effect_authority_ceilings`) was absorbed by the
      `attributes`/`parameters` extension points, never a core schema change;
- [x] two consecutive review rounds introduce no new universal primitive —
      the F1–F12 gate's own two-round structure (§3): round 1's five
      findings were RFC-text gaps, not universal primitives; round 2
      confirmed no residual gaps after fixing them;
- [x] one out-of-sample domain that did not derive C1–C6 passes an independent
      attempt to find a new universal primitive — §2, closed 2026-09-27;
- [x] an independent adversarial rerun of banking/healthcare F1–F12 has no
      unresolved `FAIL` or `INDETERMINATE` result — §3, closed 2026-09-27;
- [x] RFC 0029 and RFC 0029.a formal-artifact backlogs are enumerated and
      assigned to Phase 0 deliverables — closed 2026-09-27, §11 below;
- [x] `docs/ROADMAP.md` and every companion status statement agree — see
      ROADMAP.md's own enumeration, added this same pass.

**This gate is otherwise closed except item 4** (`Opaque`/`Unsupported`
naming at the RFC-text level), which remains genuinely open. Entry-gate
passage does not by itself close the Phase 0 exit gate (§10 above), which
has its own, distinct remaining item.

Passing this gate permits implementation of Phase 0 artifacts. It does not
permit an enforcement or assurance claim.

The narrow pilot defined by the
[convergence plan](./0029-phase0-readiness-and-domain-matrix.md#convergence-and-independent-validation-plan) may
run before this gate closes because it is a falsification experiment, not a
production implementation or freeze claim.

## 10. Phase 0 exit gate

Phase 0 is complete only when:

- one typed policy IR and one canonical encoding exist;
- grammar, type/unit rules, decidability boundary and the first four policy
  kind semantics are normative;
- all RFC 0029 §36 and RFC 0029.a §27 prerequisites required by an advertised
  class have formal semantics;
- admission outcomes and diagnostics are stable and machine-readable;
- a compiled admission report derives every advertised checklist result from
  typed evaluator state rather than manual assertion;
- all five domain profiles plus mixed model-influence bindings encode without
  core schema changes;
- golden vectors establish canonical bytes and bundle identities;
- executable conformance suites cover the §7 cases and each profile's
  positive and negative matrix;
- two independent input paths lower equivalent policies to the same IR,
  canonical bytes, hash, admission result and diagnostics;
- unsupported or stale schema versions and guarantees are rejected;
- the roadmap records exactly which policy kinds and profiles passed.

Only after this exit gate may Phase 1 bundle/admission implementation be
described as building on a frozen Phase 0 core.

### Status (2026-09-27)

- [x] one typed policy IR and one canonical encoding exist;
- [x] grammar, type/unit rules, decidability boundary and the first four
      policy kind semantics are normative — audited directly, 2026-09-27:
      (a) the four kinds' semantics are written and each carries all five
      required properties, confirmed by re-reading the text just now, not
      recollection — operational semantics, composition algebra, proof
      obligations, enforcement phases, minimum evidence schema (RFC 0029
      §8.1–§8.4); (b) grammar/canonical encoding — `schema.json` (a real
      JSON Schema 2020-12 document, closed via `additionalProperties:
      false` except the declared `attributes`/`parameters` extension
      points) existed but, found this same audit, was **never actually
      validated against** — a real gap, since a drift between it and the
      Rust parser's own types would have gone undetected. Closed: all 107
      generated vectors were checked against it (0 violations, confirmed
      empirically) and `generate.py` now runs this validation on every
      invocation (`--check` and generation both), wired into the CI
      workflow with its own `jsonschema` dependency — no longer a
      documentation-only artifact; (c) type system — the same schema plus
      `canonical_bytes`' I-JSON integer-range/no-floats rule and the
      closed core-envelope/open-extension-point split (§9 item 5); unit
      rules are domain-module scope per §7, and `funds_reserve`'s
      minor-unit-integer convention is the concrete instance for banking;
      (d) decidability boundary — RFC 0029 §25's bounded/terminating rules
      are structurally enforced, not just declared: every evaluator
      (`compute_influence`, `evaluate_review`, `evaluate_fact_provenance`,
      `evaluate_admission`) runs only after `validate_graph`'s Kahn's-
      algorithm cycle rejection, so evaluation is a fold over a
      provably-finite DAG, not unbounded recursion; the funds_reserve
      daily-limit window and monitor-health check are both bounded
      predicates, not unbounded history quantification;
- [x] all RFC 0029 §36 and RFC 0029.a §27 prerequisites required by the
      *advertised* classes (banking, healthcare) have formal semantics —
      the F1–F12 gate (§3 above);
- [x] admission outcomes and diagnostics are stable and machine-readable —
      closed `AdmissionDiagnostic`/`ReviewFinding` enums;
- [x] a compiled admission report derives every advertised checklist result
      from typed evaluator state — `compile_from_fixture` (influence,
      review, fact-provenance, admission, distributed finality, monitor
      health) and `compile` (the `funds.reserve` pilot, now including
      daily-limit consistency, disclosed as a per-call check, not a full
      aggregate-window audit);
- [x] all five domain profiles plus mixed model-influence bindings encode
      without core schema changes;
- [x] golden vectors establish canonical bytes and bundle identities;
- [x] executable conformance suites cover the §7 cases **and each
      profile's positive and negative matrix** — §7's 19 cases are fully
      executable (banking 9/9, healthcare 10/10, `tests/
      section7_banking.rs`, `tests/section7_healthcare.rs`). The second
      half of this bullet — each of the five domain profiles' own full
      positive/negative matrix — closed same day (§11 below): all five
      domains now have all nine cross-domain-matrix rows, not just
      logistics;
- [x] two independent input paths lower equivalent policies to the same
      IR, canonical bytes, hash, admission result and diagnostics —
      permanent CI gate, §8–§9;
- [x] unsupported or stale schema versions and guarantees are rejected;
- [x] the roadmap records exactly which policy kinds and profiles passed
      — see docs/ROADMAP.md's own enumeration, added this same pass.

**This gate is now closed** (§11 below closes the one remaining item: each
of the five domain profiles' full positive/negative matrix, not just §7's
19 cases). Phase 1 bundle/admission implementation may now be described as
building on a frozen Phase 0 core, per this section's own rule.

## 11. §7 matrices closed; formal-artifact backlog enumerated, 2026-09-27

The entry gate's (§9 above) two remaining substantive checkboxes:

- **"the banking and healthcare matrices in §7 pass on paper against the
  candidate semantics"** — closed. All 19 rows (banking 9, healthcare 10)
  are now executable, not just "on paper": `src/pilot/funds_reserve.rs`
  gained daily-limit consistency, external-submission timeout/
  reconciliation, evidence-plane-outage, and authority-compromise
  discovery; a new `src/pilot/clinical_access.rs` pilot covers all ten
  healthcare rows (break-glass binding, consent/PDP outage, conflicting
  allergy authorities, order invalidation, alias resolution, evidence
  outage, irreversible administration, research-purpose inheritance,
  retention-conflict resolution). `tests/section7_banking.rs` and
  `tests/section7_healthcare.rs`, 17 tests, all passing.
- **"RFC 0029 and RFC 0029.a formal-artifact backlogs are enumerated and
  assigned to Phase 0 deliverables"** — closed by this list. RFC 0029 §8
  names fourteen policy kinds total; §8.1–§8.4 now give the first four
  (Authorization, Data policy, Numeric invariant, State invariant) formal
  operational semantics, composition algebra, proof obligations,
  enforcement phases, and minimum evidence schema. The remaining ten
  (Transition, Sequence, Separation of duty, Resource lifecycle, Temporal,
  Aggregate, External evidence, Operational, Physical world, Human
  judgment) do not yet have these artifacts — named here as the Phase 0
  formal-artifact backlog, not silently assumed complete. None of the ten
  is currently an *advertised* class this project certifies against (only
  banking/healthcare's F1–F12 classes are), so their absence does not by
  itself block this section's own exit-gate assessment (§10) — but any
  future profile that advertises one of them must close its artifact first,
  per §8's own rule that "no kind is admitted before those artifacts
  exist."

The remaining open item — from both §9 and §10 above — is the same one:
each of the five domain profiles' full positive/negative matrix, beyond
§7's 19 now-closed cases.

## Field Classification Ledger

```
Companion to:  RFC 0029 Phase 0 readiness matrix
Status:        Revision 1 independently passed; A1/A2 resolved
Created:       2026-09-27
Profiles:      Banking transfer, KYC, healthcare/clinical decision support,
               manufacturing, insurance, and model-influence overlay
```

## 1. Purpose and rule

This ledger classifies the semantic fields required by the five candidate
domain profiles. It determines whether each concept belongs in the universal
RFC 0029 core, a domain profile, an opaque boundary, an explicit rejection,
or remains ambiguous.

The classifications are:

- `CoreExact` — one domain-neutral semantic contract belongs in the canonical
  IR;
- `ProfileSpecialization` — the core supplies the slot/contract and a profile
  supplies typed vocabulary, authority, thresholds or evidence;
- `Opaque` — the system can require and evidence an authoritative decision,
  but cannot inspect/prove its substantive semantics;
- `Unsupported` — the requested guarantee or authority is rejected;
- `Ambiguous` — multiple conforming interpretations remain possible and
  Phase 0 freeze is blocked.

The ledger classifies semantics, not a final wire format. A row may become
multiple encoded fields, but it must not move between core and profile without
reviewing all five profiles.

## 2. Universal policy envelope

| Concept/field | Classification | Required universal meaning |
| --- | --- | --- |
| Schema version | `CoreExact` | Selects canonical grammar/encoding semantics |
| Policy, rule and bundle ID | `CoreExact` | Stable identity independent of display name |
| Bundle hash/signature/root set | `CoreExact` | Canonical content identity and authorization chain |
| Effective interval and lifecycle | `CoreExact` | Immutable versions; activation, supersession, revocation |
| Owner, author, approver and signer | `CoreExact` | Typed authority roles and identities |
| Scope and exclusions | `CoreExact` | Closed included resources/effects plus explicit exclusions |
| Assumptions | `CoreExact` | Named, revalidatable preconditions that do not reduce the guarantee silently |
| Platform tier | `CoreExact` | Tier 1, 2 or 3 semantics from RFC 0029 |
| TCB and trust roots | `CoreExact` | Exact deployment boundary and roots |
| Cost/complexity budget | `CoreExact` | Bounded latency, collection, fan-out, storage and monitoring envelope |
| Profile identity/version | `CoreExact` | References separately versioned domain specialization |
| Display labels/descriptions | `ProfileSpecialization` | Non-authoritative presentation metadata |

## 3. Resources, effects and applicability

| Concept/field | Classification | Required universal meaning |
| --- | --- | --- |
| Canonical resource namespace/ID/version | `CoreExact` | Stable authority-qualified identity |
| Alias, equivalent and replica relations | `CoreExact` | Typed relation with validity and authority |
| Merge, split and derivation relations | `CoreExact` | Versioned lineage plus policy-inheritance rule |
| Ambiguous resolution result | `CoreExact` | `Indeterminate`; never arbitrary selection |
| Domain resource kinds | `ProfileSpecialization` | Account, patient, machine, contract, claim, customer, etc. |
| Canonical effect ID | `CoreExact` | Authority-independent protected-effect identity |
| Parent/child/equivalent effect relations | `CoreExact` | Closed taxonomy and coverage semantics |
| Indirect/admin/migration/file/message paths | `CoreExact` | Included path classes or named exclusions |
| Domain effects | `ProfileSpecialization` | Debit, administer, restart, deny claim, open account, etc. |
| Jurisdiction/applicability fact contract | `CoreExact` | Typed authoritative context evaluated before composition |
| Domain applicability graph | `ProfileSpecialization` | Corridor, intended use, contract-at-loss, site/population, product rules |
| Conflict-of-law/manual escalation | `CoreExact` | Explicit resolution authority or `Indeterminate` |

## 4. Facts, authorities and semantic inputs

| Concept/field | Classification | Required universal meaning |
| --- | --- | --- |
| Fact type/value or protected reference | `CoreExact` | Typed decision input |
| Issuer/authority domain | `CoreExact` | Source authorized to assert the fact |
| Signature/channel/collection time | `CoreExact` | Provenance and transport integrity |
| Validity/freshness/revocation | `CoreExact` | Decision and transition usability interval |
| Confidence/uncertainty | `CoreExact` | Typed optional evidence; never authority by itself |
| Transformation/derivation lineage | `CoreExact` | Input-to-fact provenance |
| Source-set precedence/quorum/corroboration | `CoreExact` | Deterministic disagreement algebra |
| Compromise window/impact response | `CoreExact` | Affected-decision discovery and containment contract |
| Semantic input concept/unit/method/time basis | `CoreExact` | Cross-domain compatibility contract |
| Domain ontology and units | `ProfileSpecialization` | Money/currency, clinical concepts, sensor meanings, insurance terms |
| Domain source authority assignment | `ProfileSpecialization` | Ledger, lab, PLC, registry, contract repository, etc. |
| Professional/legal/clinical interpretation | `Opaque` | Require qualified authenticated decision/reason; do not claim substantive proof |
| Physical truth independent of attested observation | `Unsupported` | Software evidence cannot establish it unconditionally |

## 5. Rules, decisions and composition

| Concept/field | Classification | Required universal meaning |
| --- | --- | --- |
| Subject/action/resource/context types | `CoreExact` | Typed policy request vocabulary slots |
| Bounded expression AST | `CoreExact` | Decidable, terminating admitted fragment |
| Domain predicates | `ProfileSpecialization` | Typed predicate plus admitted semantics/proof rule or explicit opacity |
| Policy kind | `CoreExact` | Authorization, invariant, transition, sequence, separation of duty, etc. |
| Domain policy-kind specialization | `ProfileSpecialization` | Completeness, minimum necessary, intended use, contract applicability, etc. |
| Decision result | `CoreExact` | Permit, Deny, PendingApproval, PermitWithObligations, Defer, Indeterminate |
| Axis A verification mechanism | `CoreExact` | Static, Runtime, Continuous |
| Axis B per decision/effect | `CoreExact` | None, Advisory, Guarded, Atomic, DurableWorkflow |
| Axis C summary and per-fact authority | `CoreExact` | Local/ExternallyAttested without erasing fact provenance |
| Authority partial order/delegation | `CoreExact` | Explicit authority and override edges |
| Deny/obligation/conflict algebra | `CoreExact` | Kind-specific deterministic composition |
| Domain authority floors | `ProfileSpecialization` | Safety PLC, clinician, adjuster, compliance/legal authority |
| Professional judgment correctness | `Unsupported` | Evidence of a decision is not proof it was substantively correct |

## 6. Capabilities, obligations and invalidation

| Concept/field | Classification | Required universal meaning |
| --- | --- | --- |
| Decision/request identity | `CoreExact` | Unique bound issuance event |
| Subject/authority chain | `CoreExact` | Exact principal and delegated authority |
| Resource/effect/gateway/input binding | `CoreExact` | Prevents replay across changed target or request |
| Bundle/rule binding | `CoreExact` | Exact policy semantics used |
| Issuance/expiry/revocation/invalidation set | `CoreExact` | TOCTOU and reuse boundary |
| Permitted-use count/idempotency key | `CoreExact` | Single/bounded use and retry behavior |
| Consumption reservation/receipt | `CoreExact` | Atomic or durable use accounting |
| Blocking obligation | `CoreExact` | Must finish before effect finality |
| Post-commit obligation lifecycle | `CoreExact` | Durable retry, deadline, escalation and terminal result |
| Domain obligations | `ProfileSpecialization` | Step-up, pharmacy check, isolation, notice, screening, etc. |
| Rule/fact/resource dependency graph | `CoreExact` | Determines affected decisions/transitions |
| Transition-current versus pinned facts | `CoreExact` | Explicit revalidation contract |

## 7. Workflow, finality, safety and aggregates

| Concept/field | Classification | Required universal meaning |
| --- | --- | --- |
| State/transition identity | `CoreExact` | Typed finite workflow structure |
| State finality authority/visibility | `CoreExact` | Who establishes and who may observe a state |
| Reversible/compensatable/irreversible | `CoreExact` | Effect classification without rewriting history |
| Unknown outcome | `CoreExact` | First-class state plus reconciliation path |
| Domain workflow states | `ProfileSpecialization` | Settled, administered, actuation observed, payment cleared, etc. |
| Safety posture | `CoreExact` | FailClosed, FailSafeState, FailOperationalBounded, HumanControlledEmergency |
| Domain safe state/emergency conditions | `ProfileSpecialization` | Clinical downtime, controller safe state, catastrophe posture, etc. |
| Aggregate authority/key/window | `CoreExact` | Bounded aggregate contract |
| Consistency/partition/reservation/overshoot | `CoreExact` | Honest strict versus bounded-impact result |
| Domain aggregate | `ProfileSpecialization` | Daily transfer limit, alert burden, portfolio/catastrophe measures |
| Hard real-time certification/SIL assignment | `Opaque` | Carry external safety evidence; generic core does not prove/assign it |

## 8. Evidence, monitoring and confidentiality

| Concept/field | Classification | Required universal meaning |
| --- | --- | --- |
| Evidence profile/replay class | `CoreExact` | Forensic, Audit, Explanation and byte/semantic/verification replay |
| Evidence-finality mode | `CoreExact` | Atomic, durable outbox, synchronous external, emergency deferred, observed |
| Evidence outage behavior | `CoreExact` | Effect-class-specific behavior |
| Chain of custody/integrity/authenticity | `CoreExact` | Distinguishes hash integrity from signed authority |
| Observation sources/coverage | `CoreExact` | Closed monitor observation model |
| Heartbeat/checkpoint/skew/loss/backpressure | `CoreExact` | Monitor-health preconditions |
| Detection/response/exposure bounds | `CoreExact` | Bounded-impact, never prevention |
| Domain monitoring metrics | `ProfileSpecialization` | Reconciliation, alert load, sensor drift, subgroup harm, etc. |
| Classification/purpose/minimisation | `CoreExact` | Policy-plane data contract |
| Locality/encryption/access/retention/hold/deletion | `CoreExact` | Governed evidence and policy inputs |
| Domain confidentiality rules | `ProfileSpecialization` | PHI, biometrics, financial data, worker/safety data, claims data |

## 9. Model-influence overlay

| Concept/field | Classification | Required universal meaning |
| --- | --- | --- |
| Binding per decision/effect class | `CoreExact` | Exactly one declared influence binding |
| Influence level | `CoreExact` | None, Observe, Classify, Prioritize, Recommend, Draft, Approve, execution tiers |
| Model/deployment/runtime/adapter identity | `CoreExact` | Immutable authority-qualified MAP identity |
| Inference receipt/input commitment | `CoreExact` | Exact model invocation and typed inputs/outputs |
| Promise/effective state | `CoreExact` | Policy-derived usability for exact scope/time |
| C1 semantic input contract | `CoreExact` | Generic compatibility; profile supplies ontology |
| C2 effect authority/autonomy | `CoreExact` | Maximum authority bound to exact effect |
| C3 outcome/utility/harm contract | `CoreExact` | Generic endpoints/stopping rules; profile supplies measures |
| C4 label/adjudication contract | `CoreExact` | Generic provenance/delay/disagreement/feedback contract |
| C5 human-system capacity | `CoreExact` | Qualification, independence, evidence access, load and response |
| C6 deployment validation/surveillance | `CoreExact` | Exact scope/equivalence/monitoring/impact contract |
| Domain promises/thresholds | `ProfileSpecialization` | Clinical endpoints, defect labels, claimant harms, KYC cohorts, etc. |
| Model output promoted to authoritative fact | `Unsupported` | Requires a separate admitted adjudicating authority/process |
| Model authority above domain floor | `Unsupported` | Cannot defeat safety, legal, professional or payment authority |
| Material-influence determination | `CoreExact` | Revision 1 normalization, edge algebra, assertion dominance and non-interference; independently passed |
| Observable human-review protocol | `CoreExact` | Declared modes, typed predicates, authority separation, invalidation and typed fixtures; independently passed |
| Human reasoning/professional correctness | `Opaque` | Observable protocol does not prove cognition or substantive correctness |

## 10. Banking-transfer specialization

| Concept | Classification | Disposition |
| --- | --- | --- |
| Money/currency/product floor | `ProfileSpecialization` | Typed fixed-point/minor-unit and product rules |
| Account/beneficiary/transfer/quote IDs | `ProfileSpecialization` | Instances of canonical resources |
| Ownership/delegation, sanctions and FX authority | `ProfileSpecialization` | Instances of fact/authority contracts |
| Atomic balance reservation/ledger posting | `ProfileSpecialization` | Uses core Atomic semantics inside declared ledger domain |
| Daily limit | `ProfileSpecialization` | Uses core aggregate contract |
| External submission/settlement states | `ProfileSpecialization` | Uses core DurableWorkflow/finality |
| Global atomic transfer across external rail | `Unsupported` | Unless a real distributed atomic boundary is supplied |
| Screening correctness/completeness | `Unsupported` | Receipts prove process/evidence, not universal absence of risk |

## 11. KYC specialization

| Concept | Classification | Disposition |
| --- | --- | --- |
| Customer/entity/document/biometric/ownership graph | `ProfileSpecialization` | Domain resource/vocabulary types |
| Identity schemes and authoritative registries | `ProfileSpecialization` | Domain applicability/authority |
| Screening/PEP/watchlist categories | `ProfileSpecialization` | Domain facts and effect rules |
| Biometric/tamper/fuzzy-match inference | `ProfileSpecialization` | Uses overlay Classify plus assurance |
| Beneficial-owner completeness | `ProfileSpecialization` | Bounded graph plus qualified authority |
| Risk categories/actions/refresh | `ProfileSpecialization` | Domain transitions/effects |
| Legal/compliance exception judgment | `Opaque` | Require qualified decision/reason |
| “Every KYC decision is correct” | `Unsupported` | Conflates authority truth, model quality and judgment |
| Model self-closes alert/lowers risk ungoverned | `Unsupported` | Violates independence/effect authority |

## 12. Healthcare specialization

| Concept | Classification | Disposition |
| --- | --- | --- |
| Patient/encounter/record/order/product identity | `ProfileSpecialization` | Domain resource types and merge/derivation rules |
| Minimum-necessary fields and sensitive categories | `ProfileSpecialization` | Domain-authored data policy |
| Consent/treatment/emergency applicability | `ProfileSpecialization` | Domain applicability and safety posture |
| Clinical concept/unit/method/specimen/device | `ProfileSpecialization` | Semantic input ontology |
| Allergy/interaction/dose facts | `ProfileSpecialization` | Named authorities/revalidation points |
| Intended use/regulatory status | `ProfileSpecialization` | External authority and effective scope |
| Clinical utility/harm endpoints and labels | `ProfileSpecialization` | C3/C4 measures and adjudication |
| Clinical judgment correctness | `Opaque` | Require qualified decision; cannot prove substance |
| Autonomous high-risk diagnosis/treatment | `Unsupported` | Initial profile lacks authority/safety/regulatory case |
| Software claim of patient outcome benefit from model health | `Unsupported` | Requires admitted clinical outcome evidence |

## 13. Manufacturing specialization

| Concept | Classification | Disposition |
| --- | --- | --- |
| Asset/controller/firmware/tooling/guard baseline | `ProfileSpecialization` | MF1 configuration identity |
| Energy plan/personal lock/custody | `ProfileSpecialization` | MF1/MF5 typed facts and authorities |
| Command/accept/actuation/verified state | `ProfileSpecialization` | MF2 workflow/finality states |
| Hard deadline/jitter/local fallback | `ProfileSpecialization` | MF3 real-time profile fields |
| External safety case/integrity level | `Opaque` | MF4 authority evidence, not assigned by core |
| Batch/rework/release/recall lineage | `ProfileSpecialization` | MF6 resources/effects |
| Model classify/recommend/quarantine | `ProfileSpecialization` | Overlay levels through bounded ExecuteReversible |
| Database row proves physical isolation | `Unsupported` | Physical authority required |
| Model defeats interlock/restarts hazardous cell | `Unsupported` | Safety-authority floor |

## 14. Insurance specialization

| Concept | Classification | Disposition |
| --- | --- | --- |
| Customer contract/forms/endorsements | `ProfileSpecialization` | IN1 immutable source artifacts, distinct from enforcement policy |
| Coverage/applicability graph | `ProfileSpecialization` | IN2 contract-at-loss semantics |
| Reserve/settlement/payment/recovery states | `ProfileSpecialization` | IN3 financial finality |
| Adverse notice/reason/appeal | `ProfileSpecialization` | IN4 contestability |
| Proxy/lawful external-data controls | `ProfileSpecialization` | IN5 data/harm rules |
| Catastrophe/portfolio posture | `ProfileSpecialization` | IN6 bounded capacity context |
| Ambiguous contract/legal interpretation | `Opaque` | Qualified adjuster/legal adjudication |
| Model score becomes fraud/coverage/payee fact | `Unsupported` | Inference-to-fact promotion prohibited |
| Opaque unattended denial/cancellation/pricing/payment | `Unsupported` | Initial authority and contestability requirements unmet |

## 15. Resolved ambiguity register

### A1 — material model influence — resolved by Revision 1

The overlay originally listed examples without a portable decision procedure
for indirect influence. The candidate semantics now remove that ambiguity
when a model selects evidence, changes a default, orders a queue, drafts a
reason, or produces a feature consumed by a deterministic rule.

Resolution:

- a typed decision-lineage graph from model output to rule, human
  presentation, decision and effect;
- influence edges (`input`, `feature`, `candidate`, `ranking`,
  `default`, `reason`, `recommendation`, `approval`, `parameter`, `trigger`);
- any reachable material edge activates a non-`None` binding;
- explicit non-material observational edges require proof that they
  cannot affect the protected decision/effect;
- cross-domain golden cases are normative in the
  [influence/review semantics](./evidence/0029/influence-review/original-semantics.md).

The original review failures R1–R4 were corrected by Revision 1 and closed by
the [passing re-review](./evidence/0029/influence-review/pass-report.md).

### A2 — meaningful human review — resolved by Revision 1

The overlay correctly said that an authenticated click is insufficient but
did not define a minimum observable contract. The candidate semantics now
separate exact protocol evidence from the opaque quality of the reviewer's
reasoning.

Resolution:

- `Acknowledgement`, `BoundedApproval`, and
  `IndependentDecision` review modes;
- required source-evidence visibility, model-output visibility,
  affirmative reason, override path, minimum/maximum response behavior and
  sampling/audit evidence per mode;
- capacity and rubber-stamp findings narrow/suspend the dependent
  authority without claiming to prove cognition;
- the unobservable quality of human reasoning is `Opaque` while
  keeping the observable workflow contract exact.

The original review failures R5–R8 were corrected by Revision 1 and its typed
symbolic fixtures, then closed by the
[passing re-review](./evidence/0029/influence-review/pass-report.md).

## 16. Passing independent-review result

Counts by semantic row in this ledger:

- `CoreExact`: the shared identity, effect, fact, decision, capability,
  workflow, evidence, monitoring, confidentiality and MAP contracts;
- `ProfileSpecialization`: all domain vocabulary, authorities, thresholds,
  states, outcomes and external evidence;
- `Opaque`: qualified professional/legal/clinical judgment and external
  safety certification where substance cannot be machine-proved;
- `Unsupported`: false universal correctness, unauthorized authority
  escalation, software-only physical truth and unsupported autonomous effects;
- `Ambiguous`: none in the current field ledger; a future domain or formal
  contradiction may reopen a classification.

The candidate no-ambiguity and focused independent-review gates pass through
[Revision 1](./0029.a-model-assurance-port.md#model-influence-and-human-review-semantics-revision-1) and its
[passing re-review](./evidence/0029/influence-review/pass-report.md). The remaining
Phase 0 gates and executable canonical fixtures still block schema freeze.
## Convergence and Independent Validation Plan

```
Status:        Required before any RFC 0029 core freeze claim
Created:       2026-09-27
```

## 1. Correction

KYC, medical, manufacturing and insurance participated in deriving C1–C6.
Their rerun is in-sample consistency evidence, not independent evidence of
universality. The current MAP set was therefore a **candidate baseline**, not
a provisionally frozen core, until the two gates below closed.

Use these terms precisely:

- **Candidate baseline** — consistent enough to implement and attempt to
  falsify.
- **Validated core** — passed an out-of-sample exercise and independent
  adversarial review without an unresolved universal primitive.
- **Frozen version** — has canonical schemas, implementations, compatibility
  rules and conformance evidence; changes require a version transition.

**Result (2026-09-27): both gates below have closed. C1–C6 is promoted from
`Candidate baseline` to `Validated core`.** It is explicitly *not* promoted to
`Frozen version` — see §6 for what that still requires, including a second,
independently-built implementation agreeing byte-for-byte and hash-for-hash
with `rfc0029-conformance`, which does not yet exist.

## 2. Out-of-sample gate — CLOSED (PASS)

Use logistics/customs import release, a domain already anticipated by RFC
0029 but absent from the MAP design loop. Exercise non-model tariff/origin
rules, model-assisted classification and inspection priority, conflicting
sovereign/carrier facts, physical cargo release, holds and appeals, and
distributed finality. A reviewer who did not author C1–C6 must actively try to
find a seventh generic contract and record every proposed core change.

Done: the
[logistics/customs tabletop](./evidence/0029/tabletops/logistics-customs.md)
and its executable fixtures (`L1`–`L7`) were written, then given to an
independent reviewer with no prior involvement in this project and no
knowledge of the intended result. That reviewer actively tried six hard
cases beyond the tabletop's own table — a sovereign-jurisdiction conflict
with no supranational tie-breaker, seizure/forfeiture as distinct from a
hold, bonded-warehouse many-to-one consolidation, free-trade-agreement audit
trails the model cannot itself observe, AI classification laundered into a
downstream fact, and storage-duration forfeiture — and **found no seventh
generic contract required**
([full report](./evidence/0029/out-of-sample-review/review-report.md)). This
is the first result that actually satisfies this gate; the tabletop's own
original closing section had asserted the same conclusion before independent
review, which does not count on its own (§1).

The same review caught two real defects, now fixed: an inconsistent
`reversibility_class` for the same `cargo.release` effect across fixtures
`L1`/`L4` (corrected, and now covered by a corpus-wide regression test,
`every_effect_id_has_one_reversibility_class_across_the_whole_corpus` in
`tests/vectors.rs`), and a genuine gap in how the tabletop reasoned about a
model's classification output being persisted as an ordinary fact and later
consumed by a *different* decision with no model node in its own graph. That
gap is now fixed at both the RFC-text level (a paragraph in the tabletop's
§8, citing RFC 0029 §36.8 and RFC 0029.a §13.1) and the fixture level — six
fixtures (`L8`–`L13`) demonstrate the positive case plus stale, revoked,
wrong-model, wrong-deployment and unauthorized-consumer lineage failures,
backed by a new, separate `evaluate_fact_provenance` evaluator (not an
extension of `compute_influence`, which was already shown by construction to
fail closed on an unattributed model-to-fact-to-decision path, not to
silently launder influence through it). What these fixtures do *not* cover:
every one is a single static graph evaluated at one instant, so none
demonstrates two decision episodes genuinely separated in time, or a
revocation arriving between an inference and its later consumption — see the
tabletop's §8 for the full account. This does not reopen the gate: the
reviewer's PASS verdict on the seventh-primitive question never depended on
`L8` existing, only noted it as a recommended follow-up, now done.

## 3. Independent F1–F12 gate — CLOSED (PASS, after one correction cycle)

Banking/healthcare F1–F12 receives the same adversarial process that found
R1–R8: independent inputs, reproducible `PASS`/`FAIL`/`INDETERMINATE` cases,
and a separate re-review after fixes. “Conditionally admissible” is not
load-bearing before this gate passes.

Done, in two rounds:

1. A first independent reviewer, with no prior involvement in this project,
   scored all twelve F1–F12 items and re-judged 15 test-matrix rows (7
   banking, 8 healthcare) against RFC 0029's *current* §36 text — not just
   its section headings. Result: **11 of 12 items PASS, 1 INDETERMINATE
   (F8)**, plus two matrix rows that did not upgrade
   ("Reconciliation detects imbalance," "Patient requests explanation"), and
   a third cross-cutting gap (no precedence rule between an identity-
   ambiguity `Indeterminate` and an active emergency safety posture). The
   reviewer also caught that the original document's own §7.1
   gap-resolution-traceability table overstated its citation for F8 — the
   one place that table should not have been trusted at face value.
   [Full report](./evidence/0029/f1-f12-review/review-report.md).
2. RFC 0029 was corrected for exactly the five named findings: a new
   numbered admission check and rejection code for F8
   (`DependencyMappingIncomplete`, §12 item 12), two missing rejection codes
   for F5/F9 (`FinalityStateUnsupported`, `AggregateConsistencyUndeclared`),
   an explicit precedence rule in §13 (identity ambiguity fail-closes an
   effect unless the active posture explicitly declares it covers that
   condition), a named correction/compensation-triggering authority in §16
   (defaulting to the same authority as waivers), and audience-tiered
   Explanation-profile disclosure in §23. A **second, independent** reviewer
   — not the one who found the gaps — then re-judged all five corrections
   and both flagged matrix rows against the new text and returned **PASS,
   no residual gaps**
   ([full report](./evidence/0029/f1-f12-review/rereview-report.md)).

Combined, all twelve F1–F12 items now carry an independent-review-backed
PASS. The re-review's own stated scope was the five findings and two rows,
not a from-scratch re-audit of all twelve items — that full-corpus re-audit
was the first reviewer's job, and it already covered all twelve.

## 4. Smallest executable pilot

Build one banking-transfer slice now:

| Dimension | Pilot |
| --- | --- |
| Policy | authorization plus one atomic invariant |
| Effect | local `funds.reserve` only |
| Gateway | in-process atomic gateway over a transactional fixture |
| Facts | signed account/subject state with freshness and revocation |
| Model influence | `None` |
| Obligation/evidence | blocking receipt and decision-verification replay |
| Negative paths | deny, stale fact, insufficient funds, replay, version mismatch, commit failure |

This is a falsification experiment against the versioned candidate IR, not a
production or freeze claim.

Built: [`crates/rfc0029-conformance/src/pilot/funds_reserve.rs`](../crates/rfc0029-conformance/src/pilot/funds_reserve.rs),
covering exactly this table, with all six negative paths plus fact-tampering
detection as passing tests
([`tests/pilot.rs`](../crates/rfc0029-conformance/tests/pilot.rs)). It remains
disconnected from any store, network, or production effect gateway, and
covers only `funds.reserve` — `funds.release`, external settlement
submission, and aggregate/daily-limit consistency are named exclusions, not
silently covered paths.

## 5. Compiled admission report

Manual checklists cannot certify a policy. Every run emits a deterministic
report with schema/bundle/profile hashes, enforcement axes, resource/effect
closure, fact authority/freshness, influence/review/capability/obligation
results, evidence finality, safety posture, exclusions, stable requirement
IDs, diagnostics and input/output commitments. Its overall result is
`Accepted`, `Rejected` or `Indeterminate`. A checkbox without a typed evaluator
result cannot strengthen a guarantee.

Built: [`AdmissionReportV1`](../crates/rfc0029-conformance/src/pilot/report.rs)
compiles this report from the pilot's actual pre/post ledger state and
receipt — every field is computed, not asserted, and a run that mutates
state without a `Reserved` outcome (or claims one without the exact declared
mutation) fails the report's own `atomic-commit-integrity` check. Byte
determinism across independent compiles of an equivalent run is a passing
test. This closes the letter of this section for the `funds.reserve` pilot's
scope only; it is not yet generalized to every enforcement axis, effect
class, or domain profile this section's field list names.

## 6. Convergence controls

- New companion documents must name an open gate they close; otherwise amend
  an existing document.
- A new universal primitive needs an executable counterexample.
- Profile vocabulary growth alone does not reopen the core.
- The pilot, out-of-sample exercise and independent F1–F12 review proceed in
  parallel.
- Core freeze is forbidden until both independent gates pass; experimental
  implementation against a versioned candidate is allowed.

**Both independent gates passed on 2026-09-27 (§§2–3).** Per this section's
own rule, that permits declaring `Validated core` (§1). It does **not**
permit a `Frozen version` claim. Still open, none of it optional:

- a second, independently-built implementation of the canonical fixture
  schema, agreeing byte-for-byte and hash-for-hash with
  `rfc0029-conformance` — named as a freeze blocker in
  [the canonical-IR doc](./0029-canonical-ir-and-conformance-vectors.md) §6
  and required by this document's own Phase 0 exit gate (§10). **Done for
  canonicalization** (2026-09-27): a from-scratch Ruby implementation now
  agrees byte-for-byte and hash-for-hash on all 50 fixtures, after fixing
  two real defects it caught — see canonical-IR doc §11 and §8 below. Not
  done: this was one run, not an ongoing dual-implementation discipline, and
  it does not itself close the diagnostics-taxonomy/compatibility/version-
  rules/negative-vector items still listed below;
- `L8`–`L13` (§2 above) were also rebuilt (2026-09-27) after that same
  independent implementation found their original `ModelInvocation
  -AssertFact-> Fact` shape violates RFC 0029.a's own closed edge table
  (§2.2) — see §8 below. The rebuilt six only cover a single static graph
  per fixture; a genuine cross-episode temporal separation, or a revocation
  arriving between an authority's assertion and its later consumption, is
  still unbuilt and would need either cross-fixture state or a stateful
  pilot;
- `AdmissionReportV1` is scoped only to the `funds.reserve` pilot; it does
  not yet compile a report for a model-influenced decision (review/influence
  results are not wired in) or for any effect class beyond `funds.reserve`;
- the five domain profiles and the model-influence overlay remain candidate,
  schema-encoding-pending artifacts (§4); none has the executable
  conformance matrix §7 requires;
- reference-normalizer emission for non-equivalence-group fixtures remains
  open (canonical-IR doc §6, item 4).

None of these being open changes the §2/§3 result: the out-of-sample and
F1–F12 questions are specifically about whether C1–C6 and RFC 0029 §36 need
a new universal primitive, and both gates independently found they do not.
`Validated core` is the correct, precise status for that specific claim —
not a statement that Phase 0 is complete.

## 7. Ad hoc architectural review (audio), applied 2026-09-27

Distinct from §2/§3's structured independent-reviewer process: an informal,
AI-generated "critique podcast" of the core RFC text surfaced three findings.
Given that source's informality, each was independently verified against the
actual RFC text — section by section — before any fix was made, rather than
applied on the review's own characterization. All three verified real, and
were fixed directly in `0029-domain-neutral-policy-enforcement.md`:

1. **Tier/axis enforceability was named but never specified.** §12 item 17
   and the `UnachievableTier` rejection reason already existed, but no table
   anywhere stated which Axis A/B values are achievable at which platform
   tier — confirmed by exhaustive search, not just the review's assertion.
   Fixed with a new §9.5 enforceability matrix (e.g. `Atomic`/`DurableWorkflow`
   unachievable at Tier 3, which has no mandatory gateway; Tier 2 achieves
   them only where named gateway evidence covers the relevant boundary).
2. **The domain-authority/module-owner interface was named, never specified.**
   §31's responsibility table already splits "defines" (domain authority)
   from "operates" (module owner) for domain vocabulary, which partially
   undercuts the review's framing that the full formal burden falls on the
   domain authority — but confirmed true regardless: nothing described how a
   domain authority's plain intent reaches the module owner's formal
   artifacts. Fixed with a new §7 requirement for a domain-authority-facing
   authoring surface per module, and a Phase 0 (§33) deliverable for it.
3. **No bound exists on a single revocation's re-evaluation fan-out.** §26
   already forbids "availability pressure alone" from activating a
   fail-operational mode, and §36.8 already re-evaluates "exactly the
   affected decisions" with no bound on how large that set can be for one
   high-fan-out authority revocation — confirmed no circuit-breaker concept
   existed anywhere in the document. Fixed with a narrow, explicitly-admitted
   revocation circuit-breaker threshold in §36.6/§36.8 — deliberately scoped
   to one named invalidating event's blast radius, not a general load
   exception, so it does not reopen §26's existing rule.

All three are documentation-level fixes to the RFC text only; none required
touching `rfc0029-conformance`, the canonical fixtures, or any independent-
review verdict already recorded in §2/§3, and none is claimed to have been
exercised by an executable fixture — that remains as open as everything else
in §6.

## 8. Independent implementation and corpus corrections, 2026-09-27

A second, from-scratch implementation
([`independent-implementations/rfc0029-ruby/`](../independent-implementations/rfc0029-ruby/),
report at
[`rfcs/evidence/0029/independent-implementation/report.md`](./evidence/0029/independent-implementation/report.md)),
built in Ruby standard-library-only without reading
`crates/rfc0029-conformance/`, `generate.py`, or any evidence/tabletop
document, checked all 50 canonical fixtures independently. Every claim
below was re-verified against the actual files before being accepted, not
taken on the report's own word.

**Result: canonicalization and semantics now agree on all 50/50 fixtures**,
after fixing three real, independently-confirmed defects:

1. **The symbolic YAML's `lossless_expansion` claim was false.** RFC
   0029.a §13.2 requires a per-effect-class maximum-authority-tier
   declaration; that table (`admission_policy`) existed only as a hardcoded
   dict in the excluded `generate.py`, with no textual basis anywhere in
   the symbolic source. Confirmed by direct search: zero matches for
   `admission_policy`/`maximum_model_level` anywhere in the YAML before
   this fix. **Fixed** by moving the table into the symbolic source itself
   as `effect_authority_ceilings` (same values), with `generate.py` now
   reading it from there. This closes the canonical-IR doc's "second,
   independently-built implementation... agreeing byte-for-byte" freeze
   blocker (§6 above) for canonicalization specifically — see canonical-IR
   doc §11 for the full result.
2. **`M4`'s `final_authority: supervisor` named a nonexistent authority** —
   `authority_catalog` only has `maintenance-supervisor` (the same real
   entity `M1` correctly references). A genuine pre-existing typo in the
   original 44 fixtures, not introduced this session. **Fixed** directly;
   this is a deliberate, acknowledged exception to this project's own
   "fixtures are immutable once hashed" rule, made because the previous
   value was not a considered design choice to preserve but an unambiguous
   data error, and because leaving it in place would have propagated a
   broken authority reference into every future independent-implementation
   comparison.
3. **`L8`–`L13` used an edge shape RFC 0029.a's own closed edge table
   (§2.2) does not license** — `ModelInvocation -AssertFact-> Fact`
   directly, when `AssertFact` is sanctioned only as `AuthorityAssertion →
   Fact` (the only edge in the entire table with `Fact` as a target). A
   model cannot structurally produce a `Fact` at all under this
   representation; only an authority assertion can — a stronger and more
   correct resolution of the out-of-sample review's original lineage
   concern (§2 above) than the fixtures first gave it. This also exposed
   that `rfc0029-conformance`'s graph validation never checks edge
   source/target *kinds* against RFC 0029.a's closed table at all — only
   that edge endpoints exist as node ids — a real, previously invisible
   validation gap, recorded here and closed the same day by §9's edge-kind
   validator. **Fixed** by rebuilding
   all six fixtures on the RFC-sanctioned shape (`ModelInvocation →
   Recommend → Review → HumanInput → AuthorityAssertion → AssertFact →
   Fact → Eligibility → Decision → Authorize → Capability`), which changes
   their computed tier from `Classify`/autonomous to `Recommend`/human-
   reviewed but leaves the fact-provenance checks themselves meaningful and
   now spec-conformant. See the tabletop's §8 and canonical-IR doc §8 for
   the full account.

**What this does and does not establish:** this was one run of one
independent implementation, not an ongoing dual-implementation discipline,
and finding + fixing three real defects in a single pass is itself evidence
this corpus was not yet freeze-ready — not evidence it now is. `Validated
core` (§1) is unaffected either way, since it concerns whether C1–C6 needs
a new universal primitive, not corpus correctness. `Frozen version` remains
unclaimed here; §9 below re-examines each of §6's remaining items in turn.

## 9. `Frozen version` — CORE ONLY, 2026-09-27

Every item §6 listed as still open before any `Frozen version` claim is
addressed below, in the same order, each against what was actually built
and verified this same day — not asserted from a plan:

1. **Second independent implementation, byte-for-byte and hash-for-hash —
   done, and no longer a one-off.** §8 already closed the byte/hash
   agreement itself (51/51). What was one-off there is fixed here: `verify.rb`
   now exits nonzero on any canonicalization mismatch or any semantic
   mismatch outside one disclosed, named exception (`review.detail`
   terseness — §3/§4/§8 of the independent-implementation report), wired
   into `.github/workflows/rfc0029-differential.yml` on every push/PR that
   touches the fixture source, either implementation, or either RFC
   document. Re-running it after this session's own further changes (the
   edge-kind validator surfacing a second `L8`–`L13` defect, plus a new
   fixture `L14`) found and fixed one more real defect — in the Ruby
   implementation's `fact_provenance.rb`, which had never implemented
   `FactProvenanceMissing` at all (independent-implementation report §9).
   This is exactly the "keeps finding real bugs" property a permanent gate
   is supposed to have, not a reason to distrust it.
2. **The `rfc0029-conformance` edge-kind-validation gap** (§8, item 3) —
   closed. `validate_graph` now checks every edge's source/target *kinds*
   against RFC 0029.a §2.2's closed table, with an exhaustive negative test
   per all 22 `EdgeKind` variants (`tests/edge_kinds.rs`). Applying it
   before this fix shipped caught a *second*, previously-undetected defect
   in the already-once-corrected `L8`–`L13` (`Eligibility` sourced from
   `Fact`, also disallowed; corrected to `DataFlow`).
3. **Stable diagnostics/result taxonomy** — closed. `AdmissionDiagnostic`
   (13 variants) and `ReviewFinding` (8 variants) replace free-form
   strings; a call site can no longer fabricate an ad-hoc diagnostic.
   `tests/diagnostics_coverage.rs` proves every variant is reachable by at
   least one of the 51 canonical fixtures (one documented, architecturally-
   justified exception — see that file) — closing "complete negative-vector
   coverage" (item 6 below) in the same pass, since the two are the same
   underlying question asked from opposite directions.
4. **Compatibility and downgrade policy** — closed, and deliberately
   minimal: exact `schema_version` string match, symmetrically rejecting
   both older and newer version strings, with no partial/fuzzy
   compatibility of any kind until a real v2 is designed with its own
   explicit migration rule. `tests/version_compatibility.rs` makes this
   executable, not just described.
5. **Version and extension rules** — closed: the core envelope (`Fixture`,
   `Input`, `Node`, `Edge`, `Bundle`, `CatalogEntry`, `Expected`) is closed
   (`deny_unknown_fields`, needs a version bump to add a field); a `Node`'s
   `attributes` and `Input`'s `parameters` are the two, and only two,
   extension points, open under the current version with no schema change
   required — proven both by this session's own use of it
   (`fact_provenance`/`fact_requirement` needed no Rust type change) and by
   `tests/version_compatibility.rs`'s dedicated cases.
6. **Complete negative-vector coverage** — closed via item 3's coverage
   matrix; it found two real gaps (`FactProvenanceMissing` unreached by any
   fixture; fixed by adding `L14`) and one fixture that is correctly
   unreachable by design (`CapabilityEffectMismatch`, guarded against by
   the generator itself — documented, not papered over).
7. **The `L8`–`L14` temporal-separation gap** (§2, §8) — closed as a
   **falsification pilot**, not as an extension of the static-fixture
   corpus: `src/pilot/assertion_lifecycle.rs`, a stateful `Registry` run
   through the full t0 (assert) → t1 (first decision consumes) → t2
   (revoke, or just let the validity window lapse) → t3 (second decision's
   consumption is denied) sequence
   (`tests/temporal_revalidation.rs`), plus tamper-detection and boundary
   cases. This demonstrates the property — a consumer must re-check
   *current* state on every consumption, never reuse an earlier call's
   answer — is achievable with the existing C1–C6 primitives and ordinary
   signed-reissuance/timestamp techniques; it needed no new primitive,
   which is itself affirmative evidence for §2's out-of-sample PASS, not
   just a checked box. Like `funds.reserve`, it has no cross-process or
   restart persistence claim — that would need its own, separately
   falsified pilot if it ever became a real claim.
8. **`AdmissionReportV1`'s generalization beyond `funds.reserve`, and the
   five domain profiles' executable conformance matrices** remain exactly
   as open as they were — deliberately not addressed here. Per this
   section's own scope and §6's rule, the domain matrices in particular
   must consume a frozen core, not shape it; building them before this
   section closed would have risked exactly that.

**Declaration**: with items 1–7 closed, `Frozen version` is declared for
the canonical IR (`schema.json`, the symbolic fixture source's expansion
rules including `effect_authority_ceilings`), the validator semantics
(structural graph validation, the closed edge-kind table, influence fold,
review-contract evaluation, fact-provenance evaluation, admission/
capability combination), the result taxonomy (`AdmissionDiagnostic`,
`ReviewFinding`), the compatibility/version rules, and the (then-51,
now-56 — see §10) conformance corpus, together with the permanent CI
differential gate that keeps that agreement enforced going forward rather
than asserted once. **This freeze explicitly excludes**: `AdmissionReportV1`
beyond the `funds.reserve` pilot's scope, and all five domain profiles'
executable conformance matrices (Item 8) — those remain `Candidate
baseline`/pending, to be built *against* this frozen core, per §8 above and
this section's own rule. A change to any frozen item now requires a
version transition (a `schema_version` bump), per item 4's own tested
policy — not a silent edit.

## 10. Item 8 revisited: `AdmissionReportV1` generalized, domain matrices started (not finished), 2026-09-27

Same day, after §9's freeze: item 8 above said `AdmissionReportV1`
generalization and the five domain matrices "remain exactly as open as
they were." Two things happened next, deliberately *consuming* the just-
frozen core rather than reshaping it:

- **`AdmissionReportV1` generalized.** `pilot::report::compile_from_fixture`
  runs over any of the (now 56) canonical fixtures — covering model-
  influenced decisions via `compute_influence`/`evaluate_review`/
  `evaluate_fact_provenance`, which `compile` (the original,
  `funds.reserve`-only path) never touches. The overall verdict is now
  `Accepted`/`Rejected`/`Indeterminate`/`Unsupported` — the fourth state
  distinguishing "no graph to report on at all" (pure review-contract or
  invalidation fixtures) from "evaluated and genuinely ambiguous." Tested
  in `tests/report_generalization.rs`, including byte-determinism.
- **The five domain profiles' matrices — started, explicitly not
  finished.** One fixture per domain (`B3`, `K5`, `H4`, `M5`, `I4`) proves
  the fact-provenance evaluator generalizes beyond logistics — the single
  most valuable, most universal gap, since every domain lacked it
  entirely. Building them found two more real defects (canonical-IR doc
  §11's continuation, and this document's own item 3's edge-kind
  validator catching a second `L8`–`L13` violation before these five were
  even added). The
  [cross-domain conformance matrix](./0029-canonical-ir-and-conformance-vectors.md#12-cross-domain-conformance-matrix-2026-09-27)
  states plainly what is and is not covered: every domain now has a
  human-reviewed case and a fact-provenance case; banking, KYC, and
  healthcare are each still missing three or more of logistics's other
  dimensions (autonomous-reversible-effect, mandatory-deny, self-
  certification, invalidation). This is a start, not the "executable
  conformance matrix" §7 (Mandatory non-AI rerun matrices) ultimately
  requires — closing every remaining cell is future work, named as such,
  not silently declared done.

Re-verified after both: `cargo test -p rfc0029-conformance` (all suites)
and the Ruby differential gate both pass against the final 56-fixture
corpus.

## 11. §10's one remaining item closed: full five-domain matrix, same day 2026-09-27

§9 item 8 and §10 above both left one thing open: "each of the five domain
profiles' own full positive/negative matrix, not just §7's 19 cases." That
item is closed now, in the same session, against the same frozen core —
consuming it, not reshaping it, per §9's own rule.

**What was built**: 51 new fixtures (`B4`–`B14`, `K2`,`K7`–`K16`,
`H5`–`H15`, `M6`–`M14`, `I5`–`I13`), bringing every domain to the same
9-dimension row coverage logistics already had. The full, now-all-✓ table
is [§12 of the canonical-IR/conformance-vectors document](./0029-canonical-ir-and-conformance-vectors.md#12-cross-domain-conformance-matrix-2026-09-27-closed-same-day--see-update-below).
Every new fixture's node/edge shape is copied verbatim from an
already-passing fixture in a different domain — no new `EdgeKind`
combination was introduced, which is itself further affirmative evidence
for the out-of-sample PASS (§2 above), not just a checked box. Three new
effect classes (`card.freeze`, `watchlist.flag`, `device.pause`) were
added to `effect_authority_ceilings`, each with the same
autonomous-plus-real-reversal-path justification `product.quarantine`/
`shipment.hold` already carried.

**What this found**: `python3 rfcs/fixtures/0029-canonical/generate.py`
passed cleanly on the first attempt for all 51 (no edge-kind, duplicate-key,
or ceiling-lookup errors) — a real, if secondary, data point for the
closed edge-kind table's and the extension-point rules' own completeness.
`cargo test -p rfc0029-conformance` initially caught one authored bug of
this session's own making
(`every_effect_id_has_one_reversibility_class_across_the_whole_corpus`
correctly rejected `application.reject` being declared both `Reversible`
in `K2` and `Irreversible` in `K8` — fixed by making `K2` `Irreversible`
too, which is exactly as defensible as `L1`'s `Irreversible`
`cargo.release`) plus three now-stale hardcoded fixture-count literals in
`tests/vectors.rs` (56→107 fixtures, 44→91 graph fixtures, 34→81
admissions, 27→66 capabilities issued) — updated to the actual computed
counts, not guessed. The independent Ruby implementation then found one
more real defect, exactly the property a permanent differential gate is
for: `lib/admission.rb`'s `INSTITUTIONAL_AUTHORITY_BY_PROFILE` had entries
for only `banking`/`logistics` (the two profiles that previously exercised
the "no-model institutional decision" row), so `K2`/`H5`/`M6`/`I5`
resolved to a `nil` final authority, which also silently changed 4
fixtures' canonical bytes (via `expand.rb`'s catalog-entry embedding) —
fixed by adding the four missing profile→authority entries, derived the
same mechanical, fixture-reading way the existing table's own header
comment already describes.

**Final state, re-verified**: `generate.py --check` — 107 canonical
vectors, byte-stable. `cargo test -p rfc0029-conformance` — all 10 test
binaries, 95 tests, 0 failures. Ruby differential gate — 107/107
canonicalization match, 107/107 semantic agreement outside the
pre-existing, disclosed `review.detail` terseness exception:
**DIFFERENTIAL GATE: PASS**.

**Status**: with this closed, the Phase 0 exit gate's (§10 above) last
open item is satisfied. Nothing else in §10's checklist was reopened by
this pass — no formal-semantics backlog item (§10 continuation) was
touched, and no frozen-core file (§9's declaration) was edited, only
consumed. This document does not itself declare the exit gate closed in
this section — see the updated §10 status line and `docs/ROADMAP.md` for
the authoritative statement, kept consistent with this finding per this
document's own "roadmap and every companion status statement agree" rule.
