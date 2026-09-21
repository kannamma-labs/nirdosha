# ============================================================================
# screens-plan.md — per-task build plan & tracker for the 152-screen register
#
# Companion to: screen.md (inventory), screens.toml / menus.toml (registers),
# tickets.md (ticket legend), inventory-wiring.md (grounding).
#
# Enforced by (all must stay green; `cargo test -p rtm`):
#   tests/verify_screen_inventory.rs      V1/V2/V3/V5/V6/V10 cross-file checks
#   tests/verify_tickets.rs               ticket legend ↔ inventory, both directions
#   tests/verify_tickets_and_blockers.rs  stale-blocker pinning + ticket resolution
#
# HOW TO USE THIS FILE
#   - One section per pending task. Checkbox status at the section header.
#   - "Done-when" is the acceptance bar; every line must be demonstrable.
#   - RULES OF ENGAGEMENT (repo law, not aspirational):
#     R1  Any commit touching screens.toml/menus.toml that changes a pinned
#         set (stale pairs, promotion candidates, ticket gates) updates
#         verify_tickets_and_blockers.rs / verify_tickets.rs IN THE SAME
#         COMMIT. No silent pin drift.
#     R2  Docs change in the same step as code — tickets.md, this file, and
#         screen-field-mappings.md move together with the work they describe.
#     R3  Build what's real, disclose the rest. A screen flips stage only
#         when its archetype, dataset (real GuardedTable), and guard_policy!
#         all exist. Never fake a screen to move the count.
#     R4  Every new guard_policy! must be reachable from menus.toml's V5
#         check — no orphan grants, no unbacked guards on reachable screens.
# ============================================================================

## Status dashboard

| #  | Task | Group | Size | Screens | Status | Depends on |
|----|------|-------|------|---------|--------|------------|
| A1 | Stale-blocker purge + 17-screen re-stage | A | M | 17 | ☑ done | A2 (landed same commit) |
| A2 | `read user_role` policy (18.1) | A | S | 1 | ☑ done | — |
| B1 | T-03 disposition vocabulary (S) | B | S | 2+2 | ☑ done | — |
| B2 | T-14 kanban drag transitions (S) | B | S | 1 | ☑ done | B1 (machine) |
| B3 | T-02 forbidden=absent drop pass (S) | B | S | gate | ☐ pending | — |
| B4 | T-10 role-ident canonicalization (S) | B | S | all | ☐ pending | — |
| B5 | T-12 predicate binding / I15 (S) | B | S | gate | ☐ pending | — |
| B6 | T-09 live streaming plane (M) | B | M | 4 | ☐ pending | — |
| B7 | T-01 governed egress (M) | B | M | 5 | ☐ pending | — |
| B8 | T-11 audit-chain projection (M) | B | M | 4 | ☐ pending | — |
| B9 | T-07+T-08 SAR wizard + datasets (M+M) | B | M | 5 | ☐ pending | — |
| B10| T-04 approvals + `approval_inbox!` (L) | B | L | 12 | ☐ pending | — |
| B11| T-06 linked context + `workspace!` (L) | B | L | 10 | ☐ pending | — |
| C1 | PG.case_task task plane | C | M | 5 | ☐ pending | — |
| C2 | Model-stats plane | C | M | 6 | ☐ pending | — |
| C3 | M7 graph (edge store + runtime + renderer) | C | L | 4 | ☐ pending | — |
| C4 | M8 rules (catalog + builder) | C | L | 9 | ☐ pending | B10 (approvals for migrate UX) |
| C5 | Simulation + what-if (RFC0026-P4) | C | L | 2 | ☐ pending | — |
| C6 | Sankey funds-flow | C | M | 1 | ☐ pending | — |
| C7 | Reporting builder | C | M | 3 | ☐ pending | — |
| C8 | Refdata/config singles (~15 screens) | C | L | ~15 | ☐ pending | — |
| C9 | M19 ops monitors (~8 screens) | C | M | ~8 | ☐ pending | — |
| C10| Long-tail singles (~10 screens) | C | M | ~10 | ☐ pending | — |
| D1 | Macro governance holes (absorbed into B3/B4/B5/B6/B9) | D | — | — | ☑ done | remaining holes each have owning ticket |
| D2 | Delegated IdP screens (M1.2–M1.5) | D | — | 4 | ⊘ never | external IdP |

Statuses: ☐ pending · ◐ in-progress · ☑ done. Update in the same commit as the work (R2).

**Two-agent execution.** Work is split per `agent-a-engine.md` (engine/
macros: B1–B5, B6–B11, C4–C7) and `agent-b-dataplanes.md` (data planes:
A1+A2 ✓, C1/C2/C8/C9/C10, then C3/C6). Register edits follow the
shared-file protocol in those files: rebase → edit → `cargo test -p rtm`
→ commit immediately.

---

# GROUP A — spec-only, no archetype/engine work

## A1 ☑ Stale-blocker purge — re-stage 17 screens (done)
**Goal.** Delete the 34 stale `dataset:PG.<x>` blocker entries in
`screens.toml` whose datasets already exist as live `GuardedTable`s in
`bridge.nir`, and re-stage the 17 screens whose every blocker is stale.

**Unlocks (17 screens → emittable).**
10.1, 10.2, 11.5, 12.1, 12.9, 14.1, 14.2, 14.3, 16.1, 16.5, 18.1, 2.4,
21.3, 21.4, 22.1, 22.2, 5.5 — the exact pinned list
`verify_tickets_and_blockers` prints on every run.

**Steps.**
1. Re-derive the stale set live: `cargo test --test verify_tickets_and_blockers
   -- --nocapture` and copy the printed candidate list (do not transcribe
   from this file — the test output is the source of truth).
2. Edit `screens.toml`: for each of the 17, remove the stale `dataset:` token
   from `blocked_by` (leave any *non-stale* blocker untouched — none of the
   17 have any, verified by the pin), and set `stage = "emittable"`.
3. Edit `tests/verify_tickets_and_blockers.rs`: shrink
   `EXPECTED_STALE_PAIRS` and `EXPECTED_PROMOTION_CANDIDATES` to the new
   (empty) sets. Same commit (R1).
4. `cargo test -p rtm` — inventory V1–V10, tickets, and stale-pin suites
   all green.

**Files.** `screens.toml`, `tests/verify_tickets_and_blockers.rs`.
**Done-when.** 41 screens at `built`+`emittable`-or-better (current 24 + 17
newly promoted); pinned sets empty; no verify test red.
**Guardrail.** 18.1's menu guard (`read user_role`) is currently a
disclosed gap because the screen is `blocked`. Promotion makes it reachable
⇒ **A2 must land in this same commit**, or V5 goes red.

## A2 ☑ `read user_role` policy — screen 18.1 (done)
**Goal.** Back `menus.toml` nav.users (`read user_role`, Admin) with a real
`guard_policy!` so 18.1 User Management is V5-clean the moment it is
stage-promoted.

**Steps.**
1. Add to `src/90_ops_admin.nir` (it already owns the `admin-grant-role`
   write path for this resource — read lives beside it):
   `guard_policy! { allow "user-role-read" for Admin
   when action == "read" && resource == "user_role" purpose(PlatformOperations)
   cap(row_cap = 500) obligate audit(sampled) }`
   (Screen 18.1's `roles` list is `Admin` only today; the policy must match
   that exact set. If Auditor or other roles need user management later,
   update the screen's `roles` first, then the policy, in the same commit.)
2. `cargo test -p rtm` — verify_screen_inventory's V5 covered-count rises
   by one; no new disclosures.

**Files.** `src/90_ops_admin.nir` (+ `lib.nir` only if a new file is preferred).
**Done-when.** `nav.users` guard resolves; 18.1 promotable without verify churn.

---

# GROUP B — ticket work-streams (each names its tickets.md section)

## B1 ☑ T-03 — disposition vocabulary + rationale invariant (S) (done)
**Unlocks (landed).** 11.2 Hold Decision → `built` (m11_intervention.nir's
hold desk was already real; only `hold_reason_valid` was missing); 4.10
Disposition & Closure → `built` (m04_cases.nir's `case-transition` was
already real; only `case_disposition_valid` was missing — no new screen
file needed, `crud_screens!`'s existing `update_fields` mount already
carries `disposition`); 4.1 keeps its board (T-14 drag graying is its only
remaining blocker); 3.3 closes its feature gate (`disposition_code_valid`
added alongside the existing `rationale_present`).
**Steps.**
1. Populate `RD.disposition_code` in the refdata corpus
   (`10_domains.nir`'s `#[reference]` block / `80_refdata`-equivalent):
   FP/false-hit/duplicate/known-fraud + hold/release codes.
2. Declare the invariant (`00_core.nir` style): disposition write requires
   non-empty rationale + valid code.
3. Wire 3.3/4.10/11.2 writes through the real field policy (no UI-only
   validation).
**Done-when.** A write with an unknown code or empty rationale is denied by
the guard (integration test proves the deny, not just the render).

## B2 ☑ T-14 kanban drag transitions (S) — done
**Goal.** `kanban_board!` grays out drags the `workflow!` machine forbids,
instead of rejecting post-hoc.
**What shipped.** Read the registered machine catalog for real, rather than
a second `transitions:` literal: discovered `WORKFLOWS`/`WorkflowRecord` were
declared but never populated (`workflow!` only ever wrote its free-text
source into `CATALOG`) — closed that gap the same way Plan Phase 15 closed
the identical one for `approval_chain!`/`APPROVAL_CHAINS`. `workflow!` now
also parses its `machine { a -> b -> [c,d]; ... }` body into real edges and
emits a const `WorkflowRegistration` into `WORKFLOWS`
(`nirdosha-guard-macros`/`nirdosha-guard-registry`). `kanban_board!` gained
an optional `machine: "Name"` clause
(`crates/nirdosha-macros/src/kanban_board.rs`) that calls the new
`nirdosha_guard_registry::workflow_allowed_transitions(name)` at request
time and passes the per-card allowed-columns map into `board_html`
(`crates/nirdosha-rt/src/board.rs`), which renders `data-allowed-to` on each
card; `board_js` grays disallowed columns pre-drop (`kanban-column
--disallowed`, `pointer-events:none`) and refuses the drop, never a
post-hoc-only reject. 4.1's mount (`examples/rtm/src/screens/m04_cases.nir`)
now passes `machine: "CaseStatus"`.
**Done-when.** Illegal drag renders grayed pre-drop; legal drag's column
stays live. Proven by
`case_board_drags_gray_out_transitions_the_real_casestatus_machine_forbids`
(`examples/rtm/tests/m03_m04_m05_screens.rs`). 4.1 flipped `blocked_by = []`,
`stage = "built"`.

## B3 ☐ T-02 forbidden=absent drop pass (S)
**Goal.** `field_policy { forbidden(x) }` renders the field **absent** — no
input, no placeholder, no `"[DROPPED]"` cell. Closes the tipping-off hole
where masking leaks a field's *existence*.
**Steps.**
1. In `crud_screens!`'s list/detail/form emission
   (`crates/nirdosha-macros/src/crud_screens.rs`), drop forbidden fields
   from the rendered column/field sets instead of masking values.
2. Add an emit-level assertion inside the macro: a forbidden field name
   appearing in generated HTML strings is a compile error. **Caution:**
   whitelist structural tokens (CSS class prefixes, dataset attribute
   names) so legitimate macro plumbing is not caught; only literal field
   identifiers in labels/placeholders/values should fail.
3. Regression: `sar_linked` absent (not masked) on 3.2 for non-MLRO roles —
   extend an existing m03 test rather than adding a new file.
**Done-when.** Emit-level check passes for every `crud_screens!` call site;
m03 test asserts absence.

## B4 ☐ T-10 role-ident canonicalization (S)
**Goal.** menus.toml's PascalCase logical roles (`ComplianceLead`, `Mlro`)
map to the PascalCase runtime `RoleProof<R>` types the guard registry
already registers; also unblocks `crud_screens!`'s `requires role "..."`
grammar (currently it parses the role string into a snake_case ident that
does not match RTM's PascalCase `role Analyst;` declarations — the
workaround is `public` verbs + `guard:` blocks).
**Steps.**
1. Central mapping fn in `nirdosha-rt` (logical → ident), reused by
   `app_shell_from_toml!` (which already emits `has_role("Analyst")`
   literals) and `crud_screens!`'s role grammar.
2. Emit-time error when a menus.toml role has no mapping (named error, per
   tickets.md T-10 done-when).
**Done-when.** A menus.toml role edit referencing an unmappable role fails
at emit time with a named error; RTM screen files can switch from the
`public`-verb workaround to real `requires role` declarations.

## B5 ☐ T-12 search/filter predicate binding — I15 (S)
**Goal.** `q`/filter inputs compile into guarded WHERE clauses restricted by
`predicate_use(...)` grants; masked fields excluded from WHERE/JOIN/GROUP/
ORDER.
**Steps.**
1. Wire `FilterExpr` pushdown into `crud_screens!`'s list handler
   (currently in-process substring over `guarded_snapshot`).
2. Reject filters on non-`predicate_use` fields with a named reason.
**Done-when.** Integration test: granted filter returns policy-correct
rows; ungranted filter is a named deny. Removes the "decorative search"
disclosure on 3.1/6.1.

**Performance note.** The current in-process substring scan over the
snapshot is O(N) per keystroke and bypasses `max_scan_rows`. Predicate
pushdown must still respect the listing policy's `row_cap` and
`max_execution`; debounce `q` at ≥150 ms and abort scans that exceed the
policy cap with a named reason rather than silently truncating.

## B6 ☐ T-09 live streaming plane (M)
**Unlocks.** Builds 11.1 Interception Queue; upgrades 2.5 wall, 10.5
rescreen progress, 19.1 health.
**Steps.**
1. Topic consumer feeding screen-pushable state (`K.guard.decisions` et al)
   behind the existing `refresh_seconds` combo (poll→push).
2. 11.1 countdown columns derive from live `PG.payment` hold state.
3. 19.1 module self-health via `notify(topic)`.
**Done-when.** A `refresh_seconds` screen receives topic-driven updates in
a test harness; countdown ticks without full page reload.

**Performance note.** Push must not outrun the browser's render budget or
the guard's audit rate. Cap update frequency to ≤5 Hz per widget, coalesce
concurrent topic messages, and provide a client-side kill switch that
falls back to the existing `refresh_seconds` poll path if the WebSocket/
SSE path stalls.

## B7 ☐ T-01 governed egress/export path (M)
**Unlocks.** 12.10, 15.5, 18.9, 20.3 built; 7.5 gate.
**Steps.**
1. `PG.governed_export` dataset + request/approval/expiry record.
2. O.* writer with watermark + encryption + share-link expiry metadata.
3. **Route `crud_screens!`'s CSV export through this path** — this is the
   fix for the `guarded_snapshot` bypass at
   `crates/nirdosha-macros/src/crud_screens.rs:503-580` (D1 partially).
4. Per-class approval hooks incl. `sar_release` quorum(2) for SAR egress.
**Done-when.** A screen export produces a purpose-tagged, approved,
watermarked artifact; any bypass of the path fails emit/verify.

**Performance note.** Watermarking and encryption of large exports must
be streaming/chunked; materializing a full result set in memory before
writing O.* will hit `max_result_bytes` and likely OOM. Use the guard's
`row_cap` and `max_execution` as backpressure, not just post-hoc checks.

## B8 ☐ T-11 unified audit-chain projection (M)
**Unlocks.** 14.5, 20.2 built; 3.10/20.1 (with B11); 12.11 tipping-off
controls.
**Steps.**
1. `AC.all_chains` projection over the per-domain append-only chains,
   immutability preserved (append-only at the projection layer too).
2. `PG.sar_visibility` visibility rules at the projection layer.
3. `timeline`/`ac_timeline` combos read from it.
**Done-when.** One guarded query answers cross-chain search; sar visibility
holds; export still egresses via B7.

## B9 ☐ T-07+T-08 SAR wizard framework + datasets (M+M)
**Unlocks.** 4.14, 12.2, 12.3, 12.4 built; 12.5 (with `PG.evidence_doc`,
C10); upgrades 3.8.
**Steps.**
1. `wizard_persistent` combo: wizard state as a guarded entity row
   (survives restart/timeout/handover). In-memory wizard FORBIDDEN for SAR.
2. `PG.sar_draft` (versioned narrative + attestations) + `PG.sar_bundle`
   (subjects, `RD.jurisdiction`/`RD.activity_code`, filing payload).
3. Attachments via `PG.evidence_doc`/O.* with redaction-confirm and
   "≥1 statement to submit".
4. Attestation step (who/what/when/where/why/how) on the narrative editor.
**Done-when.** Crashed mid-wizard session resumes from the guarded draft
row (integration test); SAR wizards physically cannot run from memory.

## B10 ☐ T-04 approval chains + `approval_inbox!` (L) — the big one
**Unlocks (12).** 4.11, 8.8, 11.3, 12.6 (archetype `approval_inbox!`);
5.10, 9.5, 15.5, 18.7 (`approval_inbox!` combo inside another archetype);
plus 4.12, 8.3, 8.10, 13.1 stage-flips. **Note:** 8.3/8.10/13.1 also need
`graph_apply_flow` / `risk_factor` / `window` work from C2/C4; B10 clears
their approval blocker only.
**Steps.**
1. `approval_inbox!` macro in `crates/nirdosha-macros/src/` (new file +
   lib.rs registration + nirdosha-rt re-export — 2 existing-file edits,
   declared here per R2).
2. Wire `RUNTIME.pending_approvals` as the inbox dataset;
   `PendingApproval`/`EscalatedWrite` runtime already exists — reuse, do
   not fork.
3. Chain semantics: quorum, `timeout(deny)`, cooling period on approve,
   return-with-reason; maker≠checker + return-reason machine-checked.
4. Emit the four archetype screens; flip the other eight from
   emittable/interim.
5. Menus.toml nav.approvals (`stage_min = "interim"`) auto-appears via
   `app_shell_from_toml!` — no shell edit needed (Blocker E pays off).
**Done-when.** Mint → inbox render → approve/return round-trips with
quorum/timeout enforced; maker≠checker deny proven by test.

## B11 ☐ T-06 cross-entity context + `workspace!` (L)
**Unlocks (10).** 4.3, 5.2, 9.6 (`workspace!`); upgrades 3.4, 4.4, 4.5,
6.2 to full fidelity; 3.2/3.10/20.1 context feeds.
**Steps.**
1. Promote `render_custom_screen`/`render_workspace` (currently hand-built
   `LayoutNode` trees in `crates/nirdosha-rt/src/showcase_screens.rs` — the
   unverified-HTML anti-pattern) into declarative `workspace!`.
2. One guarded context read per declared need (related/alerts,
   case/transactions, entity/360, explainability, timeline) — policy-capped,
   not per-screen bespoke joins.
3. Emit 4.3 Investigation Workspace, 5.2 Customer 360, 9.6 Explainability.
**Done-when.** A workspace screen declares its context needs once and gets
one guarded, capped read; hand-built LayoutNode paths retired.

**Performance note.** "One guarded context read" must not become one
giant join that pulls an unbounded cartesian product. Each declared need
(related alerts, transactions, 360 summary, explainability, timeline)
should translate to a separate, capped sub-plan that the runtime can run
in parallel and merge under a shared `BudgetToken`; a single sub-plan
hitting its cap must fail the whole workspace read with a named reason,
never render partial context silently.

---

# GROUP C — independent planes (dataset + archetype/engine + screens)

## C1 ☐ PG.case_task task plane (M) — 5 screens
3.7, 4.8, 4.9, 14.4, 16.4 + feeds 2.2's task widget and T-04's inbox later.
**Note.** 14.4 also carries `dataset:PG.qa_review`; C1 alone only unblocks
its `case_task` blocker. If C2 (qa_review table) lands first, 14.4 still
needs this table before it can stage-promote.
**Steps.** `PG.case_task` dataset → `case_task_table()` in `bridge.nir` →
guard policies (read/create/update per screen-field-mappings.md §tasks) →
`crud_screens!` mounts. **Done-when.** Task CRUD guard-enforced; 2.2's
`blocked_by` drops.

## C2 ☐ Model-stats plane (M) — 6 screens
`PG.model_run_stat` (2.6, 9.2, 9.4), `PG.feature_stat` (9.3),
`PG.model_validation` (9.5). **Note:** 9.6 Explainability embed remains
blocked by `ticket:T-06` (workspace/context) even after its
`dataset:PG.model_run_stat` blocker is gone.
**Steps.** Dataset from the live scoring path
(`40_scoring.nir` emits runs — persist them) → tables/policies → dashboard
mounts. **Done-when.** 9.x module fully built; 2.6 exec KPI unblocked.

## C3 ☐ M7 graph stream (L) — 4 screens
`GR.link_edge` store + `LinkGraphRuntime` engine + `graph_renderer`
archetype. Upgrades 7.1 from `interim` to `built`; builds 7.2, 7.3;
unblocks 7.5 export gate (needs B7 for export path).
**Steps.** Embedded edge store (reuse `nirdosha-lineage`'s GraphStore port
pattern — precedent exists in `m07_network.nir`) → runtime query surface →
SVG/canvas renderer macro reading the guarded result. **Done-when.** A
path query renders with per-node guarded fields; export routes via T-01.

## C4 ☐ M8 rules stream (L) — 9 screens
`CATALOG.catalog_projection` directly unlocks 8.1, 8.9, 9.1, 18.2.
`PG.rule_stat` unlocks 8.12. `graph_apply_flow` engine + `rule_builder!`
archetype unlocks 8.2, 8.4, 8.5; 8.3 is blocked by both `engine:graph_apply_flow`
(`C4`) and `ticket:T-04` (`B10`), so it clears only when both land.
**Note:** 8.10 Scheduling & Go-Live is formally blocked only by `ticket:T-04`
(`B10`) and uses `GR.project_graph`, so it becomes shippable with B10 but
remains empty until C4's engine is live; count it as a B10 unlock, not a
C4 unlock. 8.8 Approval Workflow is an `approval_inbox!` screen and is
entirely owned by B10 — it is **not** unlocked by C4.
**Steps.** Compiled-metadata projection of the `.nir` policy corpus (rules
ARE source — library = compiled-metadata read, per 8.1's notes) → stat
emitter → builder UI emitting graph proposals (RFC 0024 §5.1 — never
runtime files, N-A2). **Depends.** B10 for 8.3/8.10's migrate approval UX.
**Done-when.** A rule edit round-trips: builder → proposal → graph_apply →
release chain.

## C5 ☐ Simulation + what-if (L) — 2 screens
`engine:policy_simulation` + `whatif!` + `phase:RFC0026-P4` (8.6, 8.7).
Policies (`simulate/policy_simulation`) already exist. **Done-when.** A dry-
run produces measured deltas (`alert_volume`, `escalation_load`, ...) with
`obligate audit(full)` consumed.

## C6 ☐ Sankey funds-flow (M) — 1 screen
`GR.flow_edge` + `sankey!` archetype (6.4). Same renderer family as C3.

## C7 ☐ Reporting stream (M) — 3 screens
`PG.report_def`, `PG.scheduled_report` + `report_builder!` (15.1, 15.2,
15.3). Export egress via B7.

## C8 ☐ Refdata/config singles (L) — ~15 screens
M13 (`RD.country_risk`, `RD.product_risk`, `RD.pep_category`,
`PG.global_params`, `PG.risk_factor`) + M18 (`assignment_rules`,
`sla_config`, `RD.disposition_code`←B1, `notif_template`,
`retention_config`←B10, `delegation`←B10, `license_info`). Uniform
`settings_screen!`/`crud_screens!` shapes; batch them per-module.

## C9 ☐ M19 ops monitors (M) — ~8 screens
`svc_health`, `pipeline_stat`, `ingest_exception`, `recon_report`,
`integration_stat`, `batch_job`, `capacity_stat`, `sync_stat` — dashboards
over precomputed stat tables; several get live upgrades from B6.

## C10 ☐ Long-tail singles (M) — ~10 screens (deduplicated)
Datasets that feed screens not already covered by B/C tickets above:
`PG.unlock_request` (1.6), `PG.saved_view` (2.7), `PG.kyc_doc` (5.3),
`PG.adverse_media`+`news_provider` integration (5.7), `PG.entity_flag`
(5.9), `PG.device_session` (6.8, 7.4), `PG.case_note` (4.7, 16.2),
`PG.handover` (16.3), `PG.reporting_calendar` (12.8),
`PG.timeout_config` (11.4). Some rows below reference screens whose
primary blocker is handled by an earlier ticket; they are listed here only
as cross-references, not additional work: `PG.customer_restriction`
(5.10←B10), `PG.evidence_doc` (4.6, 12.5←B9), `PG.sar_submission`+goAML
(12.7, 12.10←B7), `PG.sar_visibility` (12.11←B11), `PG.allowlist`
(8.11←C4), `PG.governed_export` (15.5←B7), `engine:delegation_mint`
(21.2←B10), `PG.dsar_request` (18.9←B7), `PG.list_versions`/
`PG.internal_list` (10.3, 10.4←C8), `PG.rescreen_job`/
`PG.rescreen_campaign` (10.5, 10.6←C9), `PG.rule_stat`/`PG.feature_stat`
(see C2/C4).

---

# GROUP D — standing

## D1 ☑ Macro governance holes (absorbed)
Export bypass → absorbed by **B7** step 3. Forbidden-masked → absorbed by
**B3** (T-02). PascalCase roles → absorbed by **B4** (T-10). Pagination /
`FilterExpr` pushdown → absorbed by **B5** (T-12). `refresh_seconds` in
`crud_screens!`/`kanban_board!` → absorbed by **B6** (T-09). Durable
wizard store → absorbed by **B9** step 1. No standalone D1 work remains.

## D2 ⊘ Delegated IdP screens — M1.2–M1.5
Never built here (project decision). `stage = "delegated"` is terminal;
V10 keeps them out of menus. No task.

---

# Sequencing

```
A1+A2 (one commit)  ──►  41 screens at emittable-or-better
   │
B1 (T-03,S) ─ B2 (T-14,S) ─ B3 (T-02,S) ─ B4 (T-10,S) ─ B5 (T-12,S)
   │                        (S-ticket band; any order, all cheap)
B6 (T-09,M)   B7 (T-01,M)   B8 (T-11,M)   B9 (T-07+08,M)
   │
B10 (T-04,L) ─► C4 (needs approvals UX)
B11 (T-06,L)
C1 ─ C2 (independent M planes)
C3 (M7 graph) ─ C6 (sankey shares renderer family)
C8 ─ C9 ─ C10 (batchable singles, schedule last — lowest unblock/risk ratio)
```

**Critical path to the bulk of the register:** A1 → B10 → C4.
B11 and C3 are large but parallelizable once B10's context machinery is
available; C1/C2/C5-C10 are largely independent planes. Everything else
parallelizes off the core path.

# Ledger (append-only; keep honest)

| Date | Work | Tests touched |
|------|------|---------------|
| — | Blocker E closed: `app_shell_from_toml!` generates shell/landing from `menus.toml` | screen suite |
| — | Blocker D closed: `verify_screen_inventory.rs` (V1/V2/V3/V5/V6/V10); 3 policies added; 1 menu waiver; 1 screen role fix | verify_screen_inventory |
| — | screens-plan.md revision: fixed A1 emittable count (41 not 23), A2 role mismatch, B10 approval_inbox grouping, C4 unlock count, added performance notes | verify_screen_inventory, verify_tickets_and_blockers, verify_tickets |
| — | A1+A2 landed: 34 stale blockers removed, 17 screens → emittable (41 built+emittable), pins emptied; `user-role-read` policy added to src/90_ops_admin.nir + roles doc; two-agent split codified in agent-a-engine.md / agent-b-dataplanes.md | all 12 suites green |
| 2026-09-22 | B1 (T-03) landed: disposition vocabularies + rationale/code invariants (00_core.nir + bridge.nir both halves); 3.3/4.10/11.2 → built | all rtm suites green |
| 2026-09-22 | B2 (T-14) landed: `workflow!` now populates the real `WORKFLOWS` registry slice (was declared, never emitted into); `kanban_board!` gained `machine:` to read it and gray illegal drags pre-drop; 4.1 → built, blocked_by emptied | all rtm suites green |
| 2026-09-22 | B1 (T-03) landed: closed vocabularies (`DISPOSITION_CODES`/`CASE_DISPOSITION_CODES`/`HOLD_REASON_CODES`, 10_domains.nir) + 3 new invariants (`disposition_code_valid`/`case_disposition_valid`/`hold_reason_valid`, 00_core.nir + bridge.nir check_invariant arms), wired into `analyst-disposition-alert`/`case-transition`/`ingest-create-hold`; 3.3→built, 4.10→built, 11.2→built, 4.1 drops T-03 (T-14 only remains); executed single-agent (agent-a-engine.md's bridge.nir restriction lifted — no concurrent Agent B this run) | all rtm suites green (verify_tickets corpus-facts recomputed: 48 refs/40 lines/8 stage-gating+4 note-only) |