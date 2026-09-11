# Fintech canon v1 — results

Real `nirdosha certify` output for all 13 templates, captured directly
from running the commands below against this directory's files — not
hand-edited or illustrative. Full JSON for one file is shown in full;
the rest are summarized in the table since the shape is identical
(`certificate_version`/`source_hash`/`grammar_hash`/`toolchain_version`
repeat the same schema every time — see [`docs/LANGUAGE.md`](../../docs/LANGUAGE.md)
for Certificate v0's full field list).

## Full output — `01_nonnegative_balance_after_debit.nir`

```json
{
  "certificate_version": "0",
  "source_hash": "sha256:89851a2e08ff9b73f8185a1c89d19298ce0e8304520a2466e37e698049ff201b",
  "grammar_hash": "sha256:708e429be5c3cb081d5550d6891a385abd5dff20d511ad90652f55f8dfcb66a1",
  "toolchain_version": "0.1.0",
  "evidence_tier": "proved",
  "verdict_summary": {
    "verdict": "PROVED",
    "contracts_proved": 1,
    "contracts_unsupported": 0,
    "contracts_failed": 0
  },
  "proof_obligations": {
    "proven_in_range": 0,
    "proven_nonzero_divisor": 0,
    "proven_index_bounds": 0
  }
}
```

## All 12 `validate`-based templates

| File | Verdict | `contracts_proved` | `evidence_tier` |
|---|---|---|---|
| `01_nonnegative_balance_after_debit.nir` | PROVED | 1 | proved |
| `02_overdraft_within_limit.nir` | PROVED | 1 | proved |
| `03_fee_never_reduces_total.nir` | PROVED | 1 | proved |
| `04_refund_never_exceeds_original.nir` | PROVED | 1 | proved |
| `05_late_fee_capped.nir` | PROVED | 1 | proved |
| `06_minimum_payment_enforced.nir` | PROVED | 1 | proved |
| `07_discount_bounded_price.nir` | PROVED | 1 | proved |
| `08_transaction_within_daily_limit.nir` | PROVED | 1 | proved |
| `09_interest_accrual_nonnegative.nir` | PROVED | 1 | proved |
| `10_ledger_debit_credit_conserved.nir` | PROVED | 1 | proved |
| `11_withdrawal_within_balance.nir` | PROVED | 1 | proved |
| `12_credit_limit_not_exceeded.nir` | PROVED | 1 | proved |

**12/12 real Z3 proofs, zero `UNKNOWN`, zero `DISPROVED` in this final
set.** That "zero `DISPROVED`" is not because these templates were
trivial to get right on the first attempt — see the disclosed miss
below — but because every one that failed during writing was fixed
before landing here, the same discipline
`extensions/nirdosha-speckit/docs/examples/wallet-debit.md`'s own
worked example applies: a real counterexample is a finding to act on,
never edited away quietly.

## A real miss, caught and fixed while building this canon

`02_overdraft_within_limit.nir`'s first draft used
`balance_cents - overdraft_limit_cents` as the clamped floor instead of
the fixed value `0 - overdraft_limit_cents` — a real bug (the clamp
depended on the *current* balance instead of being a fixed floor,
so a large balance with zero overdraft room could clamp to a number
far from zero instead of exactly zero). `nirdosha certify` returned a
real `DISPROVED` with the exact counterexample
(`balance_cents = -1, amount_cents = 0, overdraft_limit_cents = 0`,
`result = -1`, violating `result >= -overdraft_limit_cents`). Fixed by
using the fixed floor and adding the missing `balance_cents >= 0`
precondition — the version in this directory is the corrected one,
now genuinely `PROVED`.

## Masking — `13_masked_account_pii.nir`

Not `validate`-based (see the README's own "why 12 and one different
one" section) — verified by compiling and running it for real:

```sh
$ nirdosha build examples/fintech-canon/13_masked_account_pii.nir -o /tmp/canon13
$ /tmp/canon13
```

Real captured stdout (kernel flight-recorder banner omitted):

```
ACC-000123456

Dana Support
```

Line 1: the compliance-cleared caller sees the real account number.
Line 2 (blank): the plain support agent's account number field masks
to `""` — the field-level gate denied a real proof, not a runtime `if`
an attacker could argue past. Line 3: the same caller's unmasked
`display_name` field still passes through untouched, confirming the
mask is scoped to exactly the one field that declares it.

## Reproduce it

```sh
nirdosha certify examples/fintech-canon/01_nonnegative_balance_after_debit.nir
# ... 02 through 12 the same way

nirdosha build examples/fintech-canon/13_masked_account_pii.nir -o /tmp/canon13 && /tmp/canon13
```
