# RFC 0029 — Unified Cross-Domain Conformance Rerun

```
Evaluates:     RFC 0029 and RFC 0029.a after incorporation of G1–G6
Status:        Architecture validation; no implementation or certification
Created:       2026-09-26
Domains:       KYC, medical, manufacturing, insurance
Processes:     AI-first and non-AI where applicable
```

## 1. Question and method

This rerun asks whether the same policy and model-assurance vocabulary can
evaluate four domains without adding another universal identity, decision,
lifecycle, receipt, evidence, or status concept.

Every AI path is evaluated against the same six core contracts:

| ID | Core contract |
| --- | --- |
| C1 | Semantic input contract |
| C2 | Effect authority and autonomy |
| C3 | Outcome, utility, and harm |
| C4 | Label and adjudication |
| C5 | Human-system capacity |
| C6 | Deployment-domain validation and surveillance |

Each test receives one architecture result:

- **Core pass:** the generic semantics can express and enforce the condition;
- **Profile pass:** the generic contract is sufficient, but a domain profile
  must supply vocabulary, authority, threshold, or evidence;
- **Open implementation:** semantics are adequate but nothing implements them;
- **Rejected:** the requested guarantee or authority is not admissible;
- **Core gap:** a new universal abstraction is still missing.

“Core pass” never means the deployed system is compliant or safe. All RFC
0029/0029.a implementation and formal proof prerequisites remain open.

## 2. Shared effect-authority vocabulary

All model-influenced effects use the RFC 0029.a ordering:

```text
Observe < Classify < Prioritize < Recommend < Draft < Approve
        < ExecuteReversible < ExecuteCompensatable < ExecuteIrreversible
```

The domain profile maps concrete effects to these tiers. It may prohibit or
subdivide a tier. A model's strong performance cannot widen its authority.

## 3. KYC rerun

### 3.1 Profile mapping

| Contract | KYC specialization |
| --- | --- |
| C1 | Identity, document, biometric, screening, ownership, address, and source provenance |
| C2 | Extract/classify/recommend versus approve onboarding, restrict, reject, report, or close |
| C3 | Financial-crime risk, exclusion, privacy, discrimination, delay, and investigator burden |
| C4 | Delayed/disputed fraud and suspicious-activity outcomes; model referrals are not truth |
| C5 | Qualified investigator capacity, source access, independence, escalation, sampling |
| C6 | Jurisdiction, product, customer/document population, provider/version, and ongoing monitoring |

### 3.2 Failure rerun

| Test | New result |
| --- | --- |
| Changed document field/unit/preprocessing | **Core pass via C1**; profile defines document semantics |
| Low confidence or out-of-domain applicant | **Core pass**: abstain and route; no silent extrapolation |
| Model misses sanctions candidate | **Rejected as correctness guarantee**; independent screening, outcome limits, and impact discovery are expressible |
| Biased rejection or proxy feature | **Core pass conceptually** via subgroup, C3, C6; lawful profile evidence required |
| Model referral becomes “fraud fact” | **Core pass via C4**: inference cannot become adjudicated fact |
| Review queue overload causes rubber-stamping | **Core pass via C5**: narrow/suspend dependent automation |
| Provider/model changes behind alias | **Core pass**: immutable identity and revalidation required |
| Applicant seeks reason/correction/appeal | **Profile pass** using faithful decision lineage and domain notice rules |
| AI lowers risk/removes restriction automatically | **Profile pass**; distinct effect and tier, asymmetric authority allowed |
| Model/vendor compromise discovered later | **Core pass conceptually** via C6 and impact discovery |

### 3.3 Verdict

Risk-based AI-first KYC is **architecturally admissible** for extraction,
classification, prioritization, recommendation, and narrowly admitted
low-risk approvals. It remains an **open implementation** until the KYC
profile and conformance suite exist.

Unbounded “fully autonomous KYC correctness” remains **rejected**. Suspicious
activity, identity truth, legal sufficiency, and sanctions completeness are
not established merely by a model receipt.

## 4. Medical rerun

### 4.1 M1–M8 refactoring

| Medical finding | Generic/core or profile treatment |
| --- | --- |
| M1 intended use/regulatory status | Medical profile binding outside G1–G6 |
| M2 measurement/data quality | C1 specialization |
| M3 clinical autonomy/effect risk | C2 specialization |
| M4 clinical benefit/harm endpoints | C3 specialization |
| M5 labels/adjudication/intervention feedback | C4 specialization |
| M6 workflow capacity/human factors | C5 specialization |
| M7 local validation/transportability | C6 specialization |
| M8 post-market safety/recall | C6 plus MAP incident/impact lifecycle |

### 4.2 Failure rerun

| Test | New result |
| --- | --- |
| Blood-pressure unit or method changes | **Core pass via C1**; medical profile supplies measurement ontology |
| Corrupt/stale clinical input with high confidence | **Core pass**: input contract overrides confidence |
| Draft order mistaken for executable treatment | **Core pass via C2**; distinct effect/capability |
| Alert performs well but increases harmful treatment | **Core pass via C3**; accuracy alone cannot activate deployment |
| Sepsis label is late, disputed, or treatment-affected | **Core pass via C4**; profile supplies endpoint adjudication |
| Clinicians cannot review alerts in time | **Core pass via C5**; dependent automation degrades/suspends |
| Model transferred to a new hospital/population | **Core pass via C6** only with equivalence or local validation |
| Safety signal requires recall | **Core pass conceptually** via surveillance and impact discovery |
| Model autonomously diagnoses or orders treatment | **Rejected** without a future high-risk clinical profile and external authorization/safety case |

### 4.3 Verdict

Advisory sepsis prioritization is now **architecturally admissible**, subject to
the medical profile, intended-use/regulatory binding, local validation,
clinical safety evidence, and implementation. Autonomous diagnosis or
treatment remains **rejected**.

The previous M2–M8 core gaps are resolved as generic semantics; they remain
unimplemented and require medical specializations. M1 was correctly retained
as a medical-profile concern rather than generalized into MAP.

## 5. Manufacturing rerun

### 5.1 Non-AI safety process

| Test | New result |
| --- | --- |
| Wrong asset/configuration or incomplete energy plan | **Profile pass** via canonical identity plus MF1 baseline/completeness rules |
| Software says isolated but equipment remains energized | **Rejected as software-only guarantee**; physical attestation and hardware interlock remain authoritative |
| Sensor/unit/method changes | **Core pass via RFC 0029 provenance and C1 where a model consumes it** |
| Central policy service unavailable | **Profile pass** only with admitted local fail-operational authority and bounded synchronization |
| Start command issued while personal lock is active | **Profile pass** only through safety-rated enforcement; Nirdosha evidence cannot replace it |
| Emergency-stop deadline/jitter proof | **Profile gap MF3/MF4**, not MAP core; requires real-time safety semantics/certification |

The non-AI workflow remains **conditionally admissible**. RFC 0029 governs
authorization, workflow, capability, configuration, and evidence. It does not
claim that a database record isolates hazardous energy.

### 5.2 AI process

| Test | New result |
| --- | --- |
| Sensor meaning/configuration drift | **Core pass via C1/C6**, profile supplies machine semantics |
| Model recommends maintenance | **Core pass at Recommend tier** |
| Model places bounded quality quarantine | **Profile pass at ExecuteReversible** |
| Model releases safety-critical product | **Rejected** unless admitted profile, independent evidence, and authority permit the exact effect |
| Model defeats interlock or restarts hazardous cell | **Rejected**; model authority cannot supersede the safety authority floor |
| Defect/failure labels depend on selective inspection | **Core pass via C4**; limitation must constrain promises |
| Operators are overloaded or rubber-stamp | **Core pass via C5**; dependent authority narrows/suspends |
| Same model moves to another line/configuration | **Core pass via C6** only after equivalence/local validation |

### 5.3 Verdict

Predictive maintenance and visual inspection are **architecturally
admissible** for observation through recommendation and for explicitly
bounded reversible holds. Hazardous autonomous actuation remains **rejected**.
MF1–MF6 remain manufacturing-profile requirements; no new MAP core primitive
was needed.

## 6. Insurance rerun

### 6.1 Non-AI claim process

| Test | New result |
| --- | --- |
| Wrong policy/endorsement version at loss time | **Profile pass** through IN1/IN2 effective applicability graph |
| Coverage evidence ambiguous or contradictory | **Profile pass**: indeterminate and qualified adjudication |
| Reserve confused with approved/issued/cleared payment | **Profile pass** through distinct IN3 effects/finality |
| Denial notice differs from actual decision | **Profile pass** through decision lineage and IN4 reason mapping |
| Legal interpretation changes | **Profile pass conceptually** through authority/effective scope and impact discovery |

The non-AI claim/payout process remains **conditionally admissible**, pending
the insurance contract, applicability, financial-finality, and contestability
profile. These are RFC 0029 domain semantics, not MAP abstractions.

### 6.2 AI process

| Test | New result |
| --- | --- |
| Photo/estimate input is manipulated, stale, or semantically incompatible | **Core pass via C1 plus adversarial assurance** |
| Model estimates or routes claim | **Core pass at Classify/Recommend** |
| Narrow low-value claim is auto-approved | **Profile pass at Approve** with admitted recovery, sampling, and legal authority |
| Fraud score becomes finding of fraud | **Core pass via C4**: prohibit inference-to-fact promotion |
| Generative system invents exclusion/reason | **Core pass**: typed validation and faithful contract/rule trace required |
| Claimant cohort suffers disproportionate harm | **Core pass conceptually** via subgroup assurance and C3/C6 |
| Review capacity collapses during catastrophe | **Core pass via C5**; catastrophe posture narrows automation |
| Threshold/model changes during catastrophe | **Core pass via identity/change control and C6** |
| AI issues payment to inferred payee | **Rejected**; authoritative payee and payment controls remain mandatory |
| Opaque AI makes unattended denial/cancellation/pricing decision | **Rejected** absent a profile proving required authority, fidelity, contestability, and applicable legality |

### 6.3 Verdict

AI extraction, estimation, routing, recommendation, and bounded approval are
**architecturally admissible**. Opaque inference cannot become contract
interpretation, fraud fact, adverse-action reason, or payment authority.
IN1–IN6 remain insurance-profile requirements; no new MAP core primitive was
needed.

## 7. Comparative result

| Domain/process | Before G1–G6 | After semantic merge | Residual gate |
| --- | --- | --- | --- |
| AI-first risk-based KYC | Conditional with AI gaps | Architecturally admissible | KYC profile, formal semantics, implementation, tests |
| Fully autonomous KYC correctness | Rejected | Rejected | Claim itself exceeds honest guarantee boundary |
| Advisory medical AI | Conditional/failing profile | Architecturally admissible | Medical profile, external authorization, validation, implementation |
| Autonomous treatment | Rejected | Rejected | Future high-risk profile and safety/regulatory case |
| Non-AI manufacturing safety | Conditional | Conditional | MF1–MF6 and safety-rated physical enforcement |
| Manufacturing advisory/reversible AI | Conditional | Architecturally admissible | Manufacturing profile and implementation |
| Hazardous autonomous machine control | Rejected | Rejected | Safety authority cannot be inferred from model health |
| Non-AI insurance claims | Conditional | Conditional | IN1–IN6 profile and implementation |
| Insurance assistive/bounded AI | Conditional | Architecturally admissible | Insurance profile and implementation |
| Opaque unattended adverse insurance action | Rejected | Rejected | Authority, fidelity, legality, and contestability unmet |

## 8. In-sample result: no seventh generic contract found

The rerun found **no additional universal MAP abstraction**. Every AI failure
mapped to identity/receipt/promise mechanics already in RFC 0029.a or to
C1–C6. Remaining gaps were genuinely domain-specific:

- KYC authority, screening, customer-action, and legal-reporting rules;
- medical intended use, clinical ontology, regulatory status, and safety case;
- manufacturing physical actuation, hard real-time safety, locks, and product
  traceability;
- insurance contract interpretation, claim states, payment finality, notices,
  and catastrophe rules.

This is positive **in-sample** evidence for the profile boundary, not proof of
universality: all four domains participated in deriving the abstractions being
tested. The result must not be used as a stability claim until the
[out-of-sample and independent gates](../../../0029-phase0-readiness-and-domain-matrix.md#convergence-and-independent-validation-plan)
pass.

## 9. Freeze decision

The conceptual MAP core is a **candidate implementation baseline**, not a
provisional freeze. Its in-sample consistency is sufficient to build
falsifiable prototypes.

This is a MAP-only conceptual freeze. It does not close the original RFC 0029
banking-transfer and healthcare-access F1–F12 rerun gate and does not freeze
the universal policy IR. The distinction and the remaining profile/schema
gate are recorded in the
[Phase 0 readiness matrix](../../../0029-phase0-readiness-and-domain-matrix.md).

It is not implementation-ready for guarantees until RFC 0029.a §27's formal
artifacts exist. No domain profile is certified or shipped. The freeze should
be reopened if a future domain requires a new universal envelope rather than
a vocabulary, authority, threshold, evidence rule, or conformance test.

## 10. Required next work

1. Specify the formal algebras and schemas listed in RFC 0029.a §27.
2. Write the four domain profiles and their executable conformance matrices.
3. Keep non-AI manufacturing and insurance requirements under RFC 0029.
4. Implement only after profile-independent schemas prove able to encode all
   four matrices without domain fields leaking into the core.
5. Repeat the rerun against implementation evidence; this document alone
   changes no shipped capability.
