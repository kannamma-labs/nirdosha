# RFC 0022 — Screen-only graph, app shell, and login archetypes

## Status

**Partially implemented.** §3 (`login!`), §4 (`app_shell!`), §5 (prompt
taxonomy), and §2's screen-map view shipped — §2 in its 2026-09-22
amendment form below (the toggle is real, and the navigation edges are
reverse-engineered from each app's own code rather than guessed from
action labels). What remains open from this RFC is §2's
`screen { action ... }`-block edge heuristic (dead in v2 — that syntax
never existed here; superseded by the code-derived edges below) and the
v2 exposure-recognition gap noted in `hi_graph.rs::sync_file` (a
`#[nirdosha_rt::contract(...)]`-gated fn is not yet marked `exposed` in
the graph, so `[API-exposed]` renders only when something else stamps
`nodes.status`).

## Problem

`nirdosha hi`'s live graph renders every `CodeUnit` (`fn`, `struct`, `enum`, `screen`) and every tracked relationship. For a Rust/systems reader that is useful; for a product owner, QA engineer, or LLM agent reasoning about user journeys it is noise. Two needs keep coming up:

1. A **screen-only view** that shows only the surfaces a user sees and the actions that move them between those surfaces.
2. Every generated app should have a **login screen** and a **top-level app shell/layout** by default, with the login behaving differently under demo and production deployment.

## Proposal

### 1. Screen taxonomy in the knowledge graph

Extend `hi_graph.rs` so every `screen`-kind `CodeUnit` carries a `screen_type` attribute derived from the macro name that produced it:

| macro | `screen_type` |
|---|---|
| `crud_screens!` | `crud` |
| `dashboard!` | `dashboard` |
| `kanban_board!` | `kanban` |
| `wizard!` | `wizard` |
| `settings_screen!` | `settings` |
| `communication_feed!` | `feed` |
| `landing!` | `landing` |
| `login!` (new) | `login` |
| `app_shell!` (new) | `shell` |

`screen_name_from_macro` currently accepts only six macro names; it will be extended to include `landing!`, `login!`, and `app_shell!`.

### 2. Screen-only graph view — as shipped (2026-09-22 amendment)

Add a header toggle **"Screens" ↔ "Data"** in `hi_graph.html` — one
button, active-filled amber while the screen map is up, reading `Data`
as the way back.

- **Data** is today's 3D view of all `CodeUnit` nodes and tracked edges
  (the code datastructure).
- **Screen map** swaps the same renderer (`run3D`/`run2D`, WebGL with a
  2D canvas fallback) over to `/api/screens`' payload: one node per
  screen, colored by `screen_type` (a dedicated legend replaces the
  code-kind legend), one arrow per navigation edge, labeled. Clicking a
  screen opens a panel showing its type, route path, what it links to,
  and what reaches it.
- A screen node now exists for every **hand-registered `router.get*`
  route too** (not only macro screens): `hi sync` reverse-engineers each
  fn body's `get`/`get_gated`/`get_gated_claim`/`get_with_auth`
  registrations into `screen`-kind nodes of type `page`, identified by
  the route path itself (unique per `menus.toml`'s V6), titled by the
  registration's title literal. POST/PUT/DELETE are actions, not
  surfaces, and are deliberately excluded.
- **Edges are derived from real code, not heuristics over prose:**
  - the nav register `app_shell_from_toml!("menus.toml", ...)` names:
    one `shell → route` edge per `[[menu]]` (labeled `label_key`) and
    one `login → route` edge per `[landing]` role (labeled
    `landing: <Role>`);
  - every static string-literal `Response::redirect(...)` inside a GET
    route's handler (`redirect` edges; `format!`-built dynamic targets
    are skipped — guessing them would be a lie);
  - every `approval_inbox!` `detail_path:` (labeled `detail`), resolved
    exact → same-shape template → crud-subtree base → loose template,
    so `/cases/{id}` lands on the crud macro screen that owns `/cases`
    rather than whatever 3-segment path a loose match finds first.
  These land in the graph as labeled `NAVIGATES_TO` edges with
  `code-sync` provenance and are regenerated wholesale each sync —
  they are pure derived data; nothing human- or model-authored writes
  that kind (`hi link` writes only the `IMPLEMENTS`/`IMPLEMENTED_BY`
  pair).
- **Fallback connectivity, narrowed:** the synthetic `App`→every-screen
  fan-out now applies only to screens no derived edge reaches, so a
  screen reached by a real menu entry no longer also carries a fake
  "app shell" edge.

`/api/screens` returns `{ screens: [...], navigations: [...] }` — the
derived edges (`inferred: false`, real labels) plus the narrowed
fallback layer (`inferred: true`) — so the screen map does not need to
re-derive the view from raw `/api/nodes`+`/api/edges`.

Prompt-mode gate note (same amendment): the prompt screen of RFC 0014
asks "what do you want to build?" only while the graph is genuinely
empty — a graph populated by code sync (units carrying synced
`content_hash`es, e.g. a reverse-engineered app) opens straight into
build mode.

### 3. New `nirdosha_rt::login!` macro

```ignore
nirdosha_rt::login! {
    mount: mount_login,
    path: "/login",
    mode: demo,
    demo_users: [
        { username: "admin", password: "admin", roles: ["admin"] },
        { username: "user",  password: "user",  roles: ["user"] },
    ],
}
```

- `mode: demo` — generated `verify` closure matches the listed users and returns their roles.
- `mode: production` — generated `verify` closure reads from `NIRDOSHA_LOGIN_USERS` (a JSON file path) at runtime. If the env var is unset the login rejects every attempt and logs a configuration warning; the app still compiles and the operator has a single documented hook.

In both modes the macro generates:
- `fn mount_login(router: ::nirdosha_rt::Router) -> ::nirdosha_rt::Router`
- a `LoginConfig`-style `verify` closure wired to `Router::with_login`

### 4. New `nirdosha_rt::app_shell!` macro

```ignore
nirdosha_rt::app_shell! {
    title: "My App",
    nav: [
        { label: "Dashboard", href: "/dashboard" },
        { label: "Products",  href: "/products",  role: "admin" },
    ],
    landing: mount_landing,
}
```

It generates:
- `fn app_shell_title() -> &'static str`
- `fn app_shell_nav() -> Vec<NavLink>`
- `fn app_shell_landing(auth: &Auth) -> &'static str` (delegates to `landing_path` if `landing:` is given)

`main()` then wires these into `Router::new(...).with_info(app_shell_title(), "0.1.0").with_nav(app_shell_nav()).with_login("/login", app_shell_login_verify())`.

This is intentionally not a wrapper around every other archetype macro; it is a **declarative shell** that `main()` applies once to the router, matching the existing `with_nav`/`with_login` API.

### 5. Prompt changes

`POPULATE_SYSTEM_PROMPT` and `graph_task_message` will be updated to:
- Explicitly ask for a `Login` screen and an `AppShell` screen in every app that has any UI at all.
- Ask for the right macro for each screen based on its driving text (`dashboard`, `crud`, `wizard`, `kanban`, `settings`, `feed`, `landing`, `login`, `shell`).
- Stop asking the model to guess whether a UI is needed; if the request is not a pure library, it needs Login + AppShell + the data screens.

The `+ Screen` rail in `hi_graph.html` already accepts only plain-language description; the prompt text will be tightened so the model infers the archetype from the description rather than offering the user a type selector.

### 6. Scope honesty

- Navigation edges in the screen map are labeled "inferred" when derived from static mount points and action descriptions; they are not proof of runtime routing.
- `workspace!` remains out of scope (per `docs/DIALECT_FORM_COVERAGE.md`); `app_shell!` is a global nav/title shell, not a multi-panel workspace.
- Production login still relies on an external credential source; the macro only supplies the wiring and a documented env-var hook.

## Acceptance criteria

1. `cargo test -p nirdosha-hi` passes.
2. `cargo test -p nirdosha-macros` passes.
3. `cargo test -p nirdosha-rt` passes.
4. A freshly generated app from `nirdosha hi` contains `login!` and `app_shell!` macros when it has any other screen.
5. The `nirdosha hi` 3D graph has a toggle that switches between the full code graph and a screen-only map with typed nodes.

## Files changed

- `crates/nirdosha-macros/src/lib.rs` — export `login`/`app_shell` (§3/§4, shipped earlier)
- `crates/nirdosha-macros/src/login.rs` — new (shipped earlier)
- `crates/nirdosha-macros/src/app_shell.rs` — new (shipped earlier)
- `crates/nirdosha-rt/src/lib.rs` — re-export `login`, `app_shell` (shipped earlier)
- `crates/nirdosha-rt/src/web.rs` — small helpers if needed (shipped earlier)
- `crates/nirdosha-hi/src/hi_graph.rs` — screen type classification (shipped earlier); 2026-09-22 amendment: route-page reverse-engineering, `ScreenFacts`, `derive_screen_navigation` (`NAVIGATES_TO` from menus.toml/redirects/detail paths), `app_shell_from_toml!`/`approval_inbox!`/`workspace!` recognition, flattened token scanning, `edges.label` migration
- `crates/nirdosha-hi/src/hi_api.rs` — `/api/screens` (2026-09-22 amendment: serves derived nav edges + narrowed orphan-only fallback); same-day follow-up: `GET /api/screen-register` (the register's own module/archetype/stage/role/file vocabularies, existing ids, next-id suggestions) and `POST /api/screen-register/add` (validates a structured entry against the register, appends the `[[screen]]` block, bumps `total_screens`, records a reviewable `screen` candidate; refuses name collisions with synced units; per-field 400s, never a partial write). Routes live in menus.toml's V6 domain — the form deliberately does not invent a route key in the register; the route reaches the graph through the candidate's driving text
- `crates/nirdosha-hi/src/hi_graph.html` — toggle + screen map rendering, screens legend, screen click panel, prompt-mode gate honoring code-populated graphs; same-day follow-up: `+ Screen` rail gains a Register/Describe tab pair (the register tab is default when the project has a screens.toml and is driven entirely by that register's vocabularies — role chips, next-id prefill, inline per-field errors, appended-TOML result), and the legend gets its own hide/show toggle (header row, collapsed state persists via localStorage like the UI-zoom key)
- `crates/nirdosha-hi/src/main.rs` — sync report line carries the derived nav-edge count
- `crates/nirdosha-hi/src/hi_revision.rs` — commit no-op when `.nir/` doesn't exist yet (git-add pathspec fix, found en route)
- `crates/nirdosha-hi/src/mcp_tools.rs` — `get_ui_conventions` (shipped earlier)
- `crates/nirdosha-hi/src/hi_llm.rs` — prompt updates (shipped earlier)
- `agent-skills/nirdosha/hi_prompt.md` — trimmed back under its own size cap (test had drifted red)
