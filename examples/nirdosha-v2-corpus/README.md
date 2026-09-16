# The v2 golden corpus

**The showcase translated: 49 of 63 files are v2 today** — valid Rust
inside, extension intact — so that post-migration, the *same
unmodified files* run under the proprietary compiler and gain the
Nirdosha-specific layer (generated UI, durable workflow store, saga
runtime, proofs in the certificate). The 12 not translated are
interpreter-only-designed by decision (register, issue #65) — see the
skip register below.

The corpus is the migration's fixed target: `tests/outputs.rs` pins
every example's observable behavior under plain cargo; the proprietary
compiler must reproduce it exactly, then add.

```
$ cargo test -p nirdosha-v2-corpus          # 55 golden tests, all green
$ cargo run -p nirdosha-v2-corpus --bin v2_enterprise_app
```

## The contract this corpus enforces

| reader | today (pre-migration) | post-migration, same files unmodified |
|---|---|---|
| plain `cargo` | builds & runs; comment layer inert; **outputs pinned by tests/outputs.rs** | identical |
| `cargo nirdosha` | verifies the `nirdosha:contract` claims (live today); 59 files / 68 contracts / 0 violations | all comment kinds cross-referenced + certified |
| proprietary `nirdosha` | — | `emit-ui` renders the screens/dashboards; workflow gains its durable store; transact gains durability enforcement; proofs discharge the `nirdosha:validate` clauses |

## The translated set (49 files, behavior-verified)

Batch 1 (12 golden tests — the showcase core):

| v2 file | constructs | golden pins |
|---|---|---|
| `hello_nir.nir` | the minimum | `8`, `hello, nirdosha` |
| `02_types_and_control_flow.nir` | scalars, operators, if-expr, while, recursion | `3628800`, `Negative()`… |
| `03_data_modeling.nir` | structs/enums/match, generics, Option, `Vector`/`Matrix`, `Money`/`Measure` | `[5, 7, 9]`, `19.99`, `USD()` |
| `04_ownership_and_concurrency.nir` | `Box` moves, `Frozen`, `&`, `spawn`/`join`, `Chan`, `sandbox` | `42`s, `-1` |
| `05_platform_services.nir` | `effect(pure/io)` contracts, `validate`, file/db/json/http/mq | `ada`, `connection refused` |
| `06_shared_lib.nir` + `06_identity_and_declarative_ui.nir` | `#[path]` modules, `requires`/`acquire`, `transact`, `workflow`, `screen`, `dashboard` | `500`, `approval 1 approved` |
| `35_validate_contracts.nir` | `pre:`/`post:` Hoare clauses | `9`, `0`, `7`, `5` |
| `36_transact.nir` | full saga: precheck/network/verify/commit/compensate/log | `committing 10`, `logged -5 false` |
| `38_workflow.nir` | owner-gated workflow | `approval 1 for amount 250000` |
| `39_screen_ui.nir` | `screen` + CRUD conventions + `requires(role)` | restocked `1` |
| `40_dashboard.nir` | `dashboard` tile/chart | `12` |
| `enterprise_app.nir` | **everything**: CRUD, workflow, saga, screens, dashboard, `box`, `chan`, `mq`, audit trail | `reversing disbursement for purchase order 1` |

Batch 2 (43 more files, ~43 more golden tests):

| group | files | notes |
|---|---|---|
| language core | `01` scalars/literals, `02` operators, `03` control flow, `04` first-class fns, `05` structs/enums/match, `06` generics, `07` Option/Result, `08` str boundary | near-1:1 mechanical |
| money & math | `09` Money, `10` Measure, `12` linalg (`transpose`/`det`/`inv`/matmul/`cross`/`solve`/`rank`…), `13` deterministic simulation (seeded SplitMix64, Kalman, geodesy) | prelude-grown; checksums pinned (det `742528617.5592908`, dot `352531173.3352983`, matmul `187065526.66763383`, fib `9227465`, floatloop `1499998.4998404514`, kalman `1999.989999998326`) |
| ownership & concurrency | `14` box, `15` borrowing, `16` effects, `17` audited block (`// nirdosha:audited` + `'audited:` label), `18` threads, `19` channels | `17` is the one `//`-form comment kind (doc comments can't attach to statements) |
| real network | `22` tcp client, `23` tcp listener, `24` file io | real `std::net` sockets; fixed ports (9700/9701) |
| contracts & services | `34` requires public, `37` transact cross-process (real 2-socket txn, `txn-` ids), `47` external service boundary (honest fallthroughs) | |
| declarative UI | `41` visual (graph/heatmap/timeline), `42` workspace panel, `43` layout, `44` module nav, `45(+helper)` `#[path]` namespacing, `46` schema/role-mapping conventions | new kinds: `visual`, `workspace`, `layout`, `nav`, `schema`, `role_mapping` |
| masking & nfr | `48` froze, `49` nfr (error_rate_max/throughput_min_per_sec added to the model), `50` field masking (`mask_f64_unless` + `nirdosha:field`) | |
| compiled serve & workflows | `51` serve (real socket golden test: spawns the bin, GETs `/api/hello`, `/api/echo`, 404), `52` state machine, `53` notifications (real HTTP POSTs w/ `Bearer` keys; fake-Redis driver in the test), `54` escalation | 53/54 pin exact sequences |
| benchmarks | `bench_matmul`, `bench_det`, `bench_kalman`, `bench_dot`, `bench_fib`, `bench_floatloop` | in the regular suite; all <2.5s debug |

## The skip register (12 files, interpreter-only — by decision, not omission)

`11` dec128 deep-dive, `20` sandbox, `21` sandbox_channels, `25` json,
`26` http/https, `27` database, `28` mq, `29` crypto_hashing, `30`
identity_oidc, `31` mock_identity_provider, `32` sessions, `33`
privileged_functions.

Why: these examples were designed against the **interpreter's**
runtime services (real TLS, real SQLite, real Redis, real JWT bytes).
The empirical JWT check confirmed the static token in 30/33/50 is not
reproducible HMAC output. The patterns they demonstrate are already
carried by ported files (05/enterprise: db-json-http-mq fixtures;
06/39: identity chains; 04: sandbox/chan), and their runtime services
are re-specified by the proprietary tier (G3's native adapters already
verify real JWT signatures and run WAL-backed sagas). Fixture
archaeology of the deleted interpreter would have bought nothing.

## The comment kinds, and their status

| kind | meaning | today |
|---|---|---|
| `nirdosha:contract` | effects / requires / nfr — the shipped dialect encoding | **verified by `cargo nirdosha`** |
| `nirdosha:validate` | `pre:`/`post:` refinement clauses | inert (scanner teeth Phase 1) |
| `nirdosha:transact` (+`retry`/`timeout`) | the saga claim | inert |
| `nirdosha:workflow` | states/transitions/owner/on_entry | inert |
| `nirdosha:screen` / `dashboard` / `visual` / `workspace` / `layout` / `nav` | UI declarations | inert |
| `nirdosha:serve` (+routes) | serve-surface metadata | inert |
| `nirdosha:schema` / `role_mapping` / `field` | db-convention + masking declarations | inert |
| `nirdosha:audited` | audit-block justification (the `//`-form kind) | inert |

## Where things live

- **Real prelude** (`crates/nirdosha-rt/src/prelude.rs`): `Chan`,
  `Frozen` (with `Deref` — `*f` works), `spawn`/`join`, `Sandbox`,
  `NirFile` (read cursor), `Tcp`/`TcpListener` wrappers (`recv`
  returns `""` on a dead connection — servers survive port scans),
  `Stoppable`, `Dec128` arithmetic, `Money`, `Measure`/`UnitCode`,
  `Vector`/`Matrix` + the linalg set, `sleep_ms`, sim set
  (`rand_seed`/`rand_f64`/`rand_gaussian`/geodesy/Kalman), `txn_id`.
- **Fixture surface** (`src/lib.nir`, the `nirdosha_v2` lib): `db`
  (in-memory row store, per-table rowids, literal+`?` INSERT parsing),
  `json` mini-extractors, **mq = a real minimal RESP client** (PINGs;
  reachable Redis/test-double yields `Ok`, else honest `connection
  refused`), `send_email/sms/push/notify` (real HTTP POSTs with
  `Authorization: Bearer`, provider rows from the db store), fixture
  identity chain, `acquire` is **fallible** (`Result` — matches the
  .nir denials), `WorkflowActionError`. Post-migration: SQLite, real
  json, TLS, Redis, Row-12 identity replace these bodies.

## Mechanical deltas at call sites (documented, not hidden)

| `.nir` | v2 (still valid Rust) |
|---|---|
| `Point(3.0, 4.0)` | `Point { x: 3.0, y: 4.0 }` (named construction) |
| `spawn double(21)` | `spawn(move \|\| double(21))` (closure form) |
| `send(out, "line")` (file) | `out.send("line")` (method; chan keeps `send(c, v)`) |
| chan into spawn | `let c2 = c.clone();` then move `c2` in (affine-final-owner rule) |
| `froze v` | `let f = Frozen::freeze(v);` — `*f` reads through `Deref` |
| `Negative()` / `None()` | `Classification::Negative` / `None` |
| `print(x, y)` | `println!("{x} {y}")`; unit values print via `{:?}` |
| `use "06_shared_lib.nir"` | `#[path = "06_shared_lib.nir"] mod shared_lib;` |
| `sandbox background_work()` | `sandbox(move \|\| background_work())` — thread stand-in today |
| `acquire(...)` (proof reuse) | `.clone()` the proof; `acquire` returns `Result` |
| tcp ops | `Tcp`/`TcpListener` wrapper methods (`c.send/recv`, `accept(&l)`) |
| floats | Rust's `Display` (e.g. `0.7899999999999999` vs `.nir`'s `0.79`) |
| db INSERT params | numerals only when shaped like SQL literals (leading-`+` stays a string — `+15550100` is a phone number) |

## Bug post-mortem (this corpus's own quality bar)

Found by the golden suite, fixed, and pinned: a param misalignment
(six params for five `?` placeholders — real POSTs never left the
process); the leading-`+` JSON number shape (a fixture rule, now
documented); `recv` panicking on a dead connection (a server must
survive a port scan); test-design hardening (child Drop-guards so a
failed serve test can never orphan a child holding cargo's pipes;
fake-Redis lifecycle with skip-if-occupied and teardown; a watchdog
that kills a blocked child and fails loudly instead of hanging
silently).