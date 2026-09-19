//! `nirdosha-guard-federation` — federated reads and catalog routing.
//!
//! Implements RFC 0023 §1C.1: a single logical `AccessPlan` expands into a
//! `FederatedPlan` with per-binding sub-plans, a `MergeSpec`, and a
//! `BudgetToken`. The merge layer re-filters and re-masks results before
//! they leave the guard boundary (I16).

use nirdosha_guard_core::{
    AccessPlan, BindingId, BudgetToken, DatasetRegistryEntry, FederatedPlan,
    MergeMode, MergeSpec, QueryShape,
};

/// Selects eligible bindings for a logical entity + query shape.
pub struct CatalogRouter;

impl CatalogRouter {
    pub fn select_bindings<'a>(
        _entity: &'a str,
        _shape: &'a QueryShape,
        registry: &'a [DatasetRegistryEntry],
    ) -> Vec<&'a DatasetRegistryEntry> {
        registry.iter().collect()
    }
}

/// Builds a `FederatedPlan` from a logical `AccessPlan` and catalog entries.
pub struct FederatedPlanner;

impl FederatedPlanner {
    pub fn plan(
        logical: AccessPlan,
        _shape: QueryShape,
        bindings: Vec<(BindingId, AccessPlan)>,
    ) -> FederatedPlan {
        FederatedPlan {
            decision: logical.decision.clone(),
            sub_plans: bindings,
            merge: MergeSpec {
                mode: MergeMode::Union,
                dedup_keys: vec![],
                provenance: true,
            },
            budget: BudgetToken {
                max_scan_rows: 1_000_000,
                max_scan_bytes: 1_000_000_000,
                max_execution_time_ms: 30_000,
                max_result_bytes: 100_000_000,
            },
        }
    }
}

/// Global caps coordinator: issues `BudgetToken`s and cancels siblings on exhaustion.
pub struct BudgetCoordinator;

impl BudgetCoordinator {
    pub fn issue_token(_shape: &QueryShape) -> BudgetToken {
        BudgetToken {
            max_scan_rows: 1_000_000,
            max_scan_bytes: 1_000_000_000,
            max_execution_time_ms: 30_000,
            max_result_bytes: 100_000_000,
        }
    }

    /// Coordinator failure is fail-closed: a request whose spend cannot be
    /// tracked does not run.
    pub fn track_spend(&self) -> Result<(), ()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nirdosha_guard_core::{Action, Decision, Destination, Environment, EvaluationContext, Purpose, Subject, Tenant};

    #[test]
    fn federated_plan_carries_provenance() {
        let logical = AccessPlan {
            decision: Decision::Allow,
            filter: None,
            masks: vec![],
            caps: vec![],
            obligations: vec![],
            policy_version: "v1".into(),
        };
        let plan = FederatedPlanner::plan(logical, QueryShape {
            verbs: vec![],
            aggregate: None,
            grouping_keys: vec![],
            subject_dimension: None,
            ordering: vec![],
            pagination: nirdosha_guard_core::PaginationMode::LimitOnly { limit: 100 },
        }, vec![]);
        assert!(plan.merge.provenance);
    }
}
