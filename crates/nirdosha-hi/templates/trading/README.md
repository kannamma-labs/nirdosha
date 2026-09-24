# Trading Platform template

A pluggable Nirdosha project template for brokerage/trading applications:
instruments, orders, positions, risk, market data, back-office settlement,
compliance surveillance, and AI/ML alpha/signal modules.

## Modules

| Module | Required | Default | What it covers |
|---|---|---|---|
| `core` | yes | yes | Login, nav shell, dashboard, accounts, audit |
| `instruments` | yes | yes | Tradeable instruments and reference data |
| `orders` | no | yes | Order book and new-order wizard |
| `positions` | no | yes | Current positions and market value |
| `risk` | no | yes | Risk limits and exposure |
| `market_data` | no | no | Live market-data dashboard |
| `backoffice` | no | no | Settlements and ledger posting |
| `compliance` | no | no | Trade surveillance and alerts |
| `ai_ml` | no | no | Alpha models, signals, model performance |

## Usage

Select the Trading template in `nirdosha-hi`, pick modules, choose a country,
and click **Compose project from template**. The composer merges the selected
modules and runs `generate-screens`.

## Money / quantity handling

All money, quantity and price fields are stored as `String`. Real arithmetic,
rounding, P/L, margin, VaR and settlement calculations must be delegated to
certified primitives listed in `blocked_by`.
