# 0019: `approval_chain!` reachability — const-compatible registration, a real quorum runtime, wired through `guarded_apply`'s escalation path

Date: 2026-09-21
Status: accepted

## Context

`Decision::Escalate { to: EscalateTarget::Approval { chain } }` has been
handled by `evaluator::evaluate` since early in this plan, and
`break_glass.rs`/`delegation.rs` (dual-approval time-boxed grants, scoped
delegation credentials) are genuinely `[DONE]` per the RFC 0026 checklist
— but neither of those is what `escalate to approval(chain sar_release)`
names. Investigating what *is* behind a named chain found two real,
compounding gaps:

1. `approval_chain! { chain name { quorum(N, of = [...]); timeout(deny); }
   }` only ever registered the block's raw source text into the `CATALOG`
   slice (useful for `cargo nirdosha verify`'s dump, not for anything that
   needs the chain's actual quorum/role requirements). The
   `ApprovalChainRecord` type existed and `APPROVAL_CHAINS` was a real
   `linkme::distributed_slice` declared for exactly this purpose, but
   nothing populated it.
2. That wasn't fixable by just calling `APPROVAL_CHAINS.push`-equivalent
   codegen from the macro: `ApprovalChainRecord { name: String, quorum: u8,
   approvers: Vec<String> }` isn't const-constructible — a `static` item's
   value must be a `const` expression, and owned `String`/`Vec` values
   can't be built at compile time. `APPROVAL_CHAINS` could never have been
   populated by any macro as the type was declared, independent of
   whether one tried. The same construction problem was found to affect
   `RoleRecord`/`PortRecord`/`ModelRecord`/`WorkflowRecord`/`DatasetRecord`
   too — none of which anything in `nirdosha-guard-macros` populates
   either, for the identical reason (see Consequences).

## Decision

**`ApprovalChainRegistration`, new in `nirdosha-guard-registry`** — the
const-constructible counterpart to `ApprovalChainRecord`
(`name: &'static str`, `approvers: &'static [&'static str]`), the same
`*Registration` (const, macro-facing) vs. `*Record` (owned, runtime-facing)
split `PolicyRegistration`/`PolicyRecord` already establishes for policies.
`APPROVAL_CHAINS`'s item type changed to it; `ApprovalChainRegistration::to_record()`
converts to the owned form `RegistryDump.approval_chains` carries.

**`approval_chain!` now real-parses `quorum(N, of = [Role, ...])`** out of
the macro input's own `.to_string()` rendering, emitting both the existing
`CatalogRegistration` (unchanged) and a real `ApprovalChainRegistration`
into `APPROVAL_CHAINS`. Whitespace-insensitive by construction (strips all
whitespace before searching) rather than hardcoding an assumed spacing
convention — developing this function found, by direct compilation, that
`proc_macro::TokenStream::to_string()` (what the macro actually receives)
and `proc_macro2::TokenStream::to_string()` (used to prototype the parser
in isolation) do *not* format identically, which the whitespace-stripping
approach sidesteps entirely rather than depending on either's exact
behavior. A chain with no parseable `quorum(...)` (or an empty `of =
[...]`) registers the raw catalog entry only, same as before this phase —
an honest gap, not a silent zero-quorum record that would trivially
"approve" with no real approvals.

**`nirdosha_guard_core::approval_chain::ApprovalChainRuntime`, new module**
— the actual quorum state machine: `open`/`approve`/`check_timeout`/
`status`. Every transition is explicit and total: there is no path from
`Pending` to `Approved` except a real quorum of *distinct* approvers whose
role is in the chain's `of = [...]` list (duplicate-approver and
ineligible-role attempts are rejected, not silently ignored or silently
counted), and no path out of `Pending` past the deadline except
`DeniedTimeout` — I3/`timeout(deny)`'s own non-negotiable default, honored
by construction rather than by a caller remembering to check it. A late
`approve()` call past the deadline resolves the escalation to
`DeniedTimeout` itself rather than silently no-op'ing, so an escalation
can't be left ambiguously "maybe pending forever" if nothing ever calls
`check_timeout` explicitly.

**`GuardClient` wiring** (`nirdosha-guard-mic`): `with_approval_chains`
registers real chain definitions (typically from a `RegistryDump.approval_chains`,
now genuinely populated). `guarded_apply`'s `Decision::Escalate` arm opens
a real pending escalation (keyed by `trace_id`) instead of returning a
debug-formatted target string with nothing behind it — best-effort
idempotent (`AlreadyOpen` on a replay is fine; `UnknownChain`/
`ChainHasNoEligibleRoles` surface as a real `Rejected::Failed`, not a
silent pass-through). `approve_escalation`/`escalation_status` are thin,
public passthroughs to the runtime.

**`commit_after_approval` is a separate method, not a retry with the same
`trace_id`.** `guarded_apply`'s idempotency gate (`insert_if_new`) treats a
repeated `trace_id` as "this exact call already happened" and short-
circuits to `Duplicate` — correct for true replay, but incompatible with
"the same logical request, now approved," which is a materially different
operation. `commit_after_approval` takes the original escalation's
`trace_id` (to check quorum) plus a genuinely new `new_trace_id` (for this
write's own idempotency) — mirroring this file's existing two-phase
pattern for `Migrate`'s dry-run-then-apply gate. It re-evaluates the
request fresh rather than trusting the original escalated decision's
stale residual filter/caps, and requires the *current* decision to still
be `Escalate` for the same target — a policy change landing between the
original attempt and the approval doesn't silently ride through on an old
decision.

## Consequences

**What this makes possible.** `cargo test -p nirdosha-rt --test
rtm_policy_corpus` proves, against the real `00_core.nir` corpus (not a
hand-built fixture standing in for it): all 7 real `approval_chain!`
blocks register structured records with the correct real quorum/approvers
(`sar_release`: quorum 2, `[ComplianceLead]`; `policy_release`: quorum 2,
`[PolicyEngineer, ComplianceLead]`), and `ApprovalChainRuntime` built from
those real records lets `sar_release` genuinely reach quorum with two
distinct `ComplianceLead` approvals, and genuinely denies a second,
independent escalation that times out with only one. `cargo test -p
nirdosha-guard-mic` proves the full `GuardClient` path: an escalating
write is rejected immediately but opens a real, queryable pending
escalation; a write attempted before quorum (even via
`commit_after_approval`) is refused; and only after two real approvals
does the exact same write actually commit to the store.

**What this does *not* make possible, stated so it's never misread later.**
`RoleRecord`/`PortRecord`/`ModelRecord`/`WorkflowRecord`/`DatasetRecord`
share the identical two-part gap this ADR diagnosed for approval chains
(unpopulated, and not const-constructible as declared) — out of scope for
this phase, which fixes only the one slice `approval_chain!` reachability
actually needed, but now clearly identified rather than left to be
rediscovered as a surprise later. No `guard_policy!`/`approval_chain!`
clause in the corpus declares an explicit escalation deadline
(`timeout(deny)` names the outcome, not a duration) — `guarded_apply`
opens escalations with a real, operator-settable 24h default
(`default_escalation_ttl_ms`), not one derived from any policy text.
`break_glass.rs`'s separate dual-approval grant ledger and `delegation.rs`'s
scoped-credential store are unchanged by this phase — they solve different
problems (time-boxed emergency access; non-transferable delegated
credentials) than a named, quorum-gated approval chain.
