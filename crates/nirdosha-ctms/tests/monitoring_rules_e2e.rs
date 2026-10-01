//! Acceptance test for the Monitoring Rules control plane (Task 1.6):
//!
//! 1. A RiskAnalyst cannot activate a rule without approval.
//! 2. An authorized approver can approve it.
//! 3. The active rule is visible to realtime processing.
//! 4. The same rule (the same `RuleEngine`) is used by batch processing.
//! 5. A matching transaction creates an alert.
//! 6. A replay does not create a duplicate alert.
//! 7. Every mutation writes evidence.
//! 8. An old rule version remains auditable.

use nirdosha_ctms::alert::{AlertStatus, AlertStore, CtmsAlertGatewayV1, InMemoryAlertStore};
use nirdosha_ctms::event::{Direction, EventType, TransactionEvent};
use nirdosha_ctms::policy::RuleWorkflowPolicy;
use nirdosha_ctms::rule_engine::RuleEngine;
use nirdosha_ctms::rule_gateway::{CtmsRuleApproveGatewayV1, CtmsRuleCreateGatewayV1, CtmsRuleEnableGatewayV1, CtmsRuleSubmitGatewayV1};
use nirdosha_ctms::rule_model::{InMemoryRuleStore, RuleDefinition, RuleStatus, RuleStore, RuleType};
use nirdosha_ctms::rules::AlertCandidate;
use nirdosha_ctms::service::{create_rule, enable_rule, submit_rule_for_approval, CtmsServiceError};
use nirdosha_guard_rfc0029::{CapabilityIssuer, EffectGateway, PolicyBundle};
use nirdosha_rt::Auth;

const VALID_BUNDLE: &str = r#"
[bundle]
authority_id = "ctms-domain-authority"
policy_owner = "risk-platform-team"
jurisdiction = "IN"
approved_by = "ctms-domain-authority"
signed_by = "ctms-domain-authority"
effective_from = "2026-01-01T00:00:00Z"
expires_at = "2027-01-01T00:00:00Z"
"#;

fn now_ms() -> u64 {
    1_780_000_000_000
}

fn issuer() -> CapabilityIssuer {
    let bundle = PolicyBundle::from_toml_str(VALID_BUNDLE).unwrap();
    CapabilityIssuer::new(bundle, 5 * 60 * 1000)
}

fn draft_rule() -> RuleDefinition {
    RuleDefinition {
        rule_id: "velocity_24h".to_string(),
        version: 1,
        name: "Velocity 24h".to_string(),
        description: "5 debits, 500000 minor, 24h".to_string(),
        rule_type: RuleType::Velocity { window_ms: 86_400_000, min_count: 5, min_total_minor: 500_000 },
        currency: "INR".to_string(),
        jurisdiction: "IN".to_string(),
        severity: "high".to_string(),
        effective_from_ms: 0,
        effective_until_ms: None,
        status: RuleStatus::Draft,
        author: "priya".to_string(),
        approved_by: None,
        created_at_ms: 0,
    }
}

fn debit_event(event_id: &str, event_time_ms: u64) -> TransactionEvent {
    TransactionEvent {
        event_id: event_id.to_string(),
        transaction_id: format!("txn-{event_id}"),
        event_type: EventType::CardDebit,
        customer_id: "cust-1".to_string(),
        account_id: "acct-1".to_string(),
        direction: Direction::Debit,
        amount_minor: 200_000,
        currency: "INR".to_string(),
        event_time_ms,
        source_system: "card-network-x".to_string(),
        source_version: "v1".to_string(),
        correction_of: None,
        jurisdiction: "IN".to_string(),
        ingestion_time_ms: 0,
        partition: 0,
        offset: 0,
    }
}

fn evidence_decisions(chain: &nirdosha_audit::envelope::ModuleAuditChain) -> Vec<(String, String)> {
    chain
        .entries()
        .into_iter()
        .map(|entry| {
            let value = serde_json::to_value(&entry).unwrap();
            (value["content"]["action"].as_str().unwrap_or_default().to_string(), value["content"]["decision"].as_str().unwrap_or_default().to_string())
        })
        .collect()
}

#[test]
fn monitoring_rules_control_plane_end_to_end() {
    let issuer = issuer();
    let now = now_ms();
    let policy = RuleWorkflowPolicy::default_v1();
    let rule_store = InMemoryRuleStore::new();

    let create_evidence = std::env::temp_dir().join(format!("ctms_rules_e2e_create_{}", std::process::id()));
    let create_gateway = CtmsRuleCreateGatewayV1::new(&create_evidence, &issuer);
    let submit_gateway = CtmsRuleSubmitGatewayV1::new(&create_evidence, &issuer);
    let approve_evidence = std::env::temp_dir().join(format!("ctms_rules_e2e_approve_{}", std::process::id()));
    let approve_gateway = CtmsRuleApproveGatewayV1::new(&approve_evidence, &issuer);
    let enable_evidence = std::env::temp_dir().join(format!("ctms_rules_e2e_enable_{}", std::process::id()));
    let enable_gateway = CtmsRuleEnableGatewayV1::new(&enable_evidence, &issuer);

    let rule_author = Auth::login("priya", &["RuleAuthor"]);
    let rule_approver = Auth::login("arjun", &["RuleApprover"]);
    let risk_analyst = Auth::login("kavya", &["RiskAnalyst"]);

    // A RuleAuthor drafts v1 and submits it for approval.
    let created = create_rule(&issuer, &create_gateway, &rule_store, &policy, &rule_author, draft_rule(), now).expect("RuleAuthor may draft a rule");
    assert_eq!(created.status, RuleStatus::Draft);
    submit_rule_for_approval(&issuer, &submit_gateway, &rule_store, &policy, &rule_author, "velocity_24h", 1, now).unwrap();

    // 1. A RiskAnalyst cannot activate a rule without approval -- rejected
    // on role alone, before the (also-true) WrongState check is even
    // reached.
    let unauthorized = enable_rule(&issuer, &enable_gateway, &rule_store, &policy, &risk_analyst, "velocity_24h", 1, now);
    assert!(matches!(unauthorized, Err(CtmsServiceError::Forbidden { ref required_role }) if required_role == "RuleApprover"), "{unauthorized:?}");
    assert_eq!(rule_store.get("velocity_24h", 1).unwrap().status, RuleStatus::PendingApproval, "the rejected attempt must not have moved the rule");

    // 2. An authorized approver can approve it.
    let approve_capability = issuer.mint(&rule_approver, CtmsRuleApproveGatewayV1::RESOURCE, CtmsRuleApproveGatewayV1::EFFECT, now).unwrap();
    let approved = approve_gateway.approve(&approve_capability, &rule_store, "velocity_24h", 1, "arjun", now).expect("an authorized approver may approve");
    assert_eq!(approved.status, RuleStatus::Approved);

    // Now a RuleApprover (the right role) can enable it.
    let active = enable_rule(&issuer, &enable_gateway, &rule_store, &policy, &rule_approver, "velocity_24h", 1, now).expect("RuleApprover may enable an approved rule");
    assert_eq!(active.status, RuleStatus::Enabled);

    // 3. The active rule is visible to realtime processing.
    let realtime_active = rule_store.active_snapshot("velocity_24h").expect("realtime consults active_snapshot");
    assert_eq!(realtime_active.version, 1);
    let engine = RuleEngine::new();
    let mut realtime_match = None;
    for i in 0..5 {
        realtime_match = engine.evaluate(&realtime_active, &debit_event(&format!("evt-{i}"), 1_000 * i), now);
    }
    let realtime_match = realtime_match.expect("5 debits totalling 1,000,000 minor must cross the threshold in realtime processing");
    assert_eq!(realtime_match.rule_id, "velocity_24h");
    assert_eq!(realtime_match.rule_version, 1);

    // 4. The same rule (the same RuleEngine, the same active snapshot) is
    // used by batch processing -- a fresh RuleEngine instance (simulating
    // a separate batch job) replaying the identical events against the
    // identical `RuleDefinition` fetched from the same `active_snapshot`
    // produces the same match.
    let batch_active = rule_store.active_snapshot("velocity_24h").expect("batch consults the same active_snapshot");
    assert_eq!(batch_active, realtime_active, "realtime and batch must evaluate the exact same governed rule snapshot");
    let batch_engine = RuleEngine::new();
    let mut batch_match = None;
    for i in 0..5 {
        batch_match = batch_engine.evaluate(&batch_active, &debit_event(&format!("evt-{i}"), 1_000 * i), now);
    }
    let batch_match = batch_match.expect("batch processing must also match using the same rule");
    assert_eq!(batch_match.rule_id, realtime_match.rule_id);
    assert_eq!(batch_match.matched_event_ids, realtime_match.matched_event_ids);

    // 5. A matching transaction creates an alert.
    let alert_store = InMemoryAlertStore::new();
    let alert_evidence = std::env::temp_dir().join(format!("ctms_rules_e2e_alert_{}", std::process::id()));
    let alert_gateway = CtmsAlertGatewayV1::new(&alert_evidence, &issuer);
    let candidate = AlertCandidate {
        rule_id: realtime_match.rule_id.clone(),
        rule_version: realtime_match.rule_version,
        key: "cust-1".to_string(),
        window_start_ms: 0,
        window_end_ms: realtime_match.evaluated_at_ms,
        severity: realtime_match.severity.clone(),
        matched_event_ids: realtime_match.matched_event_ids.clone(),
        debit_count: realtime_match.matched_event_ids.len(),
        debit_total_minor: 1_000_000,
    };
    let system_auth = Auth::login("ctms-evaluator", &["System"]);
    let alert_capability = issuer.mint(&system_auth, CtmsAlertGatewayV1::RESOURCE, CtmsAlertGatewayV1::EFFECT, now).unwrap();
    let alert = alert_gateway.create(&alert_capability, &alert_store, &candidate, now).expect("a matching transaction must create an alert");
    assert_eq!(alert.status, AlertStatus::New);
    assert_eq!(alert.rule_id, "velocity_24h");

    // 6. A replay (the batch leg's identical candidate) does not create a
    // duplicate alert.
    let replay_capability = issuer.mint(&system_auth, CtmsAlertGatewayV1::RESOURCE, CtmsAlertGatewayV1::EFFECT, now).unwrap();
    let batch_candidate = AlertCandidate {
        rule_id: batch_match.rule_id.clone(),
        rule_version: batch_match.rule_version,
        key: "cust-1".to_string(),
        window_start_ms: 0,
        window_end_ms: realtime_match.evaluated_at_ms,
        severity: batch_match.severity.clone(),
        matched_event_ids: batch_match.matched_event_ids.clone(),
        debit_count: batch_match.matched_event_ids.len(),
        debit_total_minor: 1_000_000,
    };
    let replayed_alert = alert_gateway.create(&replay_capability, &alert_store, &batch_candidate, now).expect("the same window's candidate must resolve idempotently, not error");
    assert_eq!(replayed_alert.alert_id, alert.alert_id, "must not create a duplicate alert for the same rule/window");
    assert_eq!(alert_store.list().len(), 1);

    // 7. Every mutation writes evidence.
    let create_decisions = evidence_decisions(create_gateway.evidence());
    assert!(create_decisions.contains(&("rule.create".to_string(), "accepted".to_string())), "{create_decisions:?}");
    let submit_decisions = evidence_decisions(submit_gateway.evidence());
    assert!(submit_decisions.contains(&("rule.submit_for_approval".to_string(), "accepted".to_string())), "{submit_decisions:?}");
    let approve_decisions = evidence_decisions(approve_gateway.evidence());
    assert!(approve_decisions.contains(&("rule.approve".to_string(), "accepted".to_string())), "{approve_decisions:?}");
    // The RiskAnalyst's enable attempt in step 1 was rejected by the
    // *role* gate before a capability was ever minted, so it never
    // reaches the gateway at all -- correctly absent from its evidence
    // log (a role rejection isn't a gateway decision; the gateway never
    // saw it). The WrongState-style rejection *is* a gateway-level
    // decision and is covered by `rule_gateway`/`rule_model`'s own unit
    // tests instead.
    let enable_decisions = evidence_decisions(enable_gateway.evidence());
    assert!(enable_decisions.iter().any(|(action, decision)| action == "rule.enable" && decision == "accepted"), "{enable_decisions:?}");

    // 8. An old rule version remains auditable: draft, approve, and
    // enable v2, which supersedes v1 -- v1 must still be in history.
    let mut v2 = draft_rule();
    v2.version = 2;
    let created_v2 = create_rule(&issuer, &create_gateway, &rule_store, &policy, &rule_author, v2, now).unwrap();
    assert_eq!(created_v2.version, 2);
    submit_rule_for_approval(&issuer, &submit_gateway, &rule_store, &policy, &rule_author, "velocity_24h", 2, now).unwrap();
    let approve_v2_capability = issuer.mint(&rule_approver, CtmsRuleApproveGatewayV1::RESOURCE, CtmsRuleApproveGatewayV1::EFFECT, now).unwrap();
    approve_gateway.approve(&approve_v2_capability, &rule_store, "velocity_24h", 2, "arjun", now).unwrap();
    enable_rule(&issuer, &enable_gateway, &rule_store, &policy, &rule_approver, "velocity_24h", 2, now).unwrap();

    assert_eq!(rule_store.active_snapshot("velocity_24h").unwrap().version, 2, "v2 is now the active snapshot");
    let history = rule_store.history("velocity_24h");
    assert_eq!(history.len(), 2, "both versions remain auditable: {history:?}");
    assert_eq!(rule_store.get("velocity_24h", 1).unwrap().status, RuleStatus::Disabled, "v1 is disabled, not deleted");

    let _ = std::fs::remove_file(&create_evidence);
    let _ = std::fs::remove_file(&approve_evidence);
    let _ = std::fs::remove_file(&enable_evidence);
    let _ = std::fs::remove_file(&alert_evidence);
}
