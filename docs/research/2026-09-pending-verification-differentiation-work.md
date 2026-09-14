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
| 3 — Externalize evidence | Publish methodology, invite outside reproduction, an adversarial demo for the new capability | 🟡 Item 1 done (below); items 2-3 not started |
| 4 — Close the APM loop | Feed real runtime data back into the proof/generation loop | 🟡 Partially real (see below) |
| 5 — Domain translation stretch | Pick a new failure class, idealize → build → wire in | ◻️ Not started, deliberately |

## Phase 3 — externalize evidence (next up)

1. ✅ **Done (this session, 2026-09-14): adversarial demo for the
   db-isolation checker.** `examples/isolation_demo/` — the exact
   `killer_demo` corrupting transfer, wrapped in a real `transact { }`
   block, run through the real compiled binary with
   `NIRDOSHA_OBSERVABILITY_URL` pointed at a real local listener. Three
   real runs, all three escalating real anomalies over a real HTTP POST
   (274-436 escalations per run); see `examples/isolation_demo/
   RESULTS.md`. **It also surfaced a real, unplanned second finding,
   now itself a new follow-on item below**: the checker doesn't scale
   to `killer_demo`'s own 8×250 workload at all — it hangs, measured
   and root-caused to `find_cycles`'s DFS-based cycle enumeration
   blowing up on the dense conflict graph this exact race produces,
   well before that scale (cliff measured at ~24-28 total concurrent
   transacts on one contended resource, two independent sweeps agree).
   This demo runs at 8×3 instead, with the cliff itself documented as
   part of the result, not hidden.
2. **Publish the benchmark methodology gap.** `crates/bench/RESULTS.md`
   already admits it's not the full comparison the roadmap calls for —
   write and publish the actual methodology.
3. **Invite outside reproduction** of the four new Phase-2 capabilities
   via `SECURITY.md`'s existing open-invitation posture, for real, not
   just as a standing badge.

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
3. **Surface it in the certificate over time.** `mcp_tools::Certificate`
   already has `nfr_commitments` (declared, `evidence_tier: "monitored"`)
   as of this session's Phase 4 work. A natural sibling field,
   `isolation_violations` or similar, would let a certificate become a
   living, re-attestable claim instead of a point-in-time one — named as
   a real possibility in `isolation_check.rs`'s own module doc, not yet
   built.

## Phase 5 — domain translation stretch (not started)

Pick one more failure class nobody solves well for LLM-generated code,
design the idealized solution, build a real minimal version, then wire
it into the existing graph/pack/certificate machinery the same way a
domain pack talks to the coverage gate today. Explicitly premature
before Phase 3 lands — no candidate failure class has been chosen.

## Smaller, explicitly disclosed follow-on gaps (lower priority)

Named in the code/docs at the time each was built, not hidden — pick up
opportunistically, not urgently:

- **Isolation checker has no enforcement surface.** Detection-only today
  (an async escalation after the fact) — no CI gate, no way to run it
  over a saved log on demand. A `nirdosha check-isolation <log>` CLI
  would close this.
- **Isolation checker doesn't scale to real contention (new, found
  building the Phase 3 item 1 demo above).** `record_and_check` calls
  `check()` — a full `O(ops²)` graph rebuild, `check()`'s own doc
  comment already discloses this — on *every single* `db` operation,
  over the entire history of the process's run, never rotated. On the
  dense conflict graph many concurrent transacts racing one resource
  produce (exactly the case this checker exists for), `find_cycles`'s
  DFS-based simple-cycle enumeration measurably blows up past roughly
  24-28 total concurrent transacts on that resource — see
  `examples/isolation_demo/RESULTS.md`'s "Part 2" for the measured
  cliff (two independent sweeps agree) and the root-causing that ruled
  out the durability log and thread count first. The real fix is
  windowing (`clear_shared()` on a timer/op-count threshold, wired into
  the live `db.rs` path — `clear_shared()` already exists but nothing
  calls it today) or replacing `find_cycles` with an algorithm that
  doesn't degrade like this on dense graphs; neither attempted here.
  Worth prioritizing above the CLI-surface item above it, since a
  checker that hangs the program it's attached to under real contention
  is a correctness risk of its own, not just a missing convenience.
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
- **`let x: unit = <call>()` doesn't compile (new, found building the
  Phase 3 item 1 demo above)** — `layout::llvm_ty` maps `Ty::Unit` to
  LLVM `void`, and the generic let-with-type-annotation codegen path
  unconditionally allocas whatever `llvm_ty` returns; `alloca void` is
  illegal LLVM IR, so `clang` rejects the emitted module outright. Real
  today, at HEAD, independent of anything in this session's own working
  changes — confirmed by rebuilding `nirdosha` clean and trying to
  build `examples/killer_demo/race_probe.nir` itself, unmodified: it no
  longer compiles, because its own `let slept: unit = sleep_ms(2)`
  hits exactly this. (`examples/features/20_sandbox.nir` and
  `examples/features/54_compiled_workflow_escalation.nir` avoid it by
  calling `sleep_ms(...)` as a bare statement instead of binding it —
  that's the correct idiom and the workaround this session used for
  `examples/isolation_demo/`'s own `.nir` file, but `race_probe.nir`
  itself was not touched.) `killer_demo/RESULTS.md`'s own "every number
  below is from an actual run of the actual code" claim can no longer
  be reproduced as written until either that file drops the `let`
  binding or codegen special-cases `Ty::Unit` (skip the alloca/store
  entirely, matching how a bare-statement call is already handled).
- **`governing_packs` is pack IDs only**, no per-invariant attribution.
  Already named by an existing test comment
  (`hi_api.rs::fintech_app_under_the_banking_pack_publishes_...`) as
  separate "generation audit and governing-set snapshot" future work.

## Recommendation

~~Start with Phase 3 item 1 (the adversarial demo)~~ — done this
session; see `examples/isolation_demo/`. It paid off exactly as
expected (the checker really does catch the race, live, three real
runs) *and* surfaced a real correctness-adjacent scaling bug the
project didn't know it had. Given that finding, the next highest-value
move is no longer Phase 3 item 2/3 — it's the new "isolation checker
doesn't scale to real contention" item just added to the disclosed-gaps
list above: a checker that can hang a `transact`-using program under
genuine concurrent contention, with no opt-out today, is a real risk
sitting in already-shipped Phase 2 work, not a documentation gap.
Publishing the benchmark methodology (item 2) and inviting outside
reproduction (item 3) are still real, still next after that.
