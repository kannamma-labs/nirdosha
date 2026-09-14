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
| 4 — Close the APM loop | Feed real runtime data back into the proof/generation loop | 🟡 Item 3 done; items 1-2 still design-only (see below) |
| 5 — Domain translation stretch | Pick a new failure class, idealize → build → wire in | ◻️ Not started, deliberately |

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

## Phase 4 — close the APM loop (partially real)

What's already true: the isolation checker's escalation path
(`isolation_check::escalate`, reusing `nfr.rs`'s `NIRDOSHA_OBSERVABILITY_URL`
client) **is** a real APM-kernel closed loop — a runtime-observed fact
now flows back out of the compiled binary through the same channel NFR
violations use.

What's still just a design, not code:

1. **Drift detection.** Detect when a compile-time `nfr(...)` assumption
   (e.g. an implicitly assumed max concurrency) diverges from what the
   APM kernel actually observes in production, and trigger
   re-verification instead of just firing an alert.
2. **Feed real incidents into `hint_cache`.** A production runtime
   violation (an isolation anomaly, a crossed NFR threshold) should be
   able to become a taught lesson for `hi_llm`'s self-repair loop the
   same way a `:generate`-time failure already does
   (`hint_cache::HintCache`) — right now the two systems don't talk to
   each other at all.

Now real, not just a design (2026-09-15):

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
   place or auto-update a standing claim. There is also still no
   mechanism that *produces* an ops-log from a live running process —
   `--isolation-log` consumes a log; nothing yet captures one from a
   real `.nir` binary's own in-memory checker state. That capture half
   is real, separate follow-up work, disclosed in `cmd_check_isolation`'s
   own doc comment (`main.rs`), not implied to already exist.

## Phase 5 — domain translation stretch (not started)

Pick one more failure class nobody solves well for LLM-generated code,
design the idealized solution, build a real minimal version, then wire
it into the existing graph/pack/certificate machinery the same way a
domain pack talks to the coverage gate today. Explicitly premature
before Phase 3 lands — no candidate failure class has been chosen.

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
- **Pack signing has no UI.** `agent-skills/nirdosha/hi_ux_redesign_options.md`'s
  "trust indicator" mockup was never wired to `hi_plugin::
  pack_signer_identity`. Live Fulcio/Rekor integration remains
  genuinely blocked on the registry-governance question (RFC 0016's own
  position, unchanged).
- **Borrow liveness is lexical, not true NLL.** A real, disclosed
  precision cost (`ownership.rs`'s "Borrow liveness (v1, lexical)"
  section) — rejects some programs a full last-use liveness analysis
  would accept. Worth revisiting only if it bites real generated code in
  practice, not worth pre-emptively building out.
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
items 2 and 3, and every disclosed gap it surfaced along the way
(the `find_cycles`/windowing scaling bugs, the CLI enforcement surface,
the `let x: unit` codegen bug) — all fixed, not just found, each with
its own before/after measurement or passing test, not asserted.
Phase 4 item 3 (certificate attachment) and `governing_packs`
per-invariant attribution are both real now too. Every item in this doc
that was reachable without either new external resources (an Imandra
license, raw-logit model access) or a fresh, deliberately-deferred
design decision (Phase 5's failure class) is done as of 2026-09-15.

What's left, in order of real remaining value:

1. **Phase 4 items 1-2** (drift detection triggering re-verification;
   feeding real incidents into `hint_cache`) — both still genuinely
   design-only, and now the largest remaining reachable items in this
   doc (drift detection needs a real place to compare a compile-time
   `nfr(...)` assumption against an observed APM value and decide what
   "diverged enough to re-verify" means; `hint_cache` integration needs
   a real schema for a runtime-observed lesson, not just a
   `:generate`-time one). Worth scoping properly before starting, not
   sized here.
2. **Pack signing's UI** stays genuinely blocked on the registry-
   governance question (RFC 0016's own unchanged position) — not
   picked up until that's resolved, same as before.
3. **Phase 5** (a new failure class, idealized → built → wired in) is
   no longer premature on Phase 3's own account (Phase 3 is done) — but
   still needs a real candidate failure class chosen first, a decision
   this doc has deliberately left open rather than picked under time
   pressure.
