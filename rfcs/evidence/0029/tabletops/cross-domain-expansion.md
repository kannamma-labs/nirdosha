# RFC 0029 — Cross-Domain Expansion Tabletop

```
Companion to:  RFC 0029 — Domain-neutral policy enforcement
               RFC 0029.a — Provider-neutral Model Assurance Port
Status:        Superseded as a pre-merge result by the unified rerun;
               retained as discovery evidence; no implementation
Created:       2026-09-26
Domains:       Industrial manufacturing
               Insurance
Processes:     AI-first and non-AI in each domain
Purpose:       Detect abstractions that must be generic before domain APIs
               and profiles stabilize
```

## 1. Executive verdict

The architecture generalizes beyond banking and healthcare, but this original
run identified additional core work. G1–G6 have since been merged into RFC
0029.a and the results rerun in
[the unified conformance report](./unified-conformance-rerun.md).

Four processes were tested:

1. **Manufacturing, non-AI:** lockout/tagout, maintenance authorization, and
   machine-start interlocks.
2. **Manufacturing, AI-first:** predictive maintenance and visual quality
   inspection influencing machine operation and product release.
3. **Insurance, non-AI:** effective-dated policy coverage, claim adjudication,
   reserve, approval, and payout.
4. **Insurance, AI-first:** damage estimation, fraud triage, risk assistance,
   and customer-facing adverse decisions.

Results:

| Process | Result |
| --- | --- |
| Non-AI lockout/interlock | **Conditionally admissible**, but the core lacks real-time/physical actuation and safety-case semantics |
| AI predictive maintenance/quality inspection | **Conditionally admissible** for advisory or bounded automatic quarantine; autonomous hazardous machine control is rejected without a safety profile |
| Non-AI insurance claim/payout | **Conditionally admissible**, but effective-dated contract interpretation and multi-party payout finality need stronger semantics |
| AI damage/fraud/risk assistance | **Conditionally admissible** with abstention and meaningful review; unattended denial, cancellation, or pricing based on opaque inference is rejected |

The architecture should remain layered. The generic core needs a few new
cross-domain contracts, while manufacturing and insurance retain domain
profiles that assign concrete meanings and tests.

## 2. External baseline

This tabletop uses representative authoritative sources:

- [OSHA control of hazardous energy / lockout-tagout](https://www.osha.gov/control-hazardous-energy)
  for physical maintenance safety concepts;
- [NIST AI Risk Management Framework](https://www.nist.gov/itl/ai-risk-management-framework)
  for general AI risk-management concepts;
- [NAIC artificial intelligence resources](https://content.naic.org/insurance-topics/artificial-intelligence)
  for insurance-sector AI governance context;
- applicable machinery safety, labor, insurance, consumer-protection,
  privacy, anti-discrimination, financial, and local regulatory requirements.

The exercise is not a certification, safety case, insurance-law opinion, or
finding that any particular deployment complies with these sources.

## 3. Domain A — industrial manufacturing

### 3.1 System boundary

A factory operates robotic production cells with programmable controllers,
energy-isolation hardware, safety relays, machine-vision inspection, a
manufacturing execution system, maintenance management, quality release,
historian, and remote vendor support.

Actors and authorities include:

- operator;
- maintenance technician;
- authorized lockout/tagout worker;
- safety engineer;
- production supervisor;
- quality engineer;
- machine controller and safety PLC;
- energy-isolation devices and sensors;
- AI maintenance/inspection models;
- plant management and regulatory authorities.

Protected effects include machine start/stop, hazardous-energy isolation,
maintenance-mode entry, interlock override, robot motion, batch release,
scrap/rework, maintenance scheduling, and safety-incident response.

## 4. Manufacturing non-AI process: maintenance lockout and restart

### 4.1 Policy

1. Maintenance cannot begin until all declared energy sources are isolated.
2. Isolation is verified by an authorized person using required procedure.
3. Each worker's personal lock remains independently represented.
4. Machine start is impossible while a required lock or safety condition is
   active.
5. Removing another person's lock follows a distinct emergency procedure.
6. Restart requires tool/person clearance, guard restoration, authorization,
   warning, and controller readiness.
7. Administrative software cannot substitute for physical energy isolation.
8. Every override is scoped, expiring, authorized, and reviewed.

### 4.2 Trace

```text
Work order approved
  → machine identity/configuration resolved
  → energy-isolation plan selected
  → production stopped
  → energy sources physically isolated
  → personal locks attached
  → zero-energy verification recorded
  → maintenance capability issued
  → maintenance performed
  → workers clear and remove own locks
  → guard/tool/person checks
  → authorized restart capability
  → physical controller accepts start
  → evidence committed
```

The physical safety controller remains authoritative for hazardous actuation.
Nirdosha governs capabilities, workflow, configuration, evidence, and
software-controlled effects; it must not claim that a database row physically
isolated energy.

### 4.3 Failure matrix

| Test | Required result | Architecture result |
| --- | --- | --- |
| Unauthorized worker requests maintenance mode | Deny | **Pass conceptually** through role/capability policy |
| Wrong machine alias/configuration selected | Deny or indeterminate | **Pass conceptually** through canonical resource identity |
| One energy source omitted from isolation plan | Reject plan/restart | **Fail/partial** — no generic configuration-baseline/completeness proof |
| Database says isolated but physical contactor remains energized | Physical verification blocks work | **Fail** if software evidence is treated as truth; requires physical attestation profile |
| Sensor reports stale zero-energy state | Reject on freshness | **Pass conceptually** through fact provenance |
| Two sensors disagree | Safe state/escalation | **Pass conceptually** through authority-disagreement rules |
| Network to central PDP fails after isolation | Safe local procedure continues without unsafe restart | **Partial** — safety posture fits; edge/local authority semantics need hard real-time profile |
| Controller receives start while lock active | Hardware interlock rejects | **Outside software-only guarantee**; can be externally attested and tested |
| Supervisor removes worker's lock casually | Deny; emergency removal workflow only | **Pass conceptually** through effect separation/meta-policy |
| PLC program/configuration silently changes | Invalidate safety case and suspend operation | **Partial** — identity/change binding exists; certified configuration-baseline semantics missing |
| Evidence store fails during restart | Local durable/atomic commitment or fail safe | **Pass conceptually** through evidence-finality modes |
| Deadline missed in emergency stop | Hardware safety path acts within bound | **Fail** — generic architecture has latency budgets but no real-time deadline/jitter proof model |

### 4.4 Manufacturing non-AI verdict

The workflow and authorization model is sound, but software policy cannot be
the sole safety mechanism. Admission must distinguish:

- logical authorization;
- controller command;
- physical actuation acknowledgement;
- independently verified physical state;
- safety-rated hardware enforcement.

The guarantee must stop at the last attested boundary it can justify.

## 5. Manufacturing AI-first process

Two models are introduced:

- a predictive-maintenance model estimating failure risk from vibration,
  temperature, acoustic, and controller data;
- a vision model classifying product defects and influencing release,
  quarantine, rework, or scrap.

### 5.1 Permitted initial effects

- prioritize inspection or maintenance;
- create a maintenance recommendation;
- place a reversible quality hold/quarantine;
- route an item for human inspection;
- create a non-executable draft work order.

### 5.2 Prohibited initial effects

- defeat a safety interlock;
- autonomously enter a hazardous machine cell;
- restart hazardous equipment;
- permanently scrap high-value goods without admitted review;
- release safety-critical product solely because the model reported healthy;
- close its own anomaly or quality case where independence is required.

### 5.3 AI manufacturing failure matrix

| Test | Required result | Architecture result |
| --- | --- | --- |
| Model validated for machine type A used on type B | OOD/deny or shadow | **Pass conceptually** through intended-use scope |
| Sensor replaced with different calibration | Invalidate deployment validation | **Partial** — model identity fits; asset/sensor configuration baseline must be first-class |
| Vibration sensor drifts slowly | Detect through independent monitoring | **Pass conceptually** through monitor health and drift |
| Sensor and model share same corrupted data path | Do not count as independent evidence | **Pass conceptually** through evidence independence if lineage is complete |
| Model recommends maintenance during critical production | Apply operational policy and human decision | **Pass conceptually** through typed recommendation/workflow |
| Model predicts safe condition but hardware interlock faults | Hardware/interlock wins | **Pass conceptually** through authority floor; needs manufacturing profile |
| Vision model degrades under new lighting | Narrow/suspend affected line/camera scope | **Pass conceptually** through scoped effective state |
| False rejection rate drives excessive scrap | Detect outcome harm and stop automatic scrap | **Partial** — generic outcome/harm contract proposed but not yet merged into MAP core |
| Defect labels come only from model-routed samples | Detect selective-label feedback | **Pass conceptually** after generic feedback semantics are added |
| Operator rubber-stamps every model hold/release | Meaningful-review failure | **Pass conceptually** through human-system contract, implementation absent |
| Model autonomously stops machine | Permit only under separately validated bounded safety/operational protocol | **Rejected by default**, correctly |
| Model autonomously restarts machine | Reject | **Pass as admission rejection** |
| Remote vendor changes weights | New identity, shadow/revalidate | **Pass conceptually** through MAP identity |
| Monitor blind during network partition | Suspend claim; local safe posture | **Pass conceptually**, edge/local behavior needs profile |

### 5.4 AI manufacturing verdict

AI can prioritize, recommend, draft, or apply reversible quarantine inside a
bounded policy. Hazardous actuation requires a separate industrial safety
profile and hardware-backed safety case. MAP status must be scoped by machine,
line, sensor/configuration baseline, product, environment, task, and effect.

## 6. Manufacturing-specific profile gaps

The unified candidate profile that carries these requirements across both
model-free and model-influenced decisions is
[RFC 0029 Domain Profile — Industrial Manufacturing](../../../0029-domain-profiles.md#manufacturing-profile).
It is a Phase 0 representation artifact, not an implementation or
certification result.

### MF1 — asset/configuration baseline

Canonical identity includes machine, controller program, firmware, tooling,
guards, energy sources, sensor identities/calibration, network topology,
product recipe, and safety configuration. A material change invalidates the
applicable assurance case.

### MF2 — physical command and acknowledgement

Command requested, command accepted, actuation observed, and safe state
verified are distinct states with distinct authorities. Lack of
acknowledgement is `UnknownOutcome`, never assumed success.

### MF3 — real-time and edge safety

Policies declare hard/soft deadlines, jitter, clock source, worst-case
execution, network-partition behavior, local policy snapshot, safe state, and
hardware fallback. Cloud/PDP latency cannot be placed on a safety-critical
emergency-stop path unless its bound is actually certified.

### MF4 — safety case and integrity level

The domain profile binds external safety analysis/certification, hazard,
risk-reduction claim, component assumptions, proof/test evidence, and change
control. Nirdosha tracks and enforces the declared case; it does not assign an
industry safety-integrity level itself.

### MF5 — personal lock and physical custody

Personal lock ownership, attachment, removal, and emergency removal are
physical/human facts with authenticated evidence and explicit limitations.
Software identity cannot alone prove custody or physical attachment.

### MF6 — product release and traceability

Release binds material/batch/serial identity, process/configuration versions,
inspection/model/human evidence, nonconformances, disposition, and downstream
recall scope. Derived/reworked/split/merged batches preserve lineage.

## 7. Domain B — insurance

### 7.1 System boundary

An insurer sells motor/property policies and handles claims through customer,
broker, adjuster, repairer, fraud, payment, reinsurance, subrogation, and
complaint/appeal systems.

Protected effects include quote, bind, renew, cancel, change coverage,
establish reserve, accept/deny claim, request evidence, refer fraud,
communicate an adverse decision, approve payout, issue payment, recover funds,
and close/reopen a claim.

## 8. Insurance non-AI process: claim adjudication and payout

### 8.1 Policy

1. The exact policy contract/version effective at the loss time governs
   coverage, subject to applicable law and endorsements.
2. Insured asset/person, peril, loss date/location, exclusions, deductibles,
   limits, prior payments, and evidence are resolved from authoritative
   sources.
3. Coverage interpretation and contested facts may require a qualified
   adjuster/legal workflow.
4. Reserve is distinct from approved settlement and paid amount.
5. Approval and payment follow monetary limits and separation of duty.
6. Payout, ledger, tax, recovery, and evidence records are idempotent and
   transactionally consistent within their declared boundaries.
7. Denial, partial approval, and cancellation produce accurate reasons and an
   appeal/complaint route.
8. Reopening, correction, subrogation, salvage, and reinsurance preserve
   history rather than rewriting it.

### 8.2 Non-AI insurance failure matrix

| Test | Required result | Architecture result |
| --- | --- | --- |
| Claim uses wrong policy version | Resolve effective contract at loss time | **Partial** — bundle versioning exists; domain-contract applicability and endorsement precedence need a profile |
| Policy changed after loss | Historical contract still governs as applicable | **Pass conceptually** through effective dating/pinning |
| Coverage clause is ambiguous | Human/legal escalation, not arbitrary evaluation | **Pass conceptually** through `Indeterminate`/human workflow |
| Duplicate claim/payment retry | Idempotent original result | **Pass conceptually** through capability/idempotency/evidence finality |
| Reserve mistaken for payout authority | Deny distinct effect | **Pass conceptually** through effect taxonomy |
| Adjuster approves above authority | Escalate/deny | **Pass conceptually** through role/limit/approval policy |
| Same person changes payee and approves payout | Separation-of-duty deny | **Pass conceptually** through durable workflow |
| Repairer, customer, and assessor estimates disagree | Preserve sources/uncertainty and adjudicate | **Pass conceptually** through multi-authority conflict |
| Payment provider accepts but times out | Unknown outcome/reconcile | **Pass conceptually** through distributed finality |
| Denial notice reason differs from actual decision | Reject notice or correct before dispatch | **Partial** — explanation fidelity exists; contract-to-reason trace needs profile |
| Court/regulator changes interpretation retroactively | Identify affected claims and governed reopening | **Partial** — authority compromise/changes exist; legal precedent/effective scope needs profile |
| Catastrophe creates claim surge | Preserve controls under degraded capacity | **Partial** — cost/safety posture exists; mass-event prioritization and fairness need policy |

### 8.3 Non-AI insurance verdict

The generic workflow is a strong fit. The missing core issue is that policy
here means both an RFC 0029 enforcement policy and the customer's insurance
contract. Those must be distinct typed artifacts. Nirdosha must not confuse a
customer contract with the meta-policy that governs how contracts are
interpreted and enforced.

## 9. Insurance AI-first process

Models perform:

- photo-based damage estimation;
- document extraction;
- fraud/complexity triage;
- claim severity prediction;
- repair-cost estimation;
- risk/pricing assistance;
- recommendation to approve, request information, investigate, or deny.

### 9.1 Initial permitted effects

- extract and normalize evidence;
- recommend reserve or routing;
- prioritize review;
- request policy-authorized missing information;
- auto-approve narrowly bounded low-value claims with authoritative evidence,
  reversible recovery path, audit, and sampling, if law/policy permits;
- create fraud referral without declaring fraud as fact.

### 9.2 Initial prohibited effects

- conclusively label a person fraudulent from a score;
- deny/cancel/price solely from opaque inference where meaningful review,
  explanation, or legal constraints apply;
- close its own fraud referral;
- generate reasons not traceable to the real decision;
- silently use protected or proxy characteristics;
- train on claim/customer data without authorized purpose.

### 9.3 AI insurance failure matrix

| Test | Required result | Architecture result |
| --- | --- | --- |
| Clear low-value damage within validated domain | Auto-approve only under bounded policy | **Partial pass** — MAP supports it; domain effect/recovery profile needed |
| Photo manipulated or generated | Detect/abstain/escalate | **Pass conceptually** through adversarial assurance |
| Model underestimates unusual vehicle/property | OOD/uncertainty and human review | **Pass conceptually** through operating-domain contract |
| Fraud score treated as fact | Prohibit; use as inference/referral input | **Pass** under MAP's inference/fact distinction |
| Model performs worse for neighborhood/proxy cohort | Detect, suspend/narrow, impact review | **Pass conceptually** through subgroup assurance, lawful data permitting |
| Applicant/claimant cannot understand denial | Faithful reason and appeal | **Pass conceptually**, profile-specific reason mapping absent |
| Generative explanation invents exclusion | Reject output; explanation must trace to actual clause/rule | **Pass conceptually** through fidelity class; contract citation validation needed |
| Analyst sees only model summary, not source evidence | Meaningful review fails | **Pass conceptually** through human-system contract |
| Model threshold changed during catastrophe | New governed version; no silent change | **Pass conceptually** through MAP change control |
| Fraud team labels reviewed referrals only | Detect selective labels/feedback bias | **Pass conceptually** once generalized feedback semantics enter core MAP |
| Model recommends non-renewal based on external data | Require provenance, applicability, explanation, review | **Partial** — policy fits, external-data legality/contract profile needed |
| Model closes low-risk alerts it created | Prohibit self-certification unless independently governed | **Pass conceptually** through independence rules |
| Vendor compromise discovered later | Revoke, identify affected quotes/claims/payments | **Pass conceptually** through impact discovery |
| Straight-through payment issued on wrong payee | Canonical payee and payment checks still mandatory | **Pass conceptually** — model never replaces authoritative payment facts |

### 9.4 AI insurance verdict

AI can automate extraction, routing, recommendation, and carefully bounded
straight-through approval. Opaque inference cannot silently become contract
interpretation, fraud fact, adverse-action reason, or payment authority.

## 10. Insurance-specific profile gaps

The unified candidate profile that carries these requirements across both
model-free and model-influenced decisions is
[RFC 0029 Domain Profile — Insurance Claims and Policy Servicing](../../../0029-domain-profiles.md#insurance-profile).
It is a Phase 0 representation artifact, not an implementation or
certification result.

### IN1 — customer contract versus enforcement policy

The customer's policy contract, endorsements, notices, and applicable terms
are immutable/effective-dated source artifacts. RFC 0029 policy determines how
they are selected and interpreted; it does not replace their legal text.

### IN2 — coverage and applicability graph

Coverage decisions bind insured interest, policy/endorsement versions, peril,
loss time/location, jurisdiction, limits, exclusions, deductibles, prior
payments, and authoritative evidence. Ambiguity enters qualified adjudication.

### IN3 — financial state and multi-party finality

Reserve, incurred amount, approved settlement, payable, issued payment,
cleared payment, recovery, salvage, subrogation, and reinsurance are distinct
states/effects with different authorities and finality.

### IN4 — adverse action, notice, and contestability

Denial, partial denial, cancellation, non-renewal, pricing, restriction, and
fraud referral declare notice, faithful reasons, evidence access, correction,
appeal/complaint, deadlines, interim protections, and independent review.

### IN5 — proxy discrimination and lawful data use

The profile controls direct/proxy attributes, external data, cohort
evaluation, purpose, retention, explanation, and impact remediation according
to applicable policy/law. Removing an explicit protected field does not prove
absence of proxy behavior.

### IN6 — catastrophe and portfolio context

Mass events change volumes, correlations, fraud patterns, repair costs, label
delay, workflow capacity, and human-review availability. Catastrophe mode is a
versioned, bounded operating/safety posture, not permission to discard normal
controls silently.

## 11. New generic findings

The two domains identify six concepts that should be generalized in the core
rather than duplicated in profiles.

### G1 — semantic input contracts

The generic input contract needs concept, unit, source/method, timestamp,
missingness, quality, correction, and transformation lineage. Medical labs,
machine sensors, financial fields, and insurance evidence all need this.

### G2 — effect-authority and autonomy tiers

The core should standardize:

```text
Observe
Classify
Prioritize
Recommend
Draft
Approve
ExecuteReversible
ExecuteCompensatable
ExecuteIrreversible
```

Profiles may tighten or subdivide them. Capabilities never widen implicitly.

### G3 — outcome, utility, and harm contracts

Technical accuracy alone is insufficient. Every consequential model declares
benefits, harms, operational burden, affected parties, measurement windows,
uncertainty, and stopping/suspension rules.

### G4 — label and adjudication contracts

Labels declare definition, authority, delay, correction, disagreement,
censoring, selection, and feedback/intervention bias. Model-generated labels
are not independent truth.

### G5 — human-system capacity contracts

Human review declares qualification, independence, evidence access, workload,
response bounds, escalation capacity, override, and quality sampling. A safe
model can become unsafe in an overloaded workflow.

### G6 — deployment-domain validation and surveillance

Validation and status are scoped to deployment, population, environment,
input pipeline, workflow, intended use, and time. Post-deployment surveillance
includes incident intake, recall, affected-decision/effect discovery,
remediation, and revalidation.

These generalize the medical M1–M8 findings and apply equally to KYC,
manufacturing, and insurance. Domain profiles supply the concrete meanings,
authorities, thresholds, and tests.

## 12. Profile architecture after this exercise

```text
RFC 0029 — generic policy enforcement
    │
    └── RFC 0029.a — generic Model Assurance Port
            │
            ├── Generic consequential-model profile (G1–G6)
            │
            ├── KYC profile
            ├── Medical clinical-decision-support profile
            ├── Manufacturing AI profile
            └── Insurance AI profile

Non-AI domain profiles under RFC 0029:
    ├── Manufacturing safety/effect-gateway profile
    └── Insurance contract/claim/payout profile
```

The core owns identity, policy/effect mechanics, evidence, model assurance,
and generalized socio-technical contracts. Profiles own domain meaning.

## 13. Completed follow-up and remaining action

Actions 1, 2, 3, and 5 were completed by the unified rerun at the
architecture-document level. Action 6 reached a provisional conceptual
freeze. The standalone profiles in action 4, implementation, formal artifacts,
and executable conformance suites remain pending:

1. merge G1–G6 into the generic MAP contract;
2. refactor the medical M1–M8 requirements as specializations of G1–G6;
3. create concise KYC, medical, manufacturing-AI, and insurance-AI profile
   matrices rather than repeating the core protocol;
4. create non-AI manufacturing and insurance profiles under RFC 0029;
5. rerun all four existing domain exercises using the same conformance
   vocabulary;
6. freeze the core only when a new domain can be added by profile without
   changing canonical identity, decision, status, receipt, evidence, or
   lifecycle envelopes.

## 14. Freeze criterion

The architecture is generically stable when a new domain requires only:

- vocabulary and authorities;
- effect taxonomy and resource identity;
- domain policy kinds or specializations;
- required assurance promises;
- profile-specific thresholds and evidence;
- conformance and adversarial tests;

and does **not** require a new universal decision type, model lifecycle,
evidence-finality mode, identity envelope, status protocol, inference receipt,
or bypass classification.

The unified rerun meets this criterion at the conceptual-schema level: G1–G6
are incorporated into RFC 0029.a and no seventh generic contract was needed.
MF1–MF6 and IN1–IN6 remain domain-profile requirements rather than leaking
into the generic core. Formalization, implementation, and executable
conformance are still required before any enforcement guarantee is made.
