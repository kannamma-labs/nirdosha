//! Driver capability attestation (I12, RFC 0023 §7) — Plan Phase 10.
//!
//! A `CapabilityManifest` is a driver's *claim* about what it supports.
//! Nothing checked that claim against reality anywhere in the workspace
//! before this: `PostgresStoreDriver`'s manifest is hand-verified honest
//! (its own doc comment: "exactly the `FilterExpr` variants
//! `RdbmsEmitter::emit_where` actually translates today"), but nothing
//! *proved* it, and nothing would catch a future manifest drifting out of
//! sync with what a driver's `query()` actually does.
//!
//! `attest_canary_rows` closes that gap the way RFC 0023 §7 describes:
//! seed known rows, run a real query through the driver for each claimed
//! `FilterNodeKind`, and compare the driver's actual result against a
//! reference expectation computed independently (plain Rust over the
//! seeded fixture, not the driver's own filter-matching code — checking a
//! driver's claim against *itself* would prove nothing).

use crate::{EntityBytes, PlanIr, ReadPlanIr, StoreDriver};
use nirdosha_guard_core::{CompareOp, FilterExpr, FilterNodeKind, PaginationMode, Value};
use std::collections::HashSet;

/// One canary-attestation result for a single claimed `FilterNodeKind`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttestationFinding {
    pub kind: FilterNodeKind,
    pub verified: bool,
    pub detail: String,
}

/// `FilterNodeKind`s this harness can attest with the canary fixture it
/// seeds (three rows, one field each: `resource`, `tenant`). Combinators
/// (`And`/`Or`/`Not`) aren't independently attestable — they're only
/// meaningful wrapping another node, which is already covered on its own.
/// `TimeRange` isn't either: neither driver's schema (`guard_entities`, or
/// `MemStoreDriver`'s `resource`/`tenant` pair) has a filterable timestamp
/// column for this fixture to exercise honestly — claiming to attest it
/// anyway with a fabricated field would be exactly the kind of dishonest
/// claim this harness exists to catch, just relocated into the test
/// fixture instead of the manifest.
const ATTESTABLE_KINDS: &[FilterNodeKind] = &[
    FilterNodeKind::Eq,
    FilterNodeKind::In,
    FilterNodeKind::TenantEq,
    FilterNodeKind::Pattern,
    FilterNodeKind::Compare,
];

const CANARY_TENANT: &str = "attest-tenant";
const CANARY_RESOURCES: [&str; 3] = ["attest-canary-1", "attest-canary-2", "attest-canary-3"];

/// Seeds the canary fixture (idempotent per resource key — a `Create` on
/// an already-seeded canary is expected to fail and is ignored, so this
/// can run repeatedly against the same driver instance) and returns one
/// `AttestationFinding` per manifest-claimed, attestable `FilterNodeKind`.
pub fn attest_canary_rows<D: StoreDriver>(driver: &D) -> Vec<AttestationFinding> {
    for resource in CANARY_RESOURCES {
        let plan = PlanIr {
            resource: resource.into(),
            dataset: "attestation".into(),
            filter: Some(FilterExpr::TenantEq { value: Value::Str(CANARY_TENANT.into()) }),
            row_scope: None,
            action: nirdosha_guard_core::WriteAction::Create,
            affected_row_cap: None,
            policy_version: "attestation".into(),
        };
        if let Ok(prepared) = driver.prepare(&plan) {
            // Best-effort seed: a prior attestation run (or a real row
            // that happens to collide) means Create fails here, which is
            // fine — the fixture only needs to exist, not be freshly
            // written by this call.
            let _ = driver.commit(prepared, EntityBytes(resource.as_bytes().to_vec()));
        }
    }

    let claimed: HashSet<FilterNodeKind> = driver.manifest().supported_filter_nodes.iter().cloned().collect();
    ATTESTABLE_KINDS
        .iter()
        .filter(|kind| claimed.contains(kind))
        .map(|kind| attest_one(driver, kind.clone()))
        .collect()
}

fn attest_one<D: StoreDriver>(driver: &D, kind: FilterNodeKind) -> AttestationFinding {
    let (filter, expected): (FilterExpr, Vec<&str>) = match kind {
        FilterNodeKind::Eq => (
            FilterExpr::And(vec![
                FilterExpr::TenantEq { value: Value::Str(CANARY_TENANT.into()) },
                FilterExpr::Eq { field: vec!["resource".into()], value: Value::Str("attest-canary-2".into()) },
            ]),
            vec!["attest-canary-2"],
        ),
        FilterNodeKind::In => (
            FilterExpr::And(vec![
                FilterExpr::TenantEq { value: Value::Str(CANARY_TENANT.into()) },
                FilterExpr::In { field: vec!["resource".into()], values: vec![Value::Str("attest-canary-1".into()), Value::Str("attest-canary-3".into())] },
            ]),
            vec!["attest-canary-1", "attest-canary-3"],
        ),
        FilterNodeKind::TenantEq => (
            FilterExpr::TenantEq { value: Value::Str(CANARY_TENANT.into()) },
            vec!["attest-canary-1", "attest-canary-2", "attest-canary-3"],
        ),
        FilterNodeKind::Pattern => (
            FilterExpr::And(vec![
                FilterExpr::TenantEq { value: Value::Str(CANARY_TENANT.into()) },
                FilterExpr::Pattern { field: vec!["resource".into()], matcher: nirdosha_guard_core::PatternMatcher::Prefix("attest-canary-".into()) },
            ]),
            vec!["attest-canary-1", "attest-canary-2", "attest-canary-3"],
        ),
        FilterNodeKind::Compare => (
            // Plain lexicographic string comparison over `resource` — the
            // reference expectation below computes the same comparison
            // independently, in Rust, over the known canary set.
            FilterExpr::And(vec![
                FilterExpr::TenantEq { value: Value::Str(CANARY_TENANT.into()) },
                FilterExpr::Compare { field: vec!["resource".into()], op: CompareOp::Gt, value: Value::Str("attest-canary-1".into()) },
            ]),
            vec!["attest-canary-2", "attest-canary-3"],
        ),
        _ => unreachable!("ATTESTABLE_KINDS only contains kinds handled above"),
    };

    let plan = ReadPlanIr {
        resource: "attestation".into(),
        dataset: "attestation".into(),
        filter: Some(filter),
        caps: vec![],
        pagination: PaginationMode::LimitOnly { limit: 100 },
        policy_version: "attestation".into(),
    };
    let mut expected_sorted = expected.to_vec();
    expected_sorted.sort();

    match driver.query(&plan) {
        Ok(result) => {
            let mut actual: Vec<String> = result
                .rows
                .into_iter()
                .filter_map(|row| String::from_utf8(row.0).ok())
                .filter(|resource| CANARY_RESOURCES.contains(&resource.as_str()))
                .collect();
            actual.sort();
            let verified = actual == expected_sorted;
            let detail = if verified {
                format!("expected {expected_sorted:?}, got exactly that")
            } else {
                format!(
                    "expected {expected_sorted:?}, got {actual:?} — {}'s manifest claims {kind:?} support but query() does not honor it",
                    driver.manifest().driver_name
                )
            };
            AttestationFinding { kind, verified, detail }
        }
        Err(error) => AttestationFinding { kind, verified: false, detail: format!("query failed outright: {error:?}") },
    }
}

/// Removes every `FilterNodeKind` a failed attestation caught lying about,
/// returning the manifest a caller should actually trust — I12's "dynamic
/// downgrade on detected lie," not a permanent edit to the driver's own
/// claimed manifest (which might attest cleanly again after a fix; this
/// downgrade is a per-attestation-run view, recomputed each time).
pub fn downgrade_manifest(
    manifest: &nirdosha_guard_core::CapabilityManifest,
    findings: &[AttestationFinding],
) -> nirdosha_guard_core::CapabilityManifest {
    let lying: HashSet<&FilterNodeKind> = findings.iter().filter(|f| !f.verified).map(|f| &f.kind).collect();
    let mut downgraded = manifest.clone();
    downgraded.supported_filter_nodes.retain(|kind| !lying.contains(kind));
    downgraded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MemStoreDriver, PlanError, PlanIr, Prepared, Receipt};
    use nirdosha_guard_core::CapabilityManifest;

    /// Wraps a real `MemStoreDriver` for `prepare`/`commit`/`query`/
    /// `lineage`, but reports a manifest that falsely adds
    /// `FilterNodeKind::Compare` on top of the driver's own (honest)
    /// claims. Exists so "attestation catches a lie" stays a standalone,
    /// deliberate test case — decoupled from whatever `MemStoreDriver`'s
    /// actual manifest happens to claim today.
    struct LyingStoreDriver {
        inner: MemStoreDriver,
        manifest: CapabilityManifest,
    }

    impl LyingStoreDriver {
        fn new() -> Self {
            let inner = MemStoreDriver::new();
            let mut manifest = inner.manifest().clone();
            manifest.driver_name = "lying-mem-store".into();
            manifest.supported_filter_nodes.push(FilterNodeKind::Compare);
            Self { inner, manifest }
        }
    }

    impl StoreDriver for LyingStoreDriver {
        fn manifest(&self) -> &CapabilityManifest {
            &self.manifest
        }
        fn prepare(&self, ir: &PlanIr) -> Result<Prepared, PlanError> {
            self.inner.prepare(ir)
        }
        fn commit(&self, prepared: Prepared, entity: EntityBytes) -> Result<Receipt, PlanError> {
            self.inner.commit(prepared, entity)
        }
        fn query(&self, plan: &ReadPlanIr) -> Result<crate::QueryResult, PlanError> {
            self.inner.query(plan)
        }
    }

    #[test]
    fn catches_a_driver_lying_about_compare_support() {
        // MemStoreDriver's own match_filter never honors Compare (see
        // lib.rs's doc comment there) — LyingStoreDriver claims it anyway,
        // and attestation must catch that regardless of what the wrapped
        // driver's real manifest says.
        let driver = LyingStoreDriver::new();
        assert!(driver.manifest().supported_filter_nodes.contains(&FilterNodeKind::Compare), "test assumes the mock claims Compare support");
        let findings = attest_canary_rows(&driver);
        let compare_finding = findings.iter().find(|f| f.kind == FilterNodeKind::Compare).expect("Compare must be attested since the manifest claims it");
        assert!(!compare_finding.verified, "expected the Compare claim to be caught as a lie: {compare_finding:?}");
    }

    #[test]
    fn verifies_mem_store_drivers_honest_claims() {
        // MemStoreDriver's manifest no longer claims anything it doesn't
        // honor (Compare/TimeRange were removed once this harness caught
        // the lie) — every attestable claim it makes should verify clean.
        let driver = MemStoreDriver::new();
        let findings = attest_canary_rows(&driver);
        assert!(!findings.is_empty(), "MemStoreDriver should claim at least one attestable filter kind");
        for finding in &findings {
            assert!(finding.verified, "{:?} should be a real, honest claim: {finding:?}", finding.kind);
        }
    }

    #[test]
    fn downgrade_strips_only_the_lying_claim() {
        let driver = LyingStoreDriver::new();
        let findings = attest_canary_rows(&driver);
        let downgraded = downgrade_manifest(driver.manifest(), &findings);
        assert!(!downgraded.supported_filter_nodes.contains(&FilterNodeKind::Compare), "the lying claim must be stripped");
        assert!(downgraded.supported_filter_nodes.contains(&FilterNodeKind::Eq), "honest claims must survive the downgrade");
        assert!(downgraded.supported_filter_nodes.contains(&FilterNodeKind::TenantEq));
    }

    #[test]
    fn downgraded_manifest_changes_the_computed_enforcement_level() {
        use nirdosha_guard_core::{compute_enforcement_level, EnforcementLevel};
        let driver = LyingStoreDriver::new();
        let findings = attest_canary_rows(&driver);
        let downgraded = downgrade_manifest(driver.manifest(), &findings);

        let filter = FilterExpr::And(vec![
            FilterExpr::TenantEq { value: Value::Str("t".into()) },
            FilterExpr::Compare { field: vec!["amount".into()], op: CompareOp::Gt, value: Value::Int(0) },
        ]);
        // Against the original (lying) manifest, this filter looks fully
        // pushable — Compare is "supported."
        assert_eq!(compute_enforcement_level(Some(&filter), driver.manifest()), EnforcementLevel::L4FullPushdown);
        // Against the attested, downgraded manifest, the truth comes out:
        // TenantEq still pushes, Compare doesn't — partial, not full.
        assert_eq!(compute_enforcement_level(Some(&filter), &downgraded), EnforcementLevel::L2ScanTime);
    }
}
