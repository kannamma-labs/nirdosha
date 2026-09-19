# `nirdosha-guard-core`

Store-agnostic access-control IR and invariants for Nirdosha Guard.

This crate owns the typed intermediate representation described in
[`rfcs/0023-data-guard.md`](../../rfcs/0023-data-guard.md):
`AccessPlan`, `WritePlan`, `FederatedPlan`, `FilterExpr`, capability
manifests, relation resolvers, evaluation context, and the policy
front-end boundary.

It is deliberately storage-agnostic. Drivers (`nirdosha-guard-sql`,
`nirdosha-guard-arrow`, etc.) translate these plans into the strongest
native enforcement each store supports.

Current phase: **Phase 0 foundation**. The IR types and basic trait
shapes are present; actual policy lowering and driver compilation are
future work.
