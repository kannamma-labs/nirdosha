# Real-Time Transaction Monitoring (RTTM) Application — Complete Screen Inventory

Below is an exhaustive screen map for an enterprise-grade RTTM system (AML + fraud, detective + real-time interceptive modes). It's organized into **22 modules / ~150 screens**, with the actor(s) for each.

---

## 🧑‍💼 Actor Legend (used in all tables)

| Code | Actor | Role Summary |
|---|---|---|
| **L1** | Alert/Triage Analyst | First-line alert review & disposition |
| **L2** | Senior Analyst / Investigator | Case investigations, SAR drafting |
| **TL** | Team Lead / Compliance Manager | Queue oversight, approvals, 4-eyes |
| **MLRO** | MLRO / Compliance Head | SAR decisions, regulatory sign-off |
| **RMG** | Rule/Model Manager | Scenario design, tuning, ML models |
| **QA** | QA Reviewer | Quality sampling & scorecards |
| **OPS** | Real-time Ops / Fraud Desk | Payment holds, live monitoring |
| **IT** | IT Ops / Data Engineer | Pipelines, health, integrations |
| **ADM** | System Administrator | Users, config, security |
| **AUD** | Auditor (internal/external) | Read-only assurance |
| **CS** | Customer Service Agent | Transaction status lookup (masked) |
| **BUS** | Business / Relationship Manager | Restricted customer status view |
| **REG** | Regulator | Scoped, time-boxed inspection view |
| **ALL** | All roles | — |

---

## Module 1 — Authentication & Session (6 screens)

| Screen | Purpose / Key Components | Actor |
|---|---|---|
| 1.1 Login | Credentials, SSO/SAML option, remember device, branding | ALL |
| 1.2 MFA Challenge | OTP/push/TOTP, trusted-device toggle, fallback methods | ALL |
| 1.3 Forgot/Reset Password | Identity verification, secure token flow | ALL |
| 1.4 First-Login Setup | Forced password change, consent/NDA acceptance, region/language select | ALL |
| 1.5 Session Timeout & Re-auth | Idle warning countdown, re-authentication prompt | ALL |
| 1.6 Access Denied / Locked | Lockout message, unlock request to admin, justification field | ALL |

---

## Module 2 — Dashboards & Landing (7 screens)

| Screen | Purpose / Key Components | Actor |
|---|---|---|
| 2.1 Global Navigation Shell | Persistent top/side nav, entity & environment switcher (multi-jurisdiction groups), global search, notification bell, profile menu | ALL |
| 2.2 L1 "My Day" Dashboard | My pending alerts, SLA clocks, today's throughput, aging buckets, quick links to queues | L1 |
| 2.3 Team Dashboard | Workload per analyst, WIP, reassignment stats, SLA breach risk, escalation funnel | TL |
| 2.4 MLRO / Leadership Dashboard | Alert volumes, SAR conversion rate, aging, regime deadlines, open cases by type, staffing pressure | MLRO |
| 2.5 Real-Time Monitoring Wall | Live transaction ticker, alerts/sec, severity heat map, geo map of hits, top triggering scenarios — large-screen NOC style | OPS, TL |
| 2.6 Executive KPI Dashboard | FP ratio trends, cost per alert, model health snapshot, period-over-period comparisons | MLRO, Executives |
| 2.7 Saved Views / Filter Manager | Create, share, pin, subscribe to saved queue views | ALL |

---

## Module 3 — Alert Management / L1 Triage (10 screens)

| Screen | Purpose / Key Components | Actor |
|---|---|---|
| 3.1 Alert Work Queue | Columns: ID, severity, scenario, customer, amount, trigger time, SLA countdown, status, assignee; filters, sort, bulk select, KPI strip | L1, TL |
| 3.2 Alert Detail | ⭐ Full triage view (deep dive below) | L1, L2, TL |
| 3.3 Disposition Panel | Close as FP/false hit/duplicate/known — mandatory reason codes + free text, evidence checklist | L1 |
| 3.4 Related & Linked Alerts | Alerts on same customer/counterparty/period; dedup & merge actions | L1, L2 |
| 3.5 Bulk Actions | Mass assign, close, re-prioritize, tag (with audit + reason) | L1, TL |
| 3.6 Reassignment / Pickup | Self-assign, assign to analyst/team, reassignment reason | L1, TL |
| 3.7 Pending Info (RFI) State | Snooze alert with deadline, request info from branch/business, auto-reminder | L1 |
| 3.8 Escalation Form | Escalate-to-case with rationale template, evidence summary, priority recommendation | L1 → L2 |
| 3.9 SLA & Aging Monitor | Breached/at-risk alerts, aging buckets, per-team heat view | TL, MLRO |
| 3.10 Alert Audit History | Every action/view timestamped, immutable log per alert | ALL (view), AUD |

### ⭐ Deep Dive — 3.2 Alert Detail (the heart of L1 work)
- **Header:** alert ID, scenario name + version, trigger time, severity, SLA countdown, status, assignee
- **Trigger explanation panel:** exact conditions met with values (e.g., *"11 cash deposits of $9,000–$9,900 in 7 days; threshold: ≥10 near $10k"*)
- **Customer snapshot:** KYC status, risk rating + recent delta, PEP/sanctions flags, tenure, expected activity profile
- **Triggering transactions strip** with drill-down to full transaction detail
- **Tabs:** Related alerts · Transaction history (chart + table) · Counterparties · Network mini-graph · Documents · Notes · Audit trail
- **Action bar:** Close-FP / Escalate / Reassign / Note / Link to case / RFI
- **Guardrails:** tipping-off restriction flag, role-based PII masking

---

## Module 4 — Case Management / L2 Investigation (14 screens)

| Screen | Purpose / Key Components | Actor |
|---|---|---|
| 4.1 Case Queue | ID, subject, type (ML/TF/fraud), priority, stage, age, assignee, SLA | L2, TL |
| 4.2 Case Overview | Summary, status, linked alerts count, total suspicious value, timeline of stage changes | L2 |
| 4.3 Investigation Workspace | ⭐ Tabbed workspace (deep dive below) | L2 |
| 4.4 Linked Alerts Tab | All alerts with L1 dispositions and reasoning | L2 |
| 4.5 Case Transactions Tab | In-scope transactions, filters, add/remove from scope, annotate | L2 |
| 4.6 Documents & Evidence | Upload statements, IDs, contracts, screenshots; versioning; classification | L2 |
| 4.7 Notes & Activity Timeline | Chronological log: notes, actions, decisions, who/when | L2, TL |
| 4.8 Tasks & Checklist | Investigation checklist template, owners, due dates | L2, TL |
| 4.9 Internal RFI | Structured info request to branches/business units with SLA & response thread | L2 |
| 4.10 Disposition & Closure | Suspicious–substantiated / not substantiated; reason codes; mandatory rationale | L2 |
| 4.11 Four-Eyes Review | Reviewer sees analyst conclusion, evidence; approve / return with comments | TL |
| 4.12 Escalation to MLRO | Escalation memo, recommendation, supporting bundle | L2, TL |
| 4.13 Case Merge/Link | Merge duplicate cases, link related cases (same network) | L2, TL |
| 4.14 Initiate SAR from Case | Auto-populates SAR draft (Module 12) | L2 |

### ⭐ Deep Dive — 4.3 Investigation Workspace
- **Tabs:** Overview · Alerts · Transactions · Customer 360 · Network graph · Documents · Tasks · Notes · SAR · Disposition
- **Persistent header:** case ID, type, assignee, stage (Triage → Investigation → Review → Decision), SLA timer
- **Right rail:** related entities, quick actions, internal contacts
- **Evidence pinboard:** drag transactions/alerts/graphs into a narrative-ordered evidence list
- **Every action logged** to immutable case audit trail

---

## Module 5 — Customer & Entity Intelligence (10 screens)

| Screen | Purpose / Key Components | Actor |
|---|---|---|
| 5.1 Global Entity Search | Search by customer/account/counterparty/txn ID/passport/phone — fuzzy matching | ALL analysts |
| 5.2 Customer 360 | ⭐ Full profile (deep dive below) | L1, L2, TL, BUS (masked) |
| 5.3 KYC/CDD/EDD Document Viewer | Docs with review checklist, expiry alerts, refresh triggers | L2 |
| 5.4 Risk Score Breakdown | Factor contributions, rating history, next review date | L1, L2 |
| 5.5 Expected vs Actual Activity | Declared profile vs observed behavior, deviation meters | L2 |
| 5.6 Related Parties / UBO | Ownership/control structure tree, indirect links | L2 |
| 5.7 Adverse Media Results | Hits, sources, dates, sentiment, disposition | L2 |
| 5.8 Entity Alert/Case History | Full lifecycle of all past alerts & cases, outcomes | ALL analysts |
| 5.9 Internal Risk Notes/Flags | Analyst notes, watch flags (not customer-visible) | L2, TL |
| 5.10 Restriction/Exit Recommendation | Propose account restriction/exit with justification → business approval workflow | L2 → BUS/TL |

### ⭐ Deep Dive — 5.2 Customer 360
- **Identity panel:** demographics, IDs, KYC status, onboarding date/channel, branch
- **Risk panel:** composite score + factor breakdown, PEP/sanctions/adverse-media status
- **Activity panel:** 12-month in/out volume & count charts, avg/peak txn, top counterparties, channels used
- **Relationships:** accounts, products, related parties, UBO, shared attributes (address/phone/device)
- **History:** alerts, cases, SARs (restricted), contact notes
- **Behavior:** expected profile vs actual, unusual-hours/unusual-geo indicators

---

## Module 6 — Transaction Intelligence (8 screens)

| Screen | Purpose / Key Components | Actor |
|---|---|---|
| 6.1 Transaction Search | Multi-field + memo/narrative text search, date/amount/geo/channel filters, save query | L1, L2, OPS |
| 6.2 Transaction Detail | Full record: parties, route (SWIFT/rail), channel, device/IP, FX, status, linked alert/case | ALL analysts |
| 6.3 Transaction Timeline/Ledger | Customer account ledger view with risk-flagged rows highlighted | L2 |
| 6.4 Funds Flow Visualization | Sankey/graph of money movement across entities & time; layering fan-in/fan-out | L2 |
| 6.5 Counterparty Analysis | Top counterparties, new-payee velocity, round-amount patterns | L2 |
| 6.6 Pattern/Typology Match Viewer | Which scenarios & typologies matched this behavior (e.g., structuring, rapid movement) | L2 |
| 6.7 Structuring Visual | Near-threshold deposit charts, multi-branch aggregation view | L2, OPS |
| 6.8 Device & Session Intelligence | Device ID, IP, geolocation, velocity, impossible-travel (fraud context) | OPS, L2 |

---

## Module 7 — Network / Link Analysis (5 screens)

| Screen | Purpose / Key Components | Actor |
|---|---|---|
| 7.1 Interactive Entity Graph | Nodes = customers/accounts/devices/addresses; edges = transactions/shared data; zoom, filter, expand | L2 |
| 7.2 Path Finder | Find connections between entity A and B (degrees of separation) | L2 |
| 7.3 Cluster/Community Detection | Detected rings/smurf networks, density scoring | L2 |
| 7.4 Shared Attributes Panel | Common devices, IPs, addresses, employers, beneficiaries across entities | L2, OPS |
| 7.5 Graph Snapshot Export | Export as exhibit image/data pack for SAR attachment | L2 |

---

## Module 8 — Detection Scenario & Rule Management (12 screens)

| Screen | Purpose / Key Components | Actor |
|---|---|---|
| 8.1 Scenario Library | All rules: ID, typology mapped, status, version, owner, last tuned, alert volume | RMG, TL (view) |
| 8.2 Rule Builder/Editor | ⭐ Visual condition builder (deep dive below) | RMG |
| 8.3 Threshold & Parameter Editor | Editable thresholds, per-segment overrides, environment separation (dev/test/prod) | RMG |
| 8.4 Segmentation & Scope Config | Which products/channels/geos/customer segments the rule applies to | RMG |
| 8.5 Suppression/Dedup Logic | Re-alert suppression windows, same-pair dedup, aggregation keys | RMG |
| 8.6 Simulation & Backtesting | Replay against historical data: hit counts, FP sample, estimated workload | RMG |
| 8.7 Threshold Sensitivity ("What-if") | Curves: volume vs threshold, FP trade-off visualization | RMG |
| 8.8 Approval Workflow | States: Draft → Tested → Approved → Scheduled → Live; maker-checker enforced | RMG (maker), TL/MLRO (checker) |
| 8.9 Version History & Diff | Side-by-side version compare, who changed what/when/why | RMG, AUD |
| 8.10 Scheduling & Go-Live | Effective dates, pilot cohort, staged rollout, rollback | RMG |
| 8.11 Whitelist/Allowlist Management | Exempted customers/counterparties — rationale, owner, expiry, approval required | RMG, TL (approve) |
| 8.12 Scenario Performance View | Per-rule alerts, FP%, escalation rate, SAR conversion, tuning recommendations | RMG, TL |

### ⭐ Deep Dive — 8.2 Rule Builder
- **Rule metadata:** name, ID, typology mapping (FATF/FFIEC), description, owner
- **Condition canvas:** aggregation functions (sum/count/velocity/avg/deviation), lookback windows, grouping keys (customer, account, counterparty)
- **Threshold panel:** static values or percentile-based/adaptive thresholds
- **Scope selector:** segment, product, channel, geography, currency (FX normalization setting)
- **Test panel:** run against sample/historical data inline, see resulting alerts
- **Priority & weight:** severity scoring contribution
- **Version notes + approval routing** built into save action

---

## Module 9 — ML Model Management (6 screens)

| Screen | Purpose / Key Components | Actor |
|---|---|---|
| 9.1 Model Registry | Models, versions, status (shadow/active/retired), owners, retraining dates | RMG |
| 9.2 Performance Monitor | Precision/recall, FP trend, SAR conversion, score distribution over time | RMG, TL |
| 9.3 Drift Monitoring | Data drift (PSI), alert volume anomalies, feature stability | RMG, IT |
| 9.4 Champion/Challenger | Side-by-side comparison on same population, promotion workflow | RMG |
| 9.5 Model Validation & Governance | Validation docs, assumptions, approvals, review calendar (model risk) | RMG, AUD |
| 9.6 Score Explainability Viewer | Per-alert top contributing factors (SHAP-style) shown inside Alert Detail | L1, L2 |

---

## Module 10 — Watchlist & Screening Management (6 screens)

| Screen | Purpose / Key Components | Actor |
|---|---|---|
| 10.1 Screening Hit Queue | Sanctions/PEP/adverse-media hits with match scores, list source, SLA | L1 |
| 10.2 Hit Review & Disposition | True/false match evidence, disposition codes, whitelist proposal, escalate | L1, L2 |
| 10.3 Screening Configuration | Lists in use, fuzzy-matching thresholds, algorithm selection, name-order handling | ADM |
| 10.4 Watchlist Management | Upload/update lists (OFAC, UN, EU, internal), custom internal lists, effective dates | ADM |
| 10.5 Rescreening Monitor | Delta rescreening on list updates, bulk rescreen jobs, status | IT |
| 10.6 Historical Rescreen Campaigns | Re-screen entire book against new criteria, campaign progress & results | ADM, TL |

---

## Module 11 — Real-Time Payment Intervention (5 screens) *(interceptive mode)*

| Screen | Purpose / Key Components | Actor |
|---|---|---|
| 11.1 Interception Queue | Held payments in-flight with countdown timers (must decide before release deadline) | OPS |
| 11.2 Hold Decision Screen | Payment detail, risk score, triggered reasons, sanctions hit; **Release / Block / Hold-for-review** with reason + authority capture | OPS |
| 11.3 Override/Exception Approval | TL approves overrides beyond analyst authority; dual authorization for high value | TL |
| 11.4 Auto-Release/Timeout Config | Timeout rules, auto-release defaults, escalation on timeout | ADM |
| 11.5 Hold Outcomes Log & Stats | Blocked/released stats, override rates, false-block analysis | TL, MLRO |

---

## Module 12 — SAR / Regulatory Reporting (11 screens)

| Screen | Purpose / Key Components | Actor |
|---|---|---|
| 12.1 SAR Workbench Queue | Drafts, pending review, filed, rejected; deadlines countdown | L2, MLRO |
| 12.2 SAR Draft Form | Structured fields per goAML/FinCEN/local schema, auto-filled from case | L2 |
| 12.3 Narrative Editor | Guided prompts (who/what/when/where/why/how), templates, length guidance, spellcheck | L2 |
| 12.4 Subjects & Activity Tabs | Suspect(s), accounts, transaction table auto-populated from case scope | L2 |
| 12.5 Supporting Attachments | Statements, ID docs, flow diagrams, network graph exhibits | L2 |
| 12.6 MLRO Review & Decision | File / don't-file decision with rationale; delegation of authority | MLRO |
| 12.7 Submission Tracking | Filing acknowledgment IDs, rejected filings, correction/resubmission | MLRO |
| 12.8 Reporting Calendar | Statutory deadlines per jurisdiction, countdown, responsible person | MLRO |
| 12.9 Continuing Activity SAR | 90-day review queue for ongoing suspicious activity | MLRO |
| 12.10 Filing Export/Validation | goAML XML generation + schema validator, encryption, submission receipt storage | L2/MLRO |
| 12.11 Tipping-Off Controls | Restricted-visibility flags on SAR subjects (who can/can't see SAR existence) | MLRO (config), system-enforced |

---

## Module 13 — Risk Configuration (5 screens)

| Screen | Purpose / Key Components | Actor |
|---|---|---|
| 13.1 Customer Risk Model Config | Factors, weights, scorecard bands, automatic re-rating triggers | RMG, ADM |
| 13.2 Country/Geography Risk Editor | Ratings per jurisdiction with source references & review dates | ADM |
| 13.3 Product/Channel Risk Config | Risk weights per product, delivery channel, delivery method | ADM |
| 13.4 PEP Classification & Handling | PEP categories (domestic/foreign/intl org), declassification rules | ADM |
| 13.5 Global Parameters | Default thresholds, FX rates for aggregation, business calendars, cutoff times | ADM |

---

## Module 14 — Quality Assurance & Oversight (5 screens)

| Screen | Purpose / Key Components | Actor |
|---|---|---|
| 14.1 QA Sampling Queue | Random/risk-based sample of closed alerts & cases awaiting review | QA, TL |
| 14.2 QA Scorecard | Rubric: evidence completeness, reasoning quality, correct disposition, documentation; pass/fail | QA, TL |
| 14.3 Analyst Scorecards | Individual accuracy trends, calibration vs peers, FP-overturn rates | TL |
| 14.4 Feedback & Coaching | Structured feedback threads on scored items, acknowledgment by analyst | TL, L1/L2 |
| 14.5 Overturned Decisions Log | Reopened/reversed dispositions, root causes, retraining triggers | TL, MLRO |

---

## Module 15 — Reporting, MIS & Analytics (5 screens)

| Screen | Purpose / Key Components | Actor |
|---|---|---|
| 15.1 Standard Report Library | Alert volumes, disposition stats, SLA compliance, SAR stats, rule performance, aging | TL, MLRO |
| 15.2 Ad-Hoc Report Builder | Drag-drop dimensions/measures, filters, export (Excel/PDF/CSV) | Power users, TL |
| 15.3 Scheduled Reports & Distribution | Recurring reports, subscribers, formats, delivery | TL, ADM |
| 15.4 Typology & Trend Analytics | Heatmaps by typology × geography × product; emerging-pattern views | MLRO |
| 15.5 Governed Export Center | Controlled data extracts with approval, purpose capture, expiry | TL, AUD |

---

## Module 16 — Notifications & Collaboration (5 screens)

| Screen | Purpose / Key Components | Actor |
|---|---|---|
| 16.1 Notification Center | In-app alerts: assignments, SLA warnings, approvals pending, mentions | ALL |
| 16.2 Comments & @Mentions | Threaded discussions on alerts/cases with entity-linking | ALL analysts |
| 16.3 Shift Handover | WIP summary, pending items, time-critical items, handover acknowledgment | L1, L2, TL |
| 16.4 My Tasks | Cross-case task list with due dates and priorities | ALL |
| 16.5 Notification Preferences | Channels, digests, quiet hours, SLA-warning thresholds | ALL |

---

## Module 17 — Knowledge & Guidance (4 screens)

| Screen | Purpose / Key Components | Actor |
|---|---|---|
| 17.1 Typology Library | Red-flag indicators, FATF/FFIEC typologies, worked examples, linked scenarios | ALL |
| 17.2 SOP/Procedure Viewer | Role-specific playbooks, decision trees, jurisdiction procedures | ALL |
| 17.3 Regulatory Reference | Per-jurisdiction obligations, deadlines, filing rules | ALL |
| 17.4 In-Context Help | Tooltips, guided walkthroughs, field-level definitions | ALL |

---

## Module 18 — System Administration & Security (10 screens)

| Screen | Purpose / Key Components | Actor |
|---|---|---|
| 18.1 User Management | Create/edit/deactivate users, lock/unlock, password policy enforcement | ADM |
| 18.2 Role & Permission Matrix | RBAC roles, screen-level & field-level permissions, PII masking rules | ADM |
| 18.3 Queue Assignment Rules | Round-robin, load-based, skill-based routing; team structures | ADM, TL |
| 18.4 SLA & Workflow Config | SLA definitions per severity, escalation paths, business-hours awareness | ADM, TL |
| 18.5 Reason Code Taxonomy | Disposition/reason code hierarchies, mandatory-field rules | ADM |
| 18.6 Notification Templates | Email/in-app template editor with merge fields, multi-language | ADM |
| 18.7 Data Retention & Purge | Retention schedules per data class, purge job config, legal hold | ADM, MLRO |
| 18.8 Delegate/Proxy Access | Time-boxed access delegation for leave cover, with audit | ADM |
| 18.9 Privacy/DSAR Handling | Data subject access/erasure requests workflow with legal-hold checks | ADM, MLRO |
| 18.10 License & Environment | License usage, environment info, release/version management | ADM |

---

## Module 19 — IT Operations & Data Monitoring (8 screens)

| Screen | Purpose / Key Components | Actor |
|---|---|---|
| 19.1 System Health Dashboard | Service status, latency, queue depth, uptime, incident banners | IT |
| 19.2 Data Pipeline Monitor | Per-source ingestion status (core banking, cards, payments, SWIFT), lag, throughput | IT |
| 19.3 Exception Queue | Failed/malformed records with error codes, replay, manual enrichment | IT |
| 19.4 Data Reconciliation | Txn counts in vs scored vs alerted; gap detection; completeness checks | IT, ADM |
| 19.5 Integration/API Monitor | Connectivity to core systems, screening vendors, goAML gateway; heartbeat & latency | IT |
| 19.6 Batch/EOD Job Monitor | Job schedules, run history, failure alerts, rerun controls | IT |
| 19.7 Capacity & Performance Trends | CPU/memory/storage, projected headroom, peak-hour behavior | IT |
| 19.8 Reference Data Sync | FX rate loads, country-risk list updates, holiday calendars — freshness & status | IT |

---

## Module 20 — Audit Trail (3 screens)

| Screen | Purpose / Key Components | Actor |
|---|---|---|
| 20.1 Audit Log Search | Filter by user, action, entity, date; immutable; drill to before/after values | AUD, ADM |
| 20.2 Config Change History | All rule/threshold/workflow changes with maker-checker evidence | AUD, MLRO |
| 20.3 Regulatory Audit Pack Export | Pre-built examiner evidence bundles (decisions, SLAs, model governance) | MLRO, AUD |

---

## Module 21 — Restricted / Downstream Views (4 screens)

| Screen | Purpose / Key Components | Actor |
|---|---|---|
| 21.1 Auditor Read-Only Portal | Full read access, no edit rights, dedicated watermark, access logged | AUD |
| 21.2 Regulator Inspection View | Time-boxed, scoped dataset view with session recording | REG, MLRO |
| 21.3 CS Transaction Status Lookup | *"Is my customer's payment held?"* — scripted responses only, **no alert details (tipping-off protection)** | CS |
| 21.4 RM/Business Customer Status View | Masked view: restrictions/holds on customer, required actions — **no alert rationale** | BUS |

---

## Module 22 — Miscellaneous (3 screens)

| Screen | Purpose / Key Components | Actor |
|---|---|---|
| 22.1 My Profile & Preferences | Signature, delegate settings, display prefs, language | ALL |
| 22.2 Help & Support | Raise ticket, contact compliance helpdesk, system status link | ALL |
| 22.3 About / Release Notes | Version, new features, known issues | ALL |

---

## 🔄 Core Screen Flow

```
Login (1) → Dashboard (2) → Alert Queue (3.1) → Alert Detail (3.2)
   ├─→ Close FP (3.3) ──────────────────────────────→ QA Sample (14)
   └─→ Escalate (3.8) → Case Workspace (4.3) → 4-Eyes (4.11)
          ├─→ Disposition: Not Suspicious → Close
          └─→ Escalate MLRO (4.12) → SAR Draft (12.2) → MLRO Decision (12.6)
                 → Filing/Export (12.10) → Acknowledgment (12.7)
Parallel flows:
   OPS: Interception Queue (11.1) → Hold Decision (11.2)
   RMG: Rule Builder (8.2) → Backtest (8.6) → Approve (8.8) → Go-Live (8.10)
   IT:  Pipeline Monitor (19.2) → Exception Queue (19.3)
```

---

## 📊 Summary

| Category | Modules | Screens |
|---|---|---|
| Access & Dashboards | 1–2 | 13 |
| Core Investigation (Alerts, Cases, Entity, Txn, Network) | 3–7 | 54 |
| Detection & Screening (Rules, ML, Watchlist, Intervention) | 8–11 | 29 |
| Reporting & Compliance (SAR, Risk, QA, MIS) | 12–15 | 26 |
| Platform (Notifications, Knowledge, Admin, Ops, Audit, Views, Misc) | 16–22 | 27 |
| **Total** | **22 modules** | **~150 screens** |

---
