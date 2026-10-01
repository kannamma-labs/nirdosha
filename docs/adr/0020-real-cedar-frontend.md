# 0020: Real Cedar frontend — real `cedar-policy` parsing/evaluation, AST-based lowerable-subset translation

Date: 2026-09-21
Status: accepted

## Context

`nirdosha-guard-core/src/cedar.rs` already implemented the `PolicyFrontend`
trait and asserted the "not lowerable fails closed" contract in a test,
but it was a fixture pretending to be Cedar, not an integration:
`CedarFrontend::evaluate` never read `principal_condition`/
`action_condition`/`resource_condition` on `CedarPolicy` at all — any
policy in the set was treated as equally "matching" regardless of what
those fields said — and `lower_when_clause` was a hand-rolled
`split_once("==")`/`split_once("in")` string matcher keyed on marker
substrings (`"unsupported_fn"`, `"eval_external"`), not real Cedar syntax
or semantics.

## Decision

**Real `cedar-policy` crate (v4, exact AWS/Cedar-project crate), pure
Rust, no C-toolchain dependency** — a real risk this session had already
learned to weigh carefully (`docs/adr/0016`'s `rdkafka`-avoidance
reasoning, and two mid-session `cargo clean` runs after the root
filesystem filled from accumulated build-profile variants). Confirmed to
build and link cleanly before committing to the dependency.

**`CedarFrontend::parse(&[&str])` parses real Cedar policy text**
(`permit(principal, action, resource) when {...};`) into a real
`cedar_policy::PolicySet` — genuine syntax errors surface as
`CedarLoweringError::SyntaxError`, not silently accepted.

**Principal/action/resource matching is genuine Cedar RBAC semantics.**
`build_request` maps an `EvaluationContext` to a real `cedar_policy::Request`:
`principal = User::"<subject.id>"`, `action = Action::"<wire action>"`
(`Action::wire_str()`, new — the inverse of the existing `parse_wire`,
needed because `Debug`'s PascalCase isn't the wire vocabulary any policy
author would write), `resource = Resource::"<entity>"`, and a `Context`
record carrying `tenant`/`purpose`/`destination`/`clearance`/`dataset`/
`policy_version`/`roles` (as a Cedar set) — real fields a `when` clause can
reference, not dead struct fields nothing reads. `Authorizer::is_authorized`
runs the real decision.

**Lowering walks Cedar's own JSON expression AST** (`Policy::to_json()`'s
`conditions[].body`), not a string splitter. The exact JSON shape
(`{"==": {"left": ..., "right": ...}}`, `{".": {"left": ..., "attr":
"name"}}` for attribute access, `{"Var": "resource"}` for the record root,
`{"Value": ...}` for literals, `{"Set": [...]}` for set literals) was
confirmed by direct compilation against real Cedar output before writing
the walker (the same "verify the exact shape, don't guess" discipline
`docs/adr/0019`'s `approval_chain!` fix used after a real
`proc_macro`/`proc_macro2` stringification mismatch). Supports `==`, `in`
(against a literal set only — entity-hierarchy `in` isn't attempted), and
`&&` — the same lowerable subset the pre-existing stub claimed, now backed
by a real parser instead of one that happened to accept the shapes its own
two tests used and nothing else.

**The lowerability gate still runs first and still fails the whole
evaluation closed**, per this crate's own existing (correct, kept)
contract: every policy's `when`/`unless` conditions are checked up front
(mirroring the old stub's "check everything, not just what matched"
posture), and any non-lowerable condition — including a real Cedar
`unless` clause, which this module doesn't attempt to lower (documented as
"outside the lowerable subset," not silently dropped) — denies with
`policy.not_lowerable` before the real `Authorizer` ever runs. This is
still correct for the reason the module doc explains: RFC 0023's guard
kernel needs a matched policy's condition expressible as a `FilterExpr` to
push down to a driver; a condition Cedar could evaluate today but can't
lower gives the kernel no way to guarantee the same restriction holds at
the data layer.

**`CedarPolicy`/`CedarEffect` are gone**, replaced by real policy text
strings — the honest reflection of the fact that `CedarPolicy`'s
`principal_condition`/`action_condition`/`resource_condition: Option<String>`
fields were never anything but unread struct fields; a real Cedar
`PolicySet` carries this information natively and correctly, and inventing
a parallel struct to hold pieces of it would just be a second place for
the same drift the original bug came from. `crates/nirdosha-rt/tests/e2e_data_guard.rs`'s
Cedar scenario was updated to real policy text matching its own context's
real subject/entity/tenant, replacing a `when_clause` string
(`"status == \"active\""`) that referenced a field nothing in the old
stub's evaluation ever actually checked against real context data.

## Consequences

**What this makes possible.** `cargo test -p nirdosha-guard-core cedar`
proves: a real non-lowerable Cedar expression (`resource.tags.contains(...)`,
a genuine method call, not a hand-picked marker substring) fails closed; a
policy scoped to a principal/resource pair that doesn't match the request
is genuinely not the one that allows (two policies, only the matching one
lets a real `Authorizer` decide `Allow`) — proving principal/resource
*matching* drives the decision, not previously-dead fields; a real
`context.tenant`/`context.purpose` condition genuinely evaluates against
real `EvaluationContext` data, denying on a real mismatch; and `.contains()`
on a set (real Cedar, genuinely evaluable by Cedar itself) is correctly
rejected as outside *this module's* lowerable subset even though Cedar
could evaluate it — the distinction the whole gate exists to enforce.

**What this does *not* make possible, stated so it's never misread later.**
No entity hierarchy or group-membership data is wired in
(`Entities::empty()`) — `principal in Group::"X"` scope constraints always
evaluate against an empty entity store and never match; role checks must
go through a `when`-clause condition against the real `context.roles` set
this frontend does populate instead. Negated conditions (`unless`) are not
lowered — a chain reachability item for a future phase if Cedar-native
negation is ever needed, not attempted here. `CedarFrontend` remains a
second, selectable `PolicyFrontend` implementation alongside the
registry-driven `evaluator::evaluate` — this phase doesn't change which
one any existing guard client uses by default.
