# `.nir` top-level forms vs. dialect macro coverage

Issue #70's audit: every field of `.nir`'s `Program` (`crates/compiler/src/ast.rs`,
`struct Program`) is a top-level declaration kind. `fns`/`structs`/`enums`/`imports`
already have exact plain-Rust equivalents (`fn`, `struct`, `enum`, `use`) — nothing
to substitute. The rest are `.nir`-only declarative forms with no bare-Rust syntax,
and are the actual subject of this table: does an `nirdosha_rt`/`nirdosha_macros`
item-position macro cover the same ground, with the same field/clause set, per
`docs/nirdosha-rt-dialect.md`'s "no bespoke parser, ever" substitution rule?

| `.nir` form | AST shape | Dialect macro | Status |
|---|---|---|---|
| `screen <Struct> { .. }` | `ScreenDecl` (`ast.rs:1676`) — layered UI hints on an inferred screen | `crud_screens!`, `settings_screen!`, `kanban_board!`, `dashboard!`, `wizard!`, `communication_feed!` | **Covered by archetype, not by a raw form.** Every shape `.nir`'s naming-convention UI inference + `screen{}` hints can express, the six archetypes already cover in practice; a *generic* `screen! { .. }` mirroring `ScreenDecl` field-for-field doesn't exist and isn't needed — the archetypes are the intended substitute (see `docs/nirdosha-rt-dialect.md`'s own framing: "whatever fits inside existing Rust syntax" was the thing to avoid, and each archetype is a full grammar for its own shape, not a cut-down one). |
| `dashboard { .. }` | `DashboardDecl` (`ast.rs:1681`) | `dashboard!` (`nirdosha-macros/src/dashboard.rs`) | **Covered.** Widgets, mount fn, JSON+HTML routes — direct correspondence. |
| `workflow Name { .. }` | `WorkflowDecl` (`ast.rs:1699`), lowered by `workflow_lower.rs` into ordinary `fn`s/`enum`s | `wizard!` (commit `9a78fa5`) + `workflow!` (issue #73) | **Covered.** `wizard!` remains its own archetype (a multi-step form with server-side per-run state); `workflow!` (`nirdosha-macros/src/workflow.rs`) is the general raw state/event/transition grammar `WorkflowDecl` needs, generating a state enum and a pure `advance_<name>` transition function, plus an optional `against = "spec.json"` clause running the same conformance check `workflow_conformance.rs` does (issue #73) via `nirdosha-contract-core::workflow_spec`. Deliberately no storage/HTTP routes (unlike `wizard!`) — the shape is the point, not a second UI archetype; `on_entry`/`on_exit` are plain labels compared by count only, matching `workflow_conformance.rs`'s own documented scope limit. |
| `landing { .. }` | `LandingDecl` (`ast.rs:2014`) — first-match-wins `role(..) -> target` / `default -> target` rules picking a post-login redirect | **none** | **Was fully uncovered — closed by this change** (`nirdosha-macros/src/landing.rs`, `landing!`). Scoped to role conditions only: `.nir`'s `LandingCondition::Requirement` also accepts `Claim(String, String)` (`ast.rs:1270`), but the dialect's own `Auth`/role runtime (`nirdosha-rt/src/role.rs`) has no claim concept at all yet (only `Auth::has_role`) — claim-based landing rules are blocked on that missing primitive, not on this macro, and are out of scope here. |
| `serve { expose fn_a, fn_b, .. }` | `ServeConfigDecl` (`ast.rs:2019`) — an explicit, deny-by-default exposure allow-list, independent of `.nir`'s automatic screen/dashboard-bound RPC exposure | **none** | **Fully uncovered — deliberately out of scope for this change.** The dialect's closest existing thing is each archetype macro registering its own routes on `nirdosha_rt::web::Router` directly when invoked (`web.rs`) — an *implicit* "only macro-registered routes exist," never an explicit, centralized allow-list a hand-written fn can opt into the way `.nir`'s `expose` lets any fn (archetype or not) become a route. This needs its own design pass against `Router`'s existing registration model (how does `expose` interact with a route an archetype macro *also* registers for the same fn? what happens under `--deep` if an exposed fn has no `requires(role/claim)` gate at all, mirroring `.nir`'s own deny-by-default posture?) — bundling it into "one more small archetype" would produce exactly the cut-down, doesn't-fit-the-domain grammar this issue's own problem statement warns against. |
| `workspace Name { .. }` | `WorkspaceDecl` (`ast.rs:2130`) — composite multi-panel screens referencing other screens | **none** | **Fully uncovered — deliberately out of scope for this change.** Depends on `screens` existing as a coherent, name-addressable set to reference from panels; since the dialect doesn't have a raw `screen!` form (see row 1 above — the archetypes stand in for it, each with its own generated route/name, not a shared "screen registry" `workspace!` could look up into), a faithful `workspace!` needs that referencing story worked out first, not just a parser for its own syntax. |
| `validate <fn> { pre: .. post: .. }` | `ValidateDecl` (`ast.rs:1710`) — a Hoare contract layered on an existing fn, checked statically where provable and at runtime otherwise | `requires(expr)`/`ensures(expr)` (issue #68) | **Covered, under different syntax.** Not a form-for-form port — the dialect expresses the same "precondition/postcondition on a fn" concept as `#[contract(requires(..), ensures(..))]` clauses on the fn itself rather than a separate `validate` block naming it by string, and checks it via a real Z3 MIR pass (issue #67's VC IR) rather than `.nir`'s own interpreter-level runtime backstop. No capability gap; already shipped. |

## What this change adds

`nirdosha_rt::landing! { .. }` (`crates/nirdosha-macros/src/landing.rs`) — see its
own module doc for the grammar and generated code shape, and
`crates/nirdosha-rt/tests/landing.rs` for the same validation `.nir`'s own
`typeck::check_landing` enforces (exactly one `default`, required; `default` must
be last) reproduced as real macro-expansion-time `compile_error!`s.

## Remaining gaps, in priority order

1. **`serve { expose .. }`** — the actual access-control-relevant gap: an explicit,
   centralized exposure allow-list independent of which archetype (if any) a fn
   belongs to. Needs a design pass against `web::Router`'s existing per-archetype
   route registration, not just a parser.
2. ~~A general `workflow!`~~ — closed by issue #73.
3. **`workspace!`** — blocked on a name-addressable screen registry existing first.
4. **Claim-based landing rules** — blocked on the dialect's `Auth`/role runtime
   gaining a claims concept at all (today: role-only).
