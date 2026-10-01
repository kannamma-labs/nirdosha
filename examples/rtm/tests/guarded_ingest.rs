//! Real `guarded_apply`/`guarded_read` proof against **rtm's own registered
//! policy corpus** (`nirdosha_guard_registry::candidates()`), not a
//! hand-built fixture — "get the data from the code" applied to the
//! runtime path, not just the static `cargo nirdosha verify --guard` path
//! `crates/nirdosha-rt/tests/rtm_policy_corpus.rs` already covers for the
//! doc-derived copy of this same corpus.
//!
//! Uses the in-memory `MemStoreDriver` — proven green today with no extra
//! setup, same as `crates/nirdosha-rt/tests/e2e_data_guard.rs`. Real-Postgres
//! wiring (`nirdosha-guard-store-postgres`) is a documented follow-on, not
//! done here: its own tests are `#[ignore]`d behind
//! `NIRDOSHA_TEST_POSTGRES_URL` + `docker-compose.dev.yml up`, a separate
//! opt-in infra dependency.
//!
//! One honest patch, not a fabrication: `analyst-search-transaction`'s
//! `filter tenant_scope()` clause lowers to `PolicyRecord.filter_ref =
//! Some("tenant_scope()")` with `filter: None`
//! (`crates/nirdosha-guard-registry/src/clauses.rs:318-329`, confirmed by
//! its own test at line 602) — a real, pre-existing, separately-scoped gap:
//! `PolicyRegistration::to_candidate()` never reads `filter_ref` at all, so
//! *no* corpus read policy's scope-function filter reaches
//! `nirdosha-guard-mic`'s evaluator as a concrete `FilterExpr` today, and
//! `MemStoreDriver::query` refuses any read with no filter ("refusing an
//! unscoped scan"). The read-flow test below takes the real registered
//! candidate's real subjects/purpose/caps/masks/obligations and patches in
//! the one field this pre-existing gap can't resolve automatically —
//! exactly what `tenant_scope()` is documented to mean once that gap
//! closes, not an invented shortcut. The read test's seed write needs the
//! identical patch on `ingest-create-txn` for a related, separate reason:
//! `MemStoreDriver` only tags a committed row with a tenant when the write
//! plan itself carried a tenant filter, and that policy has none (a
//! service create scopes by the payload's own `tenant_id` field) — see the
//! comment at that call site.

// Force-links rtm's own `[lib]` target (the actual policy/catalog corpus —
// see src/lib.nir) even though nothing below references it by path: the
// registrations this test needs are a `linkme` link-time side effect of
// that crate being part of this binary, not something reached through a
// normal `use`.
extern crate rtm;

use nirdosha_guard_core::evaluator::PolicyCandidate;
use nirdosha_guard_core::{
    Action, Classification, Destination, Environment, FilterExpr, PaginationMode, Purpose,
    QueryShape, Subject, Tenant, Value,
};
use nirdosha_guard_mic::{EntityBytes, EvalRequest, GuardClient, MemStoreDriver, Outcome, Rejected};

fn corpus_candidate(id: &str) -> PolicyCandidate {
    nirdosha_guard_registry::candidates()
        .into_iter()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("rtm's own registered corpus must contain a policy named `{id}`"))
}

fn context(role: &str, action: Action, resource: &str, purpose: &str) -> nirdosha_guard_core::EvaluationContext {
    nirdosha_guard_core::EvaluationContext {
        subject: Subject { id: "rtm-test-subject".into(), roles: vec![role.into()], claims: vec![], clearance: Classification::Internal },
        tenant: Tenant("tenant-rtm".into()),
        entity: resource.into(),
        dataset: "memory".into(),
        action,
        destination: Destination::Browser,
        environment: Environment { env: "test".into(), ip: None, geo: None, device_posture: None, session_freshness: None },
        time_bucket: "2026-09-21".into(),
        query_shape: QueryShape { verbs: vec![], aggregate: None, grouping_keys: vec![], subject_dimension: None, ordering: vec![], pagination: PaginationMode::LimitOnly { limit: 10 } },
        purpose: Purpose(purpose.into()),
        policy_version: "rtm-test".into(),
    }
}

fn scratch_dir(name: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("rtm-guarded-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    root
}

#[test]
fn ingest_create_txn_commits_through_the_real_corpus_policy() {
    let root = scratch_dir("ingest-create-txn");
    let policy = corpus_candidate("ingest-create-txn");
    assert_eq!(policy.subjects, vec!["SvcIngest".to_string()]);
    assert_eq!(policy.purpose.as_deref(), Some("FraudMonitoring"));
    assert_eq!(policy.affected_row_cap, Some(1));

    let mut client = GuardClient::new(vec![policy], "rtm@1.0", root.join("audit.jsonl"));
    let driver = MemStoreDriver::new();
    let ctx = context("SvcIngest", Action::Create, "transaction", "FraudMonitoring");
    let outcome = client
        .guarded_apply(&EvalRequest { context: ctx }, &driver, EntityBytes(b"txn-payload".to_vec()), "trace-rtm-001", 1_700_000_000)
        .expect("ingest-create-txn must let SvcIngest create a transaction — it's the corpus's own [RFC §8.1 verbatim] allow");
    assert!(matches!(outcome, Outcome::Committed { .. }));
    assert_eq!(driver.get("transaction"), Some(b"txn-payload".to_vec()));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn analyst_search_transaction_reads_through_the_real_corpus_policy() {
    let root = scratch_dir("analyst-search-transaction");
    let mut policy = corpus_candidate("analyst-search-transaction");
    assert_eq!(policy.subjects, vec!["Analyst".to_string(), "OpsAnalyst".to_string()]);
    assert_eq!(policy.purpose.as_deref(), Some("AmlInvestigation"));
    // The one patched field — see module doc comment for exactly why.
    policy.filter = Some(FilterExpr::TenantEq { value: Value::Str("tenant-rtm".into()) });

    let mut client = GuardClient::new(vec![policy], "rtm@1.0", root.join("audit.jsonl"));
    let driver = MemStoreDriver::new();

    // Seed one row via the real ingest policy first, so the read has
    // something real (written through the guard, not poked into the driver
    // directly) to find. `MemStoreDriver` only tags a committed row with a
    // tenant when the *write* plan itself carried a tenant filter
    // (`prepare()`'s `extract_tenant(&ir.filter)` — crates/nirdosha-guard-mic/
    // src/lib.rs), and "ingest-create-txn" has no `filter` clause at all
    // (a service create scopes by the payload's own `tenant_id` field, not
    // a policy-level filter) — so the seed write needs the same kind of
    // patch as the read, for the identical reason, or the row would land
    // untagged and no tenant-scoped read could ever find it.
    let mut seed_policy = corpus_candidate("ingest-create-txn");
    seed_policy.filter = Some(FilterExpr::TenantEq { value: Value::Str("tenant-rtm".into()) });
    let mut writer = GuardClient::new(vec![seed_policy], "rtm@1.0", root.join("audit-write.jsonl"));
    writer
        .guarded_apply(
            &EvalRequest { context: context("SvcIngest", Action::Create, "transaction", "FraudMonitoring") },
            &driver,
            EntityBytes(b"txn-payload".to_vec()),
            "trace-rtm-seed",
            1_700_000_000,
        )
        .expect("seed write must succeed");

    let outcome = client
        .guarded_read(&EvalRequest { context: context("Analyst", Action::Read, "transaction", "AmlInvestigation") }, &driver, "trace-rtm-002", 1_700_000_100)
        .expect("analyst-search-transaction must let Analyst read transaction rows in their own tenant");
    assert_eq!(outcome.rows, vec![EntityBytes(b"txn-payload".to_vec())]);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn ingest_no_readback_denies_the_service_that_wrote_it() {
    let root = scratch_dir("ingest-no-readback");
    let policy = corpus_candidate("ingest-no-readback");
    assert_eq!(policy.subjects, vec!["SvcIngest".to_string()]);

    let mut client = GuardClient::new(vec![policy], "rtm@1.0", root.join("audit.jsonl"));
    let driver = MemStoreDriver::new();
    let ctx = context("SvcIngest", Action::Read, "transaction", "FraudMonitoring");
    let result = client.guarded_read(&EvalRequest { context: ctx }, &driver, "trace-rtm-003", 1_700_000_200);

    assert!(
        matches!(result, Err(Rejected::Denied { .. })),
        "the corpus's own V7 SoD record (`ingest-no-readback`) must actually deny SvcIngest reading what it wrote, not just exist as a static record: {result:?}"
    );

    let _ = std::fs::remove_dir_all(&root);
}
