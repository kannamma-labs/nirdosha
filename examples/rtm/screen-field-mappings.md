# Field-Level Specification — All ~150 Screens
**Format:** Per screen — Fields | Type | Validation. Buttons included as rows (Type = Btn).

---

## 📌 Global Conventions (apply everywhere unless overridden)

| Rule | Specification |
|---|---|
| **R** | Required field |
| Dates | `From ≤ To`; max range per screen noted |
| Text limits | Shown as `max N` |
| Files | Allowed: pdf, jpg, png, tif, docx, xlsx, eml · ≤25MB · virus-scan · max count per context |
| All writes | Audit-logged (user, timestamp UTC, before→after) |
| Critical actions (block payment, purge, SAR submit, rule go-live, delete) | Re-authentication (password/OTP) enforced |
| Maker–Checker | Config change ≠ self-approval; enforced system-side |
| PII | Masked per role; masked fields not exportable |
| Tipping-off | Restricted entities: banner + field suppression, access logged |
| Currency fields | Numeric ≥ 0, 2 decimals, currency dropdown unless stated |

---

## 🔧 Reusable Patterns (referenced below to avoid repetition)

| Pattern | Contents (all standard) |
|---|---|
| **P-QUEUE** | Filters row · table · column sort · pagination (25/50/100) · row-open action · bulk-select checkbox · Export (max rows/context; reason R) · Save view · auto-refresh optional |
| **P-DETAIL** | Header: ID, status badge, severity/priority, assignee, SLA countdown, timestamps · tab bar · right rail: linked entities + quick actions · footer: immutable audit tab |
| **P-UPLOAD** | File picker (type/size validation) · name (auto, editable, max 100) · classification dropdown R · description max 500 · upload/remove buttons · remove requires reason R |
| **P-APPROVE** | Item summary RO + diff vs current · decision Radio R (Approve/Reject/Return) · comments R if reject/return (min 20) · maker≠checker enforced · submit = immutable |
| **P-LOOKUP** | Search field (min 2 chars, fuzzy) · results table (name, ID, risk, flags) · select action · search logged |
| **P-NOTE** | Rich text R (max 4000) · type dropdown R · visibility dropdown R (team/case/restricted) · add/edit-own-30min/delete-TL-only |

---

# MODULE 1 — Authentication & Session

### 1.1 Login
| Element | Type | Validation / Notes |
|---|---|---|
| Username / Employee ID | Text | R; max 100; email or ID format |
| Password | Password | R; masked; max 64 |
| Remember this device | Checkbox | Optional |
| Login | Btn | On fail: generic error ("Invalid credentials" — never indicates which field); lockout after 5 fails (configurable) → 1.6 |
| SSO (SAML/OIDC) | Btn | Redirect; falls back to form on IdP failure |
| Forgot password | Link | → 1.3 |

### 1.2 MFA Challenge
| Element | Type | Validation / Notes |
|---|---|---|
| OTP code | Text | R; 6–8 digits; numeric only; valid 60s; single-use; no reuse across sessions |
| Approve / Deny (push) | Btn pair | Deny = session killed + security event logged |
| Trust this device (30d) | Checkbox | Optional |
| Verify | Btn | Max 3 attempts → lockout |
| Resend code | Btn | Rate-limited: 1 per 30s, max 3/hour |
| Use backup method | Link | → alternate factor |
| Cancel | Btn | → logout |

### 1.3 Forgot / Reset Password
| Element | Type | Validation / Notes |
|---|---|---|
| Username / email | Text | R; generic confirmation shown regardless of account existence (no enumeration) |
| Secondary identifier (Employee ID / DOB) | Text | R; must match record |
| Reset token | Hidden | From email; single-use; expires 15 min |
| New password | Password | R; policy: ≥12 chars, upper+lower+digit+special; ≠ last 5 passwords; must not contain username |
| Confirm password | Password | R; must match New |
| Submit | Btn | Invalidates all active sessions on success |

### 1.4 First-Login Setup
| Element | Type | Validation / Notes |
|---|---|---|
| Temporary password | Password | R; verified |
| New password / Confirm | Password | R; policy per 1.3 |
| Region / Language | Dropdown | R |
| Code of conduct acceptance | Checkbox | R; blocks Continue if unchecked |
| Full name (signature) | Text | R; fuzzy-must-match name on record |

### 1.5 Session Timeout / Re-auth
| Element | Type | Validation / Notes |
|---|---|---|
| Countdown timer | Display | RO; warning at T-60s (configurable) |
| Stay signed in | Btn | Triggers re-auth (password) for sensitive roles |
| Sign out | Btn | Immediate |
| Password (re-auth) | Password | R |

### 1.6 Access Denied / Locked
| Element | Type | Validation / Notes |
|---|---|---|
| Reason message | Display | RO (locked / no role / no jurisdiction) |
| Justification (unlock request) | Text area | R; min 20; max 500; routes to ADM queue |
| Request access | Btn | Rate-limited 1/hour |
| Back to login | Btn | — |

---

# MODULE 2 — Dashboards

### 2.1 Global Navigation Shell
| Element | Type | Validation / Notes |
|---|---|---|
| Global search | Text | Min 2 chars; searches entities/alerts/cases/txns; results grouped by type; PII-masked per role |
| Entity/jurisdiction switcher | Dropdown | R; options role-scoped; switch resets session context |
| Environment badge | Display | RO; PROD in red |
| Notification bell | Btn | Count badge; → 16.1 |
| Profile menu | Dropdown | → 22.1, logout |
| Nav tree | Links | Rendered per RBAC (no hidden-screen access via URL — enforced server-side) |

### 2.2 L1 "My Day" Dashboard
| Element | Type | Validation / Notes |
|---|---|---|
| Date range | Date range | Default: today; max 90 days |
| Team filter | Dropdown | Own team only unless TL |
| Widgets (My pending / SLA at-risk / breached / closed today / avg handle time) | Display | RO; auto-refresh 60s |
| Go to My Queue | Btn | → 3.1 pre-filtered assignee=me |
| Go to Escalations | Btn | → 3.1 status=escalated |

### 2.3 Team Dashboard (TL)
| Element | Type | Validation / Notes |
|---|---|---|
| Team selector | Dropdown | R |
| Date range | Date range | Max 90 days |
| Analyst filter | Dropdown | Optional |
| Widgets (workload heat map, SLA-risk funnel, aging buckets, reassignment rate) | Display | RO |
| Reassign work | Btn | → 3.5/3.6 with selection |
| Export dashboard | Btn | PNG/PDF |

### 2.4 MLRO Dashboard
| Element | Type | Validation / Notes |
|---|---|---|
| Jurisdiction | Dropdown | R |
| Period | Dropdown + custom | MTD/QTD/YTD/custom; custom: From<To, max 1 year |
| Widgets (SAR pipeline, conversion, aging, statutory deadline countdown) | Display | RO |
| Drill-down | Click | Any widget → underlying filtered queue |
| Export board pack | Btn | PDF |

### 2.5 Real-Time Monitoring Wall (OPS)
| Element | Type | Validation / Notes |
|---|---|---|
| Severity / Scenario / Geography filters | Multi-select | Optional |
| Auto-refresh interval | Dropdown | 5s/10s/30s/60s; min 5s (perf guard) |
| Pause ticker / Resume | Btn pair | — |
| Full-screen toggle | Btn | — |
| Sound on new Critical | Toggle | — |
| Quick-acknowledge | Btn | Per alert row; reason R; OPS only |

### 2.6 Executive KPI Dashboard
| Element | Type | Validation / Notes |
|---|---|---|
| Period / Comparison period | Dropdowns | R; comparison = prior period or prior year |
| Business line | Multi-select | Optional |
| Export | Btn | PDF/PPT |
| Schedule email | Btn | → recipients R (valid emails); frequency R |

### 2.7 Saved Views Manager
| Element | Type | Validation / Notes |
|---|---|---|
| View name | Text | R; max 50; unique per user |
| Description | Text | max 250 |
| Share with | Dropdown | Private / Team / Organization; Org-level requires TL approval |
| Set as default | Radio | One default per user per screen |
| Save / Update / Delete | Btns | Delete: confirm modal; shared views deletable by owner or TL only |

---

# MODULE 3 — Alert Management

### 3.1 Alert Work Queue *(P-QUEUE)*
| Element | Type | Validation / Notes |
|---|---|---|
| Alert ID | Text | Numeric |
| Customer name/ID | Text | Min 2 |
| Scenario | Multi-select | From 8.1 live rules |
| Severity | Multi-select | Critical/High/Medium/Low |
| Status | Multi-select | New/In Progress/Pending Info/Escalated/Closed |
| Assigned to | Dropdown | Me/My team/Unassigned/All |
| SLA state | Dropdown | Breached/At-risk/OK |
| Trigger date range | Date range | Max 1 year |
| Amount range | Numeric ×2 | Min ≤ Max; ≥ 0 |
| Columns | Display | ID · trigger time · scenario · customer · amount+ccy · severity · SLA countdown (color-coded) · status · assignee |
| Quick close (row) | Btn | → 3.3 |
| Export | Btn | Max 10k rows; reason R |

### 3.2 Alert Detail *(P-DETAIL)*
| Element | Type | Validation / Notes |
|---|---|---|
| Header fields (ID, scenario + version, trigger time, severity, status, assignee, SLA countdown) | Display | RO |
| Trigger explanation panel | Display | RO; condition/threshold/actual per rule condition |
| Customer snapshot | Display | RO; KYC status, risk rating + delta, PEP/sanctions flags, tenure |
| Triggering transactions strip | Display | RO rows; click → 6.2 |
| Tabs | Nav | Related · Txn history (date filter: default 90d, max 24m) · Counterparties · Network (→7.1) · Documents (P-UPLOAD, max 20 files) · Notes (P-NOTE) · Audit (RO) |
| Close as FP | Btn | → 3.3 |
| Escalate to Case | Btn | → 3.8; blocked if alert already linked to case |
| Reassign | Btn | → 3.6 |
| Request Info | Btn | → 3.7 |
| Link to existing case | Btn | Case ID field R; must exist + be open; same subject or linked entity required |
| Add Note | Btn | P-NOTE |
| Tipping-off banner | Display | Shown if subject restricted; content suppressed |

### 3.3 Disposition Panel
| Element | Type | Validation / Notes |
|---|---|---|
| Disposition reason code | Dropdown | R; from 18.5 taxonomy |
| Sub-reason | Dropdown | R; dependent options |
| Detailed rationale | Text area | R; min 100; max 2000 |
| Evidence checklist (profile reviewed / txn history reviewed / related alerts checked) | Checkboxes | R — all mandatory |
| Close category | Radio | False Positive / Duplicate (alert ID R; must exist; must be same customer) / Known-accepted (whitelist ref R) / Customer explanation accepted |
| Supervisor attestation (severity=Critical) | Checkbox | R if Critical; triggers TL approval task before close completes |
| Submit | Btn | Validates all; locks alert; irreversible (reopen = TL, reason R) |
| Save draft / Cancel | Btns | Cancel: confirm discard |

### 3.4 Related & Linked Alerts
| Element | Type | Validation / Notes |
|---|---|---|
| Related alerts table | Display | RO: ID, status, assignee, disposition |
| Merge | Btn | Select ≥2; same customer enforced; blocked if any is Escalated/Closed-suspicious; confirm modal |
| Mark duplicate of | Btn | Primary alert ID R; must be open |
| Unlink | Btn | Reason R |

### 3.5 Bulk Actions
| Element | Type | Validation / Notes |
|---|---|---|
| Selection count | Display | RO; max 500 per operation |
| Action | Dropdown | R: Assign / Close / Reprioritize / Tag |
| New assignee | Dropdown | R if Assign |
| New priority | Dropdown | R if Reprioritize; upgrade requires TL |
| Reason code + rationale | Dropdown + Text | R if Close; rationale min 100; single rationale applies to all (attestation checkbox R) |
| Tags | Multi + free text | Max 10; each max 30 chars |
| Confirm | Btn | Impact summary modal; re-auth if >100 items |

### 3.6 Reassignment / Pickup
| Element | Type | Validation / Notes |
|---|---|---|
| Assign to | Radio | Me / Analyst (lookup R) / Team (dropdown R) |
| Reason | Dropdown | R: workload / skill match / leave cover / other (text R if other) |
| Comments | Text | max 500 |
| Notify assignee | Checkbox | Default checked |
| Confirm / Cancel | Btns | — |

### 3.7 Pending Info (RFI)
| Element | Type | Validation / Notes |
|---|---|---|
| Requested from | Dropdown | R: branch / business unit / external party |
| Information needed | Text area | R; min 20; max 2000 |
| Response deadline | Date | R; future; default +5 business days; max +30 |
| Reminder frequency | Dropdown | None / daily / on-due-date |
| Send request | Btn | Status → Pending Info; SLA clock paused (config flag) |
| Cancel RFI | Btn | Reason R; SLA resumes |
| Convert to escalation | Btn | → 3.8 |

### 3.8 Escalation Form
| Element | Type | Validation / Notes |
|---|---|---|
| Escalation type | Dropdown | R: suspected ML / suspected fraud / mixed / other |
| Summary of suspicion | Text area | R; min 200; max 5000 |
| Key evidence checklist | Checkboxes | R — all: txns reviewed / profile reviewed / prior alerts referenced |
| Linked alerts | Display + search | Auto RO list; add via lookup (same customer) |
| Recommended priority / case type | Dropdowns | R |
| Attachments | Upload | Optional (P-UPLOAD) |
| Submit | Btn | Creates case draft (stage=Triage); alert locked read-only |

### 3.9 SLA & Aging Monitor (TL)
| Element | Type | Validation / Notes |
|---|---|---|
| Filters: team / severity / SLA state / scenario | Standard | — |
| Aging buckets | Display | RO: <25%, 25–50%, 50–75%, 75–100%, breached |
| Auto-escalate on breach | Toggle | Per severity; confirm modal |
| Reassign breached | Btn | → bulk 3.5 |
| Export | Btn | — |

### 3.10 Alert Audit History
| Element | Type | Validation / Notes |
|---|---|---|
| Filters: user / action type / date range | Standard | Max 90 days per query |
| Table | Display | RO: timestamp UTC · user · action · before → after |
| Export | Btn | AUD/MLRO role |

---

# MODULE 4 — Case Management

### 4.1 Case Queue *(P-QUEUE)*
| Element | Type | Validation / Notes |
|---|---|---|
| Filters: ID / subject / type / priority / stage / assignee / age / opened range | Standard | Date max 1 year |
| Columns | Display | + "Linked SAR" flag column (restricted visibility) |
| New case (manual) | Btn | L2+ only; justification R min 200; auto-case link optional |
| Export | Btn | Max 5k |

### 4.2 Case Overview
| Element | Type | Validation / Notes |
|---|---|---|
| ID / subject / type / priority / stage / opened by+date / SLA timer | Display | RO |
| Linked alerts count · total suspicious value · SAR status | Display | RO |
| Stage stepper | Display | RO: Triage → Investigation → Review → Decision |
| Open workspace | Btn | → 4.3 |
| Reassign / Change priority | Btns | Priority ↑ requires TL approval (P-APPROVE) |

### 4.3 Investigation Workspace *(P-DETAIL)*
| Element | Type | Validation / Notes |
|---|---|---|
| Tabs | Nav | Alerts · Transactions · Customer 360 · Network · Documents · Tasks · Notes · SAR · Disposition |
| Evidence pinboard | List | Add (from any tab) / drag-reorder / remove (confirm); max 200 items |
| Advance stage | Btn | Validation: stage checklist complete (e.g., Investigation→Review requires disposition draft + evidence min 3) |
| Quick actions rail | Btns | Add note / task / doc · Escalate MLRO · Request 4-eyes |

### 4.4 Linked Alerts Tab
| Element | Type | Validation / Notes |
|---|---|---|
| Alerts table | Display | RO + L1 disposition + dispositioner + date |
| Add alert link | Btn | Lookup; must be same customer or shared counterparty |
| Unlink | Btn | L2+; reason R |
| Promote alert to evidence | Btn | Copies to pinboard |

### 4.5 Case Transactions Tab
| Element | Type | Validation / Notes |
|---|---|---|
| Filters: date range / amount / direction / counterparty / channel | Standard | Default: case period ±30d; max 5 years |
| In-scope checkbox (rows) | Checkbox | Bulk add requires ≥1 selected |
| Remove from scope | Btn | Reason R |
| Annotate txn | Btn | Text; max 500 per txn |
| Add txn manually | Btn | Embeds 6.1 search; must belong to subject/linked entity |
| Export scoped txns | Btn | >5k rows needs TL approval |

### 4.6 Documents & Evidence *(P-UPLOAD)*
| Element | Type | Validation / Notes |
|---|---|---|
| Source | Dropdown | R: customer / internal system / external / branch |
| Version upload | Btn | Creates version chain (RO history) |
| Delete | Btn | Soft delete; reason R; AUD-visible |

### 4.7 Notes & Activity *(P-NOTE)*
| Element | Type | Validation / Notes |
|---|---|---|
| Note type | Dropdown | R: observation / customer contact / internal consult / decision rationale |
| Pin to top | Checkbox | — |

### 4.8 Tasks & Checklist
| Element | Type | Validation / Notes |
|---|---|---|
| Checklist template | Dropdown | R on init (per case type) |
| Task name | Text | R; max 200 |
| Assignee | Dropdown | R |
| Due date | Date | R; ≥ today; warn if > case SLA |
| Priority | Dropdown | R |
| Status | Dropdown | Not started/In progress/Done/Blocked; Blocked requires reason R |
| Complete | Btn | Completion note R for checklist-critical tasks |
| Delete task | Btn | TL only; reason R |

### 4.9 Internal RFI
| Element | Type | Validation / Notes |
|---|---|---|
| To (unit/branch) | Dropdown | R |
| Subject | Text | R; max 150 |
| Questions | Text area | R; min 20; max 3000 |
| Deadline | Date | R; future |
| Send / Cancel / Escalate non-response | Btns | Cancel: reason R; Escalate enabled only after deadline passed |

### 4.10 Disposition & Closure
| Element | Type | Validation / Notes |
|---|---|---|
| Final determination | Radio | R: Suspicious–substantiated / Not suspicious / Unable to substantiate |
| Determination rationale | Text area | R; min 300 |
| Reason codes | Multi-select | R; min 1 |
| Scope attestation ("all in-scope txns reviewed") | Checkbox | R |
| No-SAR justification | Text | R min 200 if determination = Not suspicious |
| Submit for review | Btn | → 4.11; blocked if mandatory tasks incomplete |
| Save draft | Btn | — |

### 4.11 Four-Eyes Review
| Element | Type | Validation / Notes |
|---|---|---|
| Analyst conclusion | Display | RO |
| Independent-review checklist (evidence / scope / rationale) | Checkboxes | R — all |
| Decision | Radio | R: Approve / Return to analyst / Escalate MLRO |
| Return reason | Text | R if Return; min 50 |
| Reviewer note | Text | max 1000 |
| Submit | Btn | **Self-review blocked** (reviewer ≠ analyst; TL+ role enforced) |

### 4.12 Escalation to MLRO
| Element | Type | Validation / Notes |
|---|---|---|
| Escalation memo | Text area | R; min 200; max 5000 |
| Recommendation | Radio | R: File SAR / Do not file / Continue monitoring |
| Bundle completeness attestation | Checkbox | R |
| Urgency | Dropdown | R; if Urgent → phone-call-made attestation checkbox R |
| Submit | Btn | Notifies MLRO; locks case edits |

### 4.13 Merge / Link Cases
| Element | Type | Validation / Notes |
|---|---|---|
| Target case ID | Text | R; must exist + open |
| Direction | Radio | This→target / Target→this |
| Pre-merge check | Display | Auto: same subject or networked entity required; **blocks** if both have final dispositions (conflict) |
| Reason | Text | R; min 50 |
| Relationship type (link-only) | Dropdown | R: same network / same subject / related parties |
| Merge | Btn | Confirm modal; irreversible; re-auth |
| Link / Cancel | Btns | — |

### 4.14 Initiate SAR
| Element | Type | Validation / Notes |
|---|---|---|
| Confirmation dialog | Modal | Case must be ≥ Review stage |
| Subject prefill preview | Display | RO |
| Create SAR draft / Cancel | Btns | → 12.2 prefilled |

---

# MODULE 5 — Entity Intelligence

### 5.1 Global Entity Search *(P-LOOKUP)*
| Element | Type | Validation / Notes |
|---|---|---|
| Search type | Radio | Customer / Account / Counterparty / Txn / Alert / Case |
| Search term | Text | R; min 2; wildcard `*` allowed; exact-vs-fuzzy toggle |
| Contextual filters (DOB, nationality, doc no., phone) | Text | Optional; shown per type |
| Scope | Dropdown | All / selected jurisdiction; role-scoped |
| Search | Btn | Rate-limited; searches logged (tipping-off) |

### 5.2 Customer 360
| Element | Type | Validation / Notes |
|---|---|---|
| Identity panel (name, DOB, nationality, ID type/no/expiry, addresses, contacts, occupation, employer, onboarding channel/date) | Display | RO; masked per role |
| Risk panel (score, band, factor breakdown, PEP category, sanctions status, adverse-media count, next review) | Display | RO |
| Activity charts (12m in/out, count, cash %, cross-border %, channel mix) | Display | RO; period selector 3/6/12m |
| Relationships (accounts — masked numbers, related parties, UBO link, shared-attributes link) | Display | RO |
| History (alerts by status, cases, SAR-existence flag — content suppressed) | Display | RO |
| Add internal note | Btn | P-NOTE |
| Set watch flag | Btn | Flag type R; severity R; → 5.9 |
| Raise restriction recommendation | Btn | → 5.10; L2+ |
| Export profile (PDF) | Btn | Watermarked; logged; reason R |

### 5.3 KYC Document Viewer
| Element | Type | Validation / Notes |
|---|---|---|
| Doc list (type, date, expiry, status) | Display | RO; expiry status: valid/expiring(≤30d)/expired |
| Viewer (zoom, rotate) | Display | RO |
| Review checklist (authenticity / data-matches-profile) | Checkboxes | R before "mark reviewed" enabled |
| Mark reviewed | Btn | Requires checklist complete; reviewer ≠ doc uploader |
| Request renewed doc | Btn | Reason R; creates task |
| Download all | Btn | Zip; max 50MB; logged |

### 5.4 Risk Score Breakdown
| Element | Type | Validation / Notes |
|---|---|---|
| Score / band / factor table (weight, contribution, source, last update) | Display | RO |
| Score history | Chart | RO |
| Simulate | Btn | RMG only; preview only — never persists |
| Trigger re-rating | Btn | RMG/ADM; justification R |

### 5.5 Expected vs Actual
| Element | Type | Validation / Notes |
|---|---|---|
| Declared profile block | Display | RO |
| Period selector | Dropdown | R: 3/6/12m |
| Actual metrics + deviation meters | Display | RO |
| Explanation note | Text | max 1000 |
| Export / Add to case evidence | Btns | — |

### 5.6 Related Parties / UBO
| Element | Type | Validation / Notes |
|---|---|---|
| Ownership tree | Display | RO; expand/collapse; % ownership + control type |
| Add relationship | Btn | Entity lookup R + relationship type dropdown R + start date R + info-source dropdown R + supporting doc (P-UPLOAD) |
| Verify | Btn | Verification method dropdown R + reference no. R |
| Edit / End relationship | Btn | Justification R |

### 5.7 Adverse Media
| Element | Type | Validation / Notes |
|---|---|---|
| Results table (headline, source, date, sentiment, score, status) | Display | — |
| Relevance determination | Radio | R: Relevant / Not relevant |
| Rationale | Text | R if Relevant; min 50 |
| Re-run search | Btn | Source scope multi-select; date range |
| Add to case / Export | Btns | — |

### 5.8 Entity Alert/Case History
| Element | Type | Validation / Notes |
|---|---|---|
| Filters: type / date / status | Standard | — |
| Timeline toggle | Btn | — |
| Open item / Export | Btns | SAR entries show flag only |

### 5.9 Internal Risk Notes/Flags
| Element | Type | Validation / Notes |
|---|---|---|
| Note text | Text area | R; max 2000 |
| Flag type / Severity | Dropdowns | R |
| Review date | Date | Optional; future only |
| Add / Resolve | Btns | Resolve: resolution note R |

### 5.10 Restriction/Exit Recommendation
| Element | Type | Validation / Notes |
|---|---|---|
| Recommendation type | Dropdown | R: txn restriction / channel restriction / freeze proposal / exit |
| Scope | Multi-select | R: accounts, products, channels |
| Justification | Text area | R; min 300 |
| Supporting cases | Lookup | R; ≥1 |
| Legal review required | Checkbox | Reroutes workflow |
| Submit / Withdraw | Btns | Withdraw: reason R |

---

# MODULE 6 — Transaction Intelligence

### 6.1 Transaction Search
| Element | Type | Validation / Notes |
|---|---|---|
| Txn ID / external ref | Text | — |
| Date range | Date range | R; default today; max 5 years |
| Amount min/max + currency | Numeric + multi | Min ≤ Max |
| Direction | Dropdown | In/Out/Both |
| Customer / account / counterparty | Text | Lookup-supported |
| Channel | Multi-select | Branch/online/mobile/ATM/POS/wire/SWIFT |
| Geography | Multi-select | — |
| Narrative keyword | Text | Min 3; wildcard |
| Status | Multi-select | Completed/pending/held/rejected/reversed |
| Alert-linked | Dropdown | Any / linked / unlinked |
| Search / Clear / Save query / Export | Btns | Export max 50k; reason R; raw PII export needs approval |

### 6.2 Transaction Detail
| Element | Type | Validation / Notes |
|---|---|---|
| Full record (ID, refs, booking/value date, amount+ccy, FX rate+equivalent, direction, channel, originator block, beneficiary block, narrative, purpose code, rail/Swift type, charges, status+history, device ID, IP, geo, session, risk score at txn time, screening result at txn time, linked alert/case) | Display | RO |
| Add to case | Btn | Case lookup R; must be open |
| Create manual alert | Btn | L2+; justification R min 100 |
| Flag anomaly | Btn | Reason R |
| View counterparty / ledger | Btns | Tipping-off aware |
| Export PDF | Btn | Watermarked; logged |

### 6.3 Ledger View
| Element | Type | Validation / Notes |
|---|---|---|
| Account selector | Lookup | R |
| Period | Date range | R; max 12 months |
| Filters: min amount / type / flagged-only | Standard | — |
| Table (date, narrative, in, out, running balance, flags) | Display | RO |
| Export / Open txn | Btns | — |

### 6.4 Funds Flow Visualization
| Element | Type | Validation / Notes |
|---|---|---|
| Root entity | Display/Lookup | R (from context) |
| Depth | Dropdown 1–5 | Default 2; >3 requires confirm (perf) |
| Time window | Date range | R |
| Min amount | Numeric | Default 0 |
| Direction | Dropdown | Both/in/out |
| Expand / Focus / Hide node | Btns | — |
| Snapshot to evidence / Export image / Export edge CSV | Btns | CSV export logged |

### 6.5 Counterparty Analysis
| Element | Type | Validation / Notes |
|---|---|---|
| Subject + period | Display/Dropdown | Period R |
| Table (counterparty, count, total in, total out, first/last, new-payee flag, risk flags) | Display | RO; sortable |
| Add to case / Open profile / Export | Btns | — |

### 6.6 Pattern/Typology Match Viewer
| Element | Type | Validation / Notes |
|---|---|---|
| Matched scenarios table (scenario, version, match %, conditions) | Display | RO |
| Behavior metrics (velocity, round-amount count, near-threshold count, hour distribution) | Display | RO |
| Export / Add to evidence | Btns | — |

### 6.7 Structuring Visual
| Element | Type | Validation / Notes |
|---|---|---|
| Customer + period | Display/Dropdown | Period R |
| Deposit distribution chart (bins around threshold) | Display | RO |
| Branch/channel breakdown | Table | RO |
| Multi-entity aggregation toggle | Toggle | Household/business roll-up |
| Export / Add to evidence | Btns | — |

### 6.8 Device & Session Intelligence
| Element | Type | Validation / Notes |
|---|---|---|
| Device ID / fingerprint / first-last seen | Display | RO |
| IP history table (IP, geo, ISP, dates, VPN/proxy flag) | Display | RO |
| Session list · impossible-travel map | Display | RO |
| Shared-devices table | Display | RO |
| Blacklist device | Btn | OPS/ADM; reason R; confirm modal |

---

# MODULE 7 — Network Analysis

### 7.1 Interactive Entity Graph
| Element | Type | Validation / Notes |
|---|---|---|
| Seed entity | Display/Lookup | R |
| Depth | Dropdown 1–4 | Default 2 |
| Edge types | Multi-select | Txn / shared device / address / phone / ownership |
| Date range | Date range | R |
| Min edge amount | Numeric | — |
| Layout | Dropdown | Force / hierarchical / chronological |
| Node/edge click panel | Display | RO summary |
| Expand / Collapse / Hide / Pin | Btns | — |
| Save to case / Export image / Export data | Btns | Data export reason R |

### 7.2 Path Finder
| Element | Type | Validation / Notes |
|---|---|---|
| From / To entity | Lookup ×2 | R; From ≠ To enforced |
| Max degrees | Dropdown 1–6 | Default 3 |
| Edge types | Multi-select | — |
| Find paths | Btn | Async + progress; results: path list + exposure totals; select → highlight |

### 7.3 Cluster Detection
| Element | Type | Validation / Notes |
|---|---|---|
| Algorithm | Dropdown | R: community / connected components |
| Min cluster size | Numeric | ≥2; default 3 |
| Density threshold | Numeric | 0–1 |
| Scope | Dropdown | Case entities / portfolio / all (beyond-case scope requires RMG/TL approval) |
| Run | Btn | Async job; notification on completion |
| Results table (cluster ID, size, volume, risk, members) | Display | RO |
| Open in graph / Save / Export | Btns | — |

### 7.4 Shared Attributes
| Element | Type | Validation / Notes |
|---|---|---|
| Attribute types | Multi-select | R: device/IP/address/phone/email/employer/beneficiary |
| Results (value, entity count, entity list, first/last seen) | Display | RO |
| Investigate set / Export | Btns | Creates case link |

### 7.5 Graph Snapshot Export
| Element | Type | Validation / Notes |
|---|---|---|
| Title | Text | R; max 100 |
| Description | Text | max 500 |
| Include data table | Checkbox | — |
| Classification | Dropdown | R |
| Generate | Btn | Watermarked; logged; versioned to evidence |

---

# MODULE 8 — Scenario & Rule Management

### 8.1 Scenario Library
| Element | Type | Validation / Notes |
|---|---|---|
| Filters: status / typology / owner / channel / last-tuned | Standard | — |
| Table (ID, name, typology, status, version, owner, alerts 30d, FP%, last change) | Display | — |
| New / Clone / View | Btns | Clone: name auto "– COPY"; must rename |
| Retire | Btn | Reason R; confirm; shows alert-volume impact |

### 8.2 Rule Builder ⭐
| Element | Type | Validation / Notes |
|---|---|---|
| Name | Text | R; unique; max 100 |
| Description / Business rationale | Text | R; rationale min 50 |
| Typology mapping | Multi-select | R (FATF/FFIEC codes) |
| Aggregation function | Dropdown | R: sum / count / avg / velocity / stddev-deviation |
| Field | Dropdown | R (amount, txn count…) |
| Lookback window | Numeric + unit dropdown | R; 1–365; days/hours; sliding vs calendar toggle |
| Grouping keys | Multi-select | R: customer / account / counterparty / device (≥1) |
| Condition rows (field, operator, value) | Row builder | Operator: >/</=/between/in; value R; empty rows blocked; AND/OR combinator |
| Threshold | Numeric OR percentile | Static R numeric OR percentile 50–99.9 OR adaptive toggle (model-backed → extra approval) |
| Scope: segment / product / channel / geography / currency | Multi-selects | ≥1 each; FX-normalization toggle R |
| Severity / weight | Dropdown / numeric | R / 1–10 |
| Dedup suppression window | Numeric hours | 0–720 |
| Run backtest | Btn | **Must run before submit** (enforced) |
| Save draft / Validate / Submit for approval | Btns | Submit checks: all R complete + test executed + overlap-with-live-rule warning |

### 8.3 Threshold & Parameter Editor
| Element | Type | Validation / Notes |
|---|---|---|
| Scenario selector | Lookup | R |
| Current params | Display | RO |
| New threshold | Numeric | R; metric-specific range |
| Segment override rows (segment, value) | Row builder | Segment R, value R |
| Justification | Text | R; min 100 |
| Effective date | Date | R; ≥ today |
| Simulation attestation | Checkbox | R — cannot submit prod change without running 8.6 first |
| Submit | Btn | → P-APPROVE |

### 8.4 Segmentation & Scope Config
| Element | Type | Validation / Notes |
|---|---|---|
| Include / Exclude attribute builders | Row builders | Attributes: segment, product, channel, geo, customer type, tenure |
| Preview impact | Btn | Async; shows affected-customer count |
| Save / Submit | Btns | — |

### 8.5 Suppression/Dedup Logic
| Element | Type | Validation / Notes |
|---|---|---|
| Scenario | Lookup | R |
| Suppression window | Numeric hours | R; 0–720 |
| Dedup keys | Multi-select | R: customer+scenario / customer+counterparty / account+scenario |
| Family dedup (parent/child) | Toggle | — |
| VIP exception handling | Dropdown | R: allow / suppress / flag |
| Justification | Text | R; min 50 |
| Save / Submit | Btns | — |

### 8.6 Simulation & Backtesting
| Element | Type | Validation / Notes |
|---|---|---|
| Scenario | Lookup | R; draft or live |
| Historical period | Date range | R; min 30 days; max 24 months |
| Data scope | Dropdown | All / 10% sample / segment |
| Run | Btn | Async; queue position shown |
| Results (hits, unique customers, projected daily alerts, FP estimate, workload/FTE impact, sensitivity curve) | Display | RO |
| Re-run / Promote to tuning proposal / Export | Btns | Promote → creates 8.3 draft |

### 8.7 Threshold Sensitivity ("What-if")
| Element | Type | Validation / Notes |
|---|---|---|
| Scenario | Lookup | R |
| Threshold range | Min/Max/Step numerics | R; max 1,000 points |
| Curves (volume vs threshold, FP%, precision) | Display | RO |
| Selected threshold slider | Numeric | — |
| Generate / Apply (→8.3 draft) / Export chart | Btns | — |

### 8.8 Approval Workflow *(P-APPROVE)*
| Element | Type | Validation / Notes |
|---|---|---|
| Diff vs current | Display | RO |
| Checklist (test evidence attached / impact assessed / maker≠checker) | Checkboxes | R — all |
| Go-live date (if approve) | Date | R; ≥ today + cooling period (configurable, default 1 day) |

### 8.9 Version History & Diff
| Element | Type | Validation / Notes |
|---|---|---|
| Version list (version, author, date, change note) | Display | RO |
| Compare selection | 2 pickers | Must be distinct versions |
| Diff view | Display | RO side-by-side |
| Rollback | Btn | RMG/TL; reason R; routed through 8.8 |

### 8.10 Scheduling & Go-Live
| Element | Type | Validation / Notes |
|---|---|---|
| Approved change ref | Display | RO |
| Go-live datetime | DateTime | R; ≥ now + cooling |
| Rollout strategy | Radio | All-at-once / Pilot cohort (cohort selector R + % 1–100 R) / Staged by segment |
| Rollback plan | Text | R; min 50 |
| Monitoring checklist (day-1 volume check, FP sample check) | Checkboxes | R |
| Schedule / Cancel schedule | Btns | Cancel: reason R |

### 8.11 Whitelist/Allowlist
| Element | Type | Validation / Notes |
|---|---|---|
| Entry type | Dropdown | R: customer / counterparty / scenario exemption |
| Entity | Lookup | R |
| Scoped scenarios | Multi-select | R |
| Rationale | Text | R; min 100 |
| Evidence | Upload | R; ≥1 file |
| Expiry | Date | R; max +12 months; renewal reminder at T-30d |
| Submit / Renew / Revoke | Btns | Maker≠checker; Revoke: reason R, immediate, audited |

### 8.12 Scenario Performance
| Element | Type | Validation / Notes |
|---|---|---|
| Filters: period (default 30d) / typology / owner | Standard | — |
| Table (scenario, alerts, FP%, escalation%, SAR conversion%, avg handle time, trend) | Display | FP% >90 auto-flagged red |
| Export | Btn | — |
| Create tuning task | Btn | Assignee R + due date R |

---

# MODULE 9 — ML Model Management

### 9.1 Model Registry
| Element | Type | Validation / Notes |
|---|---|---|
| Table (model ID, name, type, version, status, owner, trained date, next retrain) | Display | — |
| Register model | Btn | Name R unique / type R / description R / owner R / docs upload R |
| Promote status | Btn | Shadow→Challenger→Active; via approval |
| Retire | Btn | Reason R |

### 9.2 Performance Monitor
| Element | Type | Validation / Notes |
|---|---|---|
| Model + period selectors | Dropdowns | R |
| Metrics (AUC, precision, recall, F1, volume, FP%, SAR conversion, score histogram) | Display | RO |
| Slice by | Dropdowns | Segment/channel/date |
| Export / Flag issue | Btns | Flag issue: description R min 50 + severity R |

### 9.3 Drift Monitoring
| Element | Type | Validation / Notes |
|---|---|---|
| Model selector | Lookup | R |
| Feature table (feature, PSI, status, trend) | Display | RO; PSI >0.2 flagged |
| PSI alert threshold | Numeric | Default 0.2; editable via approval |
| Trigger investigation task | Btn | Assignee R |

### 9.4 Champion/Challenger
| Element | Type | Validation / Notes |
|---|---|---|
| Champion | Display | RO |
| Challenger selector | Lookup | R; must be in Shadow status |
| Comparison period | Date range | R |
| Side-by-side metrics + score overlap | Display | RO |
| Promote challenger | Btn | → approval; rollout plan text R |
| Extend trial / Discard | Btns | Discard: reason R |

### 9.5 Validation & Governance
| Element | Type | Validation / Notes |
|---|---|---|
| Validation checklist (concept doc, data lineage, perf test, bias test, stability) | Checkboxes | RO state + doc upload R per open item |
| Validator assignment | Lookup | R; **must ≠ model owner** (enforced) |
| Review calendar | Display | RO; next review date |
| Submit for sign-off / Record sign-off | Btns | Sign-off: signer R + date R + notes R |
| Export model pack | Btn | — |

### 9.6 Score Explainability Viewer
| Element | Type | Validation / Notes |
|---|---|---|
| Context alert | Display | RO |
| Score + band | Display | RO |
| Factor contribution table (feature, value, ± contribution) | Display | RO |
| Counterfactual panel | Display | RO |
| Include in disposition / Export | Btns | Auto-attaches to 3.3 |

---

# MODULE 10 — Watchlist & Screening

### 10.1 Screening Hit Queue *(P-QUEUE)*
| Element | Type | Validation / Notes |
|---|---|---|
| Filters: list source / score range / date / status / assignee | Standard | — |
| Columns (hit ID, entity name, list, score, type, status, SLA) | Display | — |
| Bulk assign | Btn | → 3.6 |

### 10.2 Hit Review & Disposition
| Element | Type | Validation / Notes |
|---|---|---|
| Submitted name / score / algorithm / attribute-match table (name, DOB, nationality side-by-side) | Display | RO |
| List entry detail (aliases, source doc) | Display | RO |
| Disposition | Radio | R: True match / False match / Potential – needs EDD |
| Rationale | Text | R; min 100 |
| Evidence | Upload | R if True match |
| Escalate to CO | Btn | Auto-triggered on True match |
| Submit | Btn | If payment context → feeds 11.2 hold decision |

### 10.3 Screening Configuration
| Element | Type | Validation / Notes |
|---|---|---|
| Lists enabled | Multi-select | R |
| Match threshold per list | Numeric 0–100 | R; enforced 60–100; default 85 |
| Algorithm | Dropdown | R; fuzzy/phonetic/transliteration toggles |
| Name-order handling | Dropdown | R |
| Secondary attribute weights | Numeric ×N | **Must sum to 100** (validated) |
| Rescreen-on-update | Toggle | R |
| Save | Btn | ADM + approval workflow |

### 10.4 Watchlist Management
| Element | Type | Validation / Notes |
|---|---|---|
| List table (name, source, version, loaded, record count, next update) | Display | — |
| Upload list | File | R; schema-validated; record-count delta >20% → confirm modal |
| Internal entry: name / DOB / ID / category / reason | Fields | Name, category R; reason R min 100; expiry optional |
| Effective date | Date | R |
| Deactivate list | Btn | Reason R; coverage-gap warning modal |

### 10.5 Rescreening Monitor
| Element | Type | Validation / Notes |
|---|---|---|
| Jobs table (job ID, trigger, scope, records, progress, status) | Display | — |
| Delta-rescreen config | Toggle + scope | R |
| Run manual rescreen | Btn | Scope R; full-book run → confirm with est. time |
| Retry failed | Btn | Error detail shown |

### 10.6 Historical Rescreen Campaigns
| Element | Type | Validation / Notes |
|---|---|---|
| New campaign: name / criteria / scope / priority / schedule | Fields | All R |
| Campaign table (progress, hits, pending dispositions) | Display | RO |
| Create / Pause / Resume / Cancel | Btns | Create needs approval; Cancel: reason R |

---

# MODULE 11 — Real-Time Intervention

### 11.1 Interception Queue *(P-QUEUE)*
| Element | Type | Validation / Notes |
|---|---|---|
| Columns (hold ID, payment ref, amount, initiator, beneficiary, hold reason, **countdown timer**, assignee) | Display | Timer color-coded; sorted by time-remaining default |
| Filters: reason / amount / time-remaining bucket / assignee | Standard | — |
| Open decision | Btn | → 11.2 |
| Sound-on-new toggle | Toggle | — |

### 11.2 Hold Decision
| Element | Type | Validation / Notes |
|---|---|---|
| Payment detail (parties, route, narrative, purpose, docs) | Display | RO |
| Risk panel (trigger reasons + scores, sanctions hit summary: list, score, matched fields) | Display | RO |
| Decision | Radio | R: Release / Block / Hold for review / Return to originator |
| Rationale | Text | R; min 50 |
| Reason code | Dropdown | R; options depend on decision |
| Evidence-reviewed attestation | Checkbox | R |
| Second approver | Lookup | Auto-required if amount > authority threshold; maker≠checker |
| Tipping-off banner | Display | Always shown |
| Submit | Btn | Re-auth; locks decision (changes only via 11.3) |
| Escalate / Cancel | Btns | Cancel returns to queue without decision |

### 11.3 Override/Exception Approval
| Element | Type | Validation / Notes |
|---|---|---|
| Pending overrides queue (request, requester, amount, reason) | Display | — |
| Decision | Radio | R: Approve override / Uphold block |
| Comments | Text | R; min 50 |
| Authority check | Display | RO: approver limit vs amount — block if insufficient |
| Submit | Btn | Dual-auth (OTP) if > configurable limit (e.g., 100k) |

### 11.4 Auto-Release/Timeout Config
| Element | Type | Validation / Notes |
|---|---|---|
| Rail | Dropdown | R |
| Timeout action | Dropdown | R: release / block / escalate; **Release only allowed for low-risk tiers** |
| Timeout duration | Numeric minutes | R; range per rail (e.g., 5–120); warn if beyond rail bounce limit |
| Warning-at | Numeric minutes | R; must be < timeout |
| Escalation target | Lookup | R if action = escalate |
| Save / Test alert | Btns | Save via approval workflow |

### 11.5 Hold Outcomes Log
| Element | Type | Validation / Notes |
|---|---|---|
| Filters: date (max 90d/query) / decision / reason / analyst | Standard | — |
| Table (+ decision-vs-deadline delta, override flag, downstream outcome) | Display | RO |
| Stats widgets (block rate, override rate, false-block estimate) | Display | RO |
| Export / Create tuning feedback | Btns | Tuning → 8.12 task |

---

# MODULE 12 — SAR / Regulatory Reporting

### 12.1 SAR Workbench *(P-QUEUE)*
| Element | Type | Validation / Notes |
|---|---|---|
| Filters: status / jurisdiction / due-window (overdue / <7d / <30d) / subject | Standard | — |
| Columns (+ statutory deadline countdown, case ref) | Display | — |
| New SAR (manual) | Btn | **Blocked by default** — SAR must originate from case; TL override with justification R min 200 |

### 12.2 SAR Draft Form
| Element | Type | Validation / Notes |
|---|---|---|
| Jurisdiction | Dropdown | R; drives schema |
| Filing type | Dropdown | R: initial / continuing / joint |
| Subject sub-form (name, DOB, nationality, ID type+number, address, phone, occupation, relationship-to-FI, subject role) | Fields | All R except phone; repeated per subject (≥1) |
| Activity fields (activity codes multi, amount range, txn count, activity date range, instruments multi) | Fields | R; date From ≤ To |
| Institution block | Display | RO auto-filled |
| Auto-fill banner | Display | Prefilled fields marked; per-field "revert" btn |
| Save draft / Validate / Next / Back | Btns | Wizard validation per section; schema max-lengths enforced |

### 12.3 Narrative Editor
| Element | Type | Validation / Notes |
|---|---|---|
| Guided prompts (who/what/when/where/why/how) | Checkboxes | R — all attested |
| Narrative | Rich text | R; min 200 chars; max 20,000; live word count |
| Template insert | Btns | Typology-specific templates |
| Spellcheck | Toggle | — |
| PII-in-narrative warning check | Display | Non-blocking warning if subject identifiers appear inconsistently |
| Save / Validate | Btns | — |

### 12.4 Subjects & Activity Tabs
| Element | Type | Validation / Notes |
|---|---|---|
| Subjects table + Add/Edit/Remove | Btns | Remove: reason R; ≥1 subject enforced |
| Activity txn table | Display | From case scope; add via search (must belong to subject); remove reason R |
| Consistency check | Display | Totals vs 12.2 amounts — **mismatch blocks progression** |

### 12.5 Attachments *(P-UPLOAD)*
| Element | Type | Validation / Notes |
|---|---|---|
| Attachment type | Dropdown | R per file |
| Minimum | — | ≥1 statement required to submit |
| Total size | — | 100MB cap |
| PII redaction confirmation | Checkbox | R |

### 12.6 MLRO Review & Decision
| Element | Type | Validation / Notes |
|---|---|---|
| Full package render | Display | RO |
| Completeness checklist | Checkboxes | R — all |
| Decision | Radio | R: Approve filing / Reject–rework / Reject–do not file |
| Rationale | Text | R; min 20 (approve) / min 100 (reject) |
| Do-not-file extras | Fields | Legal-consult checkbox + rationale min 300 |
| Record decision | Btn | MLRO role only; immutable; maker≠checker |

### 12.7 Submission Tracking
| Element | Type | Validation / Notes |
|---|---|---|
| Table (submission ID, SAR ref, date, gateway status, ack ref) | Display | RO |
| Rejection error detail | Display | RO |
| Correction form + Resubmit | Btns | Material changes → re-approval required |
| View receipt / Export register | Btns | — |

### 12.8 Reporting Calendar
| Element | Type | Validation / Notes |
|---|---|---|
| Calendar + list (deadlines, statutory windows, responsible person) | Display | RO |
| Assign responsible | Lookup | R; bulk assign supported |
| Reminder lead-days config | Numeric | R; min 1 |
| Assign / Export / Configure reminders | Btns | — |

### 12.9 Continuing Activity SAR
| Element | Type | Validation / Notes |
|---|---|---|
| Queue (SAR, next review date default +90d) | Display | — |
| New-txns-since-last-filing summary | Display | RO |
| Continue | Checkbox | If checked → updated activity fields R |
| Cease reason | Radio + text | R: ceased / no longer suspicious / closed; text min 100 |
| Snooze review | Btn | Reason R; max 30d |
| Create continuation draft | Btn | → 12.2 prefilled |

### 12.10 Filing Export/Validation
| Element | Type | Validation / Notes |
|---|---|---|
| Generate XML | Btn | Runs schema validation → error list (element, issue); **0 errors required** |
| Encryption key status | Display | RO |
| Submit to gateway | Btn | Requires 12.6 approval on record; re-auth |
| Download XML / receipt | Btns | — |

### 12.11 Tipping-Off Controls
| Element | Type | Validation / Notes |
|---|---|---|
| Restricted-subject list | Display | RO; names masked (hash refs) |
| Role × visibility matrix (existence vs content) | Grid editor | MLRO; dual-control save |
| Access log (who viewed what) | Display | RO |
| Breach-alert toggle | Toggle | Notifies MLRO on access by restricted role |
| Save config / Export access log | Btns | — |

---

# MODULE 13 — Risk Configuration

### 13.1 Customer Risk Model Config
| Element | Type | Validation / Notes |
|---|---|---|
| Factor table (factor, weight, direction) | Editable | Weights numeric 0–100; **total must = 100** |
| Score bands (name, min, max) | Row editor | R; non-overlapping + contiguous validated |
| Auto-upgrade triggers | Multi-select | Event-based |
| Auto-downgrade | — | Prohibited (system warning if attempted) |
| Justification per change | Text | R; min 100 |
| Save draft / Submit / Simulate on portfolio | Btns | Simulation shows re-banded distribution pre-apply |

### 13.2 Country Risk Editor
| Element | Type | Validation / Notes |
|---|---|---|
| Country table (ISO, rating, source ref, effective, review date) | Editable rows | Source ref R per change |
| Edit rating | Btn | New rating R + justification R min 50 + source link R |
| Bulk upload | File | CSV schema-validated |
| Save (versioned) / Import / Export / Version history | Btns | — |

### 13.3 Product/Channel Risk Config
| Element | Type | Validation / Notes |
|---|---|---|
| Table (product, inherent risk weight 1–10, notes) | Editable | Weight R; justification R |
| Effective date | Date | R |
| Save / Export | Btns | Via approval |

### 13.4 PEP Classification & Handling
| Element | Type | Validation / Notes |
|---|---|---|
| Category rows (category, definition, EDD flag, review-frequency months) | Editable | All R |
| Declassification cooling period | Numeric months | R; min 12 (configurable floor) |
| Save / Export | Btns | Via approval |

### 13.5 Global Parameters
| Element | Type | Validation / Notes |
|---|---|---|
| Parameter grid (name, value, unit, owner, last change) | Editable | Type-validated values (numeric/date/time); justification R; effective date R |
| Restore default | Btn | Reason R |
| Save | Btn | Regulated params via approval workflow |

---

# MODULE 14 — QA & Oversight

### 14.1 QA Sampling Queue
| Element | Type | Validation / Notes |
|---|---|---|
| Method | Dropdown | R: random / risk-based / stratified |
| Sample size % | Numeric | R; 5–100 |
| Strata definition | Builder | By severity / disposition |
| Period | Date range | R |
| Own-work exclusion | Display | Auto-enforced, shown |
| Generate sample | Btn | Period locks once generated |
| Start review | Btn | → 14.2 |

### 14.2 QA Scorecard
| Element | Type | Validation / Notes |
|---|---|---|
| Item link | Display | Opens original alert/case RO |
| Rubric rows (criterion, score 0–5, comments) | Row editor | Score R; comments R if score ≤2 |
| Overall result | Display + override | Auto-computed; override Radio R + justification R min 50 |
| Error severity | Dropdown | R if fail: critical/major/minor |
| Retraining flag | Checkbox | — |
| Submit / Save draft | Btns | Notifies analyst |

### 14.3 Analyst Scorecards
| Element | Type | Validation / Notes |
|---|---|---|
| Analyst + period selectors | Dropdowns | R |
| Metrics (avg QA score, trend, fail rate, error mix, overturn rate, team percentile) | Display | RO |
| Export / Schedule coaching | Btns | → 14.4 |

### 14.4 Feedback & Coaching
| Element | Type | Validation / Notes |
|---|---|---|
| Linked scorecard | Display | RO |
| Feedback text | Text area | R; min 100; max 2000 |
| Action items (action, owner, due) | Row builder | All R per row |
| Analyst acknowledgment | Checkbox | R; enforced before closure |
| Send / Close loop | Btns | Close blocked until all action items complete |

### 14.5 Overturned Decisions Log
| Element | Type | Validation / Notes |
|---|---|---|
| Table (+ root-cause dropdown: knowledge gap / system / process / training) | Display | — |
| Corrective action | Text | R |
| Add entry / Export | Btns | — |

---

# MODULE 15 — Reporting & Analytics

### 15.1 Standard Report Library
| Element | Type | Validation / Notes |
|---|---|---|
| Search + category filter | Standard | — |
| Report list (name, description, owner, schedule, last run) | Display | — |
| Run parameters | Dynamic | Period R etc. per report |
| Output format | Dropdown | PDF/XLSX/CSV |
| Run now / Subscribe / Download last | Btns | Downloads retention-limited (config) |

### 15.2 Ad-Hoc Report Builder
| Element | Type | Validation / Notes |
|---|---|---|
| Dataset | Dropdown | R; raw-txn dataset restricted |
| Dimensions / Measures | Multi-selects | R; ≥1 each |
| Filters builder + date range | Standard | Max 24 months/query |
| Visualization | Dropdown | Table/bar/line/pie |
| Row limit | Numeric | Max 100k |
| Run / Save to library / Export | Btns | PII auto-masked; export reason R |

### 15.3 Scheduled Reports
| Element | Type | Validation / Notes |
|---|---|---|
| Report / recipients / format / frequency / time / retention days | Fields | Recipients: valid emails R; frequency R; retention numeric R |
| Active toggle | Toggle | — |
| Save / Pause / Delete | Btns | Delete: confirm |

### 15.4 Typology & Trend Analytics
| Element | Type | Validation / Notes |
|---|---|---|
| Heatmap axes (e.g., typology × geo) | Dropdowns ×2 | R |
| Period | Dropdown | R |
| Emerging-patterns panel | Display | RO |
| Drill-through | Click | → filtered list |
| Export / Save insight | Btns | Insight: annotation text R |

### 15.5 Governed Export Center
| Element | Type | Validation / Notes |
|---|---|---|
| Requests table (ID, requester, dataset, purpose, approver, status, expiry, downloads) | Display | — |
| New request: dataset / fields / purpose / format | Fields | Purpose R min 50; PII fields flagged |
| Approve/Reject | Btns | Approver role; comments R on reject |
| Download | Btn | Pre-expiry only; watermarked; logged |
| Revoke | Btn | Reason R |

---

# MODULE 16 — Notifications & Collaboration

### 16.1 Notification Center
| Element | Type | Validation / Notes |
|---|---|---|
| List (icon, message, entity link, time, read state) | Display | — |
| Filters: type / read status | Standard | — |
| Mark read / Mark all read / Delete | Btns | Delete: confirm |

### 16.2 Comments & @Mentions
| Element | Type | Validation / Notes |
|---|---|---|
| Comment box | Rich text | R; max 2000 |
| @mention picker | Autocomplete | **Cannot mention users lacking entity access** (validated) |
| Insert entity link | Btn | — |
| Post / Edit-own (30-min window) / Resolve thread | Btns | — |

### 16.3 Shift Handover
| Element | Type | Validation / Notes |
|---|---|---|
| Auto WIP summary + urgent items | Display | RO |
| Handover notes | Text | R; min 50 |
| Urgent-item acknowledgments (incoming) | Checkboxes | R — each |
| Initiate / Accept / Decline | Btns | Initiate locks outgoing WIP edits; Decline: reason R |

### 16.4 My Tasks
| Element | Type | Validation / Notes |
|---|---|---|
| Table (task, source, due, priority, status) | Display | — |
| Filters: overdue / due-today / status | Standard | — |
| Open source / Complete / Snooze | Btns | Complete note if required; Snooze: reason R, max 7d |

### 16.5 Notification Preferences
| Element | Type | Validation / Notes |
|---|---|---|
| Per-event rows: in-app toggle / email toggle / digest | Grid | Digest: immediate/hourly/daily |
| Quiet hours | Time range | From < To validated |
| SLA warning lead time | Numeric minutes | R |
| Save / Reset defaults | Btns | — |

---

# MODULE 17 — Knowledge & Guidance

### 17.1 Typology Library
| Element | Type | Validation / Notes |
|---|---|---|
| Search + source filter (FATF/FFIEC/internal) | Standard | — |
| Typology page (description, red flags, examples, linked live scenarios) | Display | RO |
| Link to case | Btn | Adds reference |

### 17.2 SOP Viewer
| Element | Type | Validation / Notes |
|---|---|---|
| Tree nav by role/process | Display | — |
| Content + interactive decision-tree mode | Display | RO |
| Version selector | Dropdown | RO; effective-dated |
| Search / Print (watermark) / Feedback | Btns | Feedback text R |

### 17.3 Regulatory Reference
| Element | Type | Validation / Notes |
|---|---|---|
| Jurisdiction selector | Dropdown | R |
| Obligation list (obligation, citation, deadline rule, linked SOP) | Display | RO |
| Search / Export extract | Btns | — |

### 17.4 In-Context Help
| Element | Type | Validation / Notes |
|---|---|---|
| `?` tooltip per field | Display | RO |
| Guided-tour launcher / topic search | Btns | — |
| Was-this-helpful | Thumbs | — |

---

# MODULE 18 — Administration & Security

### 18.1 User Management
| Element | Type | Validation / Notes |
|---|---|---|
| Search + filters (role, status, team) | Standard | — |
| Form: first/last name, email, employee ID, team, roles (multi), manager, jurisdiction access (multi), start date | Fields | All R; email + employee ID unique; jurisdiction options role-scoped |
| Create / Edit / Deactivate / Unlock / Reset password / Re-enroll MFA | Btns | Deactivate: reason R + forces session kill + WIP-reassignment prompt |

### 18.2 Role & Permission Matrix
| Element | Type | Validation / Notes |
|---|---|---|
| Role selector | Dropdown | R |
| Permission grid (screen × CRUD checkboxes) | Grid | — |
| Field-masking rules (per PII field: full/masked/hidden) | Dropdown per field | — |
| SoD validation | Display | **Blocks save** on conflicting role combos (maker+checker; analyst+QA) |
| Save / New role / Clone / Compare roles | Btns | Save dual-approval; name unique |

### 18.3 Queue Assignment Rules
| Element | Type | Validation / Notes |
|---|---|---|
| Strategy | Radio | R: round-robin / load-based / skill-based / hybrid |
| Max WIP per analyst | Numeric | R |
| Skill mapping (skill → analysts) | Row builder | All R per row |
| Priority weight multipliers | Numerics | Per severity |
| Fallback queue | Dropdown | R |
| Save / Simulate | Btns | Simulation = dry-run on yesterday's volume |

### 18.4 SLA & Workflow Config
| Element | Type | Validation / Notes |
|---|---|---|
| SLA rows (severity × type: respond hrs, resolve hrs) | Editable | All R; resolve > respond |
| Escalation ladder (level, after-X-hours, notify target) | Row builder | All R |
| Business-hours calendar / 24×7 toggle | Selector | R |
| Save / Export | Btns | Via approval |

### 18.5 Reason Code Taxonomy
| Element | Type | Validation / Notes |
|---|---|---|
| Tree editor (category, code, description, applies-to multi, mandatory-note toggle) | Editor | Code unique; all R |
| Active toggle + effective dating | Fields | — |
| Add / Edit / Deactivate / Reorder | Btns | Deactivate blocked if in-flight usage; warns on historical refs |

### 18.6 Notification Templates
| Element | Type | Validation / Notes |
|---|---|---|
| Name / channel / subject / body / languages | Fields | Name unique R; channel R; body R; merge-field picker |
| Preview pane | Display | Sample data |
| Save / Preview / Send test / Publish | Btns | Test: recipient R; Publish versioned |

### 18.7 Data Retention & Purge
| Element | Type | Validation / Notes |
|---|---|---|
| Retention rows (data class, years, regulatory basis) | Editable | Years R; **cannot go below regulatory floor** (enforced) |
| Purge job config (frequency, dry-run-first) | Fields | Dry-run enforced before execute |
| Legal hold (entity, reason, scope) | Fields | All R; blocks purge for held entities |
| Execute purge | Btn | Dual-auth + type-"PURGE" confirm; irreversible |

### 18.8 Delegate/Proxy Access
| Element | Type | Validation / Notes |
|---|---|---|
| Delegatee | Lookup | R; base-role-sufficiency validated |
| Scope / date range / justification | Fields | All R; range From<To; max 90 days |
| Grant / Revoke early / Extend | Btns | TL approval; Revoke: reason R |

### 18.9 Privacy/DSAR Handling
| Element | Type | Validation / Notes |
|---|---|---|
| Intake: subject name, identifier, request type, received date, channel | Fields | All R |
| Auto-scan results (entities found incl. alerts/cases/SAR flags) | Display | RO |
| Legal-hold conflict flag | Display | RO; exemption-claim form R min 200 if conflict |
| Deadline countdown (30d) | Display | Enforced |
| Record decision (fulfil / partial / refuse + rationale) | Radio + text | Rationale R min 100 if refuse |
| Generate response pack / Close | Btns | Redaction checklist R before generate |

### 18.10 License & Environment
| Element | Type | Validation / Notes |
|---|---|---|
| License usage per module / expiry alerts | Display | RO |
| Environment info / release notes | Display | RO |
| Feature flags (flag, state, scope) | Toggle table | Toggle: confirm; some require MLRO approval |
| Export license report | Btn | — |

---

# MODULE 19 — IT Operations

### 19.1 System Health Dashboard
| Element | Type | Validation / Notes |
|---|---|---|
| Service tiles (API, scoring, screening, case svc, DB, cache: status, latency, error rate) | Display | RO; auto-refresh 30s |
| Create incident: severity, description, affected services | Fields | All R; description min 20 |
| Acknowledge / Create incident | Btns | — |

### 19.2 Data Pipeline Monitor
| Element | Type | Validation / Notes |
|---|---|---|
| Source table (core/cards/wires/SWIFT/ACH/KYC: status, last event, lag s, events/min) | Display | RO; lag > threshold highlighted |
| Lag threshold per source | Numeric | R |
| Restart consumer | Btn | Confirm + impact note |
| Pause source | Btn | Reason R; monitoring-gap warning |
| Reprocess window | Time range | R; async |

### 19.3 Exception Queue
| Element | Type | Validation / Notes |
|---|---|---|
| Table (record ID, source, error code, message, payload preview, status) | Display | — |
| Retry / Edit & resubmit / Discard / Bulk retry | Btns | Edit+Discard: reason R; Discard audited |

### 19.4 Data Reconciliation
| Element | Type | Validation / Notes |
|---|---|---|
| Control report (source vs ingested vs scored vs alerted; variance %) | Display | RO |
| Gap drill-down (missing windows) | Display | RO |
| Run reconciliation | Btn | Period R; async |
| Raise gap task | Btn | Assignee R |

### 19.5 Integration/API Monitor
| Element | Type | Validation / Notes |
|---|---|---|
| Endpoint table (screening vendor, goAML, core, FX feed: status, p95, uptime, heartbeat) | Display | RO |
| Test connection / View error logs / Failover toggle | Btns | Failover: IT-lead + confirm |

### 19.6 Batch/EOD Job Monitor
| Element | Type | Validation / Notes |
|---|---|---|
| Job table (name, schedule, last status/duration, next run) + dependency view | Display | RO |
| Run now / Rerun step / Hold job | Btns | Run-now: dependency check; Hold: reason R |

### 19.7 Capacity & Performance Trends
| Element | Type | Validation / Notes |
|---|---|---|
| Charts (CPU/mem/storage/queue depth; hourly/daily/weekly) | Display | RO |
| Forecast + projected exhaustion date | Display | RO |
| Alert thresholds (%) | Numeric | R |
| Export / Configure alerts | Btns | — |

### 19.8 Reference Data Sync
| Element | Type | Validation / Notes |
|---|---|---|
| Table (FX rates, country risk, calendars, lists: last sync, next, status, delta) | Display | RO |
| FX staleness rule | Display | >24h stale → alert; fail-open/fail-closed dropdown R |
| Sync now / History / Configure schedule | Btns | Async; delta report after |

---

# MODULE 20 — Audit Trail

### 20.1 Audit Log Search
| Element | Type | Validation / Notes |
|---|---|---|
| Filters: users (multi) / action types / entity ID / date range / value-contains | Standard | Max 90 days/query; older → archived-request flow |
| Table (timestamp UTC, user, role-at-time, action, entity, before→after) | Display | RO; immutable |
| Search / Export / Save query | Btns | Export: AUD/MLRO + approval |

### 20.2 Config Change History
| Element | Type | Validation / Notes |
|---|---|---|
| Filters: config type (rules/thresholds/SLA/permissions/watchlists) | Standard | — |
| Table (change ID, item, maker, checker, approved, effective, diff link) | Display | RO |
| View diff / Export evidence pack | Btns | — |

### 20.3 Regulatory Audit Pack Export
| Element | Type | Validation / Notes |
|---|---|---|
| Pack type | Dropdown | R: alert handling / SAR file / model governance / screening coverage |
| Period + scope | Fields | R |
| Completeness indicator | Display | Auto |
| Generate / Download / Share link | Btns | Download encrypted+watermarked; share link expiry numeric R max 30d |

---

# MODULE 21 — Restricted / Downstream Views

### 21.1 Auditor Read-Only Portal
| Element | Type | Validation / Notes |
|---|---|---|
| Watermark overlay (AUDIT READ-ONLY + user + timestamp) | Display | Always on |
| Engagement scope selector | Dropdown | R |
| All screens | RO | No write actions rendered; exports logged + reason R |

### 21.2 Regulator Inspection View
| Element | Type | Validation / Notes |
|---|---|---|
| Session grant: entities, period, modules, expiry datetime | Fields | All R; set by MLRO |
| Session-recording indicator | Display | Always on; downloads disabled by default |
| End session / Extend | Btns | MLRO only; Extend: justification R |

### 21.3 CS Transaction Status Lookup
| Element | Type | Validation / Notes |
|---|---|---|
| Customer identifier + txn reference | Text ×2 | Both R; **must match together** |
| Result (status, resolution timeframe band, required customer action) | Display | RO; scripted bands only |
| Prohibited: reason detail / scenario / analyst / SAR existence | Display | Suppressed + banner |
| Scripted response panel + Copy | Display + Btn | — |
| Escalate to compliance queue | Btn | Reason R |

### 21.4 RM/Business Customer Status View
| Element | Type | Validation / Notes |
|---|---|---|
| Customer search | Lookup | R |
| Result (restriction yes/no, affected products coarse, required business action) | Display | RO; no reasons/analyst identity |
| Submit info request | Btn | Creates RFI (4.9) |
| Log contact | Btn | Auto |

---

# MODULE 22 — Miscellaneous

### 22.1 My Profile & Preferences
| Element | Type | Validation / Notes |
|---|---|---|
| Name / email | Display | RO |
| Phone | Text | Format-validated |
| Language / Timezone | Dropdowns | Timezone R |
| Signature block | Text | max 200 |
| Change password (old + new) | Passwords | Old R; new per policy |
| Manage MFA | Btn | OTP verify to proceed |
| Save | Btn | — |

### 22.2 Help & Support
| Element | Type | Validation / Notes |
|---|---|---|
| Ticket: category / priority / description / attachment | Fields | All R except attachment; description min 30 |
| KB search | Text | Min 2 |
| Submit ticket | Btn | — |

### 22.3 About / Release Notes
| Element | Type | Validation / Notes |
|---|---|---|
| Version / build / environment / license summary | Display | RO |
| Data classification banner | Display | RO |
| Copy system info | Btn | For support tickets |

---

## ✅ Coverage Check
| Module | Screens | Module | Screens |
|---|---|---|---|
| 1 Auth | 6 | 12 SAR | 11 |
| 2 Dashboards | 7 | 13 Risk Config | 5 |
| 3 Alerts | 10 | 14 QA | 5 |
| 4 Cases | 14 | 15 Reporting | 5 |
| 5 Entity | 10 | 16 Notifications | 5 |
| 6 Transactions | 8 | 17 Knowledge | 4 |
| 7 Network | 5 | 18 Admin | 10 |
| 8 Rules | 12 | 19 IT Ops | 8 |
| 9 ML | 6 | 20 Audit | 3 |
| 10 Watchlist | 6 | 21 Restricted | 4 |
| 11 Intervention | 5 | 22 Misc | 3 |
