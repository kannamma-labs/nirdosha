//! `CtmsRuleGatewayV1`: the only thing allowed to turn a
//! [`crate::rule_model::RuleDefinition`] draft into an approved, enabled,
//! disabled, retired, or rolled-back rule. Seven effects
//! (`rule.create`/`rule.submit_for_approval`/`rule.approve`/`rule.enable`/
//! `rule.disable`/`rule.retire`/`rule.rollback`) sharing one gateway name
//! (`ctms_rule_gateway_v1`) and evidence chain -- same "several effects,
//! one logical gateway boundary" pattern `crate::case`'s four gateways
//! already establish, for the same reason (each effect needs its own
//! `RESOURCE`/`EFFECT` consts -- `EffectGateway`'s are compile-time, one
//! effect per type).
//!
//! Idempotency and replay protection are inherited for free: every
//! mutation still goes through `GatewayCore::consume_and_execute`, so a
//! replayed capability is rejected by the same single-use nonce check
//! every other CTMS gateway already has -- nothing rule-specific was
//! needed to satisfy "supports idempotency and replay protection."

use std::path::Path;

use nirdosha_guard_rfc0029::{CapabilityIssuer, DecisionCapability, EffectGateway, GatewayCore, GatewayError};

use crate::rule_model::{RuleDefinition, RuleStore};

macro_rules! rule_gateway {
    ($name:ident, $effect:expr) => {
        pub struct $name {
            core: GatewayCore,
        }
        impl $name {
            pub fn new(evidence_path: impl AsRef<Path>, issuer: &CapabilityIssuer) -> Self {
                Self { core: GatewayCore::new(<Self as EffectGateway>::MODULE, evidence_path, issuer) }
            }
            pub fn evidence(&self) -> &nirdosha_audit::envelope::ModuleAuditChain {
                self.core.evidence()
            }
            /// Inherent forwarder to `EffectGateway::consume_and_execute`
            /// -- `cargo-nirdosha`'s `render_capability_gate` emits a
            /// plain `gateway.consume_and_execute(...)` call into
            /// generated app code that never imports this crate's
            /// `EffectGateway` trait, so method resolution needs an
            /// inherent method here (same reasoning every other CTMS
            /// gateway's identical forwarder gives).
            pub fn consume_and_execute<T>(&self, capability: &DecisionCapability, now_ms: u64, execute: impl FnOnce() -> Result<T, String>) -> Result<T, GatewayError> {
                <Self as EffectGateway>::consume_and_execute(self, capability, now_ms, execute)
            }
        }
        impl EffectGateway for $name {
            const RESOURCE: &'static str = "MonitoringRule";
            const EFFECT: &'static str = $effect;
            const MODULE: &'static str = "rfc0029:ctms_rule_gateway_v1";
            fn core(&self) -> &GatewayCore {
                &self.core
            }
        }
    };
}

rule_gateway!(CtmsRuleCreateGatewayV1, "rule.create");
rule_gateway!(CtmsRuleSubmitGatewayV1, "rule.submit_for_approval");
rule_gateway!(CtmsRuleApproveGatewayV1, "rule.approve");
rule_gateway!(CtmsRuleEnableGatewayV1, "rule.enable");
rule_gateway!(CtmsRuleDisableGatewayV1, "rule.disable");
rule_gateway!(CtmsRuleRetireGatewayV1, "rule.retire");
rule_gateway!(CtmsRuleRollbackGatewayV1, "rule.rollback");

impl CtmsRuleCreateGatewayV1 {
    pub fn create(&self, capability: &DecisionCapability, store: &dyn RuleStore, rule: RuleDefinition, now_ms: u64) -> Result<RuleDefinition, GatewayError> {
        <Self as EffectGateway>::consume_and_execute(self, capability, now_ms, || store.create_draft(rule).map_err(|e| e.to_string()))
    }
}

impl CtmsRuleSubmitGatewayV1 {
    pub fn submit(&self, capability: &DecisionCapability, store: &dyn RuleStore, rule_id: &str, version: u32, now_ms: u64) -> Result<RuleDefinition, GatewayError> {
        <Self as EffectGateway>::consume_and_execute(self, capability, now_ms, || store.submit_for_approval(rule_id, version).map_err(|e| e.to_string()))
    }
}

impl CtmsRuleApproveGatewayV1 {
    pub fn approve(&self, capability: &DecisionCapability, store: &dyn RuleStore, rule_id: &str, version: u32, actor: &str, now_ms: u64) -> Result<RuleDefinition, GatewayError> {
        <Self as EffectGateway>::consume_and_execute(self, capability, now_ms, || store.approve(rule_id, version, actor).map_err(|e| e.to_string()))
    }
}

impl CtmsRuleEnableGatewayV1 {
    pub fn enable(&self, capability: &DecisionCapability, store: &dyn RuleStore, rule_id: &str, version: u32, now_ms: u64) -> Result<RuleDefinition, GatewayError> {
        <Self as EffectGateway>::consume_and_execute(self, capability, now_ms, || store.enable(rule_id, version).map_err(|e| e.to_string()))
    }
}

impl CtmsRuleDisableGatewayV1 {
    pub fn disable(&self, capability: &DecisionCapability, store: &dyn RuleStore, rule_id: &str, version: u32, now_ms: u64) -> Result<RuleDefinition, GatewayError> {
        <Self as EffectGateway>::consume_and_execute(self, capability, now_ms, || store.disable(rule_id, version).map_err(|e| e.to_string()))
    }
}

impl CtmsRuleRetireGatewayV1 {
    pub fn retire(&self, capability: &DecisionCapability, store: &dyn RuleStore, rule_id: &str, version: u32, now_ms: u64) -> Result<RuleDefinition, GatewayError> {
        <Self as EffectGateway>::consume_and_execute(self, capability, now_ms, || store.retire(rule_id, version).map_err(|e| e.to_string()))
    }
}

impl CtmsRuleRollbackGatewayV1 {
    pub fn rollback(&self, capability: &DecisionCapability, store: &dyn RuleStore, rule_id: &str, to_version: u32, now_ms: u64) -> Result<RuleDefinition, GatewayError> {
        <Self as EffectGateway>::consume_and_execute(self, capability, now_ms, || store.rollback(rule_id, to_version).map_err(|e| e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rule_model::{InMemoryRuleStore, RuleStatus, RuleType};
    use nirdosha_guard_rfc0029::PolicyBundle;
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

    fn draft() -> RuleDefinition {
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

    #[test]
    fn create_through_the_gateway_writes_evidence_and_produces_a_draft() {
        let issuer = issuer();
        let gateway = CtmsRuleCreateGatewayV1::new(std::env::temp_dir().join(format!("ctms_rule_gw_create_{}", std::process::id())), &issuer);
        let store = InMemoryRuleStore::new();
        let auth = Auth::login("priya", &["RuleAuthor"]);
        let cap = issuer.mint(&auth, CtmsRuleCreateGatewayV1::RESOURCE, CtmsRuleCreateGatewayV1::EFFECT, now_ms()).unwrap();
        let created = gateway.create(&cap, &store, draft(), now_ms()).unwrap();
        assert_eq!(created.status, RuleStatus::Draft);
        assert_eq!(gateway.evidence().entries().len(), 1);
    }

    #[test]
    fn a_replayed_approval_capability_is_rejected() {
        let issuer = issuer();
        let store = InMemoryRuleStore::new();
        store.create_draft(draft()).unwrap();
        store.submit_for_approval("velocity_24h", 1).unwrap();

        let gateway = CtmsRuleApproveGatewayV1::new(std::env::temp_dir().join(format!("ctms_rule_gw_approve_{}", std::process::id())), &issuer);
        let auth = Auth::login("arjun", &["RuleApprover"]);
        let cap = issuer.mint(&auth, CtmsRuleApproveGatewayV1::RESOURCE, CtmsRuleApproveGatewayV1::EFFECT, now_ms()).unwrap();
        assert!(gateway.approve(&cap, &store, "velocity_24h", 1, "arjun", now_ms()).is_ok());
        let err = gateway.approve(&cap, &store, "velocity_24h", 1, "arjun", now_ms()).unwrap_err();
        assert_eq!(err, GatewayError::CapabilityReplayed);
    }
}
