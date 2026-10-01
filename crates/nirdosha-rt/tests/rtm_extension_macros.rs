//! Real-invocation acceptance tests for the six macros
//! `examples/rtm/roles-N-guard_policy.md` calls but that didn't exist
//! anywhere in the workspace before this phase: `purpose!` (as a
//! function-like taxonomy macro — the pre-existing `#[purpose(...)]` is a
//! different, unrelated attribute macro), `stream_port!`, `window!`,
//! `model_artifact!`, `matcher!`, `mcp_tools!`. Same bar
//! `rtm_policy_corpus.rs`'s `corpus_policies_lower_to_real_structured_records`
//! holds `guard_policy!`/`approval_chain!` to: a real, structured record —
//! not just "the macro didn't error."
//!
//! `nirdosha_rt::purpose!` is the existing, unrelated attribute macro
//! (`#[nirdosha_rt::purpose(code = ..., basis = ..., review = ...)]` —
//! load-bearing, see `guard_attributes.rs`), so the new taxonomy macro is
//! called fully-qualified as `nirdosha_guard_macros::purpose_taxonomy!`,
//! exactly as `nirdosha_guard_macros::workflow!` already has to be for the
//! identical reason.

#![allow(non_upper_case_globals, non_camel_case_types)]

nirdosha_guard_macros::purpose_taxonomy! {
    enum Purpose {
        Operations,
        FraudMonitoring,
        AmlInvestigation,
    }
}

nirdosha_rt::stream_port! {
    port txn_in    { bind "card_network.rails";   semantics = at_least_once; }
    port txn_out   { publish "txn.authorized";    format = "avro"; schema = "transaction"; }
    port decisions { publish "guard.decisions";   format = "avro"; }
}

nirdosha_rt::window! {
    feature velocity_1h(subject_id) =
        sliding(1h, keys = [subject_id], aggs = [count, sum(amount)]);
    feature impossible_travel(subject_id) =
        session(30m, keys = [subject_id], expr = geo_speed(geo) > 900 km/h);
}

nirdosha_rt::model_artifact! {
    model rt_fraud_v1 {
        format = onnx;
        inputs = [velocity_1h, distinct_payees_7d, amount_dev_30d, impossible_travel];
        outputs = [score: f64, explanation: vec[string]];
        threshold_alert = 0.85;
    }
}

nirdosha_rt::matcher! {
    matcher sanctions {
        algorithm = fuzzy_jaro_winkler;
        threshold = 0.92;
        lists = [ofac_sdn, un_consolidated, eu_fsf];
    }
}

nirdosha_rt::mcp_tools! {
    server analyst_copilot {
        identity = "mcp:analyst-copilot";
        tools = [query_records(Transaction, Alert, Case, Customer),
                 get_options(AlertStatus, CaseStatus, TxnChannel, DispositionCode),
                 evaluate,
                 submit_write];
        delegation { bind user + agent; ttl = 30m; max_tool_calls = 60; rate = 20/min; }
        defaults {
            audit = full;
            row_cap = 50;
            destination = llm_context;
        }
    }
}

#[test]
fn purpose_taxonomy_registers_a_real_enum_and_one_record_per_variant() {
    // The macro must emit a real Rust enum, not just registry records —
    // this line only compiles if `Purpose::FraudMonitoring` really exists.
    let _ = Purpose::FraudMonitoring;

    let dump = nirdosha_guard_registry::dump();
    let codes: Vec<&str> = dump.purposes.iter().map(|p| p.code.as_str()).collect();
    assert!(codes.contains(&"Operations"));
    assert!(codes.contains(&"FraudMonitoring"));
    assert!(codes.contains(&"AmlInvestigation"));
}

#[test]
fn stream_port_registers_bind_and_publish_ports_with_real_fields() {
    let dump = nirdosha_guard_registry::dump();
    let find = |name: &str| dump.ports.iter().find(|p| p.name == name).unwrap_or_else(|| panic!("port `{name}` must be registered"));

    let txn_in = find("txn_in");
    assert_eq!(txn_in.direction, "bind");
    assert_eq!(txn_in.target, "card_network.rails");
    assert_eq!(txn_in.semantics.as_deref(), Some("at_least_once"));

    let txn_out = find("txn_out");
    assert_eq!(txn_out.direction, "publish");
    assert_eq!(txn_out.target, "txn.authorized");
    assert_eq!(txn_out.format.as_deref(), Some("avro"));
    assert_eq!(txn_out.schema.as_deref(), Some("transaction"));

    let decisions = find("decisions");
    assert_eq!(decisions.direction, "publish");
    assert_eq!(decisions.target, "guard.decisions");
    assert_eq!(decisions.schema, None);
}

#[test]
fn window_registers_name_key_kind_and_keeps_the_expr_as_spec() {
    let dump = nirdosha_guard_registry::dump();
    let find = |name: &str| dump.windows.iter().find(|w| w.name == name).unwrap_or_else(|| panic!("window `{name}` must be registered"));

    let velocity = find("velocity_1h");
    assert_eq!(velocity.key, "subject_id");
    assert_eq!(velocity.kind, "sliding");
    assert!(velocity.spec.contains("aggs=[count,sum(amount)]"), "spec was: {}", velocity.spec);

    let travel = find("impossible_travel");
    assert_eq!(travel.kind, "session");
    assert!(travel.spec.contains("geo_speed(geo)>900km/h"), "spec was: {}", travel.spec);
}

#[test]
fn model_artifact_registers_inputs_outputs_and_threshold() {
    let dump = nirdosha_guard_registry::dump();
    let model = dump.models.iter().find(|m| m.name == "rt_fraud_v1").expect("rt_fraud_v1 must be registered");
    assert_eq!(model.format, "onnx");
    assert_eq!(model.inputs, vec!["velocity_1h", "distinct_payees_7d", "amount_dev_30d", "impossible_travel"]);
    assert_eq!(model.outputs.len(), 2);
    assert_eq!(model.threshold_alert, Some(0.85));
}

#[test]
fn matcher_registers_algorithm_threshold_and_lists() {
    let dump = nirdosha_guard_registry::dump();
    let sanctions = dump.matchers.iter().find(|m| m.name == "sanctions").expect("sanctions matcher must be registered");
    assert_eq!(sanctions.algorithm, "fuzzy_jaro_winkler");
    assert_eq!(sanctions.threshold, 0.92);
    assert_eq!(sanctions.lists, vec!["ofac_sdn", "un_consolidated", "eu_fsf"]);
}

#[test]
fn mcp_tools_registers_identity_tools_delegation_and_defaults() {
    let dump = nirdosha_guard_registry::dump();
    let copilot = dump.mcp_servers.iter().find(|s| s.name == "analyst_copilot").expect("analyst_copilot must be registered");
    assert_eq!(copilot.identity, "mcp:analyst-copilot");
    assert_eq!(copilot.tools.len(), 4);
    assert!(copilot.tools.iter().any(|t| t.starts_with("query_records(")));
    assert!(copilot.tools.contains(&"evaluate".to_string()));
    assert_eq!(copilot.ttl.as_deref(), Some("30m"));
    assert_eq!(copilot.max_tool_calls, Some(60));
    assert_eq!(copilot.rate.as_deref(), Some("20/min"));
    assert_eq!(copilot.audit_default.as_deref(), Some("full"));
    assert_eq!(copilot.row_cap_default, Some(50));
    assert_eq!(copilot.destination_default.as_deref(), Some("llm_context"));
}
