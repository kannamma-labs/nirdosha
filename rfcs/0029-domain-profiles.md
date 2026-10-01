# RFC 0029 — Consolidated Domain Profiles

## Banking Transfer Profile

```
Parent:        RFC 0029
Status:        Candidate Phase 0 profile; not implemented or certified
Created:       2026-09-27
Source:        RFC 0029 banking/healthcare tabletop, scenario A
```

## 1. Boundary

The protected process accepts a customer transfer request, makes a local
funds reservation and balanced ledger mutation, submits an instruction to an
external payment rail, and reconciles acknowledgement and settlement.

The deployment is Tier 2. The certified candidate scope includes the API PEP,
account/ledger gateway, transfer workflow, payment-rail gateway, settlement
ingestion and evidence commitment. Privileged database, migration, file and
administrative paths are in the effect closure but cannot earn a no-bypass
claim merely because they are observed.

## 2. Resources

| Resource | Canonical authority | Identity/equivalence rules |
| --- | --- | --- |
| Customer | Customer master | External/customer aliases resolve to one canonical ID; ambiguity is `Indeterminate` |
| Account | Core ledger | Product/UI aliases never replace the ledger account ID |
| Beneficiary | Beneficiary registry | Bank account and registered-beneficiary identity are distinct and versioned |
| Transfer | Transfer orchestrator | Stable transfer ID survives every workflow state and retry |
| FX quote | FX authority | Quote ID binds rate, currency pair, amount basis, issuer and expiry |
| Screening receipt | Screening authority | Receipt binds subjects, lists/provider version, observation time and expiry |

Account merge/split is a new versioned identity event that invalidates active
capabilities. Derived statements, projections and settlement records inherit
the transfer/account restrictions explicitly.

## 3. Effects

The closed taxonomy includes `transfer.request`, `funds.reserve`,
`ledger.post`, `transfer.submit`, `transfer.acknowledge`, `transfer.settle`,
`transfer.reverse`, `balance.adjust` and `customer.notify`.

Equivalent or indirect paths include database mutations, messages, settlement
files, reconciliation adjustments, migrations/backfills and administrator
commands. An unclassified mutation cannot enter certified scope.

## 4. Vocabulary and predicates

Money is an integer minor-unit amount paired with an ISO currency and an
explicit scale; binary floating point is inadmissible. Bounded predicates
cover ownership/delegation, positive amount, beneficiary status, product
floor, per-transfer limit, aggregate daily limit, screening status, risk
class, approval threshold and FX validity. Domain-specific screening/legal
predicates are `ProfileSpecialization`, not new core operators.

## 5. Facts and authorities

| Fact | Authority | Freshness/failure |
| --- | --- | --- |
| Authentication and step-up | Identity provider | Bound to session/device; missing or revoked is deny |
| Ownership/delegation | Account authority | Current account version required |
| Balance/reservations | Ledger | Read and conditionally updated inside the local atomic boundary |
| Daily usage | Limit authority | Same declared consistency domain as reservation or non-zero overshoot must be disclosed |
| Beneficiary status | Beneficiary registry | Revalidated before reservation/submission as declared |
| Screening | Named screening authorities | Freshness, disagreement and outage produce the declared deny/defer result |
| Risk/approval | Risk and approval authorities | Versioned; human approval is a distinct signed fact |
| FX quote | FX provider | Immutable quote ID; expiry invalidates the decision |
| Settlement | Payment rail/correspondent | External receipt advances only the states it authoritatively proves |

Multiple screening sources use declared precedence, quorum or corroboration;
incidental response order is forbidden. Compromise triggers affected-transfer
discovery, containment and re-evaluation/notification.

## 6. Applicability

Applicability depends on customer and institution establishment, source and
destination account jurisdiction, currency, product, payment corridor,
beneficiary-bank location, event time and effective policy versions. Each is
an authoritative typed fact. Conflict or missing jurisdiction is
`Indeterminate` or explicit manual escalation, never convenient selection.

## 7. Policy kinds and composition

The profile uses authorization, numeric invariant, aggregate constraint,
separation of duty, state transition and durable sequence. Applicable
mandatory denies dominate permits; compatible obligations accumulate;
incomparable authority conflicts are `Indeterminate`. A higher policy is a
floor unless a signed delegation names the override dimension and interval.

## 8. Decisions and enforcement axes

| Decision/effect class | Axis A | Axis B | Axis C |
| --- | --- | --- | --- |
| Request authorization | Runtime | Guarded | Local plus externally attested screening/FX where used |
| Funds reservation and ledger posting | Runtime | Atomic | Local |
| External submission | Runtime | DurableWorkflow | ExternallyAttested |
| Settlement transition | Runtime | DurableWorkflow | ExternallyAttested |
| Reconciliation/bypass response | Continuous | None | Local/ExternallyAttested per observation |

The complete transfer is never described as one atomic effect.

## 9. Decision capability

The capability binds decision/request IDs, subject and authority chain,
canonical customer/account/beneficiary/transfer identities and versions,
amount/currency/purpose digest, effect and gateway, bundle/rules, completed
blocking obligations, issue/expiry, invalidation set, use count and
idempotency key. Reservation consumes it once and emits a receipt. An
identical retry returns the prior result; the same key with changed input is
rejected.

## 10. Obligations

Blocking obligations may include step-up authentication, human approval,
fresh screening/FX verification and atomic reservation. Post-commit
obligations include submission, notification, reconciliation, investigation
and regulatory/reporting actions. Every post-commit obligation has a durable
executor, idempotency key, retry/deadline, terminal failure and escalation.

## 11. Finality and reversibility

The workflow distinguishes `Requested`, `Authorized`, `Reserved`,
`LocallyCommitted`, `Submitted`, `Accepted`, `Settled`, `Final`, `Rejected`,
`Reversed`, `Compensated` and `UnknownOutcome`. Each exposed state names its
authority. Timeout after possible external acceptance enters
`UnknownOutcome`; it never retries as a new transfer. Reversal/compensation is
a later effect and does not make external settlement historically atomic.

## 12. Safety posture

New reservations and submissions default to `FailClosed` when authoritative
policy, screening, identity or capability validation is unavailable. A
bounded operational mode requires a separate admitted meta-policy with exact
corridor, amount, duration, local snapshot/facts, journal, approval,
reconciliation and automatic termination. Availability pressure alone cannot
activate it.

## 13. Evidence

Reservation and ledger posting require `AtomicEvidence` or
`DurableOutboxEvidence` in the same transaction. External submission and
settlement retain authority receipts and idempotency/result binding. Local
evidence failure blocks the atomic commit; downstream export failure retries
from the durable outbox. Replay stores minimum typed facts/receipts and never
stores reusable credentials or prohibited authentication data.

## 14. Continuous monitoring

Continuous reconciliation declares ledger, workflow, rail and settlement
observation sources; heartbeat, checkpoint, skew/loss/backpressure bounds;
and detection/response intervals. Monitor-health uncertainty suspends the
bounded-impact claim. Monitoring never proves that a privileged bypass did
not occur.

## 15. Confidentiality

PDP inputs and evidence are purpose-limited financial data. The profile names
field disclosure, encryption, locality, access roles, query audit, retention,
legal hold, deletion/tombstone and export approval. Raw credentials, private
keys and unrelated customer data are prohibited.

## 16. Budget

The concrete deployment must declare decision latency, external-provider
timeouts, maximum authorities/rules/obligations, aggregate window size,
monitor interval and evidence estimate. No numeric production budget is
implied by this profile; missing or exceeded budgets reject admission.

## 17. Conformance cases

The mandatory cases are the banking matrix in the
[Phase 0 readiness document](./0029-phase0-readiness-and-domain-matrix.md#71-banking-transfer),
plus permit/deny boundaries, stale/revoked facts, authority disagreement,
concurrency, partition, duplicated/reordered receipts, policy change,
notification failure and privileged bypass. Every case must have canonical
inputs, expected admission result, decision/effect trace and evidence result.

## 18. Exclusions and unsupported guarantees

- No claim that screening establishes universal absence of sanctioned risk.
- No global atomicity across local ledger and external payment rails.
- No Tier-1 no-bypass claim.
- No zero-overshoot aggregate claim without one enforcing consistency domain.
- No settlement finality beyond the named external authority and state.
- No prevention claim from reconciliation or incident response.
- A transfer whose resource, effect, applicability or authority is ambiguous
  is unsupported for an enforcement guarantee.

## 19. Current representation verdict

The candidate mapping is `CoreExact` for universal identity, effect,
capability, workflow, evidence and safety concepts, with
`ProfileSpecialization` for money, corridors, screening, settlement and
aggregate rules. It remains unproved until encoded in the candidate schema
and the executable conformance matrix passes. Any schema field named only for
banking blocks universal-core freeze and must be moved to the profile or
justified as a cross-domain primitive.
## Healthcare Access and Clinical Decision Support Profile

```
Parent:        RFC 0029 and RFC 0029.a model-influence overlay
Status:        Candidate Phase 0 profile; not implemented or certified
Created:       2026-09-27
Source:        RFC 0029 banking/healthcare tabletop, scenario B
```

## 1. Boundary

The protected process authorizes normal or emergency record access; governs
medication order, pharmacy verification, dispensing and administration; and
supports model-influenced clinical prioritization/advisory alerts. It includes
identity/workforce, EHR, consent/restriction, external exchange, terminology/
allergy, laboratory/device feeds, clinical decision support, pharmacy,
administration, audit and break-glass-review systems.

The deployment is Tier 2. Nirdosha may enforce authenticated decisions and
evidence; it does not certify clinical correctness or replace licensed human
judgment. Research, analytics, exports, indexes and support paths are covered
effects or explicit exclusions, never silently inherited coverage.

## 2. Resources

| Resource | Canonical authority | Identity/equivalence rules |
| --- | --- | --- |
| Patient | Enterprise master patient index | Aliases, merges and splits are versioned; unresolved collision is `Indeterminate` |
| Record/document | EHR authority | Replicas, projections and indexes explicitly inherit purpose/field restrictions |
| Encounter | Encounter authority | Binds patient, location, care context and interval |
| Medication order | Ordering system | Stable order ID across verification, dispense and administration |
| Medication/product | Medication terminology/pharmacy authority | Code-system/version and physical product/lot remain distinct identities |
| Break-glass grant | Emergency-access authority | Binds patient, purpose, fields, action, subject and expiry |
| Clinical observation/specimen/device | Clinical source authority | Concept, code, unit, method, time, correction and source identity are bound |
| Advisory alert | Clinical workflow authority | Stable alert ID binds patient, encounter, intended use, evidence and review state |
| Model deployment | MAP registry | Model/runtime/adapter/configuration/feature pipeline/site/intended-use identity is immutable |

Identity changes invalidate dependent capabilities and transitions. A search
projection or research copy is a derived resource, not an ungoverned copy.

## 3. Effects

The closed taxonomy includes `record.view`, `record.disclose`,
`record.export`, `emergency.activate`, `order.create`, `order.override`,
`pharmacy.verify`, `medication.dispense`, `medication.administer`,
`clinical.prioritize`, `clinical.alert`, `order.draft`, `alert.review`,
`alert.close`, `research.derive` and `evidence.view`.

Equivalent/indirect effects include document-store reads, search/index
results, analytics extracts, messages, print/file export, administrative
queries and replica access. `administer` is irreversible; an audit or later
correction is not compensation for the physical effect.

## 4. Vocabulary and predicates

The profile supplies clinical code systems, units and measurement methods;
workforce role/license/scope; treatment relationship; care location;
purpose; consent/restriction category; minimum-necessary field sets;
medication/allergy/interaction/dose concepts; clinical concepts/code systems,
units, reference ranges, specimens/devices/methods, missingness and correction
states; intended use/population/site; emergency scenarios; and order/alert
workflow states. Domain correctness of a diagnosis, dose, treatment or
clinical judgment is not inferred by the universal core.

## 5. Facts and authorities

| Fact | Authority | Freshness/failure |
| --- | --- | --- |
| Workforce identity/role/license | Workforce and licensing authorities | Revocation invalidates dependent decisions |
| Treatment relationship/location | Encounter authority | Current encounter/context version required |
| Consent/restriction | Consent registry and applicable authority | Failure follows the admitted normal/emergency posture |
| Allergy/current medications | Local EHR plus named external exchange | Profile defines precedence/corroboration/uncertainty |
| Patient measurements | Named clinical source/device | Unit, method, time, correction and provenance required |
| Drug/interaction knowledge | Versioned terminology/knowledge authority | Exact version binds warning evidence |
| Pharmacy verification | Authenticated pharmacist/workflow authority | Current before dispense as policy requires |
| Administration | Medication-administration authority | Patient/product/order match at point of effect |
| Clinical input/measurement | Named EHR/lab/device authority | Critical-input completeness and semantic compatibility override model confidence |
| Model inference | Admitted model deployment | Inference receipt, uncertainty, OOD and intended-use scope required; never diagnosis by itself |
| Intended/regulatory use | Institutional/regulatory authority | Effective-dated external fact with suspension/recall behavior |
| Clinical endpoint/label | Named adjudication authority | Definition, delay, disagreement, censoring and intervention feedback retained |

Disagreement never resolves by response order. The profile returns the
declared uncertainty/escalation result. Compromise or correction supports
affected-patient/order/administration discovery and response.

## 6. Applicability

Applicability uses institution/facility, patient/data location, treatment
versus research purpose, care/emergency context, sensitive category,
clinician establishment, event time and effective legal/organizational
policy. Model influence additionally binds site, care setting, population,
intended use, task, data/feature version and effect tier. Conflict among
retention, deletion, consent and legal-hold authority is explicitly resolved
or escalated; ambiguity is not silent selection.

## 7. Policy kinds and composition

The profile uses authorization, data projection/transformation, state
transition, separation of duty, sequence, emergency policy and temporal
obligation. Mandatory restrictions and compatible obligations accumulate.
Emergency authority is a meta-policy-bounded alternative policy, not a bypass
or a rewrite of higher-order restrictions.

## 8. Decisions and enforcement axes

| Decision/effect class | Axis A | Axis B | Axis C |
| --- | --- | --- | --- |
| Normal record view | Runtime | Guarded | Local or mixed per fact |
| Emergency record view | Runtime | Guarded | Local with explicit outage posture |
| Medication order/verification transition | Runtime | DurableWorkflow | Mixed local/external facts |
| Dispense/administration transition | Runtime | DurableWorkflow | Mixed local/external facts |
| Clinical worklist prioritization | Runtime | Advisory | Mixed facts plus model receipt where used |
| Advisory clinical alert | Runtime | Guarded | Mixed facts plus model receipt |
| Non-executable draft order | Runtime | Guarded | Mixed facts plus model receipt; `Draft` influence only |
| Executable order/treatment | Runtime | DurableWorkflow | Qualified clinician authority; initial model execution authority unsupported |
| Break-glass review/monitoring | Continuous/retrospective | None | Local |

Initial model bindings may be `Prioritize`, `Recommend`, or `Draft`. A model
may not diagnose, sign an executable order, administer treatment, close its
own alert, or widen intended use. No workflow label turns medication
administration into an atomic reversible effect.

## 9. Decision capability

Record capabilities bind subject/authority chain, canonical patient and
record versions, purpose, encounter/location, allowed fields, effect/gateway,
reason, bundle/rules, issue/expiry and invalidations. Emergency capabilities
also bind the admitted emergency scenario and are short-lived and bounded-use.
Medication capabilities bind patient, order, product, dose facts, transition
and current versions. Changed patient, purpose, input or transition rejects
reuse; consumption produces an effect receipt. Model-influenced capabilities
also bind encounter, intended use, input commitments, feature-pipeline and
model/deployment identities, inference receipt, influence level and effective
assurance state. An advisory capability cannot authorize an executable order.

## 10. Obligations

Blocking obligations include authentication, relationship/license checks,
field projection, current allergy/interaction evaluation, required human
acknowledgement, pharmacy verification and point-of-administration identity
checks. Post-commit obligations include disclosure accounting, privacy/safety
notification, break-glass review and incident follow-up, all with durable
retry/deadline/escalation/terminal handling.

## 11. Finality and reversibility

The medication workflow distinguishes `Draft`, `Ordered`, `Verified`,
`DispenseReserved`, `Dispensed`, `AdministrationPrepared`, `Administered`,
`Cancelled`, `UnknownOutcome` and correction/incident states. Each state
names its authority and visibility. Record disclosure and administration may
be irreversible. A correction, antidote or later clinical response is a new
effect and never rewrites history.

Clinical-support state distinguishes `Eligible`, `Scored`, `AlertCommitted`,
`Displayed`, `Reviewed`, `Acknowledged`, `Escalated`, `Closed`, `Suspended`
and `Recalled`. Alert display/review is not diagnosis, order signature,
dispense or administration finality.

## 12. Safety posture

Normal access defaults to `FailClosed`. Emergency care may use only a
meta-policy-authorized `HumanControlledEmergency` or
`FailOperationalBounded` posture declaring activation, patient/purpose/field
scope, local policy snapshot and facts, duration, journal, notification,
review/reconciliation and automatic expiry. It never implies unrestricted
record access or autonomous clinical authority.

Model/EHR/feed/monitor outage continues ordinary admitted clinical care and
downtime procedure without granting autonomous model effects. A model status
cannot override patient-safety, professional or institutional authority.

## 13. Evidence

Normal guarded reads select `AtomicEvidence`, `DurableOutboxEvidence`, or a
declared blocking external mode supported by the gateway. Emergency mode may
use `EmergencyDeferredEvidence` only with a bounded durable local journal and
mandatory reconciliation. Evidence failure follows the selected posture; a
best-effort later log is only `ObservedEvidence`. PHI capture is minimized to
the declared replay/verification need.

## 14. Continuous monitoring

Break-glass review and misuse monitoring declare EHR/query/export observation
coverage, heartbeat/checkpoint, clock/loss/backpressure bounds and response
time. Unknown health suspends continuous claims and produces a visible gap;
it does not retroactively invalidate a correctly evidenced guarded decision
or prove absence of misuse.

Clinical-support monitoring additionally covers critical-feed completeness,
schema/unit/method drift, calibration/OOD, subgroup performance, alert burden,
response time, clinician capacity/rubber-stamping, intervention feedback,
benefit/harm endpoints, incidents and scoped recall. A model may be Active at
one site/use, Shadow or Degraded elsewhere, and Suspended for another
population.

## 15. Confidentiality

Facts, PDP inputs, evidence, simulation and replay are governed PHI or
sensitive operational data. The profile declares minimum disclosure,
purpose, local evaluation needs, encryption, access roles, query audit,
retention, legal hold, deletion/tombstone and vendor-log restrictions.
Reusable credentials and unrelated clinical history are prohibited from
forensic capture.

## 16. Budget

The deployment declares access and transition latency, external-exchange
timeouts, maximum authorities/rules/obligations, emergency journal capacity,
monitor interval and evidence estimate. Clinical urgency does not silently
waive an exceeded budget; the admitted safety posture determines the result.

## 17. Conformance cases

The mandatory cases are the healthcare matrix in the
[Phase 0 readiness document](./0029-phase0-readiness-and-domain-matrix.md#72-healthcare-access-and-medication),
plus ordinary permit/deny, excessive-field projection, stale/revoked facts,
authority disagreement, patient merge during workflow, duplicated capability,
wrong patient/product, evidence and PDP outage, monitor blindness and
privileged/derived-data bypass. Each case has canonical inputs, expected
admission result, transition/effect trace and evidence result.

Clinical-support cases additionally include unvalidated site/population,
pediatric input to an adult intended use, patient/encounter merge, changed
unit/method or preprocessing, missing critical feed with healthy service,
stale lab after alert, corrupt input with high confidence, subgroup
insufficient evidence/harm, alert overload/rubber-stamping, model self-
closure, advisory-to-order authority escalation, allergy change before
signature/administration, disputed/delayed labels, intervention feedback,
utility/harm stopping rules, provider compromise, scoped recall and PHI leak.

## 18. Exclusions and unsupported guarantees

- No guarantee of medical correctness, diagnosis, treatment efficacy or
  factual truth beyond named authorities and assumptions.
- No unrestricted break-glass bypass.
- No autonomous medication administration authority inferred from a warning
  or model output.
- No diagnosis or treatment guarantee from an advisory score/receipt.
- Initial profile rejects autonomous executable treatment; a future bounded
  protocol requires separate regulatory, safety, controller and formal proof.
- No claim that audit or review prevents an irreversible disclosure/action.
- No Tier-1 no-bypass claim.
- No safe enforcement claim for ambiguous patient identity, applicability or
  unresolved authority conflict.
- No emergency guarantee without the admitted local-fact, journal and expiry
  prerequisites.

## 19. Current representation verdict

The candidate mapping is `CoreExact` for universal identity, effect,
capability, workflow, safety and evidence concepts, with
`ProfileSpecialization` for clinical vocabulary, minimum necessary,
authority resolution, emergency constraints and M1–M8: intended use,
measurement quality, clinical autonomy, benefit/harm, label adjudication,
human capacity, local validation and post-market recall. Model-free and
model-influenced clinical decisions now share one profile. It remains
unproved until encoded in the candidate schema and its conformance matrix
passes. Any healthcare-only field in the universal core blocks freeze and
must be moved to this profile or justified as a cross-domain primitive.
## KYC Profile

```
Parent:        RFC 0029 and RFC 0029.a model-influence overlay
Status:        Candidate Phase 0 profile; not implemented or certified
Created:       2026-09-27
Source:        RFC 0029 AI-first KYC tabletop and unified rerun
```

## 1. Boundary

This profile covers individual and small-company onboarding, identity and
evidence collection, sanctions/PEP/watchlist screening, beneficial ownership,
risk classification, account opening/restriction, enhanced due diligence,
appeal/correction and ongoing refresh.

The bank owns policy and protected effects. Models may extract, classify,
prioritize, recommend, draft and—only for an exactly admitted low-risk
class—approve. They do not become identity, sanctions, legal or professional
authorities. “KYC decision” is not one undifferentiated effect.

## 2. Resources

Canonical resources include applicant/customer, legal entity, identity
document, biometric sample, address/contact, beneficial owner/controller,
screening candidate/match, evidence item, KYC case, product/application,
account, restriction, appeal/correction and model deployment.

Customer aliases, transliterations, merges/splits and possible duplicates are
versioned relations, not silent equivalence. Corporate ownership graphs are
bounded and carry completeness authority. Account/application/product
identities remain distinct. Model identity binds artifact/weights, runtime,
adapter, configuration, prompt/template, preprocessing/tokenizer, provider,
deployment and material environment.

## 3. Effects

The closed taxonomy includes evidence request/accept/reject, identity create/
merge, screening candidate create/disposition, risk assign/change, case open/
close, application approve/reject/defer, customer create, account open/
restrict/close, enhanced-due-diligence schedule/complete, explanation/notice,
appeal/correction and regulatory notification/reporting where independently
authorized.

APIs, analyst tools, batch imports, vendor callbacks, account jobs, admin
overrides, messages, files and migrations are equivalent/indirect paths in
closure. Creating an alert is distinct from closing it; raising risk is
distinct from lowering it or removing a restriction.

## 4. Vocabulary and predicates

The profile supplies customer/entity/product/jurisdiction types, permitted
identity schemes, document and biometric concepts, evidence quality,
beneficial-owner/control relations, screening categories, risk classes,
source-of-funds/expected-activity concepts, review/escalation classes and
refresh triggers.

Deterministic checks include format/checksum, expiry, issuer signature,
required fields, age/product threshold and exact authoritative identifiers.
Model outputs such as face similarity, liveness, tamper likelihood, fuzzy
match, adverse-media relevance and risk remain typed inferences with
uncertainty, operating domain, abstention/OOD and limitations.

## 5. Facts and authorities

| Fact | Authority | Required behavior |
| --- | --- | --- |
| Applicant assertion | Applicant under authenticated collection | Never authoritative merely because well formed |
| Document/registry identity | Named issuer/registry | Signature/channel, version, freshness and revocation required |
| Biometric/liveness/tamper | Admitted model plus capture/device facts | Inference only; layered controls and abstention required |
| Sanctions/PEP/watchlist | Named list/screening authorities | Source/version/time and disagreement/disposition retained |
| Beneficial ownership/control | Corporate registry plus admitted evidence/adjudicator | Ambiguity/incompleteness escalates |
| Address/source of funds/activity | Named sources and qualified review | Provenance and jurisdictional admissibility required |
| Risk/review/exception | Bank policy and qualified authority | Model score may influence but not redefine policy |
| Account/restriction state | Customer/account authority | Protected effect requires separately bound capability |

Conflicts use declared precedence, quorum, corroboration or uncertainty.
Authority/model compromise supports affected customer/application/account/
restriction discovery and containment/re-review.

## 6. Applicability

Applicability binds jurisdiction, residency/establishment, customer/entity
type, product, channel, evidence/identity scheme, geography, legal status,
effective date and intended model use. Missing/conflicting applicability is
`Indeterminate` or qualified legal/compliance escalation, never silent
selection.

## 7. Policy kinds and composition

The profile uses authorization, evidence sufficiency, bounded graph
completeness, aggregate/velocity constraint, transition, sequence,
separation of duty, adverse-action/appeal and continuous refresh. Mandatory
legal/compliance denies dominate permits after applicability. Independent
screening and human/legal authority floors cannot be displaced by model
confidence.

## 8. Decisions, axes and model influence

| Decision/effect class | Axis B | Initial model binding |
| --- | --- | --- |
| Document classification/extraction | Advisory/Guarded | `Classify` |
| Evidence completeness/expiry/issuer check | Guarded | `None` or declared influence |
| Biometric/tamper/fuzzy-match result | Advisory | `Classify` |
| Queue/case priority | Advisory | `Prioritize` |
| Request information/review route | Guarded | `Recommend` or `Draft` |
| Create screening/fraud referral | Guarded | `Draft`; not an authoritative finding |
| Narrow low-risk onboarding approval | DurableWorkflow | `Approve` only for admitted scope |
| Ambiguous/high-risk approval or exception | DurableWorkflow | Qualified independent authority required |
| Reject/restrict/exit customer | DurableWorkflow | Opaque unattended model authority initially unsupported |
| Open/close/restrict account | Atomic/DurableWorkflow by gateway | Separate capability; model never direct payment/account authority |
| Lower risk/remove restriction/close alert | DurableWorkflow | At least as governed as raising/creating it; self-certification prohibited |
| Ongoing refresh/monitor | Continuous | Declared per influenced decision |

Axis A is Runtime for protected decisions and Continuous where monitoring is
claimed. Axis C remains per fact.

## 9. Decision capability

Capabilities bind subject/authority, canonical applicant/customer/entity,
evidence and registry versions, screening/list receipts, product,
jurisdiction, requested effect/gateway, normalized inputs, policy/rules,
obligations, validity/invalidation, use count and idempotency. Model influence
adds exact model/deployment identity, inference receipt, level and effective
assurance state.

Changed evidence, product, account, list, model, threshold, prompt or input
rejects reuse. Approval cannot authorize a different product/account; a
referral cannot close itself; model approval cannot be consumed for account
restriction or regulatory reporting without a separately admitted authority.

## 10. Obligations

Blocking obligations include authoritative verification, independent
screening, step-up evidence, abstention/OOD routing, qualified review,
separation of duty and current model promises. Post-commit obligations include
notice, refresh, enhanced due diligence, case work, appeal/correction,
regulatory action and impact remediation, with durable retry/deadline/
escalation/terminal evidence.

## 11. Finality and reversibility

Workflow states distinguish evidence pending/verified/disputed, screening
candidate/disposition, review pending, approved/rejected/deferred, customer
created, account requested/open/restricted/closed, appealed/corrected and
`UnknownOutcome`. Each exposed state names its authority. Correction and
appeal preserve the original decision and re-evaluate dependents; closing or
restricting an account may be compensatable but is never rewritten as atomic.

## 12. Safety posture

Identity, screening, model, PDP or evidence outage follows a product- and
jurisdiction-specific admitted posture. New protected onboarding/account
effects normally fail closed or defer to qualified review. A bounded local or
human-controlled mode declares scope, snapshot/facts, duration, evidence,
notification, reconciliation and termination; availability pressure cannot
widen automation.

## 13. Evidence, reasons and appeal

Customer/account creation commits decision evidence or an immutable outbox
commitment within its declared finality boundary. Evidence binds authoritative
facts, inference receipts, human participation, capability/effect result and
matched policy. Adverse reasons must trace to the actual decision path;
plausible generated prose is not sufficient. Notices, correction and appeal
declare audience, evidence access, deadlines, independent authority and
dependent-decision re-evaluation.

## 14. Continuous monitoring

Monitoring covers input/output drift, calibration/OOD, subgroup errors,
deepfake/tamper/evasion, provider changes, screening freshness, selective
labels/feedback, analyst capacity/rubber-stamping, telemetry health and
bypass. Blindness suspends the corresponding claim and narrows/suspends
automation. A model/list/vendor incident triggers scoped impact discovery.

## 15. Confidentiality and lawful data

Documents, biometrics, watchlist results, ownership, financial/activity,
model inputs/outputs, prompts/logs, appeals and assurance cohorts are protected
resources. Purpose, lawful authority, minimisation, locality, encryption,
access, retention/deletion, vendor rights and training/reuse are explicit.
Fairness monitoring cannot require prohibited data collection silently.

## 16. Budget

The deployment declares latency/provider timeouts, bounded ownership graph,
screening/model fan-out, review capacity/response, evidence volume and monitor
intervals. Overload or budget failure narrows/suspends automation instead of
turning review into rubber-stamping.

## 17. Conformance cases

Mandatory cases include high-quality authoritative evidence; blurry/low-
quality input; OOD applicant/document; deepfake/tamper/prompt injection;
transliteration false match; missed independent screening match; cyclic or
incomplete ownership; authority disagreement; evidence changed after
approval; product/account capability replay; silent provider/model update;
subgroup degradation; rubber-stamping; faithful reason and appeal; correction
and dependent re-evaluation; provider/PDP/monitor outage; unlawful model data;
nondeterministic replay/equivalence; compromise impact discovery; automatic
risk lowering/restriction removal; model self-closure; and prompt/log leakage.

Each case requires canonical input, expected admission outcome, decision/
effect trace, capability behavior and evidence result. Fully autonomous “KYC
is correct” must be rejected; bounded automation with abstention/escalation
may be admitted only after the exact profile matrix passes.

## 18. Exclusions and unsupported guarantees

- No universal guarantee that identity, sanctions completeness, risk or legal
  sufficiency is correct because a model produced a receipt.
- Model inference cannot become authoritative fact through thresholding.
- Initial profile excludes opaque unattended adverse action and model
  self-certification.
- Model performance cannot widen effect authority.
- Monitoring/accepted residual risk is not prevention.
- Tier 2 does not establish global absence of bypass.

## 19. Current representation verdict

The unified profile represents deterministic, authoritative, human and
model-influenced KYC decisions without an “AI KYC” schema. A1–A8 map to the
MAP identity, uncertainty, subgroup, adversarial, explanation/appeal,
human-capacity, data-lifecycle and surveillance contracts. No new universal
primitive was required. Formal semantics, candidate-schema encoding and
executable conformance remain pending.
## Manufacturing Profile

```
Parent:        RFC 0029 and RFC 0029.a model-influence overlay
Status:        Candidate Phase 0 profile; not implemented or certified
Created:       2026-09-27
Source:        RFC 0029 cross-domain expansion and unified rerun
```

## 1. Boundary

This profile covers one industrial manufacturing system containing robotic
cells, programmable controllers, energy-isolation hardware, safety relays,
machine-vision inspection, manufacturing execution, maintenance management,
quality release, historian and remote support.

It governs logical authorization, typed capabilities, configuration binding,
workflow, evidence and software-controlled effects. It does not claim that a
database value physically isolates hazardous energy or that ordinary software
substitutes for safety-rated hardware, a certified controller or a qualified
person.

The candidate deployment is Tier 2. A certification statement stops at the
last authority and enforcement boundary supported by evidence.

## 2. Resources

| Resource | Canonical authority | Required identity closure |
| --- | --- | --- |
| Facility/line/cell | Plant asset registry | Site, line, cell and controller aliases resolve to stable identities |
| Machine/asset | Asset configuration authority | Machine type, serial, controller, firmware and installed tooling bind one versioned baseline |
| Safety configuration | Safety authority | PLC program, guards, relays, interlocks, energy sources and network topology are versioned |
| Sensor | Calibration/asset authority | Sensor identity, location, unit, method, range, calibration and replacement history are bound |
| Work order | Maintenance system | Stable identity across approval, isolation, maintenance and restart |
| Personal lock | Physical custody authority | Lock owner, attachment point, attachment/removal evidence and limitations remain distinct facts |
| Material/batch/serial | MES/traceability authority | Splits, merges, rework and derived lots preserve provenance |
| Product configuration | Product/recipe authority | Product, recipe, process version and release criteria bind inspection/release |
| Model deployment | MAP registry | Model, runtime, adapter, configuration, input pipeline, line/camera and intended use are immutable identities |

A material baseline change invalidates dependent capabilities, assurance
promises and safety cases. Ambiguous identity is `Indeterminate`.

## 3. Effects

The closed taxonomy includes:

- `machine.stop`, `machine.enter_maintenance`, `machine.start` and
  `machine.emergency_stop`;
- `energy.isolate`, `energy.verify_zero`, `lock.attach`, `lock.remove` and
  `lock.emergency_remove`;
- `interlock.override`, `guard.restore`, `robot.move` and
  `controller.command`;
- `maintenance.prioritize`, `maintenance.recommend`, `work_order.draft` and
  `work_order.approve`;
- `inspection.classify`, `product.route_for_inspection`,
  `product.quarantine`, `product.rework`, `product.scrap` and
  `product.release`;
- `incident.open`, `notification.send` and `evidence.record`.

Controller commands, acknowledgements, observed physical actuation and
verified physical state are distinct effects/facts with distinct authorities.
Administrative commands, PLC/configuration deployment, historian replay,
database changes, messages, files, remote-vendor actions, migrations and
backfills are included in closure or explicitly excluded without a coverage
claim.

## 4. Vocabulary and predicates

The profile supplies asset types/configurations, energy-source plans, lock and
custody states, safety functions, machine modes, product/recipe/batch states,
defect and disposition classes, sensor concepts/units/methods/calibration,
hard/soft deadlines, safe states and external safety-case references.

Bounded predicates cover plan completeness, zero-energy evidence, personal
lock ownership, guard/tool/person clearance, controller readiness,
configuration equality, sensor compatibility, quarantine reversibility and
release traceability. Industry safety-integrity levels remain external
certification facts; Nirdosha does not assign them.

## 5. Facts and authorities

| Fact | Authority | Failure/disagreement behavior |
| --- | --- | --- |
| Operator/technician qualification | Workforce/safety authority | Missing, expired or revoked denies the protected capability |
| Asset/configuration baseline | Asset and safety configuration authorities | Mismatch invalidates dependent workflow/model status |
| Energy-isolation plan completeness | Qualified safety authority | Missing source or ambiguous applicability blocks maintenance/restart |
| Physical isolation/zero energy | Safety-rated device plus qualified verification | Software record alone is never authoritative physical proof |
| Personal-lock state | Custody evidence plus authorized worker | Another person's removal requires the distinct emergency effect/workflow |
| Controller/interlock state | Safety PLC/controller authority | Hardware rejection dominates a software permit |
| Sensor observation | Named sensor/calibration authority | Stale, changed-method or conflicting readings produce safe state/escalation |
| Inspection/defect disposition | Quality authority | Model output remains inference until the admitted adjudication step |
| Product release | Independent quality/release authority | Model health alone cannot authorize safety-critical release |
| Physical acknowledgement | Controller/device authority | Missing acknowledgement produces `UnknownOutcome` |

Every fact records concept, unit/method, time, freshness, correction/version,
lineage and compromise behavior. Shared corrupted lineage cannot count as
independent corroboration.

## 6. Applicability

Applicability binds facility, jurisdiction, asset/configuration baseline,
product/recipe, process stage, worker/contractor status, hazard class,
maintenance procedure, time and effective safety/quality policy. A model
binding additionally requires exact line, sensor/camera, environment,
intended use and effect tier. Unresolved context or conflicting authority is
`Indeterminate`, not a convenient policy selection.

## 7. Policy kinds and composition

The profile uses authorization, state invariant, transition, sequence,
separation of duty, safety posture, bounded temporal requirement and
continuous monitoring. The physical safety/interlock authority is an
undelegated floor for hazardous actuation. Mandatory safety denies dominate
operational permits; compatible obligations accumulate; incomparable
authority conflict produces safe state/`Indeterminate`.

Emergency removal of another person's lock and interlock override are
distinct, narrowly scoped meta-policy workflows. Neither is inferred from a
supervisor role or production urgency.

## 8. Decisions, axes and model influence

| Decision/effect class | Axis A | Axis B | Initial model-influence binding |
| --- | --- | --- | --- |
| Read attested sensor | Runtime | Guarded | `None` |
| Detect anomaly/defect | Runtime/Continuous | Advisory | `Classify` |
| Rank maintenance/inspection | Runtime | Advisory | `Prioritize` |
| Propose maintenance action/window | Runtime | Advisory | `Recommend` |
| Create non-executable work order | Runtime | Guarded | `Draft` |
| Approve work order/shutdown | Runtime | DurableWorkflow | `None`, or declared model-influenced human decision if materially influenced |
| Enter maintenance after isolation | Runtime | DurableWorkflow | `None` |
| Place bounded quality quarantine | Runtime | Atomic or DurableWorkflow by gateway | `ExecuteReversible` may be admitted |
| Permanently scrap high-value/safety product | Runtime | DurableWorkflow | Model authority initially `Unsupported` |
| Release safety-critical product | Runtime | DurableWorkflow | Model authority initially `Unsupported` |
| Restart/actuate hazardous equipment | Runtime plus external hardware enforcement | DurableWorkflow | Model authority `Unsupported` |
| Detect bypass/configuration drift | Continuous | None | Model use, if any, declared separately |

Axis C is per fact. External devices, inspectors, vendors and certification
authorities require their declared attestation; local facts do not become
external merely because another input is external.

## 9. Decision capability

Every capability binds subject/authority chain, canonical asset and current
configuration, work order/batch/product identities, exact effect/gateway,
normalized inputs, bundle/rules, blocking obligations, issue/expiry,
invalidation set, use count and idempotency key. Model-influenced capabilities
also bind model/deployment/runtime/adapter/configuration identity, inference
receipt, influence level and assurance effective state.

A recommendation capability cannot be consumed for quarantine; quarantine
cannot be consumed for release, scrap, restart or interlock override. A
changed sensor, calibration, PLC program, model, line, product or input digest
rejects reuse. Consumption is idempotent and emits a result receipt.

## 10. Obligations

Blocking obligations can include qualification, plan completeness,
controller/configuration attestation, physical isolation/zero-energy
verification, personal-lock checks, guard/tool/person clearance, current
model promises and required independent human approval.

Post-commit obligations include warning/notification, quality or safety
review, work-order follow-up, enhanced inspection, reconciliation, incident
investigation and recall assessment. They use durable executors with
idempotency, retry, deadline, escalation, terminal handling and evidence.

## 11. Finality and reversibility

The maintenance path distinguishes `WorkApproved`, `StopRequested`,
`StoppedObserved`, `IsolationRequested`, `IsolationVerified`, `Locked`,
`MaintenanceAuthorized`, `MaintenanceActive`, `ClearancePending`,
`RestartAuthorized`, `StartCommandAccepted`, `ActuationObserved`, `SafeState`
and `UnknownOutcome`.

The quality path distinguishes `InspectionPending`, `Classified`,
`HumanReview`, `Quarantined`, `Rework`, `ScrapPending`, `Scrapped`,
`ReleasePending`, `Released` and `Recalled`.

Command requested, accepted, actuation observed and physical state verified
are never collapsed. Quarantine is reversible only when the profile proves
release restores the prior admissible state; scrap and hazardous physical
effects may be irreversible. Compensation never creates historical atomicity.

## 12. Safety posture and real-time boundary

The profile supports `FailClosed`, `FailSafeState`, admitted
`FailOperationalBounded` local operation and `HumanControlledEmergency`.
Every non-fail-closed mode declares activation, permitted effects, asset
scope, local policy/configuration snapshot, required local facts, hard expiry,
exposure, journal, notification, synchronization/reconciliation and automatic
termination.

Emergency-stop and hardware-interlock deadlines remain outside an ordinary
cloud/PDP guarantee. A claimed deadline requires a separately admitted
real-time profile naming clock, worst-case execution, jitter, partition
behavior, local controller and external safety evidence. Generic latency
budgets cannot establish it.

## 13. Evidence

Safety transitions select an evidence-finality mode supported at the local
controller/workflow boundary. Restart requires `AtomicEvidence` or
`DurableOutboxEvidence` for the software transition plus authoritative device
acknowledgement/observation; evidence-export outage retries from the local
commit. An admitted emergency mode may use a bounded local journal with
mandatory reconciliation.

Evidence records exact configuration, authorities, facts, capabilities,
model receipts where applicable, command/acknowledgement/observation and
obligation results. It does not claim that a signed software record proves
physical isolation independently of the declared physical authority.

## 14. Continuous monitoring

Monitoring covers configuration/PLC changes, sensor calibration/drift,
model/input drift, controller commands, interlock outcomes, inspection labels,
false accept/reject and scrap/quarantine/release outcomes, operator review
capacity and bypass paths. It declares observation sources, independence,
heartbeat/checkpoint, clock/skew, sampling/loss/backpressure and response
bounds.

Blindness suspends the affected continuous claim and narrows/suspends model
authority according to policy. It does not disable safety hardware or convert
missing evidence into a healthy state.

## 15. Confidentiality

Asset topology, PLC programs, safety cases, process recipes, vendor data,
images, worker/custody records, incident evidence and model artifacts are
classified protected resources. The profile declares minimum disclosure,
purpose, locality, encryption, access roles, query audit, retention, legal
hold, deletion/tombstone and remote-vendor restrictions. Policy/evidence
collection cannot violate worker, trade-secret or safety-information policy.

## 16. Budget

Each deployment declares separate budgets for ordinary policy decisions,
local safety workflow, hard/soft deadlines, jitter/clock, external authority
calls, bounded collection sizes, obligation fan-out, evidence storage and
monitor detection/response. A hard real-time claim is rejected unless the
actual local mechanism and external safety case support it.

## 17. Conformance cases

| Case | Required result/classification |
| --- | --- |
| Wrong asset alias/configuration | Deny or `Indeterminate`; `CoreExact` identity plus MF1 specialization |
| Isolation plan omits an energy source | Reject plan/restart; profile completeness rule |
| Database says isolated but contactor energized | Reject software-only guarantee; physical authority controls |
| Stale zero-energy sensor | Deny/safe escalation under freshness contract |
| Isolation sensors disagree | Declared safe-state/uncertainty path |
| Central PDP unavailable after isolation | Only admitted local bounded posture may continue; restart remains guarded |
| Start requested while personal lock active | Safety-rated interlock rejects; software permit cannot override |
| Another person's lock removed normally | Deny; only emergency-removal workflow can authorize |
| PLC/configuration changes | Invalidate safety/model cases and suspend affected authority |
| Emergency-stop deadline claimed through cloud PDP | `Unsupported` without real-time safety profile |
| Model trained for asset A used on B | OOD/shadow/deny; no capability issuance |
| Sensor replacement/calibration change | Invalidate semantic input and deployment validation |
| Shared corruption affects model and monitor | Reject independence claim |
| Model recommendation during critical production | Recommendation only; operational authority decides |
| Model says safe while interlock faults | Hardware/interlock deny dominates |
| Vision degradation on one line/camera | Narrow/suspend only affected deployment scope |
| Model-derived labels treated as ground truth | Reject assurance claim without admitted adjudication/correction |
| Review queue overload/rubber-stamping | Narrow/suspend human-dependent authority |
| Model attempts reversible quarantine | Permit only with exact `ExecuteReversible` binding and current promises |
| Model attempts product release, hazardous restart or interlock defeat | Reject authority escalation at admission/gateway |
| Vendor changes model weights | New identity; shadow/revalidate before authority |
| Monitor blind during partition | Suspend affected claim; enter declared local safe posture |

Every case must ultimately have canonical input, expected admission result,
decision/effect trace, capability result and evidence result. The profile is
not conformance-validated merely because this table has the intended answer.

## 18. Exclusions and unsupported guarantees

- Software-only proof of physical energy isolation is unsupported.
- Generic RFC 0029 does not certify emergency-stop worst-case latency,
  industry safety-integrity level or hardware correctness.
- Model authority may not defeat an interlock, remove a personal lock,
  restart hazardous equipment or independently release safety-critical
  product under the initial profile.
- A healthy model is not authoritative evidence that the machine/product is
  safe.
- Monitoring does not prove prevention or observation completeness while
  monitor health is unknown.
- Tier 2 does not claim global absence of privileged, vendor or physical
  bypass.

## 19. Current representation verdict

The unified domain profile represents workflows with both `None` and
non-`None` model-influence bindings without parallel manufacturing schemas.
MF1–MF6 remain profile specializations: asset/configuration baseline,
physical command/acknowledgement, real-time edge safety, external safety case,
personal-lock custody and product-release traceability.

No new universal RFC 0029 or MAP primitive was required. This is positive
conceptual evidence for the overlay, not a Phase 0 pass: the candidate schema,
formal semantics and executable conformance suite remain pending.
## Insurance Profile

```
Parent:        RFC 0029 and RFC 0029.a model-influence overlay
Status:        Candidate Phase 0 profile; not implemented or certified
Created:       2026-09-27
Source:        RFC 0029 cross-domain expansion and unified rerun
```

## 1. Boundary

This profile covers motor/property policy servicing and claims across
customer, broker, adjuster, repairer, fraud, payment, reinsurance,
subrogation, complaint and appeal systems.

Protected processes include quote, bind, renew, cancel, change coverage,
claim intake, evidence collection, coverage adjudication, reserve, fraud
referral, adverse notice, settlement approval, payment, recovery, closure and
reopening. The same process may contain decisions with no model influence and
decisions using extraction, estimation, prioritization, recommendation or
bounded approval models.

The profile governs faithful application of admitted policy and evidence. It
does not decide whether an insurance contract, legal interpretation,
adjustment, price or outcome is substantively correct.

## 2. Resources

| Resource | Canonical authority | Required identity/applicability closure |
| --- | --- | --- |
| Customer/insured/payee | Customer and payment authorities | Roles remain distinct; aliases resolve without treating an inferred payee as authoritative |
| Policy contract | Contract repository | Immutable issued version, forms, endorsements, notices and effective intervals |
| Insured interest | Policy/asset authority | Person/property/vehicle identity and relationship bind coverage |
| Claim/loss event | Claims authority | Stable claim and occurrence IDs; related/duplicate claims are explicit relationships |
| Coverage part/limit | Contract applicability graph | Exact contract node, endorsement precedence and effective scope |
| Evidence item | Evidence source/custodian | Source, acquisition method, integrity, event time and correction version |
| Reserve/settlement/payment | Financial authorities | Distinct identities and states; none aliases another |
| Complaint/appeal | Contestability authority | Stable linkage to decision, notice, evidence and deadlines |
| Model deployment | MAP registry | Immutable model/runtime/adapter/configuration/prompt/input-pipeline identity |

Contract changes after a loss do not rewrite the historical applicable
artifact. Claim reopen, correction, subrogation, salvage and reinsurance add
new linked states/effects and preserve history.

## 3. Effects

The closed taxonomy includes:

- `policy.quote`, `policy.bind`, `policy.change`, `policy.renew`,
  `policy.cancel`, `policy.nonrenew` and `policy.price`;
- `claim.create`, `claim.request_information`, `claim.route`,
  `claim.investigate`, `claim.approve`, `claim.partial_approve`, `claim.deny`,
  `claim.close` and `claim.reopen`;
- `fraud.refer`, `fraud.finding_record`, `reserve.establish`,
  `reserve.change`, `settlement.approve`, `payment.issue`, `payment.clear`,
  `payment.reverse` and `recovery.record`;
- `notice.draft`, `notice.dispatch`, `complaint.open`, `appeal.decide`,
  `subrogation.act`, `salvage.dispose` and `reinsurance.notify`.

Reserve, settlement approval, payable amount, payment issue and cleared funds
are different effects. Database/admin actions, bulk catastrophe processing,
files, messages, external payment calls, migrations and backfills are included
in closure or explicitly excluded without a coverage claim.

## 4. Vocabulary and predicates

The profile supplies contract/form/endorsement types, insured interests,
perils, exclusions, deductibles, limits, currencies, loss causes and dates,
coverage/disposition states, reserve and settlement concepts, payment and
recovery states, evidence classes, adverse actions, notices, reason codes,
appeal/complaint rights and catastrophe contexts.

Money uses integer minor units with explicit currency/scale. Bounded
predicates cover effective contract selection, insured interest, covered
peril, exclusions, deductible/limit/prior-payment calculations, authority
limits, separation of duty, evidence sufficiency and deadline rules. Ambiguous
contract interpretation is not converted into a generic Boolean predicate; it
enters qualified adjudication.

## 5. Facts and authorities

| Fact | Authority | Failure/disagreement behavior |
| --- | --- | --- |
| Issued contract/endorsements | Contract authority | Exact effective artifact required; conflict escalates |
| Loss time/location/peril | Named claimant, assessor and external sources | Preserve provenance/disagreement; claimant assertion alone is not automatically authoritative |
| Insured interest/asset/person | Contract and identity authorities | Ambiguity is `Indeterminate` |
| Damage/repair estimate | Repairer, assessor or admitted model | Sources remain separate; model estimate is inference |
| Coverage interpretation | Admitted rule or qualified adjuster/legal authority | Opaque/ambiguous clauses require human/legal workflow |
| Fraud evidence/finding | Investigation/adjudication authority | A model score can create referral, never a fraud fact |
| Adjuster/approver authority | Workforce/delegation authority | Monetary/scope limit and revocation revalidated |
| Payee/payment instruction | Authoritative payment/customer process | Model inference cannot establish or change payee |
| Payment status | Payment provider/bank/ledger authority | Timeout after possible acceptance produces `UnknownOutcome` |
| Legal/regulatory interpretation | Named legal/regulatory authority | Effective scope and retroactive impact are explicit |

Authority disagreement uses declared precedence, corroboration, quorum or
qualified adjudication. Later correction/compromise supports affected-policy,
claim, notice and payment discovery and governed reopening/remediation.

## 6. Applicability

The coverage/applicability graph binds issued contract and endorsements,
insured interest, peril, loss time/location, jurisdiction, product, limits,
exclusions, deductibles, prior payments, claimant/payee status and effective
legal interpretation. It is distinct from the RFC 0029 enforcement policy
that controls how these artifacts are selected, interpreted and evidenced.

Missing, inconsistent or contested applicability produces `Indeterminate` or
qualified adjudication. The engine never selects a favorable contract,
endorsement or jurisdiction silently.

## 7. Policy kinds and composition

The profile uses authorization, numeric/aggregate constraint, state
transition, sequence, separation of duty, applicability, temporal obligation,
data policy and contestability workflow. Applicable law and issued contract
terms are source artifacts under named authorities, not interchangeable
policy tiers.

Mandatory denies and compatible obligations compose only after applicability
selection. Contract/authority conflict, incomparable evidence or ambiguous
interpretation enters the declared adjudication path. Catastrophe posture may
change prioritization/capacity behavior but cannot silently discard coverage,
payment, notice, fairness or appeal controls.

## 8. Decisions, axes and model influence

| Decision/effect class | Axis A | Axis B | Initial model-influence binding |
| --- | --- | --- | --- |
| Extract/normalize submitted evidence | Runtime | Advisory | `Classify` |
| Route or prioritize claim | Runtime | Advisory | `Prioritize` |
| Estimate damage/severity/repair cost | Runtime | Advisory | `Recommend` |
| Draft adjustment, notice or information request | Runtime | Guarded | `Draft` |
| Create fraud referral | Runtime | Guarded | `Recommend` or bounded `Draft`; never authoritative fraud finding |
| Establish/change reserve | Runtime | Atomic or DurableWorkflow by gateway | `Recommend`; final authority remains policy/adjuster unless separately admitted |
| Approve narrow low-value claim | Runtime | DurableWorkflow | `Approve` may be admitted for an exact bounded class |
| Deny, cancel, non-renew, materially restrict or price | Runtime | DurableWorkflow | Opaque/unattended model authority initially `Unsupported` |
| Approve settlement | Runtime | DurableWorkflow | Qualified authority; model may recommend/draft |
| Identify/change payee | Runtime | Guarded | Model authority `Unsupported` |
| Issue payment | Runtime | Atomic locally plus DurableWorkflow externally | Model authority `Unsupported`; authoritative payee/payment controls mandatory |
| Appeal/complaint decision | Runtime | DurableWorkflow | Independent authority; model may not review itself |
| Portfolio/catastrophe surveillance | Continuous | None/Advisory | Declared per monitor/decision |

Axis C remains per fact. A single external estimate does not change local
contract or ledger facts into externally attested facts.

## 9. Decision capability

Capabilities bind subject/authority chain, exact contract/endorsement and
claim versions, insured interest/loss/evidence identities, effect/gateway,
amount/currency/payee where applicable, normalized input digest, bundle/rules,
obligations, issue/expiry/invalidation, use count and idempotency key.

Model-influenced capabilities additionally bind model/deployment/runtime/
adapter/configuration/prompt identity, inference receipt, influence level and
effective assurance state. A routing or recommendation capability cannot be
used to approve, deny, dispatch notice, change payee or issue payment. Changed
inputs, contract, model, threshold, catastrophe scope or payee reject reuse.

## 10. Obligations

Blocking obligations include contract/applicability resolution, evidence and
authority checks, human/legal adjudication, separation of duty, authoritative
payee verification, faithful reason validation and required appeal/notice
content. Model-influenced paths add current promises, abstention/OOD behavior,
source-evidence access and human-capacity checks.

Post-commit obligations include notice dispatch, payment/reconciliation,
complaint/appeal routing, sampling, investigation, regulatory reporting,
reinsurance/subrogation and affected-claim remediation. Each is durably
tracked with idempotency, retry, deadline, escalation and terminal handling.

## 11. Finality and reversibility

Claim state distinguishes `Reported`, `InformationPending`, `Investigating`,
`CoveragePending`, `Approved`, `PartiallyApproved`, `Denied`, `Contested`,
`Closed`, `Reopened` and correction states.

Financial state distinguishes `ReserveSet`, `SettlementApproved`, `Payable`,
`PaymentIssued`, `ProviderAccepted`, `Cleared`, `Reversed`, `Recovered`,
`Subrogated`, `ReinsurancePending` and `UnknownOutcome`. Each state names its
authority and visibility. Reserve is never payment authority. External
timeout does not imply failure or success; reconciliation resolves it without
duplicating effects. Correction/reversal is a later recorded effect.

## 12. Safety and catastrophe posture

Normal protected decisions use their admitted fail-closed/manual-escalation
posture. Catastrophe mode is a versioned, bounded operating posture declaring
event/scope, activation authority, capacity assumptions, permitted automation,
prioritization, local snapshots/facts, duration, exposure, sampling, fairness
and harm monitoring, notification, reconciliation and automatic termination.

Queue pressure cannot widen model authority, suppress meaningful review,
change thresholds silently, weaken payee/payment controls or remove appeal.
When review capacity falls below its contract, affected automation narrows or
suspends.

## 13. Evidence and explanation fidelity

Local claim/financial mutations select `AtomicEvidence` or
`DurableOutboxEvidence`; external payment and notice providers supply bound
receipts with explicit finality. Evidence outage follows the selected mode and
cannot silently erase a decision, notice or payment trace.

An adverse notice binds to the actual contract clauses, applicable rules,
facts, decision lineage, human/model participation and safe reason mapping.
Generated explanations are validated against that lineage; an invented
exclusion or different reason is rejected before dispatch. Replay minimizes
personal/financial data and never retains reusable credentials.

## 14. Continuous monitoring

Monitoring covers input/data drift, manipulation/deepfake signals, subgroup
outcomes, false accept/reject, repair/settlement outcomes, selective labels,
review capacity/rubber-stamping, reason fidelity, complaints/appeals,
catastrophe scope, provider/model changes and payment reconciliation.

Observation sources, independence, heartbeat/checkpoint, skew/loss/sampling,
backpressure and detection/response bounds are explicit. Blindness suspends
the affected continuous claim and model authority as policy requires; it
never becomes evidence of no harm or no violation.

## 15. Confidentiality, lawful data and proxy controls

Contracts, claims, images, location, financial, health, identity, fraud,
external data, model inputs/outputs and appeal evidence are protected
resources. The profile declares purpose, lawful/admitted source, direct and
proxy attribute controls, minimum disclosure, locality, encryption, access,
query audit, retention, legal hold, deletion/tombstone and training/reuse
permissions.

Removing an explicit protected field does not prove absence of proxy
discrimination. Cohort evaluation, explanation and impact remediation follow
the applicable policy and authority without requiring prohibited collection.

## 16. Budget

Deployments declare decision latency, provider timeouts, maximum evidence and
authority fan-out, aggregate/portfolio window, review queue/load and response
bounds, catastrophe capacity, obligation fan-out, evidence storage and
monitoring intervals. Over-budget operation follows the admitted posture; it
does not silently reduce review or widen automation.

## 17. Conformance cases

| Case | Required result/classification |
| --- | --- |
| Wrong policy/endorsement version | Resolve loss-time applicability or `Indeterminate` |
| Contract changes after loss | Historical applicable artifact remains bound |
| Coverage clause ambiguous | Qualified adjudication, not arbitrary Boolean result |
| Duplicate claim/payment retry | Idempotent original result/receipt |
| Reserve used as payout authority | Reject distinct-effect mismatch |
| Adjuster exceeds monetary/scope authority | Deny/escalate |
| Same person changes payee and approves | Separation-of-duty deny |
| Customer/repairer/assessor disagree | Preserve sources and adjudicate uncertainty |
| Payment provider accepts then times out | `UnknownOutcome`; reconcile without duplicate payment |
| Denial notice differs from actual decision | Reject/correct before dispatch |
| Legal interpretation changes | Discover affected claims and govern reopening |
| Catastrophe overwhelms review | Narrow/suspend automation; preserve controls |
| Clear low-value claim in validated scope | `Approve` only under exact bounded policy/recovery/sampling/legal authority |
| Photo/document manipulated or generated | Detect/abstain/escalate |
| Model sees unusual asset/loss | OOD/uncertainty and human review |
| Fraud score treated as fraud fact | Reject inference-to-fact promotion; referral only |
| Model harms a claimant/proxy cohort | Narrow/suspend and perform impact review |
| Generated reason invents exclusion | Reject through faithful contract/rule trace |
| Reviewer sees only model summary | Meaningful-review contract fails |
| Threshold/model changes during catastrophe | New identity/policy; no old capability reuse |
| Labels include only model-selected referrals | Record selection/feedback limitation; reject stronger promise |
| Model recommends non-renewal from external data | Require provenance, applicability, lawful use, explanation and admitted authority |
| Model closes referral it created | Reject self-certification without admitted independent process |
| Vendor compromise discovered | Revoke and discover affected quotes/claims/notices/payments |
| Model attempts denial/cancellation/pricing/payment | Reject authority escalation under initial profile |
| Payment targets inferred payee | Reject; authoritative payee checks remain mandatory |

Every case ultimately requires canonical inputs, expected admission outcome,
decision/effect trace, capability behavior, notice/explanation result and
evidence result. This prose matrix is not executable conformance.

## 18. Exclusions and unsupported guarantees

- RFC 0029 policy is not the customer's insurance contract and does not
  replace legal interpretation.
- A model score is not fraud, coverage, liability, payee or loss truth.
- Initial model authority excludes opaque unattended denial, cancellation,
  non-renewal, material restriction/pricing and payment execution.
- A model may not create the authoritative payee or close its own referral
  where independence is required.
- Technical accuracy alone does not establish lawful use, fair outcome,
  faithful reasons, adequate human review or contract correctness.
- Catastrophe mode is not permission to discard controls.
- Tier 2 monitoring cannot claim prevention or global absence of bypass.

## 19. Current representation verdict

The unified profile represents both model-free and model-influenced insurance
decisions without parallel schemas. IN1–IN6 remain profile specializations:
contract versus enforcement policy, coverage/applicability graph, financial
and multi-party finality, adverse action/contestability, proxy/lawful-data
controls and catastrophe/portfolio context.

No new universal RFC 0029 or MAP primitive was required. This is positive
conceptual evidence for the per-decision overlay, not a Phase 0 pass: the
candidate schema, formal semantics and executable conformance suite remain
pending.
