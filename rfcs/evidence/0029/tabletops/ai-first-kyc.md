# RFC 0029 — AI-First KYC Automation Tabletop

```
Companion to:  RFC 0029 — Domain-neutral policy admission, enforcement,
               and evidence
Status:        Failure-oriented architecture exercise; no implementation
Created:       2026-09-26
Scenario:      AI-first customer onboarding and continuous KYC
Follow-up:     Unified C1–C6 conformance rerun
Purpose:       Evaluate full automation and risk-based human escalation
```

> **Amendment status (2026-09-26):** RFC 0029.a now specifies the proposed
> Model Assurance Port that gives A1–A8 a normative interface and lifecycle.
> This is design accommodation, not implementation or validation. Fully
> autonomous KYC remains rejected. The architecture-level rerun is complete
> in the [unified conformance report](./unified-conformance-rerun.md),
> but implementation and executable profile gates remain open.

## 1. Executive verdict

RFC 0029 can govern an **AI-assisted or AI-first KYC workflow**, but the
present architecture cannot honestly certify a general-purpose AI model as a
fully autonomous KYC authority.

Two operating models have different admission outcomes:

| Model | Present RFC 0029 outcome |
| --- | --- |
| AI gathers evidence, verifies bounded signals, scores risk, and escalates uncertainty to qualified humans | **Conditionally admissible** as a `DurableWorkflow` with externally attested facts, opaque/model inference outputs, explicit abstention, evidence, and continuous monitoring |
| AI alone decides every applicant, resolves document/biometric/sanctions/beneficial-owner ambiguity, files or suppresses cases, and restricts accounts with no meaningful human review | **Rejected at the requested guarantee** because model correctness, calibration, fairness, adversarial robustness, explanation, appeal, and legal applicability lack admitted semantics and proof obligations |

This does not mean a bank is technically unable to automate decisions. It
means Nirdosha must not certify “KYC correctness” merely because every model
call passed through a gateway and produced evidence.

The safe architecture is:

```text
Authoritative source evidence
        │
        ▼
Bounded deterministic checks ───────────────┐
        │                                   │
        ▼                                   ▼
AI inference with uncertainty        Policy applicability
        │                                   │
        ├── high confidence + eligible ─────┤
        ├── uncertain/conflicting ──────────► qualified review workflow
        └── suspected fraud/adversarial ────► specialist investigation
                                            │
                                            ▼
                                 Typed policy decision
                                            │
                                            ▼
                             onboarding/restriction gateway
                                            │
                                            ▼
                                evidence + appeal + monitoring
```

The model supplies typed observations and recommendations. The admitted
policy defines the decision boundary, abstention, escalation, and effects.

## 2. Regulatory and risk baseline

This tabletop uses international KYC/CDD practice as a representative
baseline, particularly:

- [FATF Recommendations](https://www.fatf-gafi.org/en/publications/Fatfrecommendations/Fatf-recommendations.html)
  for customer due diligence, beneficial ownership, record keeping, ongoing
  monitoring, and risk-based controls;
- [FATF Digital Identity Guidance](https://www.fatf-gafi.org/en/publications/Financialinclusionandnpoissues/Digital-identity-guidance.html)
  for assessing whether a digital identity system is sufficiently reliable
  and independent for its use;
- [EBA Guidelines on remote customer onboarding](https://www.eba.europa.eu/activities/single-rulebook/regulatory-activities/anti-money-laundering-and-countering-financing-1)
  as a representative remote-onboarding control source;
- applicable local KYC, privacy, biometric, anti-discrimination, consumer
  protection, record-retention, sanctions, and automated-decision rules.

This is an architecture exercise, not a legal conclusion that unattended AI
onboarding is permitted in a particular jurisdiction or product.

## 3. Concrete system under test

A retail bank wants straight-through digital onboarding for individuals and
small companies.

The proposed AI system performs:

1. document classification and field extraction;
2. document authenticity/tamper detection;
3. selfie/document face comparison and liveness/deepfake detection;
4. address and contact normalization;
5. identity resolution against authoritative sources;
6. sanctions, PEP, adverse-media, and internal-watchlist matching;
7. beneficial-owner and control-structure extraction for companies;
8. occupation/source-of-funds and expected-activity interpretation;
9. customer risk scoring;
10. approve, reject, request-more-information, restrict, or escalate;
11. ongoing KYC refresh and event-driven reclassification.

Effects include:

- create customer identity;
- create/open/restrict/close an account;
- assign customer-risk tier;
- accept or reject evidence;
- create or disposition a screening match;
- create an investigation/case;
- request additional personal data;
- disclose an explanation;
- schedule refresh or enhanced due diligence;
- notify or file with an authority where independently required.

Each is a distinct effect. “KYC decision” is not one undifferentiated action.

## 4. Policy and authority separation

The bank, not the model, owns policy:

- required identity attributes and evidence;
- permitted digital identity schemes;
- document and biometric thresholds;
- risk categories and actions;
- sanctions/PEP match handling;
- beneficial-owner requirements;
- conditions for straight-through approval;
- mandatory human-review cases;
- account restrictions;
- refresh frequency and triggering events;
- appeal and correction process;
- jurisdiction/product applicability.

The AI model produces **inferences**, not authoritative facts merely by being
confident. Every output declares:

- model artifact and runtime identity;
- task and output schema;
- input/evidence references;
- confidence or calibrated uncertainty where meaningful;
- operating-domain version;
- abstention and out-of-distribution state;
- explanation/evidence class;
- known limitations;
- observation time and validity;
- adversarial/tamper indicators.

An inference becomes usable only through an admitted policy that states how
that inference combines with authoritative evidence.

## 5. Required policy decomposition

### 5.1 Deterministic and authoritative checks

Examples:

- identifier checksum and format;
- document expiry date;
- issuer signature/QR verification;
- authoritative registry response;
- required field presence;
- age/product threshold;
- exact sanctions identifier match;
- corporate-registry ownership data with provenance.

These may support ordinary `Guarded` decisions when their authority and
freshness contracts hold.

### 5.2 Statistical/model inferences

Examples:

- face similarity;
- liveness/deepfake likelihood;
- document-tamper likelihood;
- fuzzy name match;
- adverse-media relevance;
- occupation/source-of-funds extraction;
- shell-company or fraud risk;
- out-of-distribution detection.

These require an inference assurance contract. They cannot be converted into
facts by thresholding without preserving model identity, calibration scope,
uncertainty, and error behavior.

### 5.3 Human and legal judgments

Examples:

- whether ambiguous ownership evidence is satisfactory;
- whether a potential match is the same person;
- whether an exception is legally permitted;
- whether adverse media is credible and relevant;
- whether enhanced due diligence resolves the identified risk;
- whether a relationship should be exited.

Nirdosha can require a qualified, authenticated, independent decision and its
reason. It cannot prove the professional judgment is substantively correct.

## 6. Present-architecture mapping

| RFC 0029 construct | KYC use |
| --- | --- |
| Domain vocabulary | Customer, document, biometric sample, company, beneficial owner, screening match, case, account |
| Resource identity (§36.1) | Resolve aliases/transliterations and prevent duplicate/synthetic customer identities |
| Effect closure (§36.2) | Cover onboarding APIs, analyst tools, batch imports, vendor callbacks, account-opening jobs, and admin overrides |
| Decision capability (§36.3) | Bind approval to applicant, evidence versions, model versions, product, jurisdiction, and account-opening gateway |
| Evidence finality (§36.4) | Commit onboarding decision/commitment with customer/account creation |
| Durable workflow | Request information, enhanced due diligence, analyst review, approval, restriction, appeal |
| Safety posture (§36.6) | Define behavior when identity/screening/model/PDP providers fail |
| Authority conflict (§36.7) | Handle conflicting registries, document data, watchlists, and identity providers |
| Transition revalidation (§36.8) | Re-screen or refresh evidence before approval, activation, high-risk actions, and periodic review |
| Aggregate consistency (§36.9) | Velocity, repeated-device, address, identity, and linked-application patterns |
| Monitor health (§36.10) | Model drift, vendor availability, telemetry loss, screening freshness, and bypass monitoring |
| Confidentiality (§36.11) | Protect biometrics, identity documents, watchlist results, model inputs, evidence, and appeal records |
| Applicability (§36.12) | Resolve jurisdiction, customer/product type, legal entity, channel, and residency requirements |

This mapping demonstrates that the twelve hardening additions were necessary.
It does not supply the missing AI-specific semantics below.

## 7. Adversarial test matrix

| Test | Required response | Result under present architecture |
| --- | --- | --- |
| Valid applicant, high-quality authoritative evidence | Straight-through approval if policy permits | **Partial pass** — workflow and evidence fit; model assurance remains unspecified |
| Blurry document or low-quality selfie | Abstain/request new evidence | **Fail** — no normative calibrated abstention contract |
| Applicant outside model training domain | Out-of-distribution abstention | **Fail** — AI §30 mentions provenance/nondeterminism but has no OOD semantics |
| Deepfake passes liveness model | Layered detection, challenge/escalation, monitor and incident response | **Fail** — no adversarial model assurance or residual-risk semantics |
| Prompt injection in uploaded document/adverse-media content | Treat content as data, isolate tools, prevent instruction execution | **Partial** — §30 names prompt injection, but no KYC pipeline conformance contract exists |
| Transliteration causes sanctions false positive | Preserve aliases/evidence, calibrated match, human disposition | **Partial** — authority conflict and resource aliases fit; model calibration/review quality missing |
| Model misses a sanctions match | Independent controls/re-screening and impact discovery | **Fail** for “KYC correct” — gateway enforcement cannot prove model recall |
| Corporate ownership graph is cyclic/ambiguous | Detect incompleteness and escalate | **Partial** — decidable bounded graph and workflow fit; completeness authority absent |
| Two vendors disagree on identity | Apply admitted disagreement/quorum/uncertainty policy | **Pass conceptually** under §36.7 once formalized |
| Applicant changes evidence after AI approval | Invalidate bound capability and re-evaluate | **Pass conceptually** under §§36.3/36.8 |
| Reuse approval for different product/account | Reject capability binding mismatch | **Pass conceptually** under §36.3 |
| Vendor silently updates its model | Reject unpinned artifact or treat as new authority/model version | **Partial** — bundle binding helps; model registry/validation lifecycle missing |
| Performance degrades for a demographic subgroup | Detect, suspend affected automation, escalate/remediate | **Fail** — no fairness/subgroup assurance semantics |
| Analyst rubber-stamps every escalation | Detect review-quality and independence failure | **Fail** — authenticated human action is not meaningful review by itself |
| Applicant asks why they were rejected | Provide safe, accurate reason and appeal route | **Fail/partial** — explanation profile exists; model-to-decision reason fidelity and appeal semantics do not |
| Incorrect record is corrected on appeal | Correct authoritative data, re-evaluate dependent decisions, preserve history | **Partial** — dependency graph fits; correction/appeal lifecycle missing |
| Screening or model service unavailable | Fail according to product/jurisdiction safety posture | **Pass conceptually**, subject to formal §36.6 semantics |
| KYC model telemetry fails | Suspend continuous assurance and automation as declared | **Pass conceptually** under §36.10 |
| Training/evaluation data was unlawfully sourced | Reject artifact/use under data governance | **Fail** — model-data lineage and lawful-basis admission are not normative |
| Generative model returns different decisions on replay | Deny direct adjudication or use constrained deterministic envelope | **Fail** — nondeterminism is named but no replay/equivalence contract exists |
| Model/provider is compromised | Revoke artifact/authority, find affected decisions, restrict/re-review | **Partial** — §36.7 supports authority compromise; model blast-radius semantics missing |
| Continuous KYC lowers risk and removes restrictions automatically | Require policy-authorized transition and evidence | **Partial** — workflow fits; asymmetry between raising and lowering risk is unspecified |
| AI suppresses or closes its own alert | Separation of duties/model independence | **Fail** — policy must prohibit self-certification; no model independence rule yet |
| KYC evidence is copied into prompts/logs | Minimise and govern policy/model plane | **Pass conceptually** under §36.11, implementation absent |

## 8. New AI-specific gaps

The twelve cross-domain blockers are necessary but not sufficient. AI-first
KYC exposes eight additional requirements.

### A1 — model artifact and runtime identity

Every inference binds to immutable model weights/artifact, code, configuration,
prompt/template, tokenizer/preprocessor, dependency set, hardware/runtime where
material, provider identity, and deployment attestation. An unversioned hosted
model endpoint cannot support a replayable enforcement claim.

### A2 — uncertainty, calibration, abstention, and operating domain

Each model task declares its intended population/domain, calibrated output
meaning, validation intervals, quality floors, abstention region,
out-of-distribution behavior, and escalation. Confidence is not accepted as
calibrated probability without evidence. Missing/low-quality inputs do not
default to approval or rejection.

### A3 — fairness and subgroup assurance

Where applicable, the policy declares protected or relevant evaluation
cohorts, performance/error metrics, minimum sample/evidence requirements,
threshold-change governance, unacceptable disparities, and response. A global
average cannot establish acceptable subgroup behavior. Collection/use of
sensitive attributes for evaluation must itself be legally and policy
authorized.

### A4 — adversarial and fraud resilience

The model contract covers presentation attacks, deepfakes, document tampering,
prompt/data injection, poisoning, evasion, replay, synthetic identities,
vendor compromise, and coordinated attacks. It declares layered controls,
red-team cases, residual risk, detection, and safe fallback. No single model
score is treated as proof of authenticity.

### A5 — explanation, adverse decision, correction, and appeal

Every decision class declares audience-appropriate reason codes, whether model
explanation is faithful to the actual decision path, notice requirements,
appeal/correction workflow, review authority, deadlines, evidence access, and
dependent-decision re-evaluation. Generated prose is not accepted as faithful
explanation merely because it sounds plausible.

### A6 — meaningful human review and automation bias

Human escalation declares reviewer qualification, independence, workload and
time envelope, evidence visibility, required affirmative reasoning, override
authority, conflict checks, and quality sampling. Clicking “approve” does not
turn an automated decision into meaningful human judgment. The model that
raised a case cannot be the sole authority that closes it where policy
requires independence.

### A7 — model/data lifecycle and lawful provenance

Training, validation, calibration, and monitoring data declare provenance,
lawful purpose, consent/authority where applicable, representativeness,
label-quality governance, retention, deletion, leakage controls, and vendor
rights. Artifact promotion, threshold change, rollback, retirement, and
emergency revocation follow meta-policy and separation of duty.

### A8 — drift, feedback loops, and change control

Continuous assurance covers input drift, output drift, subgroup degradation,
fraud adaptation, calibration decay, label delay, feedback contamination, and
policy/model coupling. Monitor health follows §36.10. A threshold, prompt,
feature, provider, or model change creates a new admitted version. Automation
suspends or narrows when required evidence falls below its floor.

## 9. Required AI inference assurance contract

Before an AI output may influence a protected decision, its domain module
must provide an admitted contract containing:

- task and prohibited uses;
- model/artifact/runtime identity;
- input and output schemas;
- authoritative versus inferred-field classification;
- operating domain and excluded populations/conditions;
- uncertainty, calibration, abstention, and OOD semantics;
- validation and subgroup evidence;
- adversarial threat model and fallback;
- explanation class and known fidelity limits;
- data provenance and governance;
- drift/health monitors and suspension thresholds;
- change, rollback, compromise, and affected-decision procedures;
- cost, latency, availability, and provider failure behavior;
- human-escalation and independence requirements;
- decision/effect classes the output may influence.

Opaque or externally hosted models may be used, but their achievable claim is
reduced to the evidence their contract can support. Opacity never upgrades a
model output into an authoritative fact.

## 10. Admission decision for the proposed bank

### 10.1 Fully autonomous AI KYC

Requested claim:

> Every applicant is correctly identified, screened, risk-classified, and
> accepted or rejected by AI without human review.

**Decision: Rejected.** Reasons:

- `UnsupportedSemantics`: “correctly” combines identity truth, model quality,
  legal applicability, and professional judgment;
- `InsufficientModelAssurance`: A1–A8 are not part of the present formal core;
- `AuthorityConflict`: ambiguous identity/watchlist/ownership facts need
  admitted disagreement behavior;
- `ExplanationAndAppealIncomplete`;
- `AdversarialResidualUnbounded`;
- `FairnessEvidenceMissing`;
- `HumanJudgmentRequired` for policy-defined ambiguous/high-risk cases;
- `UnachievableGuarantee`: model error cannot be compiled away by gateway
  coverage.

### 10.2 AI-first with risk-based human escalation

Requested claim:

> Eligible applicants whose authoritative evidence and bounded model checks
> satisfy an admitted straight-through policy may proceed automatically;
> uncertainty, conflict, high-risk cases, unsupported populations, and model
> health failures abstain and enter qualified review.

**Decision: Conditionally admissible after A1–A8 are added and formalized.**
The resulting guarantee is not “KYC is always correct.” It is:

> Every onboarding/account effect in certified scope was produced through the
> admitted KYC policy using the recorded evidence and model versions; required
> checks and obligations completed; uncertainty and declared exceptions were
> escalated; model assurance held within its stated operating domain and
> monitoring interval; exclusions and residual model error remain explicit.

## 11. Recommended automation boundary

Straight-through automation is a policy choice per decision class, not a
single bank-wide switch.

Good initial candidates:

- document type/extraction with authoritative verification;
- deterministic completeness and expiry checks;
- exact identifier/watchlist checks;
- duplicate application detection with human resolution of ambiguity;
- low-risk refresh where facts are authoritative and unchanged;
- routing and evidence assembly;
- requesting missing information;
- continuous monitoring that creates, but does not self-close, material
  alerts.

Mandatory escalation candidates until stronger evidence exists:

- uncertain biometric/document authenticity;
- sanctions/PEP/adverse-media possible matches;
- conflicting identity sources;
- complex or opaque beneficial ownership;
- out-of-distribution applicants/documents;
- high-risk products, geographies, or activity;
- exceptions and overrides;
- account exit, restriction, or adverse decision where policy/law requires
  meaningful review;
- model-health, telemetry, authority, or provenance failure.

## 12. Required RFC amendment and rerun gates

The unified candidate domain profile that carries these requirements across
deterministic, authoritative, human and model-influenced decisions is
[RFC 0029 Domain Profile — Customer Onboarding and KYC](../../../0029-domain-profiles.md#kyc-profile).
It is a Phase 0 representation artifact, not an implementation or
certification result.

RFC 0029.a now gives A1–A8 a normative Model Assurance Port covering model
identity, status, receipts, promises, evidence independence, effective state,
continuous monitoring, human review, explanation/appeal, impact discovery,
and conformance profiles. The remaining work is implementation, formalization,
and rerunning these gates—not merely retaining the interface design.

After amendment, rerun this tabletop and require:

- immutable model/runtime/version binding;
- reproducible or explicitly equivalent inference behavior;
- calibrated abstention and OOD tests;
- subgroup evaluation and authorized fairness monitoring;
- deepfake, tamper, injection, replay, poisoning, and evasion tests;
- faithful decision reasons and end-to-end appeal/correction tests;
- meaningful-review tests that detect rubber-stamping and self-certification;
- training/evaluation data provenance and deletion tests;
- drift, monitor failure, rollback, compromise, and affected-decision tests;
- a proof that account/customer effects cannot use an unadmitted model output;
- a proof that lowering risk or closing an alert is at least as governed as
  raising risk or creating one;
- an evidence-outage and provider-outage run under each admitted safety
  posture.

## 13. Final assessment

An AI-first bank is compatible with RFC 0029's direction if “AI-first” means
automation by default inside a bounded, evidenced operating domain with real
abstention and qualified escalation. It is not compatible if it means that an
opaque model becomes the policy, the fact authority, the adjudicator, the
explainer, and the reviewer of its own mistakes.

The present architecture should reject fully autonomous KYC certification.
That rejection is a success of the guarantee boundary, not a product failure.
