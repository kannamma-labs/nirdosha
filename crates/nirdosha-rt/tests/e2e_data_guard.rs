//! End-to-end integration proof for RFC 0026 Metadata Plane & RFC 0023 Data Guard.

use nirdosha_guard_mic::{EntityBytes, EvalRequest, GuardClient, MemStoreDriver, Outcome};
use nirdosha_guard_core::evaluator::{PolicyCandidate, PolicyEffect};
use nirdosha_guard_core::{Action, Classification, Destination, Environment, PaginationMode, Purpose, QueryShape, Subject, Tenant};

fn build_context() -> nirdosha_guard_core::EvaluationContext {
    nirdosha_guard_core::EvaluationContext {
        subject: Subject {
            id: "user-100".into(),
            roles: vec!["compliance_officer".into()],
            claims: vec![],
            clearance: Classification::Internal,
        },
        tenant: Tenant("tenant-alpha".into()),
        entity: "trade_records".into(),
        dataset: "memory".into(),
        action: Action::Read,
        destination: Destination::Browser,
        environment: Environment {
            env: "production".into(),
            ip: None,
            geo: None,
            device_posture: None,
            session_freshness: None,
        },
        time_bucket: "2026-09-19".into(),
        query_shape: QueryShape {
            verbs: vec![],
            aggregate: None,
            grouping_keys: vec![],
            subject_dimension: None,
            ordering: vec![],
            pagination: PaginationMode::LimitOnly { limit: 10 },
        },
        purpose: Purpose("compliance_audit".into()),
        policy_version: "v2026.09".into(),
    }
}

fn sample_policy() -> PolicyCandidate {
    PolicyCandidate {
        id: "policy-read-trades".into(),
        effect: PolicyEffect::Allow,
        subjects: vec!["compliance_officer".into()],
        action: Action::Read,
        resource: "trade_records".into(),
        purpose: Some("compliance_audit".into()),
        conditions: vec![],
        filter: None,
        obligations: vec![],
        escalation: None,
        caps: vec![],
        masks: vec![],
    }
}

#[test]
fn e2e_data_guard_and_metadata_plane_workflow() {
    let temp_dir = std::env::temp_dir().join(format!("nirdosha-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);

    let audit_file = temp_dir.join("audit_chain.jsonl");
    let mut client = GuardClient::new(vec![sample_policy()], "ingest@2.0", &audit_file);
    let driver = MemStoreDriver::new();
    let ctx = build_context();
    let req = EvalRequest { context: ctx };

    // 1. Guarded Apply (Commit payload with audit append & lineage observation)
    let outcome = client
        .guarded_apply(&req, &driver, EntityBytes(b"trade_data_payload".to_vec()), "trace-e2e-001", 1_700_000_000)
        .expect("guarded_apply should succeed under policy");

    assert!(matches!(outcome, Outcome::Committed { .. }));
    assert_eq!(driver.get("trade_records"), Some(b"trade_data_payload".to_vec()));

    // 2. Audit Chain Verification
    let audit_count = client.verify_audit().expect("audit chain must be intact and verified");
    assert_eq!(audit_count, 3); // Decision + Mutation + Lineage

    // 3. Verification Passes
    let reg_view = nirdosha_guard_verify::RegistryView {
        policies: vec![nirdosha_guard_verify::PolicyView {
            id: "policy-read-trades".into(),
            action: "read".into(),
            resource: "trade_records".into(),
            purpose: Some("compliance_audit".into()),
            effect: "allow".into(),
            ..Default::default()
        }],
        ports: vec![nirdosha_guard_verify::PortView { name: "store".into() }],
        drivers: vec![
            nirdosha_guard_verify::DriverView {
                port: "store".into(),
                vendor: "memory".into(),
                version: "1.0".into(),
                capabilities: vec![],
                lineage_support: "datasets".into(),
            },
            nirdosha_guard_verify::DriverView {
                port: "store".into(),
                vendor: "remote".into(),
                version: "1.0".into(),
                capabilities: vec![],
                lineage_support: "datasets".into(),
            },
            // Real Postgres StoreDriver (crates/nirdosha-guard-store-postgres):
            // pooled + TLS, RLS via session variables, honest FilterNodeKind
            // manifest. Registered here so V8's "≥2 drivers per port" check
            // reflects the platform's actual driver roster.
            nirdosha_guard_verify::DriverView {
                port: "store".into(),
                vendor: "postgres".into(),
                version: "1.0".into(),
                capabilities: vec![],
                lineage_support: "datasets".into(),
            },
        ],
    };
    let findings = nirdosha_guard_verify::verify(&reg_view);
    assert!(findings.is_empty(), "verification passes must find 0 errors for valid catalog");

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn full_rfc0023_data_guard_pipeline_workflow() {
    use nirdosha_guard_core::cedar::{CedarEffect, CedarFrontend, CedarPolicy};
    use nirdosha_guard_core::drivers::datafusion::DataFusionBridge;
    use nirdosha_guard_core::drivers::fga::OpenFgaResolver;
    use nirdosha_guard_core::drivers::rdbms::{DdlAst, RdbmsEmitter, SqlDialect};
    use nirdosha_guard_core::{
        BindingRouting, DatasetRegistryEntry, FieldMask, FilterExpr, MaskTransform,
        PolicyFrontend, RelationExpr, RelationResolver, Value, WriteAction, WritePlan,
    };
    use nirdosha_guard_federation::{
        BudgetCoordinator, CatalogRouter, MergeLayerReFilter,
    };
    use nirdosha_guard_mcp::GuardMcpServer;

    let ctx = build_context();

    // 1. Cedar Lowerable-Subset Policy Front-End (§1A)
    let cedar_frontend = CedarFrontend::new(vec![CedarPolicy {
        id: "cedar-allow-trade".into(),
        effect: CedarEffect::Permit,
        principal_condition: None,
        action_condition: None,
        resource_condition: None,
        when_clause: Some("status == \"active\"".into()),
    }]);
    let (cedar_decision, obligations) = cedar_frontend.evaluate(&ctx).expect("cedar evaluation");
    assert_eq!(cedar_decision, nirdosha_guard_core::Decision::Allow);
    assert_eq!(obligations.len(), 1);

    // 2. DataFusion & Arrow Physical Pushdown Bridge (§1A, §9.5)
    let filter_expr = FilterExpr::Eq {
        field: vec!["tenant_id".into()],
        value: Value::Str("tenant-alpha".into()),
    };
    let (pruning, row_filter) = DataFusionBridge::compile_filter(&filter_expr);
    assert!(pruning.expression.contains("tenant_id_min"));
    assert!(row_filter.filter_expr.contains("tenant_id = ?"));

    let batch_masks = DataFusionBridge::compute_batch_masks(
        &["id".to_string(), "card_num".to_string()],
        &[FieldMask {
            field: vec!["card_num".into()],
            transform: MaskTransform::PartialLast4,
        }],
    );
    assert_eq!(batch_masks[1].1, Some(MaskTransform::PartialLast4));

    // 3. RDBMS SQL Emitter & DDL AST Generator (§9.3, §9.5)
    let sql_plan = RdbmsEmitter::compile_plan(&filter_expr, SqlDialect::Postgres, &ctx.tenant);
    assert_eq!(sql_plan.where_clause, "\"tenant_id\" = $1");
    assert_eq!(sql_plan.rls_session_settings[0].1, "tenant-alpha");

    let ddl = DdlAst::CreateView {
        view_name: "ng_secured_trades".into(),
        target_table: "trade_records".into(),
        where_clause: sql_plan.where_clause.clone(),
    };
    assert!(ddl.render_ddl().contains("CREATE OR REPLACE VIEW \"ng_secured_trades\""));

    // 4. ReBAC Relation Resolver (§4)
    let fga = OpenFgaResolver::new("store-1", "model-1");
    let rel_expr = RelationExpr {
        name: "branches_under".into(),
        source: "fga".into(),
        max_cardinality: 50,
        ttl_seconds: 60,
    };
    let resolved = fga.resolve(&ctx.subject, &rel_expr).expect("fga resolve");
    assert_eq!(resolved.tier, nirdosha_guard_core::ResolutionTier::Tier1InList);

    // 5. Federated Read Engine & Budget Coordinator (§1C.1)
    let reg_entries = vec![DatasetRegistryEntry {
        entity: "trade_records".into(),
        dataset: "parquet_lake".into(),
        store: "parquet".into(),
        routing: BindingRouting {
            binding_id: "parquet_lake".into(),
            freshness_lag_seconds: 5,
            latency_class: "low".into(),
            cost_tier: "medium".into(),
            affinities: vec![],
            authoritative_for: vec![],
        },
        primary_write_binding: true,
    }];
    let selected = CatalogRouter::select_bindings("trade_records", &ctx.query_shape, 10, &reg_entries);
    assert_eq!(selected.len(), 1);

    let budget = BudgetCoordinator::new(100, 1000);
    assert!(budget.track_spend(10, 100).is_ok());

    let remasked = MergeLayerReFilter::refilter_and_remask(
        vec![vec![("ssn".into(), "000-00-0000".into())]],
        &None,
        &[FieldMask {
            field: vec!["ssn".into()],
            transform: MaskTransform::Full,
        }],
    );
    assert_eq!(remasked[0][0].1, "[MASKED]");

    // 6. Guarded MCP Agent Server (§1C.3)
    //
    // The MCP server only records the session hash when destination==LlmContext;
    // build a dedicated context for the agent call so submit_write can verify the
    // evaluate-then-act invariant.
    let mut mcp = GuardMcpServer::new();
    mcp.agent_writes_enabled = true;

    let token = GuardMcpServer::mint_token(
        ctx.subject.id.clone(),
        "agent-007",
        ctx.purpose.clone(),
        Destination::LlmContext,
        ctx.policy_version.clone(),
    );
    // Build an agent-specific context: destination must be LlmContext so that
    // evaluate() passes the destination guard and records the plan hash.
    let agent_ctx = nirdosha_guard_core::EvaluationContext {
        destination: Destination::LlmContext,
        ..ctx.clone()
    };
    let (_mcp_dec, _mcp_plan) = mcp.evaluate(&token, &agent_ctx);
    let plan_hash = "plan-hash-trade_records".to_string();

    let write_dec = mcp.submit_write(
        &token,
        plan_hash,
        WritePlan {
            decision: nirdosha_guard_core::Decision::Allow,
            action: WriteAction::Update,
            row_scope: None,
            preconditions: vec![],
            postconditions: vec![],
            field_policy: vec![],
            affected_row_cap: 1,
            obligations: vec![],
            policy_version: ctx.policy_version.clone(),
        },
    );
    assert_eq!(write_dec, nirdosha_guard_core::Decision::Allow);
}

