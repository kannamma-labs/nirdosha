# Rough edges discovered while designing module composition

This file collects the concrete gaps that appear when you try to assemble a
real banking platform from pluggable modules. They are ordered from
"mechanical tooling" to "deep semantic".

## 1. No composer tool exists

The generator today takes one `screens.toml` + one `menus.toml` in a single
project directory. There is no `cargo nirdosha compose` that:

- reads a root manifest,
- loads N module registers,
- merges them,
- resolves conflicts,
- emits a composed project.

This is purely new tooling; no language/runtime change needed.

## 2. Screen / entity / route / role namespace collisions

When two modules are independent, collisions are inevitable:

- Two modules might both want screen id `2.1`.
- Two modules might both declare an entity named `Account`.
- Two modules might both use route `/reports`.
- Two modules might both define a role `Manager` with different meanings.

A composer must either:
- namespace everything automatically (`accounts.2.1`, `loans.2.1`), or
- require module-scoped IDs in the source and validate globally.

The current register has a `module` field on screens but no enforced
namespace. This is the first rule the composer must add.

## 3. Shared entities require a single source of truth

`Customer`, `AuditEntry`, and `Holiday` are used by multiple modules. If
`accounts` declares `Customer { customer_id, name }` and `loans` declares
`Customer { customer_id, kyc_status }`, the composer must detect the
mismatch and refuse, not silently merge fields.

The root manifest's `[shared_entities]` section is a proposal for how to
declare ownership. The enforcement rule is missing.

## 4. Cross-module references are implicit

A loan screen wants to show the linked customer account. A card screen
wants to show the linked account balance. A transfer wants to check that
the debit account belongs to the logged-in customer.

The register has no way to say:

```toml
[[screen]]
data_binding = { entities = ["LoanAccount", "Account"], relation = { from = "LoanAccount.account_number", to = "Account.account_number" } }
```

Without this, cross-module workspace panels and validation rules must be
hand-written.

## 5. Approval chains have unclear scope

`transfers` declares `large_transfer`. `accounts` declares `large_debit`.
`loans` declares `loan_disbursement`. If `fx` later wants a conversion
above a threshold to also use `large_transfer`, is that chain global or
module-local?

The simplest answer is: all chains are global after composition. The
danger is that a malicious/incompetent module can add a chain that
shadows another. The composer must reject duplicate chain names.

## 6. Landing pages are per-module, not global

`core/menus.toml` says `Customer = "/accounts"`. If `cards` is enabled,
a bank might want `Customer = "/cards"`. The root manifest needs a global
`[landing]` that overrides module landings.

## 7. Menu group ordering is per-module

Each module declares its own `[[group]]` with an `order`. When composed,
two modules can claim the same `order` value. The composer needs a global
ordering policy or a merge rule (e.g., core groups first, then product
groups by module weight).

## 8. Certified primitives are not declared in the register

Blocked screens cite `blocked_by = ["primitive:transfer_execute"]`, but
that is just a string. There is no:

```toml
[[primitive]]
name = "transfer_execute"
path = "src/primitives/transfers.nir"
contracts = [
    { name = "sufficient_funds", expr = "debit_balance >= amount_cents" },
    { name = "idempotency", expr = "transfer_requests[transfer_id].status != 'completed'" },
]
```

Without a primitive declaration, the generator cannot:
- collect primitives from all modules,
- emit a `primitives.nir` module,
- run a coverage gate that refuses inline money math.

## 9. Country-specific parameters are runtime, not compile-time

`banking.toml` has `[country.IN]` with limits. These are intended for
primitives to read at runtime. But some parameters should shape the UI
(e.g., showing "Daily limit: ₹50,000" on a screen). The register has no
way to bind a country parameter to a screen label.

## 10. Jurisdiction-aware role lists

A real bank has country-specific roles: `RelationshipManager_IN`,
`ComplianceOfficer_DE`. The current role model is a flat string list.
A composed multi-country platform needs role namespaces or
`(jurisdiction, role)` pairs, otherwise a single role accidentally spans
countries.

## 11. Logging domain per module

The root manifest declares a single logging domain. If `insurance` is later
added as a module, its logging domain may differ. The composer needs to
allow per-module logging contracts or refuse incompatible domains.

## 12. External plugins are not resolved

`compliance` marks sanction screening as `blocked_by =
["plugin:sanction_screening_api"]`. There is no dependency resolver that:

- checks that the plugin is installed/signed,
- links it into the generated crate,
- exposes its functions to primitives.

## 13. No "primitive completeness" gate

Even if primitives are declared, the current `cargo nirdosha` verifier
does not yet enforce that generated code calls certified primitives for
money movement. That is RFC 0016 §6, the largest remaining engineering
item.

## 14. Test composition

`examples/helpdesk/tests/smoke.rs` builds a router directly. For a
composed banking app, tests must know which modules are enabled. The
composer should generate a `tests/smoke.rs` that imports the right
modules, or at least a `TestRouter` helper that mirrors the composed
`serve.nir` mount order.

## 15. Feature flags vs. module selection

A composed Cargo crate could use Cargo features to exclude modules at
compile time. But the module selection is currently a TOML-level decision,
not a Cargo feature. Aligning the two (so `cargo build --no-default-
features --features accounts` produces a smaller binary) is future work.
