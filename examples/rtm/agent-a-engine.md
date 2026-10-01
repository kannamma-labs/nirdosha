# Agent A — engine / macros track

You are Agent A of the two-agent execution of `screens-plan.md`. Agent B
runs concurrently on the data-planes track (see `agent-b-dataplanes.md`).
Both tracks land work on the same branch; rebase discipline below keeps
the registers conflict-free.

## Your task order (one unit = one commit)

| # | Task | First concrete action |
|---|------|----------------------|
| 1 | B1 (T-03, S) disposition vocabulary | Populate `RD.disposition_code` values in `src/16_domains_ops.nir` refdata corpus + rationale invariant in `00_core.nir` |
| 2 | B2 (T-14, S) kanban drag transitions | `crates/nirdosha-macros/src/kanban_board.rs`: accept `transitions:`, emit `data-allowed-to` + graying |
| 3 | B3 (T-02, S) forbidden=absent drop pass | `crates/nirdosha-macros/src/crud_screens.rs`: drop forbidden fields from rendered sets |
| 4 | B4 (T-10, S) role-ident canonicalization | Central mapping fn in `nirdosha-rt`; wire into `app_shell_from_toml!` + `crud_screens!` role grammar |
| 5 | B5 (T-12, S) predicate binding / I15 | `FilterExpr` pushdown into `crud_screens!` list handler |
| 6 | B6 (T-09, M) live streaming plane | Topic consumer behind `refresh_seconds` (poll→push, ≤5 Hz cap, poll fallback) |
| 7 | B7 (T-01, M) governed egress | `PG.governed_export` + O.* writer; route `crud_screens!` CSV export through it (streaming!) |
| 8 | B10 (T-04, L) approvals + `approval_inbox!` | New macro file + `RUNTIME.pending_approvals` wiring — the critical-path rock |
| 9 | B11 (T-06, L) workspace!/context | Promote `showcase_screens.rs` LayoutNode trees to declarative `workspace!` |
| 10 | C4 (M8 rules), C5 (simulation), C6 (sankey), C7 (reporting) | After B10 |

## File ownership

**Yours (edit freely):**
- `crates/nirdosha-macros/src/*` (kanban_board, crud_screens, wizard, new approval_inbox.rs, workspace.rs, …)
- `crates/nirdosha-rt/src/` runtime surfaces for the above
- `src/00_core.nir`, `src/40_scoring.nir`, `src/65_sar.nir`, `src/85_intervention.nir` policy additions your tickets name
- `tickets.md` — only to flip a ticket's status when you close it

**Shared with Agent B (protocol required):**
- `screens.toml` / `menus.toml` — you edit ONLY `ticket:` blockers of
  tickets you are closing (B1: 3.3/4.1/4.10/11.2; B10: the 12 T-04 screens;
  …). Never touch `dataset:` blockers — those are Agent B's.
  Before every register edit: `git pull --rebase`, make the edit, run
  `cargo test -p rtm`, commit immediately (R1: pins move in the same
  commit — B1/B10 change `tickets.md` blocked-screens sets AND
  `tests/verify_tickets.rs` counts).
- `examples/rtm/src/screens/mXX_*.nir` — coordinate; emit files only after
  the macro/engine exists.

**Never touch:** `src/bridge.nir` (Agent B's), `tests/verify_tickets_and_blockers.rs` dataset pins (Agent B's).

## Per-unit workflow

1. `git pull --rebase`
2. Do the task (macro/engine + policy + screen emission).
3. Update `tickets.md` (status/meaning/gates) + `tests/verify_tickets.rs`
   counts IN THE SAME CHANGE (R1).
4. `cargo test -p rtm` — all green.
5. Commit: `feat(rtm): <ticket> — <one-liner>`.
6. Update `screens-plan.md` checkbox + ledger in the same commit (R2).

## Hard invariants (repo law)

- R3: a screen flips stage only when archetype + real GuardedTable +
  guard_policy! all exist. Never fake a screen to move the count.
- R4: every new guard_policy! must be reachable from menus.toml's V5 check.
- B9's rule if you reach it via C5/C7: in-memory wizard FORBIDDEN for SAR.
- Timeout semantics resolve to DENY, never auto-release (11.4 stance).

## Handoffs you owe Agent B

- **B10 lands** → tell Agent B: nav.approvals is live; 5.10/9.5/15.5/18.7
  combo screens can emit; C8's retention/delegation singles unblock.
- **B7 lands** → tell Agent B: 7.5/12.10/15.5/18.9/20.3 export path exists.
- **B4 lands** → tell Agent B: `crud_screens!` can take real
  `requires role "PascalCase"` declarations; register role edits become safe.