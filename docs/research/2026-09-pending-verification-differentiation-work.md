# Pending work: verification/differentiation strategy (2026-09)

> Tracking doc for everything from the "catch up to parity, then
> differentiate" strategy that is **not** done yet. Companion to
> [`2026-09-competitive-verification-and-signing-landscape.md`](./2026-09-competitive-verification-and-signing-landscape.md)
> (the Phase 1 research) and the session that closed Phase 2 (commit
> `b1b3173` on `codegen-refactor`: the db-isolation checker, Sigstore-
> pattern pack signing, `primitive_exclusivity`, and the `ownership.rs`
> borrow-liveness fix). This doc exists so the backlog survives past one
> conversation instead of living only in chat history.

## The 5-phase framework

| Phase | What it is | Status |
|---|---|---|
| 1 — Research | Study Polonius/Dafny/Prusti/Imandra/Sigstore/Elle | ✅ Done |
| 2 — Close gaps | db-serializability checker, inter-procedural VC gen (found already done), borrow-checker liveness, Sigstore-pattern signing | ✅ Done |
| 3 — Externalize evidence | Publish methodology, invite outside reproduction, an adversarial demo for the new capability | ✅ Items 1 and 3 done; item 2 as done as this environment allows (below) |
| 4 — Close the APM loop | Feed real runtime data back into the proof/generation loop | ✅ All three items done (below) |
| 5 — Domain translation stretch | Pick a new failure class, idealize → build → wire in | ✅ Real minimal version, 2026-09-15 (below) |

## Phase 3 — externalize evidence (next up)

1. ✅ **Done (2026-09-14/15): adversarial demo for the db-isolation
   checker — and both real bugs it surfaced, also fixed.**
   `examples/isolation_demo/` — the exact `killer_demo` corrupting
   transfer, wrapped in a real `transact { }` block, run through the
   real compiled binary with `NIRDOSHA_OBSERVABILITY_URL` pointed at a
   real local listener, **now at `killer_demo`'s own literal 8×250
   (2,000-transfer) scale**. Three real runs, all three escalating real
   anomalies over a real HTTP POST (24,000+ escalations per run at this
   scale); see `examples/isolation_demo/RESULTS.md`.
   **Building it at 8×250 from the start surfaced two real, sequential
   bugs, both root-caused and fixed the same session, not just found**:
   (1) `find_cycles`'s DFS-based cycle enumeration was exponential on
   the dense conflict graph this exact race produces (cliff was ~24-28
   total concurrent transacts) — rewritten as plain graph reachability,
   now strictly `O(V+E)`; (2) the checker's history was never rotated,
   so its still-real `O(ops²)`-ish per-call cost grew unbounded over a
   long run — fixed with `MAX_TRACKED_OPS`-based windowing. With only
   fix 1, the full 2,000-transfer scale still didn't finish in 15
   minutes; with both, it finishes in ~23s. Both fixes landed in a new
   shared crate, `crates/isolation-core`, extracted out of
   `isolation_check.rs` so the same detector backs the live FFI path
   *and* two new tooling surfaces built the same session — see Phase 4
   item 3 and the disclosed-gaps list below. Full before/after numbers
   for both bugs: `examples/isolation_demo/RESULTS.md`'s "Part 2."
2. 🟡 **Mostly already done, checked directly rather than assumed.**
   `crates/bench/RESULTS.md` turns out to already publish real
   methodology, in depth: a "Why three tasks, and why these three"
   section explaining task-design decisions, per-task rationale for two
   independent model runs, a real cross-language (TypeScript/Rust)
   plain-LLM baseline with committed generated-source artifacts, and a
   "Reproduce it" section with exact commands for both a cloud-API-key
   provider and a local-daemon one. What's genuinely still missing is
   the master plan's full six-column comparison matrix (Nirdosha /
   TypeScript / Rust / plain-LLM / LLM+XGrammar / LLM+Imandra, plus
   AlgoVeri/Vericoding) — three of those six columns are done, and the
   other three are named with real, checked-not-assumed blockers in the
   file's own "What this is *not*" section (LLM+XGrammar needs raw
   logit access this environment's Ollama-proxied setup doesn't expose;
   LLM+Imandra needs a commercial license not available here;
   AlgoVeri/Vericoding needs quantifier/loop-invariant reasoning
   `contract_check.rs` was never built to attempt — confirmed by
   reading one real task's spec, not inferred). Nothing left to
   "write and publish" that isn't already there; the remaining gap is
   external resources (an Imandra license, a raw-logit inference stack)
   this environment doesn't have, not missing documentation.
3. ✅ **Done (this session, 2026-09-14).** `SECURITY.md`'s "Areas most
   worth scrutiny" now names the four new Phase-2 capabilities by name
   — Sigstore-pattern pack signing, the isolation checker,
   `primitive_exclusivity` — as fresh, not-yet-externally-scrutinized
   surface, with concrete examples of what a real finding against each
   would look like, and points at this session's own isolation-checker
   scaling fix as a worked example of the invitation actually being
   honored (found, fixed, and the record kept, same day).

## Phase 4 — close the APM loop (✅ all three items real, 2026-09-15)

What was already true before this session: the isolation checker's
escalation path (`isolation_check::escalate`, reusing `nfr.rs`'s
`NIRDOSHA_OBSERVABILITY_URL` client) **is** a real APM-kernel closed
loop — a runtime-observed fact flows back out of the compiled binary
through the same channel NFR violations use. All three items below
were "still just a design" as of 2026-09-14; all three are real code,
tested, as of 2026-09-15.

1. ✅ **Drift detection.** `nirdosha check-drift <file.nir>
   <escalations.json>` (`main.rs::cmd_check_drift`,
   `mcp_tools::nfr_drift`) compares `<file.nir>`'s own declared
   `nfr(...)` commitments against a real, saved escalation log --
   **the exact wire shape `nfr.rs`'s own `send_escalation` already
   POSTs** to `NIRDOSHA_OBSERVABILITY_URL` (`{function,nfr,threshold,
   actual,timestamp_ms}`), reused verbatim rather than a second format
   invented for this. If any commitment drifted, this **actually
   triggers real re-verification** (`run_verify_pipeline`, the identical
   pipeline `verify`/`certify` run) and includes that fresh verdict in
   the output — the RFC's own "trigger re-verification instead of just
   firing an alert," not a passive notification with nothing
   downstream. Tested end to end, 6 tests
   (`crates/compiler/tests/check_drift_command.rs`), including that a
   clean comparison does *not* trigger a needless re-verify and that an
   escalation for an undeclared commitment is correctly ignored.
2. ✅ **Feed real incidents into `hint_cache`.**
   `hint_cache::RuntimeLessons` is a real, separate store (not folded
   into the existing compile-diagnostic-keyed `HintCache`, on purpose —
   see that struct's own doc comment for why the two have genuinely
   different provenance and can't share a mechanism). `nirdosha check-
   isolation --teach <hint>` / `check-drift --teach <hint>` record a
   lesson on a real finding (never on a clean check), keyed by a small,
   fixed incident-kind vocabulary
   (`"transact_isolation_anomaly"`/`"nfr_drift"`) rather than per-
   function or per-anomaly, and only on deliberate human/operator
   confirmation -- there is no automatic "proof" step for a runtime
   lesson the way `promote_validated_hints` has for a compile-time one.
   **Real consultation, not just a write-only store**:
   `hi_llm.rs::generate_from_task_prompt` (the bench harness's own
   generate entry point) proactively surfaces a recorded lesson before
   the model ever sees a task whose own prompt mentions the matching
   construct (`"transact"` / the literal `"nfr("` syntax) --
   `runtime_lesson_guidance`'s own doc comment names the real,
   disclosed v1 limit: keyword matching on the prompt text, not
   AST-shape similarity against the eventual generated code, a bigger,
   separate problem not attempted here (still real, still not
   attempted). **✅ `generate_program`'s own (graph-shaped) prompt
   construction wired 2026-09-15** — the "real, disclosed follow-up"
   this item used to name: `generate_program` now consults the
   identical `runtime_lesson_guidance`/`RuntimeLessons` store against
   `graph_task_message`'s own JSON prompt text, the same call shared
   with `generate_from_task_prompt` rather than a second copy of the
   matching rule. 12 tests total (`mcp_tools`/`hint_cache`/`hi_llm` unit
   tests plus the `--teach` integration tests in both CLI test files) --
   `generate_program`'s own wiring reuses `runtime_lesson_guidance`'s
   existing tests rather than adding new ones (it's a thin, mechanical
   call-site addition around already-tested logic, no new behavior to
   pin beyond what those tests already cover).
3. ✅ **Surface it in the certificate over time.**
   `mcp_tools::Certificate::isolation_violations` (a `Vec<
   IsolationViolation>`, `evidence_tier: "monitored"` per entry, same
   discipline `nfr_commitments` already holds itself to) is real, and
   `nirdosha certify <file.nir> --isolation-log <ops.json>` actually
   populates it from a saved operation-history log, re-issuing the
   certificate with the observed violations attached. Tested end to end
   (`crates/compiler/tests/certify_command.rs`'s `isolation_log_*`
   tests). **What "over time" honestly means here, not oversold**: each
   certificate is still a static, re-issued artifact — attaching a new
   log re-issues a new certificate, it doesn't mutate an old one in
   place or auto-update a standing claim. **✅ The capture half is real
   now too, 2026-09-15** — `runtime-kernels`' `isolation_check::
   maybe_start_capture` (opt-in via `NIRDOSHA_ISOLATION_LOG_PATH`, same
   env-var-gated posture `nfr.rs`'s own `NIRDOSHA_OBSERVABILITY_URL`
   already uses) periodically writes the live process's own `Checker::
   ops()` window to a file, atomically (write-to-temp-then-rename), in
   the identical `Vec<Op>` JSON shape `--isolation-log`/`check-
   isolation` already read — a real producer now exists, closing what
   this bullet used to name as still-missing. Real, disclosed choice:
   periodic (every 2s), not per-op, to avoid reintroducing the exact
   per-call cost `MAX_TRACKED_OPS` was built to bound; a crash between
   two snapshots loses at most one interval's worth of ops, never a
   torn file. Tested (`isolation_check.rs`'s own capture tests).

**Real, disclosed limits across all three, not new gaps**: none of
this closes the loop *automatically* end to end -- a human or an
external process still has to run `check-drift`/`check-isolation`
against a saved log and, for item 2, explicitly confirm a lesson via
`--teach`. What's real is that every step in the chain (declare →
observe → detect drift → re-verify → optionally teach → proactively
guide the next generation) now has working code behind it, wired
together and tested, where before this session none of it did.

## Phase 5 — domain translation stretch (✅ real minimal version, 2026-09-15)

Pick one more failure class nobody solves well for LLM-generated code,
design the idealized solution, build a real minimal version, then wire
it into the existing graph/pack/certificate machinery the same way a
domain pack talks to the coverage gate today.

**Chosen failure class: the artifact-boundary gap named in
`docs/research/2026-09-generated-code-guarantee-evaluation.md`.** This
project could verify/certify a `.nir` *source* file, but had no
checkable artifact that travels with the *generated* code — an
operator or auditor handed only a binary had no Nirdosha-provided way
to ask "does this still satisfy the guarantees the source claimed?"
Idealized in `rfcs/0017-security-guarantee-manifest.md` (per-module
JSON policy contracts, `guarantee_check` compile-time pass, an emitted
guarantee bundle, `verify-binary` as the binary-side dual of `verify`).

**Built, real, tested — the RFC's own explicit v1 scope cut, not
hidden**: `crates/compiler/src/guarantee_manifest.rs` implements 3 of
the RFC's 6 `guarantee_check` items (capability ceiling, exported
`requires`/`public` contract, role/claim vocabulary) — the other 3
(resource-budget static bounding, network/file literal allowlists,
import-boundary checking) need call-graph/codegen-level plumbing or
multi-file manifest threading this pass doesn't reach into yet, real
disclosed follow-up, not silently assumed done. Wired in for real, not
just unit-tested in isolation:
- `nirdosha build` fails the build outright on a real violation when
  `<file.nir>.guarantees.json` exists next to the source, and always
  emits `<out>.guarantees.json` (the RFC §4 bundle: inferred effects,
  gated/public exports, source hash) whether or not a manifest is
  present.
- `nirdosha check-guarantees <file.nir> [--manifest <path>]` — the
  on-demand check without building.
- `nirdosha verify-binary <bundle.guarantees.json> --against
  <policy.json>` — the RFC §6 binary-side dual of `verify`: checks an
  already-built bundle against an operator policy with **no
  recompilation**, the actual "give me the generated code and I'll
  tell you whether it satisfies these guarantees" answer this whole
  doc's Phase 3/4 work was building toward.

All four paths (clean build, build-time hard failure, `check-
guarantees`, `verify-binary` both satisfying and violating a policy)
were run end to end against real compiled output, not just asserted in
unit tests — see the 13 `guarantee_manifest` unit tests plus the manual
smoke run this session recorded. Runtime enforcement (RFC §5 — manifest-
derived resource ceilings/network policy baked into `runtime-kernels`)
is real, separate follow-up work: this phase is the compile-time check
+ emitted bundle + policy check, not the full runtime story.

## Smaller, explicitly disclosed follow-on gaps (lower priority)

Named in the code/docs at the time each was built, not hidden — pick up
opportunistically, not urgently:

- ✅ **Isolation checker has no enforcement surface — fixed 2026-09-15.**
  Used to be detection-only (an async escalation after the fact only),
  no way to run it over a saved log on demand. `nirdosha check-
  isolation <ops.json>` (`main.rs::cmd_check_isolation`) closes this: a
  real 3-valued exit code (0 clean / 1 anomaly found / 2 reserved for
  a genuinely inconclusive case, matching `verify`'s own convention),
  `--in-toto` wrapping, using `crates/isolation-core`'s detector
  directly (the identical algorithm the live path runs, not a
  reimplementation). Tested end to end
  (`crates/compiler/tests/check_isolation_command.rs`). **What's still
  genuinely missing, disclosed in that command's own doc comment**: no
  mechanism yet *produces* an ops-log from a live compiled `.nir`
  process — a log has to come from a caller's own tooling. The
  *checking* half of this gap is closed; the *capture* half is real,
  separate follow-up work.
- ✅ **Isolation checker didn't scale to real contention — fixed
  2026-09-14/15, in two parts.** Used to call `check()` — a graph
  rebuild — on *every single* `db` operation, over the entire history
  of the process's run, never rotated, and the cycle-detection
  algorithm itself was exponential on a dense conflict graph. Both
  fixed: `find_cycles` rewritten as plain graph reachability
  (`crates/isolation-core`, strictly `O(V+E)` per start node, not
  exponential), and `MAX_TRACKED_OPS`-based windowing bounds `check()`'s
  own still-real `O(ops²)`-ish per-call cost to a fixed ceiling
  regardless of run length (`isolation_check.rs`, `clear_shared()` now
  actually gets called automatically). `examples/isolation_demo/
  RESULTS.md`'s "Part 2" has the complete measured before/after for
  both fixes, including the cliff that only the first fix alone still
  left (the full 2,000-transfer scale didn't finish in 15 minutes with
  just fix 1; with both, ~23s). Both fixes are pinned at the unit-test
  level too (`crates/isolation-core::tests::
  dense_conflict_graph_does_not_blow_up`, `isolation_check.rs`'s two
  rotation tests), not just "it worked when tried." **Real, disclosed
  remaining limit, not a new gap**: windowing trades recall (a
  cross-window anomaly is invisible) for the bound — a real tradeoff,
  not a perfect fix; `MAX_TRACKED_OPS = 300` is a measured-comfortable
  value, not a proven-optimal one.
- ✅ **Pack signing UI — the local half wired 2026-09-15; the live half
  stays genuinely blocked.** `agent-skills/nirdosha/
  hi_ux_redesign_options.md`'s "trust indicator" mockup is now wired to
  `hi_plugin::pack_signer_identity`: `hi_api::handle_packs_list`'s
  `/api/packs` response carries a real `signer_identity`/
  `trust_indicator` ("signed"/"unsigned") per pack, sourced from the
  same DB column `verify_and_install_signed_pack` already writes — no
  new mechanism, just the read side that was missing. **Real, disclosed
  scope cut**: this exposes *who* signed a pack, not *how*
  (`trust_anchor` vs `tofu` — that distinction lives only in the local
  append-only signing log, which has no reader yet, real separate
  follow-up). Live Fulcio/Rekor integration (the registry-issued-
  identity trust tier) remains genuinely blocked on the registry-
  governance question (RFC 0016's own position, unchanged) — that part
  of this gap was never touched, because it can't be.
- **Borrow liveness is lexical, not true NLL — deliberately left as is
  this session.** A real, disclosed precision cost (`ownership.rs`'s
  "Borrow liveness (v1, lexical)" section) — rejects some programs a
  full last-use liveness analysis would accept. Considered for this
  session and deliberately not attempted: a real CFG-based last-use
  liveness analysis is a substantial, research-grade rewrite of
  `ownership.rs`'s borrow tracking, not a bounded bugfix, and this
  project's own prior position (unchanged) is that it's worth
  revisiting only if it bites real generated code in practice — no such
  case is on record. Flagged here rather than silently skipped.
- ✅ **`let x: unit = <call>()` didn't compile — fixed 2026-09-14.**
  `layout::llvm_ty` maps `Ty::Unit` to LLVM `void`, and the generic
  let-with-type-annotation codegen path unconditionally alloca'd
  whatever `llvm_ty` returned; `alloca void` is illegal LLVM IR, so
  `clang` rejected the emitted module outright — real at HEAD,
  independent of anything else in flight, confirmed by rebuilding
  `nirdosha` clean and finding `examples/killer_demo/race_probe.nir`
  itself, unmodified, no longer compiled (its own `let slept: unit =
  sleep_ms(2)`). Fixed in `codegen.rs`'s `Stmt::Let` scalar arm,
  mirroring the identical fix `Stmt::Return` already had for a `return
  <unit-typed call>`: evaluate the expression for its side effect, skip
  the alloca/store, register the binding with a placeholder address
  that's never dereferenced (`Expr::Ident`'s own `ty == Ty::Unit` arm
  already short-circuits to a placeholder before ever loading). Full
  `nirdosha` test suite (217 lib tests, 128/129 codegen integration
  tests — the one failure is a pre-existing, unrelated live-MQ-broker
  dependency) passes; `race_probe.nir` itself, unmodified, compiles and
  reproduces the race again (3/3 runs, nonzero drift), restoring
  `killer_demo/RESULTS.md`'s own "actual run of the actual code" claim.
- ✅ **`governing_packs` per-invariant attribution — fixed 2026-09-15.**
  Used to be pack IDs only (`Vec<String>`, which packs are active, not
  which one demanded which proved contract). `Certificate::
  governing_invariants` (`Vec<GoverningInvariant { fn_name, pack_id }>`,
  `hi_plugin::governing_invariants`) is real now: computed post-hoc
  from the final published `Program` against every active pack's own
  declared `fn` invariants, so it answers "which pack governed this
  artifact" correctly regardless of whether a `validate` block was
  actually injected by the pack or independently authored by the model
  to match its demanded signature. Wired into `handle_publish`
  alongside `governing_packs`/`nfr_commitments`. The existing test this
  gap was named from (`hi_api.rs::fintech_app_under_the_banking_pack_
  publishes_...`) now asserts the real per-fn attribution, both from a
  direct call and from the certificate a real `/api/publish` call
  writes to disk.

## Recommendation

~~Start with Phase 3 item 1 (the adversarial demo)~~ — done, along with
every other item in Phases 1-4, Phase 5 (below), and every disclosed
gap surfaced along the way (the `find_cycles`/windowing scaling bugs,
the CLI enforcement surface, the `let x: unit` codegen bug,
`governing_packs` per-invariant attribution, Phase 4's drift
detection/`hint_cache` integration/certificate attachment, the
`hint_cache`↔`generate_program` graph-shaped-prompt wiring, live
isolation ops-log capture, and the pack-signing UI's local half) — all
fixed, not just found, each with its own before/after measurement or
passing test, not asserted.

**2026-09-15, later the same day — the two items this doc previously
called "not engineering work" turned out to have real, buildable
halves after all**, and both are done:

1. **Phase 5's failure class was chosen and built, real minimal
   version** (see its own section above) — the artifact-boundary gap
   (`docs/research/2026-09-generated-code-guarantee-evaluation.md`),
   idealized as `rfcs/0017-security-guarantee-manifest.md`, built as
   `guarantee_manifest.rs` + `nirdosha build`'s bundle emission/hard-
   enforcement + `check-guarantees`/`verify-binary` — 3 of the RFC's 6
   `guarantee_check` items, the bundle, and the binary-side policy
   check; the other 3 items and runtime enforcement are real, disclosed
   follow-up, not this session's claim.
2. **Pack signing's UI had an unblocked half**: exposing the *already-
   real* `signer_identity` DB column through `/api/packs` needed no
   registry-governance decision at all — only the live Fulcio/Rekor
   trust tier was ever blocked on that. Wired, tested.

**Borrow liveness (lexical → true NLL) was considered and deliberately
left alone** — a real CFG-based rewrite, not a bounded fix, and this
project's own standing position (no real generated code has hit the
gap yet) hasn't changed. The only genuinely remaining, correctly-
unscheduled item is the live Fulcio/Rekor registry-governance question
itself — not UI work, an organizational one.

Everything that could be closed by writing and testing code, this doc
recommends closing, has been closed.
