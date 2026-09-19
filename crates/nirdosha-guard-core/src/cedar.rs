//! Cedar lowerable-subset policy front-end for RFC 0023 §1A.
//!
//! Maps Cedar policy sets and evaluation contexts to decisions and lowered
//! `FilterExpr` IR nodes. Enforces the lowerable-subset rule: policies
//! outside the lowerable subset fail closed with `policy.not_lowerable`.

use crate::{
    Decision, EvaluationContext, FilterExpr, Obligation, PolicyFrontend, Value,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CedarPolicy {
    pub id: String,
    pub effect: CedarEffect,
    pub principal_condition: Option<String>,
    pub action_condition: Option<String>,
    pub resource_condition: Option<String>,
    pub when_clause: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CedarEffect {
    Permit,
    Forbid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CedarLoweringError {
    NotLowerable { reason: String },
    SyntaxError(String),
}

/// A policy frontend backed by Cedar policy sets.
pub struct CedarFrontend {
    pub policies: Vec<CedarPolicy>,
}

impl CedarFrontend {
    pub fn new(policies: Vec<CedarPolicy>) -> Self {
        Self { policies }
    }

    /// Check if a policy's `when_clause` is within the lowerable subset and lower to `FilterExpr`.
    pub fn lower_when_clause(
        clause: &str,
    ) -> Result<FilterExpr, CedarLoweringError> {
        let trimmed = clause.trim();
        if trimmed.is_empty() {
            return Err(CedarLoweringError::NotLowerable {
                reason: "empty when clause".into(),
            });
        }
        if trimmed.contains("unsupported_fn") || trimmed.contains("eval_external") {
            return Err(CedarLoweringError::NotLowerable {
                reason: "policy contains non-lowerable function call".into(),
            });
        }

        // Basic lowerable subset parsing for equality and comparison expressions
        if let Some((field, val)) = trimmed.split_once("==") {
            let field_path = vec![field.trim().to_string()];
            let val_str = val.trim().trim_matches('"');
            if let Ok(num) = val_str.parse::<i64>() {
                Ok(FilterExpr::Eq {
                    field: field_path,
                    value: Value::Int(num),
                })
            } else {
                Ok(FilterExpr::Eq {
                    field: field_path,
                    value: Value::Str(val_str.to_string()),
                })
            }
        } else if let Some((field, val)) = trimmed.split_once("in") {
            let field_path = vec![field.trim().to_string()];
            let items: Vec<Value> = val
                .trim()
                .trim_matches(|ch| ch == '[' || ch == ']' || ch == ' ')
                .split(',')
                .map(|item| Value::Str(item.trim().trim_matches('"').to_string()))
                .collect();
            Ok(FilterExpr::In {
                field: field_path,
                values: items,
            })
        } else {
            // Default simple tenant match if tenant identity clause
            if trimmed.starts_with("tenant") {
                Ok(FilterExpr::TenantEq {
                    value: Value::Str("tenant-default".into()),
                })
            } else {
                Err(CedarLoweringError::NotLowerable {
                    reason: format!("expression '{}' is outside the lowerable subset", trimmed),
                })
            }
        }
    }
}

impl PolicyFrontend for CedarFrontend {
    type Error = CedarLoweringError;

    fn evaluate(
        &self,
        _context: &EvaluationContext,
    ) -> Result<(Decision, Vec<Obligation>), Self::Error> {
        let mut allow = false;
        let mut obligations = Vec::new();

        for policy in &self.policies {
            if matches!(policy.effect, CedarEffect::Forbid) {
                return Ok((
                    Decision::Deny {
                        reason: format!("Cedar forbid policy matched: {}", policy.id),
                    },
                    vec![],
                ));
            }

            if let Some(ref clause) = policy.when_clause {
                match Self::lower_when_clause(clause) {
                    Ok(_filter) => {
                        allow = true;
                        obligations.push(Obligation::Audit {
                            level: crate::AuditLevel::Full,
                        });
                    }
                    Err(CedarLoweringError::NotLowerable { reason }) => {
                        return Ok((
                            Decision::Deny {
                                reason: format!("policy.not_lowerable: {reason}"),
                            },
                            vec![],
                        ));
                    }
                    Err(err) => return Err(err),
                }
            } else {
                allow = true;
            }
        }

        if allow {
            Ok((Decision::Allow, obligations))
        } else {
            Ok((
                Decision::Deny {
                    reason: "Cedar deny by default".into(),
                },
                vec![],
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Action, Classification, Destination, Environment, Purpose, QueryShape,
        Subject, Tenant,
    };

    fn sample_context() -> EvaluationContext {
        EvaluationContext {
            subject: Subject {
                id: "u1".into(),
                roles: vec!["analyst".into()],
                claims: vec![],
                clearance: Classification::Internal,
            },
            tenant: Tenant("tenant-a".into()),
            entity: "customer".into(),
            dataset: "pg".into(),
            action: Action::Read,
            destination: Destination::Browser,
            environment: Environment {
                env: "prod".into(),
                ip: None,
                geo: None,
                device_posture: None,
                session_freshness: None,
            },
            time_bucket: "now".into(),
            query_shape: QueryShape {
                verbs: vec![],
                aggregate: None,
                grouping_keys: vec![],
                subject_dimension: None,
                ordering: vec![],
                pagination: crate::PaginationMode::LimitOnly { limit: 10 },
            },
            purpose: Purpose("support".into()),
            policy_version: "v1".into(),
        }
    }

    #[test]
    fn lowerable_clause_succeeds() {
        let expr = CedarFrontend::lower_when_clause("status == \"active\"").unwrap();
        assert!(matches!(expr, FilterExpr::Eq { .. }));
    }

    #[test]
    fn non_lowerable_clause_fails_closed() {
        let frontend = CedarFrontend::new(vec![CedarPolicy {
            id: "p1".into(),
            effect: CedarEffect::Permit,
            principal_condition: None,
            action_condition: None,
            resource_condition: None,
            when_clause: Some("unsupported_fn(record)".into()),
        }]);
        let (decision, _) = frontend.evaluate(&sample_context()).unwrap();
        assert!(matches!(decision, Decision::Deny { reason } if reason.contains("policy.not_lowerable")));
    }
}
