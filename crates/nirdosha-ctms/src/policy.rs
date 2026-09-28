//! `[case_workflow]` policy: which role each case-workflow step requires,
//! loaded from data (a TOML string, same convention
//! `nirdosha_guard_rfc0029::PolicyBundle` uses for governance fields)
//! rather than hardcoded in `crate::service`. Severity routes disposition
//! to a different required role -- "SeniorInvestigator: approve
//! medium-risk dispositions" / "ComplianceOfficer: approve high-risk
//! dispositions" -- and that routing table is itself data here, so
//! changing which severities need which role is a policy edit, not a
//! Rust recompile.
//!
//! This crate does not yet read this policy out of the real
//! `policy-services.toml`/`screens.toml` a generated app would use (see
//! crate module doc's "explicitly out of scope" list -- screen/codegen
//! wiring is separate, not-yet-built work); [`CaseWorkflowPolicy::default_v1`]
//! is what `crate::service`'s functions use until then, but it's still one
//! `from_toml_str` call away from being swapped for a real file today.

use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
struct RawCaseWorkflowPolicyFile {
    case_workflow: RawCaseWorkflowPolicy,
}

#[derive(Debug, Clone, Deserialize)]
struct RawCaseWorkflowPolicy {
    assign_role: String,
    escalate_role: String,
    senior_review_role: String,
    disposition_default_role: String,
    #[serde(default)]
    disposition_role_by_severity: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaseWorkflowPolicyError {
    Parse(String),
    EmptyField(&'static str),
}

impl std::fmt::Display for CaseWorkflowPolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CaseWorkflowPolicyError::Parse(e) => write!(f, "case workflow policy is invalid: {e}"),
            CaseWorkflowPolicyError::EmptyField(name) => write!(f, "case workflow policy field `{name}` must not be empty"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseWorkflowPolicy {
    pub assign_role: String,
    pub escalate_role: String,
    pub senior_review_role: String,
    pub disposition_default_role: String,
    pub disposition_role_by_severity: HashMap<String, String>,
}

impl CaseWorkflowPolicy {
    pub fn from_toml_str(src: &str) -> Result<Self, CaseWorkflowPolicyError> {
        let raw: RawCaseWorkflowPolicyFile = toml::from_str(src).map_err(|e| CaseWorkflowPolicyError::Parse(e.to_string()))?;
        let p = raw.case_workflow;
        for (name, value) in [
            ("assign_role", &p.assign_role),
            ("escalate_role", &p.escalate_role),
            ("senior_review_role", &p.senior_review_role),
            ("disposition_default_role", &p.disposition_default_role),
        ] {
            if value.trim().is_empty() {
                return Err(CaseWorkflowPolicyError::EmptyField(name));
            }
        }
        Ok(CaseWorkflowPolicy {
            assign_role: p.assign_role,
            escalate_role: p.escalate_role,
            senior_review_role: p.senior_review_role,
            disposition_default_role: p.disposition_default_role,
            disposition_role_by_severity: p.disposition_role_by_severity,
        })
    }

    /// This slice's own governed default, matching the design note's role
    /// table verbatim. A real deployment supplies its own TOML instead of
    /// calling this.
    pub fn default_v1() -> Self {
        Self::from_toml_str(DEFAULT_V1_TOML).expect("crate's own default policy TOML must parse")
    }

    pub fn disposition_role_for_severity(&self, severity: &str) -> &str {
        self.disposition_role_by_severity.get(severity).map(String::as_str).unwrap_or(&self.disposition_default_role)
    }
}

const DEFAULT_V1_TOML: &str = r#"
[case_workflow]
assign_role = "RiskAnalyst"
escalate_role = "RiskAnalyst"
senior_review_role = "SeniorInvestigator"
disposition_default_role = "SeniorInvestigator"

[case_workflow.disposition_role_by_severity]
high = "ComplianceOfficer"
critical = "ComplianceOfficer"
"#;

#[derive(Debug, Deserialize)]
struct RawSarWorkflowPolicyFile {
    sar_workflow: RawSarWorkflowPolicy,
}

#[derive(Debug, Deserialize)]
struct RawSarWorkflowPolicy {
    draft_role: String,
    compliance_review_role: String,
    mlro_approval_role: String,
    submit_role: String,
}

/// `[sar_workflow]` policy: which role each SAR-workflow step requires --
/// same "loaded from data, not hardcoded" discipline as
/// [`CaseWorkflowPolicy`]. Matches the design note's role table
/// ("ComplianceOfficer: approve high-risk dispositions and SAR drafts";
/// "MLRO: approve or reject SAR submission").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SarWorkflowPolicy {
    pub draft_role: String,
    pub compliance_review_role: String,
    pub mlro_approval_role: String,
    pub submit_role: String,
}

impl SarWorkflowPolicy {
    pub fn from_toml_str(src: &str) -> Result<Self, CaseWorkflowPolicyError> {
        let raw: RawSarWorkflowPolicyFile = toml::from_str(src).map_err(|e| CaseWorkflowPolicyError::Parse(e.to_string()))?;
        let p = raw.sar_workflow;
        for (name, value) in [
            ("draft_role", &p.draft_role),
            ("compliance_review_role", &p.compliance_review_role),
            ("mlro_approval_role", &p.mlro_approval_role),
            ("submit_role", &p.submit_role),
        ] {
            if value.trim().is_empty() {
                return Err(CaseWorkflowPolicyError::EmptyField(name));
            }
        }
        Ok(SarWorkflowPolicy { draft_role: p.draft_role, compliance_review_role: p.compliance_review_role, mlro_approval_role: p.mlro_approval_role, submit_role: p.submit_role })
    }

    /// This slice's own governed default. A real deployment supplies its
    /// own TOML instead of calling this.
    pub fn default_v1() -> Self {
        Self::from_toml_str(SAR_DEFAULT_V1_TOML).expect("crate's own default SAR policy TOML must parse")
    }
}

const SAR_DEFAULT_V1_TOML: &str = r#"
[sar_workflow]
draft_role = "RiskAnalyst"
compliance_review_role = "ComplianceOfficer"
mlro_approval_role = "MLRO"
submit_role = "MLRO"
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sar_default_v1_matches_the_design_notes_role_table() {
        let policy = SarWorkflowPolicy::default_v1();
        assert_eq!(policy.compliance_review_role, "ComplianceOfficer");
        assert_eq!(policy.mlro_approval_role, "MLRO");
    }

    #[test]
    fn default_v1_routes_high_severity_disposition_to_compliance_officer() {
        let policy = CaseWorkflowPolicy::default_v1();
        assert_eq!(policy.disposition_role_for_severity("high"), "ComplianceOfficer");
        assert_eq!(policy.disposition_role_for_severity("medium"), "SeniorInvestigator");
        assert_eq!(policy.disposition_role_for_severity("low"), "SeniorInvestigator");
    }

    #[test]
    fn rejects_an_empty_governance_field() {
        let src = DEFAULT_V1_TOML.replace(r#"assign_role = "RiskAnalyst""#, r#"assign_role = "  ""#);
        assert_eq!(CaseWorkflowPolicy::from_toml_str(&src).unwrap_err(), CaseWorkflowPolicyError::EmptyField("assign_role"));
    }

    #[test]
    fn a_deployment_can_supply_a_different_policy_without_a_recompile() {
        let src = r#"
[case_workflow]
assign_role = "FraudAnalyst"
escalate_role = "FraudAnalyst"
senior_review_role = "FraudLead"
disposition_default_role = "FraudLead"

[case_workflow.disposition_role_by_severity]
high = "FraudDirector"
"#;
        let policy = CaseWorkflowPolicy::from_toml_str(src).unwrap();
        assert_eq!(policy.assign_role, "FraudAnalyst");
        assert_eq!(policy.disposition_role_for_severity("high"), "FraudDirector");
        assert_eq!(policy.disposition_role_for_severity("medium"), "FraudLead");
    }
}
