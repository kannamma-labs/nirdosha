# Response — external red-team submission, 2026-09-11

`FULL_REDTEAM_REPORT.md` and `STARTER_KIT.md` in this directory are the
submission verbatim, unedited, per `SECURITY.md`'s own "the reward on
offer is a real fix ... and, for a marquee-severity find, a public
disclosure write-up" — the same treatment the two internal findings
(A10/A11, `docs/ROADMAP.md`) already got. This file is that write-up:
every claim actually checked, not just read, against this compiler.

## The headline result: Critical #2 was real, and severe

The report's own words: *"Codegen lowering of requires / acquire /
masking ... This is the real security boundary. Soundness here
determines whether the static story survives into binaries."* Acting
on exactly that steer — not the report's own probe 03 sketch, which was
a scaffold, but a real end-to-end HTTP exploit attempt against a real
compiled binary — found a genuine, severe bug:

**`nirdosha build --serve` let a client forge its own `RoleView` via
the request body, bypassing field-level masking.** A real compiled
binary, exposing `fn list_employees(caller: RoleView) -> Employee
requires(role: "hr_staff")` where `Employee.salary requires(role:
"admin")`, given a real demo-mode bearer token proving only
`hr_staff`:

```sh
curl -X POST http://localhost:8080/api/list_employees \
  -H "Authorization: Bearer <hr_staff-only token>" \
  -d '[{"role":"admin"}]'
# before the fix: {"department":"Engineering","name":"Ada","salary":150000.0}
# after the fix:  {"department":"Engineering","name":"Ada","salary":0.0}
```

`RoleView` is unforgeable *inside* compiled `.nir` code —
`RoleView("admin")` is a compile-time error
(`typeck::UnforgeableProofConstruction`), the exact property
`examples/attack_demo/` demonstrates and probe 01 (below) reconfirms.
Compiled `serve`'s HTTP route wrapper was the one caller that wasn't
itself already-typechecked `.nir` code holding that guarantee, and
nothing filled the gap: `codegen.rs::emit_serve_route_wrapper` special-
cased `VerifiedIdentity` parameters to be filled from the verified
JWT, but `RoleView`/`ClaimView` fell through to the same generic
per-argument JSON decode any ordinary struct parameter gets — so a
client supplying `{"role":"admin"}` in the request body got a real,
functioning `RoleView` handed straight into the callee, no different
from one `check_role` had actually produced.

**Fixed same day.** Full technical writeup, both parts of the fix, and
the exact regression tests: `docs/ROADMAP.md`'s **A18** entry (search
for it) and `docs/PUBLIC_ROADMAP.md`'s corresponding `[DONE]` entry.
Short version: `typeck::check_serve_exposure` now refuses to compile an
exposed `RoleView`/`ClaimView` parameter with no matching
`requires(role/claim: ...)` to anchor it
(`ExposedFnRoleViewParamUnverifiable`), and
`codegen.rs::emit_serve_route_wrapper` constructs the real value from
the already-verified `nir_check_role`/`nir_extract_claim` result
instead of ever decoding one from `args_json`. Verified against the
real compiled binary with the exact exploit payload:
`crates/compiler/tests/codegen.rs::compiled_serve_never_lets_a_client_supplied_role_view_bypass_field_masking`.

## The other three probes, run for real

| # | Claim | Result |
|---|---|---|
| 01 | `RoleView` unforgeable by direct construction | **Holds.** `nirdosha verify` on the submitted probe: `DISPROVED`, typecheck error at line 24 — `` `RoleView` can't be constructed directly — it's a proof value only `check_role`/`extract_claim` may produce, against a real validated identity ``. Exactly the claim, exactly as claimed. |
| 02 | Field masking fail-closed | The submitted probe is a scaffold, not an executable test (its own comment: "If masking is correct, salary must be 0.0 regardless" — but nothing in the file actually checks that) — it doesn't compile as written (a `let` with no type annotation on a `Result`). The property itself is independently confirmed, though: `examples/attack_demo/agent_b/hr_assistant.nir` (re-run 2026-09-11) shows a real `hr_staff`-only caller getting `salary: 0.000000` back, direct-call path, and the *serve* path is now covered too, by the Critical #2 fix and its regression test above. |
| 03 | Deny-by-default `serve` exposure | **Holds.** Built the submitted probe with `nirdosha build --serve`, ran it, and probed both routes for real: `curl .../api/secret_admin_only` (never exposed) → real `404`; `curl .../api/list_employees` (exposed) with no `Authorization` header → real `401`. The deny-by-default claim is not just present in `typeck.rs` source, it's the actual observed behavior of a real running binary. |
| 04 | Incomplete `&` exclusivity model | Already disclosed, not new: `README.md`'s own "What these guarantees do *not* cover" section and `SECURITY.md`'s scope section both already state this exactly (only `box`'s affine tracking is fully enforced; `&` has no `&mut`-style liveness/exclusivity yet). The submitted probe is a placeholder (its own comment: "exact syntax depends on current language surface"), consistent with this being a known, named gap rather than a fresh finding. |

## The starter kit's own recommendations

1. *"Run every adversarial probe through `nirdosha verify` + `certify`
   and capture JSON."* Done for 01/03 above (02/04 aren't independently
   runnable as submitted, for the reasons in the table).
2. *"Build the serve sketch and fuzz the HTTP surface."* Done for the
   specific case that turned into Critical #2's fix; a general fuzz
   harness across arbitrary methods/headers/encodings is real, further
   work, not attempted here.
3. *"Add the four probes (or hardened versions) to the project's own
   test suite as permanent negative tests."* Done, in spirit: probe
   01's exact claim already has independent coverage
   (`crates/compiler/tests/`'s existing `RoleView`-forgery tests); the
   Critical #2 fix ships 6 new typeck tests plus the real HTTP-level
   regression test above; probe 03's deny-by-default claim already has
   `crates/compiler/tests/serve_exposure.rs` coverage at the typeck
   layer.
4. *"Publish the starter-kit pattern ... under `docs/redteam/` or
   linked from `SECURITY.md`."* This directory, and `SECURITY.md`'s
   red-team invitation section, both now do.
5. *"Consider a public 'known gaps' page."* Already exists in substance
   — `SECURITY.md`'s "Scope" section and `docs/PUBLIC_ROADMAP.md`'s
   `[PARTIAL]`/`[OPEN]` tags — not restructured into a new page for
   this pass.

## One more thing this pass found, not in the original report

While reproducing probe 03's build, `nirdosha build --serve` printed:

```
35:1: warning: `main` has no `requires(...)` and takes no `VerifiedIdentity` parameter — it will be callable by anyone with no token at all once served
```

...for a `main` that the exposure model never actually routes (`curl
.../api/main` 404s, confirmed). This is a real, disclosed, **not
fixed in this pass** loose end — `typeck::ungated_fn_warnings` predates
compiled `serve`'s real deny-by-default dispatch and still scans every
declared `fn`, not just `exposed_fn_names`. An attempted one-line
filter fix broke `tests/ungated_fn_warning.rs`'s own deliberate
"warn on any ungated fn, exposed or not, as an early lint" design —
reconciling the two needs a real design decision, not a quick patch.
Left as a named follow-up (`docs/ROADMAP.md` A18's closing paragraph),
not silently dropped.

## Thank you

This is the first external submission through `SECURITY.md`'s red-team
invitation, and it found a real, severe bug in exactly the area the
project's own docs pointed at as the highest-risk surface. That's the
invitation working as intended.
