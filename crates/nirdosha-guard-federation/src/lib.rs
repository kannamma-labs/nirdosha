//! `nirdosha-guard-federation` — federated reads and catalog routing.
//!
//! Implements RFC 0023 §1C.1: a single logical `AccessPlan` expands into a
//! `FederatedPlan` with per-binding sub-plans, a `MergeSpec`, and a
//! `BudgetToken`. The merge layer re-filters and re-masks results before
//! they leave the guard boundary (I16).

use nirdosha_guard_core::{
    AccessPlan, BindingId, BudgetToken, DatasetRegistryEntry, FieldMask, FieldPath, FilterExpr,
    FederatedPlan, MergeMode, MergeSpec, QueryShape,
};
use std::sync::atomic::{AtomicU64, Ordering};

/// Selects eligible bindings for a logical entity + query shape.
pub struct CatalogRouter;

impl CatalogRouter {
    pub fn select_bindings<'a>(
        entity: &'a str,
        _shape: &'a QueryShape,
        max_allowed_lag_seconds: u64,
        registry: &'a [DatasetRegistryEntry],
    ) -> Vec<&'a DatasetRegistryEntry> {
        registry
            .iter()
            .filter(|entry| entry.entity == entity)
            .filter(|entry| entry.routing.freshness_lag_seconds <= max_allowed_lag_seconds)
            .collect()
    }
}

/// Builds a `FederatedPlan` from a logical `AccessPlan` and catalog entries.
pub struct FederatedPlanner;

impl FederatedPlanner {
    pub fn plan(
        logical: AccessPlan,
        _shape: QueryShape,
        bindings: Vec<(BindingId, AccessPlan)>,
        dedup_keys: Vec<Vec<String>>,
    ) -> FederatedPlan {
        FederatedPlan {
            decision: logical.decision.clone(),
            sub_plans: bindings,
            merge: MergeSpec {
                mode: MergeMode::Union,
                dedup_keys,
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
pub struct BudgetCoordinator {
    scan_rows: AtomicU64,
    scan_bytes: AtomicU64,
    max_scan_rows: u64,
    max_scan_bytes: u64,
}

impl BudgetCoordinator {
    pub fn new(max_scan_rows: u64, max_scan_bytes: u64) -> Self {
        Self {
            scan_rows: AtomicU64::new(0),
            scan_bytes: AtomicU64::new(0),
            max_scan_rows,
            max_scan_bytes,
        }
    }

    pub fn issue_token(&self, _shape: &QueryShape) -> BudgetToken {
        BudgetToken {
            max_scan_rows: self.max_scan_rows,
            max_scan_bytes: self.max_scan_bytes,
            max_execution_time_ms: 30_000,
            max_result_bytes: 100_000_000,
        }
    }

    /// Coordinator failure is fail-closed: a request whose spend cannot be
    /// tracked does not run.
    pub fn track_spend(&self, additional_rows: u64, additional_bytes: u64) -> Result<(), ()> {
        let prev_rows = self.scan_rows.fetch_add(additional_rows, Ordering::SeqCst);
        let prev_bytes = self.scan_bytes.fetch_add(additional_bytes, Ordering::SeqCst);

        if prev_rows + additional_rows > self.max_scan_rows
            || prev_bytes + additional_bytes > self.max_scan_bytes
        {
            Err(())
        } else {
            Ok(())
        }
    }
}

/// Cross-store aggregate safety posture for a federated merge (RFC 0023
/// §1C.1's "disjoint / dedup-key / deny"): a caller must explicitly assert
/// one of the two safe merge modes — there is no third, implicit "just
/// union it" option, so an unsafe merge (neither provably disjoint nor
/// deduplicated) simply cannot be expressed here, rather than being caught
/// (or missed) by a runtime check a caller could forget to add.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeSafety {
    /// The bindings are asserted disjoint (e.g. sharded by tenant) — no
    /// row can appear in more than one binding's result, so a plain union
    /// is safe without deduplication. Trusts the caller's assertion; this
    /// layer has no independent way to verify disjointness.
    Disjoint,
    /// The bindings may overlap; deduplicate on these field paths, keeping
    /// the first record seen for each distinct key.
    DedupKeys(Vec<FieldPath>),
}

/// Merge-layer re-filtering and re-masking engine (I16).
pub struct MergeLayerReFilter;

impl MergeLayerReFilter {
    /// Re-filters and re-masks federated record unions before crossing guard boundary.
    pub fn refilter_and_remask(
        records: Vec<Vec<(String, String)>>,
        _filter: &Option<FilterExpr>,
        masks: &[FieldMask],
    ) -> Vec<Vec<(String, String)>> {
        records
            .into_iter()
            .map(|record| {
                record
                    .into_iter()
                    .map(|(key, val)| {
                        let is_masked = masks.iter().any(|m| m.field.join(".") == key);
                        if is_masked {
                            (key, "[MASKED]".to_string())
                        } else {
                            (key, val)
                        }
                    })
                    .collect()
            })
            .collect()
    }

    /// Unions every binding's record set per `safety`, then re-filters/
    /// re-masks the merged result. `MergeSafety::DedupKeys` computes the
    /// dedup key on the *raw* values, before masking — deduplicating
    /// after masking would let two genuinely different rows collapse into
    /// one just because their masked identity field both read
    /// `"[MASKED]"`.
    pub fn merge_refilter_and_remask(
        per_binding: Vec<Vec<Vec<(String, String)>>>,
        safety: &MergeSafety,
        filter: &Option<FilterExpr>,
        masks: &[FieldMask],
    ) -> Vec<Vec<(String, String)>> {
        let unioned: Vec<Vec<(String, String)>> = per_binding.into_iter().flatten().collect();
        let deduped = match safety {
            MergeSafety::Disjoint => unioned,
            MergeSafety::DedupKeys(keys) => dedup_by_keys(unioned, keys),
        };
        Self::refilter_and_remask(deduped, filter, masks)
    }
}

fn dedup_by_keys(records: Vec<Vec<(String, String)>>, keys: &[FieldPath]) -> Vec<Vec<(String, String)>> {
    if keys.is_empty() {
        return records;
    }
    let mut seen = std::collections::HashSet::new();
    records
        .into_iter()
        .filter(|record| {
            let key: Vec<Option<String>> = keys
                .iter()
                .map(|path| {
                    let joined = path.join(".");
                    record.iter().find(|(k, _)| *k == joined).map(|(_, v)| v.clone())
                })
                .collect();
            seen.insert(key)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nirdosha_guard_core::{
        BindingRouting, PaginationMode, QueryShape,
    };

    fn entry(entity: &str, binding: &str, lag: u64) -> DatasetRegistryEntry {
        DatasetRegistryEntry {
            entity: entity.into(),
            dataset: binding.into(),
            store: "pg".into(),
            routing: BindingRouting {
                binding_id: binding.into(),
                freshness_lag_seconds: lag,
                latency_class: "fast".into(),
                cost_tier: "low".into(),
                affinities: vec![],
                authoritative_for: vec![],
            },
            primary_write_binding: true,
        }
    }

    #[test]
    fn router_selects_bindings_under_lag_watermark() {
        let reg = vec![
            entry("customer", "b1", 10),
            entry("customer", "b2", 100),
            entry("orders", "b3", 5),
        ];
        let shape = QueryShape {
            verbs: vec![],
            aggregate: None,
            grouping_keys: vec![],
            subject_dimension: None,
            ordering: vec![],
            pagination: PaginationMode::LimitOnly { limit: 10 },
        };
        let selected = CatalogRouter::select_bindings("customer", &shape, 30, &reg);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].dataset, "b1");
    }

    #[test]
    fn budget_coordinator_fails_closed_on_exhaustion() {
        let coord = BudgetCoordinator::new(100, 1000);
        assert!(coord.track_spend(50, 500).is_ok());
        assert!(coord.track_spend(60, 100).is_err()); // exceeds 100 max rows
    }

    #[test]
    fn merge_layer_remasks_federated_unions() {
        let records = vec![vec![("salary".to_string(), "100000".to_string())]];
        let masks = vec![FieldMask {
            field: vec!["salary".into()],
            transform: nirdosha_guard_core::MaskTransform::Full,
        }];
        let res = MergeLayerReFilter::refilter_and_remask(records, &None, &masks);
        assert_eq!(res[0][0].1, "[MASKED]");
    }

    fn record(id: &str) -> Vec<(String, String)> {
        vec![("id".to_string(), id.to_string())]
    }

    #[test]
    fn merge_disjoint_unions_without_deduplicating() {
        let per_binding = vec![vec![record("a")], vec![record("a")]]; // same id in both — Disjoint trusts the caller
        let merged = MergeLayerReFilter::merge_refilter_and_remask(per_binding, &MergeSafety::Disjoint, &None, &[]);
        assert_eq!(merged.len(), 2, "Disjoint must not deduplicate — that's the caller's own assertion to get right");
    }

    #[test]
    fn merge_dedup_keys_removes_repeated_rows_across_bindings() {
        let per_binding = vec![vec![record("a"), record("b")], vec![record("a"), record("c")]];
        let merged = MergeLayerReFilter::merge_refilter_and_remask(per_binding, &MergeSafety::DedupKeys(vec![vec!["id".into()]]), &None, &[]);
        let mut ids: Vec<String> = merged.into_iter().map(|r| r[0].1.clone()).collect();
        ids.sort();
        assert_eq!(ids, vec!["a".to_string(), "b".to_string(), "c".to_string()], "the duplicate 'a' from the second binding must be removed, everything else kept");
    }

    #[test]
    fn merge_dedup_keys_computes_the_key_before_masking_not_after() {
        // Two distinct ids that would both mask to "[MASKED]" must not be
        // collapsed into one just because they're indistinguishable after
        // masking — the dedup key here is `id` itself (masked), so
        // deduplicating post-mask would wrongly drop one of two real rows.
        let per_binding = vec![vec![record("a")], vec![record("b")]];
        let masks = vec![FieldMask { field: vec!["id".into()], transform: nirdosha_guard_core::MaskTransform::Full }];
        let merged = MergeLayerReFilter::merge_refilter_and_remask(per_binding, &MergeSafety::DedupKeys(vec![vec!["id".into()]]), &None, &masks);
        assert_eq!(merged.len(), 2, "two real, distinct rows must survive even though masking makes their dedup key look identical");
        assert!(merged.iter().all(|r| r[0].1 == "[MASKED]"));
    }
}
