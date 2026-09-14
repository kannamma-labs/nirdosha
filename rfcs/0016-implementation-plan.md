# RFC 0016 implementation plan

> Companion to [`0016-domain-packs-and-whose-job-domain-correctness-is.md`](./0016-domain-packs-and-whose-job-domain-correctness-is.md).
> This is the build order, not a second design discussion — every item
> below implements something the RFC already decided. Phases are
> dependency-ordered: each one is shippable on its own and nothing
> later in the list is needed to close the demonstrated PROVED 0/0
> seam. Blocked items at the bottom are gated behind governance or
> explicit scope sign-off, exactly as the RFC says, and must not be
> pulled forward.
>
> **Branch:** `feat/rfc-0016-domain-plugins` off `feat/hi-dogfoods-mcp`
> tip. Small commits per phase; every phase lands green.

## Phase 0 — proof-engine hardening (both later gates inherit this)

The coverage gate and the plugin both read `ContractCheckResult`; the
RFC's fail-closed/non-vacuous/fuel semantics must live there first.

1. **Merge `fix/contract-check-vacuous-precondition` (`0ccc837`)** —
   adds `ContractCheckResult::VacuousPrecondition` (pre_logic
   satisfiability checked once, after preconditions are asserted,
   before the body walk). This is the RFC's non-vacuity requirement,
   already half-built on that branch.
2. **`ContractCheckResult::EngineLimit`** — a new variant carrying the
   failed obligation, classified as *engine limit*, not violation:
   Z3 `unknown` or resource exhaustion, never a wall-clock timeout.
3. **Deterministic solver fuel** — Z3 `rlimit` caps (deterministic
   conflict/step counters) set in `contract_check.rs`; exhaustion
   reports `EngineLimit` deterministically — same source, same fuel,
   same verdict on any machine. Fuel constants are toolchain constants.
4. Tests: contradictory pre_logic → `VacuousPrecondition` (merge's own
   tests); fuel exhaustion → `EngineLimit` classification; a normal
   proof still `Proved` under the default fuel.

*Estimate: 1–2 days.*

## Phase 1 — solution 4, the coverage gate (closes the demonstrated seam)

1. **`hi_llm::contract_coverage_check(source, units)`** — parse the
   draft, match every confirmed unit's demanded validate-contract
   attributes against the `ast::ValidateDecl`s actually present
   (`fn.validates`), run `contract_check::run_program_validates` on
   what's there. Failure classes: dropped / violated / vacuous / engine
   limit — each attributed to the unit (and later, the pack) that
   demanded it.
2. **Wire into `generate_program`** after `typecheck_and_build_check`,
   inside the existing repair conversation: failures get a
   machine-readable block (2026-09-11 diagnostics format) plus three
   new `self_repair_hint` arms — dropped-contract ("you dropped it;
   write it, and it must prove, not merely parse"), vacuous ("the
   preconditions are unsatisfiable; fix the code or the contract"),
   engine-limit ("simplify the arithmetic into provable form —
   linearize the fee, split the tier").
3. **`ENGINE_LIMIT` does not consume the violation budget** — after one
   failed simplification the run escalates to the operator with the
   proof obligation attached (the RFC's VIOLATED/ENGINE_LIMIT split).
4. **Publish re-checks coverage** on the file actually being published
   — hand-edits between generate and publish get caught.
5. Tests in the existing `self_repair_hint`/`units_prompt` pattern:
   demanded+proved passes; demanded+dropped fails with the hint;
   vacuous fails with its hint; engine-limit escalates without
   consuming the budget.

*Estimate: 2–3 days.*

## Phase 2 — 5a plugin mechanics + banking pack v0 (zero crypto)

1. **Schema:** `hi_graph::open` gains additive `plugin_origin` +
   `non_waivable` columns; `waive_node` / `unwaive_node` /
   `delete_node` refuse for non-waivable rows, pointing at the pack ID.
2. **`hi_graph::load_domain_pack`** — bounded parse of a small 5a
   manifest (invariant nodes carrying contract templates, `RELATES_TO`
   edges, mandatory primitives list), sha256 of the canonical bytes
   pinned at install (**TOFU — named as such**), re-hashed on every
   load, nodes written via the existing `add_candidate` →
   `confirm_node` → `attach_attribute` → `lock_units_after_sync` path.
   Packs live in `.nir/plugins/<name>/`.
3. **Injected mode:** the pack's contract templates are inserted into
   the draft's AST at generate time — the model's code is checked
   *against* the sealed law; it never authors it.
4. **`units_prompt` renders plugin invariants as data** — a delimited,
   clearly non-negotiable section (the T1 prompt-construction
   invariant), never rendered as directives.
5. **CLI:** `nirdosha plugin install [--dry-run] / list / revoke` —
   operator-level; `--dry-run` performs the full load and runs the
   pack's own contracts against a stub program (the RFC's
   fail-closed-cuts-both-ways guard).
6. **Banking pack v0 — the domain law itself** (needs the domain
   expert's sign-off, see below): money is whole cents; balances are
   non-negative; debits equal credits (conservation); no net money
   creation. Contracts stay in the proven Z3-provable subset —
   linear arithmetic, no division (the v1 lesson).
7. **Acceptance test:** the fintech v4 run *under the pack* — generate
   in one shot, `nirdosha verify` reports `PROVED n/n` with the
   governing pack named. Extends the `paste_prompt_recipes.rs` pattern:
   a test that loads the pack and proves its contracts at test time.

*Estimate: 3–5 days + pack authoring.*

## Phase 3 — solution 6, scoped v1 (certified primitives prelude)

1. Primitives as `pack.invariants.nir`, parsed + typechecked at
   install (never executed), `ValidateDecl`s proven at install and
   re-proven at generate; `generate_program` prepends the prelude
   before typecheck.
2. **Mandatory call-site coverage** — every primitive the pack marks
   mandatory must have a real call site (cheap half of "uses the
   ledger correctly").
3. **Reserved namespace** — generated code cannot redeclare or shadow
   primitive names (cheap half of `primitive_exclusivity`); field
   ownership exclusivity is the expensive half and is staged after.
4. **Call-site precondition obligations, v1 scope:** provable from
   caller-local facts only. Cross-function summaries and loop
   invariants are the research-grade remainder — an explicit scope
   decision recorded in code comments, not a quiet promise.

*Estimate: 1–2+ weeks for the scoped version.*

## Phase 4 — certificate wiring: mandatory certificate, optional signature (issue #59)

The demonstrated seam this phase closes: `hi_api.rs::handle_publish` runs
typecheck → ownership → the Phase 1 coverage gate → `codegen::build` and
never calls `build_certificate` at all — a project can `:publish` a real
binary today with zero certificate produced. Same *shape* of gap Phase 1
closed for contracts (proved 0/0, nobody looked), one layer up. Scoped
to stay entirely inside what's already shippable — nothing here is
blocked on 5b's registry governance.

1. **`handle_publish` calls `build_certificate`.** Right after the
   existing coverage-gate check passes, build a `Certificate` from the
   published source and write it alongside the binary
   (`<out>.certificate.json`). No new CLI surface — `nirdosha certify`
   keeps working standalone for anyone who wants to re-derive one.
2. **Extend `Certificate` (`mcp_tools.rs`) additively** — this is
   "What certification emits" (the main RFC doc, above) actually
   implemented, not a new schema:
   - `governing_packs: Vec<PackAttribution>` — `{pack_id, invariants:
     Vec<String>}` for every 5a pack whose confirmed units fed this
     artifact's coverage-gate pass. Available today (Phase 2's packs
     already carry `pack_id`); does **not** wait on 5b signing — an
     unsigned pack is still real attribution, labeled as such (same
     "trust on first use" honesty 5a already uses for the pack pin
     itself).
   - `nfr_commitments: Vec<NfrCommitment>` — `{fn_name, latency_ms,
     error_rate_max, throughput_min_per_sec, concurrency_max}` (each
     field optional, mirroring `nfr(...)`'s own all-optional
     thresholds), sourced from the AST's `nfr` attributes at publish
     time. **`evidence_tier: "monitored"`, distinct from
     `verdict_summary`'s `"proved"`/`"unknown"`** — an NFR claim is
     APM-kernel-tracked at runtime (`rfcs/0007`), never Z3-proved, and
     must not be allowed to read as a compile-time proof inside the
     same certificate. This is the same discipline "Compliance
     profiles" already enforces for `external_conformance` — a
     runtime/external fact stays visibly a different kind of evidence
     from a proof, never blended into one undifferentiated "passed."
3. **Mandatory certificate, optional signature.** Certificate emission
   at publish is unconditional (deterministic, no key required — same
   cost class as the coverage gate it rides on). An actual **signature**
   stays gated behind an explicit opt-in (e.g.
   `NIRDOSHA_PUBLISH_SIGNING_KEY`, mirroring `certify --sign`'s existing
   `key.pk8` argument) for deployments with an operator key and a
   governance story — never on by default, and never something
   `generate_program`/the model can reach (unchanged from 5b's existing
   "AI never gets signing" line). This is the RFC's own "5b must not
   gate 5a/4" principle applied one layer up: don't let governance-
   blocked work (real signing) block something shippable (the
   certificate itself).
4. Tests: `handle_publish` produces a certificate file whenever publish
   succeeds; `governing_packs` names the banking-pack fixture from
   Phase 2's acceptance test; `nfr_commitments` round-trips a real
   `nfr(...)` fn and carries `evidence_tier: "monitored"`, never
   `"proved"`; publish still succeeds with `NIRDOSHA_PUBLISH_SIGNING_KEY`
   unset (no signature, certificate still written).

*Estimate: 2–3 days.*

## Blocked — post-approval / governance-gated, do not start

- **5b sealing** (Ed25519, JCS, certificates, transparency logs,
  registries) — blocked on "who runs the registry" (RFC 0016's own
  honest position). The only permitted groundwork is exactly what 5a
  builds — pack ID as hash of canonical bytes, verification pipeline
  shape — so 5b swaps signatures in without re-architecture.
- **Solution 7 compliance profiles** — needs Phase 3's machinery plus
  the wiring emitter; FAPI profile content authoring needs a
  regulation-literate author and external-verification decisions.
  Enforcement model per the RFC: the plugin gates the claim, the
  external suite grounds it.
- **Operator identity / dual-control** — hi has no operator-identity
  concept; break-glass ships with out-of-band dual control, recorded
  as such. Tracked dependency.
- **`Money` type, `attest_code` rename** — deferred decisions, unchanged.

## Sign-off needed before/at each phase

- **Phase 2:** the 5a manifest shape; the banking pack's actual
  invariants (the domain law — domain-expert sign-off); packs living
  in `.nir/plugins/`; CLI naming.
- **Phase 3:** the v1 scope bounds (caller-local obligations only).
- **Phase 4:** the `Certificate` schema additions (`governing_packs`/
  `nfr_commitments` field shapes); the signing-key env var name and
  default-off posture.
- **Blocked items:** registry governance (5b), profile content
  authorship (7), operator identity design.