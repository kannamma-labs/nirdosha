# Fintech Payments Hub template

A pluggable Nirdosha project template for payment-platform applications:
merchants, payment gateways, PSP connections, wallets, payouts, fraud
monitoring, and reporting.

## Modules

| Module | Required | Default | What it covers |
|---|---|---|---|
| `core` | yes | yes | Dashboard, audit log, shared entities |
| `merchants` | yes | yes | Merchant directory and onboarding |
| `payment_gateways` | no | yes | Stripe/Adyen/Braintree-style gateway config |
| `psp` | no | no | Direct processor / acquiring-bank connections |
| `wallets` | no | yes | Stored-value wallets and top-up flows |
| `payouts` | no | no | Merchant / partner payouts |
| `fraud` | no | yes | Fraud alert queue and risk-score review |
| `reporting` | no | no | Aggregated reports and exports |

## Usage

Select the Fintech template in `nirdosha-hi`, pick modules, choose a country,
and click **Compose project from template**. The composer merges the selected
modules, drops blocked screens from menus, and runs `generate-screens`.

## Money handling

All money fields are stored as `String` (cents). Real decimal arithmetic,
rounding, fee splits, and ledger posting must be delegated to certified
primitives listed in `blocked_by` — the generator does not compute money.
