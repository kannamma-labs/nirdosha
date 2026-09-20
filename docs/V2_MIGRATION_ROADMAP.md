# Nirdosha product roadmap — the pickup map

Written 2026-09-16, after a full codebase verification sweep. Everything
marked **[VERIFIED]** was confirmed against the tree/branch named beside
it this date. Where a fact needs re-confirmation on pickup, the exact
command is given. If a claim here fails its verify command, trust the
command, not this doc — then update this doc.

## 0. The end state (the one-paragraph product)

One compiler. Input: a `.nir` program (native syntax). Output: **one
self-contained native binary** — statically linked, no runtime
dependency on the compiler's installation [VERIFIED: `codegen.rs`
embeds `runtime_kernels.rs` static-lib bytes via `include_bytes!`].
Depending on what the program declares, the compiler links in only
what it uses: an API component → a hardened HTTP server **and the APM
kernel** are inside the binary; a UI → its rendering assets are inside
the same binary; nothing unused ships. The binary's crypto is
FIPS 140-3-module-backed (ADR 0012), its packs are Sigstore-pattern
signed, and its evidence is carried in guarantee manifests. `nirdosha
hi` is the authoring + verification console on top. The free tier
(the Rust dialect + `cargo nirdosha`) remains the parallel adoption
lane — different product, same semantics, same contract grammar.

**The strategic decision this roadmap encodes (2026-09-16, user):**
the product is native-first. There is **no transpiler milestone** —
`.nir` files are never converted to the v2 dialect shape; the dialect
is the free lane, not a migration step. hi generates `.nir`
natively (it already does).

## 1. Repo & branch topology — where everything lives [VERIFIED]

| Location | What it is | Status |
|---|---|---|
| `/home/arun/nirdosha`, branch **`codegen-refactor`** | **THE FRONTIER. 164 commits ahead of `remove-interpreter`.** hi system, FIPS ADR + compliance profiles, pack signing, Tier-2 Z3, APM loop, self-repair hints, red-teamed compiled-serve, deadlock-detector fixes, guarantee manifests (RFC 0017), domain packs (RFC 0016) | product work continues HERE |
| `/home/arun/nirdosha`, branch `remove-interpreter` | the cleaned base the free-tier fork was cut from; 7-subcommand CLI | frozen base |
| `/home/arun/nirdosha`, branch `feat/nirdosha-hi-agentic-console` | hi's first console cut; **fully contained in `codegen-refactor`** (0 commits the other way) | superseded |
| `/home/arun/nirdosha-rt`, branch `rt-dialect/stage-1` | the free tier: dialect, contract-core, cargo-nirdosha, driver, G1–G6 work, the 49-file v2 corpus | free lane |
| `runtime-kernels` exists in **BOTH** repos, files **differ** (recorder.rs at minimum) | kernel drift between lanes | decision needed (§6-W6) |

Verify on pickup:
```bash
git -C /home/arun/nirdosha rev-list --count remove-interpreter..codegen-refactor
git -C /home/arun/nirdosha log codegen-refactor --oneline -5
git -C /home/arun/nirdosha-rt status -sb   # expect: 4 dirty files fixed by M0
```

## 2. THE COMPILER — what is there [VERIFIED, file-by-file]

All paths relative to `/home/arun/nirdosha/crates/compiler/src/` on
**`remove-interpreter`** unless the row says `codegen-refactor` (where
it means: present on the frontier; verify with
`git show codegen-refactor:crates/compiler/src/<file> | head`).

### 2.1 Front end — DONE
| File | Lines | What it does |
|---|---|---|
| `token.rs` / `parser.rs` | 521 / 2258 | LL(1) hand-written parser, cross-verified |
| `ast.rs` | 2645 | full program model incl. `NfrSpec`, workflow/screen/serve decls |
| `typeck.rs` | 5188 | static type checker, generics, `Option`/`Result` |
| `effects.rs` | 512 | effects analysis — the fact source for conditional embedding (§3-W1) |
| `ownership.rs` | 871 | affine checks, `box`/`&`, `FreeMap` driving codegen's `nir_free` |
| `refine.rs` | 676 | `validate` Hoare-style pre/post refinement checking |
| `smt.rs` | 699 | Z3 integer/overflow proofs; Tier-1/2 bounds-check elision |
| `workflow_lower.rs` + `workflow_conformance.rs` | 398+183 | workflow state-machine lowering + conformance |
| `contract_check.rs` | 909 | contract/claim checking |
| `extraction_schema.rs` | 163 | tool-facing extraction schema |

### 2.2 Back end — DONE for the compiled subset
| File | What it does |
|---|---|
| `codegen.rs` (**9012 lines**) | the whole native backend: `check_supported` is the ground-truth list of what compiles (grep it, don't trust docs); LLVM IR text emission; `nfr` auto-instrumentation (`nir_nfr_call_begin/end`); affine frees; SMT-elided bounds guards; match→switch; the fully-unrolled linalg cluster; `emit_c_main`; `OptLevel -O0/-O2`; **`build()` — clang/linker invocation producing the native binary** |
| `RUNTIME_KERNELS_LIB` (in codegen.rs) | `include_bytes!`-embedded static-kernel library — **compiled binaries are already self-contained** |

### 2.3 Product layer — present, needs the packaging pass (§3)
| File | What it does | Gap |
|---|---|---|
| `ui_gen.rs` (2318) + `ui_gen_template.html` | UI generation from screen/dashboard DSL; `emit-ui` writes HTML **beside** the binary | embed into the binary (W2) |
| `ui_plugin.rs` + plugin crates | plugin ABI + reference plugins (shout/kv/authed-http) | exists |
| `crud_gen.rs` / `migrate.rs` | zero-syntax CRUD; additive schema sync | exists |
| `rqlite.rs` (427) | durable tx log substrate | exists |
| `init.rs` | project scaffolding | exists |

### 2.4 Runtime kernel — present, single-artifact today (§3-W1 splits it)
`crates/runtime-kernels/src/kernel/`: admission control
(`acquire/release/stats/dump_report` over **Tcp/File/Thread/Db/Mq**
domains, env-var ceilings, fail-open), **flight recorder**
(double-buffered 1024-event pages, 10-byte events, never-block,
gzip flush on thread pool, `flush_remaining` at exit), **NFRs-as-language**
(`nfr(latency_ms/error_rate_max/throughput_min_per_sec/concurrency_max)`
→ compiler-injected per-call tracking; violation → fire-and-forget
JSON POST to `NIRDOSHA_OBSERVABILITY_URL`), **concurrency observability**
(`wait_begin/end` with `ChanRecv/ThreadJoin` targets — the deadlock
plane). Explicitly missing per its own docs: OTLP export, sampling,
real sink pipeline.

### 2.5 hi — present and dogfooded (on `codegen-refactor`)
| File | What it does |
|---|---|
| `hi_api.rs` | transport-neutral route table |
| `hi_graph.rs` + `hi_graph.html` | the graph store (`CandidateUnit`) — units, not whole programs |
| `hi_llm.rs` | the ONE outbound-network module; RFC 0014's prompt→build→generate→publish pipeline; env trio `NIRDOSHA_LLM_PROVIDER_KEY/MODEL/BASE`; bounded retry-against-real-diagnostics |
| `hint_cache.rs` / `self_repair_hints.rs` | hint cache + one shared home for self-repair hints, stall detection |
| `hi_server.rs` / `hi_window.rs` / `hi_preview.rs` | server, window UI, live preview |
| `hi_plugin.rs` | plugin surface |
| `cmd_hi` (main.rs) | thin: `resolve_activation` + `run_console` — activation contract is unit-tested in the library half |

The hi stream also delivered: **FIPS 140-3 crypto backend swap
(`docs/adr/0012-fips-140-3-crypto-backend-swap.md`)**, **compliance
profiles wired for real**, **Sigstore-pattern pack signing +
certificate-mandatory publish (RFC 0016 Phase 4)**, **RFC 0017
guarantee manifests**, **Tier-2 verification (loop invariants,
quantifiers, real Z3 arrays)**, **APM loop closure (drift detection +
hint cache)**, red-team P1/P2 compiled-serve fixes, deadlock-detector
report/abort race fix, db-kernel deadlock fix, a use-after-free fix in
ownership.rs. 13 `hi:` commits 2026-09-14→15.

### 2.6 What hi does NOT do
It generates **native `.nir`** programs as graph units. It does not
emit v2-dialect files — and per the native-first decision it doesn't
need to. The v2 corpus remains the free lane's proof and pattern book.

## 3. THE COMPILER — what needs to be built (work items)

Ordered; each has an exit criterion and the file it touches. These
run on `codegen-refactor`, which should be promoted to the working
main of the product first (W0).

| # | Work item | What exists to build on | Exit criterion | Est. |
|---|---|---|---|---|
| **W0** | Promote `codegen-refactor` to the product base; merge strategy with `remove-interpreter`-cut forks (nirdosha-rt) documented; **runtime-kernels drift resolved** — one home, one consumer pattern (nirdosha-rt already consumes it as an ordinary dependency via G3) | branch topology (§1) | a written merge map; `runtime-kernels` single-sourced or synced with a script; both lanes green | 1d |
| **W1** | **Conditional service embedding**: split the runtime kernel's service surface into link-time-selectable modules; `effects.rs` + serve/workflow/db declarations already tell the compiler exactly what a program touches — drive module selection from that | `check_supported`, `effects.rs`, admission domains | an API-declaring binary embeds http+apm and NOT, say, the db/mq kernels it never uses; a db-only binary embeds no http server; verified by `nm`/size diff | 3–5d |
| **W2** | **UI inside the binary**: `emit-ui` assets linked in; compiled serve renders its own UI (single artifact serves itself) | `ui_gen.rs`, `ui_gen_template.html`, compiled serve | `nirdosha build` of an app with a UI produces one binary whose serve path serves the UI from embedded bytes; no sidecar files | 2–4d |
| **W3** | **HTTP path promotion**: the compiled serve's HTTP handling hardened to the "high-performance" claim — bind a vetted server stack behind the kernel surface (same never-hand-roll decision as TLS), keep admission+APM boundary hooks | compiled serve (red-teamed), kernel `nir_tcp_*` | a load-shaped test (concurrent conns, keep-alive) passes against the embedded server; admission events still recorded | 3–5d |
| **W4** | **APM egress**: OTLP export wiring (the recorder's own doc names what's missing: OTLP, sampling, real sink), request-level spans on serve, TLS on the violation POST (rustls policy), p99/sliding windows as a disclosed upgrade | `recorder.rs`, `nfr.rs`, RFC 0007 §6 | a binary exports real OTLP traces/metrics to a local collector; spans exist per request; NFR stats still O(1) hot path | 4–6d |
| **W5** | **FIPS module path**: implement ADR 0012's backend swap end-to-end if not already complete; compliance profile = build-time selection; binary carries the validated-module story | ADR 0012, compliance profiles (hi stream) | a compliance-profile build uses the FIPS-backed crypto modules; profile is recorded in the guarantee manifest | 2–4d |
| **W6** | **Artifact evidence**: guarantee manifest (RFC 0017) + pack signature + provenance in the shipped binary; single-auditable-artifact doc | RFC 0016/0017, pack signer, G6 provenance work (free lane, portable) | the binary embeds its manifest; `verify-artifact` reproduces evidence offline | 2–3d |

## 4. Milestones (revised — much closer than the transpiler-era plan)

- **M0 — land the free-lane tail (today)**: fix-set commit, push 10+
  commits, README, #65 post. [in progress, this session]
- **M1 — product base (W0)**: ~1 day. `codegen-refactor` promoted,
  drift resolved, merge map written.
- **M2 — the single binary (W1+W2)**: ~1 week. The artifact you
  described exists: one binary, conditional services, UI inside.
- **M3 — serve+APM+compliance (W3+W4+W5)**: ~2 weeks cumulative.
  High-performance API binary with built-in APM egress and FIPS-backed
  packaging.
- **M4 — hi as the front door, product mode**: ~days after M2 (hi
  already exists; this milestone = hi's pipeline drives build→verify→
  publish of the single binary and shows the artifact's evidence).
  **The answer to "when can we run `nirdosha hi`": the console runs
  today on `codegen-refactor` (dogfooded); it becomes the
  generate-the-audited-binary front door at M4, ≈2–3 weeks out.**

## 5. The free lane (nirdosha-rt) — parallel, not blocking

Done: dialect + contracts + certificates; G1, G2, G4; G3 (native
adapters: real JWT via runtime-kernels, WAL saga, kill/restart
tests); G5 scaffolding (honest cross-reader reporting);
G6 mechanical half (provenance); the 49-file v2 corpus with 55
golden tests green; 68 contracts, 0 violations.
Remaining (its own register, not the product's critical path):
G6 artifact-bytes + cfg binding; #13 signed trust chain; G3's
shared-ABI tests; Stage 2.5 Z3-over-MIR for free-tier `proofs`;
Phase-1 scanner teeth completion. `docs/V1_CAPABILITY_PORT_MAP.md`
tracks this same "still needed, not yet built in v2" register
per-capability, for the specific case of something `crates/compiler`
(now deleted) used to do — the Z3-over-MIR item above and this repo's
still-kept-but-currently-unlinkable native-plugin reference crates are
both rows there.

## 6. Decisions recorded (all dated 2026-09-16, user-driven)

1. **Native-first**: no transpiler; `.nir` is the product input; the
   v2 dialect is the free lane, not a migration step. (Reverses the
   earlier Phase-3 transpiler plan; superseded by this doc.)
2. **Never hand-roll crypto/TLS**; rustls policy on the free lane;
   ADR 0012's FIPS 140-3 backend on the product; full CMVP
   certification of the whole binary = optional later program, not a
   gate.
3. **Vetted components over hand-rolled** for the HTTP server and APM
   egress (the kernel's admission/APM boundary hooks remain ours).
4. **runtime-kernels drift must be resolved at W0** — one home, one
   consumer pattern.
5. **APM scope today** (§2.4) is the honest baseline: always-on,
   never-block, drop-with-accounting + NFRs-as-language; OTLP/spans
   are W4, not existing claims.

## 7. Pickup protocol (how anyone starts)

1. Read §1, run the three verify commands. If the free lane is dirty,
   M0 wasn't landed — land it first (see `docs/V2_IMPLEMENTATION_BOOK.md`
   for its work log).
2. Product work: `git -C /home/arun/nirdosha checkout codegen-refactor`,
   read `crates/compiler/src/INDEX.md` (durable names; line numbers
   drift), then §3's table for your work item. Start only from a
   green suite: `cargo test -p compiler --offline`.
3. Free-lane work: `docs/V2_IMPLEMENTATION_BOOK.md` is the work log;
   `docs/V2_GUARANTEES.md` is the guarantee profile; issue #65 is the
   decision register.
4. Update this doc when a verify command disagrees with it. That is
   the only rule that keeps this map alive.