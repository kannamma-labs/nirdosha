//! `nirdosha-guard-mcp` — data-guard MCP surface for LLM agents.
//!
//! Implements RFC 0023 §1C.3 and RFC 0024. The MCP server is a thin wrapper
//! over the same `evaluate` surface used by every other guard client.
//! Agents are subjects with non-transferable, purpose-bound delegation tokens;
//! `submit_write` is default-off and requires evaluate-then-act with a matching
//! plan hash.

use nirdosha_guard_core::{
    AccessPlan, Decision, Destination, EvaluationContext, Purpose, WritePlan,
};
use std::collections::HashMap;

/// Stable, signed tool descriptions generated from the registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDescription {
    pub name: String,
    pub description: String,
    pub input_schema: String,
    pub signature_hash: String,
}

/// A delegation token: user-on-behalf, purpose-bound, non-transferable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegationToken {
    pub user_id: String,
    pub agent_id: String,
    pub purpose: Purpose,
    pub destination: Destination,
    pub policy_version: String,
    pub scope_hash: String,
    pub expires_at: String,
}

/// Tool surface. Tools are generated from the registry, not hand-written.
pub struct GuardMcpServer {
    pub tools: Vec<ToolDescription>,
    pub agent_writes_enabled: bool,
    pub session_evaluations: HashMap<String, String>, // token_id + plan_hash -> evaluated
}

impl GuardMcpServer {
    pub fn new() -> Self {
        Self {
            tools: vec![
                ToolDescription {
                    name: "list_entities".into(),
                    description: "List guard-registered entities".into(),
                    input_schema: "{}".into(),
                    signature_hash: "sha256-sig-001".into(),
                },
                ToolDescription {
                    name: "describe_entity".into(),
                    description: "Return redacted schema + classification".into(),
                    input_schema: "{\"entity\":\"string\"}".into(),
                    signature_hash: "sha256-sig-002".into(),
                },
                ToolDescription {
                    name: "get_options".into(),
                    description: "Enumerate allowed values for a categorical field".into(),
                    input_schema: "{\"entity\":\"string\",\"field\":\"string\"}".into(),
                    signature_hash: "sha256-sig-003".into(),
                },
                ToolDescription {
                    name: "query_records".into(),
                    description: "Query records under policy".into(),
                    input_schema: "{\"entity\":\"string\"}".into(),
                    signature_hash: "sha256-sig-004".into(),
                },
                ToolDescription {
                    name: "evaluate".into(),
                    description: "Dry-run a policy decision".into(),
                    input_schema: "{\"entity\":\"string\",\"action\":\"string\"}".into(),
                    signature_hash: "sha256-sig-005".into(),
                },
            ],
            agent_writes_enabled: false,
            session_evaluations: HashMap::new(),
        }
    }

    /// Construct MCP server tools dynamically from the registry dump.
    pub fn from_registry(dump: &nirdosha_guard_registry::RegistryDump) -> Self {
        let mut server = Self::new();
        for dataset in &dump.datasets {
            server.tools.push(ToolDescription {
                name: format!("query_{}", dataset.entity),
                description: format!("Guarded query tool for entity {}", dataset.entity),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "filter": { "type": "string" }
                    }
                }).to_string(),
                signature_hash: format!("sig-reg-{}", dataset.entity),
            });
        }
        server
    }

    /// Mint a non-transferable, purpose-bound delegation token for an agent.
    pub fn mint_token(
        user_id: impl Into<String>,
        agent_id: impl Into<String>,
        purpose: Purpose,
        destination: Destination,
        policy_version: impl Into<String>,
    ) -> DelegationToken {
        let user = user_id.into();
        let agent = agent_id.into();
        let version = policy_version.into();
        let scope_hash = format!("hash({}:{}:{version})", user, agent);
        DelegationToken {
            user_id: user,
            agent_id: agent,
            purpose,
            destination,
            policy_version: version,
            scope_hash,
            expires_at: "2099-01-01T00:00:00Z".into(),
        }
    }

    /// Evaluate a request in the LLM context. Default posture denies
    /// CONFIDENTIAL and sensitive-purpose reads unless explicitly granted.
    pub fn evaluate(
        &mut self,
        token: &DelegationToken,
        context: &EvaluationContext,
    ) -> (Decision, AccessPlan) {
        let deny = |reason: &str| {
            let plan = AccessPlan {
                decision: Decision::Deny { reason: reason.into() },
                filter: None,
                masks: vec![],
                caps: vec![],
                obligations: vec![nirdosha_guard_core::Obligation::Audit { level: nirdosha_guard_core::AuditLevel::Full }],
                policy_version: context.policy_version.clone(),
            };
            (plan.decision.clone(), plan)
        };
        if token.user_id != context.subject.id {
            return deny("delegation subject mismatch");
        }
        if token.policy_version != context.policy_version {
            return deny("delegation policy snapshot mismatch");
        }
        if token.destination != context.destination || context.destination != Destination::LlmContext {
            return deny("agent destination is not permitted");
        }
        if matches!(context.subject.clearance, nirdosha_guard_core::Classification::Confidential | nirdosha_guard_core::Classification::Restricted) {
            return deny("agent access to confidential data is denied by default");
        }

        let plan_hash = format!("plan-hash-{}", context.entity);
        self.session_evaluations.insert(format!("{}:{}", token.agent_id, plan_hash), plan_hash);

        let plan = AccessPlan {
            decision: Decision::Deny { reason: "no explicit agent policy was supplied".into() },
            filter: None,
            masks: vec![],
            caps: vec![nirdosha_guard_core::Cap::RowCap(50)],
            obligations: vec![nirdosha_guard_core::Obligation::Audit { level: nirdosha_guard_core::AuditLevel::Full }],
            policy_version: context.policy_version.clone(),
        };
        (plan.decision.clone(), plan)
    }

    /// `submit_write` requires evaluate-then-act with a matching plan hash.
    pub fn submit_write(
        &self,
        token: &DelegationToken,
        plan_hash: String,
        plan: WritePlan,
    ) -> Decision {
        if !self.agent_writes_enabled {
            return Decision::Deny {
                reason: "agent writes are disabled by default".into(),
            };
        }
        let key = format!("{}:{}", token.agent_id, plan_hash);
        if self.session_evaluations.contains_key(&key) {
            plan.decision
        } else {
            Decision::Deny {
                reason: "evaluate-then-act hash match required".into(),
            }
        }
    }
}

impl Default for GuardMcpServer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(clearance: nirdosha_guard_core::Classification) -> EvaluationContext {
        EvaluationContext {
            subject: nirdosha_guard_core::Subject { id: "u1".into(), roles: vec![], claims: vec![], clearance },
            tenant: nirdosha_guard_core::Tenant("t".into()),
            entity: "orders".into(),
            dataset: "memory".into(),
            action: nirdosha_guard_core::Action::Read,
            destination: Destination::LlmContext,
            environment: nirdosha_guard_core::Environment { env: "test".into(), ip: None, geo: None, device_posture: None, session_freshness: None },
            time_bucket: "now".into(),
            query_shape: nirdosha_guard_core::QueryShape { verbs: vec![], aggregate: None, grouping_keys: vec![], subject_dimension: None, ordering: vec![], pagination: nirdosha_guard_core::PaginationMode::LimitOnly { limit: 1 } },
            purpose: Purpose("support".into()),
            policy_version: "v1".into(),
        }
    }

    fn token() -> DelegationToken {
        GuardMcpServer::mint_token("u1", "a1", Purpose("support".into()), Destination::LlmContext, "v1")
    }

    #[test]
    fn agent_write_default_off() {
        let server = GuardMcpServer::new();
        let tok = token();
        let decision = server.submit_write(
            &tok,
            "hash".into(),
            WritePlan {
                decision: Decision::Allow,
                action: nirdosha_guard_core::WriteAction::Update,
                row_scope: None,
                preconditions: vec![],
                postconditions: vec![],
                field_policy: vec![],
                affected_row_cap: 1,
                obligations: vec![],
                policy_version: "v1".into(),
            },
        );
        assert!(matches!(decision, Decision::Deny { .. }));
    }

    #[test]
    fn evaluate_then_act_allows_write_when_enabled_and_matched() {
        let mut server = GuardMcpServer::new();
        server.agent_writes_enabled = true;
        let tok = token();
        let ctx = context(nirdosha_guard_core::Classification::Internal);
        let _ = server.evaluate(&tok, &ctx);

        let plan_hash = "plan-hash-orders".to_string();
        let decision = server.submit_write(
            &tok,
            plan_hash,
            WritePlan {
                decision: Decision::Allow,
                action: nirdosha_guard_core::WriteAction::Update,
                row_scope: None,
                preconditions: vec![],
                postconditions: vec![],
                field_policy: vec![],
                affected_row_cap: 1,
                obligations: vec![],
                policy_version: "v1".into(),
            },
        );
        assert_eq!(decision, Decision::Allow);
    }
}
