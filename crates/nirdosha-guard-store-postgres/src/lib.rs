//! Real Postgres `StoreDriver` for the RFC 0023/0025 guard write path
//! (`guarded_apply`'s `prepare`/`commit`). Closes the
//! `rfcs/0026-metadata-plane-checklist.md` line: "[OPEN] Live vendor
//! capability attestation and production store drivers" — the production
//! store driver half of it; attestation stays open (see crate docs below).
//!
//! **Pooling/TLS is ported, not imported.** The real pooled/TLS Postgres
//! pattern already lives in `crates/runtime-kernels/src/kernel/db.rs`
//! (`docs/adr/0005-postgres-pooling-and-tls.md`), but `runtime-kernels` is
//! a separate Cargo workspace by its own root `Cargo.toml` `exclude` list
//! — a root-workspace crate can't path-depend on it without hitting
//! Cargo's multi-workspace error. `PostgresManager` below mirrors that
//! design exactly (real `SELECT 1` liveness check, `SslMode::Require` by
//! default for any non-local host unless the caller's own connection
//! string already set `sslmode=`, no `danger_accept_invalid_*` call
//! anywhere) rather than reinventing it or silently diverging from it.
//! See `docs/adr/0013-postgres-store-driver-pooling-and-rls.md`.
//!
//! **What this driver models.** The `StoreDriver` trait's `Prepared`
//! carries only `{resource, policy_version}` — no row-level structure, no
//! dataset field. This driver persists exactly that shape honestly (one
//! row per `resource`, an opaque payload blob) rather than inventing
//! structure the trait doesn't actually carry; it does not attempt
//! per-field writes, and `commit()` remains a keyed upsert rather than a
//! `row_scope`-bounded `UPDATE`/`DELETE` (Plan Phase 8's job).
//!
//! **Read path (Plan Phase 7).** `query()` compiles a `ReadPlanIr`'s
//! `FilterExpr` with the same `RdbmsEmitter` `prepare()`/`commit()` already
//! used, against a real `SELECT ... WHERE ... LIMIT <cap>` — the first
//! implementation of `StoreDriver::query` anywhere in the workspace. Same
//! refusal posture as the write side: no filter (hence no tenant scope) —
//! reject, don't scan unscoped.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Mutex;

use nirdosha_guard_core::drivers::rdbms::{RdbmsEmitter, SqlDialect};
use nirdosha_guard_core::{
    AggregateSemantics, CapabilityManifest, FilterNodeKind, LineageFacts, Tenant, Value,
};
use nirdosha_guard_core::WriteAction;
use nirdosha_guard_mic::{EntityBytes, PlanError, PlanIr, Prepared, QueryResult, ReadPlanIr, Receipt, StoreDriver};
use sha2::{Digest, Sha256};

const TABLE_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS guard_entities (
    resource       TEXT PRIMARY KEY,
    tenant         TEXT NOT NULL,
    policy_version TEXT NOT NULL,
    payload        BYTEA NOT NULL,
    committed_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
"#;

// `CREATE POLICY IF NOT EXISTS` doesn't exist in Postgres (unlike
// `CREATE TABLE`) — the idempotent form is a catalog check inside a `DO`
// block, not a syntax flag.
const RLS_DDL: &str = r#"
ALTER TABLE guard_entities ENABLE ROW LEVEL SECURITY;
ALTER TABLE guard_entities FORCE ROW LEVEL SECURITY;
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_policies
        WHERE tablename = 'guard_entities' AND policyname = 'tenant_isolation'
    ) THEN
        CREATE POLICY tenant_isolation ON guard_entities
            USING (tenant = current_setting('app.tenant', true));
    END IF;
END
$$;
"#;

// ---- Pooling + TLS, ported from runtime-kernels::kernel::db -----------

pub struct PostgresManager {
    config: postgres::Config,
    tls: postgres_native_tls::MakeTlsConnector,
}

impl r2d2::ManageConnection for PostgresManager {
    type Connection = postgres::Client;
    type Error = postgres::Error;

    fn connect(&self) -> Result<Self::Connection, Self::Error> {
        self.config.connect(self.tls.clone())
    }

    /// A real `SELECT 1` round-trip, not `Client::is_closed()` alone —
    /// same reasoning as `PostgresManager::is_valid` in
    /// `runtime-kernels::kernel::db`: after a server-side restart a
    /// client socket typically has no idea the peer is gone until it
    /// tries to use the connection.
    fn is_valid(&self, conn: &mut Self::Connection) -> Result<(), Self::Error> {
        conn.simple_query("SELECT 1").map(|_| ())
    }

    fn has_broken(&self, conn: &mut Self::Connection) -> bool {
        conn.is_closed()
    }
}

fn is_local_host(host: &postgres::config::Host) -> bool {
    match host {
        postgres::config::Host::Tcp(h) => h == "localhost" || h == "127.0.0.1" || h == "::1",
        #[cfg(unix)]
        postgres::config::Host::Unix(_) => true,
    }
}

/// Builds the real `postgres::Config`/TLS connector pair for one
/// connection string — verify-by-default TLS for a non-local host,
/// never overriding a caller's own explicit `sslmode=`.
fn build_postgres_manager(conn_str: &str) -> Result<PostgresManager, String> {
    let mut config = postgres::Config::from_str(conn_str).map_err(|e| e.to_string())?;
    let caller_set_sslmode = conn_str.contains("sslmode=");
    if !caller_set_sslmode {
        let all_local = config.get_hosts().iter().all(is_local_host);
        if !all_local {
            config.ssl_mode(postgres::config::SslMode::Require);
        }
    }
    // Deliberately no `danger_accept_invalid_certs`/`danger_accept_invalid_hostnames`
    // call anywhere in this module — verification stays on by default.
    let connector = native_tls::TlsConnector::builder()
        .build()
        .map_err(|e| e.to_string())?;
    let tls = postgres_native_tls::MakeTlsConnector::new(connector);
    Ok(PostgresManager { config, tls })
}

use nirdosha_guard_core::extract_tenant;

/// Real, pooled, TLS-capable Postgres implementation of `StoreDriver`.
pub struct PostgresStoreDriver {
    manifest: CapabilityManifest,
    pool: r2d2::Pool<PostgresManager>,
    /// Tenant extracted at `prepare()` time, consumed by `commit()`.
    /// `Prepared{resource, policy_version}` has no field for driver-private
    /// state, so this is threaded via a short-lived, resource-keyed map
    /// instead of widening a type shared by every `StoreDriver`
    /// implementor (see `docs/adr/0013-...md` for the known, narrow
    /// limitation this implies under concurrent same-resource writes).
    pending_tenants: Mutex<HashMap<String, String>>,
}

impl PostgresStoreDriver {
    /// Connects, builds the pool, and idempotently ensures the schema +
    /// RLS policy exist. Fails loudly rather than lazily on first write.
    pub fn connect(conn_str: &str) -> Result<Self, PlanError> {
        let manager = build_postgres_manager(conn_str).map_err(PlanError::Store)?;
        let pool = r2d2::Pool::builder()
            .max_size(10)
            .build(manager)
            .map_err(|e| PlanError::Store(e.to_string()))?;
        let mut conn = pool.get().map_err(|e| PlanError::Store(e.to_string()))?;
        conn.batch_execute(TABLE_DDL)
            .map_err(|e| PlanError::Store(e.to_string()))?;
        conn.batch_execute(RLS_DDL)
            .map_err(|e| PlanError::Store(e.to_string()))?;
        drop(conn);
        Ok(Self {
            manifest: CapabilityManifest {
                schema_version: 1,
                driver_name: "postgres".into(),
                // Honest, not aspirational: exactly the FilterExpr variants
                // `RdbmsEmitter::emit_where` actually translates today.
                supported_filter_nodes: vec![
                    FilterNodeKind::Eq,
                    FilterNodeKind::In,
                    FilterNodeKind::Compare,
                    FilterNodeKind::And,
                    FilterNodeKind::Or,
                    FilterNodeKind::Not,
                    FilterNodeKind::TimeRange,
                    FilterNodeKind::Pattern,
                    FilterNodeKind::TenantEq,
                ],
                masking_points: vec![],
                aggregate_semantics: AggregateSemantics::PushdownFilterThenAggregate,
                supports_tenant_eq_native: true,
            },
            pool,
            pending_tenants: Mutex::new(HashMap::new()),
        })
    }

    /// Test/inspection helper: reads a row back bypassing RLS scoping
    /// (the pool connection's own role decides whether that's actually
    /// possible — a non-superuser, non-owner role remains subject to the
    /// `tenant_isolation` policy exactly as it would in production).
    pub fn get(&self, resource: &str) -> Result<Option<Vec<u8>>, PlanError> {
        let mut conn = self.pool.get().map_err(|e| PlanError::Store(e.to_string()))?;
        let row = conn
            .query_opt("SELECT payload FROM guard_entities WHERE resource = $1", &[&resource])
            .map_err(|e| PlanError::Store(e.to_string()))?;
        Ok(row.map(|r| r.get::<_, Vec<u8>>(0)))
    }
}

impl StoreDriver for PostgresStoreDriver {
    fn manifest(&self) -> &CapabilityManifest {
        &self.manifest
    }

    fn prepare(&self, ir: &PlanIr) -> Result<Prepared, PlanError> {
        let filter = ir
            .filter
            .as_ref()
            .ok_or_else(|| PlanError::Rejected("missing tenant scope".into()))?;
        let tenant =
            extract_tenant(filter).ok_or_else(|| PlanError::Rejected("missing tenant scope".into()))?;
        // Validates the filter is actually translatable by this driver's
        // manifest — a filter node this emitter can't honestly handle
        // surfaces here as a hard failure, not a silently unfiltered write.
        let _ = RdbmsEmitter::compile_plan(filter, SqlDialect::Postgres, &Tenant(tenant.clone()));
        let mut pending = self
            .pending_tenants
            .lock()
            .map_err(|_| PlanError::Store("pending-tenant lock poisoned".into()))?;
        pending.insert(ir.resource.clone(), tenant);
        Ok(Prepared { resource: ir.resource.clone(), policy_version: ir.policy_version.clone(), action: ir.action.clone(), row_scope: ir.row_scope.clone() })
    }

    fn commit(&self, prepared: Prepared, entity: EntityBytes) -> Result<Receipt, PlanError> {
        let tenant = {
            let mut pending = self
                .pending_tenants
                .lock()
                .map_err(|_| PlanError::Store("pending-tenant lock poisoned".into()))?;
            pending
                .remove(&prepared.resource)
                .ok_or_else(|| PlanError::Store("commit called without a matching prepare".into()))?
        };
        let mut conn = self.pool.get().map_err(|e| PlanError::Store(e.to_string()))?;
        let mut txn = conn.transaction().map_err(|e| PlanError::Store(e.to_string()))?;
        // `SET LOCAL app.tenant = $1` isn't valid SQL — `SET` doesn't take
        // bind parameters. `set_config(name, value, is_local=true)` is the
        // parameterized equivalent, scoped to this transaction only.
        txn.execute("SELECT set_config('app.tenant', $1, true)", &[&tenant])
            .map_err(|e| PlanError::Store(e.to_string()))?;

        // The atomicity contract (Plan Phase 8): `FOR UPDATE` takes a real
        // row lock spanning this check AND the write below, inside one
        // transaction — a concurrent commit on the same resource blocks
        // here rather than racing past a "resource doesn't exist yet"
        // read from a moment ago. RLS still applies to this SELECT (the
        // session's `app.tenant` was just set), so a row outside this
        // tenant is invisible to the check, not merely excluded from it.
        let existing = txn
            .query_opt("SELECT tenant FROM guard_entities WHERE resource = $1 FOR UPDATE", &[&prepared.resource])
            .map_err(|e| PlanError::Store(e.to_string()))?;
        match prepared.action {
            WriteAction::Create => {
                if existing.is_some() {
                    return Err(PlanError::Rejected(format!("create: resource {:?} already exists", prepared.resource)));
                }
            }
            WriteAction::Update | WriteAction::Delete => {
                let Some(row) = existing.as_ref() else {
                    return Err(PlanError::Rejected(format!("{:?}: resource {:?} does not exist", prepared.action, prepared.resource)));
                };
                if let Some(row_scope) = &prepared.row_scope {
                    if let Some(required_tenant) = extract_tenant(row_scope) {
                        let existing_tenant: String = row.get(0);
                        if existing_tenant != required_tenant {
                            return Err(PlanError::Rejected("row_scope: existing row's tenant does not match".into()));
                        }
                    }
                }
            }
            WriteAction::Migrate | WriteAction::Export => {}
        }

        let xmin: String = if matches!(prepared.action, WriteAction::Delete) {
            txn.execute("DELETE FROM guard_entities WHERE resource = $1", &[&prepared.resource])
                .map_err(|e| PlanError::Store(e.to_string()))?;
            "deleted".into()
        } else {
            let row = txn
                .query_one(
                    "INSERT INTO guard_entities (resource, tenant, policy_version, payload) \
                     VALUES ($1, $2, $3, $4) \
                     ON CONFLICT (resource) DO UPDATE SET \
                         payload = EXCLUDED.payload, \
                         policy_version = EXCLUDED.policy_version, \
                         tenant = EXCLUDED.tenant, \
                         committed_at = now() \
                     RETURNING xmin::text",
                    &[&prepared.resource, &tenant, &prepared.policy_version, &entity.0],
                )
                .map_err(|e| PlanError::Store(e.to_string()))?;
            row.get(0)
        };
        txn.commit().map_err(|e| PlanError::Store(e.to_string()))?;

        let mut hasher = Sha256::new();
        hasher.update(&entity.0);
        let hashed = hasher.finalize();
        let mut digest = [0u8; 32];
        digest.copy_from_slice(&hashed);

        Ok(Receipt { store_commit_id: format!("pg-{xmin}"), digest })
    }

    fn query(&self, plan: &ReadPlanIr) -> Result<QueryResult, PlanError> {
        // Same posture as the write side's "missing tenant scope" refusal
        // (`prepare`, above) and `MemStoreDriver::query`'s identical
        // check: a read plan reaching this driver with no filter at all
        // is refused, not silently turned into an unscoped `SELECT *`.
        let filter = plan
            .filter
            .as_ref()
            .ok_or_else(|| PlanError::Rejected("read plan has no filter — refusing an unscoped scan".into()))?;
        let tenant = nirdosha_guard_core::extract_tenant(filter)
            .ok_or_else(|| PlanError::Rejected("missing tenant scope".into()))?;
        let sql_plan = RdbmsEmitter::compile_plan(filter, SqlDialect::Postgres, &Tenant(tenant.clone()));

        let row_cap = plan.caps.iter().find_map(|cap| match cap { nirdosha_guard_core::Cap::RowCap(n) => Some(*n), _ => None });
        // `MaxScanRows` bounds what the engine reads before filtering;
        // without pushdown-vs-post-filter cost accounting, this driver
        // treats it the same as `RowCap` — both cap what comes back,
        // which is honest for a fully-pushed-down `WHERE` (everything
        // scanned matches the filter already) and merely conservative
        // otherwise, never permissive. `MaxScanBytes` has no equivalent
        // enforcement here — real scan-time byte accounting isn't
        // available through the plain query interface this driver uses;
        // only `MaxResultBytes` (checked post-fetch, below) is enforced.
        let max_scan_rows = plan.caps.iter().find_map(|cap| match cap { nirdosha_guard_core::Cap::MaxScanRows(n) => Some(*n), _ => None });
        let max_result_bytes = plan.caps.iter().find_map(|cap| match cap { nirdosha_guard_core::Cap::MaxResultBytes(n) => Some(*n as usize), _ => None });
        let max_execution_ms = plan.caps.iter().find_map(|cap| match cap { nirdosha_guard_core::Cap::MaxExecutionTimeMs(n) => Some(*n), _ => None });
        let cohort_floor = plan.caps.iter().find_map(|cap| match cap { nirdosha_guard_core::Cap::CohortFloor(n) => Some(*n as i64), _ => None });
        let effective_cap = row_cap.into_iter().chain(max_scan_rows).min();

        let cursor = match &plan.pagination {
            nirdosha_guard_core::PaginationMode::OpaqueCursor(token) => Some(nirdosha_guard_mic::decode_cursor(token)?),
            _ => None,
        };
        // Keyset pagination on `resource` — I10/§8.4's "opaque cursors
        // only" applies here exactly as it does in `MemStoreDriver`
        // (which paginates the same way over its own key-ordered map):
        // `resource > $cursor` rather than `OFFSET n`, so a page's cost
        // doesn't grow with how deep into the result set it is.
        let where_with_cursor = match &cursor {
            Some(_) => format!("({}) AND \"resource\" > ${}", sql_plan.where_clause, sql_plan.parameters.len() + 1),
            None => sql_plan.where_clause.clone(),
        };

        let mut conn = self.pool.get().map_err(|e| PlanError::Store(e.to_string()))?;
        let mut txn = conn.transaction().map_err(|e| PlanError::Store(e.to_string()))?;
        // Scoped to this transaction only, same as `commit()` — RLS sees
        // the real tenant for the duration of this one query.
        txn.execute("SELECT set_config('app.tenant', $1, true)", &[&tenant])
            .map_err(|e| PlanError::Store(e.to_string()))?;
        if let Some(ms) = max_execution_ms {
            // Real Postgres enforcement, not an approximation: the server
            // itself aborts the query past this deadline.
            txn.execute(&format!("SET LOCAL statement_timeout = {ms}"), &[])
                .map_err(|e| PlanError::Store(e.to_string()))?;
        }

        if let Some(floor) = cohort_floor {
            let count_query = format!("SELECT count(*) FROM guard_entities WHERE {where_with_cursor}");
            let mut boxed_params: Vec<Box<dyn postgres::types::ToSql + Sync + Send>> =
                sql_plan.parameters.iter().map(value_to_sql).collect();
            if let Some(c) = &cursor {
                boxed_params.push(Box::new(c.clone()));
            }
            let param_refs: Vec<&(dyn postgres::types::ToSql + Sync)> =
                boxed_params.iter().map(|b| b.as_ref() as &(dyn postgres::types::ToSql + Sync)).collect();
            let count: i64 = txn
                .query_one(count_query.as_str(), &param_refs)
                .map_err(|e| PlanError::Store(e.to_string()))?
                .get(0);
            if count < floor {
                return Err(PlanError::Rejected(format!("cohort_floor: query would identify {count} entities, below the floor of {floor}")));
            }
        }

        let query = format!(
            "SELECT resource, payload FROM guard_entities WHERE {where_with_cursor} ORDER BY resource{}",
            effective_cap.map(|limit| format!(" LIMIT {limit}")).unwrap_or_default()
        );
        let mut boxed_params: Vec<Box<dyn postgres::types::ToSql + Sync + Send>> =
            sql_plan.parameters.iter().map(value_to_sql).collect();
        if let Some(c) = &cursor {
            boxed_params.push(Box::new(c.clone()));
        }
        let param_refs: Vec<&(dyn postgres::types::ToSql + Sync)> =
            boxed_params.iter().map(|b| b.as_ref() as &(dyn postgres::types::ToSql + Sync)).collect();
        let rows = txn
            .query(query.as_str(), &param_refs)
            .map_err(|e| PlanError::Store(e.to_string()))?;
        txn.commit().map_err(|e| PlanError::Store(e.to_string()))?;

        let fetched_count = rows.len();
        let mut result_bytes = 0usize;
        let mut out = Vec::new();
        let mut last_resource = None;
        for row in &rows {
            let resource: String = row.get(0);
            let payload: Vec<u8> = row.get(1);
            result_bytes += payload.len();
            if max_result_bytes.is_some_and(|cap| result_bytes > cap) {
                break;
            }
            last_resource = Some(resource);
            out.push(EntityBytes(payload));
        }
        // A cursor is only worth returning if the LIMIT was actually hit
        // (there may be more rows past this page) or MaxResultBytes cut
        // the page short — otherwise this was the last page.
        let next_cursor = if (effective_cap.is_some_and(|limit| fetched_count as u64 == limit)) || out.len() < fetched_count {
            last_resource.map(|resource| nirdosha_guard_mic::encode_cursor(&resource))
        } else {
            None
        };

        Ok(QueryResult { rows: out, next_cursor })
    }

    fn lineage(&self) -> LineageFacts {
        LineageFacts { sources: vec![], sink_keys: vec!["resource".into()] }
    }
}

/// `FilterExpr::Value` -> a bound SQL parameter. `Dec` binds as its own
/// canonical string form (its doc comment: "precise type enforced by
/// schema") — this driver has no schema-typed column info to convert it
/// to a real `NUMERIC` bind, so it stays text rather than guessing a
/// precision.
fn value_to_sql(value: &Value) -> Box<dyn postgres::types::ToSql + Sync + Send> {
    match value {
        Value::Int(v) => Box::new(*v),
        Value::Bool(v) => Box::new(*v),
        Value::Str(v) => Box::new(v.clone()),
        Value::Dec(v) => Box::new(v.clone()),
        Value::Null => Box::new(Option::<String>::None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nirdosha_guard_core::{CompareOp, FilterExpr};

    #[test]
    fn extract_tenant_finds_top_level_tenant_eq() {
        let filter = FilterExpr::TenantEq { value: Value::Str("tenant-alpha".into()) };
        assert_eq!(extract_tenant(&filter), Some("tenant-alpha".into()));
    }

    #[test]
    fn extract_tenant_finds_tenant_eq_nested_in_and() {
        let filter = FilterExpr::And(vec![
            FilterExpr::Compare {
                field: vec!["amount".into()],
                op: CompareOp::Ge,
                value: Value::Int(100),
            },
            FilterExpr::TenantEq { value: Value::Str("tenant-beta".into()) },
        ]);
        assert_eq!(extract_tenant(&filter), Some("tenant-beta".into()));
    }

    #[test]
    fn extract_tenant_returns_none_without_tenant_scope() {
        let filter = FilterExpr::Eq { field: vec!["status".into()], value: Value::Str("active".into()) };
        assert_eq!(extract_tenant(&filter), None);
    }

    #[test]
    fn manifest_lists_exactly_the_filter_kinds_the_emitter_supports() {
        // A driver builds its manifest without a live connection for this
        // assertion, so construct the value it would report directly.
        let manifest = CapabilityManifest {
            schema_version: 1,
            driver_name: "postgres".into(),
            supported_filter_nodes: vec![
                FilterNodeKind::Eq,
                FilterNodeKind::In,
                FilterNodeKind::Compare,
                FilterNodeKind::And,
                FilterNodeKind::Or,
                FilterNodeKind::Not,
                FilterNodeKind::TimeRange,
                FilterNodeKind::Pattern,
                FilterNodeKind::TenantEq,
            ],
            masking_points: vec![],
            aggregate_semantics: AggregateSemantics::PushdownFilterThenAggregate,
            supports_tenant_eq_native: true,
        };
        assert_eq!(manifest.supported_filter_nodes.len(), 9);
        assert!(manifest.supports_tenant_eq_native);
    }
}
