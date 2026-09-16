# The v2 golden corpus

**Every showcase `.nir` example, translated to v2 today** — valid Rust
inside, extension intact — so that post-migration, the *same
unmodified files* run under the proprietary compiler and gain the
Nirdosha-specific layer (generated UI, durable workflow store, saga
runtime, proofs in the certificate).

The corpus is the migration's fixed target: `tests/outputs.rs` pins
every example's observable behavior under plain cargo; the proprietary
compiler must reproduce it exactly, then add.

```
$ cargo test -p nirdosha-v2-corpus          # 12 golden tests + identity fixture
$ cargo run -p nirdosha-v2-corpus --bin v2_enterprise_app
```

## The contract this corpus enforces

| reader | today (pre-migration) | post-migration, same files unmodified |
|---|---|---|
| plain `cargo` | builds & runs; comment layer inert; **outputs pinned by tests/outputs.rs** | identical |
| `cargo nirdosha` | verifies the `nirdosha:contract` claims (live today); workflow/screen/validate/transact comments ignored until Phase 1 | all comment kinds cross-referenced + certified |
| proprietary `nirdosha` | — | `emit-ui` renders the screens/dashboards; workflow gains its durable store; transact gains durability enforcement; proofs discharge the `nirdosha:validate` clauses |

## The translated set (12 files, behavior-verified)

| v2 file (`.nir` extension kept) | constructs | golden pins |
|---|---|---|
| `hello_nir.nir` | the minimum | `8`, `hello, nirdosha` |
| `02_types_and_control_flow.nir` | scalars, operators, if-expr, while, recursion | `3628800`, `Negative()`… |
| `03_data_modeling.nir` | structs/enums/match, generics, Option, `Vector`/`Matrix`, `Money`/`Measure` | `[5, 7, 9]`, `[[1, 3], [2, 4]]`, `19.99`, `USD()` |
| `04_ownership_and_concurrency.nir` | `Box` moves, `Frozen`, `&`, `spawn`/`join`, `Chan`, `sandbox` | `42`s, `-1` |
| `05_platform_services.nir` | `effect(pure/io)` contracts, `validate`, file/db/json/http/mq | `ada`, `hello from level 5` |
| `06_shared_lib.nir` + `06_identity_and_declarative_ui.nir` | `#[path]` modules, `requires`/`acquire`, `transact`, `workflow`, `screen`, `dashboard` | `500`, `approval 1 approved` |
| `35_validate_contracts.nir` | `pre:`/`post:` Hoare clauses | `9`, `0`, `7`, `5` |
| `36_transact.nir` | full saga: precheck/network/verify/commit/compensate/log | `committing 10`, `logged -5 false` |
| `38_workflow.nir` | owner-gated workflow (`role("finance_director")`) | `approval 1 for amount 250000` |
| `39_screen_ui.nir` | `screen` + CRUD conventions + `requires(role)` | restocked `1` |
| `40_dashboard.nir` | `dashboard` tile/chart | `12` |
| `enterprise_app.nir` | **everything**: CRUD, workflow, saga, screens, dashboard, `box`, `chan`, `mq`, audit trail, acquire denials | `reversing disbursement for purchase order 1`, `-2` |

## The comment kinds, and their status

| kind | meaning | today | Phase 1+ |
|---|---|---|---|
| `nirdosha:contract` | effects / requires / nfr — the shipped dialect encoding | **verified by `cargo nirdosha`** | — |
| `nirdosha:validate` | `pre:`/`post:` refinement clauses | inert | cross-referenced; Z3 discharge (2.5) puts proofs in the certificate |
| `nirdosha:transact` | the saga claim (which fns are verify/commit/compensate/log) | inert | body checked against claim |
| `nirdosha:workflow` | states/transitions/owner/on_entry | inert | generates the runtime; hand-desugared fns must match |
| `nirdosha:screen` / `nirdosha:dashboard` | UI declarations | inert | `emit-ui` renders from them |
| `nirdosha:serve` | serve-surface metadata (`requires_public`) | inert | `serve` consumes |

## Where things live

- **Real prelude** (`crates/nirdosha-rt/src/prelude.rs`, entry #11):
  `Chan`, `Frozen`, `spawn`/`join`, `Sandbox`, `NirFile` + the
  `Stoppable` trait, `Dec128`, `Money`, `Measure`, `Vector`, `Matrix`,
  `txn_id`. Post-migration the proprietary runtime replaces the
  corpus-grade bodies behind these same names.
- **Fixture surface** (`src/lib.nir`, the `nirdosha_v2` lib):
  `db` (in-memory row store, per-table rowids), `json` mini-extractors,
  `http`/`mq` (always `Err` — exactly the no-listener behavior the
  originals document), the fixture identity (`mock_issue_token` →
  `oidc_validate_token` → `check_role` → `acquire`), workflow errors,
  instance ids. Post-migration: SQLite, real json, TcpStream/TLS,
  Redis, Row-12 identity.

## Mechanical deltas at call sites (documented, not hidden)

| `.nir` | v2 (still valid Rust) |
|---|---|
| `Point(3.0, 4.0)` | `Point { x: 3.0, y: 4.0 }` (named construction) |
| `spawn double(21)` | `spawn(move \|\| double(21))` (closure form) |
| `send(out, "line")` (file) | `out.send("line")` (method; chan keeps `send(c, v)`) |
| `Negative()` / `None()` | `Classification::Negative` / `None` |
| `print(x, y)` | `println!("{x} {y}")` |
| `use "06_shared_lib.nir"` | `#[path = "06_shared_lib.nir"] mod shared_lib;` |
| `sandbox background_work()` | `sandbox(move \|\| background_work())` — thread stand-in today (real OS process post-migration; `stop` reports `-1` for a still-running worker either way) |
| floats | Rust's `Display` (e.g. `0.7899999999999999` vs `.nir`'s `0.79`) |

## The remaining 51 files — the full index

All under `examples/` in the proprietary repo; status: **translated** /
mechanical (pure-Rust constructs, near-1:1) / prelude-ready (needs only
the shipped prelude) / fixture-ready (needs only the fixture surface) /
comment-kind (needs a Phase-0 grammar item) / proprietary (needs the
real kernel; fixture approximates).

| file | class |
|---|---|
| syntax/hello_nir, 02, 03, 04, 05, 06(+shared), enterprise_app | **translated** |
| features/01–08 (scalars, operators, control flow, fns, structs, generics, Option/Result, str boundary) | mechanical |
| features/09 Money, 10 Measure, 11 dec128, 12 linalg, 13 deterministic simulation | prelude-ready (09–11 verified via 03; 12–13 need general `det`/`transpose` beyond 2×2) |
| features/14 box, 15 borrowing | mechanical (`Box`, `&`) |
| features/16 effects | **translated pattern** (see 05) |
| features/18 threads, 19 channels, 48 froze | prelude-ready (verified via 04) |
| features/20, 21 sandbox(+channels) | prelude-ready (thread stand-in; process post-migration) |
| features/24 file io, 25 json, 27 database, 30 identity, 31 mock IdP, 50 field masking | fixture-ready (verified via 05/39) |
| features/22 tcp client, 23 tcp listener, 26 http/https, 28 mq, 29 crypto, 32 sessions, 47 external boundary | proprietary services (fixtures approximate: `Err`-on-connect) |
| features/33 privileged fns, 34 requires(public) | comment-kinds `contract`/`serve` (verified via 39) |
| features/35 validate, 36 transact, 38 workflow, 39 screen, 40 dashboard | **translated** |
| features/37 transact cross-process | prelude + `nirdosha:transact` (pattern: 36 + 20) |
| features/41 dashboard visual, 42 workspace panel, 43 layout, 44 module nav | comment-kinds (Phase-0 grammar: widgets/panels/layout) |
| features/45a/b module namespacing | mechanical (`#[path]` modules, verified via 06) |
| features/46 db schema/role conventions | fixture-ready + conventions |
| features/49 nfr | comment-kind `contract` (nfr) — runtime guard rides the dialect's `#[contract]` macro form |
| features/51 compiled serve, 52–54 compiled workflows | `nirdosha:serve`/`nirdosha:workflow` kinds (52–54 = the 38 pattern) |
| benchmarks/ matmul, det, kalman, dot, fib, floatloop | mechanical + prelude (linalg) |
| 17 audited block | comment-kind (`nirdosha:audit`) — Phase-0 grammar item |

**Nothing in the remaining set needs a new *mechanism*** — every class
above already has a working pattern in the translated twelve or is a
Phase-0 grammar item recorded on the register.