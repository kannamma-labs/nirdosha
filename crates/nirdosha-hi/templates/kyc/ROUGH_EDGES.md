# KYC Template — Known Rough Edges

## E1. Maker-checker enforcement
The same user must never be able to both create/submit and approve a KYC
case. The UI disables the action, but the real invariant must be enforced
by the service primitive.

## E2. Third-party vendor switching
A single KYC case may use one vendor for document OCR (e.g. Onfido) and
another for government ID (e.g. Aadhaar). The composer treats each
vendor module independently; runtime orchestration is still needed.

## E3. Screening disposition workflow
False-positive management (`TRUE_MATCH` / `FALSE_POSITIVE` /
`NO_RELATION`) needs a workflow primitive, not just a report screen.

## E4. Jurisdiction-aware document taxonomy
The country table only carries monetary primitives. Document types,
expiry windows and regulatory report formats are hard-coded in screens
and should move to per-jurisdiction primitive configuration.

## E5. Risk rule versioning
`risk_assessment.factor_breakdown` is JSON to accommodate frequent rule
changes. Schema drift must be tracked explicitly via `rule_version` for
audit.

## E6. Re-KYC scheduler
The expiry report screen is built. The date-driven / event-driven
scheduler primitive that flips cases to `EXPIRY_DUE` is not yet wired.
