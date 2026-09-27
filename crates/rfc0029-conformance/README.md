# rfc0029-conformance

This dependency-light crate is the experimental executable boundary for RFC
0029 Revision 1. It is intentionally disconnected from every live policy and
effect gateway.

Implemented:

- strict versioned top-level IR with closed node, edge and catalog enums;
- unknown-field, unknown-enum and schema-downgrade rejection;
- node uniqueness, endpoint existence and DAG validation;
- independent RFC 8785 encoding and SHA-256 verification of all 51 vectors;
- computed model-influence folding for all 39 graph-bearing vectors;
- combined admission and capability evaluation for all asserted graph cases;
- structural-only consequential-path rejection, authority cutoff,
  `ObserveOnly` freshness and execution reversibility mapping;
- a separate fact-provenance evaluator (producer/deployment identity,
  freshness, revocation, authorized consumer) for an authority-asserted
  `Fact` — derived from a model's classification, per RFC 0029.a's closed
  edge table §2.2, which licenses `AssertFact` only from `AuthorityAssertion`
  — consumed by a later, separate `Decision`;
- edge source/target *kind* validation against RFC 0029.a §2.2's closed
  edge table (`validate_graph`, `tests/edge_kinds.rs` — one negative case
  per all 22 `EdgeKind` variants); and
- malformed graph, cycle and downgrade tests.

Not implemented yet:

- closed Rust types for every node attribute and profile parameter;
- dependency invalidation evaluator;
- normalized graph emission for every applicable fixture;
- integration with production policy admission or effect gateways.

The review evaluator now uses closed contract, predicate-definition and
predicate-evaluation types. It validates definition versions, evaluator
authority, input/evidence commitments, validity and revocation, and computes
all nine review fixtures with deterministic false-before-stale-before-missing
precedence. The admission evaluator combines that result with computed model
influence, mandatory denies, fact-provenance results (revoked-before-stale-
before-wrong-model-before-wrong-deployment-before-unauthorized-consumer-
before-missing precedence, seven fixtures `L8`–`L14`), explicit per-effect
profile limits and exact capability/effect binding. Invalidation and
complete normalization remain pending.

Diagnostics are closed enums, not free-form strings: `AdmissionDiagnostic`
(13 variants) and `ReviewFinding` (8 variants) — a call site can no longer
fabricate an ad-hoc diagnostic string, and `tests/diagnostics_coverage.rs`
asserts every variant is actually reached by at least one of the 51
canonical fixtures (one documented exception, `CapabilityEffectMismatch`,
which the generator itself makes unreachable by any fixture and which is
covered by a dedicated Rust-level test instead — see that file's comments).
`tests/version_compatibility.rs` makes the schema's version/compatibility
and attribute/parameter extension-point policy executable rather than only
described in prose.

## Pilot: assertion lifecycle (temporal revalidation)

`src/pilot/assertion_lifecycle.rs` is the stateful harness `L8`–`L14`
couldn't be: a `Registry` of authority assertions consumed by decisions
across real, separate calls, proving the property no static graph fixture
can — a consumer must re-check an assertion's *current* state (authenticity,
revocation, expiry) every time, never reuse an earlier call's `Consumed`
answer. `tests/temporal_revalidation.rs` runs the full t0 (assert) → t1
(first decision consumes) → t2 (revoke, or just let time pass the validity
window) → t3 (second decision's consumption is denied) sequence, plus
tamper-detection and boundary cases. Like `funds_reserve`, this has no
store, network, or effect-gateway dependency — and, unlike a real
deployment, no cross-process/restart persistence claim at all; state lives
only in one `Registry` for one process's lifetime.

## Pilot: `funds.reserve`

`src/pilot/` is the narrow executable pilot required by
`rfcs/0029-phase0-readiness-and-domain-matrix.md` §4 — one authorization
policy, one atomic balance invariant, one capability-consuming in-process
gateway over an in-memory transactional fixture, signed account facts with
freshness and revocation, and negative-path tests for deny, stale fact,
insufficient funds, replay, resource-version mismatch, and simulated commit
failure. It has no store, network, or effect-gateway dependency and
authorizes nothing outside this process — it is a falsification experiment
against the versioned candidate IR, not a production or freeze claim.

`src/pilot/report.rs` compiles `AdmissionReportV1` per §5 of the same
document ("Compiled admission report"): every field is computed from the
actual pre/post ledger state and receipt of one run, not asserted — a run
that mutates state without a `Reserved` outcome, or claims `Reserved`
without the exact declared mutation, fails the report's own
`atomic-commit-integrity` check rather than the test's assertions alone.

Not implemented yet:

- a CLI or `[[bin]]` entry point (a `cargo test`-emitted report is treated
  as sufficient for now; add one only if it earns its keep);
- any effect beyond `funds.reserve` (`funds.release` and external
  settlement-rail submission are named exclusions in every emitted report,
  not silently covered paths);
- aggregate/daily-limit consistency.

Run:

```sh
cargo test -p rfc0029-conformance
```
