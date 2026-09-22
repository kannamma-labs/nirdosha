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
11 `ticket:T-…` references on 11 `blocked_by` lines across the 152-screen
register (0 lines block on two tickets at once); 9 of the 14 ticket slots
carry references — 2 stage-gating tickets
(T-06, T-09) and 7 note/menu-level
only (T-01, T-02, T-03, T-04, T-07, T-10, T-12); T-05 and T-13 are reserved and referenced nowhere.
T-08, T-11, and T-14 each closed their only gates and are no longer
referenced anywhere in the corpus (they stay `active`, not `reserved` — a
closed-out ticket keeps its slot's identity, it just currently gates/notes
nothing). T-01 (B7), T-04 (B10), and T-07 (B9) closed all their
stage-gating blockers and moved to note-only: T-01 via menus.toml's
nav.exports/sar-export route comments; T-04 via menus.toml's
nav.approvals/nav.exports route comments ("appears the moment T-04 mounts
anything", "screen itself still gates on T-04" — both pre-date this
closure and are now historical, like 7.1's T-03 correction note); T-07 via
3.8's screens.toml note + menus.toml's escalate route note.
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
- meaning: maker≠checker approval_inbox! over a real cross-entity pending-approvals read; approval_chain! quorum/timeout/cooling/return-with-reason
- blocked-screens: none
- note-mentions: none

**Scope (closed).** `ApprovalChainRuntime` (`nirdosha-guard-core`) gained
`cooling_period_ms` (a `cooling(days|hours|minutes|seconds = N)` clause,
parsed by `approval_chain!`), `return_with_reason` (mandatory non-empty
reason, machine-checked, not a UI hint), `finalize` (resolves a `Cooling`
escalation once its window elapses, no fresh approval needed), and
`list_pending`. `GuardedTable` gained `list_pending_approvals`,
`guarded_return_escalated`, and `guarded_finalize_escalated_action` on top
of the pre-existing `guarded_propose/confirm_escalated_{update,action,
export}` pairs (which already implemented real maker≠checker and
quorum — found already built per-module across M4/M8/M9/M11/M12/M13/M15/
M18 before this ticket, not invented here). New `approval_inbox!` macro
(`crates/nirdosha-macros/src/approval_inbox.rs`) merges one or more named
`GuardedTable`s' pending escalations into one real cross-entity worklist,
with a generic return-with-reason action and per-row links out to the
entity's own real propose/confirm screen (a cross-entity list has no way
to know the domain-specific mutation fields a specific confirm needs —
see that file's own doc comment).

**Landed.** Four dedicated inboxes — `mount_case_review_inbox` (4.11,
`case_review`), `mount_hold_override_inbox` (11.3, `override_release`),
`mount_sar_release_inbox` (12.6, `sar_release`), `mount_policy_release_inbox`
(8.8, `policy_release` across window/refdata/model) — plus
`mount_unified_approvals_inbox` at `/approvals` (menus.toml's real
`nav.approvals` target) merging all of the above and 15.5's
`egress_release` escalations. `policy_release` also gained
`cooling(days = 1)` (real go-live cooling, screen-field-mappings.md's
"cooling period on approve"), which flipped 4 pre-existing HTTP tests'
final assertions from an immediate 200 commit to a 202 cooling-pending
state (M8/M13/M18/M19's own migrate/replay/grant tests) — the underlying
maker≠checker/quorum mechanics those tests prove are unchanged, only the
post-quorum resolution timing is.

**Screens.** 4.11, 11.3, 12.6 → `built` (archetype, dataset, and
guard_policy! all real). 8.8 → `interim`, not `built`: the merged
`policy_release` worklist is real, but its declared `GR.project_graph`
rule-dependency diff view is C4's engine and stays honestly empty (same
posture `m08_rules.nir`'s `v10_findings`/`policy_simulation` already use —
screens-plan.md's own C4 section: "8.8 is entirely owned by B10... not
unlocked by C4"). 15.5 → `interim`: `governed-export-read` (new
`guard_policy!`) and a real `GET /exports` land, but B7's own disclosed
reconciliation gap is still open (the quorum path commits via
`scan_allowed`, not yet through B7's watermarked `write_governed_export`).
4.12, 8.3, 8.10, 5.10, 9.5, 13.1, 18.7 keep whatever OTHER blocker they
already carried (`dataset:`/`engine:`) — T-04 was the only thing closing
for them; per-screen detail in `screens-plan.md`'s B10 ledger entry.

**Done when (met).** Mint → inbox render → approve/return round-trips with
quorum/timeout enforced (`nirdosha-guard-core`'s own
`approval_chain::tests`, `nirdosha-guard-screens`'s
`escalated_update_with_cooling_stays_pending_until_finalized_after_the_window`/
`guarded_return_escalated_blocks_finalize_and_requires_a_real_reason`/
`list_pending_approvals_reflects_cooling_and_resolved_state`, and rtm's own
`approval_inbox_lists_a_real_pending_escalation_and_return_with_reason_closes_it`
over real HTTP); maker≠checker deny proven by the pre-existing
`escalated_update_blocks_self_review_and_leaves_it_pending`/
`admin_grant_role_is_a_real_maker_neq_checker_escalation` tests, unchanged.

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
- meaning: real, capped guarded context reads (related alerts, txn scope, audit timeline) merged under one shared budget that fails whole-not-partial
- blocked-screens: 5.2
- note-mentions: 12.4

**Scope, as closed (B11).** The declarative `workspace!` archetype
(`crates/nirdosha-macros/src/workspace.rs`, `nirdosha_rt::workspace`)
retired the hand-built `LayoutNode`/`render_workspace`/`render_custom_screen`
anti-pattern (`showcase_screens.rs`, zero call sites before this ticket).
Each declared "context need" is a real guarded sub-read
(`bridge.nir`'s `related_alerts_for_case`/`case_transactions_for_case`/
`related_alerts_for_alert`/`related_alerts_for_transaction`/
`entity_audit_timeline`) run in parallel and merged under a shared
`WorkspaceBudget` — `nirdosha_rt::workspace::assemble_context` fails the
WHOLE assembly with a named reason on any sub-read error or budget
overflow, never a partial render (proven by unit tests in
`workspace.rs`). `PG.case_txn_scope` is real: derived from a case's
linked alerts' `txn_id`s and materialized via `system_write`, read
through a real `case-txn-scope-read` policy (`60_case_management.nir`).

**Gates closed for real.** 3.2, 3.4, 4.4, 4.5, 6.2 → `built` (real guarded
context reads, own routes). 3.10, 20.1 → `built` (`entity_audit_timeline`
reuses T-11/B8's `AC.all_chains` real projection under `purpose(Audit)`).
4.3 Investigation Workspace → `interim` (`workspace!` real, two panels;
`PG.case_evidence` still unbuilt — that dataset gap is its own remaining
blocker, unrelated to this ticket). 9.6 → still `blocked`, but ONLY on
`dataset:PG.model_run_stat` (C2) now — the archetype itself is real.

**5.2 deliberately NOT closed.** No `guard_policy!` anywhere in this
corpus grants `Analyst`/`ComplianceLead`/`McpCopilot` (5.2's own declared
roles) any read on `customer` — only `RmUser` (one masked field) and
`Auditor` (broad) — `m05_customer.nir`'s own doc comment discloses this
pre-existing gap. Inventing a grant to make the screen "work" is a policy
decision outside this ticket's authority (same restraint that file's own
comment already establishes), so `ticket:T-06` stays on 5.2 as the real
remaining blocker — this corpus's `blocked_by` vocabulary has no
dedicated token for "missing role grant."

**4.3's audit_timeline panel deliberately omitted.** `entity_audit_timeline`
runs under `purpose(Audit)`; `Analyst` — 4.3's own primary declared role —
has no grant on that resource. Since one failing need fails the whole
workspace read (never partial), including it would 422 this screen for
its main user on every request.

**Done when (met, for the closed gates).** A screen declares its context
needs once (related/alerts, case/transactions, audit timeline) and the
emitted screen gets one guarded, policy-capped, budget-merged context
read — not per-screen bespoke joins. Explainability (9.6) and Customer
360 (5.2) each have their own separate, disclosed remaining blocker
(dataset, missing role grant) — this ticket's own scope is closed.

---

## T-07 — Case→SAR conversion + persistent wizard framework
- status: active
- size: M
- meaning: SAR draft state lives only in the guarded sar_bundle entity — no in-memory session ever holds SAR content
- blocked-screens: none
- note-mentions: 3.8

**Scope, as closed.** batch 2's own investigation (documented in
`m12_sar.nir`'s header) judged giving `wizard!` a guarded/persistent mode
disproportionate to one module's forms and used `crud_screens!`'s
`guard: { create_fields, update_fields }` instead — every SAR field write is
already a direct `guarded_insert_checked`/`guarded_update` against
`sar_bundle_table()`, addressable by `sar_id` from the moment the row is
created. There is no cookie-keyed session store involved at any point for
SAR, which satisfies this ticket's real intent (no in-memory SAR wizard,
ever) more directly than a purpose-built stateful wizard mechanism would
have. 3.8 Escalation to Case deliberately keeps its existing single-form
shape (a one-shot guarded write has no multi-step session to lose).

**Disclosed, not built.** The six who/what/when/where/why/how attestation
sub-fields (12.3) and a multi-subject registry (12.4) collapse into single
flat fields (`narrative`, `subject`) rather than being separately modeled —
see 12.2/12.3/12.4's screens.toml notes.

**Gates.** None — 3.8 stays `interim` by design (see its note); 4.14, 12.2,
12.3, 12.4 are `built`.

**Done when.** ~~A crashed mid-wizard session resumes from the guarded draft
row~~ — moot: SAR writes never leave the guarded row in the first place, so
there is no session state to resume FROM (proven by
`sar_state_survives_a_fresh_session_because_it_was_never_in_one` in
`tests/m07_m11_m12_m17_screens.rs`). SAR wizards physically cannot run from
memory: true, no in-memory store is ever used.

---

## T-08 — SAR draft/bundle datasets + attachments
- status: active
- size: M
- meaning: PG.sar_bundle is the one real guarded table backing both draft and bundle state; a field value, not a separate resource
- blocked-screens: none
- note-mentions: none

**Scope, as closed.** `65_sar.nir`'s real policies (`analyst-draft-sar`,
`analyst-edit-sar`, `mlro-decide-sar`, `sar-export`) all name resource
`sar_bundle` — the corpus never actually split draft/bundle into two tables;
`bridge.nir`'s `SarBundleRow`/`sar_bundle_table()` is the one real
`GuardedTable`, real `SarStatus` machine (`draft -> in_review -> [filed,
rejected, do_not_file]; ...`), real `sar_subject_in_case` invariant.
screens.toml's `PG.sar_draft` dataset references were renamed to
`PG.sar_bundle` to match (stale-name correction, same class as A1's stale
blockers). Subjects/activity codes/filing payload live as flat fields
(`subject`, `activity_codes`, `amount_total`, `txn_refs`, `goaml_ref`), not
`RD.jurisdiction`/`RD.activity_code` lookup-backed registries (disclosed
gap).

**Gates.** None. 12.5 Supporting Attachments still shows `stage = "blocked"`
but on `dataset:PG.evidence_doc` alone now (C10's scope) — this ticket no
longer gates it.

**Done when.** sar_bundle rows exist as a guarded table (true) with the
attachment pipeline (still open, C10) and 12.10's filing export can read a
completed bundle (true — `mount_sar_export`'s `guarded_propose/confirm_escalated_export`
reads real `sar_bundle_table()` rows filtered to `confirmed_fraud`,
`sar_export_narrows_to_confirmed_fraud_rows_only_via_the_new_condition_filter`
proves it).

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
- meaning: AC.all_chains + PG.sar_visibility are real, hash-chain-verified projections; no screen is gated on this ticket anymore
- blocked-screens: none
- note-mentions: none

**Scope (closed, B8).** `AC.all_chains` (`bridge.nir::all_chains_table` +
`nirdosha_rt::audit_projection`) reads every table's real, already-written
`ModuleAuditChain` (the hash-chained log `GuardClient` appends to on every
guarded decision — not a second audit mechanism) and merges them, excluding
(and naming) any chain that fails its own hash-chain verification —
immutability enforced at the projection layer, not just the source chains.
`PG.sar_visibility` (`sar_visibility_table`) derives from it: every real
`alert`/`case` read-decision becomes one visibility record. **Disclosed
limitation**: `GuardClient`'s own audit envelope carries the bare resource
*type* (`"alert"`), not a per-row id (a real, separate `nirdosha-guard-mic`
change, out of this ticket's blast radius) — so `sar_visibility` cannot
attribute a read to the *specific* SAR-linked row, only to the resource type.
14.5/20.2/20.3 close fully on this ticket (real routes: `/qa/overturned`,
`/audit/config-history`, `/audit/pack/export`); 20.2's `AC.policy_chain` half
stays undeclared (`guard_policy!` corpus is static, not a runtime dataset).

**Still gated (not by T-11 anymore).** 3.10 Alert Audit History and 20.1
Audit Log Search both keep `ticket:T-06` (cross-entity context / `workspace!`,
B11) — the projection dataset each declares is now real, but their own
context/workspace requirement isn't.

**Done when.** ~~One guarded projection answers cross-chain audit queries
with immutability intact, and sar visibility rules hold at the projection
layer.~~ Done: proven by `nirdosha-rt`'s `audit_projection` unit tests (merge
ordering, tampered-chain exclusion) and the rtm integration test asserting a
real query result + a real B10-independent access record.

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