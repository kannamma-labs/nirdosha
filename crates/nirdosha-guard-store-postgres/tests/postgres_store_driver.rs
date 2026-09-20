//! Opt-in, real-Postgres proof for `PostgresStoreDriver`, matching the
//! repo's existing `NIRDOSHA_TEST_POSTGRES_URL`-gated, `#[ignore]`-by-default
//! convention (`crates/compiler/tests/postgres.rs`,
//! `crates/runtime-kernels/src/kernel/db.rs`'s own tests): never run by
//! CI, fails loudly rather than skipping when no real server is reachable.
//!
//! Run against `docker-compose.dev.yml`:
//!   docker compose -f docker-compose.dev.yml up -d
//!   NIRDOSHA_TEST_POSTGRES_URL=postgres://nirdosha:nirdosha@127.0.0.1:5432/nirdosha_dev \
//!       cargo test -p nirdosha-guard-store-postgres -- --ignored

use nirdosha_guard_core::evaluator::{PolicyCandidate, PolicyEffect};
use nirdosha_guard_core::{
    Action, Classification, Destination, Environment, FilterExpr, PaginationMode, Purpose, QueryShape,
    Subject, Tenant, Value,
};
use nirdosha_guard_mic::{EntityBytes, EvalRequest, GuardClient, Outcome};
use nirdosha_guard_store_postgres::PostgresStoreDriver;
use std::str::FromStr;

fn test_url() -> String {
    std::env::var("NIRDOSHA_TEST_POSTGRES_URL")
        .unwrap_or_else(|_| "postgres://postgres@127.0.0.1:5432/postgres".to_string())
}

fn context_for(tenant: &str, resource: &str) -> nirdosha_guard_core::EvaluationContext {
    nirdosha_guard_core::EvaluationContext {
        subject: Subject { id: "user-rls-test".into(), roles: vec!["analyst".into()], claims: vec![], clearance: Classification::Internal },
        tenant: Tenant(tenant.into()),
        entity: resource.into(),
        dataset: "postgres".into(),
        action: Action::Update,
        destination: Destination::Browser,
        environment: Environment { env: "test".into(), ip: None, geo: None, device_posture: None, session_freshness: None },
        time_bucket: "2026-09-20".into(),
        query_shape: QueryShape { verbs: vec![], aggregate: None, grouping_keys: vec![], subject_dimension: None, ordering: vec![], pagination: PaginationMode::LimitOnly { limit: 1 } },
        purpose: Purpose("aml_investigation".into()),
        policy_version: "v-rls-test".into(),
    }
}

fn tenant_scoped_policy(resource: &str, tenant: &str) -> PolicyCandidate {
    PolicyCandidate {
        id: format!("allow-{resource}"),
        effect: PolicyEffect::Allow,
        subjects: vec!["analyst".into()],
        action: Action::Update,
        resource: resource.into(),
        purpose: Some("aml_investigation".into()),
        conditions: vec![],
        filter: Some(FilterExpr::TenantEq { value: Value::Str(tenant.into()) }),
        obligations: vec![],
        escalation: None,
    }
}

/// Creates (idempotently) a real, non-superuser role and grants it
/// read-only access to `guard_entities` — the actual RLS assertion below
/// only means something against a role that isn't exempt from row
/// security the way a superuser or table owner would be.
fn ensure_probe_role(admin_url: &str) {
    let config = postgres::Config::from_str(admin_url).expect("parse admin url");
    let mut admin = config.connect(postgres::NoTls).expect("connect as admin for role setup");
    admin
        .batch_execute(
            "DO $$ BEGIN \
                IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'guard_probe') THEN \
                    CREATE ROLE guard_probe LOGIN PASSWORD 'guard_probe_test_password'; \
                END IF; \
             END $$; \
             GRANT SELECT ON guard_entities TO guard_probe;",
        )
        .expect("provision guard_probe role");
}

fn probe_client(admin_url: &str) -> postgres::Client {
    let mut config = postgres::Config::from_str(admin_url).expect("parse admin url");
    config.user("guard_probe").password("guard_probe_test_password");
    config.connect(postgres::NoTls).expect("connect as guard_probe")
}

#[test]
#[ignore]
fn guarded_apply_commits_to_real_postgres_and_rls_enforces_tenant_isolation() {
    let url = test_url();
    let driver = PostgresStoreDriver::connect(&url).expect("connect + provision schema");
    ensure_probe_role(&url);

    let temp_dir = std::env::temp_dir().join(format!("nirdosha-pg-driver-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    let mut client = GuardClient::new(
        vec![tenant_scoped_policy("rls_probe_row", "tenant-alpha")],
        "rls-test",
        temp_dir.join("audit.jsonl"),
    );

    let req = EvalRequest { context: context_for("tenant-alpha", "rls_probe_row") };
    let outcome = client
        .guarded_apply(&req, &driver, EntityBytes(b"alpha-payload".to_vec()), "trace-rls-1", 1_700_000_000)
        .expect("guarded_apply should commit under an allow policy");
    assert!(matches!(outcome, Outcome::Committed { .. }));

    // The audit chain still verifies against the real driver.
    assert_eq!(client.verify_audit(), Ok(3));

    // Real row landed with the right payload.
    let stored = driver.get("rls_probe_row").expect("read back via driver").expect("row exists");
    assert_eq!(stored, b"alpha-payload".to_vec());

    // RLS enforcement proof: a non-superuser role sees the row only when
    // `app.tenant` matches, and sees zero rows for a different tenant —
    // the actual `tenant_isolation` policy predicate, not just its DDL
    // existing.
    let mut probe = probe_client(&url);
    let mut probe_txn = probe.transaction().expect("start probe txn");
    probe_txn
        .execute("SELECT set_config('app.tenant', $1, true)", &[&"tenant-beta"])
        .expect("set wrong tenant");
    let wrong_tenant_rows = probe_txn
        .query("SELECT payload FROM guard_entities WHERE resource = $1", &[&"rls_probe_row"])
        .expect("query as guard_probe with wrong tenant");
    assert_eq!(wrong_tenant_rows.len(), 0, "RLS must hide the row from a different tenant's session");
    probe_txn.rollback().ok();

    let mut probe_txn = probe.transaction().expect("start probe txn 2");
    probe_txn
        .execute("SELECT set_config('app.tenant', $1, true)", &[&"tenant-alpha"])
        .expect("set correct tenant");
    let right_tenant_rows = probe_txn
        .query("SELECT payload FROM guard_entities WHERE resource = $1", &[&"rls_probe_row"])
        .expect("query as guard_probe with correct tenant");
    assert_eq!(right_tenant_rows.len(), 1, "RLS must reveal the row to its own tenant's session");
    probe_txn.rollback().ok();

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
#[ignore]
fn prepare_rejects_a_write_plan_with_no_tenant_scope() {
    let url = test_url();
    let driver = PostgresStoreDriver::connect(&url).expect("connect + provision schema");

    let temp_dir = std::env::temp_dir().join(format!("nirdosha-pg-driver-notenant-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    // A policy with no `filter` at all — the residual_filter is `None`,
    // which `prepare()` must reject rather than write ungoverned.
    let policy = PolicyCandidate {
        id: "allow-no-scope".into(),
        effect: PolicyEffect::Allow,
        subjects: vec!["analyst".into()],
        action: Action::Update,
        resource: "no_scope_row".into(),
        purpose: Some("aml_investigation".into()),
        conditions: vec![],
        filter: None,
        obligations: vec![],
        escalation: None,
    };
    let mut client = GuardClient::new(vec![policy], "rls-test-notenant", temp_dir.join("audit.jsonl"));
    let req = EvalRequest { context: context_for("tenant-alpha", "no_scope_row") };
    let result = client.guarded_apply(&req, &driver, EntityBytes(b"payload".to_vec()), "trace-rls-2", 1_700_000_001);
    assert!(result.is_err(), "a write plan with no tenant scope must be rejected, not committed");

    let _ = std::fs::remove_dir_all(&temp_dir);
}
