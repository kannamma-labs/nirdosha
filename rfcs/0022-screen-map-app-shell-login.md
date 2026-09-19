# RFC 0022 — Screen-only graph, app shell, and login archetypes

## Status

**Pending implementation.** This RFC proposes additions to `nirdosha-hi`'s graph view and the `nirdosha_rt` v2 UI macro catalog. It does not change `.nir` language semantics; the compiler tree is unaffected.

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

### 2. Screen-only graph view

Add a header toggle **"Code graph" ↔ "Screen map"** in `hi_graph.html`.

- **Code graph** is today's 3D view of all `CodeUnit` nodes and tracked edges.
- **Screen map** filters the same renderer to show only `screen`-kind nodes.
- Edges are derived from:
  - `action "<label>" -> <fn>` inside a `screen <Struct> { ... }` block when the backing `fn` can be matched to another mounted screen (heuristic, labeled as inferred).
  - A synthetic `App` node with edges to every mounted screen (from macro `path:` attributes).
- Nodes are colored by `screen_type`; a legend is shown.

A new backend endpoint `/api/screens` returns `{ screens: [...], navigations: [...] }` so the screen map does not need to re-derive the view from raw `/api/nodes`+`/api/edges`.

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

- `crates/nirdosha-macros/src/lib.rs` — export `login`/`app_shell`
- `crates/nirdosha-macros/src/login.rs` — new
- `crates/nirdosha-macros/src/app_shell.rs` — new
- `crates/nirdosha-rt/src/lib.rs` — re-export `login`, `app_shell`
- `crates/nirdosha-rt/src/web.rs` — small helpers if needed
- `crates/nirdosha-hi/src/hi_graph.rs` — screen type classification
- `crates/nirdosha-hi/src/hi_api.rs` — `/api/screens`
- `crates/nirdosha-hi/src/hi_graph.html` — toggle + screen map rendering
- `crates/nirdosha-hi/src/mcp_tools.rs` — `get_ui_conventions`
- `crates/nirdosha-hi/src/hi_llm.rs` — prompt updates
