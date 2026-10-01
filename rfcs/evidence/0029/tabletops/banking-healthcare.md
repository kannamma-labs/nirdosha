# RFC 0029 — Banking and Healthcare Tabletop Validation

```
Companion to:  RFC 0029 — Domain-neutral policy admission, enforcement,
               and evidence
Status:        Failure-oriented architecture exercise; no implementation
Created:       2026-09-26
Scenarios:     Cross-border retail-bank transfer
               Emergency hospital record access and medication order
Purpose:       Find architectural failures before implementation
```

> **Amendment status (2026-09-26):** RFC 0029 now incorporates F1–F12 as
> normative requirements in its §36 and threads them through admission,
> gateways, evidence, formal prerequisites, phases, and checklists. This
> records design accommodation only. The scenarios in this document have not
> yet been rerun against formal semantics or an implementation, so the
> original failure verdict remains open until the §8 gates pass.
> The shared representation classifications, concrete rerun cases, and the
> resulting Phase 0 entry gate are tracked in the
> [Phase 0 readiness matrix](../../../0029-phase0-readiness-and-domain-matrix.md).

## 1. Method and verdict

This is a hostile tabletop, not a proof or product demonstration. Each
scenario is evaluated against RFC 0029's admission, decision, enforcement,
coverage, obligation, provenance, evidence, and recovery model. A result is:

- **Pass** — the RFC already defines an honest mechanism and guarantee;
- **Partial** — the architecture has the right category but lacks normative
  semantics needed for interoperable enforcement;
- **Fail** — the architecture would currently accept ambiguity, permit a false
  guarantee, or cannot represent a required control honestly.

Overall verdict: **RFC 0029 is a sound umbrella direction, but it is not ready
to freeze as an implementable architecture.** It passes local authorization,
local atomic invariants, typed provenance, obligation separation, and honest
bypass classification. It fails or remains underspecified at several
real-world boundaries shared by both domains:

1. canonical resource identity and aliasing;
2. applicability completeness and effect-taxonomy closure;
3. multi-authority conflict and jurisdiction selection;
4. decision-token binding, consumption, and replay prevention;
5. evidence atomicity and behavior when evidence storage is unavailable;
6. distributed finality and irreversible effects;
7. safety profiles for fail-closed versus fail-operational behavior;
8. authority compromise, disagreement, and recovery;
9. continuous-monitor blind spots and monitor health;
10. data disclosed to the policy/evidence plane itself.

These are pre-implementation blockers for the affected guarantee classes, not
reasons to abandon the architecture.

## 2. External baseline used for the exercise

The tabletop treats the following as representative sources of real-world
requirements rather than importing their text into the policy language:

- [FATF Recommendations](https://www.fatf-gafi.org/en/publications/Fatfrecommendations/Fatf-recommendations.html):
  customer due diligence, record keeping, suspicious activity controls, and
  wire-transfer information;
- [Basel Committee principles for operational resilience](https://www.bis.org/bcbs/publ/d516.htm);
- [PCI DSS](https://www.pcisecuritystandards.org/document_library/) where
  payment-card data is in scope;
- United States [HIPAA Privacy Rule](https://www.hhs.gov/hipaa/for-professionals/privacy/laws-regulations/index.html)
  and [Security Rule](https://www.hhs.gov/hipaa/for-professionals/security/laws-regulations/index.html)
  for healthcare privacy,
  minimum-necessary access, access control, audit, integrity, and emergency
  operation;
- clinical and patient-safety governance for medication ordering.

The exercise is not a legal-compliance opinion. An actual deployment needs a
jurisdiction and product-specific policy authority. The purpose here is to
test whether Nirdosha can faithfully enforce policy once that authority has
expressed it.

## 3. Scenario A — cross-border retail-bank transfer

### 3.1 System boundary

A customer uses a mobile application to transfer money from a domestic
current account to a newly registered beneficiary at a foreign bank.

Components:

- mobile client and API gateway;
- identity and step-up authentication provider;
- account and available-balance service;
- double-entry ledger;
- transfer orchestration service;
- sanctions and customer-risk providers;
- FX quote provider;
- domestic and correspondent payment rails;
- notification, fraud, case-management, reconciliation, and evidence systems;
- privileged operations, migrations, settlement files, and support tooling.

The deployment is Tier 2. Application services are ordinary Rust/services;
gateways, network policy, database permissions, workload identity, and
monitoring are intended to control effects. Static no-bypass coverage is not
claimed.

### 3.2 Proposed bank policy

1. The authenticated customer owns or is delegated authority over the source
   account.
2. The beneficiary is active and not blocked.
3. Amount is positive and within per-transfer and daily limits.
4. Source available balance remains above the product floor after reservation.
5. Customer, beneficiary, banks, and relevant parties pass required screening.
6. A high-risk or high-value transfer receives step-up authentication and,
   where applicable, human approval.
7. The accepted FX quote is fresh and immutable for the transfer.
8. Reservation, ledger entries, idempotency record, and local transfer state
   change atomically.
9. External submission is idempotent and recoverable.
10. Completion is not reported until the appropriate settlement/finality
    state exists.
11. Every decision and state transition has durable evidence.
12. Direct database, message, file, admin, and provider bypass paths are
    controlled or excluded explicitly.

### 3.3 Happy-path trace

1. The API PEP authenticates the customer and binds subject, device, session,
   source account, beneficiary, amount, currency, purpose, and request ID.
2. The PIP obtains account ownership/version, available balance, cumulative
   limit use, risk state, screening receipts, beneficiary state, and FX quote.
3. The PDP returns `PermitWithObligations`, a short decision validity interval,
   and obligations for step-up authentication and reservation.
4. The account/ledger gateway begins a local transaction, revalidates resource
   versions and limit counters, conditionally reserves funds, writes balanced
   reservation postings, records idempotency and policy evidence, and commits.
5. A durable workflow submits the transfer to the external rail using the
   stable transfer ID.
6. Provider acknowledgement and later settlement messages advance explicit
   states. Notification is a post-commit obligation.

This path is representable by RFC 0029 if local atomicity is limited to the
local store and external work is classified `DurableWorkflow`, not `Atomic`.

### 3.4 Banking test matrix

| Test | Expected enforcement | Result | Finding |
| --- | --- | --- | --- |
| Unauthorized customer | Guarded deny before effect | **Pass** | Typed subject/action/resource and PEP model are adequate if identity is authoritative |
| Insufficient funds under concurrency | Atomic conditional reservation and limit consumption | **Pass** | Requires one conformant transaction boundary and resource versioning |
| Retry after client timeout | Idempotent return of original workflow/receipt | **Partial** | RFC requires idempotency but lacks a canonical decision/effect idempotency binding |
| Same request ID, changed amount | Reject immutable-input mismatch | **Partial** | Needs normative request digest binding |
| Screening provider stale | Deny/defer under freshness policy | **Pass** | Provenance and failure semantics cover this |
| Screening changes after reservation | Re-screen at declared transition or compensate | **Partial** | Invalidation exists, but transition-specific fact revalidation needs a normative dependency map |
| FX quote expires during approval | Re-evaluate before reservation/submission | **Pass** | Fact expiry is an invalidating event |
| Two services claim `Atomic` | Reject claim or use real distributed transaction | **Pass** | RFC explicitly rejects aspirational cross-service atomicity |
| External rail times out after accepting | Enter unknown outcome and reconcile | **Partial** | Durable workflow fits, but distributed finality states are not standardized |
| Ledger commits but evidence store fails | Policy-specific fail/commit behavior | **Fail** | RFC does not define when evidence must share the effect transaction or how an unavailable evidence plane affects finality |
| Direct DBA changes balance | Prevent/attest/detect/exclude | **Pass honestly, not safely** | Tier 2 can only issue bounded-impact/exclusion statements; a bank may declare this insufficient |
| Migration bypasses account gateway | Coverage/bypass classification | **Partial** | Classification exists, but migrations/backfills need first-class effect identities and activation policy |
| Sanctions authority compromised | Revoke authority, invalidate receipts, contain effects | **Fail** | Root/fact authority compromise recovery is not specified beyond general incident response |
| Two screening authorities disagree | Composition under authority policy | **Fail** | Per-kind multi-authority disagreement semantics are absent |
| Daily limit spans shards/regions | Globally consistent aggregate or reduced claim | **Partial** | Aggregate policy exists, but consistency and partition-failure semantics are not formalized |
| Beneficiary/account represented by aliases | One canonical protected resource | **Fail** | No canonical resource-identity/equivalence model prevents policy evasion through aliases |
| Settlement file injected outside API | File/message gateway coverage | **Partial** | Gateways are named, but effect-taxonomy completeness has no normative closure rule |
| Policy changes mid-transfer | Pinned workflow plus critical invalidation | **Partial** | Pinning exists; which rules may force transition revalidation needs policy-dependency semantics |
| Notification fails | Durable post-commit retry | **Pass** | Correctly does not roll back the transfer |
| Reconciliation detects imbalance | Incident, quarantine, compensation | **Partial** | Retrospective controls exist, but correction authority and irreversible-effect semantics need definition |

### 3.5 Banking verdict

The architecture can honestly model a transfer as:

```text
Guarded request authorization
  → Atomic local reservation and ledger mutation
  → Durable external settlement workflow
  → Continuous reconciliation and bounded-impact response
```

It must not describe the entire transfer as one atomic effect. Before a bank
could adopt the architecture, RFC 0029 needs normative resource identity,
decision/effect idempotency, evidence finality, external-finality states,
authority-disagreement handling, and globally consistent aggregate semantics.

## 4. Scenario B — emergency healthcare access and medication order

### 4.1 System boundary

An unconscious patient arrives in an emergency department. A clinician who is
not on the patient's normal care team needs access to the record and submits a
medication order. Allergy information is partly local and partly obtained from
an external health-information exchange. Pharmacy verification, administration,
clinical decision support, audit, and later break-glass review are separate
services.

Components:

- hospital identity, workforce, role, and location services;
- EHR record and document stores;
- consent and patient-restriction registry;
- external health-information exchange;
- medication terminology and allergy services;
- ordering, pharmacy, dispensing, and administration workflows;
- clinical decision support;
- emergency break-glass process;
- audit, disclosure accounting, notification, and evidence systems;
- research, analytics, exports, and support tools.

The deployment is Tier 2. Patient safety may require explicitly authorized
fail-operational behavior during identity, consent, PDP, network, or external
exchange outages.

### 4.2 Proposed hospital policy

1. Normal access requires treatment relationship, purpose, role, and minimum
   necessary field scope.
2. Emergency break-glass access is allowed only for a declared emergency
   scenario, with narrow scope, short duration, reason, enhanced audit,
   notification, and post-hoc review.
3. Sensitive categories may have additional restrictions and specially
   authorized emergency behavior.
4. Medication ordering requires an authenticated licensed prescriber acting
   within scope and location/context.
5. Allergy, interaction, dose, patient, medication, and order facts are
   versioned and fresh at the relevant transition.
6. Pharmacy verification and medication administration are distinct actors
   and workflow transitions where policy requires them.
7. Clinical judgment remains human responsibility; Nirdosha verifies the
   required authenticated decision and evidence, not medical correctness.
8. Access and order evidence is durable and protected from ordinary users.
9. Availability failures follow an explicit clinical-safety profile rather
   than an unconditional fail-closed rule.

### 4.3 Emergency happy-path trace

1. The clinician authenticates through the workforce identity authority.
2. Normal access is denied because no current treatment relationship exists.
3. The clinician activates a pre-admitted emergency scenario, supplies a
   reason, and receives a short-lived, patient-specific, purpose-specific,
   field-scoped capability.
4. The record PEP returns only the emergency minimum-necessary view and writes
   access evidence.
5. The ordering workflow gathers patient weight, allergies, current
   medications, renal state, drug identity, dose, prescriber authority, and
   external exchange receipts under freshness contracts.
6. Clinical decision support produces warnings. The clinician's authenticated
   response is recorded where an override is allowed.
7. Pharmacy verifies the order; dispensing and administration use their own
   transition guards and patient/medication identity checks.
8. Post-commit obligations notify privacy/safety personnel and schedule
   break-glass review.

RFC 0029 can represent this path, but it cannot yet resolve the safety versus
privacy behavior under outages without a first-class safety profile.

### 4.4 Healthcare test matrix

| Test | Expected enforcement | Result | Finding |
| --- | --- | --- | --- |
| Clinician on care team reads ordinary record | Guarded minimum-necessary view | **Pass** | Data guard specialization is a strong fit |
| Unrelated employee browses record | Deny and audit attempt | **Pass** | Requires authoritative relationship and purpose facts |
| Emergency break-glass | Scoped, expiring emergency policy | **Partial** | Meta-policy model fits; capability scoping and single-use/replay semantics need normalization |
| Break-glass exposes entire longitudinal record | Reject excessive scope | **Partial** | Policy can require field scope, but “minimum necessary” is domain-authored and cannot be inferred generically |
| Consent registry unavailable during emergency | Safety-specific fallback | **Fail** | Generic critical-effect fail-closed default is unsafe; no formal fail-operational safety profile exists |
| External allergy feed stale | Warn, deny, or require human handling per policy | **Pass conditionally** | Provenance supports it, but correctness remains conditional on the authority and chosen clinical posture |
| Local and external allergy facts disagree | Resolve/flag under clinical policy | **Fail** | No multi-authority disagreement algebra or uncertainty type |
| Allergy added after order but before administration | Invalidate and re-check at administration | **Partial** | Invalidation exists; dependency-to-transition mapping is missing |
| Prescriber license revoked mid-workflow | Revalidate before next protected effect | **Pass conceptually** | Needs bounded revocation propagation and cancellation behavior |
| Same person orders and verifies prohibited medication | Separation-of-duty deny | **Pass** | Durable workflow and person-equivalence contract fit |
| Emergency override of interaction warning | Human judgment plus durable reason | **Pass honestly** | Nirdosha can require the decision, not certify clinical correctness |
| Wrong-patient alias/merged record | Prevent resource confusion | **Fail** | Canonical patient identity, merge/split, and alias semantics are absent |
| Evidence store unavailable during record read | Deny, buffer, or emergency fallback | **Fail** | Evidence durability is not tied to effect finality or safety posture |
| Research replica bypasses treatment policy | Separate purpose/resource/effect coverage | **Partial** | Coverage model fits; derived-copy identity and policy inheritance are underspecified |
| Search index reveals sensitive terms | Cover derived read effect | **Fail** | No mandatory derivation/alias closure ensures all projections inherit protection |
| Cached clinician decision reused after patient transfer | Resource/context invalidation | **Partial** | Decision validity exists; context-version dependencies require explicit binding |
| PDP outage during active resuscitation | Fail-operational within bounded emergency mode | **Fail** | Needs admitted safety envelope, local policy snapshot, expiry, and retrospective evidence |
| Auditor accesses forensic evidence | Dual-authorized, purpose-bound evidence view | **Pass conceptually** | Corrected evidence profiles support this if secrets/minimization rules are enforced |
| Patient requests explanation | Safe explanation without exposing sensitive security logic | **Partial** | Explanation class exists; audience-specific disclosure rules need semantics |
| Deletion request conflicts with medical retention/legal hold | Authority/jurisdiction conflict | **Fail** | Generic composition does not yet model conflicts of law or retention authority adequately |

### 4.5 Healthcare verdict

The architecture correctly separates:

- access policy from clinical judgment;
- emergency policy from bypass;
- external fact provenance from truth;
- blocking clinical checks from post-commit notification/review;
- workflow durability from atomic database mutation.

It is not yet safe for healthcare implementation because it lacks a formal
fail-operational safety profile, canonical patient/resource identity,
multi-authority uncertainty, derived-data policy inheritance, and evidence
failure semantics.

## 5. Cross-domain failure register

### F1 — canonical resource identity and equivalence

**Failure.** Policies bind to a resource identifier, but real systems contain
aliases, merged identities, external identifiers, replicas, projections,
materialized views, and split records. An attacker or accident can route an
effect through a different identifier for the same protected thing.

**Required change.** Add a Resource Identity and Equivalence contract:

- canonical authority and identifier namespace;
- alias/merge/split/version events;
- equivalence and derivation relations;
- invalidation when identity changes;
- gateway obligation to resolve canonical identity before decision;
- policy-inheritance rules for derived resources;
- ambiguity produces `Indeterminate`.

### F2 — applicability completeness and effect-taxonomy closure

**Failure.** A policy can cover `money.transfer` while settlement-file
injection, migration, backfill, reconciliation adjustment, search-index read,
or administrator mutation performs the same domain effect under another name.

**Required change.** Add an Effect Taxonomy and Closure contract:

- canonical effect identity;
- parent/child and equivalence relationships;
- all gateway operations register effects;
- policy scope declares whether descendants and equivalents are included;
- unclassified effects cannot enter certified scope;
- coverage checks aliases, indirect effects, migrations, and administrative
  operations.

### F3 — decision capability binding and consumption

**Failure.** A permit can be replayed for a different request, resource
version, payload, gateway, or effect unless its binding is normative.

**Required change.** Define a decision capability bound to:

- decision and request identity;
- subject and authority chain;
- canonical resource identity/version;
- action/effect and gateway identity;
- normalized input digest;
- policy bundle and matched rules;
- completed blocking obligations;
- issuance, validity, invalidation, and revocation;
- single-use or bounded-use semantics;
- idempotency and consumption receipt.

### F4 — evidence atomicity and outage behavior

**Failure.** RFC 0029 requires durable evidence but does not state whether an
effect may commit if evidence storage fails.

**Required change.** Every decision class selects one evidence-finality mode:

- `AtomicEvidence`: evidence or an immutable evidence commitment commits with
  the effect;
- `DurableOutboxEvidence`: a local durable record commits with the effect and
  is later exported;
- `SynchronousExternalEvidence`: external evidence acknowledgement is blocking
  and its availability consequences are explicit;
- `EmergencyDeferredEvidence`: only for meta-policy-authorized safety mode,
  using a bounded local journal and mandatory reconciliation;
- `ObservedEvidence`: non-preventive only.

No protected effect may claim durable evidence without one of these modes.

### F5 — distributed finality and irreversible effects

**Failure.** `DurableWorkflow` is too broad to distinguish reserved, locally
posted, submitted, accepted, settled, final, reversed, compensated, and
unknown outcomes. Compensation cannot undo disclosure, medication
administration, or some external settlements.

**Required change.** Add explicit finality and reversibility metadata:

- workflow state and external-finality authority;
- visibility point;
- reversible, compensatable, or irreversible classification;
- timeout/unknown-outcome state;
- compensation semantics that never claim history was atomic;
- which state may be reported to each audience.

### F6 — safety profile and fail-operational modes

**Failure.** “Critical operations fail closed” is not universally safe.
Healthcare, industrial control, and emergency response may require bounded
operation during control-plane failure.

**Required change.** Add a meta-policy-authorized Safety Posture:

- `FailClosed`;
- `FailSafeState`;
- `FailOperationalBounded`;
- `HumanControlledEmergency`.

Each non-fail-closed posture declares activation evidence, cached/local policy
snapshot, permitted effects, scope, duration, local fact requirements,
exposure, notification, reconciliation, and automatic termination. This is
not a generic availability override.

### F7 — multiple authorities, disagreement, and compromise

**Failure.** Axis C distinguishes local from external but not two authorities
that disagree or one that is later compromised.

**Required change.** Per fact type, define:

- authoritative source set and trust threshold;
- precedence, quorum, corroboration, or uncertainty semantics;
- conflict result and safe behavior;
- authority health and revocation;
- retroactive compromise window;
- affected-decision discovery and response;
- evidence needed to prove which authority version was used.

### F8 — policy dependency and transition revalidation

**Failure.** Pinning and invalidation exist, but the architecture does not map
which facts/rules must be revalidated at each workflow transition.

**Required change.** Compile a dependency graph from rule/fact/resource to
decision and transition. Each transition lists required current facts,
permitted pinned facts, revalidation triggers, and maximum stale interval.

### F9 — aggregate consistency

**Failure.** Bounded aggregates such as daily limits may span partitions,
regions, offline channels, or replicas. `Atomic` has no meaning until the
aggregate consistency domain is declared.

**Required change.** Aggregate policy declares authority, key, window,
late-event rule, consistency level, partition behavior, reservation model,
reconciliation, and maximum overshoot if not strictly serialized. Overshoot
is an exposure, not compliance.

### F10 — monitor health and observation completeness

**Failure.** A detection bound is meaningless while the monitor or telemetry
path is down, delayed, sampled, or blind to a bypass.

**Required change.** Continuous claims include:

- observation-source coverage;
- heartbeat and last-complete checkpoint;
- clock/skew assumptions;
- loss, sampling, and backpressure limits;
- detection-bound suspension rule;
- fail/alert behavior when monitor health is unknown;
- evidence of both negative observation and monitor health.

### F11 — policy-plane confidentiality

**Failure.** PDP, evidence, simulation, and replay can become new aggregation
points for financial, health, identity, and security-sensitive data.

**Required change.** Policies declare minimum fact disclosure to PDP/PIP,
confidential-computing or local-evaluation needs where applicable, evidence
classification, field-level access, purpose, retention, deletion, and query
controls. A policy must not require evidence that violates a higher-order data
policy.

### F12 — jurisdiction and applicability selection

**Failure.** Authority tiers alone do not determine which jurisdiction or law
applies to a customer, patient, data location, institution, transaction, or
event.

**Required change.** Add an applicability phase before composition:

- typed jurisdiction and governing-context facts;
- authority for each applicability fact;
- conflict-of-law/manual-escalation result;
- effective-date handling;
- no silent selection when applicability is ambiguous.

## 6. Findings that passed and should be preserved

The tabletop validates several RFC 0029 decisions:

1. Per-decision Axis B is necessary; both scenarios mix Guarded, Atomic, and
   DurableWorkflow behavior.
2. Monitoring must remain bounded-impact rather than prevention.
3. Local versus externally attested facts is useful when combined with
   per-fact provenance.
4. Blocking and post-commit obligations must remain separate.
5. Cross-service compensation must not be called atomicity.
6. Emergency access must be a pre-admitted policy, not an undocumented
   bypass.
7. Human judgment can be required and evidenced without being machine-proven.
8. Tier-2 systems can make useful local claims while refusing a global
   no-bypass claim.
9. Evidence minimization is essential; unrestricted raw forensic capture
   would create unacceptable new risk.
10. Reduction must remain a new, explicitly approved submission.

## 7. Required RFC 0029 amendments before Phase 0 freezes

RFC 0029 now has normative accommodation for:

- resource identity, aliases, merges, splits, and derivation inheritance;
- effect taxonomy closure and effect equivalence;
- decision-capability binding, replay, consumption, and idempotency;
- evidence-finality modes;
- workflow finality, visibility, reversibility, and unknown outcomes;
- safety posture and bounded fail-operational operation;
- multi-authority conflict, thresholds, compromise, and retroactive response;
- rule/fact/transition dependency and revalidation;
- aggregate consistency domains and overshoot exposure;
- monitor health and observation completeness;
- policy-plane confidentiality;
- jurisdiction/applicability selection before policy composition.

RFC 0029's formal-artifact table and delivery phases now require semantics for
each item before its associated guarantee may be implemented. The remaining
work is to formalize and rerun the cases, not merely to retain the headings.

### 7.1 Gap-resolution traceability

| Gap | Normative accommodation in RFC 0029 | Remaining validation |
| --- | --- | --- |
| F1 Resource identity | §§6–7, 12–15, 21, 24, 36.1 | Alias/merge/split/derived-resource conformance cases |
| F2 Effect closure | §§6–7, 12–15, 21, 24, 36.2 | Migration, admin, file, message, and indirect-effect coverage tests |
| F3 Decision capability | §§6, 12–15, 24, 36.3 | Replay, changed-input, double-consume, and idempotent-retry tests |
| F4 Evidence finality | §§6, 12–13, 15, 23–24, 36.4 | Evidence outage at every effect/commit boundary |
| F5 Distributed finality | §§6, 12–13, 15, 22, 26, 36.5 | Unknown outcome and irreversible-effect workflow tests |
| F6 Safety posture | §§6, 12–13, 20, 26, 36.6 | Fail-operational activation, expiry, isolation, and recovery tests |
| F7 Authority conflict | §§6, 12, 17, 20, 24, 36.7 | Disagreement, quorum loss, compromise, and retrospective impact tests |
| F8 Transition revalidation | §§6, 12–13, 18, 22, 36.8 | Fact change at every protected transition boundary |
| F9 Aggregate consistency | §§6, 12, 25, 36.9 | Partition, late event, reservation, and overshoot tests |
| F10 Monitor health | §§6, 11–12, 21, 29, 36.10 | Telemetry loss, skew, sampling, checkpoint, and blind-spot tests |
| F11 Policy-plane confidentiality | §§6, 12, 23–24, 36.11 | Data-minimisation, access, locality, retention, and deletion tests |
| F12 Applicability | §§6, 12–13, 19, 24, 36.12 | Conflicting jurisdiction and ambiguous-applicability tests |

“Accommodation” means the RFC now has a normative home and admission/proof
hook for the gap. It does not mean the formal semantics or implementation has
passed the remaining validation column.

## 8. Re-run gates

After amendment, rerun both scenarios and require:

### Banking gate

- concurrent insufficient-funds attempts cannot overspend;
- daily limits remain within declared consistency/overshoot behavior;
- duplicate and mutated retries are rejected deterministically;
- sanctions/FX facts revalidate at declared transitions;
- local reservation and ledger evidence have explicit atomic finality;
- external unknown outcomes reconcile without duplicate effects;
- every reported transfer status corresponds to a declared finality state;
- account aliases, settlement files, migrations, and admin operations are in
  the effect/resource closure;
- evidence outage follows an admitted mode;
- authority compromise identifies affected decisions.

### Healthcare gate

- normal and emergency access return only the admitted field scope;
- emergency capabilities are patient-, purpose-, action-, and time-bound and
  cannot be replayed elsewhere;
- consent/PDP outage invokes an admitted safety posture;
- conflicting allergy facts produce the declared uncertainty behavior;
- changes before dispense/administration invalidate earlier decisions where
  required;
- patient aliases, merges, replicas, and search projections inherit policy;
- evidence outage does not silently erase access records;
- irreversible administration is never represented as compensatable atomic
  work;
- research and treatment purposes remain separated across derived data;
- retention and deletion conflicts enter explicit applicability resolution.

## 9. Final assessment

The architecture did not collapse under the mock run, but it also did not
pass. Its central abstractions are useful and honest. The failures are at the
hard boundaries real regulated systems expose: identity of the thing being
protected, completeness of the effect surface, distributed finality,
availability-versus-safety decisions, disagreement among authorities, and
evidence that must survive the effect it describes.

Implementation should not begin with a broad generic policy language. The
safer next step is to amend RFC 0029 with F1–F12, formalize a minimal core,
and rerun these scenarios on paper. Only then should implementation select a
small closed slice such as local authorization plus one atomic database
invariant and one durable workflow.
