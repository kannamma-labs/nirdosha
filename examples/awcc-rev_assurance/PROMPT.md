# AWCC Revenue Assurance project prompt

Build a Revenue Assurance (RA) application for Afghan Wireless Communication
Company (AWCC) — CDR/usage reconciliation, billing accuracy, interconnect &
roaming, VAS/digital revenue, leakage & margin, and ATRA regulatory reporting,
with adjacent fraud-overlap surfaces surfaced to RA (not the fraud product itself).

## Domain & roles

10 human roles + service principals as backend identities only:
- RaAnalyst — L1 investigator (CDR discrepancies, leak cases)
- RaLead — L2 team lead / four-eyes approver
- FinanceController — L3 final ATRA signer
- OpsEngineer — real-time wall, holds, intervention queue
- PolicyEngineer — rule/threshold/model author
- Admin — system & ops admin
- Auditor — read-only assurance portal
- QaReviewer — QA sampling & scorecards
- CsAgent — CS transaction status lookup (4 fields only)
- RetailLiaison — business customer status (single field)
- RegulatorViewer — ATRA inspector via scoped delegation
- Service principals: SvcIngest, SvcRecon, SvcRating, SvcAtra, SvcAlerts,
  SvcCase, SvcBi, McpCopilot

## Scope

Reconciliation and detection: Kafka-style CDR ingestion, guarded data plane
(row 12 identity, purpose-scoped reads), reconciliation rules (missing CDR,
duplicate, rating mismatch, unbilled event, over/under charge, mediation miss),
and anomaly/model scoring.

Operations: L1 triage discrepancy queue with SLA timers, real-time CDR monitoring
wall, payment/charge holds with expiry and override approvals, leak-case
management with investigation workspace, linked CDRs, RFIs, tasks, and 4-eyes
approval chains (unified approvals inbox at /approvals).

ATRA lifecycle: draft bundles, FinanceController review, release chains,
submissions calendar, continuing-activity reporting, and tipping-off controls
(ATRA data absent from CsAgent/RetailLiaison/McpCopilot views).

Governance and assurance: rule/model approval chains, risk config
(country/product/global params), QA sampling and scorecards, governed exports,
MIS/reconciliation reporting, notifications and knowledge base, admin
(users, delegation, DSAR, retention, SLA config), IT ops (pipeline health,
integrations), audit chains with an auditor watermark portal, and a time-boxed
regulator inspection scope.

## Shell

Demo avatar login (10 human roles), per-role post-login landing, nav from
menus.toml (grouped, stage-gated), auditor/regulator shell variants,
forbidden/not-found fallbacks to /denied. Every screen rides the real
nirdosha_rt UI macros (crud_screens!, dashboard!, kanban_board!, wizard!,
approval_inbox!, workspace!) with guard-policy role gating on every read and
write; data access goes through GuardedTable with declared purposes.

## Structure

This package mirrors `examples/rtm`:
- `screens.toml` — 152-screen register
- `menus.toml` — role→menu→route register
- `src/00_core.nir` — roles, purposes, approval chains, invariants, policies
- `src/10_domains.nir` — dataset structs (CDR, alert, leak_case, subscriber,
  payment, atra_bundle)
- `src/20_ingestion.nir` — reconciliation/ingestion pipeline
- `src/bridge.nir` — GuardedEntity / GuardedTable bridge for screens
- `src/screens/*.nir` — screen modules
- `tests/verify_screen_inventory.rs` — cross-file invariant check
