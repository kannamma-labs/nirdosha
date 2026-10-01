//! SAR (Suspicious Activity Report) workflow: `Candidate/Draft ->
//! ComplianceReviewed -> MLROApproved -> Submitted -> Acknowledged`,
//! bounded to that happy path plus one correction/resubmission step (the
//! design note's fuller state list -- `Returned`, general re-drafting --
//! is not built here; disclosed, not silently missing).
//!
//! A [`SarFiling`] only ever exists for a case whose disposition is
//! already `SuspiciousActivity` (`crate::case::CaseStatus`) -- SAR
//! candidacy is downstream of, not a replacement for, the case-disposition
//! gate `crate::case` already enforces. **Candidate and Draft are one
//! step here** (`sar.draft`), not two separate gateways: this slice has no
//! meaningful distinct "candidate, not yet drafted" state to protect
//! beyond what "the case was found suspicious" already establishes.
//!
//! Submission is real, but mock: [`CtmsSarSubmitGatewayV1::submit`] builds
//! a real goAML XML envelope (`nirdosha_egress_report_goaml::ReportEnvelope`),
//! validates it against the real XSD schema
//! (`nirdosha_egress_report_goaml::GoamlSchemaValidator`), and submits
//! through `nirdosha_egress_report_goaml::FileOutboxTransport` -- the same
//! "one honest implementation, vendor slot stays open" driver that
//! crate's own module doc describes, not a fabricated regulator
//! acknowledgement. **This never implies a real regulatory filing
//! happened.**
//!
//! **Known, disclosed data gap**: `crate::event::TransactionEvent` carries
//! no counterparty/payee field (see crate module doc's "External KYC and
//! banking APIs" gap), so the goAML envelope's `to` party is a clearly
//! labeled placeholder (`"counterparty-unknown"`), not fabricated real
//! counterparty data. A real deployment's KYC/counterparty fact provider
//! closes this.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use nirdosha_egress_report_goaml::{GoamlSchemaValidator, GuardedSubmitter, PartyRef, ReportCode, ReportEnvelope, ReportingPerson, SubmissionCode, SubmitError, Transaction};
use nirdosha_guard_rfc0029::{CapabilityIssuer, DecisionCapability, EffectGateway, GatewayCore, GatewayError};
use nirdosha_rt::Auth;

use crate::case::MonitoringCase;
use crate::policy::SarWorkflowPolicy;
use crate::rules::AlertCandidate;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SarStatus {
    Draft,
    ComplianceReviewed,
    MlroApproved,
    Submitted,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SarFiling {
    pub sar_id: String,
    pub case_id: String,
    pub alert_customer_key: String,
    /// The prior filing this one corrects/resubmits, if any.
    pub corrects_sar_id: Option<String>,
    pub narrative: String,
    pub status: SarStatus,
    pub drafted_by: String,
    pub compliance_reviewed_by: Option<String>,
    pub mlro_approved_by: Option<String>,
    pub submission_reference: Option<String>,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SarStoreError {
    UnknownFiling(String),
    CaseNotSuspiciousActivity,
    WrongState { expected: &'static str, actual: &'static str },
}

impl std::fmt::Display for SarStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SarStoreError::UnknownFiling(id) => write!(f, "no such SAR filing: {id}"),
            SarStoreError::CaseNotSuspiciousActivity => write!(f, "a SAR candidate can only be drafted for a case dispositioned SuspiciousActivity"),
            SarStoreError::WrongState { expected, actual } => write!(f, "SAR filing must be {expected} for this action, was {actual}"),
        }
    }
}

pub trait SarStore: Send + Sync {
    fn draft(&self, case: &MonitoringCase, narrative: &str, drafted_by: &str, corrects_sar_id: Option<&str>, now_ms: u64) -> Result<SarFiling, SarStoreError>;
    fn compliance_review(&self, sar_id: &str, actor: &str) -> Result<SarFiling, SarStoreError>;
    fn mlro_approve(&self, sar_id: &str, actor: &str) -> Result<SarFiling, SarStoreError>;
    fn mark_submitted(&self, sar_id: &str, reference: &str) -> Result<SarFiling, SarStoreError>;
    fn get(&self, sar_id: &str) -> Option<SarFiling>;
    fn list(&self) -> Vec<SarFiling>;
}

fn sar_status_name(status: SarStatus) -> &'static str {
    match status {
        SarStatus::Draft => "Draft",
        SarStatus::ComplianceReviewed => "ComplianceReviewed",
        SarStatus::MlroApproved => "MlroApproved",
        SarStatus::Submitted => "Submitted",
    }
}

#[derive(Default)]
struct Inner {
    filings: HashMap<String, SarFiling>,
    next_id: u64,
}

#[derive(Default)]
pub struct InMemorySarStore {
    inner: Mutex<Inner>,
}

impl InMemorySarStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SarStore for InMemorySarStore {
    fn draft(&self, case: &MonitoringCase, narrative: &str, drafted_by: &str, corrects_sar_id: Option<&str>, now_ms: u64) -> Result<SarFiling, SarStoreError> {
        if case.status != crate::case::CaseStatus::SuspiciousActivity {
            return Err(SarStoreError::CaseNotSuspiciousActivity);
        }
        let mut inner = self.inner.lock().expect("sar store lock poisoned");
        let sar_id = format!("sar-{}", inner.next_id);
        inner.next_id += 1;
        let filing = SarFiling {
            sar_id: sar_id.clone(),
            case_id: case.case_id.clone(),
            alert_customer_key: case.alert_id.clone(),
            corrects_sar_id: corrects_sar_id.map(str::to_string),
            narrative: narrative.to_string(),
            status: SarStatus::Draft,
            drafted_by: drafted_by.to_string(),
            compliance_reviewed_by: None,
            mlro_approved_by: None,
            submission_reference: None,
            created_at_ms: now_ms,
        };
        inner.filings.insert(sar_id, filing.clone());
        Ok(filing)
    }

    fn compliance_review(&self, sar_id: &str, actor: &str) -> Result<SarFiling, SarStoreError> {
        let mut inner = self.inner.lock().expect("sar store lock poisoned");
        let filing = inner.filings.get_mut(sar_id).ok_or_else(|| SarStoreError::UnknownFiling(sar_id.to_string()))?;
        if filing.status != SarStatus::Draft {
            return Err(SarStoreError::WrongState { expected: "Draft", actual: sar_status_name(filing.status) });
        }
        filing.status = SarStatus::ComplianceReviewed;
        filing.compliance_reviewed_by = Some(actor.to_string());
        Ok(filing.clone())
    }

    fn mlro_approve(&self, sar_id: &str, actor: &str) -> Result<SarFiling, SarStoreError> {
        let mut inner = self.inner.lock().expect("sar store lock poisoned");
        let filing = inner.filings.get_mut(sar_id).ok_or_else(|| SarStoreError::UnknownFiling(sar_id.to_string()))?;
        if filing.status != SarStatus::ComplianceReviewed {
            return Err(SarStoreError::WrongState { expected: "ComplianceReviewed", actual: sar_status_name(filing.status) });
        }
        filing.status = SarStatus::MlroApproved;
        filing.mlro_approved_by = Some(actor.to_string());
        Ok(filing.clone())
    }

    fn mark_submitted(&self, sar_id: &str, reference: &str) -> Result<SarFiling, SarStoreError> {
        let mut inner = self.inner.lock().expect("sar store lock poisoned");
        let filing = inner.filings.get_mut(sar_id).ok_or_else(|| SarStoreError::UnknownFiling(sar_id.to_string()))?;
        if filing.status != SarStatus::MlroApproved {
            return Err(SarStoreError::WrongState { expected: "MlroApproved", actual: sar_status_name(filing.status) });
        }
        filing.status = SarStatus::Submitted;
        filing.submission_reference = Some(reference.to_string());
        Ok(filing.clone())
    }

    fn get(&self, sar_id: &str) -> Option<SarFiling> {
        self.inner.lock().expect("sar store lock poisoned").filings.get(sar_id).cloned()
    }

    fn list(&self) -> Vec<SarFiling> {
        self.inner.lock().expect("sar store lock poisoned").filings.values().cloned().collect()
    }
}

macro_rules! sar_gateway {
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
        }
        impl EffectGateway for $name {
            const RESOURCE: &'static str = "SARFiling";
            const EFFECT: &'static str = $effect;
            const MODULE: &'static str = "rfc0029:ctms_sar_gateway_v1";
            fn core(&self) -> &GatewayCore {
                &self.core
            }
        }
    };
}

sar_gateway!(CtmsSarDraftGatewayV1, "sar.draft");
sar_gateway!(CtmsSarComplianceReviewGatewayV1, "sar.compliance_review");
sar_gateway!(CtmsSarMlroApprovalGatewayV1, "sar.mlro_approve");

impl CtmsSarDraftGatewayV1 {
    pub fn draft(&self, capability: &DecisionCapability, store: &dyn SarStore, case: &MonitoringCase, narrative: &str, drafted_by: &str, corrects_sar_id: Option<&str>, now_ms: u64) -> Result<SarFiling, GatewayError> {
        <Self as EffectGateway>::consume_and_execute(self, capability, now_ms, || store.draft(case, narrative, drafted_by, corrects_sar_id, now_ms).map_err(|e| e.to_string()))
    }
}

impl CtmsSarComplianceReviewGatewayV1 {
    pub fn compliance_review(&self, capability: &DecisionCapability, store: &dyn SarStore, sar_id: &str, actor: &str, now_ms: u64) -> Result<SarFiling, GatewayError> {
        <Self as EffectGateway>::consume_and_execute(self, capability, now_ms, || store.compliance_review(sar_id, actor).map_err(|e| e.to_string()))
    }
}

impl CtmsSarMlroApprovalGatewayV1 {
    pub fn mlro_approve(&self, capability: &DecisionCapability, store: &dyn SarStore, sar_id: &str, actor: &str, now_ms: u64) -> Result<SarFiling, GatewayError> {
        <Self as EffectGateway>::consume_and_execute(self, capability, now_ms, || store.mlro_approve(sar_id, actor).map_err(|e| e.to_string()))
    }
}

/// Reads `<this crate's manifest dir>/../nirdosha-egress-report-goaml/schema/goaml_report.xsd`
/// -- a real, checked-in sibling-crate resource, not a copy: both crates
/// live in the same workspace checkout.
pub fn default_goaml_schema_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../nirdosha-egress-report-goaml/schema/goaml_report.xsd")
}

/// The only thing allowed to turn an `MlroApproved` [`SarFiling`] into a
/// real (mock) regulator submission. Owns the real XSD validator and a
/// local outbox directory; `submit` builds the real goAML XML envelope
/// from the filing + case + rule-candidate data, validates it, and writes
/// it -- `GuardedSubmitter::submit_if_valid` never reaches the transport
/// for an invalid envelope (see `nirdosha_egress_report_goaml`'s own doc).
pub struct CtmsSarSubmitGatewayV1 {
    core: GatewayCore,
    validator: GoamlSchemaValidator,
    outbox_dir: PathBuf,
}

impl CtmsSarSubmitGatewayV1 {
    pub fn new(evidence_path: impl AsRef<Path>, issuer: &CapabilityIssuer, schema_path: impl AsRef<Path>, outbox_dir: impl Into<PathBuf>) -> Result<Self, Vec<String>> {
        let validator = GoamlSchemaValidator::from_xsd_file(schema_path)?;
        Ok(Self { core: GatewayCore::new(<Self as EffectGateway>::MODULE, evidence_path, issuer), validator, outbox_dir: outbox_dir.into() })
    }

    pub fn evidence(&self) -> &nirdosha_audit::envelope::ModuleAuditChain {
        self.core.evidence()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn submit(
        &self,
        capability: &DecisionCapability,
        store: &dyn SarStore,
        sar_id: &str,
        candidate: &AlertCandidate,
        reporting_entity_id: &str,
        reporting_entity_name: &str,
        jurisdiction: &str,
        date_transaction: &str,
        now_ms: u64,
    ) -> Result<SarFiling, GatewayError> {
        <Self as EffectGateway>::consume_and_execute(self, capability, now_ms, || {
            let filing = store.get(sar_id).ok_or_else(|| SarStoreError::UnknownFiling(sar_id.to_string()).to_string())?;
            if filing.status != SarStatus::MlroApproved {
                return Err(SarStoreError::WrongState { expected: "MlroApproved", actual: sar_status_name(filing.status) }.to_string());
            }
            let envelope = ReportEnvelope {
                report_code: ReportCode::Str,
                submission_code: if filing.corrects_sar_id.is_some() { SubmissionCode::Correction } else { SubmissionCode::New },
                reporting_person: ReportingPerson { entity_id: reporting_entity_id.to_string(), entity_name: reporting_entity_name.to_string(), jurisdiction: Some(jurisdiction.to_string()) },
                transactions: candidate
                    .matched_event_ids
                    .iter()
                    .map(|event_id| Transaction {
                        transaction_number: event_id.clone(),
                        transmode_code: "TRANSFER".to_string(),
                        date_transaction: date_transaction.to_string(),
                        amount_local: format!("{}.{:02}", candidate.debit_total_minor / 100, candidate.debit_total_minor % 100),
                        from: PartyRef { party_id: candidate.key.clone(), party_name: candidate.key.clone() },
                        // See module doc: this crate's TransactionEvent has
                        // no counterparty field yet -- clearly labeled
                        // placeholder, never fabricated real data.
                        to: PartyRef { party_id: "counterparty-unknown".to_string(), party_name: "counterparty-unknown".to_string() },
                    })
                    .collect(),
            };
            let transport = nirdosha_egress_report_goaml::FileOutboxTransport::new(&self.outbox_dir);
            let submitter = GuardedSubmitter::new(&self.validator, transport);
            let ack = submitter.submit_if_valid(&envelope).map_err(|e| match e {
                SubmitError::Invalid(errors) => format!("goAML envelope failed schema validation: {errors:?}"),
                SubmitError::Transport(err) => format!("mock regulator transport failed: {err:?}"),
            })?;
            store.mark_submitted(sar_id, &ack.reference).map_err(|e| e.to_string())
        })
    }
}

impl EffectGateway for CtmsSarSubmitGatewayV1 {
    const RESOURCE: &'static str = "SARFiling";
    const EFFECT: &'static str = "sar.submit";
    const MODULE: &'static str = "rfc0029:ctms_sar_gateway_v1";

    fn core(&self) -> &GatewayCore {
        &self.core
    }
}

#[derive(Debug)]
pub enum SarServiceError {
    Forbidden { required_role: String },
    Gateway(GatewayError),
}

fn require_role(auth: &Auth, required_role: &str) -> Result<(), SarServiceError> {
    if auth.has_role(required_role) {
        Ok(())
    } else {
        Err(SarServiceError::Forbidden { required_role: required_role.to_string() })
    }
}

/// `[service.sar_draft]`: `access = 'requires role "{policy.draft_role}"'`.
#[allow(clippy::too_many_arguments)]
pub fn draft_sar(
    issuer: &CapabilityIssuer,
    gateway: &CtmsSarDraftGatewayV1,
    store: &dyn SarStore,
    policy: &SarWorkflowPolicy,
    auth: &Auth,
    case: &MonitoringCase,
    narrative: &str,
    corrects_sar_id: Option<&str>,
    now_ms: u64,
) -> Result<SarFiling, SarServiceError> {
    require_role(auth, &policy.draft_role)?;
    let capability = issuer.mint(auth, CtmsSarDraftGatewayV1::RESOURCE, CtmsSarDraftGatewayV1::EFFECT, now_ms).map_err(SarServiceError::Gateway)?;
    gateway.draft(&capability, store, case, narrative, auth.user(), corrects_sar_id, now_ms).map_err(SarServiceError::Gateway)
}

/// `[service.sar_compliance_review]`.
pub fn compliance_review_sar(issuer: &CapabilityIssuer, gateway: &CtmsSarComplianceReviewGatewayV1, store: &dyn SarStore, policy: &SarWorkflowPolicy, auth: &Auth, sar_id: &str, now_ms: u64) -> Result<SarFiling, SarServiceError> {
    require_role(auth, &policy.compliance_review_role)?;
    let capability = issuer.mint(auth, CtmsSarComplianceReviewGatewayV1::RESOURCE, CtmsSarComplianceReviewGatewayV1::EFFECT, now_ms).map_err(SarServiceError::Gateway)?;
    gateway.compliance_review(&capability, store, sar_id, auth.user(), now_ms).map_err(SarServiceError::Gateway)
}

/// `[service.sar_mlro_approve]`.
pub fn mlro_approve_sar(issuer: &CapabilityIssuer, gateway: &CtmsSarMlroApprovalGatewayV1, store: &dyn SarStore, policy: &SarWorkflowPolicy, auth: &Auth, sar_id: &str, now_ms: u64) -> Result<SarFiling, SarServiceError> {
    require_role(auth, &policy.mlro_approval_role)?;
    let capability = issuer.mint(auth, CtmsSarMlroApprovalGatewayV1::RESOURCE, CtmsSarMlroApprovalGatewayV1::EFFECT, now_ms).map_err(SarServiceError::Gateway)?;
    gateway.mlro_approve(&capability, store, sar_id, auth.user(), now_ms).map_err(SarServiceError::Gateway)
}

/// `[service.sar_submit]`.
#[allow(clippy::too_many_arguments)]
pub fn submit_sar(
    issuer: &CapabilityIssuer,
    gateway: &CtmsSarSubmitGatewayV1,
    store: &dyn SarStore,
    policy: &SarWorkflowPolicy,
    auth: &Auth,
    sar_id: &str,
    candidate: &AlertCandidate,
    reporting_entity_id: &str,
    reporting_entity_name: &str,
    jurisdiction: &str,
    date_transaction: &str,
    now_ms: u64,
) -> Result<SarFiling, SarServiceError> {
    require_role(auth, &policy.submit_role)?;
    let capability = issuer.mint(auth, CtmsSarSubmitGatewayV1::RESOURCE, CtmsSarSubmitGatewayV1::EFFECT, now_ms).map_err(SarServiceError::Gateway)?;
    gateway
        .submit(&capability, store, sar_id, candidate, reporting_entity_id, reporting_entity_name, jurisdiction, date_transaction, now_ms)
        .map_err(SarServiceError::Gateway)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::case::CaseStatus;
    use nirdosha_guard_rfc0029::PolicyBundle;

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

    fn disposed_case() -> MonitoringCase {
        MonitoringCase {
            case_id: "case-0".to_string(),
            alert_id: "alert-0".to_string(),
            severity: "high".to_string(),
            assigned_analyst: Some("priya".to_string()),
            status: CaseStatus::SuspiciousActivity,
            escalated_by: Some("priya".to_string()),
            escalation_reason: Some("structuring".to_string()),
            senior_reviewed_by: Some("arjun".to_string()),
            disposition: Some("SuspiciousActivity".to_string()),
            disposition_by: Some("meera".to_string()),
            created_at_ms: 0,
        }
    }

    fn candidate() -> AlertCandidate {
        AlertCandidate {
            rule_id: "velocity_24h".to_string(),
            rule_version: 1,
            key: "cust-1".to_string(),
            window_start_ms: 0,
            window_end_ms: 1_000,
            severity: "high".to_string(),
            matched_event_ids: vec!["evt-0".to_string(), "evt-1".to_string()],
            debit_count: 5,
            debit_total_minor: 1_000_000,
        }
    }

    #[test]
    fn drafting_a_sar_for_a_case_not_dispositioned_suspicious_activity_is_rejected() {
        let store = InMemorySarStore::new();
        let mut case = disposed_case();
        case.status = CaseStatus::FalsePositive;
        let err = store.draft(&case, "narrative", "priya", None, 0).unwrap_err();
        assert_eq!(err, SarStoreError::CaseNotSuspiciousActivity);
    }

    #[test]
    fn full_sar_workflow_produces_a_real_validated_submitted_envelope_in_the_outbox() {
        let issuer = issuer();
        let now = now_ms();
        let case = disposed_case();

        let draft_gateway = CtmsSarDraftGatewayV1::new(std::env::temp_dir().join(format!("ctms_sar_test_{}", std::process::id())), &issuer);
        let review_gateway = CtmsSarComplianceReviewGatewayV1::new(std::env::temp_dir().join(format!("ctms_sar_test_review_{}", std::process::id())), &issuer);
        let mlro_gateway = CtmsSarMlroApprovalGatewayV1::new(std::env::temp_dir().join(format!("ctms_sar_test_mlro_{}", std::process::id())), &issuer);
        let outbox = std::env::temp_dir().join(format!("ctms_sar_outbox_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&outbox);
        let submit_gateway = CtmsSarSubmitGatewayV1::new(std::env::temp_dir().join(format!("ctms_sar_test_submit_{}", std::process::id())), &issuer, default_goaml_schema_path(), &outbox).unwrap();

        let store = InMemorySarStore::new();
        let policy = SarWorkflowPolicy::default_v1();

        let risk_analyst = Auth::login("priya", &["RiskAnalyst"]);
        let filing = draft_sar(&issuer, &draft_gateway, &store, &policy, &risk_analyst, &case, "5 rapid debits consistent with structuring", None, now).unwrap();
        assert_eq!(filing.status, SarStatus::Draft);

        let compliance_officer = Auth::login("meera", &["ComplianceOfficer"]);
        let reviewed = compliance_review_sar(&issuer, &review_gateway, &store, &policy, &compliance_officer, &filing.sar_id, now).unwrap();
        assert_eq!(reviewed.status, SarStatus::ComplianceReviewed);

        let mlro = Auth::login("vikram", &["MLRO"]);
        let approved = mlro_approve_sar(&issuer, &mlro_gateway, &store, &policy, &mlro, &filing.sar_id, now).unwrap();
        assert_eq!(approved.status, SarStatus::MlroApproved);

        // Wrong role rejected: compliance officer cannot submit (MLRO-only).
        let wrong_role = submit_sar(&issuer, &submit_gateway, &store, &policy, &compliance_officer, &filing.sar_id, &candidate(), "FIU-0001", "Test Bank", "IN", "2026-09-27", now);
        assert!(matches!(wrong_role, Err(SarServiceError::Forbidden { ref required_role }) if required_role == "MLRO"));

        let submitted = submit_sar(&issuer, &submit_gateway, &store, &policy, &mlro, &filing.sar_id, &candidate(), "FIU-0001", "Test Bank", "IN", "2026-09-27", now).unwrap();
        assert_eq!(submitted.status, SarStatus::Submitted);
        let reference = submitted.submission_reference.unwrap();
        let written = std::fs::read_to_string(outbox.join(format!("{reference}.xml"))).expect("a real, schema-valid XML file must exist in the outbox");
        assert!(written.contains("<report_code>STR</report_code>"));
        assert!(written.contains("counterparty-unknown"));

        let _ = std::fs::remove_dir_all(&outbox);
    }

    #[test]
    fn a_correction_filing_references_the_original_and_submits_with_submission_code_correction() {
        let issuer = issuer();
        let now = now_ms();
        let case = disposed_case();
        let draft_gateway = CtmsSarDraftGatewayV1::new(std::env::temp_dir().join(format!("ctms_sar_test_corr_draft_{}", std::process::id())), &issuer);
        let review_gateway = CtmsSarComplianceReviewGatewayV1::new(std::env::temp_dir().join(format!("ctms_sar_test_corr_review_{}", std::process::id())), &issuer);
        let mlro_gateway = CtmsSarMlroApprovalGatewayV1::new(std::env::temp_dir().join(format!("ctms_sar_test_corr_mlro_{}", std::process::id())), &issuer);
        let outbox = std::env::temp_dir().join(format!("ctms_sar_outbox_corr_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&outbox);
        let submit_gateway = CtmsSarSubmitGatewayV1::new(std::env::temp_dir().join(format!("ctms_sar_test_corr_submit_{}", std::process::id())), &issuer, default_goaml_schema_path(), &outbox).unwrap();
        let store = InMemorySarStore::new();
        let policy = SarWorkflowPolicy::default_v1();
        let risk_analyst = Auth::login("priya", &["RiskAnalyst"]);
        let compliance_officer = Auth::login("meera", &["ComplianceOfficer"]);
        let mlro = Auth::login("vikram", &["MLRO"]);

        let original = draft_sar(&issuer, &draft_gateway, &store, &policy, &risk_analyst, &case, "original narrative", None, now).unwrap();
        compliance_review_sar(&issuer, &review_gateway, &store, &policy, &compliance_officer, &original.sar_id, now).unwrap();
        mlro_approve_sar(&issuer, &mlro_gateway, &store, &policy, &mlro, &original.sar_id, now).unwrap();
        let submitted_original = submit_sar(&issuer, &submit_gateway, &store, &policy, &mlro, &original.sar_id, &candidate(), "FIU-0001", "Test Bank", "IN", "2026-09-27", now).unwrap();

        let correction = draft_sar(&issuer, &draft_gateway, &store, &policy, &risk_analyst, &case, "corrected amount", Some(&submitted_original.sar_id), now).unwrap();
        assert_eq!(correction.corrects_sar_id.as_deref(), Some(submitted_original.sar_id.as_str()));
        compliance_review_sar(&issuer, &review_gateway, &store, &policy, &compliance_officer, &correction.sar_id, now).unwrap();
        mlro_approve_sar(&issuer, &mlro_gateway, &store, &policy, &mlro, &correction.sar_id, now).unwrap();
        let submitted_correction = submit_sar(&issuer, &submit_gateway, &store, &policy, &mlro, &correction.sar_id, &candidate(), "FIU-0001", "Test Bank", "IN", "2026-09-27", now).unwrap();

        let reference = submitted_correction.submission_reference.unwrap();
        let written = std::fs::read_to_string(outbox.join(format!("{reference}.xml"))).unwrap();
        assert!(written.contains("<submission_code>C</submission_code>"), "a correction filing must submit with submission_code C: {written}");

        let _ = std::fs::remove_dir_all(&outbox);
    }

    #[test]
    fn submitting_before_mlro_approval_is_rejected() {
        let issuer = issuer();
        let now = now_ms();
        let case = disposed_case();
        let draft_gateway = CtmsSarDraftGatewayV1::new(std::env::temp_dir().join(format!("ctms_sar_test_wrongstate_draft_{}", std::process::id())), &issuer);
        let outbox = std::env::temp_dir().join(format!("ctms_sar_outbox_wrongstate_{}", std::process::id()));
        let submit_gateway = CtmsSarSubmitGatewayV1::new(std::env::temp_dir().join(format!("ctms_sar_test_wrongstate_submit_{}", std::process::id())), &issuer, default_goaml_schema_path(), &outbox).unwrap();
        let store = InMemorySarStore::new();
        let policy = SarWorkflowPolicy::default_v1();
        let risk_analyst = Auth::login("priya", &["RiskAnalyst"]);
        let filing = draft_sar(&issuer, &draft_gateway, &store, &policy, &risk_analyst, &case, "narrative", None, now).unwrap();

        let mlro = Auth::login("vikram", &["MLRO"]);
        let result = submit_sar(&issuer, &submit_gateway, &store, &policy, &mlro, &filing.sar_id, &candidate(), "FIU-0001", "Test Bank", "IN", "2026-09-27", now);
        assert!(result.is_err(), "submitting a mere Draft (not yet compliance-reviewed/MLRO-approved) must be rejected");
        let _ = std::fs::remove_dir_all(&outbox);
    }
}
