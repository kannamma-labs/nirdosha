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
| B3 | T-02 forbidden=absent drop pass (S) | B | S | gate | ☑ done | — |
| B4 | T-10 role-ident canonicalization (S) | B | S | all | ☑ done | — |
| B5 | T-12 predicate binding / I15 (S) | B | S | gate | ☑ done | — |
| B6 | T-09 live streaming plane (M) | B | M | 4 | ☑ done (poll half; 19.1 still blocked) | — |
| B7 | T-01 governed egress (M) | B | M | 5 | ☑ done (core path; sar_release quorum deferred to B10) | — |
| B8 | T-11 audit-chain projection (M) | B | M | 4 | ☑ done | — |
| B9 | T-07+T-08 SAR wizard + datasets (M+M) | B | M | 5 | ☑ done (no wizard_persistent — see notes) | — |
| B10| T-04 approvals + `approval_inbox!` (L) | B | L | 12 | ☑ done (3/12 fully built; 9 partial/unblocked-only, disclosed) | — |
| B11| T-06 linked context + `workspace!` (L) | B | L | 10 | ☑ done (9/10; 5.2 disclosed) | — |
| C1 | PG.case_task task plane | C | M | 5 | ☑ done | — |
| C2 | Model-stats plane | C | M | 6 | ☑ done (3/6 built; 9.3/9.4/9.6 interim, disclosed) | — |
| C3 | M7 graph (edge store + runtime + renderer) | C | L | 4 | ☐ pending | — |
| C4 | M8 rules (catalog + builder) | C | L | 9 | ◐ dataset half only (`CATALOG.catalog_projection`+`PG.rule_stat` real, disclosed) | B10 (approvals for migrate UX) |
| C5 | Simulation + what-if (RFC0026-P4) | C | L | 2 | ☐ pending | — |
| C6 | Sankey funds-flow | C | M | 1 | ☐ pending | — |
| C7 | Reporting builder | C | M | 3 | ◐ dataset half only (`PG.report_def`+`PG.scheduled_report` real, disclosed) | — |
| C8 | Refdata/config singles (~15 screens) | C | L | ~15 | ◐ dataset half only, disclosed | — |
| C9 | M19 ops monitors (~8 screens) | C | M | ~8 | ◐ dataset half only, disclosed | — |
| C10| Long-tail singles (~10 screens) | C | M | ~10 | ◐ dataset half only, disclosed | — |
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

## B3 ☑ T-02 forbidden=absent drop pass (S) — done
**Goal.** `field_policy { forbidden(x) }` renders the field **absent** — no
input, no placeholder, no `"[DROPPED]"` cell. Closes the tipping-off hole
where masking leaks a field's *existence*.

**What actually shipped (differs from the plan below — disclosed, not
silently substituted, per R3).** `forbidden(...)` is a *runtime* policy
fact resolved per (role, purpose) at evaluation time — `crud_screens!`
never sees it at compile time, so a macro-level "drop the field from the
column set" or "compile error on a literal field name in generated HTML"
(this section's original steps 1–2) has nothing to check against; the
macro emits identical code regardless of which policy ends up matching a
given request. The real fix lives one layer down, where forbidden-ness
*is* known (`nirdosha_guard_screens::field_policy::read_masks_from_field_
policy`, evaluated per request): it now synthesizes `MaskTransform::Drop`
instead of `Full` for `field_policy { forbidden(...) }`, and
`masking::apply_one` removes the JSON key outright for `Drop` rather than
writing a placeholder string. Every render path (`list_html`/
`detail_html`/`form_html`/JSON API) already renders off the row's own
JSON keys, so a genuinely-absent key is genuinely absent everywhere for
free — no per-screen HTML-string scanning needed. The entity struct field
a dropped key decodes into must be `Option<T>` (not a masked-shaped bare
`String`) so decode doesn't hard-fail on the missing key; converted every
field currently reachable through a real read policy's `forbidden(...)`:
`AlertRow.sar_linked`, `CaseRow.sar_id`, `CustomerRow.{name, national_id,
dob, risk_rating, pep_flag, sanctions_status}`, `PaymentRow.{rail_ref,
originator, beneficiary, amount, hold_reason, decision_by,
decision_rationale}`. Surfaced one real, separate bug along the way:
`crud_screens!`'s ungated `__parse`/`core_fns` helpers were
unconditionally generated and type-checked (`Default`+`Display` bounds on
every declared field) even on guard-only screens that never call them —
now gated off `input.guard.is_none()`, true dead-code elimination.

**Steps (original plan, superseded by the above).**
1. ~~In `crud_screens!`'s list/detail/form emission drop forbidden fields
   from the rendered column/field sets instead of masking values.~~
2. ~~Add an emit-level assertion inside the macro: a forbidden field name
   appearing in generated HTML strings is a compile error.~~
3. Regression: `sar_linked` absent (not masked) on 3.2 for non-MLRO roles —
   done, extended `examples/rtm/tests/m19_m20_m21_m22_screens.rs` (the CS/RM
   masked-view tests, which already exercised the same real bug for
   `PaymentRow`/`CustomerRow`) and `nirdosha-guard-screens`'s own
   `g1_read_side_forbidden_field_policy_is_now_dropped_not_placeholder_
   masked` unit test, rather than m03 (m03's `AlertRow` fields subject to
   `forbidden(sar_linked)` were never in that screen's declared HTML
   `fields:` list to begin with — the live leak was the JSON API path).
**Done-when.** Every real forbidden-on-read field renders absent
(`row.get(name).is_none()`), proven by the tests above; `cargo test -p rtm`
and `cargo test -p nirdosha-guard-screens` both green.

## B4 ☑ T-10 role-ident canonicalization (S) — done
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

**Landed.** The central mapping fn already existed —
`nirdosha_contract_core::role::role_ident` (`crud_screens!`'s `requires
role "..."` grammar was already using it; it accepts both snake_case wire
names and bare PascalCase `roles! { role X; }` type names). What was
missing was `app_shell_from_toml!` reusing it: every role its nav guards
and `[landing]` rules consume now gets `role_ident`'s shape check at
macro-expansion time, plus a dead type alias `type
__AssertRoleDeclared_<Role> = crate::nirdosha_roles::<Role>;` that forces
the consuming crate's own `rustc` to resolve the path — an undeclared role
is now a named "cannot find type" compile error instead of silently-dead
nav. `crates/nirdosha-macros/src/app_shell_from_toml.rs` gained two unit
tests proving the assertion is emitted for both a stale/typo'd role and a
genuinely declared one; `examples/rtm`'s own `cargo build -p rtm` is the
live proof for all 11 of RTM's declared roles. Did not do a corpus-wide
sweep of screens off the `public`-verb workaround onto `requires role`
declarations — out of this ticket's real scope once the mapping fn turned
out to already exist; that's cosmetic cleanup, not a blocker closing.

## B5 ☑ T-12 search/filter predicate binding — I15 (S) — done
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

**Landed.** Real *store-level* `FilterExpr` pushdown for arbitrary payload
fields (step 1 as literally written) isn't possible yet — a pre-existing,
disclosed gap (`read_scope_clauses`'s own doc comment: `MemStoreDriver`/
`PostgresStoreDriver`'s flat `resource`/`tenant`/`payload` schema can't
resolve a payload-field predicate at the driver). Built the same
"coarse driver pushdown + fine in-process filter" shape the codebase
already uses for `subject_scope()`: new `GuardedTable::guarded_search`
(`nirdosha-guard-screens/src/lib.rs`) ORs a case-insensitive substring
match only across whichever of the screen's fields the winning policy's
`grant predicate_use(...)` actually names, applied in-process after
decode/mask; zero eligible fields for a non-empty `q` is a named
`GuardScreenError::Denied`, not silent. `crud_screens!`'s guarded list/
JSON-list routes call it instead of the old `q` capture — which, on the
`guard:` path, never filtered anything at all (worse than decorative:
`q` was accepted and displayed but `matched` came straight from
`guarded_snapshot`, unfiltered). Discovered `analyst-read-alert`
(3.1's policy) had zero `predicate_use` grant at all — added a real one
(`status, assignee, model_version, policy_version, txn_id`) rather than
leaving 3.1 permanently un-searchable; mirrored into
`roles-N-guard_policy.md`'s verbatim copy. Performance note's debounce/
scan-abort half is client-side/follow-on UX, not built this pass — the
existing `row_cap`/`widest_caps` enforcement already bounds the scan
before search narrows it, same tradeoff `apply_subject_scope_in_process`
already accepts. 6.1 gained its missing `file =` register line and
flipped `emittable` → `built` (the file already existed and was tested;
only the search-decorative disclosure was holding its stage back).

## B6 ☑ T-09 live streaming plane (M) — done (poll half only)
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

**Landed (honest partial close — disclosed, per R3).** `refresh_seconds`
existed only on `dashboard!`/`communication_feed!` before this ticket —
`crud_screens!` had the combo declared in `screens.toml` (2.5/10.5/11.1)
but silently ignored it (D1's governance hole). `crud_screens!` gained
real `refresh_seconds` (a real `<meta http-equiv="refresh">` poll) plus
two new opt-in clauses: `sort_by:` (ascending) and `countdown_field:`
(renders an epoch-seconds field as a live "Xm Ys remaining"/"EXPIRED"
string computed fresh every request — no client-side timer, matching
`screens.rs`'s "no client-side JS" posture; new
`nirdosha_rt::screens::{now_epoch_secs, format_countdown}` helpers).
11.1 Interception Queue: discovered its read/decide/confirm backend
(`m11_intervention.nir`) was ALREADY real and tested — the only gap was
that `/holds` returned JSON only, no HTML screen existed at all. Now
`/holds` is a real, sorted, auto-refreshing, live-countdown HTML screen
(hand-written, reusing the same `screens.rs` helpers `crud_screens!`'s
new clauses use so the two don't drift); `/api/holds` keeps the JSON
shape. Fixed `menus.toml`'s stale `route = "/intervention"` (nothing was
ever mounted there) to the real `/holds`. → `stage = "built"`,
`blocked_by` emptied. 2.5 Real-Time Monitoring Wall: new `crud_screens!`
over `payment_table()` (`ops-read-holds`, `OpsAnalyst`) at `/wall`, same
refresh/sort/countdown treatment. **Not built — disclosed, not routed
around**: 2.5's other named dataset, `K.guard.decisions` (a live
decision ticker) — no real topic consumer or queryable projection exists
anywhere in this corpus (`GuardClient`'s own JSONL audit log is
file-backed and unqueried by any screen, same gap 3.10's doc comment
already names) — so 2.5 stays `stage = "interim"`, notes updated
honestly. 19.1's real gap — modules self-reporting health via
`notify(topic)` — is untouched (`dashboard!`'s `refresh_seconds` already
existed pre-ticket, so nothing built here closes it); it's now T-09's
sole remaining `blocked-screens` entry. 10.5 dropped the `ticket:T-09`
half of its blocker (poll mechanism now real) but stays blocked on
`dataset:PG.rescreen_job` (C9). **True topic-push (WebSocket/SSE) was
never built** — this ticket's own Performance-note framing (poll→push)
already scoped that as a *later* step; poll is what's real now.
`tests/verify_tickets.rs` corpus-facts recomputed (44 refs/36 lines,
unchanged 7 stage-gating/4 note-only/2 reserved).

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

## B8 ☑ T-11 unified audit-chain projection (M) — done
**Real, first-time fix this unit found and closed**: `nirdosha-guard-screens
::GuardedTable` — every table `bridge.nir` builds, RTM's only write/read
path — calls `GuardClient::evaluate()` directly and never appended to its
own `audit` (`ModuleAuditChain`) field; the `audit_path` every
`GuardedTable::new(...)` call site already passed was dead. Fixed at the
root (`GuardedTable::record_audit_decision`, called after all 9 `evaluate()`
call sites) rather than building a projection over data that would always
have been empty — the actual per-table chains are real now, not just the
projection code that reads them.

**Landed.** `nirdosha_rt::audit_projection` (`project`, tested: merge
ordering, a tampered chain excluded-and-named not merged). `bridge.nir`:
`all_chains_table`/`refresh_all_chains` (`AC.all_chains`, real, sourced from
18 tables' own now-real chains), `sar_visibility_table`/
`refresh_sar_visibility` (`PG.sar_visibility`, resource-type granularity —
disclosed limitation: `GuardClient`'s audit envelope carries the bare
resource type, not a per-row id, so it can't attribute a read to the
*specific* SAR-linked row; a real, separate `nirdosha-guard-mic` change, out
of this unit's blast radius). Real routes: `/audit/chains` (20.1),
`/audit/config-history` (20.2, `window_config`/`user_role`/
`delegation_token` chains only — `AC.policy_chain` has no runtime table,
`guard_policy!` is static corpus), `/sar/tipping-off` (12.11),
`/qa/overturned` (14.5), `/audit/pack/export` (20.3, via B7's governed-export
path). New `guard_policy!`s: `auditor-read` extended to `audit_chain`/
`sar_visibility`; new `mlro-audit-chain-read`, `compliance-lead-audit-chain-
read`, `admin-audit-chain-read`, `compliance-lead-read-qa-review`,
`mlro-read-qa-review`.

**Screens.** 12.11, 14.5, 20.2, 20.3 → `built`. 3.10, 20.1 drop `ticket:T-11`
but keep `ticket:T-06` (their own cross-entity context/`workspace!`
requirement, B11's — the projection dataset each declares is now real, their
context requirement isn't).

**Done-when.** Met: `nirdosha_rt::audit_projection`'s own unit tests plus two
rtm integration tests (`t11_audit_chain_projection_answers_a_real_cross_
chain_query_and_feeds_sar_visibility`, `t11_a_tampered_source_chain_is_
excluded_from_the_projection_not_silently_merged`) prove a real guarded
decision reaches the projection, a real HTTP query answers it, and a
corrupted source chain is excluded and named, never silently merged.

## B9 ☑ T-07+T-08 SAR wizard framework + datasets (M+M) — done, no `wizard_persistent` built
**Unlocked.** 4.14, 12.2, 12.3, 12.4 → `built`. 3.8 stays `interim` by
design. 12.5 still `blocked` on `dataset:PG.evidence_doc` (C10) — its
`ticket:T-08` blocker is gone, that's the only change there.
**What actually landed (deviates from the original plan, disclosed):**
`m12_sar.nir` already existed (an earlier "Phase B" batch, predating this
screens-plan.md sequence) and had already made a real architectural call:
`crud_screens!`'s `guard: { create_fields, update_fields }` over the real
`sar_bundle_table()` (`bridge.nir`), not a `wizard_persistent` combo —
judged disproportionate to build a second stateful-wizard macro for one
module's forms. Every SAR field write is a direct `guarded_insert_checked`/
`guarded_update`, addressable by `sar_id` from creation onward — there is
no cookie-keyed session for SAR at all, which satisfies T-07's real intent
(no in-memory SAR wizard, ever) more directly than a purpose-built stateful
wizard would have. This batch: corrected screens.toml's stale
`archetype = "wizard!"` / `combo = ["wizard_persistent"]` labels to match
that reality (`crud_screens!`, no combo), renamed the stale `PG.sar_draft`
dataset to the real `PG.sar_bundle`, fixed 4 stale menus.toml routes still
naming resource `sar_draft` (V5 was failing on them the moment 4.14/12.2/
12.3/12.4 flipped to `built`), and closed both tickets in `tickets.md`.
**Disclosed, not built:** the six who/what/when/where/why/how attestation
sub-fields (12.3) and a multi-subject registry (12.4) stay collapsed into
single flat fields (`narrative`, `subject`); 12.4's totals-consistency
advance-block isn't enforced (needs T-06's cross-entity context, noted on
T-06's own note-mentions).
**Done-when, re-litigated honestly.** "Crashed mid-wizard session resumes
from the guarded draft row" is moot, not failed: there is no session to
resume FROM, proven by
`sar_state_survives_a_fresh_session_because_it_was_never_in_one`
(`tests/m07_m11_m12_m17_screens.rs`) — a fresh, unrelated identity reads
the draft back by `sar_id` alone. "SAR wizards physically cannot run from
memory": true, trivially, since no in-memory store is ever used for SAR.

## B10 ☑ T-04 approval chains + `approval_inbox!` (L) — the big one — done (disclosed partial, see below)
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

## B11 ☑ T-06 cross-entity context + `workspace!` (L) — done (9/10; 5.2 disclosed)
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

## C2 ☑ Model-stats plane (M) — 6 screens — done (3/6 built; 3 interim, disclosed)
`PG.model_run_stat` (2.6, 9.2, 9.4, 9.6), `PG.feature_stat` (9.3),
`PG.model_validation` (9.5) — all real `GuardedTable`s (`bridge.nir`).
`model_run_stat` is genuinely derived from `alert_table()`'s real
`score`/`model_version`/`disposition_code`/`case_id` fields
(`refresh_model_run_stat`, recomputed fresh on every read, cross-
referencing `sar_bundle_table()` for `sar_conversion_rate`) — never a
hardcoded fixture (proved by `model_run_stat_derives_from_real_alerts_
and_model_validation_enforces_validator_ne_owner`, whose `fp_rate`
assertion depends on the exact seeded disposition mix). `feature_stat` is
demo-seeded reference data (disclosed: no raw per-transaction feature
value is computed or persisted anywhere in this corpus to derive real
PSI from). `model_validation`'s `validator ≠ owner` is machine-checked
(`requires invariant(validator_ne_owner)` on create, `00_core.nir` +
`bridge.nir::ModelValidationRow::check_invariant`).

**Landed:** 2.6 Executive KPI Dashboard (`/exec`, model-health metrics
added to its existing widgets) → `built`. 9.2 Model Performance Monitor
(`/models/performance`) → `built` — AUC/precision/recall/F1 disclosed
not computed (no independent ground-truth signal beyond the same
`disposition_code` already spent on `fp_rate`). 9.3 Drift Monitoring
(`/models/drift`) → `interim` (real plumbing, seeded PSI). 9.4
Champion/Challenger (`/models/challenger`) → `interim` (comparison
chart real; reuses 9.1's real `guarded_propose/confirm_escalated_update`
swap engine for promotion; no dedicated picker/trial UI). 9.5 Model
Validation & Governance (`/model-validations`, `crud_screens!` with real
`create_fields`/`update_fields`) → `built`. 9.6 Score Explainability
Viewer (`/alerts/{id}/explain`, `workspace!`) → `interim` (context alert
+ real `model_run_stat` context for the alert's model_version; factor-
contribution/counterfactual panels not built — no per-feature
explanation output persisted anywhere in this corpus, fabricating one
would violate R3).

**Corrected two stale `menus.toml` routes** (same posture B6/B9 already
established): `/models/{id}/performance` → `/models/performance` (no
`dashboard!` mount in this corpus takes a path param), `/models/{id}/
validation` → `/model-validations` (flat `crud_screens!` resource, not
nested under `/models`). **Mount-ordering fix**: `/models/performance`,
`/models/drift`, `/models/challenger` registered before
`mount_model_screens`'s `/models/{id}` wildcard in both `serve.nir` and
the test router (same "literal before wildcard" rule the SAR/board
mounts already document) — caught live as a 403 "deny by default" on
`resource == "model"` before the fix, not a policy bug.

**Done-when** (as originally written) was "9.x module fully built" —
not fully met: 9.3/9.4/9.6 are real but `interim`, each for a disclosed
reason above, not a shortcut. 2.6 exec KPI is unblocked.

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
| 2026-09-22 | B3 (T-02) landed: `read_masks_from_field_policy` synthesizes `MaskTransform::Drop` (true key removal) instead of `Full` for `field_policy { forbidden(...) }`; `masking::apply_one` removes the key outright for `Drop`; converted every field currently reachable through a real read policy's `forbidden(...)` to `Option<T>` (`AlertRow.sar_linked`, `CaseRow.sar_id`, `CustomerRow.{name,national_id,dob,risk_rating,pep_flag,sanctions_status}`, `PaymentRow.{rail_ref,originator,beneficiary,amount,hold_reason,decision_by,decision_rationale}`); `crud_screens!`'s ungated `__parse`/`core_fns` (dead code on any guard-only screen, previously always type-checked) now gated off `input.guard.is_none()`. Plan's original macro-level approach (steps 1-2) superseded — disclosed in B3's own section — since `forbidden(...)` is runtime policy, invisible to the macro at compile time | `cargo test -p rtm` (63 tests) + `cargo test -p nirdosha-rt -p nirdosha-macros -p nirdosha-guard-screens` (36+68+other suites) all green; full-workspace build has one pre-existing, unrelated failure in `nirdosha-guard-mcp` (`RegistryDump` missing fields) not touched by this change |
| 2026-09-22 | B4 (T-10) landed: the central role-name mapping fn already existed (`nirdosha_contract_core::role::role_ident`, already used by `crud_screens!`); `app_shell_from_toml!` now reuses it for every nav-guard/`[landing]` role, plus emits a dead `type __AssertRoleDeclared_<Role> = crate::nirdosha_roles::<Role>;` alias per role so an undeclared role fails the *consuming* crate's build with a named "cannot find type" error instead of silently-dead nav; 2 new unit tests in `app_shell_from_toml.rs` prove the assertion fires for both a stale/typo'd role and a genuinely declared one; disk was found at 100%/60MB free mid-run (`target/` at 84GB) and `cargo clean` reclaimed 93GB before this unit ran | all rtm suites green; `cargo test -p nirdosha-macros app_shell_from_toml` green (2 new tests) |
| 2026-09-22 | B5 (T-12) landed: new `GuardedTable::guarded_search` (nirdosha-guard-screens/src/lib.rs) — `q` ORs a case-insensitive substring match only across a screen's fields the winning policy's `grant predicate_use(...)` actually names, in-process after decode (driver-level payload-field pushdown is a separate, pre-existing, disclosed gap); zero eligible fields on a non-empty `q` is a named deny; `crud_screens!`'s guarded list/JSON-list routes now call it (previously `q` was captured and displayed but never actually filtered anything on the `guard:` path at all — worse than decorative). `analyst-read-alert` (50_alerts.nir + roles-N-guard_policy.md mirror) gained a real `grant predicate_use(status, assignee, model_version, policy_version, txn_id)` since it had none; 6.1 gained its missing `file =` line and flipped emittable→built | 4 new integration tests (m03_m04_m05_screens.rs, m06_transactions_screen.rs) proving both the granted-rows and named-deny halves through real HTTP routes; all rtm suites green |
| 2026-09-22 | B6 (T-09) landed, poll half only (disclosed): `crud_screens!` gained real `refresh_seconds` (`<meta http-equiv="refresh">`, previously silently ignored — D1's hole) plus `sort_by:`/`countdown_field:` (new `nirdosha_rt::screens::{now_epoch_secs, format_countdown}` helpers, no client-side JS). 11.1 Interception Queue: found its guard-enforced backend already real/tested, only the HTML screen was missing (`/holds` was JSON-only) — built it (sorted, auto-refreshing, live countdown), added `/api/holds` for JSON, fixed menus.toml's dead `route = "/intervention"` → `/holds` → stage built, blocked_by emptied. 2.5 Real-Time Monitoring Wall: new `crud_screens!` over `payment_table()` at `/wall`, same treatment, stays `interim` — its `K.guard.decisions` half has no real topic consumer/projection anywhere in this corpus, disclosed not faked. 19.1's real T-09 gap (notify(topic) health self-report) untouched, remains T-09's sole blocked-screens entry. 10.5 dropped the ticket half, stays blocked on dataset:PG.rescreen_job (C9). True WebSocket/SSE push not built — always scoped as a later step per this ticket's own "poll→push" framing | 3 new integration tests (m07_m11_m12_m17_screens.rs) proving real auto-refresh/sort/countdown on both screens; verify_tickets corpus-facts recomputed (44 refs/36 lines); all rtm suites green |
| 2026-09-22 | B7 (T-01) landed, core path only (disclosed): new `nirdosha_rt::export` module — `write_governed_export` refuses an empty purpose or a >`MAX_EXPORT_ROWS`(50k) row count before writing anything (cap as backpressure, not post-hoc truncation), assembles the artifact in 500-row chunks, sha256 content-hashes it, writes it into a real in-memory `O.*` object store under a fresh `export_id`, and returns a watermark footer line (purpose/export_id/exported_at/expires_at/sha256). `crud_screens!`'s guarded CSV export route (the documented `guarded_snapshot`→`Response::csv` bypass) now calls it before returning any bytes — every `guard:`-gated screen's export, not just the 5 named. `sha2` promoted from `dpop`-feature-optional to a hard `nirdosha-rt` dependency. New `bridge.nir` `GovernedExportRow`/`governed_export_table()` (real `PG.governed_export` `GuardedTable`, `system_write`-only like `NotificationRow` — no screen reads export history yet), wired via a new `nirdosha_rt::export::set_export_sink` hook registered once in `src/bin/serve.nir`'s `main()`. Disclosed gap: per-class approval (`sar_release` quorum(2)) is NOT wired here — depends on T-04/B10's `approval_chain!`/`RUNTIME.pending_approvals`, which doesn't exist yet; found and documented a DIFFERENT pre-existing mechanism (`GuardedTable::guarded_propose_escalated_export`/`guarded_confirm_escalated_export`, mounted at `/exports/{resource}/propose\|confirm` in `m15_reporting.nir`, JSON-only, no watermark/hash) that already does real quorum-gated export for alert/case/transaction — reconciling the two paths is left for B10. screens.toml: dropped stale `ticket:T-01` from 7.5/12.10/15.5/18.9/20.3 (none promote further — each still has its OTHER real blocker: `archetype:graph_renderer`+`dataset:GR.link_edge`, `integration:goAML`, `ticket:T-04`, `dataset:PG.dsar_request`, `ticket:T-11` respectively); 15.5's `dataset:PG.governed_export` also dropped (now real, caught by the stale-pin test). menus.toml nav.exports comment updated (was "pre-T-01", now accurate) | 1 new integration test (`m06_transactions_screen.rs`) proving the watermark footer on a real guarded export; 3 new unit tests in `export.rs`; verify_tickets corpus-facts recomputed (39 refs/33 lines/6 stage-gating+5 note-only, T-01 moved to note-only); all rtm + nirdosha-rt + nirdosha-macros + nirdosha-guard-screens suites green |
| 2026-09-22 | B8 (T-11) landed: discovered `nirdosha-guard-screens::GuardedTable` (RTM's only real read/write path) never wrote to its own `audit` (`ModuleAuditChain`) field — it calls `GuardClient::evaluate()` directly, bypassing `guarded_read`/`guarded_apply`'s audit-append entirely, so the `audit_path` every `bridge.nir` table already passed was dead. Root-fixed: `GuardedTable::record_audit_decision`, called after all 9 `evaluate()` call sites, appends a real `AuditEnvelope` for every decision (allow/deny/escalate/pending alike). New `nirdosha_rt::audit_projection` module (`project`, unit-tested: merge ordering, tampered-chain exclusion). `bridge.nir`: `all_chains_table`/`refresh_all_chains` (`AC.all_chains`, real, over 18 tables' now-real chains), `sar_visibility_table`/`refresh_sar_visibility` (`PG.sar_visibility`, resource-type granularity — disclosed: no per-row attribution, `GuardClient`'s envelope carries the bare resource type, a real separate `nirdosha-guard-mic` change out of this unit's blast radius). Real routes: `/audit/chains` (20.1), `/audit/config-history` (20.2), `/sar/tipping-off` (12.11), `/qa/overturned` (14.5), `/audit/pack/export` (20.3, via B7's path). New `guard_policy!`s: `auditor-read` extended + `mlro-audit-chain-read`/`compliance-lead-audit-chain-read`/`admin-audit-chain-read`/`compliance-lead-read-qa-review`/`mlro-read-qa-review`. screens.toml: 12.11/14.5/20.2/20.3 → `built`; 3.10/20.1 drop `ticket:T-11`, keep `ticket:T-06` (B11's) | 3 new `nirdosha-rt` unit tests + 2 new rtm integration tests (real cross-chain query + tampered-chain exclusion, both in `m19_m20_m21_m22_screens.rs`); `cargo test -p nirdosha-guard-screens` (36, unaffected by the audit-append addition) + `cargo test -p rtm` (all 15 suites) + `cargo test -p nirdosha-rt -p nirdosha-guard-mic` all green; verify_tickets corpus-facts recomputed (33 refs/29 lines/5 stage-gating, T-11 fully closed) |
| 2026-09-22 | B9 (T-07+T-08) landed, `wizard_persistent` NOT built (disclosed, see B9's own section for the full reasoning): discovered `m12_sar.nir`/`bridge.nir`'s `SarBundleRow`/`sar_bundle_table()` already existed, real and tested (an earlier "Phase B" batch, predating this screens-plan.md sequence) — a deliberate prior call to use `crud_screens!`'s `guard: {create_fields, update_fields}` over one real `sar_bundle` `GuardedTable` instead of extending `wizard!` with a second stateful macro. Every SAR write is already a direct guarded call addressable by `sar_id`, with zero in-memory session involved at any point — satisfies T-07's actual intent without new macro code. This batch: corrected screens.toml's stale `archetype = "wizard!"`/`combo = ["wizard_persistent"]` labels on 4.14/12.2/12.3/12.4 to the real `crud_screens!`/no-combo shape, renamed the stale `PG.sar_draft` dataset references to the real `PG.sar_bundle`, fixed 4 menus.toml routes still naming the dead `sar_draft` resource (V5 started failing on them the moment these screens flipped to `built`), dropped `ticket:T-07`/`ticket:T-08` everywhere closed (3.8/4.14/12.2/12.3/12.4/12.5), added a note-mention of `T-06` on 12.4 (its totals-consistency gap needs T-06's context read). 3.8 deliberately stays `interim` (a one-shot guarded write was never going to get a multi-step persistent wizard). 12.5 stays `blocked`, now purely on `dataset:PG.evidence_doc` (C10). Disclosed gaps: narrative's 6-field who/what/when/where/why/how attestation and 12.4's multi-subject registry both stay flat single fields | 1 new integration test (`sar_state_survives_a_fresh_session_because_it_was_never_in_one`, m07_m11_m12_m17_screens.rs) proving a draft is readable by `sar_id` alone from a wholly separate identity that never touched the creating session; all rtm suites green; verify_tickets corpus-facts recomputed (23 refs/23 lines/3 stage-gating+6 note-only, T-07 moved to note-only, T-08 fully closed) |
| 2026-09-22 | B10 (T-04) landed, the critical-path rock — real, not faked, but disclosed-partial per-screen: extended `ApprovalChainRuntime` (`nirdosha-guard-core`) with `cooling_period_ms` (new `cooling(days\|hours\|minutes\|seconds = N)` clause on `approval_chain!`, parsed the same way `quorum(...)` is), `return_with_reason` (mandatory non-empty reason, machine-checked — `ApprovalChainError::EmptyReturnReason`), `finalize` (resolves a `Cooling` escalation once its window elapses, no fresh approval required), and `list_pending`. `GuardedTable` gained `guarded_return_escalated`, `guarded_finalize_escalated_action`, and `list_pending_approvals` (`PendingApprovalRow`) on top of the pre-existing `guarded_propose/confirm_escalated_{update,action,export}` pairs — found ALREADY BUILT, real, tested, across M4/M8/M9/M11/M12/M13/M15/M18 (a predating batch), not invented this unit. New `approval_inbox!` macro (`crates/nirdosha-macros/src/approval_inbox.rs` + `nirdosha-rt/src/approval_inbox.rs`'s decoupled `InboxRow`/render fn, same `Card`-in-`board.rs` split `kanban_board!` already established) merges named `GuardedTable` sources' pending escalations into one real worklist with a generic return-with-reason form; "Review & approve" links out to the entity's own real confirm screen rather than guessing domain-specific mutation fields (a cross-entity list structurally can't know them — see that file's doc comment). `policy_release` gained real `cooling(days = 1)` (screen-field-mappings.md's "cooling period on approve"), which flipped 4 pre-existing HTTP tests' final assertions from immediate-200-commit to 202-cooling-pending (M8/M13/M18/M19's migrate/replay/grant tests) — same maker≠checker/quorum mechanics, different post-quorum timing, all fixed and green. New `guard_policy!`s: `approvals-nav-gate` (pure V5 nav-reachability grant, same posture as A2's `user-role-read`) and `governed-export-read` (bridge.nir's own `GovernedExportRow` doc comment explicitly deferred this "once 15.5 actually needs one" — it does now); `governed_export_table()` wired to the real corpus policies (was `vec![]`/`vec![]`). Five mounts: `mount_case_review_inbox` (4.11), `mount_hold_override_inbox` (11.3), `mount_sar_release_inbox` (12.6), `mount_policy_release_inbox` (8.8, merging window/refdata/model), `mount_unified_approvals_inbox` (menus.toml's real `nav.approvals` target at `/approvals`, merging all of the above plus 15.5's `egress_release` alert/case/transaction export escalations). menus.toml: `nav.approvals`'s `screen_id` moved from 4.11 to 8.8 (V2 needs the primary ref to carry every nav role; only 8.8 lists both ComplianceLead and Mlro). Screens: 4.11/11.3/12.6 → `built` (archetype+dataset+policy all real, no remaining gap). 8.8/15.5 → `interim`, ticket:T-04 dropped: 8.8's `GR.project_graph` rule-dependency diff view is C4's engine, disclosed empty (`m08_rules.nir`'s own `v10_findings`/`policy_simulation` precedent) — screens-plan.md's own C4 section already calls this "entirely owned by B10... not unlocked by C4"; 15.5's quorum path is real but B7's OWN disclosed reconciliation gap (watermarked `write_governed_export` vs. this `scan_allowed` path) is still open. 4.12/8.3/8.10/5.10/9.5/13.1/18.7 (the remaining 7 of 12): only `ticket:T-04` dropped, stage untouched — each still carries its own real, separate `dataset:`/`engine:` blocker (or, for 4.12, no distinct implementation exists yet at all, disclosed in `m04_cases.nir`'s own doc comment) | New: 5 `approval_chain.rs` unit tests (cooling/return/finalize/list_pending), 3 `nirdosha-guard-screens` unit tests (cooling-to-finalize round-trip, return blocks finalize + empty-reason deny, `list_pending_approvals` state transitions), 1 rtm HTTP integration test (`approval_inbox_lists_a_real_pending_escalation_and_return_with_reason_closes_it`, m03_m04_m05_screens.rs) proving mint→list→return→re-confirm-denied over real HTTP. `cargo test -p rtm` (all 15 suites, incl. verify_screen_inventory's V2/V5 and verify_tickets' full pin set) + `cargo test -p nirdosha-guard-core -p nirdosha-guard-screens -p nirdosha-guard-registry -p nirdosha-macros -p nirdosha-rt` all green; verify_tickets corpus-facts recomputed (11 refs/11 lines/2 stage-gating T-06+T-09, T-04 moved to note-only via menus.toml's own pre-existing "T-04" comments) |
| 2026-09-22 | B11 (T-06) landed, 9/10 screens closed for real (disclosed: 5.2 deliberately not mounted): new declarative `workspace!` archetype (`crates/nirdosha-macros/src/workspace.rs` + `nirdosha_rt::workspace`) retires the hand-built `LayoutNode`/`render_workspace`/`render_custom_screen` anti-pattern in `showcase_screens.rs` (confirmed zero call sites anywhere in this corpus before this ticket — pure dead code, deleted). `nirdosha_rt::workspace::assemble_context` runs each declared context need's guarded sub-read on its own OS thread (genuinely parallel), merges under a shared `WorkspaceBudget`, and fails the WHOLE assembly with a named reason on any sub-read error or a running row total that would exceed the budget — never a partial render (proven by 3 unit tests). New real context-read fns in `bridge.nir`: `related_alerts_for_case`/`case_transactions_for_case` (4.3/4.4/4.5 — the latter derives+materializes the real `PG.case_txn_scope` table via `system_write` from the case's linked alerts' `txn_id`s, read back through a new `case-txn-scope-read` policy in `60_case_management.nir`), `related_alerts_for_alert` (3.4 — same-case, or same-customer-transaction-siblings fallback), `related_alerts_for_transaction` (6.2), `entity_audit_timeline` (3.10/20.1 — reuses T-11/B8's real `AC.all_chains` projection under `purpose(Audit)`, no second audit mechanism invented). Mounted: `mount_investigation_workspace` (4.3, 2 panels — `audit_timeline` deliberately excluded since `Analyst`, 4.3's own primary role, has no `audit_chain` grant and one failing need fails the whole page), `mount_case_context_tabs` (4.4/4.5), `mount_alert_context` (3.4/3.10), `mount_transaction_context` (6.2) — all wired into `src/bin/serve.nir`. Screens: 3.2/3.4/4.4/4.5/6.2 → `built`; 3.10/20.1 → `built` (20.1's route was already complete since B8 — this ticket only removed the process-gate blocker, zero code needed there); 4.3 → `interim` (workspace! real, `PG.case_evidence` still unbuilt — its own remaining blocker); 9.6 → still `blocked`, archetype dropped, only `dataset:PG.model_run_stat` (C2) remains. **5.2 Customer 360 deliberately NOT mounted**: `m05_customer.nir`'s own doc comment already discloses that no `guard_policy!` anywhere in this corpus grants `Analyst`/`ComplianceLead`/`McpCopilot` (5.2's own declared roles) any read on `customer` — inventing a grant to make the screen "work" is a policy decision outside this ticket's authority; `ticket:T-06` stays on 5.2 as the honest remaining blocker | 3 new `nirdosha-rt::workspace` unit tests (merge-under-budget, budget-overflow named reason, failing-need named reason); full `cargo test -p rtm` (all 15 suites) + `cargo test -p nirdosha-rt -p nirdosha-macros -p nirdosha-guard-screens -p nirdosha-guard-core` all green; verify_tickets corpus-facts recomputed (2 refs/2 lines — T-06's only survivor is 5.2, T-09's only survivor is 19.1 from B6) |
| 2026-09-22 | C1 (`PG.case_task` plane) landed, first Group C unit: new `CaseTaskRow`/`case_task_table()` (`bridge.nir`), modeled on `qa_review_table()`'s shape — `item_ref` (generic `"alert:<id>"`/`"case:<id>"`/`"qa:<id>"` owning-entity ref, since 3.7 is alert-scoped and 4.8/4.9 are case-scoped) + `task_type` (`"checklist"`/`"rfi"`/`"coaching_action"`) distinguish the five screens' presentations over ONE real table, the same "one resource, several screen views" posture `case`/`alert` already have. New real `machine CaseTaskStatus { open -> in_progress -> [done, blocked, escalated]; blocked/escalated -> in_progress; }` (`10_domains.nir`) + 3 new cross-cutting `guard_policy!`s in `00_core.nir` (`case-task-read` — the same all-human-roles list `notification-read` already established; `case-task-create`/`case-task-update` — `Analyst`/`ComplianceLead`). Mounted: `mount_alert_rfi` (3.7, `m03_alerts.nir`), `mount_case_tasks` (4.8+4.9, `m04_cases.nir`), `mount_qa_task_form` (14.4's `task_form` combo, `m14_qa.nir`), `mount_my_tasks` (16.4, `m16_notifications.nir`), `my_open_task_count` widget (2.2, `m02_dashboards.nir`). **Stale-label correction, same pattern B9 found for SAR**: 14.4's `archetype = "communication_feed!"` was never mounted anywhere in this corpus (grepped, zero call sites) — corrected to `crud_screens!` (its real primary resource, `qa_review`, already uses it); `task_form` stays as the combo label for the new embedded widget. Screens: 2.2/3.7/4.8/4.9/14.4 → `built`; 16.4 → `interim` (case_task half real; the screen's OTHER declared dataset, `PG.alert` — assigned alerts surfaced as tasks per screen-field-mappings.md's "source" column — not merged in this pass, disclosed). `blocked_by` for all six dropped `dataset:PG.case_task`. Not touched: `screen-field-mappings.md` (no prior B/C unit has annotated it; `roles-N-guard_policy.md`, same precedent — B11's own new `case-txn-scope-read` policy isn't mirrored there either) | 1 new integration test (`case_task_rfi_is_real_guard_enforced_crud_over_case_task`, `m03_m04_m05_screens.rs`) proving real create/read/transition over HTTP plus two real denials (Auditor has `case-task-read` but no `case-task-create`; an `open -> done` transition skipping `in_progress` is rejected by the real `CaseTaskStatus` machine); full `cargo test -p rtm` (all 15 suites) green |
| 2026-09-22 | C2 (model-stats plane) landed, 3/6 screens `built` + 3 `interim` (disclosed, not faked): new `ModelRunStatRow`/`model_run_stat_table()` + `refresh_model_run_stat` (`bridge.nir`) — genuinely derives `run_count`/`avg_score`/`fp_rate`/`sar_conversion_rate`/score-band histogram from `alert_table()`'s real `score`/`model_version`/`disposition_code`/`case_id` fields (cross-referencing `sar_bundle_table()` for conversion), recomputed fresh on every read (each snapshot keyed `{version}:{now}:{row_id}` — the atomic counter, not just `now`, because a single dashboard render calls the refresh fn once per widget, colliding within the same second otherwise). New `FeatureStatRow`/`feature_stat_table()` — demo-seeded reference data, disclosed (no raw per-transaction feature value computed/persisted anywhere in this corpus). New `ModelValidationRow`/`model_validation_table()` + catalog invariant `validator_ne_owner` (`00_core.nir` + new `ModelValidation` `#[dataset]`, `10_domains.nir`) — machine-checked on create, mirrored in `bridge.nir::check_invariant`. Mounted in `m09_ml_models.nir`: `mount_model_performance` (9.2, `/models/performance`), `mount_model_drift` (9.3, `/models/drift`), `mount_champion_challenger` (9.4, `/models/challenger`, reuses 9.1's real swap-escalation engine), `mount_model_validations` (9.5, `crud_screens!` with real `create_fields`/`update_fields`), `mount_score_explainability` (9.6, `workspace!` over the alert subject, one real context need — factor-contribution/counterfactual panels deliberately not built, no per-feature explanation persisted anywhere). 2.6's dashboard gained two real model-health widgets. **Two stale menus.toml routes corrected** (`/models/{id}/performance` → `/models/performance`, `/models/{id}/validation` → `/model-validations` — no `dashboard!` mount in this corpus takes a path param; `crud_screens!` resources aren't nested under a different resource's id). **Mount-ordering bug caught live**: the three new `/models/*` literal routes initially returned 403 "deny by default" on `resource == "model"` — they were registered AFTER `mount_model_screens`'s `/models/{id}` wildcard, which swallowed them; fixed by reordering (same "literal before wildcard" rule the SAR/board mounts already document) in both `serve.nir` and the test router | 1 new integration test (`model_run_stat_derives_from_real_alerts_and_model_validation_enforces_validator_ne_owner`, `m08_m09_m10_m13_screens.rs`) — seeds two alerts with a known disposition mix and asserts the dashboard's `fp_rate` comes out to exactly the real derived value (0.5), plus validator==owner is denied and validator!=owner commits and is readable by Mlro; full `cargo test -p rtm` (all 15 suites) green |
| 2026-09-22 | Dataset-plane closeout across C4/C7/C8/C9/C10 (dataset half only, disclosed — no new archetype/screen mounts this pass): a prior in-session batch had already landed ~40 new real `GuardedTable`s in `bridge.nir` (`unlock_request`, `assignment_rules`, `notif_template`, `evidence_doc`, `case_note`, `kyc_doc`, `risk_factor`, `entity_flag`, `customer_restriction`, `device_session`, `list_versions`, `internal_list`, `rescreen_job`, `rescreen_campaign`, `timeout_config`, `sar_submission`, `reporting_calendar`, `global_params`, `report_def`, `scheduled_report`, `handover`, `license_info`, `retention_config`, `delegation`, `dsar_request`, `svc_health`, `pipeline_stat`, `ingest_exception`, `recon_report`, `integration_stat`, `batch_job`, `capacity_stat`, `sync_stat`, `customer_rel`, `adverse_media`, `catalog_projection`, `rule_stat`, `saved_view`, plus `link_edge`/`agg_bins`/`flow_edge` for the still-untouched C3/C6) and a new `src/97_bulk_policies.nir` with a real read-only `guard_policy!` per resource, but never removed the now-stale `dataset:PG.<x>` blockers from `screens.toml` or re-pinned `verify_tickets_and_blockers.rs` — leaving 41 (screen, resource) pairs red on `stale_dataset_blockers_match_the_pinned_set` (R1 violation-in-progress, caught before commit). This unit closed that out mechanically: removed the 41 stale tokens (plus 2 more for `CATALOG.catalog_projection`/`PG.rule_stat` on 8.1, real but not yet in the failing set) from `blocked_by`; 37 screens whose *only* blocker was the closed dataset flipped `blocked`/`interim`→`emittable` (R3: ingredients present, screen not yet written — no archetype/mount work was done, so `built` would have been a fake) — `1.6, 2.7, 4.6, 4.7, 5.3, 5.4, 5.9, 5.10, 6.8, 8.1, 8.12, 10.3, 10.4, 10.5, 10.6, 11.4, 12.5, 12.8, 13.1, 13.5, 15.1, 15.3, 16.2, 16.3, 18.3, 18.6, 18.7, 18.8, 18.9, 18.10, 19.2, 19.3, 19.4, 19.5, 19.6, 19.7, 19.8`; 5 screens kept their dataset blocker removed but stage untouched because a second, real, non-dataset blocker remains: `12.7` (`integration:goAML`), `19.1` (`ticket:T-09`), `5.6` (`archetype:tree_view`), `5.7` (`integration:news_provider`), `7.4` (blocker now empty but its file — `m07_network.nir`'s pre-existing Stage-0 mount — is still txn-only and never reads `device_session_table`, so `interim` stays honest, not `built`). Zero pin-file edits were needed in `tests/verify_tickets_and_blockers.rs` — both `EXPECTED_STALE_PAIRS`/`EXPECTED_PROMOTION_CANDIDATES` were already pinned empty (A1's convention) and are still empty. **Real regression caught along the way**: promoting these screens made `verify_screen_inventory`'s V5 check reach 6 `menus.toml` routes whose `guard = { action = "create", ... }` had no backing policy at all — `/cases/{id}/evidence` (4.6, `evidence_doc`), `/cases/{id}/notes` (4.7, `case_note`), `/customers/{id}/flags` (5.9, `entity_flag`), `/customers/{id}/restriction` (5.10, `customer_restriction`), `/denied` (1.6, `unlock_request`), and `/sar/{id}/attachments` (12.5) which was still guarding on the stale resource name `sar_draft` — a B9-era leftover B9 itself disclosed fixing "4" other routes but missed this one. Added 5 new `create`-side `guard_policy!`s to `97_bulk_policies.nir`, each `for`-list matching the screen's own R/W role subset exactly (not a broader grant), and corrected 12.5's menus.toml guard to the real resource, `evidence_doc`. C4/C7 status: only their dataset half (`catalog_projection`+`rule_stat` for C4, `report_def`+`scheduled_report` for C7) is real; the `rule_builder!`/`report_builder!` archetypes and engine work are untouched. C3/C6 untouched (their `GR.link_edge`/`GR.agg_bins`/`GR.flow_edge` tables exist in the same prior batch but no screen's `blocked_by` names them as a `dataset:` token yet — those screens are blocked on `archetype:graph_renderer`/`archetype:sankey!` instead) | `cargo test -p rtm` (all 15 suites, incl. `verify_screen_inventory`'s V5 and `verify_tickets_and_blockers`'s stale-pin check) green before and after the V5 fix; no pin-file changes required |
