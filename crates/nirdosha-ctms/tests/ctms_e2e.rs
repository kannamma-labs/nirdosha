//! End-to-end proof of the CTMS design note's own "first meaningful
//! target", extended with the escalation chain and the AI model signal:
//!
//! ```text
//! synthetic Kafka event -> velocity rule -> AI model signal (advisory)
//!   -> alert.create -> RiskAnalyst alert queue -> case assignment
//!   -> RiskAnalyst escalation -> SeniorInvestigator review
//!   -> ComplianceOfficer disposition (high severity) -> evidence/audit record
//! ```
//!
//! Every step here runs through the real, shipped types (`InMemoryTopic`,
//! `StreamAdapter`, `VelocityWindowStore`, `VelocitySignalModel`,
//! `CtmsAlertGatewayV1`, `CtmsCaseAssignGatewayV1`,
//! `CtmsCaseEscalateGatewayV1`, `CtmsCaseSeniorReviewGatewayV1`,
//! `CtmsCaseDispositionGatewayV1`, `CaseWorkflowPolicy`) -- nothing in this
//! test re-implements the logic it's checking.

use std::path::Path;

use nirdosha_ctms::alert::{AlertStatus, AlertStore, CtmsAlertGatewayV1, InMemoryAlertStore, ModelSignal};
use nirdosha_ctms::case::{CaseStatus, CaseStore, CtmsCaseAssignGatewayV1, CtmsCaseDispositionGatewayV1, CtmsCaseEscalateGatewayV1, CtmsCaseSeniorReviewGatewayV1, InMemoryCaseStore};
use nirdosha_ctms::model::VelocitySignalModel;
use nirdosha_ctms::policy::CaseWorkflowPolicy;
use nirdosha_ctms::rules::{RuleCatalog, VelocityWindowStore};
use nirdosha_ctms::service::{assign_case, disposition_case, escalate_case, senior_review_case, CtmsServiceError};
use nirdosha_ctms::stream::{InMemoryTopic, StreamAdapter};
use nirdosha_guard_rfc0029::{CapabilityIssuer, EffectGateway, GatewayError, PolicyBundle};
use nirdosha_ingestion_topic_kafka::{RawRecord, StreamSink, StreamSource};
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

fn debit_event_json(event_id: &str, event_time_ms: u64) -> String {
    serde_json::json!({
        "event_id": event_id,
        "transaction_id": format!("txn-{event_id}"),
        "event_type": "card_debit",
        "customer_id": "cust-1",
        "account_id": "acct-1",
        "direction": "debit",
        "amount_minor": 200_000,
        "currency": "INR",
        "event_time_ms": event_time_ms,
        "source_system": "card-network-x",
        "source_version": "v1",
        "jurisdiction": "IN",
    })
    .to_string()
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
fn synthetic_kafka_event_to_disposition_evidence_end_to_end() {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();

    // 1. Publish five synthetic debit transactions for one customer onto a
    // demo Kafka-compatible topic.
    let topic = InMemoryTopic::new();
    runtime.block_on(async {
        let records = (0..5)
            .map(|i| RawRecord { key: None, payload: debit_event_json(&format!("evt-{i}"), 1_000 * i as u64).into_bytes() })
            .collect();
        topic.publish("ctms.transactions", records).await.unwrap();
    });

    // Consumer side: deserialize -> validate -> dedup -> normalize.
    let batch = runtime.block_on(async { topic.poll_batch("ctms.transactions", 0, 0).await.unwrap() });
    let adapter = StreamAdapter::new();
    let (_outcomes, normalized) = adapter.process_batch(&batch, 0, now_ms());
    assert_eq!(normalized.len(), 5, "all five well-formed debit events must be accepted");

    // 2. The velocity rule (this org's own governed threshold, loaded
    // from data -- not a Rust constant) triggers on the fifth debit.
    let rule_catalog = RuleCatalog::default_v1();
    let velocity_rule = rule_catalog.velocity_rule("velocity_24h").expect("default catalog defines velocity_24h");
    let window_store = VelocityWindowStore::new();
    let mut candidate = None;
    for record in &normalized {
        candidate = window_store.observe_and_evaluate(velocity_rule, &record.event);
    }
    let candidate = candidate.expect("5 debits totalling 1,000,000 minor units must cross velocity_24h's threshold");

    // The AI model scores the same real candidate features -- advisory
    // only, never consulted by the gateway's authorization decision.
    let fixture_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let model = VelocitySignalModel::load_from_fixture(
        fixture_dir.join("ctms_velocity_signal_v1_test_fixture.onnx"),
        fixture_dir.join("ctms_velocity_signal_v1_test_fixture.onnx.sha256"),
    )
    .expect("real fixture model must load");
    let model_receipt = model.score_candidate(&candidate, now_ms()).expect("scoring must succeed");
    assert!(model_receipt.score > 0.5, "5 debits crossing the rule threshold should also score as elevated risk: {model_receipt:?}");

    let issuer = issuer();
    let policy = CaseWorkflowPolicy::default_v1();
    let now = now_ms();

    // 3. alert.create through the guarded gateway, then the model signal
    // is attached as advisory metadata (not part of the gateway-guarded
    // mutation itself -- see crate::model's module doc).
    let alert_store = InMemoryAlertStore::new();
    let alert_evidence_dir = std::env::temp_dir().join(format!("ctms_e2e_alert_{}", std::process::id()));
    let alert_gateway = CtmsAlertGatewayV1::new(&alert_evidence_dir, &issuer);
    let system_auth = Auth::login("ctms-evaluator", &["System"]);
    let alert_capability = issuer.mint(&system_auth, CtmsAlertGatewayV1::RESOURCE, CtmsAlertGatewayV1::EFFECT, now).unwrap();
    let alert = alert_gateway.create(&alert_capability, &alert_store, &candidate, now).unwrap();
    assert_eq!(alert.status, AlertStatus::New);
    assert_eq!(alert.severity, "high", "the rule alone sets severity -- the model score never overwrites it");

    let model_signal: ModelSignal = model_receipt.into();
    let enriched_alert = alert_store.attach_model_signal(&alert.alert_id, model_signal).unwrap();
    assert!(enriched_alert.model_signal.is_some());
    assert_eq!(enriched_alert.severity, alert.severity, "attaching a model signal must never change the rule's own severity");

    // 4. The alert shows up in the (read-only) RiskAnalyst alert queue.
    let queued = alert_store.list();
    assert!(queued.iter().any(|a| a.alert_id == alert.alert_id));

    // 5. A RiskAnalyst assigns the case to themselves.
    let case_store = InMemoryCaseStore::new();
    let case_evidence_dir = std::env::temp_dir().join(format!("ctms_e2e_case_{}", std::process::id()));
    let assign_gateway = CtmsCaseAssignGatewayV1::new(&case_evidence_dir, &issuer);
    let risk_analyst = Auth::login("priya", &["RiskAnalyst"]);
    let assigned_case = assign_case(&issuer, &assign_gateway, &case_store, &policy, &risk_analyst, &alert.alert_id, &alert.severity, now).expect("RiskAnalyst may assign a case");
    assert_eq!(assigned_case.status, CaseStatus::Assigned);
    assert_eq!(assigned_case.assigned_analyst.as_deref(), Some("priya"));

    // 6. A second RiskAnalyst who wasn't assigned this case cannot
    // escalate it -- the role check alone would admit them (same role);
    // the store's own same-actor invariant is what actually stops them.
    let escalate_gateway = CtmsCaseEscalateGatewayV1::new(&case_evidence_dir, &issuer);
    let other_analyst = Auth::login("kavya", &["RiskAnalyst"]);
    let not_assigned_result = escalate_case(&issuer, &escalate_gateway, &case_store, &policy, &other_analyst, &assigned_case.case_id, "trying to escalate someone else's case", now);
    assert!(not_assigned_result.is_err(), "{not_assigned_result:?}");
    assert_eq!(case_store.get(&assigned_case.case_id).unwrap().status, CaseStatus::Assigned, "the rejected attempt must not have moved the case");

    // The RiskAnalyst it's actually assigned to investigates and
    // escalates it for real.
    let escalated_case = escalate_case(&issuer, &escalate_gateway, &case_store, &policy, &risk_analyst, &assigned_case.case_id, "structuring pattern across 5 debits", now)
        .expect("the assigned RiskAnalyst may escalate their own case");
    assert_eq!(escalated_case.status, CaseStatus::Escalated);

    // Wrong role is rejected: a RiskAnalyst cannot review their own
    // escalation (no SeniorInvestigator role) -- rejected before a
    // capability is even minted.
    let senior_review_gateway = CtmsCaseSeniorReviewGatewayV1::new(&case_evidence_dir, &issuer);
    let wrong_role_result = senior_review_case(&issuer, &senior_review_gateway, &case_store, &policy, &risk_analyst, &escalated_case.case_id, now);
    assert!(matches!(wrong_role_result, Err(CtmsServiceError::Forbidden { ref required_role }) if required_role == "SeniorInvestigator"), "{wrong_role_result:?}");

    // 7. A SeniorInvestigator reviews the escalation.
    let senior_investigator = Auth::login("arjun", &["SeniorInvestigator"]);
    let reviewed_case = senior_review_case(&issuer, &senior_review_gateway, &case_store, &policy, &senior_investigator, &escalated_case.case_id, now).expect("a SeniorInvestigator may review the escalation");
    assert_eq!(reviewed_case.status, CaseStatus::SeniorReviewed);

    // Disposing a "high"-severity case requires ComplianceOfficer, not
    // SeniorInvestigator, per the policy's severity routing -- the same
    // SeniorInvestigator who just reviewed the escalation is rejected here.
    let disposition_gateway = CtmsCaseDispositionGatewayV1::new(&case_evidence_dir, &issuer);
    let wrong_role_disposition = disposition_case(&issuer, &disposition_gateway, &case_store, &policy, &senior_investigator, &reviewed_case.case_id, &assigned_case.severity, "SuspiciousActivity", now);
    assert!(matches!(wrong_role_disposition, Err(CtmsServiceError::Forbidden { ref required_role }) if required_role == "ComplianceOfficer"), "{wrong_role_disposition:?}");

    // 8. A ComplianceOfficer dispositions the high-severity case. The
    // capability is minted directly here (rather than via
    // `disposition_case`) so it can be deliberately reused below to prove
    // replay is rejected.
    let compliance_officer = Auth::login("meera", &["ComplianceOfficer"]);
    let disposition_capability = issuer.mint(&compliance_officer, CtmsCaseDispositionGatewayV1::RESOURCE, CtmsCaseDispositionGatewayV1::EFFECT, now).unwrap();
    let disposed_case = disposition_gateway
        .disposition(&disposition_capability, &case_store, &reviewed_case.case_id, "meera", "SuspiciousActivity", now)
        .expect("a ComplianceOfficer may disposition a high-severity case");
    assert_eq!(disposed_case.status, CaseStatus::SuspiciousActivity);
    assert_eq!(disposed_case.disposition_by.as_deref(), Some("meera"));

    // Replay is rejected: reusing the exact same already-consumed
    // capability a second time is a capability replay, rejected at the
    // gateway (single-use nonce check) before the guarded effect -- now
    // also state-invariant-protected (`WrongState`, since the case has
    // already left `SeniorReviewed`) -- would even run a second time.
    let replay_err = disposition_gateway
        .disposition(&disposition_capability, &case_store, &reviewed_case.case_id, "meera", "SuspiciousActivity", now)
        .unwrap_err();
    assert_eq!(replay_err, GatewayError::CapabilityReplayed);

    // 9. Evidence/audit record: every real decision (accepted and
    // rejected) is traceable, across every gateway boundary.
    let alert_decisions = evidence_decisions(alert_gateway.evidence());
    assert!(alert_decisions.contains(&("alert.create".to_string(), "accepted".to_string())), "{alert_decisions:?}");

    let assign_decisions = evidence_decisions(assign_gateway.evidence());
    assert!(assign_decisions.contains(&("case.assign".to_string(), "accepted".to_string())), "{assign_decisions:?}");

    let escalate_decisions = evidence_decisions(escalate_gateway.evidence());
    assert!(escalate_decisions.contains(&("case.escalate".to_string(), "accepted".to_string())), "{escalate_decisions:?}");

    let senior_review_decisions = evidence_decisions(senior_review_gateway.evidence());
    assert!(senior_review_decisions.contains(&("case.senior_review".to_string(), "accepted".to_string())), "{senior_review_decisions:?}");

    let disposition_decisions = evidence_decisions(disposition_gateway.evidence());
    assert!(disposition_decisions.iter().filter(|(action, decision)| action == "case.disposition" && decision == "accepted").count() == 1, "{disposition_decisions:?}");
    assert!(disposition_decisions.iter().any(|(action, decision)| action == "case.disposition" && decision == "rejected"), "the replay must also be evidence-recorded: {disposition_decisions:?}");

    let _ = std::fs::remove_file(&alert_evidence_dir);
    let _ = std::fs::remove_file(&case_evidence_dir);
}
