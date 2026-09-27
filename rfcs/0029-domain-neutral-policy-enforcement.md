# RFC 0029 — Domain-Neutral Policy Admission, Enforcement, and Evidence

```
RFC:           0029
Title:         Domain-neutral policy admission, enforcement, and evidence
Status:        Proposed — reviewed design; no implementation claimed
Created:       2026-09-26
Depends:       RFC 0016 (domain packs)
               RFC 0017 (guarantee manifests)
               RFC 0021.c (coverage and analysis findings)
               RFC 0023 (data guard)
               RFC 0026 (metadata and lineage plane)
Related:       docs/V2_GUARANTEES.md
               docs/nirdosha-rt-dialect.md
Migration:     rfcs/0029-impact-and-migration-plan.md
Validation:    rfcs/evidence/0029/tabletops/banking-healthcare.md
AI/KYC:        rfcs/evidence/0029/tabletops/ai-first-kyc.md
Model port:    rfcs/0029.a-model-assurance-port.md
Medical AI:    rfcs/evidence/0029/tabletops/medical-ai.md
Expansion:     rfcs/evidence/0029/tabletops/cross-domain-expansion.md
```

## 1. Summary

Nirdosha should admit a domain policy only when its terms are typed, its
required facts have named authorities, its protected effects have known
enforcement points, and the requested guarantee is achievable within a
declared platform boundary. Accepted policy is compiled to a typed policy
IR, enforced through effect gateways, and bound to durable decision evidence.

This RFC does **not** claim that the current workspace implements that
architecture. It defines the target contract and the conditions that must be
met before a tool may say that a policy is enforced.

The core promise is:

> For every admitted policy and every effect in its certified scope,
> Nirdosha verifies the declared enforcement mechanism, requires all blocking
> obligations before the effect becomes final, durably tracks post-commit
> obligations, and binds the decision to the exact policy bundle and evidence.
> Effects covered only by monitoring or accepted risk receive a bounded-impact
> or risk-acceptance statement, never an enforcement-completeness claim.

Nirdosha verifies interpretation, coverage, and execution of an admitted
policy. The domain authority remains responsible for whether the policy is
legally, ethically, professionally, and operationally correct.

## 2. Review disposition

This RFC is a reviewed revision of the proposed “Nirdosha: Domain-Neutral
Policy Enforcement Architecture” reference specification. The source design's
main structure is retained: typed admission, three enforcement dimensions,
PDP/PEP separation, effect gateways, obligations, provenance, lifecycle,
evidence, replay, platform tiers, and a decidable IR.

The review made the following normative corrections:

1. A residual window is an **exposure bound**, not an extension of the
   interval over which a property is guaranteed.
2. Tier-2 detection of a bypass supports a **bounded-impact** statement, not a
   claim that the bypass could not occur or that all effects were guarded.
3. Axis B is a mapping from decision/effect classes to atomicity class, not an
   unordered set whose association with decisions is implicit.
4. External attestation is required for authoritative external facts; local
   facts do not acquire meaningless external attestations merely because one
   other input is external.
5. Emergency policy is authorized and bounded by meta-policy. It never ranks
   above or rewrites meta-policy.
6. Forensic evidence is minimised and schema-governed. Raw secrets,
   credentials, unnecessary personal data, and unrestricted payload retention
   are never required merely to claim replay.
7. A deployment declares a trust domain and root set, which may combine a
   genesis policy, hardware attestation, and quorum ceremony. It is not forced
   into exactly one mutually exclusive realization.
8. Policy conflicts are not universally reducible to “higher tier wins.” Deny
   precedence, obligation accumulation, overrides, and incomparable rules are
   explicit parts of each policy kind's composition algebra.
9. “Accepted with reduction” is a new policy submission and bundle identity,
   not an in-place downgrade of an existing policy.

## 3. Goals and non-goals

### 3.1 Goals

The architecture must:

- represent policies from unrelated domains in one typed semantic core;
- reject ambiguous, unobservable, undecidable, uncovered, or under-enforced
  policies rather than issue a false guarantee;
- distinguish prevention, atomic enforcement, durable workflow, observation,
  and continuous bounded-impact response;
- prove or attest that protected effects use the required enforcement path;
- make policy versions, assumptions, authorities, obligations, and evidence
  replayable;
- allow certified domain modules without allowing opaque extensions to inherit
  guarantees they cannot support;
- remain honest about platform and trusted-computing-base assumptions.

### 3.2 Non-goals

Nirdosha does not:

- accept arbitrary natural language as executable policy;
- decide legal, ethical, clinical, safety, or professional correctness;
- establish physical truth independently of trusted observations;
- guarantee availability unless a policy explicitly models it;
- protect against compromise of the declared trust roots or TCB;
- claim static coverage over arbitrary application code;
- make a detected violation equivalent to prevention;
- make cross-service work atomic without an actual distributed transaction;
- replace the accountability of policy owners, operators, or external
  authorities.

## 4. Guarantee boundary

Four questions have different owners:

| Question | Primary owner |
| --- | --- |
| Is this the right policy? | Domain authority |
| Was the admitted policy interpreted unambiguously? | Policy language and compiler |
| Did every effect in certified scope receive the declared enforcement? | Compiler, coverage verifier, gateways, deployment controls |
| Were the facts used by the decision true? | Fact authorities; Nirdosha verifies provenance, validity, and declared freshness |

Every certificate must name its system boundary, platform tier, TCB, policy
bundle, assumptions, certified effect scope, excluded paths, and evidence
profile. No certificate may silently widen those declarations.

## 5. Architecture

```text
                       Domain authorities
            legal · safety · product · security · operations
                                │
                                ▼
                       Domain policy bundle
       vocabulary · rules · invariants · workflows · tests · owners
                                │
                                ▼
                       Policy admission layer
       typed? observable? decidable? covered? strong enough? affordable?
                       │                         │
                    reject                    admit
                                                 │
                                                 ▼
                                          Typed policy IR
                                                 │
                  ┌──────────────────────────────┼───────────────────────────┐
                  ▼                              ▼                           ▼
          static derivations              PDP and PEPs             transaction/workflow
          and coverage graph              runtime decisions         atomicity and recovery
                  │                              │                           │
                  └──────────────────────────────┼───────────────────────────┘
                                                 ▼
                                           Effect gateway
                                                 │
                                                 ▼
                                          Protected effect
                                                 │
                                                 ▼
                                  Evidence · replay · monitoring
```

The architecture is split into:

1. domain vocabulary and modules;
2. canonical typed policy IR;
3. admission and composition;
4. Policy Administration, Information, Decision, and Enforcement Points;
5. typed effect gateways;
6. transactional and workflow adapters;
7. coverage and bypass classification;
8. signed bundle distribution;
9. evidence, replay, and continuous assurance.

## 6. Universal policy model

Every policy compiles to a model containing:

- stable policy and rule identities;
- authority and ownership;
- scope and excluded scope;
- subject, action, canonical resource, effect, and context types;
- resource namespaces, aliases, equivalence, merge/split, and derivation rules;
- effect taxonomy, descendants, equivalents, and closure rules;
- preconditions and input facts;
- fact-authority sets, disagreement, uncertainty, and compromise behavior;
- decisions and decision classes;
- decision-capability binding, permitted reuse, and consumption semantics;
- blocking pre-effect obligations;
- post-commit obligations;
- state transitions, finality, visibility, reversibility, and postconditions;
- failure, timeout, revocation, and invalidation behavior;
- safety posture for control-plane or authority failure;
- fact provenance and freshness requirements;
- evidence profile and evidence-finality mode;
- aggregate consistency domain and any maximum overshoot;
- continuous-monitor observation and health contract;
- policy-plane confidentiality and minimisation requirements;
- jurisdiction/applicability rules and their authorities;
- verification-time requirements;
- atomicity requirement **per decision/effect class**;
- assurance-source summary and per-fact authorities;
- claim interval and any exposure bound;
- platform tier and TCB assumptions;
- cost and complexity budget;
- effective version and bundle hash.

The semantic kernel is domain-neutral:

| Domain | Subject | Action | Resource |
| --- | --- | --- | --- |
| Banking | Customer | Transfer | Account |
| Healthcare | Clinician | Read | Medical record |
| Manufacturing | Operator | Start | Machine |
| Logistics | Carrier | Handover | Shipment |
| Education | Teacher | Update | Student grade |
| Insurance | Adjuster | Approve | Claim |
| Cloud | Workload identity | Deploy | Cluster |
| AI | Agent | Invoke | Tool |

## 7. Domain vocabulary and certified modules

The core does not embed the meaning of an account, patient, shipment, or
machine. A domain module contributes versioned definitions for:

- entities, fields, states, actions, and relationships;
- units and conversions;
- classifications, roles, claims, and principals;
- predicates and bounded aggregate windows;
- effect identities and gateways;
- canonical resource namespaces, identity resolvers, equivalence and
  derivation relations;
- effect taxonomies, descendants, equivalents, and closure declarations;
- workflow templates;
- external fact providers;
- evidence schemas and simulation scenarios.

Unknown fields, actions, states, units, or relationships are admission
errors. Unit conversions must be explicit and deterministic.

A module also declares semantics, termination bound, computational
complexity, effects, trust assumptions, failure behavior, supported proof
obligations, and compatibility range. An extension predicate outside the
core decidable logic is **opaque**. Opaque predicates may be runtime-guarded
or monitored, but cannot participate in a Static claim until their semantics
and proof rule are admitted.

Per §31, a domain authority *defines* domain vocabulary; a module owner
*operates* it. That split only holds if a domain authority is never required
to personally author the formal artifacts above. Each domain module must
therefore additionally declare a domain-authority-facing authoring surface:
a bounded, human-readable template, DSL, or UI-constraint set that lets a
domain authority express a policy-kind instance (an §8 row — "qualified
clinician approves treatment" and similar) using only already-admitted
domain vocabulary terms, with no direct exposure to the module's semantics,
composition algebra, or proof obligations. The module owner is responsible
for building this surface and for translating an accepted submission from it
into the formal representation admission requires; a module without one is
not admission-complete for domain-authority-authored policy kinds.

### 7.1 Service catalogs are the translation boundary

An authoring surface may accept a bounded human-readable request, including a
screen or module description, but it must not translate unconstrained English
directly into executable policy. The supported pattern is a versioned domain
**service catalog** maintained by the module owner and approved by the domain
authority. A catalog entry names the already-admitted resource, effect,
policy kind, enforcement gateway, required facts/evidence, review mode,
failure posture, and the parameters that an author may configure.

An authoring request selects a catalog entry and supplies only those declared
parameters. The compiler then performs this lowering:

```text
human-readable request or screen intent
  -> catalog entry + validated parameters
  -> typed policy draft
  -> canonical RFC 0029 IR
  -> admission and AdmissionReportV1
  -> generated route/capability/gateway binding
```

The catalog, not the screen and not an LLM, is the source of the
security-critical meaning. A screen action such as `transfer.create` may be
mapped to a pre-reviewed service, while `funds.reserve`, `funds.release`,
and `transfer.approve` remain distinct effects with distinct authorities,
evidence, and gateways. UI visibility or a role label never grants an effect
by implication.

Natural-language assistance may propose a catalog entry or fill a declared
parameter, but it must surface missing or ambiguous values for confirmation.
It cannot invent an effect, broaden an authority, select a gateway, or turn an
opaque predicate into a Static claim. An unresolved parameter is an admission
failure or `Indeterminate`, according to the declared failure rule. A module
without a complete catalog-to-IR mapping is an authoring convenience only and
is not an RFC 0029 enforcement frontend.

## 8. Policy kinds

The first standard kinds are:

| Kind | Example | Natural enforcement |
| --- | --- | --- |
| Authorization | Only assigned clinicians may read a record | Runtime guarded authorization |
| Data policy | Researchers receive de-identified fields | Guarded read plus transformation |
| Numeric invariant | Stock after allocation is non-negative | Atomic guard |
| State invariant | Cancelled orders cannot ship | Guarded/atomic state transition |
| Transition | Draft → reviewed → approved | Durable workflow |
| Sequence | Sterilize before packaging | Durable workflow or verified local sequence |
| Separation of duty | Author cannot approve own change | Identity-bound durable workflow |
| Resource lifecycle | Maintenance lock held during service | Guard plus lifecycle analysis |
| Temporal | Result valid for 72 hours | Trusted clock plus invalidation |
| Aggregate | At most five requests per day | Atomic bounded counter/window |
| External evidence | Certification must be active | Guard plus attested fact |
| Operational | Data remains in an approved region | Deployment guard plus continuous assurance |
| Physical world | Temperature remains below threshold | Attested sensor plus response policy |
| Human judgment | Qualified clinician approves treatment | Authenticated durable approval |

Each policy kind has a formal operational semantics, composition algebra,
proof obligations, allowed enforcement phases, and minimum evidence schema.
No kind is admitted before those artifacts exist. §8.1–§8.4 give these for
the first four kinds, satisfying the Phase 0 exit gate's own requirement
(readiness matrix §10) that they be normative before that gate closes; the
remaining ten kinds' artifacts remain part of the RFC 0029/0029.a
formal-artifact backlog (readiness matrix §12).

### 8.1 Authorization

**Operational semantics.** A decision `d` of kind Authorization over
(subject `S`, resource `R`, action `A`) is satisfied iff an admitted rule
binds `(S, R, A)` to `permit`, evaluated against `S`'s and `R`'s current
authoritative facts (role/grant, canonical resource identity per §36.1),
and no admitted rule of equal or higher declared precedence binds
`(S, R, A)` to `deny`. Absent any binding rule, the result is `deny`
(fail-closed); a policy may declare a default `permit` only by explicitly
naming the authority accepting that risk (§20).

**Composition algebra.** Rules over the same `(S, R, A)` resolve by
declared precedence; absent one, `deny` overrides `permit` (§19's default).
Rules over disjoint `(S, R, A)` spaces never conflict.

**Proof obligations.** `Guarded`: the permit decision is evaluated and
returns `permit`, using facts no staler than the rule's declared freshness
bound, before the gateway executes `A` on `R`, and is bound to the *exact*
resource and effect identity the gateway executes — never a broader or
narrower one. `Atomic`: additionally, no invalidating event (grant
revocation, a change to `R`'s protected attributes) may occur between the
permit and the effect's commit within the same atomic boundary.

**Enforcement phases.** `Guarded` and `Atomic` only. `Advisory`/`None`
foreclose the guarantee this kind exists to make — a permission that does
not gate the effect is not an authorization. `DurableWorkflow` applies only
when the authorized action is itself entry into an admitted durable
transition (§8's Transition kind), not to the atomic grant/deny act itself.

**Minimum evidence schema.** The authority-granting role/grant record, the
resolved rule id, the bound resource/effect identity, the evaluation
timestamp, and either `permitted` or the specific denying rule.

### 8.2 Data policy

**Operational semantics.** A decision of kind Data policy is satisfied iff
every field of a returned resource projection subject to a declared
transformation (redaction, de-identification, aggregation) has that exact
transformation applied before the projection crosses the enforcement
boundary, and no untransformed copy of a protected field is observable on
the delivered path.

**Composition algebra.** Multiple data policies over overlapping fields
compose by the strictest declared transformation per field. A declared
composition order changes the result (redact-then-aggregate is not
aggregate-then-redact); absent a declared order, the single strictest
per-field transformation applies (§19's conservative default).

**Proof obligations.** `Guarded`: the transformation is applied
server-side, inside the enforcement boundary, before the projection is
returned; the untransformed values never cross that boundary in the
response. `Atomic` is not meaningful here — a pure read-side projection
gates no effect to make atomic with a check.

**Enforcement phases.** `Guarded` (default) or `Advisory` (an audit-only
mode that logs what would have been redacted without enforcing it, and
must be named as non-enforcing). `Atomic`/`DurableWorkflow` are foreclosed.

**Minimum evidence schema.** Which transformation function and version was
applied, to which fields, and a commitment proving the returned projection
is consistent with applying that function to the authoritative source —
not merely an assertion that it was applied.

### 8.3 Numeric invariant

**Operational semantics.** A decision of kind Numeric invariant over
quantity `Q` is satisfied iff, immediately after the effect commits, `Q`'s
declared invariant predicate (e.g. `Q >= 0`) holds against the
authoritative post-commit value of `Q`, computed within the same atomic
boundary as the effect — never from a stale or optimistically-read
pre-commit estimate. This is the same property `funds_reserve`'s
`atomic-commit-integrity` check (§5 below) already computes for one
concrete instance.

**Composition algebra.** Multiple invariants over the *same* quantity
compose conjunctively — all must hold, and none overrides another; if two
are jointly unsatisfiable for a given effect, admission rejects
(`Ambiguous`/`UnsupportedSemantics`, §12), it does not silently relax
either. Invariants over different quantities are independent.

**Proof obligations.** `Atomic` (default, and the only phase for an
invariant whose violation is itself the harm being prevented): the read of
`Q`, the invariant check, and the mutation share one atomic boundary
excluding concurrent interleaving. `Guarded` alone (check, then a
non-atomic later commit) is inadmissible unless the policy explicitly
declares and names a bounded overshoot exposure (§11).

**Enforcement phases.** `Atomic` (default); `Guarded` only with a named
overshoot exposure; `Advisory`/`None`/`DurableWorkflow` foreclosed — an
invariant not enforced atomically is not invariant.

**Minimum evidence schema.** Pre-value, post-value, the invariant predicate
evaluated, and the atomic-commit receipt.

### 8.4 State invariant

**Operational semantics.** A decision of kind State invariant over an
entity's state field is satisfied iff the entity's current authoritative
state, read within the same atomic boundary as the gated effect, is a
member of the declared allowed-source-states set for the attempted effect.

**Composition algebra.** Multiple state invariants over the same entity
compose conjunctively — the effect is allowed only if the current state is
in *every* declared invariant's allowed set; they never widen each other.

**Proof obligations.** `Guarded`: no state-changing transition occurs
between the state read and the effect commit. `Atomic`: excluded by the
same transaction boundary instead of a non-atomic gap. `DurableWorkflow`
applies when the state is itself defined by an admitted state machine
(§8's Transition kind) rather than a bare field — State invariant gates an
*effect* conditioned on state; Transition governs the state *change*
itself, a related but distinct concern.

**Enforcement phases.** `Guarded`, `Atomic`, or `DurableWorkflow` (per the
state-machine distinction above). `Advisory` only as an explicitly-named
non-enforcing logging mode. `None` is foreclosed.

**Minimum evidence schema.** The observed state at check time, the declared
allowed-source-states set, and — for `Atomic` — the same atomic-commit
receipt structure as Numeric invariant.

## 9. Platform tiers

### 9.1 Tier 1 — closed effect platform

All in-scope effects are capability-mediated; direct effect APIs, native
escape hatches, reflection, and unrestricted store clients are absent or
formally outside scope. The compiler and platform attestation can support a
Static coverage claim.

Tier 1 is a target architecture. The current Nirdosha Rust dialect is not
declared Tier 1 merely because it uses Rust or a rustc driver.

### 9.2 Tier 2 — controlled gateways

In-scope effects are expected to use gateways and deployment controls can
attest, restrict, or monitor bypass paths. Individual guarded or atomic
decisions can be certified where their gateway evidence supports it.
Platform-wide no-bypass coverage cannot be claimed. Detected bypass paths
receive bounded-impact statements with detection and response bounds.

### 9.3 Tier 3 — advisory or observational

Gateways are not mandatory. Only advisory, observed, or monitor-only results
are available. No enforcement-completeness claim is made.

### 9.4 Admission versus reduction

If submitted requirements are unachievable at the declared tier, admission
rejects the submission. There is no silent downgrade.

A reduced policy is a new submission with a new bundle identity. It records
the original request, reduced request, reason, authorized approver, and
foreclosed guarantee. Admission outcomes are exactly one of:

- `Accepted`;
- `AcceptedWithAssumptions`;
- `AcceptedReduced` for a separately submitted reduced bundle;
- `MonitorOnly`, which earns no enforcement guarantee;
- `Rejected` with a stable reason code.

Named assumptions preserve, rather than lower, the requested enforcement and
are revalidated on activation.

### 9.5 Platform-tier enforceability matrix

§9.1–9.3 and §10 are stated as separate concerns deliberately — a domain-
neutral model must not hard-wire enforcement dimensions to any one platform's
physical capabilities. That separation does not mean every axis value is
achievable at every tier. Admission item 17 (§12) and the `UnachievableTier`
rejection reason are checked against exactly this table, not against an
unstated assumption:

| Axis B value | Tier 1 | Tier 2 | Tier 3 |
| --- | --- | --- | --- |
| `None` | Achievable | Achievable | Achievable |
| `Advisory` | Achievable | Achievable | Achievable |
| `Guarded` | Achievable | Achievable only where the gateway's own evidence covers the check-before-effect ordering for that decision class | Unachievable — §9.3 gateways are not mandatory, so no blocking checkpoint exists |
| `Atomic` | Achievable | Achievable only where the gateway's own transaction boundary covers check, obligations, and commit | Unachievable |
| `DurableWorkflow` | Achievable | Achievable only where the gateway supplies a durable executor with recovery semantics | Unachievable |

| Axis A value | Tier 1 | Tier 2 | Tier 3 |
| --- | --- | --- | --- |
| `Static` | Achievable — compiler derivation covers mediated paths | Unachievable — unmediated paths exist outside compiler coverage | Unachievable |
| `Runtime` | Achievable | Achievable, bounded by the same gateway evidence Axis B relies on | Achievable only as a non-blocking, advisory check — it does not by itself imply the effect was gated |
| `Continuous` | Achievable | Achievable, bounded by the gateway's declared detection/response bounds — a bypass is detected, not prevented (§9.2) | Achievable as observation only; no enforcement-completeness claim follows from it (§9.3) |

A submission naming an Axis B/A value the declared tier cannot achieve is
rejected `UnachievableTier`; it is not silently reduced to a weaker value
(§9.4). A Tier 2 submission claiming `Guarded` or `Atomic` must name the
specific gateway evidence that closes the relevant row above — a bare tier
declaration is not sufficient admission evidence for that row.

## 10. Enforcement dimensions

The original single “strength ladder” is replaced by orthogonal dimensions.

### 10.1 Axis A — verification time

A policy declares one or more mechanisms:

| Value | Meaning |
| --- | --- |
| `Static` | A Tier-1 compiler derivation covers the declared effect paths |
| `Runtime` | A PDP/PEP check occurs for a decision with a bounded validity interval |
| `Continuous` | A monitor observes the property with declared detection and response bounds |

These mechanisms may be combined. Their parameters remain attached to the
mechanism rather than being flattened into a single interval.

### 10.2 Axis B — atomicity by decision class

Each decision/effect class maps to exactly one value:

| Value | Meaning |
| --- | --- |
| `None` | Detection only; no preventive claim |
| `Advisory` | Warn and permit |
| `Guarded` | Successful check and blocking obligations precede the effect |
| `Atomic` | Check, obligations, and effect commit in one transaction boundary |
| `DurableWorkflow` | Transition and obligations are durably logged with recovery semantics |

The bundle stores a mapping such as `read → Guarded`, `write → Atomic`; a bare
set `{Guarded, Atomic}` is insufficient because it does not say which class
receives which guarantee.

### 10.3 Axis C — assurance source

The policy summary is:

| Value | Meaning |
| --- | --- |
| `Local` | Every authoritative fact originates within the declared local TCB |
| `ExternallyAttested` | At least one authoritative fact originates outside the local TCB and is accepted only with the declared attestation, validity, and revocation checks |

The summary does not erase per-fact provenance. Proof obligations apply an
external-attestation check only to external authoritative facts; local facts
retain their local authority and integrity requirements.

## 11. Claim intervals and exposure bounds

Three concepts must not be conflated:

- **decision validity**: how long a decision may be reused before
  re-evaluation;
- **property claim interval**: the time interval over which the stated
  property is actually claimed;
- **exposure bound**: the maximum time from a violation to detection and
  completion of the declared response.

For a continuous monitor:

```text
exposure_bound = detection_bound + response_bound
```

That exposure bound is not part of the property claim interval and does not
turn a violation into compliance. It supports only this statement:

> Violations may occur; the mechanism is designed to detect them within the
> detection bound and complete the declared response within the response
> bound.

An approver may accept that bounded-impact posture, but approval cannot
upgrade it to prevention. Zero exposure may be claimed only when the actual
mechanism provides zero-time prevention or atomic rejection.

## 12. Policy admission

Admission checks:

1. the rule and scope are unambiguous;
2. all terms are typed;
3. expressions lie in an admitted decidable fragment;
4. required facts are observable;
5. every fact has an authority, provenance contract, validity, and failure
   behavior;
6. every resource resolves to a canonical identity or produces
   `Indeterminate`; aliases, merges, splits, replicas, and derived resources
   have declared inheritance behavior;
7. every protected effect belongs to a closed effect taxonomy and has an
   identified enforcement point, including equivalent, administrative,
   migration, backfill, file, message, and indirect effects;
8. Axis A mechanisms meet the policy-kind minimum;
9. every Axis B decision mapping meets its minimum;
10. Axis C and per-fact provenance are consistent, including authority
    disagreement and compromise behavior;
11. the decision capability has complete binding, expiry, invalidation,
    idempotency, and consumption semantics;
12. the dependency graph from facts, rules, resources, and authorities to
    decisions and transitions is complete for the certified scope, and every
    transition declares which facts must be current, which may remain
    pinned, its invalidation triggers, and its maximum permitted staleness
    (§18, §36.8) — an incomplete map is an admission failure here, not only
    an Appendix A checklist item;
13. blocking obligations are executable before finality;
14. post-commit obligations have a durable executor, retry, deadline, and
    terminal handling;
15. each decision class declares evidence finality and evidence-outage
    behavior;
16. workflow states declare external finality, visibility, reversibility, and
    unknown-outcome behavior;
17. the certified scope is covered at the declared platform tier;
18. conflicts are resolved by the policy kind's composition algebra after
    jurisdiction and applicability selection;
19. aggregate policies declare their consistency domain, partition behavior,
    and any maximum overshoot exposure;
20. claim intervals and exposure bounds are consistent and honestly worded;
21. continuous claims include observation coverage and monitor-health
    semantics;
22. evidence and PDP/PIP inputs satisfy policy-plane confidentiality and can
    be collected without violating a governing data policy;
23. the selected fail-closed or fail-operational safety posture is admitted,
    and where a canonical-identity result may be `Indeterminate` for an
    effect a non-`FailClosed` posture also governs, the bundle states which
    of the two controls (§13.1, §36.6) — identity ambiguity is not silently
    overridden by an emergency posture, or vice versa, by omission;
24. the policy fits the declared latency, storage, and fan-out envelope;
25. meta-policy authorizes the submission and activation, and, where the
    policy declares any compensation or correction obligation (§36.5), names
    the meta-policy-authorized authority permitted to trigger it, on the same
    pattern §16 requires for waivers.

Rejection reasons include `Ambiguous`, `UnsupportedSemantics`,
`Undecidable`, `MissingAuthority`, `MissingEnforcementPoint`,
`InsufficientAxisA`, `InsufficientAxisB`, `AssuranceMismatch`,
`UnachievableTier`, `UncontrolledBypass`, `UnresolvedResourceIdentity`,
`UnclassifiedEffect`, `DecisionBindingIncomplete`, `DependencyMappingIncomplete`,
`EvidenceConflict`, `EvidenceFinalityUnsupported`, `FinalityStateUnsupported`,
`AuthorityConflict`, `ApplicabilityAmbiguous`, `SafetyPostureUnsupported`,
`MonitorHealthUnknown`, `AggregateConsistencyUndeclared`, and `OverBudget`.

## 13. Decision and enforcement protocol

Every protected operation follows this logical protocol:

1. resolve jurisdiction/applicability, identify the canonical effect, and
   select the decision class;
2. establish the authenticated subject and authority chain;
3. resolve canonical resource identity, equivalence, derivation, and version;
4. collect facts under their provenance contracts;
5. resolve the exact effective bundle;
6. evaluate applicable rules;
7. apply the kind-specific composition algebra;
8. produce a bound decision capability and obligations;
9. complete blocking obligations;
10. consume or reserve the capability and perform the effect through its
    bound gateway;
11. verify pre-commit postconditions;
12. commit the effect and its required evidence commitment, reject, or enter
    the declared unknown-outcome/compensation path;
13. durably enqueue post-commit obligations;
14. write decision and effect evidence.

Decisions are `Permit`, `Deny`, `PendingApproval`,
`PermitWithObligations`, `Defer`, and `Indeterminate`. Critical effects fail
closed on `Indeterminate` unless the policy explicitly and validly defines a
different safety posture.

An `Indeterminate` canonical-identity result (§36.1) and an active,
meta-policy-authorized non-`FailClosed` posture (§36.6) can coincide on the
same in-flight decision — for example an ambiguous or merged patient identity
surfacing during an active `HumanControlledEmergency`. Precedence is
explicit, not left to the implementer: identity ambiguity continues to fail
close the effect **unless** the active posture's own admitted declaration
names canonical-identity ambiguity as a condition it is specifically
authorized to operate through, with its own bounded scope, evidence, and
expiry for that exact case. A posture that is silent on identity ambiguity
does not implicitly cover it; the effect still fails closed. This is the
same default the rest of this RFC uses throughout — an unaddressed
interaction resolves toward the safer, more restrictive outcome, never
toward silent availability.

## 14. PDP, PIP, PEP, and PAP

- The **Policy Administration Point** authors, validates, approves, signs,
  activates, revokes, and distributes bundles.
- The **Policy Information Point** obtains facts and attaches provenance,
  validity, freshness, and revocation status.
- The **Policy Decision Point** evaluates a versioned bundle and returns a
  decision capability bound to the subject, authority chain, canonical
  resource and version, effect and gateway, normalized request digest, policy
  bundle, matched rules, obligations, issuance/expiry, invalidation set,
  permitted-use count, and explanation/evidence class.
- The **Policy Enforcement Point** ensures that the returned decision and
  obligations govern the protected effect.

Decision validity is the maximum cache lifetime. A platform may shorten it
but may never extend it. Resource-version changes and declared invalidation
events expire a decision early. A reusable cache entry is not itself authority
to perform an effect: the PEP must instantiate or consume a capability under
the policy's reuse and idempotency rules, and produce a consumption receipt.

## 15. Effect gateways

Protected effects occur through typed gateways:

```text
application
    │
    ▼
typed decision/capability
    │
    ▼
effect gateway
    ├── database
    ├── message
    ├── file/export
    ├── network/provider
    ├── device
    ├── administrative control
    └── agent tool
```

Each gateway declares canonical effect identities and taxonomy relationships,
canonical resource namespaces and resolvers, transaction and isolation
semantics, idempotency, decision-capability validation/consumption, resource
versioning, supported obligation phases, finality/visibility/reversibility,
failure and unknown-outcome modes, and evidence-finality production. An
adapter cannot claim `Atomic` merely because its
implementation uses a lock or transaction somewhere; the policy check and
effect must share the declared atomic boundary.

## 16. Enforcement phases and obligations

Phases are:

- admission;
- precondition;
- atomic guard;
- transition guard;
- pre-commit postcondition;
- post-commit obligation;
- continuous invariant;
- retrospective control.

A phase mismatch is an admission error.

Obligations are either:

- **blocking pre-effect**, which must succeed before finality; or
- **post-commit**, which must be durably recorded before the effect's commit
  becomes externally acknowledged and then tracked to a terminal state.

Obligation states are `Pending`, `Running`, `Succeeded`, `Failed`,
`TimedOut`, `Compensated`, and `Waived`. Every instance declares an
idempotency key, retry policy, owner, deadline, escalation, evidence, and
compensation behavior.

A waiver requires a meta-policy-authorized authority, explicit scope, expiry,
reason, and review obligation. A waiver changes the applicable policy result;
it does not erase the original obligation or its evidence.

Triggering a compensation or correction obligation (§36.5) follows the same
pattern: a meta-policy-authorized authority, explicit scope, reason, and
review obligation, recorded as evidence bound to the compensating effect.
Reversibility/compensatability classification (§36.5) states what kind of
effect a correction may be; it does not by itself say who may trigger one.
Absent a more specific domain-module declaration, the authority that may
trigger a correction is the same authority the policy names for waivers
within that decision class — this is a default, not a domain-neutral
judgment about who *should* hold that authority in a given business.

## 17. Fact provenance

Every decision input records:

- stable fact type;
- value or protected value reference;
- issuer and authority domain;
- authenticated channel or signature;
- observation and collection times;
- validity and freshness interval;
- resource version where applicable;
- revocation status;
- transformations and derivation lineage;
- declared confidence where meaningful;
- failure behavior.

Each fact type also declares its authoritative source set and whether it uses
one source, precedence, quorum, corroboration, or an uncertainty result.
Conflicting sources cannot be resolved by incidental response order. Authority
health, revocation, retroactive compromise, affected-decision discovery, and
the required containment or re-evaluation behavior are part of the contract.

Caller-supplied data is not authoritative merely because it has the expected
shape. Facts derived from sensors or human inspection remain conditional on
the declared calibration, identity, and environmental assumptions.

## 18. Invalidation and time-of-check/time-of-use

The default invalidation events are:

- resource version changes;
- an input fact expires;
- a relevant authority, subject, or grant is revoked;
- an unpinned policy bundle is superseded;
- a policy-declared state transition occurs.

For `Guarded`, no declared invalidation may occur between the check and effect
commit without re-evaluation. For `Atomic`, the gateway must exclude relevant
interleavings inside the transaction boundary. Continuous monitoring after
commit does not repair a violated guarded or atomic proof obligation; it is a
separate bounded-impact mechanism.

The compiler/admission layer builds an explicit dependency graph from facts,
rules, resources, authorities, and bundles to decisions and workflow
transitions. Each transition declares facts that must be current, facts that
may remain pinned, invalidation triggers, and maximum permitted staleness.

## 19. Policy composition

Policies may come from platform, jurisdiction, industry, organization,
department, product, resource, subject-specific, and emergency sources.

Composition begins only after an applicability phase resolves the governing
contexts (for example jurisdiction, establishment, data location, product,
event location, and subject status) from typed, authoritative facts. Ambiguous
applicability produces `Indeterminate` or a declared manual escalation; the
engine never silently selects a convenient jurisdiction.

Composition has two layers:

1. a partial authority order and explicit, signed delegation/override edges;
2. the policy kind's decision and obligation algebra.

Normative defaults are:

- mandatory denies dominate permits within the same applicable composition
  domain;
- mandatory compatible obligations accumulate;
- incompatible obligations are an admission or decision error;
- a higher authority is a floor unless it explicitly delegates an override
  dimension, scope, and effective interval;
- incomparable authorities that disagree produce `Indeterminate`, not an
  arbitrary lexicographic winner;
- no applicable rule fails closed for a protected effect unless the governing
  meta-policy explicitly defines another posture.

Specificity and explicit priority break ties only where the policy kind's
formal algebra permits them. Every decision records applied, overridden,
inapplicable, and conflicting rules.

## 20. Meta-policy, emergency policy, and trust roots

Meta-policy governs who may author, review, approve, sign, activate, revoke,
waive, and operate policy. It also governs separation of duty, emergency
scenarios, evidence access, and root rotation.

Emergency policy is a meta-policy-authorized policy for a predeclared
scenario. It does **not** rank above, override, or rewrite meta-policy. It may
override ordinary policy only within the dimensions, scope, duration, and
obligations that meta-policy already authorizes.

Emergency activation requires an authenticated activator, reason, scope,
automatic expiry, durable evidence, notification, and post-hoc review. A
rejected review marks the activation and resulting effects as violations and
starts declared compensation; it cannot pretend the effects never occurred.

Each deployment declares one trust domain with a versioned root set. The root
set may combine:

- an offline quorum-signed genesis policy;
- hardware/measured-boot attestation;
- witnessed governance ceremony;
- threshold keys and explicitly authorized subordinate issuers.

Root compromise compromises claims rooted in that trust domain. Rotation,
revocation, and recovery are themselves meta-policy-governed ceremonies. Fact
authorities use the disagreement and compromise contract in §36.7; a trusted
root does not make two contradictory facts simultaneously true.

## 21. Coverage graph and bypass classification

The coverage graph connects:

```text
policy → protected action/effect → entry points → call paths
       → enforcement point → gateway → store/provider/device
       → obligations → evidence
```

Coverage is computed over canonical resource equivalence/derivation closure
and effect taxonomy closure, not only literal resource and action names. That
closure includes migrations, backfills, administrative operations, derived
stores and indexes, settlement/import files, message producers, and indirect
gateway effects where applicable.

Certification fails for the requested scope when there is an unprotected
effect, missing PEP, missing policy, unenforced obligation, missing executor,
missing evidence path, unauthorised fact, transactional invariant outside its
boundary, or unhandled failure path.

Each bypass path is classified as:

- **Prevented**: Tier-1 derivation proves it unreachable;
- **AttestedRestricted**: deployment attestation and controls exclude the path
  for the attestation validity interval;
- **DetectedAndResponded**: violations remain possible, with explicit
  detection and response bounds;
- **AcceptedRisk**: a named authority accepts bounded scope and exposure;
- **Uncontrolled**: no valid control exists.

Only `Prevented` and, within its validity and assumptions,
`AttestedRestricted` contribute to enforcement-completeness coverage.
`DetectedAndResponded` contributes a bounded-impact statement.
`AcceptedRisk` is an exclusion from the guarantee. `Uncontrolled` rejects the
requested certification.

Tools must never label detected or accepted-risk paths “closed,” “prevented,”
or “cannot be bypassed.”

Continuous coverage is valid only while its observation sources and monitor
health satisfy §36.10. Missing heartbeats, incomplete checkpoints, excessive
loss/sampling, or blind telemetry suspend the detection-bound claim rather
than silently extending it.

## 22. Bundles, lifecycle, and durable workflow pinning

Policy bundles are immutable, content-addressed, signed, dependency-pinned,
effective-dated, revocable, and replayable.

```text
Draft → Typechecked → Simulated → Reviewed → Approved → Signed
      → Staged → Active → Superseded → Retired/Revoked
```

Every decision records the exact bundle hash.

A durable workflow pins a bundle at initiation by default. A workflow policy
may instead require evaluation at every transition. Safety-critical
revocation forces re-evaluation before the next protected effect; it does not
automatically permit an in-flight step to continue. Each step declares a
cancellation model and a maximum non-interruptible interval. If a running
effect cannot be cancelled, that interval is an explicit exposure, and the
workflow enters the declared stop, quarantine, or compensation path.

Every workflow state that may be exposed outside the workflow declares its
finality authority and status, visibility audience, and whether its effects
are reversible, compensatable, or irreversible. `UnknownOutcome` is a
first-class state. Compensation records a later effect; it never rewrites
history into an atomic claim.

## 23. Evidence and replay

Evidence profiles are selected per policy and jurisdiction. The standard
profiles are:

| Profile | Purpose | Typical content |
| --- | --- | --- |
| Forensic | Authorized deterministic or semantic replay | Minimal encrypted replay envelope, protected fact references or payloads where necessary, external receipts, bundle and runtime identity |
| Audit | Control and compliance review | Hashes, rule IDs, decision metadata, obligation results, redacted subject/resource data |
| Explanation | Subject/operator-facing reason | Authorized rule summary and safe reason codes |

An Explanation profile declares one or more named audiences (for example
subject/patient/customer, operator, auditor, and regulator) and, per
audience, exactly which rule summary, reason codes, and evidence references
that audience may receive. Content that is safe for an internal operator or
auditor is not automatically safe for the subject the decision was about,
and vice versa; a single undifferentiated "safe reason code" is not a
complete Explanation profile. Audience tiering is a specialization of the
policy-plane confidentiality contract (§36.11) applied specifically to the
explanation artifact, not a separate confidentiality model.

Forensic evidence is not synonymous with unrestricted raw capture. Its schema
must identify which values are necessary for the declared replay class.
Passwords, private keys, bearer credentials, prohibited payment
authentication data, and unrelated personal data are never retained for
replay. Token claims or authority receipts replace reusable credentials.

Replay may be:

- byte-exact, when all nondeterminism and permitted inputs are captured;
- semantic, when protected references can reconstruct equivalent typed facts;
- decision verification, when hashes and authority receipts establish that a
  prior decision was valid without reconstructing all execution.

Evidence is append-only or equivalently tamper-evident, encrypted according to
classification, access-controlled, and bound to chain-of-custody records.
Deletion follows retention and legal-hold policy and emits an authorized
tombstone. Hash chains provide integrity evidence, not issuer authenticity,
unless anchored by a trusted signature or external transparency service.

Every decision/effect class selects an evidence-finality mode:

- `AtomicEvidence`: evidence or an immutable commitment shares the effect's
  atomic boundary;
- `DurableOutboxEvidence`: a durable local evidence record shares the effect's
  commit and is later exported;
- `SynchronousExternalEvidence`: external acknowledgement is a blocking
  obligation and its availability consequences are declared;
- `EmergencyDeferredEvidence`: a meta-policy-authorized safety mode writes a
  bounded local journal and mandates reconciliation;
- `ObservedEvidence`: non-preventive evidence only.

An effect cannot claim durable decision evidence without one of these modes.
Policy-plane data collection is itself governed: PDP/PIP inputs, evidence,
simulation, and replay declare classification, purpose, minimisation,
retention, access, locality, and deletion/hold behavior. An evidence profile
that violates a governing data policy is rejected.

## 24. Formal semantics and proof obligations

Before implementation claims a policy kind or enforcement strength, the
following normative artifacts must exist:

| Artifact | Required before |
| --- | --- |
| Grammar and canonical encoding | Shipping a parser or signed bundle |
| Type system and unit rules | Shipping admission |
| Operational semantics per policy kind | Admitting that kind |
| Temporal semantics | Temporal/continuous policies |
| Composition algebra | Resolving conflicts |
| Resource identity/equivalence and derivation semantics | Certifying resource-scoped coverage |
| Effect taxonomy and closure semantics | Certifying effect coverage |
| Decision-capability issue/consume semantics | Reusing or consuming a permit |
| Authority disagreement and compromise semantics | Admitting multi-authority facts |
| Applicability and jurisdiction semantics | Composing context-dependent authorities |
| Evidence-finality semantics | Claiming durable evidence |
| Finality/reversibility semantics | Exposing distributed workflow outcomes |
| Safety-posture semantics | Any fail-operational behavior |
| Aggregate consistency semantics | Admitting aggregate policies |
| Monitor-health semantics | Making Continuous claims |
| Policy-plane confidentiality semantics | Collecting protected facts/evidence |
| Proof obligations for each supported enforcement combination | Static or Atomic claims |
| Complexity and decidability boundary | Accepting expressions/extensions |

Minimum proof schemas are:

- **Guarded**: every committed effect has a preceding permit on the same
  path, all blocking obligations completed, and no declared invalidation
  occurred before commit.
- **Atomic**: the permit, blocking obligations, postcondition, and effect share
  one declared atomic boundary that excludes invalidating interleavings.
- **DurableWorkflow**: every committed transition belongs to the admitted
  state machine under its pinned/evaluated bundle; transition and obligations
  are durably ordered and recoverable.
- **Static**: a Tier-1 derivation covers every in-scope effect path and binds
  it to a gateway and policy meeting the requested dimensions.
- **ExternallyAttested**: every authoritative **external** fact has a valid,
  unrevoked attestation from its declared authority and satisfies freshness
  and failure semantics.
- **Continuous**: every violation in the monitor's observation model is
  detected within the declared bound and its response reaches a declared
  terminal state within the response bound, with durable evidence. This is a
  bounded-impact claim, not absence of violation.
- **Decision capability**: every committed effect presents an unexpired,
  unrevoked, correctly bound and permitted-use capability; consumption is
  idempotent and produces a receipt bound to the effect result.
- **Evidence finality**: every effect claiming durable evidence satisfies its
  selected mode, including the declared failure behavior when the external
  evidence plane is unavailable.
- **Resource/effect closure**: every in-scope alias, derived resource,
  equivalent effect, and administrative/indirect path is either covered or
  named as an exclusion; ambiguity cannot be certified as coverage.

## 25. Decidability and budgets

The core IR admits only bounded, terminating expressions:

- no unbounded recursion;
- aggregates only over declared bounded windows;
- no unbounded universal history quantification;
- admitted temporal logic restricted to a decidable bounded fragment;
- external calls have latency, freshness, and failure contracts;
- opaque extension predicates disable incompatible static claims.

Every policy declares a cost envelope: decision latency, complexity class,
bounded collection sizes, monitoring interval, detection/response bounds,
evidence storage estimate, and obligation fan-out. The platform declares its
aggregate capacity envelope. Over-budget submissions are rejected; a reduced
submission must receive a new bundle identity.

Aggregate policies additionally declare the aggregate authority, key,
bounded window, late-event behavior, consistency domain, partition and outage
behavior, reservation model, reconciliation, and maximum overshoot. Any
non-zero overshoot is an exposure bound, not satisfaction of a strict
invariant.

## 26. Distributed execution

Gateways declare isolation, locking or optimistic concurrency, idempotency,
commit behavior, outbox/inbox behavior, retry safety, timeout handling, and
delivery semantics.

Cross-service work is `Atomic` only when a real distributed transaction
mechanism supplies the required boundary. Otherwise it is a
`DurableWorkflow`, `Guarded`, or observational process with explicitly stated
partial-failure and compensation semantics. Compensation does not make an
already visible effect historically atomic.

PDP unavailability behavior is policy-defined. Reuse of a cached permit is
limited by decision validity, fact expiry, resource-version matching,
revocation bounds, and platform maximums.

The governing meta-policy assigns each affected decision class one safety
posture: `FailClosed`, `FailSafeState`, `FailOperationalBounded`, or
`HumanControlledEmergency`. Non-fail-closed modes declare activation,
permitted effects, local policy snapshot, required local facts, duration,
scope, exposure, notification, journal/evidence mode, reconciliation, and
automatic termination. Availability pressure alone cannot activate them.

## 27. Simulation and assurance

Policy packs include owner-authored:

- permit and deny cases;
- boundary values;
- missing, stale, and revoked fact cases;
- concurrency and invalidation cases;
- retry and partial-failure cases;
- policy-conflict cases;
- adversarial bypass cases;
- obligation failure and compensation cases.

Generated cases supplement but never replace owner requirements. Shadow and
canary evaluation report changed decisions, newly triggered obligations,
unmatched rules, conflicts, cost impact, and assurance reductions before
activation.

## 28. Interoperability

Cedar, OPA/Rego, XACML-like models, OAuth/OIDC, SCIM, SPIFFE/SPIRE, and other
systems may be front ends or authorities. Imported policy is translated into
the canonical IR and re-admitted.

Translation is strength-preserving only when a proof or conformance result
establishes semantic preservation for the used subset. Dynamic built-ins,
open-world entity behavior, partial evaluation, or unsupported relationships
become typed external facts or opaque predicates. If requested enforcement no
longer holds, import is rejected or the owner submits a separately identified
reduced policy.

## 29. Operational requirements

Operators expose metrics and alerts for decision latency and availability,
bundle freshness, revocation propagation, stale facts, bypass attempts,
obligation backlog/failure, evidence durability, gateway failures, and
monitor coverage and health.

Runbooks cover PDP outage, policy corruption, root/key compromise, evidence
failure, gateway failure, revocation storms, emergency activation, quarantine,
rollback, compensation, and post-incident replay. Recovery tests validate
policy/evidence restoration, root recovery, regional failover, workflow
resumption, and absence of duplicate protected effects.

## 30. AI agents and physical systems

AI-agent policies additionally model agent identity, delegation chains,
per-tool effects, tool sandboxing, output validation, model/version
provenance, nondeterminism, prompt-injection boundaries, memory access,
rate limits, and human approval.

Physical-system policies additionally model sensor identity and calibration,
tamper evidence, sampling and freshness, real-time deadlines and jitter,
fail-safe versus fail-operational posture, manual override, emergency stop,
maintenance locks, and the applicable external safety certification.

These declarations do not cause Nirdosha certification to replace an
industry safety assessment.

## 31. Responsibility model

| Responsibility | Defines | Verifies | Operates |
| --- | --- | --- | --- |
| Desired and legally correct policy | Domain authority | Domain governance | — |
| Domain vocabulary | Domain authority | Nirdosha admission | Module owner |
| Typed interpretation | Language team | Conformance/proof suite | Compiler |
| Conflict resolution | Domain/meta-policy | Nirdosha algebra | PDP |
| Tier-1 coverage | Scope owner | Compiler and attestation | Build/deployment gate |
| Tier-2 gateway control | Scope owner | Attestation/monitoring | Operator |
| Runtime decision | Policy owner | PDP conformance | PDP service |
| Authentic facts | Fact authority | Provenance verifier | Identity/sensor/provider |
| Atomicity | Policy owner | Gateway conformance | Store/transaction system |
| Durable obligations | Policy owner | Workflow conformance | Runtime and humans |
| Evidence and retention | Policy/legal owner | Evidence verifier | Evidence store |
| Incident response | Domain/operator | Evidence and alert checks | Operator |
| Meta-policy and roots | Governance | Quorum/ceremony | HSM/root custodians |

## 32. Relationship to existing Nirdosha work

This RFC does not replace existing mechanisms:

- RFC 0016's domain packs can become signed domain modules and mandatory
  invariant inputs.
- RFC 0017's guarantee bundles can carry the admitted bundle, scope, effect
  coverage, and enforcement results.
- RFC 0023's guard IR and PDP/PEP/store ladder are the data-policy
  specialization of this architecture.
- RFC 0026's declared/observed lineage can supply fact derivation and evidence
  relationships.
- `nirdosha-contract-core`, `nirdosha-driver`, `nirdosha-guard-*`,
  `nirdosha-workflow`, and `nirdosha-audit` are candidate implementation
  components, not proof that this RFC already ships.

Existing source-scan certificates do not satisfy this RFC's coverage or
enforcement claims. Existing macros remain useful enforcement mechanisms but
must be connected to the canonical IR and coverage graph before they earn the
new guarantee.

The per-RFC responsibility changes, probable implementation surfaces, staged
migration order, and completion checks are maintained in the companion
[responsibility impact and migration plan](./0029-impact-and-migration-plan.md).
The failure-oriented banking and healthcare exercise, including the blockers
that must be resolved before Phase 0 freezes, is recorded in the companion
[tabletop validation](./evidence/0029/tabletops/banking-healthcare.md).
The AI-first KYC exercise, including the distinction between conditionally
admissible risk-based automation and rejected fully autonomous adjudication,
is recorded in the companion
[AI-first KYC tabletop](./evidence/0029/tabletops/ai-first-kyc.md).
The provider-neutral identity, status, inference receipt, assurance promise,
lifecycle, independent-monitoring, and impact interface required to close its
A1–A8 gaps is specified by
[RFC 0029.a](./0029.a-model-assurance-port.md).
Its safety-critical medical evaluation and proposed clinical-decision-support
profile are recorded in the companion
[medical AI tabletop](./evidence/0029/tabletops/medical-ai.md).
A further manufacturing and insurance exercise, covering one AI-first and one
non-AI critical process in each domain, is recorded in the
[cross-domain expansion tabletop](./evidence/0029/tabletops/cross-domain-expansion.md).

## 33. Phased delivery and acceptance criteria

### Phase 0 — formal core

- grammar, canonical encoding, type system, vocabulary model;
- formal semantics for authorization, numeric invariant, transition, and
  separation-of-duty kinds;
- composition algebra and invalidation model;
- resource/effect closure, decision capability, evidence finality, finality
  and reversibility, safety posture, authority conflict, applicability,
  aggregate consistency, monitor health, and policy-plane confidentiality
  semantics from §36;
- stable admission result and diagnostic schema;
- domain-authority-facing authoring surfaces (§7) for the policy kinds this
  phase gives formal semantics to, so a domain authority can author an
  instance without touching the underlying formal artifacts.

### Phase 1 — bundles and admission

- signed, content-addressed bundles with root-set validation;
- admission checks for axes, decision mappings, provenance, phases, cost, and
  evidence compatibility;
- canonical resource/effect registries, authority/applicability rules, and
  evidence-finality selection;
- lifecycle and activation registry;
- conformance suite proving no silent reduction.

### Phase 2 — runtime enforcement

- PAP/PIP/PDP/PEP interfaces;
- database, message, network, and export gateway contracts;
- blocking and post-commit obligation runtimes;
- durable workflow pinning, invalidation, and revocation behavior;
- decision-capability issuance/consumption, explicit finality states, safety
  postures, and evidence-outage behavior.

### Phase 3 — coverage and evidence

- coverage graph and stable bypass classifications;
- alias/derivation and effect-taxonomy closure plus monitor-health evidence;
- Tier-2 attestation/monitor evidence;
- evidence profiles, chain of custody, and replay classes;
- guarantee-bundle integration.

### Phase 4 — constrained Tier 1

- a closed effect subset with capability-only gateways;
- sound compiler derivation for the supported subset;
- platform attestation binding compiler, runtime, bundle, and deployment;
- adversarial attempts demonstrating that direct in-scope effects fail.

No phase may advertise a later phase's guarantee. In particular, Phase 2
runtime guards do not imply Phase 4 static no-bypass coverage.

## 34. End-to-end example: sterilization before packaging

A policy requires sterilization service A to complete before packaging service
B starts for a batch.

Declared posture:

- Axis A: `Runtime` for the transition and `Continuous` for bypass detection;
- Axis B mapping: `package.start → DurableWorkflow`;
- Axis C: `Local` only if both services and their evidence authority are
  inside the declared TCB; otherwise `ExternallyAttested`;
- property claim: the admitted workflow transition never starts packaging
  without valid sterilization evidence;
- bypass posture at Tier 2: direct packaging writes may remain possible but
  are detected within 30 seconds and quarantined/responded to within five
  minutes; this is a 5 minute 30 second exposure bound, not an extension of
  the property guarantee.

At the packaging transition the PEP obtains the pinned bundle and current
batch version, verifies sterilization evidence, acquires the packaging lock,
and commits the transition durably. QA notification and a future audit are
post-commit obligations recorded in the workflow/outbox.

Because services A and B do not share a transaction, the policy does not
claim `Atomic`. If a safety-critical revocation arrives, the next protected
effect requires re-evaluation. A currently executing non-cancellable step may
continue only for its declared maximum interval and is recorded as an
exposure requiring the configured stop, quarantine, or compensation response.

Forensic replay stores the minimum protected facts or authority receipts
needed for the declared replay class; it does not store reusable credentials
or unrelated raw personal data. Audit evidence stores hashes, rule identities,
decisions, and obligation results. Explanation evidence exposes a safe reason
appropriate to its audience.

## 35. Final principle

Nirdosha does not claim:

> Any text called a policy will be honoured.

It claims, once the corresponding phases exist:

> Any policy admitted into the typed policy model is enforced for the effects
> in its certified scope, within its declared platform and trust boundary, by
> mechanisms meeting its verification-time, per-decision atomicity, provenance,
> obligation, and evidence requirements. Unsupported semantics, missing facts,
> incomplete coverage, insufficient enforcement, uncontrolled bypasses, and
> over-budget execution cause rejection. Monitoring and accepted risk are
> reported as bounded-impact or exclusions, never as prevention.

## 36. Cross-domain hardening requirements

The banking and healthcare tabletop found twelve requirements shared across
otherwise unrelated domains. They are normative parts of admission and proof,
not optional future refinements.

### 36.1 Canonical resource identity and equivalence

Every protected resource belongs to a canonical authority and identifier
namespace. A domain module declares aliases, merges, splits, versions,
replicas, projections, indexes, and derivation relations. The PEP resolves the
canonical resource before decision. Derived-resource policy inheritance is
explicit. Ambiguous or conflicting identity produces `Indeterminate`, which
fail-closes a critical effect by default even under an active §36.6 posture
unless that posture explicitly names identity ambiguity as a condition it is
authorized to operate through (§13).

### 36.2 Effect taxonomy and closure

Every gateway operation has a canonical effect identity with parent/child and
equivalence relationships. Policy scope states whether descendants and
equivalents are included. Coverage includes indirect effects, migrations,
backfills, administrative paths, files, messages, and external calls. An
unclassified effect cannot enter certified scope.

### 36.3 Decision capability binding and consumption

A permit is represented as a capability bound to decision/request identity,
subject and authority chain, canonical resource and version, effect and
gateway, normalized input digest, bundle and rules, blocking obligations,
issuance/expiry/invalidation/revocation, use count, and idempotency key. The
gateway validates and atomically reserves or consumes it and writes a
consumption receipt. Reuse with changed inputs is rejected.

### 36.4 Evidence finality

Every decision class selects one evidence-finality mode from §23 and declares
outage behavior. Durable evidence means an evidence record or immutable
commitment is bound to the effect's finality through that mode. A later
best-effort log is `ObservedEvidence`, not durable decision evidence.

### 36.5 Distributed finality and irreversible effects

Workflows distinguish requested, reserved, locally committed, submitted,
accepted, settled, final, reversed, compensated, and unknown outcomes as
applicable. Each exposed state names its authority. Effects are classified as
reversible, compensatable, or irreversible. Compensation is a later recorded
effect and cannot establish historical atomicity or undo disclosure or a
physical action. Triggering a compensation or correction obligation requires
a named, meta-policy-authorized authority on the same pattern §16 requires
for waivers; classification alone does not say who may act on it.

### 36.6 Safety posture and bounded fail-operational modes

Meta-policy assigns a safety posture per decision class. A bounded
fail-operational or human-controlled emergency mode is itself an admitted
policy with authenticated activation, narrow effects/scope, trusted local
snapshot and facts, hard expiry, exposure statement, evidence journal,
notification, reconciliation, and automatic termination. It is not an
implicit exception to policy. Where a canonical-identity `Indeterminate`
result (§36.1) could coincide with an active posture, the posture's own
admission record states whether it covers that condition; silence means the
identity ambiguity still fail-closes the effect (§13).

A decision class assigned `FailOperationalBounded` may additionally declare
a revocation circuit-breaker threshold: a maximum count of simultaneously
affected decisions (§36.8) from one invalidating event, beyond which the
posture activates using its already-required trusted local snapshot instead
of synchronously re-evaluating every affected decision at once. This is not
the "availability pressure" trigger §26 forbids — it is a named, admitted
fan-out bound tied to one specific invalidating event's blast radius, not a
generic load escape hatch, and it inherits every other requirement this
section already places on a bounded fail-operational mode (hard expiry,
exposure statement, evidence journal, notification, reconciliation,
automatic termination). A decision class with no declared threshold gets no
circuit breaker: it re-evaluates synchronously, however large the affected
set, exactly as §36.8 already requires.

### 36.7 Multiple authorities, disagreement, and compromise

Each authoritative fact type declares its source set and precedence, quorum,
corroboration, or uncertainty rule. Conflicts produce the declared safe result
and evidence. Authority revocation or compromise declares a retroactive
window, supports affected-decision discovery, and triggers containment,
re-evaluation, notification, or compensation as applicable.

### 36.8 Dependency and transition revalidation

Admission compiles dependencies from policy/rule/fact/resource/authority to
decisions and transitions. Each transition declares which facts must be
current, which may remain pinned, invalidating events, and maximum staleness.
The runtime re-evaluates exactly the affected decisions and records why. An
incomplete dependency map for the certified scope is an admission failure
(§12 item 12, `DependencyMappingIncomplete`), not only an Appendix A
checklist item.

Where a single invalidating event's affected-decision set exceeds a declared
revocation circuit-breaker threshold (§36.6), the runtime enters that named
posture instead of synchronously re-evaluating every affected decision — a
declared exception to "re-evaluates exactly the affected decisions" that
must itself be named in the transition's admission record, never assumed by
default. Without a declared threshold, a single high-fan-out authority
revocation re-evaluates its entire affected set synchronously; a
certified scope with unbounded fan-out and no threshold is exposed to that
cost at admission time, not discovered operationally.

### 36.9 Aggregate consistency

Aggregate rules declare authority, key, bounded window, late-event behavior,
consistency domain, partition/outage behavior, reservation, reconciliation,
and maximum overshoot. A strict invariant requires a mechanism capable of
zero overshoot within the declared domain; otherwise the result is a
bounded-impact aggregate claim.

### 36.10 Monitor health and observation completeness

A Continuous claim declares observation sources, effect/resource coverage,
heartbeat, last complete checkpoint, time/skew assumptions, loss/sampling and
backpressure bounds, and the behavior when health becomes unknown. Detection
bounds are suspended when those preconditions fail; tools expose the gap
rather than report an uninterrupted claim.

### 36.11 Policy-plane confidentiality

PDP/PIP facts, evidence, simulation, and replay are protected resources. Each
declares minimum disclosure, purpose, classification, locality, encryption,
access, retention, legal hold, deletion, and query controls. Local evaluation
or an admitted confidential-computing mechanism is used where facts may not
be disclosed centrally. Policy enforcement cannot require evidence forbidden
by a higher-order governing data policy. Audience-tiered disclosure for the
Explanation evidence profile (§23) is a specialization of this contract: what
an auditor or regulator may see is not automatically what the subject the
decision was about may see, and a single undifferentiated reason code does
not satisfy either audience's disclosure rule on its own.

### 36.12 Jurisdiction and applicability

Applicability is resolved before composition from typed authoritative facts
such as subject establishment, resource/data location, product, institution,
event location, and effective date. Conflicts of law produce an admitted
resolution or manual escalation. Ambiguous applicability cannot silently
select a policy and cannot earn an enforcement guarantee.

## Appendix A — admission checklist

- [ ] Scope, exclusions, policy owner, and platform tier declared.
- [ ] Every declared Axis A/B value is achievable at the declared platform
      tier (§9.5); a Tier 2 `Guarded`/`Atomic` claim names the specific
      gateway evidence that closes it.
- [ ] Vocabulary, units, actions, states, and effects typecheck.
- [ ] Each domain module used by this policy declares a domain-authority-
      facing authoring surface for the policy kinds it supports (§7); no
      domain-authority-authored instance bypasses it.
- [ ] Canonical resource identities, aliases, derivations, and inheritance
      close over the requested scope.
- [ ] Effect taxonomy descendants/equivalents and indirect/admin paths close
      over the requested scope.
- [ ] Expressions are in an admitted decidable fragment.
- [ ] Every fact has authority, freshness, revocation, and failure semantics.
- [ ] Multi-authority disagreement and compromise behavior is defined.
- [ ] Jurisdiction/applicability is resolved or safely escalated.
- [ ] Axis A mechanisms satisfy the policy kind.
- [ ] Axis B maps every decision/effect class to one adequate mechanism.
- [ ] Axis C summary matches per-fact provenance.
- [ ] Decision capability binding, permitted reuse, consumption, and
      idempotency are complete.
- [ ] Enforcement phases are valid.
- [ ] Blocking obligations are executable before finality.
- [ ] Post-commit obligations have durable tracking and terminal handling.
- [ ] Evidence-finality mode and evidence-outage behavior are declared.
- [ ] Workflow finality, visibility, reversibility, and unknown outcomes are
      declared.
- [ ] Safety posture is admitted for each affected decision class, and its
      declaration states whether it covers a coinciding canonical-identity
      `Indeterminate` result or leaves identity ambiguity fail-closed.
- [ ] Aggregate consistency domain and any overshoot exposure are declared.
- [ ] Rule/fact/transition dependencies and revalidation triggers are mapped
      (§12 item 12; an incomplete map is `DependencyMappingIncomplete`, not
      only this checklist item).
- [ ] A `FailOperationalBounded` decision class with high-fan-out dependency
      exposure either declares a revocation circuit-breaker threshold
      (§36.6, §36.8) or is knowingly accepting synchronous re-evaluation of
      its entire affected set on a single invalidating event.
- [ ] Compensation/correction obligations name their meta-policy-authorized
      triggering authority on the same pattern required for waivers.
- [ ] Explanation evidence declares its audience tiers and per-audience
      disclosure, not one undifferentiated reason code.
- [ ] Composition is resolved by the admitted kind-specific algebra.
- [ ] Meta-policy authorizes author, approver, signer, and activation.
- [ ] Coverage graph includes every effect in requested scope.
- [ ] Bypasses are classified; risk exclusions are not described as coverage.
- [ ] Continuous claims include observation coverage and monitor health.
- [ ] Decision validity, property claim interval, and exposure bound are
      distinct and consistent.
- [ ] Evidence profile supports its replay class without prohibited capture.
- [ ] Policy-plane confidentiality and minimisation requirements are met.
- [ ] Policy fits the platform cost envelope.
- [ ] Translation from an external policy language has been re-admitted.
- [ ] Tests cover permit, deny, boundary, stale fact, invalidation,
      concurrency, retry, partial failure, conflict, bypass, and revocation.

## Appendix B — minimum viable implementation

This appendix defines the minimum for an RFC 0029 implementation claim, not
the minimum experiment. The smaller banking `funds.reserve` pilot in the
[convergence plan](./0029-phase0-readiness-and-domain-matrix.md#convergence-and-independent-validation-plan)
must pressure-test the candidate IR and compiled admission report earlier,
without claiming completion of this appendix.

1. Canonical typed IR and decidability boundary.
2. Vocabulary and unit validation.
3. Formal semantics for the first four policy kinds.
4. Admission with explicit rejection and no silent reduction.
5. Signed bundles, lifecycle, root set, and exact bundle binding.
6. PDP/PIP/PEP interfaces and typed effect identities.
7. At least one conformant guarded gateway and one atomic gateway.
8. Blocking and durable post-commit obligations.
9. Per-fact provenance, freshness, and invalidation.
10. Coverage graph with honest bypass classifications.
11. Evidence profiles and at least decision-verification replay.
12. Guarantee artifact that names scope, assumptions, exclusions, and tier.
13. Canonical resource/effect closure and decision-capability consumption.
14. Evidence-finality and distributed-finality models.
15. Safety posture, authority conflict, transition revalidation, aggregate
    consistency, monitor health, confidentiality, and applicability semantics.
