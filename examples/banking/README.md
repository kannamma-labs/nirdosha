# Retail Banking Platform — Module Composition Design

This example explores how a real banking product could be assembled from
pluggable domain modules: accounts, transfers, loans, cards, FX, and
compliance. Each module owns its own `screens.toml` + `menus.toml`; a root
`banking.toml` selects which modules are included. The goal is to surface
the rough edges where the current generator and runtime stop being enough.

## Status

This directory is a **design scaffold**, not a compiling app. The individual
module `screens.toml` files use only archetypes the generator already supports
for the "built" screens, and mark the rest `blocked` with the missing primitive
named. A future `cargo nirdosha compose` (or `compose-banking`) command would
merge the selected modules into one generated crate.

## Layout

```
examples/banking/
├── banking.toml              # module selection + global policy
├── modules/
│   ├── core/                 # mandatory: auth, shell, shared entities, audit
│   ├── accounts/             # current/savings accounts
│   ├── transfers/            # fund transfers + payment execution
│   ├── loans/                # loan accounts + disbursement
│   ├── cards/                # debit/credit card management
│   ├── fx/                   # foreign-exchange rates + conversion
│   └── compliance/           # audit reports + sanction screening
└── src/                      # generated output (not present yet)
```

## Composition idea

A composer reads `banking.toml`, loads each enabled module, and emits one
combined project:

1. Merge all `screens.toml` into one register.
2. Merge all `menus.toml` into one navigation graph.
3. Merge approval chains and roles.
4. Wire cross-module references (loans reference accounts, cards reference accounts).
5. Generate `src/primitives.nir` from module-certified primitives.
6. Run `cargo nirdosha generate-screens` on the composed result.

## What is built vs. blocked

| Module | Built screens (generator today) | Blocked / needs primitive |
|---|---|---|
| core | Login, shell, audit report, holidays, product config | Customer entity, real OIDC |
| accounts | Account CRUD | Account open wizard, balance guard |
| transfers | Transfer request CRUD | Transfer execution, idempotency, daily limits |
| loans | Loan account CRUD | Loan disbursement, EMI schedule |
| cards | Card CRUD, card transaction CRUD | Real authorization, daily limits, hotlist |
| fx | FX rate CRUD | FX conversion, dual-currency posting |
| compliance | Audit report | Sanction screening plugin |

## Rough edges discovered

See the bottom of `banking.toml` and each module's `notes` field for the
gaps that came out of this exercise.
