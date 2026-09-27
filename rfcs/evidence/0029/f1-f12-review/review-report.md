# RFC 0029 — Independent F1-F12 Review: Banking/Healthcare

Reviews:       rfcs/evidence/0029/tabletops/banking-healthcare.md
               against rfcs/0029-domain-neutral-policy-enforcement.md §36
Status:        FAIL
Reviewed:      2026-09-27
Method:        Independent read-only review of current RFC 0029 text (§36.1-
               36.12, plus §§6-26, 29, Appendix A) against the original
               F1-F12 failure register (§5) and both test matrices (§3.4,
               §4.4), with independent verification of the §7.1
               gap-resolution traceability table's own section citations.

## 1. Overall verdict

**FAIL**, but with an important qualification: this is a materially better
outcome than the original tabletop found. Read against current RFC 0029 text
(not just section headings), eleven of the twelve F1-F12 items now have
concrete, checkable normative content — a resolution rule with a stated
default (usually `Indeterminate` or a named safety posture), an admission
gate that can reject a submission for failing to declare it, and in most
cases a dedicated stable rejection-reason code. That is a real upgrade from
"names the concept without resolving the ambiguity," which is what the
original tabletop was checking for. Most of the "Partial"/"Fail" rows in
both test matrices upgrade cleanly to "Pass" against the current §36 text.

The verdict is FAIL rather than PASS for three specific, defensible reasons,
none of which is cosmetic:

1. **F8 (dependency/transition revalidation) is not actually gated at
   admission.** §18 and §36.8 describe the mechanism in prose, but the
   numbered admission-check list in §12 — which every other one of the
   twelve items maps onto explicitly — has no corresponding item, and there
   is no dedicated rejection-reason code. Two honest implementers could
   disagree about whether an incomplete transition/fact dependency map is a
   hard admission failure or just an Appendix A "checklist" nicety. This is
   scored INDETERMINATE below.
2. **Two matrix rows checked against the current text still show a genuine,
   named gap**, not a stale artifact of the old text: banking's
   "Reconciliation detects imbalance" (who is authorized to trigger a
   correction/compensation is never pinned down the way capability
   consumption and evidence-finality are) and healthcare's "Patient requests
   explanation" (the Explanation evidence profile in §23 names the artifact
   but not audience-tiered disclosure rules).
3. **A precedence gap between F1's `Indeterminate` default and F6's
   fail-operational safety posture is never resolved**: RFC 0029 does not
   say which wins when an ambiguous canonical-identity result (which
   fail-closes a critical effect per §13) coincides with an active,
   meta-policy-authorized `FailOperationalBounded`/`HumanControlledEmergency`
   mode (which is designed to keep protected effects available). This
   matters specifically in the healthcare scenario (wrong-patient identity
   during an active resuscitation) and is not a hypothetical edge case.

Per the task's own rule — PASS requires all twelve items to score PASS *and*
no checked matrix row to show a genuine unresolved contradiction — the
correct verdict is FAIL. It should be read as "close, and much improved,
with three named, fixable gaps," not as "the tabletop's findings still stand
unchanged." Section 5 below lists exactly what would flip this to PASS.

## 2. F1-F12 scoring

| Finding | Claimed resolution (RFC 0029 §) | Verified accurate? | Score | Reason |
| --- | --- | --- | --- | --- |
| F1 Resource identity | §§6-7, 12-15, 21, 24, 36.1 | Yes | PASS | §36.1 + §12 item 6 make canonical-resource resolution mandatory *before* decision with a safe default (`Indeterminate` on ambiguity, which fail-closes critical effects per §13). §21's bypass classification (`Uncontrolled` rejects certification) honestly bounds the residual case of an alias the domain module never declared — that is an inherent, disclosed Tier-2 limitation (§9.2), not a new specification ambiguity. Residual concern: interacts with F6 (see §1, item 3) without a stated precedence rule. |
| F2 Effect closure | §§6-7, 12-15, 21, 24, 36.2 | Yes | PASS | §12 item 7 plus §36.2's "an unclassified effect cannot enter certified scope" is a real closure obligation, not just a naming exercise. §21 explicitly enumerates migrations, backfills, admin ops, derived stores/indexes, settlement files, and message producers as in the closure — this directly answers F2's original example list. |
| F3 Decision capability | §§6, 12-15, 24, 36.3 | Yes | PASS | §14 and §36.3 bind the capability to request identity, subject/authority chain, canonical resource+version, effect+gateway, a normalized input digest, bundle/rules, obligations, issuance/expiry/revocation, use-count, and idempotency, with an explicit "reuse with changed inputs is rejected" rule and a mandated consumption receipt. This is a genuinely complete, checkable contract. |
| F4 Evidence finality | §§6, 12-13, 15, 23-24, 36.4 | Yes | PASS | §23 defines five named evidence-finality modes with distinct commit semantics (`AtomicEvidence`, `DurableOutboxEvidence`, `SynchronousExternalEvidence`, `EmergencyDeferredEvidence`, `ObservedEvidence`) and §36.4 forbids claiming durable evidence without selecting one. This directly answers "does the effect commit if evidence storage fails" — the answer is now mode-dependent and explicit rather than undefined. |
| F5 Distributed finality | §§6, 12-13, 15, 22, 26, 36.5 | Yes | PASS | §22/§36.5 name a workflow-state vocabulary (requested/reserved/committed/submitted/accepted/settled/final/reversed/compensated/unknown), make `UnknownOutcome` first-class, and forbid compensation from claiming historical atomicity or undoing disclosure/physical acts. Minor gap: no dedicated admission rejection code (see §5, finding 2), and "as applicable" leaves the exact state subset per-workflow, which is a reasonable domain-neutral design choice rather than an ambiguity that lets an implementer under-deliver on the anti-cheat clauses. |
| F6 Safety posture | §§6, 12-13, 20, 26, 36.6 | Yes | PASS | §26/§36.6 give a closed four-value posture set with mandatory elements (activation, scope, local snapshot, hard expiry, exposure statement, evidence journal, notification, reconciliation, automatic termination) for any non-`FailClosed` mode, and explicitly bar availability pressure alone from activating one. This matches F6's required change almost point for point. |
| F7 Authority conflict | §§6, 12, 17, 20, 24, 36.7 | Yes | PASS | §17/§36.7 require each fact type to declare its authoritative source set and a precedence/quorum/corroboration/uncertainty rule, forbid resolving conflicts by response order, and require a retroactive-compromise window, affected-decision discovery, and containment/re-evaluation/notification/compensation. `AuthorityConflict` is a dedicated admission rejection code. |
| F8 Transition revalidation | §§6, 12-13, 18, 22, 36.8 | **No — §12 citation is inaccurate** | INDETERMINATE | §18/§36.8 give real prose semantics (compile a dependency graph; each transition declares current-required facts, pinned facts, invalidation triggers, max staleness) that closely track F8's required change. But unlike every other one of the twelve items, **the numbered admission-check list in §12 (items 1-24) has no item testing this**, and there is no dedicated rejection-reason code (only Appendix A's unnumbered checkbox names it). The §7.1 table's citation of "§12" for F8 therefore overstates what §12 itself does. Two honest implementers could disagree on whether an incomplete dependency map is a hard admission failure. **What would resolve it:** add a numbered §12 admission check and a rejection code (e.g. `DependencyMappingIncomplete`) parallel to the other eleven items. |
| F9 Aggregate consistency | §§6, 12, 25, 36.9 | Yes | PASS | §12 item 18 explicitly requires aggregate policies to declare consistency domain, partition behavior, and max-overshoot exposure — this one *is* correctly present in the numbered list (unlike F8). §25/§36.9 add reservation/reconciliation/late-event detail and the rule that non-zero overshoot is an exposure bound, not invariant satisfaction. Minor gap: no dedicated rejection code (falls back to a generic one), noted in §5. |
| F10 Monitor health | §§6, 11-12, 21, 29, 36.10 | Yes | PASS | §12 item 20, §21, and §36.10 require observation-source coverage, heartbeat/checkpoint, skew assumptions, loss/sampling/backpressure bounds, and — critically — suspend the detection-bound claim (rather than silently continuing it) when health becomes unknown. `MonitorHealthUnknown` is a dedicated rejection code. |
| F11 Policy-plane confidentiality | §§6, 12, 23-24, 36.11 | Yes | PASS | §12 item 21 and §36.11 require minimum disclosure, classification, locality, retention, deletion, and query controls for PDP/PIP/evidence/simulation/replay, and forbid a policy from requiring evidence a governing data policy forbids collecting. `EvidenceConflict` is a dedicated rejection code. |
| F12 Applicability | §§6, 12-13, 19, 24, 36.12 | Yes | PASS | §19/§36.12 make an applicability phase mandatory *before* composition, from typed authoritative facts, with ambiguous applicability producing `Indeterminate`/manual escalation and never a silent jurisdiction choice. `ApplicabilityAmbiguous` is a dedicated rejection code, and §13 step 1 places this first in the decision protocol. |

**11 of 12 PASS, 1 INDETERMINATE (F8), 0 FAIL** at the item level. The
citation-accuracy check (task step 1) found the §7.1 traceability table's
section list is accurate for eleven items and **overstated for F8** (see
above) — this is the one place where the original document's own table
should not be trusted at face value.

## 3. Banking matrix re-judgment (7 rows, §3.4)

| Test | Original result | Re-judged result | Reason for change (or lack of it) |
| --- | --- | --- | --- |
| Retry after client timeout | Partial | **Upgraded to Pass** | §36.3's idempotency-key + consumption-receipt requirement, with the gateway atomically reserving/consuming the capability, is exactly the "canonical decision/effect idempotency binding" the original finding said was missing. |
| Same request ID, changed amount | Partial | **Upgraded to Pass** | §36.3's "normalized input digest" bound into the capability, plus the explicit "reuse with changed inputs is rejected," directly supplies the missing normative request-digest binding. |
| Screening changes after reservation | Partial | **Upgraded to Pass** | §18's compiled dependency graph plus §36.8's per-transition current-fact/staleness declarations give the missing "normative dependency map." (Caveat: this item's own admission enforceability is the F8 gap noted above — the *mechanism* now exists in prose, but nothing yet forces a bank's bundle to actually populate it.) |
| Ledger commits but evidence store fails | Fail | **Upgraded to Pass** | §23's five evidence-finality modes plus §36.4's "no protected effect may claim durable evidence without one of these modes" directly answers the original question: e.g. under `DurableOutboxEvidence` the local record commits atomically with the ledger effect and remote-store failure only delays export, never threatens the effect's durability claim. |
| Sanctions authority compromised | Fail | **Upgraded to Pass** | §36.7 plus §17/§20 now require a retroactive compromise window, affected-decision discovery, and containment/re-evaluation/notification/compensation "as applicable," with `AuthorityConflict` as a dedicated rejection code — this replaces "general incident response" with a specific contract. |
| Two screening authorities disagree | Fail | **Upgraded to Pass** | §36.7's per-fact-type precedence/quorum/corroboration/uncertainty rule directly supplies the "composition under authority policy" the original finding said was absent. |
| Beneficiary/account represented by aliases | Fail | **Upgraded to Pass** | §36.1's canonical-authority/namespace + alias/merge/split declarations + `Indeterminate`-on-ambiguity default gives a real (if domain-module-dependent) canonical-identity model, replacing the prior total absence. |
| Reconciliation detects imbalance | Partial | **Still Partial — not upgraded** | §36.5 defines irreversible/compensatable/reversible classification and bars compensation from claiming historical atomicity, but never states who is authorized to *trigger* a correction/compensation action at runtime. §16's waiver-authorization contract (meta-policy-authorized authority, scope, expiry, reason, review) is not explicitly extended to compensation/correction actions the way it is to waivers. "Correction authority" — half of the original finding — remains unresolved. |

## 4. Healthcare matrix re-judgment (8 rows, §4.4)

| Test | Original result | Re-judged result | Reason for change (or lack of it) |
| --- | --- | --- | --- |
| Emergency break-glass | Partial | **Upgraded to Pass** | The general decision-capability contract (§36.3: bounded/single use, issuance/expiry/revocation, idempotency) plus §36.6's `HumanControlledEmergency` posture (narrow scope, hard expiry, automatic termination) together normalize exactly the "capability scoping and single-use/replay semantics" the original finding called out as missing. |
| Consent registry unavailable during emergency | Fail | **Upgraded to Pass** | §36.6 is a direct, purpose-built answer: a meta-policy-authorized `FailOperationalBounded`/`HumanControlledEmergency` posture replaces the unsafe generic fail-closed default, with mandatory activation evidence, local snapshot, expiry, exposure statement, notification, and reconciliation. |
| Local and external allergy facts disagree | Fail | **Upgraded to Pass** | §36.7's precedence/quorum/corroboration/uncertainty contract for each fact type is exactly the missing "multi-authority disagreement algebra or uncertainty type." |
| Search index reveals sensitive terms | Fail | **Upgraded to Pass** | §21 explicitly lists "derived stores and indexes" inside the mandatory coverage closure, and §36.2's "unclassified effect cannot enter certified scope" forbids leaving a search-index read out of the covered effect set. This directly answers "no mandatory derivation/alias closure." |
| PDP outage during active resuscitation | Fail | **Upgraded to Pass** | §36.6's required elements (activation evidence, local policy snapshot, hard expiry, evidence journal/retrospective reconciliation) match the original required change ("admitted safety envelope, local policy snapshot, expiry, and retrospective evidence") almost verbatim. |
| Wrong-patient alias/merged record | Fail | **Upgraded to Pass, with a residual caveat** | §36.1 gives merge/split events, declared inheritance, and `Indeterminate` on ambiguous identity — genuinely no longer "absent." Caveat: the RFC never states whether an `Indeterminate` canonical-identity result takes precedence over an *active* `HumanControlledEmergency`/`FailOperationalBounded` posture, or vice versa, when both apply to the same in-flight decision (exactly the resuscitation-plus-wrong-patient case). This is the interaction gap named in §1 — real, but narrower than the original blanket "absent" finding. |
| Evidence store unavailable during record read | Fail | **Upgraded to Pass** | §36.4 ties durable-evidence claims to a selected finality mode for every decision class, including reads, and `EmergencyDeferredEvidence` is explicitly coordinated with a meta-policy-authorized safety mode — this directly answers "evidence durability is not tied to effect finality or safety posture." |
| Patient requests explanation | Partial | **Still Partial — not upgraded** | §23 names an `Explanation` evidence profile ("authorized rule summary and safe reason codes") but does not give audience-tiered disclosure rules (what a patient may see vs. a clinician, auditor, or regulator). §36.11 covers policy-plane confidentiality generally (purpose, classification, field-level access) but does not connect that machinery to explanation-audience tiering specifically. The original finding — "audience-specific disclosure rules need semantics" — still stands. |

## 5. Findings requiring correction

1. **F8 has no admission-time enforcement hook.** §18/§36.8 supply real
   prose semantics for dependency/transition revalidation, but §12's
   numbered admission-check list (the only one of the twelve items this is
   true for) has no corresponding check, and there is no dedicated
   rejection-reason code. Fix: add a numbered item to §12 (parallel to item
   18 for aggregates or item 20 for monitor health) and a rejection code
   such as `DependencyMappingIncomplete`.
2. **F5 and F9 also lack dedicated rejection-reason codes** in §12's
   enumeration (`Ambiguous`, `UnsupportedSemantics`, `Undecidable`,
   `MissingAuthority`, `MissingEnforcementPoint`, `InsufficientAxisA`,
   `InsufficientAxisB`, `AssuranceMismatch`, `UnachievableTier`,
   `UncontrolledBypass`, `UnresolvedResourceIdentity`, `UnclassifiedEffect`,
   `DecisionBindingIncomplete`, `EvidenceConflict`,
   `EvidenceFinalityUnsupported`, `AuthorityConflict`,
   `ApplicabilityAmbiguous`, `SafetyPostureUnsupported`,
   `MonitorHealthUnknown`, `OverBudget`) even though F5 and F9 do have
   numbered admission checks (items 15 and 18 respectively). This is a
   smaller, cosmetic inconsistency next to F8's, but fixing all three
   together would make the code list exhaustively map onto all twelve
   hardening requirements, removing any temptation to fall back on a vague
   generic code. Suggested additions: `FinalityStateUnsupported` (F5),
   `AggregateConsistencyUndeclared` (F9).
3. **No stated precedence between an `Indeterminate` canonical-identity
   result (F1/§36.1) and an active fail-operational/emergency safety
   posture (F6/§36.6).** Both are individually well-specified, but their
   interaction is not addressed anywhere in §36, §13, or §26. This matters
   most exactly where the tabletop put it: a wrong-patient/merged-record
   ambiguity surfacing during an active resuscitation. Fix: state explicitly
   whether a safety posture may override an identity-ambiguity fail-close
   (and if so, under what bounded conditions and with what evidence), or
   whether identity ambiguity always takes precedence and forces a stop.
4. **"Correction authority" for compensation/reconciliation actions is
   undefined.** §36.5 and §22 define reversible/compensatable/irreversible
   classification and bar compensation from claiming historical atomicity,
   but never state who may authorize triggering a correction, unlike the
   explicit meta-policy-authorized-authority requirement §16 gives to
   waivers. Fix: extend §16's waiver-authorization pattern (or an
   equivalent) explicitly to compensation/correction obligations.
5. **Explanation-audience tiering is unaddressed.** §23's `Explanation`
   evidence profile states content ("authorized rule summary and safe
   reason codes") but not audience-differentiated disclosure (patient vs.
   clinician vs. auditor vs. regulator). Fix: either fold this into §36.11's
   policy-plane confidentiality contract explicitly, or add an
   audience/purpose dimension to the `Explanation` profile in §23.

None of these five findings is a full regression to the original tabletop's
verdict — each sits on top of substantial, genuine normative progress in
§36 and its supporting sections. But per the stated PASS bar (all twelve
items PASS, no matrix row shows a genuine unresolved contradiction), they are
sufficient, and specific enough, to keep the overall verdict at FAIL until
addressed.
