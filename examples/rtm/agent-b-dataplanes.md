# Agent B — data planes / screens track

You are Agent B of the two-agent execution of `screens-plan.md`. Agent A
runs concurrently on the engine/macros track (see `agent-a-engine.md`).
A1+A2 are DONE (stale blockers purged, 17 screens promoted,
`user-role-read` policy live, full suite green) — your next unit is C1.

## Your task order (one unit = one commit)

| # | Task | First concrete action |
|---|------|----------------------|
| ✓ | A1+A2 (done) | stale-blocker purge + `user-role-read` policy landed |
| 1 | C1 PG.case_task plane | `case_task_table()` in `bridge.nir` (model on `qa_review_table()`), policies per screen-field-mappings.md §tasks, unblock 2.2/3.7/4.8/4.9/14.4/16.4 |
| 2 | C2 model-stats plane | `PG.model_run_stat`/`PG.feature_stat`/`PG.model_validation` from the live scoring path in `40_scoring.nir`; unblock 2.6/9.2/9.3/9.4 (9.5 needs B10's T-04 too; 9.6 stays T-06) |
| 3 | C8 refdata/config singles | M13/M18 datasets (`RD.country_risk` … `PG.license_info`); batch per-module; 18.7/18.8 wait on B10 |
| 4 | C9 M19 ops monitors | `PG.svc_health` … `PG.sync_stat`; several upgrade live when B6 lands |
| 5 | C10 long-tail singles | the deduplicated list in screens-plan.md; skip rows owned by B tickets |
| 6 | C3 (M7 graph) / C6 (sankey) | after B10's context machinery exists; C3 shares renderer family with C6 |

## File ownership

**Yours (edit freely):**
- `src/bridge.nir` — all new `GuardedTable`s, row types, seeds, notify hooks
- `src/10_domains.nir`, `src/16_domains_ops.nir` — new `#[dataset]` declarations
- `src/screens/mXX_*.nir` — `crud_screens!`/`dashboard!`/`settings_screen!` mounts for your datasets
- `tests/verify_tickets_and_blockers.rs` — dataset pins only
- `screens.toml` / `menus.toml` — ONLY `dataset:` blockers of datasets you
  wired (remove `dataset:PG.x`, flip stage, shrink `EXPECTED_STALE_PAIRS`
  in the same commit per R1). Never touch `ticket:` blockers — Agent A's.

**Shared with Agent A (protocol required):**
- `screens.toml` / `menus.toml` — before every register edit:
  `git pull --rebase`, edit, `cargo test -p rtm`, commit immediately.
  If a rebase shows Agent A touched the same screen block, re-derive from
  `verify_tickets_and_blockers -- --nocapture` output, never hand-merge.
- `screen-field-mappings.md` — update §-sections for datasets you wire.

**Never touch:** `crates/nirdosha-macros/*` (Agent A's), `tickets.md`
ticket statuses (Agent A's), `tests/verify_tickets.rs`.

## Per-unit workflow

1. `git pull --rebase`
2. Add dataset + `GuardedTable` + guard policies (match screen `roles`
   exactly — V2) + screen file.
3. Remove the now-wired `dataset:` blockers in `screens.toml`; shrink
   `EXPECTED_STALE_PAIRS`/`EXPECTED_PROMOTION_CANDIDATES` in the same
   change (R1 — the test red-lines any drift).
4. `cargo test -p rtm` — all green (V5 will hard-fail any newly reachable
   screen whose guard has no policy; add the policy or defer the screen
   with a documented pin — never invent silent grants).
5. Commit: `feat(rtm): <plane> — <one-liner>`.
6. Update `screens-plan.md` checkbox + ledger in the same commit (R2).

## Hard invariants (repo law)

- R3: a screen flips stage only when archetype + real GuardedTable +
  guard_policy! all exist — the stale-pin test enforces this mechanically.
- No orphan grants: a policy naming a role the screen's `roles` list
  doesn't have is a design bug (update screen roles first, same commit).
- Bridge tables follow the file's own conventions: flat screen projections
  (string-typed masked fields, `id` synth keys), disclosed `raw_driver_seed`
  bootstrap, `HUMAN_ROLES` + service-principal subject sets.

## Handoffs you owe Agent A

- **C1 lands** → tell Agent A: `PG.case_task` is real; B10's inbox can
  mount task rows; 2.2's tasks widget unblocked.
- **C2 lands** → tell Agent A: 9.5/13.1's `risk_factor`/`model_validation`
  datasets exist for the approval-coupled screens.
- **Any dataset wired** → tell Agent A: its `guard_policy!` resources are
  now real for policy authoring (R4 reachability).

## Known traps (from the A1+A2 run)

- The test's V5 pair-check is subject-agnostic: `auditor-read`
  (96_restricted_views.nir) already covers `(read, …)` pairs for
  sar_bundle/screening_hit/qa_review/customer/payment. Do NOT invent
  duplicate read policies; check `policy_guard_pairs()` coverage first.
- New policies must also be mirrored into `roles-N-guard_policy.md` (the
  `.nir` corpus header claims verbatim provenance — R2).