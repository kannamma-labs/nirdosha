# `nirdosha-guard-federation`

Federated reads, catalog routing, and the global caps coordinator for
Nirdosha Guard.

Implements RFC 0023 §1C.1: one logical `AccessPlan` expands into a
`FederatedPlan` with per-binding sub-plans, a `MergeSpec`, and a
`BudgetToken`. The merge layer re-filters and re-masks results before
they leave the guard boundary (I16).

Current phase: scaffolding. `CatalogRouter`, `FederatedPlanner`, and
`BudgetCoordinator` are sketched; the real execution engine and cross-
store aggregation safety checks are future work.
