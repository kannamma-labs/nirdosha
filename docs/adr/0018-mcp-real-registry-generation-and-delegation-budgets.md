# 0018: MCP copilot completion — real registry-derived tools, evaluate() wired to the real evaluator, non-guessable plan hashes, real I17 delegation budgets

Date: 2026-09-21
Status: accepted

## Context

`crates/nirdosha-guard-mcp` existed before this phase, but three real gaps
made it aspirational rather than load-bearing:

1. `GuardMcpServer::evaluate` never consulted any policy set. It ran a few
   agent-specific guard checks (subject/policy-version/destination/
   clearance) and then, regardless of what was registered, always
   returned a hardcoded `Decision::Deny { "no explicit agent policy was
   supplied" }`. An MCP client's `evaluate()` call could never actually be
   allowed by a real policy — the method's own name implied it evaluated
   something, but it didn't.
2. `submit_write`'s "evaluate-then-act with a matching plan hash" gate
   (I17) used `format!("plan-hash-{entity}")` as the hash — a predictable
   string any caller could reconstruct without ever calling `evaluate()`.
   Worse: the old `evaluate()` inserted that predictable hash into
   `session_evaluations` *unconditionally*, before even looking at the
   decision it was about to return — so a caller could call `evaluate()`
   once (getting a `Deny` back) and then successfully `submit_write` with
   the guessed hash and any `WritePlan` claiming `Decision::Allow`,
   entirely bypassing the evaluation the API's own name promises. This was
   found while wiring the real evaluator through this phase (`cargo test
   -p nirdosha-rt --test e2e_data_guard` had been passing *because of*
   this bug, not despite it — see Consequences).
3. `DelegationToken` carried no real TTL, call budget, or rate limit —
   `mint_token` hardcoded `expires_at: "2099-01-01T00:00:00Z"`, and
   nothing anywhere checked expiry, a call count, or a rate. I17's own
   declared defaults (`examples/rtm/roles-N-guard_policy.md`'s `70_mcp.nir`:
   `delegation { bind user + agent; ttl = 30m; max_tool_calls = 60; rate =
   20/min; }`) existed only as documentation text, unenforced by any code.

## Decision

**`PolicyRecord::to_candidate()`, new in `nirdosha-guard-registry`.** The
only existing conversion (`PolicyRegistration::to_candidate`) works from a
live `linkme`-linked macro registration, not from a plain `RegistryDump` a
caller loaded from disk or built by hand. `GuardMcpServer::from_registry`
needs the latter — nothing outside a live macro-linked process could
previously construct a real `PolicyCandidate` set from a dump at all.

**`evaluate()` now calls the real `evaluator::evaluate`** against
`GuardMcpServer.policies` (populated by `from_registry`, or explicitly via
`with_policies` for a server without a full dump) after the existing
agent-specific guard checks. The fix is the direct implication: this is
Plan Phase 14's actual core requirement — "MCP copilot completion" means a
copilot's decisions are real decisions, not a hardcoded stub.

**Plan-hash is a real SHA-256 digest of the evaluated `AccessPlan`**
(`access_plan_hash`, `pub` so a real caller can compute the exact matching
hash from the `AccessPlan` `evaluate()` returned), and `session_evaluations`
is only populated when the real decision was `Allow` — closing the bypass
described in Context point 2. `crates/nirdosha-rt/tests/e2e_data_guard.rs`'s
existing MCP scenario needed updating to match: it now registers a real
policy (reusing the test file's own `sample_policy()`, which already
grants the right subject/action/resource) so `evaluate()` genuinely
allows, and uses the real `access_plan_hash` instead of the old guessed
string.

**Real I17 delegation budgets**, read directly off `70_mcp.nir`'s declared
values (`DelegationDefaults::default()`: `ttl_ms = 30*60*1000`,
`max_tool_calls = 60`, `rate_per_min = 20`) rather than invented numbers.
`mint_token` now takes `now_ms` (matching every other time-sensitive
method in this workspace's guard code — `guarded_apply`/`guarded_read`
already take explicit `now_ms` for deterministic, testable time rather
than reading the wall clock internally) and `DelegationToken` carries
`issued_at_ms`/`ttl_ms`/`max_tool_calls`. `check_and_record_call` (shared
by `evaluate` and `submit_write` — every tool call spends budget, not just
writes) enforces expiry, `max_tool_calls`, and a real sliding-one-minute-
window rate limit, tracked server-side per `scope_hash` (usage has to live
with the granter, the same reason a real API-key rate limiter doesn't
trust a client-supplied counter).

**Real registry-driven tool generation**, replacing the old
`from_registry`'s "append a `query_<entity>` tool for every dataset,
unconditionally, on top of a hardcoded 5-tool base" with catalog-grounded
generation: `query_records_<entity>` only for entities with at least one
real `read` grant in the dump's policies (advertising a tool nothing could
ever authorize would itself be a real, misleading gap), and
`get_options_<workflow>` per `WorkflowRecord` — a workflow's declared
`states` genuinely are "allowed values for a categorical field," the exact
concept RFC 0025's illustrative `get_options(AlertStatus, CaseStatus, ...)`
names (those *are* `workflow!`-declared state machines in the RTM corpus).

**I17's "masked fields are ABSENT, not masked-in-place"** is now a real,
tested function (`fields_present_for_copilot`) rather than only a doc
comment — the MCP surface's own, stricter posture than the general read
path's `ReadOutcome` (which hands masks back for redaction-in-place).

## Consequences

**What this makes possible.** `cargo test -p nirdosha-guard-mcp` proves: an
empty policy set denies (via the real evaluator, not a stub that happened
to also produce Deny); a real registered policy is genuinely honored,
including the I17 default `row_cap = 50` when no matched policy set a
tighter one; a guessed/predictable plan hash is rejected; a token past its
real TTL is rejected; the 60-call budget and the 20/min sliding-window
rate limit are both enforced and independently testable (the rate limit
recovers after its window passes; the call budget does not); and
`from_registry` only generates tools for catalog entities/workflows that
are actually reachable, verified against a real, deliberately-mixed dump
(one dataset with a read grant, one without).

**What this does *not* make possible, stated so it's never misread later.**
`mcp_tools!` is still not a callable macro — the real defaults it declares
are honored by `DelegationDefaults`, but nothing yet parses a `mcp_tools!
{ ... }` block into a live registration the way `from_registry` consumes
one; the same class of grammar-mismatch gap `docs/adr/0014`-`0017`
document for `matcher!`/`model_artifact!`/`stream_port!`. `guard-verify`'s
V6 pass ("`mcp_tools!` tool list ↔ catalog entities") stays a documented
no-op — `from_registry`'s generation is now grounded in real catalog data
by construction, but wiring a separate verify pass to cross-check it
against `nirdosha-guard-verify`'s `RegistryView` would need that crate to
carry MCP tool data it doesn't have a field for yet, a distinct piece of
work this phase didn't scope in.
