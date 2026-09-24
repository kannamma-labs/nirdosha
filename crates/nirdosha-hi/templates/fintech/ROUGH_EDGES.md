# Fintech Payments Hub — known rough edges

1. **Certified primitives are not implemented.** Screens marked `stage = "blocked"`
   need real primitives such as `primitive:payment_intent`,
   `primitive:wallet_topup`, `primitive:gateway_routing`, `primitive:payout_schedule`,
   `primitive:ledger_post`, `primitive:aggregations`, `primitive:report_export`.

2. **Rule-builder / report archetypes are stubbed.** The `rule_builder` and `report`
   archetypes are declared but not yet supported by the generator.

3. **Gateway provider list is illustrative.** Provider names (Stripe, Adyen, etc.)
   are labels; real integrations need API schemas, credentials handling, and
   webhook verification primitives.

4. **PSP protocols are not modeled.** Only a minimal connection register exists;
   ISO 8583, SEPA, ACH, RTP, etc. are not wired.

5. **Fraud rules are not generated.** The fraud screen lists alerts but has no
   rule engine or model integration.

6. **Cross-border / FX not included.** Currency conversion is out of scope for this
   template; use the Banking FX module or a dedicated remittance template.

7. **Sandbox credentials / keys.** No key-management or sandbox-onboarding flow
   is implemented.
