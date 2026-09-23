# menus.toml — schema and register-pair invariants

`menus.toml` is one half of a **register pair**: `screens.toml` declares what
the generator builds, `menus.toml` declares what the generated app promises
in its nav. JSON schema: [`MENUS_SCHEMA.json`](MENUS_SCHEMA.json). The
screens side: [`SCREEN_REGISTER_SCHEMA.md`](SCREEN_REGISTER_SCHEMA.md).

The two files are validated **against each other** by
`cargo nirdosha generate-screens` before any code is emitted. A violation is
a build-time error, not a dead link or a runtime deny — a menu entry is a
promise the generated app can keep, or the generator refuses to ship it.

## Invariants (checked per `cargo nirdosha generate-screens` run)

| # | Invariant | Error behavior |
|---|-----------|----------------|
| V1 | `menu.screen_id` references a declared `[[screen]]` id | generation error |
| V2 | `menu.roles` ⊆ target screen's declared role names (the role part of `Role:R/W`); screens declaring `AllRoles`/`AllHuman` satisfy any role set | generation error |
| V3 | target screen `stage == "built"` — a generated app cannot promise a route for a screen the register does not build | generation error |
| — | **route agreement**: `menu.route == target.codegen.route_path` — the nav link points at the route the generator actually emits, no hand-written spelling | generation error |
| — | **route uniqueness**: two menu entries must not promise the same route (V6-lite) | generation error |
| V5-lite | `menu.guard { action, resource }` resolves to a **synthesized policy**: `resource` (lowercased) is an entity of the register, and `action` appears in some built screen's `policy.allowed_actions` for that entity; guard references on presentational screens (`static_embed`, `login`, `app_shell_from_toml`) are always refused | generation error |
| V8-lite | every `[landing]` key is a role some built screen declares; every value is a served route that the landing role may reach (target screen declares the role, `AllRoles`, or `AllHuman`) | generation error |

Checked invariants are counted in the generator report
(`invariants_checked`), so a green run states *how much* it proved, not
just that it passed.

## What the generator does NOT check

- **Group ordering** (`[[group]]` blocks) — chrome; the app-shell macro owns
  ordering at compile time.
- **`label_key` spelling** — display strings, not behavior.
- Anything the shell macro re-reads at compile time beyond entries
  (`chrome`, `shell_variant`, ...) — unknown keys are passed through.

## Guard as single authority, restated for menus

A `menu.guard` reference is a *claim about the data plane*, not a second
enforcement point. The guard runtime decides per-subject whether a request
matches a synthesized policy record; the menu's guard reference must merely
resolve to a policy that exists (V5-lite). Masking, caps, and audit follow
the subject and the policy — never the route or the nav entry.