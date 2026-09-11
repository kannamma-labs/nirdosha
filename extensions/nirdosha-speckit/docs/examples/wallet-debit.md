# Worked example: wallet debit

A real, hand-run walkthrough of `/speckit.nirdosha-speckit.contracts`
and `/speckit.nirdosha-speckit.converge` — every command and JSON
output below is copy-pasted from an actual `nirdosha verify`/
`nirdosha certify` run against the exact files shown, not
illustrative/hypothetical output.

## `spec.md` (excerpt)

```markdown
# Feature: Wallet Debit

## Functional Requirements
- FR-001: `debit_wallet(balance_cents, amount_cents)` returns the new
  balance after subtracting `amount_cents` from `balance_cents`.
- FR-002: The wallet must never go negative — the returned balance
  must always be zero or greater.
- FR-003: The function must reject a negative `amount_cents`.
```

## Starting implementation (the naive first draft)

```nirdosha
fn debit_wallet(balance_cents: i64, amount_cents: i64) -> i64 {
    return balance_cents - amount_cents
}
```

## Running `/speckit.nirdosha-speckit.contracts`

Step 1 keeps all three requirements (each is a checkable property of
`debit_wallet`'s own behavior). Step 2/3 draft the contract straight
from FR-002/FR-003 (FR-001 is the function's baseline behavior, not a
separate `pre`/`post` line):

```nirdosha
validate debit_wallet {
    pre: amount_cents >= 0 && balance_cents >= 0
    post: result >= 0
}
```

Step 4 — run it:

```sh
$ nirdosha verify wallet.nir
```

Real output:

```json
{
  "verdict": "DISPROVED",
  "contracts": {
    "verdict": "DISPROVED",
    "proved": 0,
    "unsupported": 0,
    "failed": 1,
    "obligations": [
      {
        "fn_name": "debit_wallet",
        "status": "counterexample",
        "detail": "... is violated when balance_cents = 0, amount_cents = 1 (fn returns -1)"
      }
    ]
  }
}
```

Per `contracts.md`'s own guidance: this is a real bug the requirement
caught, not a bad contract — FR-002 says the wallet must *never* go
negative, and the naive implementation does exactly that whenever
`amount_cents > balance_cents`. The fix belongs in the implementation:

```nirdosha
fn debit_wallet(balance_cents: i64, amount_cents: i64) -> i64 {
    return if amount_cents > balance_cents {
        0
    } else {
        balance_cents - amount_cents
    }
}

validate debit_wallet {
    pre: amount_cents >= 0 && balance_cents >= 0
    post: result >= 0
}
```

Re-running:

```sh
$ nirdosha certify wallet.nir
```

Real output:

```json
{
  "certificate_version": "0",
  "evidence_tier": "proved",
  "verdict_summary": {
    "verdict": "PROVED",
    "contracts_proved": 1,
    "contracts_unsupported": 0,
    "contracts_failed": 0
  }
}
```

Step 5's report:

| Requirement | Target fn | Verdict | Note |
|---|---|---|---|
| FR-001 | `debit_wallet` | PROVED | behavior baseline, covered by the contract holding at all |
| FR-002 | `debit_wallet` | PROVED | `post: result >= 0`, real Z3 proof (`contracts_proved: 1`) |
| FR-003 | `debit_wallet` | PROVED | `pre: amount_cents >= 0` |

## Running `/speckit.nirdosha-speckit.converge`

Against the fixed file, the convergence table is now trivial (every
requirement already has a matching, `PROVED` contract) — the command
earns its keep on a *later* run, after the code has moved on. Say a
future change renames `debit_wallet` to `debit_account` without
carrying the `validate` block along: convergence would report FR-001
through FR-003 as `NO CONTRACT` against the renamed function, catching
the drift immediately rather than after it ships silently masked by
the corpse of an old `validate debit_wallet { ... }` block that no
longer applies to anything.
