# Trading Platform — known rough edges

1. **Certified primitives are not implemented.** Blocked screens need
   primitives such as `primitive:order_validate`, `primitive:margin_calc`,
   `primitive:market_data_feed`, `primitive:settlement_calc`,
   `primitive:trade_surveillance`, `primitive:alpha_model`, etc.

2. **Market data is illustrative.** No real feed integration, WebSocket,
   or price-cache is wired.

3. **Order execution is not modeled.** The order book is read-only; matching
   engine, exchange connectivity and FIX protocols are not implemented.

4. **Risk calculations are delegated.** VaR, Sharpe, max drawdown and
   position limits rely on future primitives.

5. **Back-office settlement is incomplete.** Ledger posting and T+2 settlement
   calculations are not generated.

6. **Compliance surveillance is stubbed.** Real surveillance needs pattern
   detection, wash-trade rules and regulatory reporting primitives.

7. **AI/ML models are future primitives.** Alpha models, signal generation,
   backtesting and model performance are listed for roadmap visibility.

8. **Multi-country regulations.** US/GB/IN parameters are placeholders;
   real jurisdictional rules are not implemented.
