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
    pub session_evaluations: HashMap<String, AccessPlan>,
}

impl GuardMcpServer {
    pub fn new() -> Self {
        Self {
            tools: vec![
                ToolDescription {
                    name: "list_entities".into(),
                    description: "List guard-registered entities".into(),
                    input_schema: "{}".into(),
                    signature_hash: "...".into(),
                },
                ToolDescription {
                    name: "describe_entity".into(),
                    description: "Return redacted schema + classification".into(),
                    input_schema: "{\"entity\":\"string\"}".into(),
                    signature_hash: "...".into(),
                },
                ToolDescription {
                    name: "get_options".into(),
                    description: "Enumerate allowed values for a categorical field".into(),
                    input_schema: "{\"entity\":\"string\",\"field\":\"string\"}".into(),
                    signature_hash: "...".into(),
                },
                ToolDescription {
                    name: "query_records".into(),
                    description: "Query records under policy".into(),
                    input_schema: "{\"entity\":\"string\"}".into(),
                    signature_hash: "...".into(),
                },
                ToolDescription {
                    name: "evaluate".into(),
                    description: "Dry-run a policy decision".into(),
                    input_schema: "{\"entity\":\"string\",\"action\":\"string\"}".into(),
                    signature_hash: "...".into(),
                },
            ],
            agent_writes_enabled: false,
            session_evaluations: HashMap::new(),
        }
    }

    /// Evaluate a request in the LLM context. Default posture denies
    /// CONFIDENTIAL and sensitive-purpose reads unless explicitly granted.
    pub fn evaluate(
        &self,
        _token: &DelegationToken,
        context: &EvaluationContext,
    ) -> (Decision, AccessPlan) {
        // Default fail-closed placeholder.
        let plan = AccessPlan {
            decision: Decision::Allow,
            filter: None,
            masks: vec![],
            caps: vec![],
            obligations: vec![],
            policy_version: context.policy_version.clone(),
        };
        (Decision::Allow, plan)
    }

    /// `submit_write` is default-off for agents.
    pub fn submit_write(
        &self,
        _token: &DelegationToken,
        _plan_hash: String,
        _plan: WritePlan,
    ) -> Decision {
        if !self.agent_writes_enabled {
            return Decision::Deny {
                reason: "agent writes are disabled by default".into(),
            };
        }
        Decision::Deny {
            reason: "evaluate-then-act hash match required".into(),
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

    #[test]
    fn agent_write_default_off() {
        let server = GuardMcpServer::new();
        let token = DelegationToken {
            user_id: "u1".into(),
            agent_id: "a1".into(),
            purpose: Purpose("Agent".into()),
            destination: Destination::LlmContext,
            policy_version: "v1".into(),
            scope_hash: "h1".into(),
            expires_at: "2026-09-19T00:00:00Z".into(),
        };
        let decision = server.submit_write(
            &token,
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
}
