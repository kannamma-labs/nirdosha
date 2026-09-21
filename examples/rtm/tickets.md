# ============================================================================
# tickets.md — the ticket legend for `ticket:T-xx` references
#
# Definition site for every ticket id cited by screens.toml (in `blocked_by`
# arrays and `notes` lines) and menus.toml (in comments and route notes).
# screens.toml and menus.toml stay untouched; this file is what makes their
# references resolvable instead of dangling.
# Consumed by: tests/verify_tickets.rs   (cargo test -p rtm)
#
# Machine contract (parsed and enforced by the test):
#   - one `## T-xx — <title>` section per ticket, ids exactly T-01..T-14
#   - `- status:`          active | reserved   (reserved = referenced nowhere)
#   - `- size:`            S | M | L | unassigned
#   - `- meaning:`         one line, single sentence
#   - `- blocked-screens:` comma-separated screen ids from screens.toml whose
#                          `blocked_by` carries `ticket:T-xx`; `none` if none
#   - `- note-mentions:`   comma-separated screen ids whose `notes` cite the
#                          bare ticket id; `none` if none
#
# The test enforces: every T-xx token appearing anywhere in screens.toml or
# menus.toml resolves to a section here; each legend's blocked-screens set is
# EXACTLY the screens.toml `blocked_by` reality (both directions); each
# note-mentions set is EXACTLY the screens whose `notes` cite the ticket
# (both directions); reserved slots carry zero references; the corpus facts
# below match live counts.
# ============================================================================

## Reading the legend

**How tickets gate screens.** `ticket:T-xx` in a screen's `blocked_by` means
the screen ships nothing (or only its declared `interim` shape) until the
ticket closes — same semantics as a `dataset:`/`archetype:` blocker, but for
in-repo application work rather than data/engine/archetype availability. A
bare `T-xx` in a `notes` line is softer: the screen already renders
decoratively or interim, and the note says which capability becomes real when
the ticket lands. menus.toml cites tickets the same way in comments (e.g.
nav.exports' "request form works pre-T-01; downloads gate on it") and in
meta.roles_note (T-10).

**Size scale.**

| Size | Rough scope |
|---|---|
| S | one emit-side pass, vocabulary, or mapping change; single-file work |
| M | one guarded dataset plus one integration point (approval/quorum, stream, writer) |
| L | cross-cutting runtime + archetype work gating ten or more screens |

**Corpus facts this legend is pinned to** (asserted by tests/verify_tickets.rs):
39 `ticket:T-…` references on 33 `blocked_by` lines across the 152-screen
register (6 lines block on two tickets at once); 11 of the 14 ticket slots
carry references — 6 stage-gating tickets
(T-04, T-06, T-07, T-08, T-09, T-11) and 5 note/menu-level
only (T-01, T-02, T-03, T-10, T-12); T-05 and T-13 are reserved and referenced nowhere.
T-14 closed its only gate (4.1) and is no longer referenced anywhere in the
corpus (it stays `active`, not `reserved` — a closed-out ticket keeps its
slot's identity, it just currently gates/notes nothing). T-01 (B7) closed
all 5 of its stage-gating blockers the same way and moved to note-only
(menus.toml's nav.exports/sar-export route comments still cite it).
The register's own header line ("blocked_by: ticket:T-xx | dataset:<id> | …")
is format documentation, not a reference.

---

## T-01 — Governed egress/export path
- status: active
- size: M
- meaning: every guarded CSV export is purpose-mandatory, watermarked, content-hashed; per-class quorum (sar_release) stays dependent on T-04
- blocked-screens: none
- note-mentions: none

**Landed this pass.** `nirdosha_rt::export::write_governed_export` — refuses
an empty purpose or an over-`MAX_EXPORT_ROWS` row count before writing
anything (cap as backpressure, not post-hoc truncation), assembles the
artifact in fixed-size chunks, content-hashes it (sha256), writes it into a
real in-memory `O.*` object store under a fresh `export_id`, and appends a
watermark footer line (purpose, export_id, exported_at, expires_at, sha256)
to the returned artifact. `crud_screens!`'s guarded CSV export route (the
documented `guarded_snapshot`→`Response::csv` bypass) now calls this before
returning any bytes — fixed for every `guard:`-gated screen, not just the
five originally named. `bridge.nir`'s new `governed_export_table()` is a
real `PG.governed_export` `GuardedTable`, populated via a sink
(`nirdosha_rt::export::set_export_sink`, registered at boot in
`src/bin/serve.nir`) so every minted export also produces a real row there.

**Disclosed gap, not faked (R3).** Per-class approval hooks — specifically
`sar_release` quorum(2) for SAR bundle egress — are NOT wired into this
path. They depend on the `approval_chain!`/`RUNTIME.pending_approvals`
machinery T-04 (B10) builds, which doesn't exist yet. Note: a DIFFERENT,
pre-existing mechanism already provides real quorum-gated export for
alert/case/transaction resources —
`GuardedTable::guarded_propose_escalated_export`/
`guarded_confirm_escalated_export`, mounted at `/exports/{resource}/
propose|confirm` in `m15_reporting.nir`'s `mount_governed_export` (JSON API,
no HTML screen, predates this ticket) — but it does not watermark/hash/cap
its artifact the way this pass's path does. The two mechanisms are
currently separate; unifying them (or deciding one supersedes the other) is
left for whoever builds B10's `approval_inbox!` and 15.5's screen for real.

**Gates.** No screen is stage-blocked by `ticket:T-01` any more — 7.5, 15.5,
18.9, 20.3 stay blocked on their OTHER real blockers (`archetype:
graph_renderer`+`dataset:GR.link_edge`; `ticket:T-04`; `dataset:
PG.dsar_request`; `ticket:T-11`, respectively); 12.10 stays blocked on
`integration:goAML` (external gateway, out of this ticket's scope). Menus.
toml's nav.exports note ("request form works pre-T-01; downloads gate on
it") is now stale — the download path is real; 15.5's screen itself still
doesn't exist pending T-04.

**Done when.** A screen-declared export produces a purpose-tagged, approved
(or quorum-released), watermarked artifact in O.* carrying expiry metadata,
and any egress bypassing the path fails emit/verify.

---

## T-02 — Drop-mask render enforcement (forbidden = absent)
- status: active
- size: S
- meaning: field_policy forbidden(x) renders the field absent, not masked or disabled
- blocked-screens: none
- note-mentions: 3.2

**Scope.** The emit-side drop pass that honors `field_policy { forbidden(...) }`
at render time: a forbidden field produces no input, no placeholder, no
disabled control — absence, never masking. This is the screen-layer face of
the tipping-off rule already enforced in the policy corpus (`forbidden(sar_linked)`
in src/50_alerts.nir, 95_qa.nir, 96_restricted_views.nir) and the V7 menu rule
("absence, never disabled-state").

**Gates (closed).** No screen is stage-blocked on it. Feature gate on 3.2
Alert Detail: "sar_linked must be absent (T-02 drop-mask)" — closed:
`AlertRow.sar_linked` is `Option<String>`, absent (no JSON key at all) for
every role `analyst-read-alert`/`analyst-disposition-alert` mask it from.

**Done when.** Every emitted screen omits forbidden fields entirely — no
masked placeholder, no empty cell — verified by an emit-level check. ✓
`nirdosha_guard_screens::field_policy::read_masks_from_field_policy` now
synthesizes `MaskTransform::Drop` (not `Full`) for `field_policy {
forbidden(...) }`, and `masking::apply_one` removes the JSON key entirely
for `Drop` instead of writing a `"[REDACTED]"`/`"[DROPPED]"` placeholder.
Applied to every real forbidden field currently reachable through a read
policy: `AlertRow.sar_linked`, `CaseRow.sar_id`, `CustomerRow.{name,
national_id, dob, risk_rating, pep_flag, sanctions_status}`,
`PaymentRow.{rail_ref, originator, beneficiary, amount, hold_reason,
decision_by, decision_rationale}` — all converted from a masked-shaped
`String` to `Option<String>` with `#[serde(skip_serializing_if =
"Option::is_none")]` so a dropped key decodes to `None` and stays absent on
the way back out. `crud_screens!`'s own ungated `__parse`/`core_fns` helpers
(previously unconditionally generated and type-checked even on guard-only
screens, requiring `Default`/`Display` on every declared field type) are now
gated off `input.guard.is_none()` — true dead code elimination, not just
unused, since nothing in a guarded screen ever called them. Regression:
`g1_read_side_forbidden_field_policy_is_now_dropped_not_placeholder_masked`
(`nirdosha-guard-screens`), `cs_agent_sees_only_the_real_allowed_payment_
fields_forbidden_fields_are_genuinely_absent` /
`rm_user_sees_only_restriction_status_every_other_customer_field_is_
genuinely_absent` (`examples/rtm/tests/m19_m20_m21_m22_screens.rs`).

---

## T-03 — Disposition vocabulary + rationale invariant
- status: active
- size: S
- meaning: shared reason-code taxonomy and mandatory-rationale invariant across alert/case/payment dispositions
- blocked-screens: none
- note-mentions: 7.1

**Scope.** Closed reason-code vocabularies declared in `10_domains.nir`
(`DISPOSITION_CODES`, `CASE_DISPOSITION_CODES`, `HOLD_REASON_CODES`) and bound
to the disposition machines: close-as-FP/false-hit/duplicate/known reason
codes with mandatory free-text rationale on alerts (3.3, `analyst-disposition-
alert`'s `requires invariant(rationale_present) requires invariant
(disposition_code_valid)`), case disposition & closure (4.10, `case-
transition`'s matching `case_disposition_valid`; "CaseStatus machine;
alert_ids immutable"), and payment hold reason codes (11.2, `ingest-create-
hold`'s `hold_reason_valid`; "I3 revalidate at commit; above-authority
auto-routes to 11.3"). Board/board-column semantics for 4.1 derive from the
same disposition machine (its remaining blocker is T-14's drag graying only).
Each invariant is real on both sides: `00_core.nir`'s catalog-registered
function and `bridge.nir`'s matching `check_invariant` runtime match-arm.

**Gates (closed).** 3.3 Disposition Panel (built), 4.1 Case Queue / Board
(interim — T-14 is its only remaining blocker), 4.10 Disposition & Closure
(built), 11.2 Hold Decision (built).
7.1's note ("corrected sample's T-03 ref") is a historical correction note,
not a gate — the graph sample once cited the wrong ticket; left as-is.

**Done when.** Every disposition write validates its reason code against the
closed set and refuses to commit without the rationale invariant. ✓ — proven
by `disposition_code_valid`/`case_disposition_valid`/`hold_reason_valid`
wired into `analyst-disposition-alert`/`case-transition`/`ingest-create-hold`
and enforced at the row layer in `bridge.nir`.

---

## T-04 — Approval chain runtime + approval_inbox! emission
- status: active
- size: L
- meaning: maker≠checker approval inbox backed by RUNTIME.pending_approvals and approval_chain! quorum/timeout/cooling
- blocked-screens: 4.11, 4.12, 5.10, 8.3, 8.8, 8.10, 9.5, 11.3, 12.6, 13.1, 15.5, 18.7
- note-mentions: none

**Scope.** The approval engine the whole compliance posture hangs off:
`approval_chain!` chains (quorum, timeout(deny), cooling period on approve,
return-with-reason) surfaced as the `approval_inbox!` archetype with
RUNTIME.pending_approvals as the inbox dataset. Covers dual-control purge
(18.7), validator≠owner (9.5), diff-vs-current + cooling period (8.8),
authority-limit checks (11.3), and sar_release quorum(2) consumption (12.6).

**Gates.** Twelve screens — 4.11 Four-Eyes Review, 4.12 Escalation to MLRO,
5.10 Restriction/Exit Recommendation, 8.3 Threshold & Parameter Editor, 8.8
Approval Workflow, 8.10 Scheduling & Go-Live, 9.5 Model Validation &
Governance, 11.3 Override/Exception Approval, 12.6 MLRO Review & Decision,
13.1 Customer Risk Model Config, 15.5 Governed Export Center, 18.7 Data
Retention & Purge. Largest ticket in the register. Menus.toml: the approvals
inbox nav item "appears the moment T-04 mounts anything" (stage_min=interim).

**Done when.** pending_approvals round-trips: mint → inbox render →
approve/return with the chain's quorum/timeout semantics enforced and
maker≠checker + return-reason invariants machine-checked.

---

## T-05 — Reserved slot
- status: reserved
- size: unassigned
- meaning: unallocated ticket id held so the T-01..T-14 range stays contiguous
- blocked-screens: none
- note-mentions: none

**Scope.** None yet. No screens.toml blocked_by, note, or menus.toml line
references T-05 (verified by tests/verify_tickets.rs). The slot is reserved
up-front so a future ticket keeps its neighbors' ids stable instead of
renumbering.

**Promotion rule.** The commit that first references T-05 must, in the same
step, fill in this section (status/size/meaning/gates) — docs and code change
together.

---

## T-06 — Cross-entity linked-context assembly
- status: active
- size: L
- meaning: one guarded cross-entity context read feeding workspaces and tabs (related alerts, txn scope, 360, explainability, audit timeline)
- blocked-screens: 3.2, 3.4, 3.10, 4.3, 4.4, 4.5, 5.2, 6.2, 9.6, 20.1
- note-mentions: none

**Scope.** The linked-context layer every detail/workspace screen composes
from instead of N ad-hoc joins: related alerts on the same
customer/counterparty/period, transactions inside a case's scope
(PG.case_txn_scope), Customer 360 roll-ups, score-explainability embeds, and
per-entity audit timelines. Feeds the `workspace!` archetype, the
`linked_detail`/tab combos, and cross-chain search (20.1 pairs it with T-11's
unified chain store). menus.toml: nav.audit_log — "T-11 projection; timeline
via T-06".

**Gates.** Ten screens — 3.2 Alert Detail, 3.4 Related & Linked Alerts, 3.10
Alert Audit History, 4.3 Investigation Workspace, 4.4 Linked Alerts Tab, 4.5
Case Transactions Tab, 5.2 Customer 360, 6.2 Transaction Detail, 9.6 Score
Explainability Viewer, 20.1 Audit Log Search. Most of these ship an
`interim = crud/table` shape today; full fidelity waits on this ticket.

**Done when.** A screen declares its context needs once (related/alerts,
case/transactions, entity/360, ...) and the emitted screen gets one guarded,
policy-capped context read — not per-screen bespoke joins.

---

## T-07 — Case→SAR conversion + persistent wizard framework
- status: active
- size: M
- meaning: wizard state lives in a guarded entity (wizard_persistent), never in memory; case-to-SAR promotion built on it
- blocked-screens: 3.8, 4.14, 12.2, 12.3, 12.4
- note-mentions: none

**Scope.** The `wizard_persistent` combo's backing: multi-step wizard state
stored as a guarded entity row so a wizard survives restart, timeout, and
handover — "in-memory wizard FORBIDDEN for SAR — draft must be guarded
entity" (12.2's note). On top of it, the escalation/convert flow: escalate
alert→case (3.8, menus.toml: "single-form now; persistent wizard after T-07"),
initiate SAR from a case past investigating (4.14), SAR Draft Wizard (12.2),
Narrative Editor with who/what/when/where/why/how attestations (12.3),
Subjects & Activity Tabs with case-scoped lookup and totals-consistency
advance-blocking (12.4).

**Gates.** 3.8 Escalation to Case (interim single-form today), 4.14 Initiate
SAR, 12.2 SAR Draft Wizard, 12.3 Narrative Editor, 12.4 Subjects & Activity
Tabs.

**Done when.** A crashed mid-wizard session resumes from the guarded draft
row, and SAR wizards physically cannot run from memory.

---

## T-08 — SAR draft/bundle datasets + attachments
- status: active
- size: M
- meaning: PG.sar_draft and PG.sar_bundle guarded tables with subject/activity registries and document attachments
- blocked-screens: 4.14, 12.2, 12.3, 12.4, 12.5
- note-mentions: none

**Scope.** The SAR data plane the T-07 wizard writes into: PG.sar_draft
(versioned narrative + attestations), PG.sar_bundle (subjects, activity codes
RD.jurisdiction/RD.activity_code, assembled filing payload), and supporting
document attachments via PG.evidence_doc/O.* storage with redaction-confirm
and "≥1 statement to submit" (12.5).

**Gates.** 4.14 Initiate SAR, 12.2 SAR Draft Wizard, 12.3 Narrative Editor,
12.4 Subjects & Activity Tabs, 12.5 Supporting Attachments. Always cited
alongside T-07 (T-07 = flow, T-08 = storage).

**Done when.** sar_draft/sar_bundle rows exist as guarded tables with the
attachment pipeline, and 12.10's filing export can read a completed bundle.

---

## T-09 — Live streaming plane for screens
- status: active
- size: M
- meaning: the refresh_seconds poll mechanism is real everywhere it's declared; true topic-push stays a later ticket
- blocked-screens: 19.1
- note-mentions: 2.5, 10.5, 11.1, 19.1

**Scope.** `refresh_seconds` existed only on `dashboard!`/`communication_feed!`
before this ticket — `crud_screens!` had the combo declared in `screens.toml`
(2.5, 10.5, 11.1) but silently ignored it (D1's disclosed governance hole).
`crud_screens!` now supports `refresh_seconds` (a real `<meta
http-equiv="refresh">` poll, same mechanism `dashboard.rs` already used),
plus two new opt-in clauses, `sort_by:` (ascending) and `countdown_field:`
(renders an epoch-seconds field as a live "Xm Ys remaining"/"EXPIRED"
string computed fresh every request — no client-side timer). 2.5's Real-Time
Monitoring Wall and 11.1's Interception Queue are both built on top: real
`ops-read-holds`-guarded reads over `PG.payment`, sorted soonest-expiring-
first, auto-refreshing. **What this ticket does NOT build**: real `K.*`
topic consumption or push-based delivery — that's still poll (a real page
reload), not push. 11.1's countdown/sort were never actually a Kafka
concern; deriving from `PG.payment`'s own `hold_expires_at` at request time
was always sufficient and is what's built. 2.5's OTHER named dataset,
`K.guard.decisions` (a live decision ticker), has no real topic consumer or
queryable projection anywhere in this corpus — `GuardClient`'s own JSONL
audit log is file-backed and unqueried by any screen (same gap `m03_alerts
.nir`'s 3.10 doc comment already discloses) — so 2.5 stays `stage =
"interim"`, not `built`. 19.1's own T-09 gap, "modules self-report via
notify(topic)", is untouched — `dashboard!`'s `refresh_seconds` already
existed pre-ticket, so nothing this ticket built closes 19.1's real gap; it
stays the sole `blocked-screens` entry. 10.5 dropped its `ticket:T-09` half
(the poll mechanism it needs is now real) but stays `blocked` on
`dataset:PG.rescreen_job` (C9, not yet landed).

**Gates.** 19.1 System Health Dashboard remains stage-blocked. 2.5 (interim,
real poll now — see its own notes), 10.5 (still blocked, dataset-only now),
11.1 (built) all dropped `ticket:T-09` from their `blocked_by`.

**Done when.** A screen marked refresh_seconds receives real topic-driven
updates (poll → push), and countdown columns derive from live payment
state. Countdown half: ✓ (11.1/2.5, both real, proven by
`tests/m07_m11_m12_m17_screens.rs`'s
`hold_queue_renders_a_real_auto_refreshing_sorted_countdown_html_screen`
and `monitoring_wall_auto_refreshes_and_shows_a_live_sorted_countdown`).
Topic-push half: not done — poll is real, push is a later ticket (no
`K.*` topic consumer exists in this corpus yet).

---

## T-10 — Role ident canonicalization
- status: active
- size: S
- meaning: maps PascalCase logical roles (menus.toml) to runtime `nirdosha_roles::<Role>` types; an undeclared role fails the build
- blocked-screens: none
- note-mentions: none

**Scope.** menus.toml writes logical roles PascalCase ("ComplianceLead",
"Mlro"). `nirdosha_contract_core::role::role_ident` (already the mapping
`crud_screens!`'s `requires role "..."` grammar uses) is the one central
mapping fn; `app_shell_from_toml!` now reuses it for every role its nav
guards and `[landing]` rules consume. Each role gets (1) a shape check via
`role_ident` at macro-expansion time and (2) a dead type-alias, `type
__AssertRoleDeclared_<Role> = crate::nirdosha_roles::<Role>;`, forcing the
*consuming* crate's own `rustc` to resolve that path — an undeclared role
fails with a named "cannot find type `<Role>` in module `nirdosha_roles`"
error, not silently-unreachable nav. This also completes the V5 probe
input — every route.guard {action, resource} must resolve to an existing
guard_policy! record ("public must reference guard:").

**Gates.** No individual screen. Register-wide: the whole menu/nav emission
and the V5 guard-reference check. Cited only in menus.toml comments.

**Done when.** A menus.toml role/route edit that references a nonexistent
guard record or unmappable role fails at emit time with a named error. ✓ —
`crates/nirdosha-macros/src/app_shell_from_toml.rs`'s
`every_consumed_role_gets_a_type_existence_assertion` /
`declared_role_gets_the_same_assertion_and_expands_cleanly` tests prove the
assertion is wired; `examples/rtm`'s own build is the live proof for all 11
of RTM's declared roles.

---

## T-11 — Unified audit chain projection + tipping-off visibility
- status: active
- size: M
- meaning: per-domain AC.* chains projected into one queryable store (AC.all_chains) plus PG.sar_visibility tipping-off controls
- blocked-screens: 3.10, 12.11, 14.5, 20.1, 20.2, 20.3
- note-mentions: none

**Scope.** Consolidate the per-domain append-only chains (AC.alert_chain,
AC.case_chain, AC.entity_chain, AC.access_log, ...) into the AC.all_chains
projection that audit screens search, with the `timeline`/`ac_timeline`
combos reading from it; plus PG.sar_visibility as the tipping-off visibility
projection (12.11), overturned-decision records (14.5), and config-change
history (20.2). Export of the projection still egresses via T-01 (20.3).

**Gates.** 3.10 Alert Audit History, 12.11 Tipping-Off Controls, 14.5
Overturned Decisions Log, 20.1 Audit Log Search, 20.2 Config Change History,
20.3 Regulatory Audit Pack Export. Menus.toml: nav.audit_log — "T-11
projection; timeline via T-06".

**Done when.** One guarded projection answers cross-chain audit queries with
immutability intact, and sar visibility rules hold at the projection layer.

---

## T-12 — Search/filter predicate binding (I15)
- status: active
- size: S
- meaning: q/filter parameters bind to real WHERE clauses limited by predicate_use grants instead of rendering decoratively
- blocked-screens: none
- note-mentions: 3.1, 6.1

**Scope.** The I15 face of search: `q` on list screens compiles into a real,
policy-restricted predicate — `GuardedTable::guarded_search` (new,
`crates/nirdosha-guard-screens/src/lib.rs`) ORs a case-insensitive substring
match only across whichever of the screen's declared fields the winning
`Allow` record's `grant predicate_use(...)` actually names; a masked/
ungranted field can never be searched into existence. Applied in-process
after decode — the same "coarse driver pushdown + fine in-process filter"
posture `apply_subject_scope_in_process` already uses for `subject_scope()`,
since arbitrary payload-field predicates aren't yet driver-pushable
(`read_scope_clauses`'s own doc comment; a separate, pre-existing gap, not
new). `crud_screens!`'s guarded list/JSON-list routes call it instead of
the old dead `q` capture (guarded reads never actually filtered on `q` at
all before this — worse than decorative). `analyst-read-alert`
(`50_alerts.nir`) gained a real `grant predicate_use(status, assignee,
model_version, policy_version, txn_id)` so 3.1 has something to search on;
6.1's transaction grant already existed.

**Gates.** No screen is stage-blocked. 3.1 Alert Work Queue and 6.1
Transaction Search both closed the "decorative search" disclosure — see
their `screens.toml` notes.

**Done when.** A filter on a non-predicate_use field is rejected with a named
reason, and a granted filter returns policy-correct results. ✓ —
`examples/rtm/tests/m03_m04_m05_screens.rs`'s
`q_search_on_the_alert_queue_uses_the_real_predicate_use_grant` /
`q_search_on_customers_is_a_named_deny_no_predicate_use_grant_exists_at_all`
and `tests/m06_transactions_screen.rs`'s
`q_search_on_a_granted_predicate_use_field_returns_policy_correct_rows` /
`q_search_cannot_reach_a_field_outside_the_predicate_use_grant` prove both
halves against real HTTP routes. (`customer`'s `auditor-read` grants no
predicate_use at all — a real corpus gap, not invented for this ticket —
so its `q` is the "zero eligible fields → named deny" proof; alert/
transaction prove the "granted filter returns policy-correct rows" half.)

---

## T-13 — Reserved slot
- status: reserved
- size: unassigned
- meaning: unallocated ticket id held so the T-01..T-14 range stays contiguous
- blocked-screens: none
- note-mentions: none

**Scope.** None yet. No screens.toml blocked_by, note, or menus.toml line
references T-13 (verified by tests/verify_tickets.rs). Reserved for the same
reason as T-05.

**Promotion rule.** Identical to T-05: the commit that first references T-13
fills this section in the same step.

---

## T-14 — Kanban drag transitions via workflow! table
- status: active
- size: S
- meaning: board drag-and-drop transitions validated against the workflow! state machine (graying disallowed moves)
- blocked-screens: none
- note-mentions: none

**Scope.** The interaction layer on 4.1 Case Queue / Board: drag-and-drop
column moves check the workflow! table's allowed transitions and gray out
illegal drags rather than rejecting them post-hoc — "board+list shippable;
drag graying T-14 from workflow! table". Builds on T-03's disposition machine
for which transitions exist.

**Gates.** None — closed. 4.1 Case Queue / Board is `stage = "built"`:
`kanban_board!`'s `machine: "CaseStatus"` clause reads the real
`workflow!`-registered transition graph (via the new
`nirdosha_guard_registry::WORKFLOWS` slice / `workflow_allowed_transitions`)
and grays every column a card's current status cannot legally reach.

**Done when.** A drag to a column the CaseStatus machine forbids renders
grayed/disallowed before drop, and allowed drags emit the guarded transition.
✓ Proven by `case_board_drags_gray_out_transitions_the_real_casestatus_machine_forbids`
(examples/rtm/tests/m03_m04_m05_screens.rs).