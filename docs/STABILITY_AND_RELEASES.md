# Stability promise & release cadence

Closes the `docs/PUBLIC_ROADMAP.md` "In progress / next" item: *"A
compatibility/versioning policy before the next breaking language
change."* This is that policy. One page, on purpose.

## Where this project actually is today

Honestly, not the framing of a mature project pretending otherwise:
four tagged alpha releases (`v0.1.0-alpha.1` through `v0.1.0-alpha.4`),
the first three shipped the same day, the fourth eleven days later, no
fixed cadence between them. The interpreter was deleted this cycle.
Track B (full compilation) is still landing pieces. `sandbox` is
currently interpreter-only and therefore **not runnable in any form
today** (`docs/LANGUAGE.md` §1's own callout). This document is not a
claim that any of that has changed — it's a commitment about how
releases happen from here, and an honest checklist for what "v1.0"
actually has to mean before it's declared.

## The cadence, starting now

**A tagged release on the 1st of every month**, beginning
**2026-10-01**, regardless of how much or little landed that month — a
month with nothing release-worthy still gets a tag with an empty or
maintenance-only changelog, not a skip. The point is the rhythm being
unbroken and verifiable from the outside (`git tag`/`gh release list`),
not the size of any one release.

Every tagged release must, before the tag is pushed:
- Pass the full test suite in CI (no `--no-verify`, no skipped jobs).
- Ship a changelog entry naming what's new, what changed, and — this is
  the part easy to skip and the part that matters most for trust —
  **any behavior that changed silently in a way a user could observe**,
  even if it wasn't the intended point of the change. `docs/ROADMAP.md`
  already tracks `[DONE]`/`[PARTIAL]`/`[OPEN]`/`[NOT RUNNABLE]` per
  feature; a release changelog entry should read as a diff against that
  tracker, not a fresh marketing summary.
- Update `docs/PUBLIC_ROADMAP.md` if the release changes what's
  `[DONE]` vs `[PARTIAL]` vs `[OPEN]` for anything user-visible.

This is a promise about *process*, not about the language surface being
frozen yet — see below for what has to be true before that promise
extends to API stability.

## What "v1.0" will mean, and what has to be true first

`v1.0` is not declared by this document — it's **defined** by it, so
the bar is public before anyone hits it, not decided after the fact.
Nirdosha reaches `v1.0` when all of the following are true, checked
against `docs/PUBLIC_ROADMAP.md`'s own tags (the source of truth, not
this list — this list names the bar, that file reports the score):

1. **No `[NOT RUNNABLE]` items remain on the compiled path.** A feature
   that was real against the now-deleted interpreter but unreachable
   today either gets ported to the compiled path or is explicitly
   descoped from the v1.0 surface (removed from the language, not left
   in limbo).
2. **`sandbox` is either compiled or formally descoped from v1.0.**
   Today it's interpreter-only and therefore inert — that's a real gap
   in the "no GC, no data races, no deadlocks, no buffer overflow"
   promise's *reach*, not its *soundness*, but v1.0 can't ship with a
   language keyword that compiles to nothing.
3. **A deployment story exists for compiled `serve`** (containerization,
   secrets/JWKS handling) — currently `[OPEN]` in
   `docs/PUBLIC_ROADMAP.md`'s Track A.
4. **Real Windows verification**, not just a port that compiles —
   currently `[OPEN]`.
5. **This document's own cadence has held for at least 3 consecutive
   months** — a promise kept once isn't yet a promise trusted; a
   platform deciding whether to depend on this project needs to see the
   rhythm survive contact with a slow month, not just a good one.
6. **A public breaking-change policy is in force** (below), so `v1.0`
   isn't just a version number but a real change in what downstream
   users can assume won't move under them.

Until all six hold, every release stays in the `v0.1.0-alpha.N` line —
no `v0.x` (non-alpha) or `v1.0-rc` tag gets cut as a way to *feel*
closer to stable without actually clearing this list.

## Breaking-change policy (takes effect at v1.0, documented now)

- Pre-`v1.0` (now): breaking changes to the language surface, the CLI,
  or `nirdosha verify`'s JSON verdict schema can land in any monthly
  release, always called out explicitly in that release's changelog
  entry — never silently.
- Post-`v1.0`: a breaking change to anything covered by an RFC (see
  `rfcs/README.md`) requires a new RFC documenting the migration, and
  ships in the next **major** version only, never a patch or minor
  release.
- `nirdosha verify`'s verdict JSON is treated as an external contract
  from the moment any outside tool depends on it (a CI pipeline, an
  agent framework's repair loop) — additive fields are always safe;
  removing or renaming a field is a breaking change under the rule
  above, even before `v1.0`, because unlike the language surface, this
  is the one thing this project is explicitly asking outside tooling to
  build against today (`docs/LANGUAGE.md` §1, `docs/PUBLIC_ROADMAP.md`'s
  "Shipped" entry for `verify`).
  - **Breaking change, called out here per the rule above (2026-09):**
    the top-level `status` field (`"passed"`/`"failed"`) is renamed to
    `verdict`, and its value set widened to a genuine three-valued
    result: `"PROVED"`/`"DISPROVED"`/`"UNKNOWN"`. `ContractsResult`
    gained the same `verdict` field alongside its existing `status`
    (which now means only "did this stage run," never "did it find a
    problem" — see `crates/compiler/src/main.rs`'s `ContractsResult`
    doc comment). The exit code widened to match: `0`/`1`/`2`, not
    `0`/`1` — a caller that only checked `$?  == 0` for "safe to
    proceed" now needs to check for `0` specifically, since `2`
    (`UNKNOWN`) is a new, real outcome that used to be silently folded
    into a passing `0`. Any outside tool built against the two-valued
    schema needs this update; nothing before this note ever shipped a
    tagged release depending on the old shape.
