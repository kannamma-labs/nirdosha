# Nirdosha RTM — The 150-Screen Inventory Wired to the Actual Project

Your generic spec now gets grounded in what the RFCs actually specify. Three things change when screens are built on Nirdosha:

1. **Screens are not hand-built apps over a DB.** They are (a) macro-generated UI archetypes (`login!`, `app_shell!`, `crud_screens!`, `dashboard!`, `kanban_board!`, `wizard!`, `settings_screen!`, `communication_feed!` — RFC 0022), (b) MCP tools consumed by `mcp:analyst-copilot` (RFC 0025 §8.8), and (c) governance surfaces over the catalog + audit chains + lineage plane (RFC 0026).
2. **Every screen's data path is a guarded surface.** A "screen" = a `read_plan` (filters/masks/caps attached) + `guarded_apply` mutations + `workflow!`/`approval_chain!` macros. My generic validations don't live in the UI — they live at four enforcement layers: **rustc/macros (Gate 1) → verify passes (Gate 2) → plan-time (guard) → driver/runtime (L1 + invariants)**.
3. **Some of my generic validations are forbidden here** — the guard is stricter than a typical web app. Flagged as ⚠️ divergences below.

---

## Part 1 — Actors → Nirdosha Principals & Roles

| My actor | Nirdosha principal / role | How it materializes |
|---|---|---|
| L1 Analyst | Human role `Analyst` | Direct guarded UI (destination=`browser`) or via `mcp:analyst-copilot` delegation (RFC 0024 §2) |
| L2 Investigator | `Analyst` (case-scope policies) | Same role, broader policies on `case` |
| TL / Compliance Mgr | `ComplianceLead` (quorum member in `sar_release`) | `approval_chain! { quorum(2, of=[ComplianceLead]) }` — RFC 0025 §8.6 |
| MLRO | **2nd `ComplianceLead` in quorum(2)** — or add dedicated `Mlro` role | Gap: RFCs only name `ComplianceLead`; add role |
| RMG (rule/model mgr) | Engineer authoring `.nir` + `PolicyEngineer` principal for `action="migrate"` | Policy edits are graph proposals (RFC 0024 §5.1), never runtime files (N-A2) |
| QA reviewer | **Gap** — add role `QaReviewer` in `roles!` | No QA entities/policies exist yet |
| OPS fraud desk | **Gap** — add role `OpsAnalyst` for Mode B hold release | Holds exist (Pending + I3); no release-role policy yet |
| IT Ops | Platform principal; driver install = guarded catalog mutation | RFC 0025 §6.6, §7.5 |
| ADM | `admin` role; admin ops are guarded mutations | RFC 0025 §6.6 (`admin()`) |
| Auditor | Read-only principal, purpose=`audit`, `destination(export-file)` gated by approval | Export = egress action (RFC 0023 §2) |
| CS / BUS | **No grants on `alert`/`case` at all**; restricted coarse views via purpose + masks | Deny-by-default (P3); destination control |
| Regulator | Time-boxed scoped **delegation token** (exact scope, TTL, full audit) | RFC 0023 §9.4 mechanics |
| System services | `svc:ingest`, `svc:features`, `svc:scoring`, `svc:screening`, `svc:alerts`, `svc:case`, `svc:bi` | RFC 0025 §8 table |
| LLM agent | `mcp:analyst-copilot` — non-transferable dual-bound delegation | TTL 30m, max_tool_calls=60, rate 20/min, row_cap=50, audit=full (I17) |

---

## Part 2 — Data Dictionary (entities/fields actually declared or derivable from the RFCs)

| Entity | Fields (from policies/macros) | Source | Status |
|---|---|---|---|
| `transaction` | required: `tenant_id, subject_id, amount, currency, status, occurred_at` · allowed: `channel, merchant_id, card_token, device_id, geo` | `ingest-create-txn` policy, RFC 0025 §8.1 | ✅ Declared |
| `alert` | required: `tenant_id, txn_id, score, model_version, policy_version` + `rule_hits` · `status` = **analyst-only** (forbidden to svc:alerts) | `alert-raise` policy, §8.5 | ✅ Declared |
| `case` | `status` (workflow machine), `assigned_to` · `alert_ids` immutable | `case-transition` policy, §8.6 | ✅ Declared |
| Case status machine | `open → investigating → [confirmed_fraud, false_positive, escalate]; confirmed_fraud → sar_filed → closed; false_positive → closed` | `workflow! CaseStatus` | ✅ Declared |
| Features | `velocity_1h` (count, sum(amount)), `impossible_travel`, `distinct_payees_7d`, `amount_dev_30d` | `window!`, `model_artifact!` §8.2–8.3 | ✅ Declared |
| Model `rt_fraud_v1` | inputs = 4 features; outputs `score: f64, explanation: vec[string]`; `threshold_alert = 0.85` | §8.3 | ✅ Declared |
| Sanctions lists | `ofac_sdn, un_consolidated, eu_fsf`; matcher `fuzzy_jaro_winkler @ 0.92` | `matcher!` §8.4 | ✅ Declared |
| Topics/channels | `txn_events`, `txn.authorized`, `guard.decisions`, channels `analyst-inbox`, `regulatory-log` | §8.1, §6.5 | ✅ Declared |
| `sar` bundle, `customer`, `screening_hit`, `qa_review`, `pending_wire` | — | **Gaps to declare** in a `90_ops.nir` / `65_sar.nir` | 🔲 New work |

---

## Part 3 — Screen-Archetype Assignment (RFC 0022)

| My screen family | Nirdosha archetype macro |
|---|---|
| 1.x Login | `login!` (mode: demo \| production) |
| 2.1 Shell / nav | `app_shell!` (nav entries role-gated: `{ label, href, role }`) |
| 3.1 / 4.1 / 10.1 / 11.1 queues | `crud_screens!` (read-forward variant) |
| 2.2–2.6 dashboards | `dashboard!` |
| 4.x case pipeline board | `kanban_board!` — natural fit for the CaseStatus machine |
| 12.2 SAR draft | `wizard!` (section-validated steps) |
| 13.x, 18.x config | `settings_screen!` |
| 16.1 notifications | `communication_feed!` (fed by `notify(channel(...))`) |
| Everything under copilot | MCP tools: `query_records`, `get_options`, `evaluate`, `submit_write` |

---

## Part 4 — Module-by-Module Mapping

### M1 — Auth & Session
| Screen | Nirdosha backing | Status |
|---|---|---|
| 1.1 Login | `login!` macro; demo users or `NIRDOSHA_LOGIN_USERS` env hook; unset env = reject-all + warn | RFC 0022 **pending implementation** |
| 1.2 MFA | **Not in login path.** Guard has `escalate to step_up(mfa)` for sensitive writes only | ⚠️ Gap — put behind IdP/SSO |
| 1.3 Password reset | External credential source by design (RFC 0022 §6 scope honesty) | Out of scope → IdP |
| 1.4 First-login | Not specified | App-layer gap |
| 1.5 Session timeout | Copilot delegation TTL 30m / max 1h, revoked on logout/policy change (RFC 0024 §9); guard bulk-revocation cancels in-flight | Partial |
| 1.6 Access denied | **Every screen surfaces `GuardError` with named reason** (`sod.ingestion_is_write_only`, `policy.not_lowerable`) — deny-by-default is the norm, not an error page | ✅ Native |

### M2 — Dashboards
| Screen | Backing | Notes |
|---|---|---|
| 2.1 Shell | `app_shell!` | Role-gated nav |
| 2.2–2.4, 2.6 | `dashboard!` over aggregate `read_plan`s | Follow `bi-aggregate` shape: `cohort_floor`, `time_range(... retention_window())`, `audit(full)` (I9) |
| 2.5 Real-time wall | Consumer of `guard.decisions` topic-dataset via `StreamSource` | ⚠️ Row-level live view needs an explicit Ops read policy with caps; lag > watermark flags provenance lag (§13) |
| 2.7 Saved views | Catalog entities | Guarded mutations |

### M3 — Alerts
| Screen | Backing | Governing surface |
|---|---|---|
| 3.1 Queue | `query_records(Alert)` / `read_plan("alert")`; opaque cursors only | Analyst read policy: `cap(row_cap)`, masks, `obligate audit(sampled)` for listing, full on open |
| 3.2 Detail | Read plan + evidence tabs; copilot variant sets `destination(llm_context)` | ⚠️ `denied_above(CONFIDENTIAL)`; masked fields **absent**, not masked-in-place |
| 3.3 Disposition | `guarded_apply(update, alert)`; status via **new `AlertStatus` workflow machine** (add — gets type-state illegal-transition compile errors) | `field_policy{ allowed(status, disposition_code, rationale, ...) forbidden(txn_id, score, rule_hits, model_version, policy_version) }`, `cap(affected_rows=1)`, `audit(full)` |
| 3.4 Related/merge | `alert_ids` set at creation, immutable after (`forbidden(alert_ids)` mirrors case rule) | |
| 3.5 Bulk | **⚠️ Divergence:** `affected_row_cap` must cover bulk; audit is per-record (I1, receipts embedded) — no "one rationale for 500" without a guarded bulk policy | |
| 3.6 Reassign | `allowed(assignee)` field write | |
| 3.7 RFI/Pending | Maps to `Decision::Pending` semantics conceptually; implement as status + task entity | |
| 3.8 Escalate | Creates `case` — but **only `svc:case`-authorized principal may create cases** (SoD V7); analyst escalation = case-creation policy for Analyst or via svc:case API | |
| 3.9 SLA monitor | Aggregate read, `cohort_floor` | |
| 3.10 Audit history | Per-module hash chain + decision traces (`policy_version` + `model_version`, I4) | ✅ Native |

### M4 — Cases *(most fully specified module in the RFCs)*
| Screen | Backing |
|---|---|
| 4.1 Queue | `kanban_board!` over CaseStatus — board columns = machine states |
| 4.2 Overview | Read plan; SAR-linked flag restricted (tipping-off) |
| 4.3 Workspace | Copilot MCP session or direct UI; both hit identical guard |
| 4.10 Disposition | `case-transition` policy: `requires field(status).transition_allowed()`, `allowed(status, assigned_to)`, `forbidden(alert_ids)`, `cap(affected_rows=1)`, `audit(full)` — **illegal transitions don't compile** (type-state enum) |
| 4.11 Four-eyes | Generalize: add `approval_chain! { chain case_review { quorum(2, of=[ComplianceLead]); timeout(deny); } }`; **self-approval blocked** structurally (conformance SoD probe) |
| 4.12 Escalate MLRO | Status `escalate` exists in the machine |
| 4.14 Initiate SAR | `sar_filed` transition + `sar-export` policy below |

### M5 — Customer 360
`customer` entity **not yet declared** — needs `90_ops.nir`: `#[dataset(entity="customer")]`, fields per my 5.2 spec, classifications (`#[classify]`), relations via `#[relation]` (UBO tree = Tier 1 bounded or Tier 2 materialized `visible_closure` — negated relations are compile errors). Screening/PEP flags come from `matcher!` hits + reference data. Adverse media = external list provider driver (same `ListProvider` port).

### M6 — Transactions
| Screen | Backing |
|---|---|
| 6.1 Search | `read_plan("transaction", &QueryShape)` — filters compile to `FilterExpr`; narrative keyword = `Pattern{matcher}` |
| 6.2 Detail | Full record incl. `risk score at txn time` = decision trace lookup by `trace_id` |
| 6.4 Funds flow | **FederatedPlan** — per-binding sub-plans + `MergeSpec` + `BudgetToken`; mid-stream abort = explicit `Watermark`, never silent truncation (I16, §1C.1) |
| 6.5 Counterparties | Aggregate read w/ `cohort_floor=10`, `mask(subject_id, tokenized)` |
| 6.8 Device | Fields exist (`device_id`, `geo`); impossible-travel is a **declared feature** (`window!` session) |

### M7 — Network Analysis
⚠️ **Semantic correction:** Nirdosha's native graph is the **lineage plane** (flows between datasets/models/policies), not a social network of customers. Map my screens as: 7.1/7.2 → `lineage_query!` views `downstream_of`/`upstream_of` (depth-capped `max_depth=5, max_nodes=1_000, max_execution=5s`); 7.4 shared attributes → ordinary guarded filters on `transaction` (device_id etc.); customer-ring detection → new dataset/relation work. `provenance_of("01JD8WQ7…")` = the regulator-facing "how did alert A-12891 happen" screen, receipt-backed (RFC 0026 §9.3).

### M8 — Rules & Scenarios *(re-imagined: rules ARE Rust source)*
| My screen | Nirdosha reality |
|---|---|
| 8.1 Library | Catalog nodes (window/model/matcher/policy records) — the declared graph |
| 8.2 Builder | **Editing `.nir` via `graph_apply`** → creates a *proposed policy revision*; **not live until `graph_accept`** with promotion grant (RFC 0024 §5.1). rustc diagnostics are the validation UX |
| 8.3 Thresholds | `action="migrate"` guarded mutation: dry-run **mandatory**, maker-checker, full audit (RFC 0023 §2) |
| 8.6 Backtest | **`policy_simulation!`** — measures `alert_volume, denied_flows, escalation_load, case_load` over a 7d window vs baseline; read-only, V10-subject |
| 8.7 What-if | Same replay engine; V9-reserved model-pack gate shares it |
| 8.8 Approval | `approval_chain!` + Gate-2 verify re-run + signed `.nirpkg` |
| 8.9 Versions | Policy snapshots per version (I4); lineage edges keyed by `policy_version` — historical flows never silently merged |
| 8.11 Whitelist | Reference dataset + explicit `allow` policies; guarded mutations |

### M9 — ML
Registry = catalog (`model_artifact!` + `SignedArtifact{sig, sha256}`); swap = `admin()` **guarded mutation** (never config edit), shadow-mode flag carried (§8.3); rollback = re-activate previous signed artifact, backtest delta in audit chain. Drift/monitoring screens = Phase 4 ("model governance, shadow mode, replay backtest, drift").

### M10 — Screening
`matcher!` + `ListProvider` port (osanctions driver; World-Check/Dow Jones = second driver for V8). Every check `audit(full)`, `cap(max_scan_rows=50)`. List refresh cadence **re-attested via manifest** (V5). Hit disposition = new `screening_hit` entity + policy (gap).

### M11 — Real-Time Intervention *(⚠️ key divergence)*
| My screen | Nirdosha |
|---|---|
| 11.1 Interception queue | Query over **Pending entities** created by `Effect::Escalate(hold)` in `guarded_apply` |
| 11.2 Hold decision | Release = `revalidate_at_commit` (I3) — state re-checked at commit; deny-by-timeout |
| 11.4 Auto-release on timeout | **⚠️ Forbidden by default:** `approval_chain! timeout(deny)` — "approvals expire to deny, **never allow**". Auto-release requires an explicit, separate policy — flag as a compliance decision |
| 11.3 Override | Second approver via chain; `escalate to step_up(mfa)` above thresholds |

### M12 — SAR
| Screen | Backing |
|---|---|
| 12.1–12.5 | `wizard!` over a new `sar` dataset; activity txns from case scope |
| 12.6 MLRO decision | `sar_release` chain: `quorum(2, of=[ComplianceLead]); timeout(deny)` — verbatim RFC 0025 §8.6 |
| 12.7–12.10 Filing | SAR export = **egress action**: ReadPlan + egress obligations (full audit, no sampling, caps, escalate above thresholds); `destination(export-file)`; goAML gateway = external driver behind an export port (new work) |
| 12.11 Tipping-off | **Native:** classification + `destination(llm_context) denied_above(CONFIDENTIAL)` + masked fields absent; SAR-existence visibility = field-level classification, enforced at every surface incl. copilot |

### M13–M15 — Risk Config / QA / Reporting
- **M13:** country risk = `#[reference]` dataset; changes = migrate (maker-checker); risk model = policy records + `#[classify]`.
- **M14:** no QA machinery in RFCs — new role + `qa_review` entity + policies; QA *reads* dispositions via decision traces (already receipt-backed).
- **M15:** `bi-aggregate` policy is the template verbatim: `cohort_floor=10`, `mask(subject_id, tokenized)`, `destination(warehouse)`, `audit(full)`; exports = egress actions; ⚠️ totals only when `count_allowed` (§8.4).

### M16–M17 — Notifications / Knowledge
`notify(channel("analyst-inbox"))`, `notify(topic(guard.decisions))` → feed screens. Knowledge = **signed build-time artifacts** (tool descriptions, `.nirpkg`-signed docs) — never runtime-generated (RFC 0024 §6).

### M18 — Admin
Everything-is-a-catalog-entity (P6): users via `roles!` (external store via login hook), permission matrix = **the `guard_policy!` set itself** (P3: surface = granted policies — there is no separate matrix to drift), SoD = V7 + conformance probe, retention = `data_contract! { retention = 7y }`, DSAR = export action + legal-hold via retention override.

### M19 — IT Ops
Driver manifests + capability attestation (V5, canary rows, differential tests, **dynamic downgrade on detected lie**); plan-time rejection = "the swap fails at plan time with a named missing capability"; failure modes table (§13) = your ops runbooks; degraded-mode reads = per-domain opt-in, TTL ≤60s, separate kill switch, reconciliation required before health-check passes.

### M20 — Audit *(upgraded by RFC 0026)*
Per-module hash chains (prev_hash/hash), I4 replayability, dual streams (graph audit + guard audit, RFC 0024 §7) correlatable by session. **New:** lineage observations as audit records (`kind:"lineage"`), `provenance_of`, V10 delta findings, signed audit-chain export for packs.

### M21–M22 — Restricted views / Misc
Destination dimension (`browser | api-client | llm-context | export-file | webhook`) is the mechanism my "masked CS/RM views" hand-waved. Regulator view = scoped delegation + full audit. About screen = `.nirpkg`/manifest data.

---

## Part 5 — Flagship Screens in the v2 Dialect

```rust
// ============ 90_ops.nir — additions the screens need (illustrative) ============
nirdosha_rt::workflow! {
    machine AlertStatus {
        new -> in_progress -> [closed_false_positive, escalated, pending_info];
        pending_info -> [in_progress, escalated];
    }
}

// ---- 3.1 Alert queue (Analyst, browser) ----
nirdosha_rt::guard_policy! {
    allow "analyst-list-alerts" for Analyst
    when action == "read" && resource == "alert"
    filter tenant_scope()
    cap(row_cap = 200, max_scan_rows = 50_000, max_execution = 5s)
    field_policy { forbidden(sar_linked) }              // tipping-off
    obligate audit(sampled)
}

// ---- 3.3 Disposition (write) ----
nirdosha_rt::guard_policy! {
    allow "analyst-disposition" for Analyst
    when action == "update" && resource == "alert"
    requires field(status).transition_allowed()
    field_policy {
        allowed(status, disposition_code, rationale, closed_category, assignee)
        forbidden(txn_id, score, rule_hits, model_version, policy_version)
    }
    cap(affected_rows = 1)
    obligate audit(full)
}
// rationale min-length: presence via required(); content quality via
// #[invariant] registered in the Condition::Custom registry — or app-layer.

// ---- 12.x SAR export (egress, RFC 0023 §2 + RFC 0025 §8.6) ----
nirdosha_rt::guard_policy! {
    allow "sar-export" for ComplianceLead
    when action == "export" && resource == "sar_bundle"
    requires field(status) == "confirmed_fraud"
    destination(export-file)
    escalate to approval(chain sar_release)             // quorum(2), timeout(deny)
    obligate audit(full)                                // never sampled
    obligate notify(channel("regulatory-log"))
}

// ---- 8.3 Threshold change (maker-checker migrate) ----
nirdosha_rt::guard_policy! {
    allow "window-threshold-migrate" for PolicyEngineer
    when action == "migrate" && resource == "window"
    escalate to approval(chain policy_release)
    obligate audit(full)
}
// gate: policy_simulation! dry-run + cargo nirdosha verify before apply.

// ---- Lineage explorer (RFC 0026 §9.1, verbatim posture) ----
nirdosha_rt::guard_policy! {
    allow "lineage-explore" for Analyst
    when action == "lineage_query" && resource == "downstream_of"
    filter tenant_scope()
    cap(max_depth = 5, max_nodes = 1_000, max_execution = 5s)
    destination(llm_context) denied_above(CONFIDENTIAL)
    obligate audit(full)                 // lineage reads are never sampled
}
```

---

## Part 6 — New Screens Nirdosha Adds (absent from my generic inventory)

| New screen | Actor | Source |
|---|---|---|
| Policy Proposal Review (`graph_apply` drafts → diff → `graph_accept`) | PolicyEngineer + ComplianceLead | RFC 0024 §5.1 |
| Break-Glass request/approve/reconcile (reason, ticket, TTL ≤1h, dual-approve, auto review task) | Ops + ComplianceLead | RFC 0023 §14 |
| Delegation Session Monitor (TTL, tool-call budget 60, rate 20/min, evaluate↔submit_write hash journal) | ADM/TL | RFC 0024 §5.3, §8.8 |
| **V10 Findings** (dormant / undeclared / issued-unconsumed / break-glass-open / degraded-pending) | RMG, AUD | RFC 0026 §8 |
| Lineage Explorer (`provenance_of`, authority classes, ExternalClaimed quarantine) | L2, AUD | RFC 0026 §9 |
| Guard Posture / Degraded-Mode banner + kill switch | IT | RFC 0023 §3.2 |
| Audit Chain Integrity viewer (per-module hash chains, reconciliation state) | AUD | RFC 0025 §6.3 |
| Driver Capability & Attestation viewer (manifests, downgrades) | IT | RFC 0023 §7 |
| Simulation Runs (`policy_simulation!` results) | RMG | RFC 0026 §10.3 |
| Catalog Browser (`list_entities` / `describe_entity` redacted) | All | RFC 0023 §1C.3 |
| GuardError surfacing standard (named deny reasons on every screen) | ALL | Native |

---

## Part 7 — ⚠️ Divergences: where the guard is stricter than my generic spec

| My generic rule | Nirdosha reality |
|---|---|
| Pagination 25/50/100 | **Opaque cursors only; offset rejected** (§8.4) |
| Timeout → auto-release (11.4) | **Timeout → deny, never allow** (`approval_chain! timeout(deny)`) |
| Bulk close, one rationale (3.5) | `affected_row_cap` + per-record audit with embedded Receipts (I1) |
| Export max rows + reason | Export = egress action: full audit, no sampling, escalate above thresholds; "reason" = declared `purpose` code |
| "No rows vs no access" UX distinction | **Forbidden** — uniform response shape externally (§8.4) |
| Totals on queues | Only when `count_allowed` granted |
| Masked-but-filterable | `predicate_use` grants only; masked fields excluded from WHERE/JOIN/GROUP/HAVING/ORDER/window (I15) |
| Free-text validation (min lengths) | Presence via `field_policy`; semantics via `#[invariant]` registry or app layer |

---

## Part 8 — Feasibility vs the Checklist (grounded truth)

| Phase (RFC 0025 §15 / 0026 §15) | Screens unlocked | Blockers (0023 checklist) |
|---|---|---|
| **Now** | Shell/login UI authored (RFC 0022 pending impl); all `.nir` compiles; GuardError surfaces | Guard-core IR ✅; macros parse ✅ — **but `guard_policy!` lowering to plans is [OPEN] ("macros are no-ops")** |
| **Phase 2** (RDBMS slice, V1–V4) | M6 search/detail, M3 partial, decision feed | Drivers: write path [DONE] at the platform level (`crates/nirdosha-guard-store-postgres`, RFC 0026 checklist, `docs/adr/0013-...md`) — **but this example's own `src/` is still empty, so nothing in `examples/rtm` actually uses it yet**; read-path execution has no trait/contract at all (separate, undesigned gap). I1–I6 [OPEN]; write-plan semantics [OPEN] |
| **Phase 3** (windows, scoring, Mode B) | M8 partial, M11, M2.5 | Capability ladder, QueryShape leak controls [OPEN] |
| **Phase 4** (models, screening, drift) | M9, M10, M8 complete | Drivers [OPEN] |
| **Phase 5** (case, MCP copilot, `cargo nirdosha build`) | M4, M12, copilot versions of M3–M7, M16–17 | Federation merge-layer (I16), MCP registry — [PARTIAL] skeletons |
| **RFC 0026 Ph1–2** | M20 upgrade, lineage explorer, V10 | Draft; needs I18/V10 amendments accepted |
| **RFC 0026 Ph4** | 8.6/8.7 simulation screens, M13 | Replay service |

**Hard gaps to schedule:** MFA/IdP; `customer`/`sar`/`screening_hit`/`qa_review` datasets; `AlertStatus` + `case_review` workflow/chains; OpsAnalyst/QaReviewer/Mlro roles; goAML egress driver; payments-rail timeout semantics (must resolve the timeout(deny) conflict).

---

## Part 9 — End-to-End Crosswalk (RFC 0025 §10 trace ↔ my flow)

| RFC trace | Step | My screens |
|---|---|---|
| t0–t2 | ingest + plan commit + audit | 6.2 |
| t3 | decision trace → `guard.decisions` | 2.5 |
| t4–t5 | features + score 0.91 under `2025.11.4` | 8.12, 9.2 |
| t6 | `alert-raise` (score ≥ 0.85) + inbox notify | 3.1 → 16.1 |
| t7 | copilot session: evaluate → submit_write | 4.3 |
| t8 | SAR export → quorum(2) → I3 revalidate → egress | 12.6 → 12.10 |

---
