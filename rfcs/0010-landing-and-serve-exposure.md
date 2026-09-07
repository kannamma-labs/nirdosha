# RFC 0010: Per-role/claim `landing` screens, and the compiled-`serve` route-exposure model

> **Status.** Built and verified on `track-b/db-codegen`, not a proposal
> — same "Status box updated in place, keep the original design capture
> below as written" convention `rfcs/0008`/`rfcs/0009` already established
> for this repo.
>
> - **`landing { role(...)/claim(...)/default -> <Screen> }`**: real
>   grammar (`Tok::Landing`, one new reserved keyword; `role`/`claim`/
>   `default` are plain idents matched by string inside the block, same
>   contextual treatment `tile`/`chart`/`visual` already get inside
>   `dashboard { ... }`), real `typeck::check_landing` (every target
>   resolves to a real `screen`, exactly one `default`, `default` must
>   be last, no unreachable duplicate rule), and a real client-side
>   redirect: `ui_gen.rs` emits the rule table as `window`-scope
>   `const LANDING = [...]`, and the generated bundle's `postLoginRedirect`
>   (every sign-in path, including the OIDC `/auth/callback` redirect)
>   consults it before falling back to its pre-existing "dashboard, else
>   first readable screen" behavior. `crates/compiler/tests/landing_dsl.rs`
>   (11 tests) covers the typeck rules.
> - **`serve { expose fn_a, fn_b, ... }` + the exposure model**: real
>   grammar (`Tok::Serve`, one new reserved keyword), and a real,
>   unconditional `typeck::check_serve_exposure` — deny-by-default
>   (`create_`/`update_`/`delete_`-prefixed functions in the exposure
>   set need `requires(...)`, `requires(public)` included) is a hard
>   `TypeError`, enforced today even though nothing yet dispatches these
>   routes over real HTTP (compiled `serve`, ROADMAP B8, is a later
>   phase — see Compatibility below for why this is deliberately not
>   deferred). The confidentiality axis (an exposed read with no
>   `requires(...)` at all) is a separate, non-fatal
>   `exposed_public_read_warnings`, one summarized warning per program.
>   `crates/compiler/tests/serve_exposure.rs` (13 tests).
> - **Narrower than originally sketched below, disclosed, not silent**:
>   the *implicit* half of the exposure set (`implicitly_exposed_fn_names`)
>   only covers `screen { list/create/update/delete }` and
>   `dashboard { tile/chart }` bindings — not `visual`s, not `workspace`
>   panels/actions, not `screen` `action`s, and not Row 12's own
>   *automatic*, no-`screen`-block-at-all naming-convention inference.
>   Anything not covered needs an explicit `serve { expose ... }` entry
>   to be reachable at all once compiled `serve` exists. Widening this
>   set is real, separate follow-up work — a strictly *additive* change
>   (more things become implicitly exposed) rather than one that could
>   ever silently narrow what's already reachable.
> - **`nirdosha build --serve`'s own printed exposure-table audit dump**
>   (this document's original design) is **not** built yet — it's tied
>   to the `--serve` flag itself, which doesn't exist until compiled
>   `serve` (ROADMAP B8) lands. The typeck rules above don't need it to
>   be real today; the printed table is real, separate follow-up work
>   for that phase.

## Motivation

Two gaps, found while designing the "Nirdosha Ops Console" demo app
(multi-user login, each role landing on its own screen, backed by real
compiled `db`/`http`/`identity`/`transact`):

1. **There is no concept anywhere of a per-role default landing page.**
   The now-deleted interpreter's `serve.rs` served one identical SPA
   bundle to every user regardless of role — "which screen do I land on
   after signing in" would have had to be hand-rolled client JS logic,
   with no static checking that every role/claim actually has somewhere
   to go.
2. **Nothing in the language decides which functions become HTTP
   routes once a program is served.** "Every `fn` the dispatch table
   walks" would silently turn every internal helper into a remotely
   callable endpoint; "every naming-convention-matched function" is
   scarcely better, since `nirdosha gen-crud` generates exactly the
   `create_`/`update_`/`delete_` convention functions this would auto-
   expose, with no human ever deciding they should be reachable. A
   compile-time exposure model — decided once, checkable statically,
   the same way `requires(...)` already decides who may call a gated
   function — closes both the "what's exposed" question and the "is a
   mutating exposed route gated" question before compiled `serve` (a
   later phase) can ever ship with either open.

`GOVERNANCE.md` requires an RFC for any grammar/public-interface change
— both items here add real, reserved top-level keywords (`landing`,
`serve`), so both land as one RFC rather than two smaller ones, since
they're designed together (a `landing` target must itself be a `screen`,
same resolution machinery `check_serve_exposure`'s implicit half reuses).

## Design

### `landing { ... }`

```
landing {
    role("admin") -> AdminDashboard
    role("clinician") -> ClinicianQueue
    claim("department", "cardiology") -> CardiologyQueue
    default -> HomeScreen
}
```

- At most one `landing` block per program (`ast::LandingDecl`, parsed by
  `parser::parse_landing_decl`, `Program.landing: Option<LandingDecl>`).
- Each rule (`ast::LandingRule`) is a `LandingCondition` (`Requirement`
  reused verbatim — no new proof/credential concept — plus `Default`,
  the required fallback) and a `target: String` naming a `screen`'s own
  `struct_name`.
- `typeck::check_landing` (`crates/compiler/src/typeck.rs`): every
  `target` must resolve to a real declared `screen`; exactly one
  `Default` rule is required (an authenticated identity matching
  nothing and having nowhere to land is a real, statically-catchable
  UX bug, not a runtime surprise); `Default` must be the last rule
  (anything after it is unreachable, since `Default` always matches);
  a `role`/`claim` rule identical to an earlier one in the same block
  is rejected as unreachable too (first-match-wins makes it dead code,
  almost always a copy-paste mistake) — reported as a distinct
  `TypeErrorKind` from the "after `Default`" case, since the fix
  differs (move the rule vs. delete the duplicate).
- Produces no LLVM IR — `ui_gen.rs::landing_json` emits the rule table
  (targets converted to the same snake-case route the screen manifest's
  own `.snake` field already uses) as `const LANDING = [...]` in the
  generated client bundle. The bundle's `postLoginRedirect` (called from
  every sign-in path: the mock-IdP form, the pure client-side stub, the
  demo-avatar picker, and the real OIDC `/auth/callback` redirect) tries
  `landingTargetFor(identity)` first, falling back to its pre-existing
  "dashboard, else first readable screen" behavior only when `LANDING`
  is `null` (no `landing` block) or no rule matches — impossible once a
  `Default` exists, but the fallback stays as defense in depth, not
  something a well-formed program can actually reach.
- **Disclosed, low-severity leak**: the bundle is identical for every
  visitor, so `LANDING`'s full role-to-screen mapping is readable by
  anyone before login. Targets are compile-time screen names, not URLs
  (no open-redirect risk) — an unauthenticated visitor can enumerate
  internal screen names this way, accepted for this cut rather than
  building a per-session bundle to hide it.
- **Redirect only fires at the moment identity is (re-)established**,
  never on an ordinary page load/reload with an existing hash — a
  bookmarked or deep-linked URL is never fought with on refresh
  (`validateStoredIdentityThenStart`, the reload-time bootstrap, never
  calls `postLoginRedirect`).

### `serve { expose ... }` and the exposure model

```
serve {
    expose utility_report, admin_reindex
}
```

- At most one `serve` block per program (`ast::ServeConfigDecl`,
  `Program.serve_config: Option<ServeConfigDecl>`). Deliberately a
  **general per-program serve-config section**, not a single-purpose
  `expose { ... }` block — `expose` is its first entry, not its only
  reason to exist, so a later addition (port, TLS, CORS-origin config,
  all real needs of the compiled-`serve` phase this sets up) extends
  this same struct instead of forcing another grammar change.
- The exposure set (`typeck::exposed_fn_names`) is the union of two
  halves, both real today:
  - **Implicit** (`implicitly_exposed_fn_names`): every function bound
    as a `screen`'s `list`/`create`/`update`/`delete`, or a
    `dashboard`'s `tile`/`chart` target. These are already meant to be
    UI-callable (`ui_gen.rs`'s own convention), so exposing them over
    HTTP needs no new annotation.
  - **Explicit**: every `serve { expose ... }` entry, for anything
    outside that convention.
  - There is no third, implicit "everything else" case — a `fn` neither
    screen/dashboard-bound nor named in `expose` is never reachable
    over HTTP, full stop.
- **Deny-by-default** (`TypeErrorKind::ExposedMutatingFnMissingRequires`,
  a hard error): any exposed `create_`/`update_`/`delete_`-prefixed
  function with no `requires(...)` at all is rejected — `requires(public)`
  satisfies the rule (an *explicit* "yes, public" is a real decision;
  a plain absence is not). `list_`/`stat_`/`chart_` (read-only) may
  stay ungated. This one rule directly bounds `gen-crud`'s own
  auto-exposure risk: a table's generated `create_`/`update_`/`delete_`
  functions, wired into a generated `screen`, are exactly what this
  targets.
- **Confidentiality is a separate axis from mutation, and the hard rule
  above only covers the second one.** An exposed `list_`/`stat_`/
  `chart_` function left ungated (allowed by the rule above) returns
  its full result set to any unauthenticated visitor once served —
  `typeck::exposed_public_read_warnings` surfaces this as a non-fatal
  `TypeWarning`, **one summarized warning per program**, not one per
  function: `gen-crud` generates exactly the convention names this
  targets, so a naive one-per-function warning would drown every
  generated screen the moment it compiles, training people to ignore
  warnings — the standard failure mode of this kind of guard.
  `requires(public)` on a read silences it, same as it satisfies the
  hard rule for a write.
- `serve { expose <name> }` where `<name>` doesn't resolve to a real
  `fn` is `TypeErrorKind::UnknownExposedFn`.
- **`check_serve_exposure` runs unconditionally**, even for a program
  with no `serve` block at all — the risk it closes (a `gen-crud`-
  generated mutating route with no human ever deciding it should exist)
  comes entirely from `screen`'s own `create`/`update`/`delete`
  bindings, which need no `serve` block to exist. See Compatibility
  below for why enforcing this now, ahead of compiled `serve` itself
  existing, is deliberate.

## Effect on the permission model

`landing` adds no new proof/credential concept — its `role(...)`/
`claim(...)` conditions are `Requirement::Role`/`Requirement::Claim`
verbatim, the same type `requires(...)`/`acquire` already use, and
`check_landing` never grants or denies access to anything: it only
decides which screen a request for `/#/` effectively redirects to,
client-side, after the server has already authenticated the request
through its own unrelated path. A `landing` rule pointing at a screen a
role can't actually read is a real UX bug (redirect to a screen that
then 403s) but not a security hole — `screen`/`requires`'s own gates are
what actually enforce access, unchanged by this RFC. A future
`check_landing` enhancement could cross-check a `landing` target's
screen against its own `view`/`edit`/`requires` gates for the same
role/claim — not built here, named as a real, separate follow-up.

`serve`'s exposure model **is** a real permission-model addition: it's
the boundary that decides which functions are reachable over HTTP at
all, before `requires(...)`'s own per-call check ever runs. Getting it
wrong (over-exposing) is the actual security risk this RFC exists to
close; getting it right narrows what a compiled `serve` process can
ever be asked to do, strictly on top of (never instead of) every
function's own `requires`/`acquire` gate.

## Compatibility

Both `landing` and `Tok::Serve` are new reserved keywords occupying a
spot no legal program could use before (confirmed: neither `landing`
nor `serve` appears as an identifier anywhere in `examples/`) —
backward-compatible by construction for parsing, the same reasoning
`docs/LANGUAGE.md` §17 already uses for `Dashboard`/`Screen`.

**`check_serve_exposure`'s deny-by-default rule is a real, deliberate
behavior change for an existing program, not purely additive** — a
program with a `screen { create: some_fn }` binding where `some_fn` has
no `requires(...)` compiled cleanly before this RFC and does not after.
Found immediately by this session's own test suite:
`crates/compiler/tests/screen_dsl.rs`'s well-formed fixture needed
`requires(public)` added to its `create_product`/`update_product`/
`delete_product` functions to keep typechecking. **This is intentional,
not an oversight to soften**: the whole point of deny-by-default is
that an existing program with this shape was already carrying an
unexamined risk (silent full exposure once served), and the RFC's job
is to surface it at the moment it can still be *fixed* — a one-line
`requires(public)`/`requires(role: ...)` addition — rather than leave it
latent until a real, working compiled `serve` binary ships it live.
Enforcing this now, before compiled `serve` (ROADMAP B8) exists, rather
than waiting for that phase, means every `screen { create/update/delete
}` binding written between now and then already carries a real
`requires(...)` decision by construction, instead of a wave of
newly-surfaced errors landing all at once when `serve` finally ships.

## Rejected alternatives

**Landing targets naming a `dashboard`/`workspace`, not just a
`screen`.** `DashboardDecl` has no name field at all — there's at most
one, anonymous, per program — so "target the dashboard" would need a
special sentinel (a literal `dashboard` target, say) rather than an
ordinary identifier, and `workspace`s are parameterized per-subject-
instance in a way that doesn't obviously resolve to "the page an
identity lands on" without picking a specific instance. Scoped out for
this cut: `landing` targets a `screen` only, disclosed as a real,
narrower-than-first-sketched scope rather than solved awkwardly.

**Deferring `check_serve_exposure` until compiled `serve` (B8) lands.**
Considered, rejected: the deny-by-default risk is entirely about
`screen`'s own `create`/`update`/`delete` bindings, which already exist
and already compile today — deferring the check would let the same
`gen-crud`-shaped risk this RFC exists to close keep accumulating in
every program written in the meantime, only to surface as a wave of
errors the moment B8 ships. Enforcing it now costs nothing (compiled
`serve` doesn't need to exist for the rule to be checkable) and pays
down the risk continuously instead of in one disruptive batch later.

**One `TypeErrorKind`/message for both "rule after `default`" and
"duplicate `role`/`claim` rule."** Both are "unreachable rule" in a
sense, but the fix differs (move the rule earlier vs. delete the
duplicate) — collapsing them would leave the fix to guesswork. Kept as
two distinct `TypeErrorKind` variants instead.

## Open questions

- Whether `visual`s, `workspace` panels/actions, and `screen` `action`s
  should join the *implicit* exposure set, and on what schedule — real,
  disclosed follow-up work, not blocking this RFC (anything not yet
  covered is still reachable via an explicit `serve { expose ... }`
  entry, so nothing is permanently unreachable, only not-yet-implicit).
- Whether `check_landing` should eventually cross-check a landing
  target's own `view`/`requires` gates against the same role/claim the
  `landing` rule fires for, to catch a "redirects to a screen this role
  can't actually read" mistake statically rather than leaving it a
  runtime 403. Named in "Effect on the permission model" above, not
  decided here.
- The exact shape of `nirdosha build --serve`'s printed exposure-table
  audit dump (columns, ordering, whether it's plain text or JSON) is
  real, separate follow-up work for the compiled-`serve` phase (ROADMAP
  B8) — not decided by this RFC, which only establishes what the table
  *contains* (the exposure set `exposed_fn_names` already computes).
