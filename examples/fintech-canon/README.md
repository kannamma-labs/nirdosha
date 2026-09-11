# Fintech canon v1

`nirdosha-master-plan.md` Part 3 Dec 2026's "Fintech canon v1 — 12–15
`validate` templates (payments, ledger, masking)" (parity target:
C Proof). Thirteen small, real, individually `nirdosha certify`-proved
`.nir` files, each demonstrating one canonical financial-code
correctness pattern — not illustrative snippets, every one of them
compiles, and twelve of the thirteen carry a real Z3-proved `validate`
contract. See [`RESULTS.md`](./RESULTS.md) for the actual captured
`nirdosha certify` output for every file.

## Payments (6)

| File | Guarantees |
|---|---|
| [`03_fee_never_reduces_total.nir`](./03_fee_never_reduces_total.nir) | Adding a fee never produces a total below the original amount |
| [`04_refund_never_exceeds_original.nir`](./04_refund_never_exceeds_original.nir) | A refund can never exceed what was originally charged |
| [`05_late_fee_capped.nir`](./05_late_fee_capped.nir) | A late fee never exceeds a policy/regulatory cap |
| [`06_minimum_payment_enforced.nir`](./06_minimum_payment_enforced.nir) | An accepted payment never falls below the configured minimum |
| [`07_discount_bounded_price.nir`](./07_discount_bounded_price.nir) | A discount never produces a negative price, and never exceeds the original price |
| [`08_transaction_within_daily_limit.nir`](./08_transaction_within_daily_limit.nir) | An accepted transaction respects a daily spending limit against what's already been spent |

## Ledger (6)

| File | Guarantees |
|---|---|
| [`01_nonnegative_balance_after_debit.nir`](./01_nonnegative_balance_after_debit.nir) | A debit never takes a balance negative |
| [`02_overdraft_within_limit.nir`](./02_overdraft_within_limit.nir) | A debit with an approved overdraft never goes past the agreed floor |
| [`09_interest_accrual_nonnegative.nir`](./09_interest_accrual_nonnegative.nir) | Accruing interest never reduces a balance below principal |
| [`10_ledger_debit_credit_conserved.nir`](./10_ledger_debit_credit_conserved.nir) | Posting a debit+credit entry never leaves the account negative |
| [`11_withdrawal_within_balance.nir`](./11_withdrawal_within_balance.nir) | A withdrawal never pays out more than the account holds, and never leaves it negative |
| [`12_credit_limit_not_exceeded.nir`](./12_credit_limit_not_exceeded.nir) | A charge never pushes a balance past its approved credit limit |

## Masking (1)

| File | Guarantees |
|---|---|
| [`13_masked_account_pii.nir`](./13_masked_account_pii.nir) | An account number field masks itself unless the caller separately proves a compliance role — the real, compiled `check_role`/field-masking mechanism, not a `validate` contract (see below) |

## Why 12 `validate` files and one different one

Twelve of these are Hoare-style `pre`/`post` contracts, proved by Z3 —
the exact same `nirdosha certify` pipeline every other file in this
repo uses. Masking is a genuinely different guarantee (access control
over *data returned*, not an input/output relationship Z3 reasons
about), enforced by a different, already-real mechanism:
`requires(role: ...)` field annotations compiled directly into
`codegen.rs`'s return-value lowering — see
`examples/features/50_field_masking_and_check_role.nir` for the full,
unabridged demonstration this file's own template is trimmed from.
Both are real, both are compiled, neither is decorative.

## Reproduce it

```sh
nirdosha certify examples/fintech-canon/01_nonnegative_balance_after_debit.nir
# ... same for 02 through 12

nirdosha build examples/fintech-canon/13_masked_account_pii.nir -o /tmp/canon13 && /tmp/canon13
```
