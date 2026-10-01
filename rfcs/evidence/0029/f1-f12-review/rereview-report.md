# RFC 0029 — Independent Re-review of F1-F12 Corrections

Reviews:       rfcs/0029-domain-neutral-policy-enforcement.md (post-correction)
               against rfcs/evidence/0029/f1-f12-review/review-report.md §5
Status:        PASS
Reviewed:      2026-09-27
Method:        Independent read-only re-review of each of the 5 corrections
               plus the 2 flagged matrix rows

## 1. Overall verdict

**PASS.** I read the current text of §12, §13, §16, §23, and §36.1/36.5/36.6/
36.8/36.11 (plus Appendix A) directly, without assuming the edits work, and
checked each one against the exact gap the first reviewer named. All five
findings now have real, numbered, checkable normative force — a stated
default, an admission gate, or a named authority requirement, not just a
sentence gesturing at the concept. The two "Still Partial" matrix rows both
upgrade cleanly against the new text for the same reasons the first
reviewer's own "what would resolve it" language predicted.

I did not find a case here of "two honest implementers could disagree" —
the failure mode the first reviewer explicitly flagged for F8's original
gap. Each of the five fixes ties a concrete rejection code or a concrete
authority/audience declaration to a specific numbered admission item, and
the cross-references between §12, §13/§16/§23, §36.x, and Appendix A are
mutually consistent (same section numbers, same code spellings) rather than
drifting the way a partial or cosmetic patch often does.

## 2. Finding-by-finding re-judgment

| Finding | Original issue | New text (quote/paraphrase) | Resolved? | Reason |
| --- | --- | --- | --- | --- |
| 1. F8 admission hook | §12's numbered list had no item for dependency/transition revalidation, and no rejection code. | §12 item 12 (new numbering): "the dependency graph ... is complete for the certified scope, and every transition declares which facts must be current, which may remain pinned, its invalidation triggers, and its maximum permitted staleness (§18, §36.8) — an incomplete map is an admission failure here, not only an Appendix A checklist item." Rejection list (line ~451) now includes `DependencyMappingIncomplete`. §36.8 and Appendix A both cross-cite "§12 item 12" by the same number. | **Yes** | This is exactly the fix the first reviewer specified: a genuinely numbered admission check (not prose elsewhere) plus a dedicated code, and the RFC's own cross-references agree on the item number, so there's no room for the "which document actually governs" ambiguity that produced the original INDETERMINATE. |
| 2. F5/F9 rejection codes | `FinalityStateUnsupported` (F5) and `AggregateConsistencyUndeclared` (F9) — or equivalent — were missing from §12's code enumeration. | The rejection-reason list now reads in part: "...`EvidenceFinalityUnsupported`, `FinalityStateUnsupported`, `AuthorityConflict`, `ApplicabilityAmbiguous`, `SafetyPostureUnsupported`, `MonitorHealthUnknown`, `AggregateConsistencyUndeclared`, and `OverBudget`." | **Yes** | Both suggested code names appear verbatim in the enumeration, sitting alongside the other ten domain-specific codes rather than off in a separate list, so the code set now maps onto all twelve F1-F12 items rather than eleven. |
| 3. F1/F6 precedence | No stated precedence between `Indeterminate` canonical identity and an active fail-operational/emergency posture; no default; no handling of the silent-posture case. | §13: "identity ambiguity continues to fail close the effect **unless** the active posture's own admitted declaration names canonical-identity ambiguity as a condition it is specifically authorized to operate through, with its own bounded scope, evidence, and expiry for that exact case. A posture that is silent on identity ambiguity does not implicitly cover it; the effect still fails closed." Mirrored in §36.1, §36.6, and gated at admission by §12 item 23 ("the bundle states which of the two controls"). | **Yes** | This gives an actual default (fail-close wins) plus a real, bounded escape hatch (the posture must *explicitly* name identity ambiguity, with its own scope/evidence/expiry) plus an explicit answer for the silence case, which is the exact scenario ("wrong-patient during resuscitation") the first reviewer said mattered. It is a rule an implementer can apply mechanically, not "the policy decides" with no fallback. |
| 4. Correction authority | Nobody was named as authorized to trigger compensation/correction, unlike the explicit waiver-authorization pattern in §16. | §16: "Triggering a compensation or correction obligation (§36.5) follows the same pattern: a meta-policy-authorized authority, explicit scope, reason, and review obligation... Absent a more specific domain-module declaration, the authority that may trigger a correction is the same authority the policy names for waivers within that decision class." Reinforced by §12 item 25 (admission-time: bundles declaring a compensation/correction obligation must name the authority) and §36.5's closing sentence: "classification alone does not say who may act on it." | **Yes** | This is a concrete authority-naming *requirement*, explicitly parallel to the waiver pattern, with a stated default rather than an open classification scheme. It is also gated at admission (§12 item 25), so a bundle that declares a correction obligation without naming an authority is rejectable, not merely "should" language. |
| 5. Explanation audience tiering | §23 named Explanation content but not which audience may see what. | §23: "An Explanation profile declares one or more named audiences (for example subject/patient/customer, operator, auditor, and regulator) and, per audience, exactly which rule summary, reason codes, and evidence references that audience may receive... a single undifferentiated 'safe reason code' is not a complete Explanation profile." Tied explicitly to §36.11 ("Audience-tiered disclosure for the Explanation evidence profile (§23) is a specialization of this contract"). Also in Appendix A: "Explanation evidence declares its audience tiers and per-audience disclosure, not one undifferentiated reason code." | **Yes** | This requires *declaring* named audiences and per-audience content, and explicitly disqualifies the old single-tier approach ("is not a complete Explanation profile") — real normative force, not a passing mention that audiences might differ. |

## 3. Matrix row re-judgment

| Row | Still-Partial reason | New text | Resolved? | Reason |
| --- | --- | --- | --- | --- |
| Banking — "Reconciliation detects imbalance" | Retrospective controls existed, but correction authority and irreversible-effect semantics were undefined; specifically, no one was named as authorized to trigger a correction. | §16 + §36.5 (see Finding 4 above): named-authority requirement with a stated default (same authority as waivers for that decision class), gated at admission by §12 item 25. | **Yes — upgrades to Pass** | The irreversible/compensatable/reversible classification was already present pre-correction; the missing half — "who may trigger it" — is now answered with a concrete default and an admission gate, closing the row's stated gap directly. |
| Healthcare — "Patient requests explanation" | Explanation class existed, but audience-specific disclosure rules had no semantics. | §23 (see Finding 5 above): named audiences, per-audience content requirement, explicit rejection of an undifferentiated reason code, tied to §36.11's confidentiality contract. | **Yes — upgrades to Pass** | The row's exact complaint — "audience-specific disclosure rules need semantics" — is now met with a declared-audience-list-plus-per-audience-content requirement, which is checkable (a submitted Explanation profile either names audiences and per-audience content or it does not). |

## 4. Remaining issues (if any)

None found for the five findings and two matrix rows in scope for this
re-review. Two minor observations, neither of which is a finding requiring
correction under the stated PASS bar:

1. The first reviewer's report separately flagged a citation-accuracy
   problem in a "§7.1 gap-resolution traceability table" that does not
   correspond to any section actually numbered §7.1 in the current document
   (§7 is "Domain vocabulary and certified modules," with no §7.1
   subsection, and §2 is "Review disposition," which contains a different,
   unrelated correction list). That citation-accuracy question is outside
   the five findings and two matrix rows this task was scoped to (the task's
   §5 findings list did not include it as one of the five to re-check), so
   it is noted here for completeness rather than folded into the PASS/FAIL
   verdict above. If that table exists elsewhere or under a different
   heading, it was not located during this re-review.
2. §36.9's aggregate-consistency rejection code (`AggregateConsistencyUndeclared`)
   and §12 item 19's wording are consistent, but I did not independently
   re-verify all twelve F1-F12 items end-to-end in this pass (only the five
   named findings and the two matrix rows, per the task's scope) — a full
   re-audit of all twelve items was outside what was asked here and was
   already the first reviewer's completed work in §2 of their report.
