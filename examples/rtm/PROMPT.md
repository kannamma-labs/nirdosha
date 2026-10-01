# RTM project prompt — what to paste into hi's prompt mode

This file is the project's own answer to hi's "What do you want to build?"
screen: the canonical prompt for the Real-Time Transaction Monitoring (RTM)
app that already lives in this folder. It is ingested into `.nir/hi.db`
(`nirdosha-hi ingest PROMPT.md`) so Ask and the graph's context layer carry
it, and it is the suggested starting point for any `:prompt` / `+ Screen`
extension of the app.

---

The prompt:

```text
Build a Real-Time Transaction Monitoring (RTM) application for AML + fraud
compliance — detective and real-time interceptive modes in one system, with
11 human roles (Analyst, ComplianceLead, Mlro, QaReviewer, OpsAnalyst,
PolicyEngineer, Admin, Auditor, CsAgent, RmUser, RegulatorViewer) and
service principals as backend identities only.

Ingestion and detection: Kafka-style transaction ingestion, a guarded data
plane (row 12 identity, purpose-scoped reads), a rules/scenario engine with
windows and allowlists, and ML model scoring with drift/champion-challenger
monitoring. Screening (sanctions/PEP/watchlists) with rescreen campaigns.

Operations: an L1 triage alert queue with SLA timers and a monitoring wall
for the fraud desk; payment holds (interceptive mode) with expiry countdown
and override approvals; case management with an investigation workspace,
linked alerts/transactions, RFIs, tasks, and 4-eyes approval chains
(unified approvals inbox at /approvals plus per-chain inboxes).

SAR lifecycle: draft wizards with MLRO review, release chains, submissions
calendar, continuing-activity SARs, and tipping-off controls (SAR data is
absent from CsAgent/RmUser views entirely).

Governance and assurance: rule/model approval chains with policy-release
inboxes, risk config (country/product/PEP/global params), QA sampling and
scorecards, governed exports with egress release, MIS/typology reporting,
notifications and a knowledge base, admin (users, delegation, DSAR,
retention, SLA config), IT ops (pipeline health, integrations), audit
chains with an auditor watermark portal, and a time-boxed regulator
inspection scope.

Shell: demo avatar login (11 roles), per-role post-login landing, nav from
menus.toml (grouped, stage-gated), auditor/regulator shell variants,
forbidden/not-found fallbacks to /denied. Every screen rides the real
nirdosha_rt UI macros (crud_screens!, dashboard!, kanban_board!, wizard!,
approval_inbox!, workspace!) with guard-policy role gating on every read
and write; data access goes through GuardedTable with declared purposes.
```

## Why this file exists

- `hi`'s prompt screen is skipped for this project (its graph is populated
  from real code — RFC 0014's build mode), but `:prompt` and the `+ Screen`
  rail still take free text; this is the grounded starting point for both.
- Ingested, it gives `Ask` a direct answer to "what is this project about".
- It mirrors `screen.md` (the 152-screen inventory) and `menus.toml`
  (the role→menu→route register), so the prompt never drifts further from
  the app than those two registers do.