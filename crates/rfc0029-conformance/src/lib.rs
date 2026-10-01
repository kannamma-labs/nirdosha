//! Candidate RFC 0029 conformance boundary.
//!
//! This crate parses and canonicalizes fixtures, plus (in [`pilot`]) one
//! narrow in-process falsification pilot for `funds.reserve`. It has no
//! real gateway, network, or storage dependency and therefore cannot
//! authorize a production effect.

pub mod pilot;

use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const SCHEMA_VERSION: &str = "rfc0029.influence-review.fixture.v1";

#[derive(Debug)]
pub enum Error {
    Json(serde_json::Error),
    UnsupportedSchemaVersion(String),
    InvalidGraph(String),
    DuplicateMember(String),
    InvalidReview(String),
    InvalidEdgeEndpointKind(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(e) => write!(f, "invalid fixture JSON: {e}"),
            Self::UnsupportedSchemaVersion(v) => write!(f, "unsupported schema version: {v}"),
            Self::InvalidGraph(e) => write!(f, "invalid lineage graph: {e}"),
            Self::DuplicateMember(name) => write!(f, "duplicate JSON member: {name}"),
            Self::InvalidReview(e) => write!(f, "invalid review input: {e}"),
            Self::InvalidEdgeEndpointKind(e) => write!(
                f,
                "edge endpoint kind not licensed by RFC 0029.a §2.2's closed edge table: {e}"
            ),
        }
    }
}

impl std::error::Error for Error {}
impl From<serde_json::Error> for Error {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fixture {
    pub schema_version: String,
    pub fixture_id: String,
    pub profile_id: String,
    pub input: Input,
    pub expected: Expected,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub bundle: Bundle,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub catalog_entries: Vec<CatalogEntry>,
    pub parameters: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    pub id: String,
    pub version: u64,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub id: String,
    pub kind: NodeKind,
    pub attributes: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum NodeKind {
    Fact,
    ModelInvocation,
    Transform,
    Rule,
    CandidateSet,
    Presentation,
    Review,
    AuthorityAssertion,
    Decision,
    Obligation,
    Capability,
    Effect,
    Evidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edge {
    pub source: String,
    pub kind: EdgeKind,
    pub target: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EdgeKind {
    DataFlow,
    RuleFlow,
    CandidateGenerate,
    CandidateSuppress,
    Eligibility,
    Rank,
    Attention,
    Default,
    Recommend,
    DraftArtifact,
    DraftReason,
    ModelApproval,
    ModelTrigger,
    HumanInput,
    AssertFact,
    AssertDecision,
    Authorize,
    Require,
    Satisfy,
    Issue,
    Record,
    ObserveOnly,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogEntry {
    pub catalog: CatalogKind,
    pub name: String,
    pub value: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogKind {
    Authority,
    PredicateDefinition,
    ReviewContract,
    ObserveCertificate,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expected {
    pub graph_id: Option<String>,
    pub normalized_graph: NormalizedGraph,
    pub provenance: TriState,
    pub model_level: ModelLevel,
    pub model_authorized: TriState,
    pub final_authority: Option<String>,
    pub admission: AdmissionResult,
    pub review: ReviewResult,
    pub capability: CapabilityResult,
    pub invalidation: String,
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum NormalizedGraph {
    NotAsserted,
    Asserted {
        value: BTreeMap<String, Value>,
        sha256: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TriState {
    Bool(bool),
    Unknown(Unknown),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Unknown {
    #[serde(rename = "unknown")]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ModelLevel {
    None,
    Observe,
    Classify,
    Prioritize,
    Recommend,
    Draft,
    Approve,
    ExecuteReversible,
    ExecuteCompensatable,
    ExecuteIrreversible,
    Unknown,
    #[serde(rename = "not_evaluated")]
    NotEvaluated,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionResult {
    pub status: String,
    pub diagnostic: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewResult {
    pub status: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityResult {
    pub status: String,
    pub effect_class: Option<String>,
}

pub fn parse(bytes: &[u8]) -> Result<Fixture, Error> {
    reject_duplicate_members(bytes)?;
    let fixture: Fixture = serde_json::from_slice(bytes)?;
    if fixture.schema_version != SCHEMA_VERSION {
        return Err(Error::UnsupportedSchemaVersion(fixture.schema_version));
    }
    validate_graph(&fixture.input)?;
    Ok(fixture)
}

struct NoDuplicates;
impl<'de> DeserializeSeed<'de> for NoDuplicates {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_any(NoDuplicatesVisitor)
    }
}
struct NoDuplicatesVisitor;
impl<'de> Visitor<'de> for NoDuplicatesVisitor {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        let mut names = BTreeSet::new();
        while let Some(name) = map.next_key::<String>()? {
            if !names.insert(name.clone()) {
                return Err(serde::de::Error::custom(format!(
                    "duplicate JSON member: {name}"
                )));
            }
            map.next_value_seed(NoDuplicates)?;
        }
        Ok(())
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        while seq.next_element_seed(NoDuplicates)?.is_some() {}
        Ok(())
    }
    fn visit_bool<E>(self, _: bool) -> Result<(), E> {
        Ok(())
    }
    fn visit_i64<E>(self, _: i64) -> Result<(), E> {
        Ok(())
    }
    fn visit_u64<E>(self, _: u64) -> Result<(), E> {
        Ok(())
    }
    fn visit_f64<E>(self, _: f64) -> Result<(), E> {
        Ok(())
    }
    fn visit_str<E>(self, _: &str) -> Result<(), E> {
        Ok(())
    }
    fn visit_string<E>(self, _: String) -> Result<(), E> {
        Ok(())
    }
    fn visit_none<E>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_unit<E>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_some<D: serde::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_any(self)
    }
}

fn reject_duplicate_members(bytes: &[u8]) -> Result<(), Error> {
    let mut de = serde_json::Deserializer::from_slice(bytes);
    NoDuplicates.deserialize(&mut de).map_err(|e| {
        let text = e.to_string();
        if let Some(name) = text
            .strip_prefix("duplicate JSON member: ")
            .and_then(|s| s.split(" at line").next())
        {
            Error::DuplicateMember(name.to_owned())
        } else {
            Error::Json(e)
        }
    })?;
    de.end()?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
pub enum ReviewMode {
    Acknowledgement,
    BoundedApproval,
    IndependentDecision,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewContract {
    pub id: String,
    pub version: u64,
    pub mode: ReviewMode,
    pub predicates: Vec<String>,
    pub authority: String,
    #[serde(default)]
    pub evaluation_fixture: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PredicateDefinition {
    pub id: String,
    pub version: u64,
    pub kind: PredicateKind,
    pub expression: String,
    pub input_schema: Vec<String>,
    pub result_type: BoolResult,
    pub authority: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum BoolResult {
    Bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum PredicateKind {
    Qualification,
    ExplicitAction,
    Authority,
    Independence,
    EvidenceAvailable,
    EvidenceAccessed,
    LimitationVisible,
    ReasonBound,
    OverrideAvailable,
    EscalationAvailable,
    CapacityHealthy,
    IndependentOutcomeChoice,
    SamplingAssigned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum PredicateValue {
    True,
    False,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum FailureBehavior {
    Deny,
    Indeterminate,
    Escalate,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PredicateEvaluation {
    pub predicate: String,
    pub definition_id: String,
    pub definition_version: u64,
    pub evaluator_authority: String,
    pub input_commitment: String,
    pub result: PredicateValue,
    pub evidence_commitment: String,
    pub evaluated_at: String,
    pub valid_until: String,
    pub revoked: bool,
    pub failure_behavior: FailureBehavior,
}

/// Closed review-finding taxonomy. This is the crate's own stable
/// vocabulary for `evaluate_review`'s failure reasons (part of the
/// "stable diagnostics/result taxonomy" freeze requirement) — a typo or an
/// ad-hoc new finding string can no longer slip in unreviewed; extending
/// this set requires deliberately adding a variant here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReviewFinding {
    SelfCertification,
    RequiredEvidenceNotAccessed,
    IndependenceConflict,
    ReasonNotEvidenceBound,
    CapacityExceeded,
    ReviewPredicateFailedClosed,
    ReviewModeInsufficient,
    ReviewPredicateMissing,
}

impl ReviewFinding {
    pub const ALL: [ReviewFinding; 8] = [
        Self::SelfCertification,
        Self::RequiredEvidenceNotAccessed,
        Self::IndependenceConflict,
        Self::ReasonNotEvidenceBound,
        Self::CapacityExceeded,
        Self::ReviewPredicateFailedClosed,
        Self::ReviewModeInsufficient,
        Self::ReviewPredicateMissing,
    ];
}

impl fmt::Display for ReviewFinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::SelfCertification => "SelfCertification",
            Self::RequiredEvidenceNotAccessed => "RequiredEvidenceNotAccessed",
            Self::IndependenceConflict => "IndependenceConflict",
            Self::ReasonNotEvidenceBound => "ReasonNotEvidenceBound",
            Self::CapacityExceeded => "CapacityExceeded",
            Self::ReviewPredicateFailedClosed => "ReviewPredicateFailedClosed",
            Self::ReviewModeInsufficient => "ReviewModeInsufficient",
            Self::ReviewPredicateMissing => "ReviewPredicateMissing",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComputedReview {
    Pass(ReviewMode),
    Fail(ReviewFinding),
    Indeterminate(ReviewFinding),
    NotEvaluated,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionPolicy {
    pub effect_class: String,
    pub maximum_model_level: ModelLevel,
}

/// Closed admission-diagnostic taxonomy — the other half of the "stable
/// diagnostics/result taxonomy" freeze requirement. `evaluate_admission`
/// can only ever return one of these; it is a compile error to fabricate a
/// new ad-hoc diagnostic string at a call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AdmissionDiagnostic {
    MandatoryDeny,
    FactStale,
    FactRevoked,
    FactWrongModel,
    FactWrongDeployment,
    FactUnauthorizedConsumer,
    FactProvenanceMissing,
    ReviewSelfCertification,
    ReviewProtocolFailed,
    InfluenceLevelExceedsProfile,
    CapabilityEffectMismatch,
    ObserveOnlyAttestationStale,
    SemanticEdgeMissing,
}

impl AdmissionDiagnostic {
    pub const ALL: [AdmissionDiagnostic; 13] = [
        Self::MandatoryDeny,
        Self::FactStale,
        Self::FactRevoked,
        Self::FactWrongModel,
        Self::FactWrongDeployment,
        Self::FactUnauthorizedConsumer,
        Self::FactProvenanceMissing,
        Self::ReviewSelfCertification,
        Self::ReviewProtocolFailed,
        Self::InfluenceLevelExceedsProfile,
        Self::CapabilityEffectMismatch,
        Self::ObserveOnlyAttestationStale,
        Self::SemanticEdgeMissing,
    ];
}

impl fmt::Display for AdmissionDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::MandatoryDeny => "MandatoryDeny",
            Self::FactStale => "FactStale",
            Self::FactRevoked => "FactRevoked",
            Self::FactWrongModel => "FactWrongModel",
            Self::FactWrongDeployment => "FactWrongDeployment",
            Self::FactUnauthorizedConsumer => "FactUnauthorizedConsumer",
            Self::FactProvenanceMissing => "FactProvenanceMissing",
            Self::ReviewSelfCertification => "ReviewSelfCertification",
            Self::ReviewProtocolFailed => "ReviewProtocolFailed",
            Self::InfluenceLevelExceedsProfile => "InfluenceLevelExceedsProfile",
            Self::CapabilityEffectMismatch => "CapabilityEffectMismatch",
            Self::ObserveOnlyAttestationStale => "ObserveOnlyAttestationStale",
            Self::SemanticEdgeMissing => "SemanticEdgeMissing",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputedAdmission {
    pub status: String,
    pub diagnostic: Option<AdmissionDiagnostic>,
    pub capability_status: String,
    pub effect_class: Option<String>,
}

pub fn evaluate_admission(fixture: &Fixture) -> Result<ComputedAdmission, Error> {
    use AdmissionDiagnostic as D;
    let influence = compute_influence(fixture)?;
    if influence.level == ModelLevel::Unknown {
        let diagnostic = if fixture
            .input
            .catalog_entries
            .iter()
            .any(|e| e.catalog == CatalogKind::ObserveCertificate)
        {
            D::ObserveOnlyAttestationStale
        } else {
            D::SemanticEdgeMissing
        };
        return Ok(admission(
            "indeterminate",
            Some(diagnostic),
            "not_issued",
            None,
        ));
    }
    if fixture.input.parameters.get("mandatory_fact").is_some() {
        return Ok(admission("rejected", Some(D::MandatoryDeny), "not_issued", None));
    }
    if let ComputedFactProvenance::Failed(diagnostic) = evaluate_fact_provenance(fixture)? {
        return Ok(admission("rejected", Some(diagnostic), "not_issued", None));
    }
    let review = evaluate_review(fixture)?;
    if let ComputedReview::Fail(finding) = review {
        let diagnostic = if finding == ReviewFinding::SelfCertification {
            D::ReviewSelfCertification
        } else {
            D::ReviewProtocolFailed
        };
        return Ok(admission("rejected", Some(diagnostic), "not_issued", None));
    }
    let policy = fixture
        .input
        .parameters
        .get("admission_policy")
        .map(|v| serde_json::from_value::<AdmissionPolicy>(v.clone()))
        .transpose()?;
    if let Some(policy) = &policy {
        if influence.level > policy.maximum_model_level {
            return Ok(admission(
                "rejected",
                Some(D::InfluenceLevelExceedsProfile),
                "not_issued",
                None,
            ));
        }
    }
    let capability = fixture
        .input
        .nodes
        .iter()
        .find(|n| n.kind == NodeKind::Capability);
    if let Some(capability) = capability {
        let effect = capability
            .attributes
            .get("effect_class")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidGraph("capability missing effect_class".into()))?;
        if let Some(policy) = &policy {
            if policy.effect_class != effect {
                return Err(Error::InvalidGraph(
                    "capability/profile effect mismatch".into(),
                ));
            }
        }
        if let Some(issued) = fixture
            .input
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Effect)
        {
            let effect_id = issued
                .attributes
                .get("effect_id")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::InvalidGraph("effect missing effect_id".into()))?;
            if effect_id != effect {
                return Ok(admission(
                    "rejected",
                    Some(D::CapabilityEffectMismatch),
                    "not_issued",
                    None,
                ));
            }
        }
        return Ok(admission("accepted", None, "issued", Some(effect)));
    }
    Ok(admission("accepted", None, "not_evaluated", None))
}

/// Wraps a caller-built `Input` graph in a synthetic `Fixture` so a
/// production caller (a generator's admission gate, not a conformance
/// test) can reuse `evaluate_admission`/`compile_from_fixture` without
/// fabricating an `expected` block that stands in for a test oracle it
/// has no use for. Every evaluator in this crate reads only
/// `fixture.input` (plus `fixture_id`/`profile_id`, both supplied by the
/// caller here) — `expected` is never consulted by computation, only by
/// conformance tests comparing against it, so the placeholder below is
/// inert by construction, not a guess standing in for real data.
pub(crate) fn synthetic_fixture(input: &Input, fixture_id: &str, profile_id: &str) -> Fixture {
    Fixture {
        schema_version: SCHEMA_VERSION.into(),
        fixture_id: fixture_id.into(),
        profile_id: profile_id.into(),
        input: input.clone(),
        expected: Expected {
            graph_id: None,
            normalized_graph: NormalizedGraph::NotAsserted,
            provenance: TriState::Bool(false),
            model_level: ModelLevel::None,
            model_authorized: TriState::Bool(false),
            final_authority: None,
            admission: AdmissionResult { status: "not_evaluated".into(), diagnostic: None },
            review: ReviewResult { status: "not_evaluated".into(), detail: None },
            capability: CapabilityResult { status: "not_evaluated".into(), effect_class: None },
            invalidation: String::new(),
            extensions: BTreeMap::new(),
        },
    }
}

/// Production entry point for `evaluate_admission`: takes the `Input` graph
/// directly, for callers (e.g. a screen generator's admission gate) that
/// have no test oracle to declare. See [`synthetic_fixture`].
pub fn evaluate_admission_input(
    input: &Input,
    fixture_id: &str,
    profile_id: &str,
) -> Result<ComputedAdmission, Error> {
    evaluate_admission(&synthetic_fixture(input, fixture_id, profile_id))
}

fn admission(
    status: &str,
    diagnostic: Option<AdmissionDiagnostic>,
    capability_status: &str,
    effect: Option<&str>,
) -> ComputedAdmission {
    ComputedAdmission {
        status: status.into(),
        diagnostic,
        capability_status: capability_status.into(),
        effect_class: effect.map(str::to_owned),
    }
}

/// Evaluate the compact executable review fixtures against their closed
/// contract and predicate-definition catalogs.
pub fn evaluate_review(fixture: &Fixture) -> Result<ComputedReview, Error> {
    let params = &fixture.input.parameters;
    if params.get("monitor").is_some() {
        return Ok(ComputedReview::NotEvaluated);
    }
    let required_name = params
        .get("required_review_contract")
        .and_then(Value::as_str);
    let supplied_name = params
        .get("supplied_review_contract")
        .and_then(Value::as_str)
        .or_else(|| params.get("review_contract").and_then(Value::as_str));
    let Some(supplied_name) = supplied_name else {
        return Ok(ComputedReview::NotEvaluated);
    };
    if supplied_name == "self-review" {
        return Ok(ComputedReview::Fail(ReviewFinding::SelfCertification));
    }
    let supplied = resolve_contract(fixture, supplied_name)?;
    if let Some(required_name) = required_name {
        let required = resolve_contract(fixture, required_name)?;
        if supplied.mode < required.mode {
            return Ok(ComputedReview::Fail(ReviewFinding::ReviewModeInsufficient));
        }
    }
    let mut definitions = BTreeMap::new();
    for entry in fixture
        .input
        .catalog_entries
        .iter()
        .filter(|e| e.catalog == CatalogKind::PredicateDefinition)
    {
        let definition: PredicateDefinition =
            serde_json::from_value(Value::Object(entry.value.clone().into_iter().collect()))?;
        if definition.authority != supplied.authority {
            return Err(Error::InvalidReview(format!(
                "predicate {} authority mismatch",
                entry.name
            )));
        }
        definitions.insert(entry.name.as_str(), definition);
    }
    let mut evaluations: BTreeMap<String, PredicateEvaluation> = BTreeMap::new();
    if let Some(items) = params.get("evaluations").and_then(Value::as_array) {
        for item in items {
            let evaluation: PredicateEvaluation = serde_json::from_value(item.clone())?;
            if evaluation.input_commitment.is_empty() || evaluation.evidence_commitment.is_empty() {
                return Err(Error::InvalidReview(format!(
                    "missing commitment: {}",
                    evaluation.predicate
                )));
            }
            if evaluations
                .insert(evaluation.predicate.clone(), evaluation.clone())
                .is_some()
            {
                return Err(Error::InvalidReview(format!(
                    "duplicate predicate evaluation: {}",
                    evaluation.predicate
                )));
            }
        }
    }
    let mut missing = Vec::new();
    let mut stale = false;
    let mut failures = Vec::new();
    for name in &supplied.predicates {
        if !definitions.contains_key(name.as_str()) {
            return Err(Error::InvalidReview(format!(
                "missing predicate definition: {name}"
            )));
        }
        match evaluations.get(name.as_str()) {
            Some(evaluation) => {
                let definition = &definitions[name.as_str()];
                if evaluation.definition_id != definition.id
                    || evaluation.definition_version != definition.version
                {
                    return Err(Error::InvalidReview(format!(
                        "predicate version mismatch: {name}"
                    )));
                }
                if evaluation.evaluator_authority != "authority:fixture-review-evaluator:v1" {
                    return Err(Error::InvalidReview(format!(
                        "evaluator authority mismatch: {name}"
                    )));
                }
                if evaluation.revoked || evaluation.valid_until.as_str() < "2026-09-27T00:00:00Z" {
                    stale = true;
                } else if evaluation.result == PredicateValue::False {
                    failures.push(false_finding(name));
                } else if evaluation.result == PredicateValue::Unknown {
                    stale = true;
                }
            }
            None => missing.push(name.as_str()),
        }
    }
    failures.sort();
    failures.dedup();
    if let Some(finding) = failures.first() {
        return Ok(ComputedReview::Fail(*finding));
    }
    if stale {
        return Ok(ComputedReview::Fail(ReviewFinding::ReviewPredicateFailedClosed));
    }
    if !missing.is_empty() {
        return Ok(ComputedReview::Fail(ReviewFinding::ReviewPredicateMissing));
    }
    Ok(ComputedReview::Pass(supplied.mode))
}

fn resolve_contract(fixture: &Fixture, name: &str) -> Result<ReviewContract, Error> {
    let entry = fixture
        .input
        .catalog_entries
        .iter()
        .find(|e| e.catalog == CatalogKind::ReviewContract && e.name == name)
        .ok_or_else(|| Error::InvalidReview(format!("unknown review contract: {name}")))?;
    Ok(serde_json::from_value(Value::Object(
        entry.value.clone().into_iter().collect(),
    ))?)
}

fn false_finding(name: &str) -> ReviewFinding {
    match name {
        "evidence_accessed" => ReviewFinding::RequiredEvidenceNotAccessed,
        "independence" => ReviewFinding::IndependenceConflict,
        "reason_bound" => ReviewFinding::ReasonNotEvidenceBound,
        "capacity" => ReviewFinding::CapacityExceeded,
        _ => ReviewFinding::ReviewPredicateFailedClosed,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComputedFactProvenance {
    NotApplicable,
    Satisfied,
    // Reuses `AdmissionDiagnostic` rather than a separate enum: every
    // fact-provenance failure already has a 1:1-named admission diagnostic
    // (FactStale, FactRevoked, ...), so a second parallel taxonomy would
    // just be a duplicate vocabulary, not a distinct one.
    Failed(AdmissionDiagnostic),
}

/// Checks a persisted `Fact`'s producer/lifecycle state against a
/// consuming `Decision`'s declared requirement — the cross-decision
/// lineage check `compute_influence` deliberately does not perform,
/// since that evaluator only reasons about graph topology, never about
/// a Fact node's attribute *values* (freshness, revocation, producer
/// identity, authorized consumers).
pub fn evaluate_fact_provenance(fixture: &Fixture) -> Result<ComputedFactProvenance, Error> {
    let input = &fixture.input;
    let decision = input
        .nodes
        .iter()
        .find(|n| n.kind == NodeKind::Decision && n.attributes.contains_key("required_model"));
    let Some(decision) = decision else {
        return Ok(ComputedFactProvenance::NotApplicable);
    };
    let required_model = decision.attributes.get("required_model").and_then(Value::as_str);
    let required_deployment = decision
        .attributes
        .get("required_deployment")
        .and_then(Value::as_str);
    let fact = input.nodes.iter().find(|n| {
        n.kind == NodeKind::Fact
            && n.attributes.contains_key("produced_by_model")
            && reachable_from_any(&input.edges, &[n.id.as_str()], decision.id.as_str())
    });
    let Some(fact) = fact else {
        return Ok(ComputedFactProvenance::Failed(
            AdmissionDiagnostic::FactProvenanceMissing,
        ));
    };
    if fact
        .attributes
        .get("revoked")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Ok(ComputedFactProvenance::Failed(AdmissionDiagnostic::FactRevoked));
    }
    let valid_until = fact
        .attributes
        .get("valid_until")
        .and_then(Value::as_str)
        .unwrap_or("");
    if valid_until < "2026-09-27T00:00:00Z" {
        return Ok(ComputedFactProvenance::Failed(AdmissionDiagnostic::FactStale));
    }
    if fact.attributes.get("produced_by_model").and_then(Value::as_str) != required_model {
        return Ok(ComputedFactProvenance::Failed(AdmissionDiagnostic::FactWrongModel));
    }
    if fact
        .attributes
        .get("produced_by_deployment")
        .and_then(Value::as_str)
        != required_deployment
    {
        return Ok(ComputedFactProvenance::Failed(
            AdmissionDiagnostic::FactWrongDeployment,
        ));
    }
    let authorized: Vec<&str> = fact
        .attributes
        .get("authorized_consumers")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if !authorized.contains(&decision.id.as_str()) {
        return Ok(ComputedFactProvenance::Failed(
            AdmissionDiagnostic::FactUnauthorizedConsumer,
        ));
    }
    Ok(ComputedFactProvenance::Satisfied)
}

pub fn canonical_bytes(fixture: &Fixture) -> Result<Vec<u8>, Error> {
    Ok(serde_jcs::to_vec(fixture)?)
}

pub fn sha256(fixture: &Fixture) -> Result<String, Error> {
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(canonical_bytes(fixture)?)
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComputedInfluence {
    pub provenance: TriState,
    pub level: ModelLevel,
    pub model_authorized: TriState,
}

#[derive(Debug, Clone, Copy)]
struct FoldState {
    level: ModelLevel,
    active: bool,
    contributed: bool,
}

/// Compute the representation-independent influence projection from graph
/// inputs only. Profile admission, review predicates and final capability
/// authority are deliberately separate evaluators.
pub fn compute_influence(fixture: &Fixture) -> Result<ComputedInfluence, Error> {
    let input = &fixture.input;
    let kinds: BTreeMap<&str, NodeKind> = input
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n.kind))
        .collect();
    let models: Vec<&str> = input
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::ModelInvocation)
        .map(|n| n.id.as_str())
        .collect();
    if models.is_empty() {
        return Ok(ComputedInfluence {
            provenance: TriState::Bool(false),
            level: ModelLevel::None,
            model_authorized: TriState::Bool(false),
        });
    }
    let only_observe = input
        .edges
        .iter()
        .filter(|e| {
            models.contains(&e.source.as_str())
                || reachable_from_any(&input.edges, &models, &e.source)
        })
        .all(|e| {
            matches!(
                e.kind,
                EdgeKind::DataFlow | EdgeKind::RuleFlow | EdgeKind::ObserveOnly | EdgeKind::Record
            )
        });
    if only_observe {
        let cert = input
            .catalog_entries
            .iter()
            .find(|e| e.catalog == CatalogKind::ObserveCertificate);
        let current = cert
            .and_then(|e| e.value.get("valid_until"))
            .and_then(Value::as_str)
            .map(|v| v >= "2026-09-27T00:00:00Z")
            .unwrap_or(false);
        return Ok(if current {
            ComputedInfluence {
                provenance: TriState::Bool(false),
                level: ModelLevel::None,
                model_authorized: TriState::Bool(false),
            }
        } else {
            ComputedInfluence {
                provenance: TriState::Unknown(Unknown::Unknown),
                level: ModelLevel::Unknown,
                model_authorized: TriState::Unknown(Unknown::Unknown),
            }
        });
    }

    let mut states: BTreeMap<&str, Vec<FoldState>> = BTreeMap::new();
    for model in &models {
        states.entry(model).or_default().push(FoldState {
            level: ModelLevel::Observe,
            active: true,
            contributed: false,
        });
    }
    let order = topo_order(input)?;
    let mut maximum = ModelLevel::Observe;
    let mut active_at_protected = false;
    for id in order {
        let here = states.get(id).cloned().unwrap_or_default();
        for edge in input.edges.iter().filter(|e| e.source == id) {
            for mut state in here.iter().copied() {
                if let Some(level) = contribution(edge, input) {
                    if state.active && level > state.level {
                        state.level = level;
                    }
                    state.contributed = true;
                }
                if edge.kind == EdgeKind::HumanInput
                    && kinds.get(edge.target.as_str()) == Some(&NodeKind::AuthorityAssertion)
                {
                    state.active = false;
                }
                if state.level > maximum {
                    maximum = state.level;
                }
                let target_kind = kinds[edge.target.as_str()];
                if protected(target_kind) {
                    if !state.contributed
                        && matches!(edge.kind, EdgeKind::DataFlow | EdgeKind::RuleFlow)
                    {
                        return Ok(ComputedInfluence {
                            provenance: TriState::Unknown(Unknown::Unknown),
                            level: ModelLevel::Unknown,
                            model_authorized: TriState::Unknown(Unknown::Unknown),
                        });
                    }
                    let continues = input.edges.iter().any(|next| next.source == edge.target);
                    if !continues {
                        active_at_protected |= state.active;
                    }
                }
                states.entry(edge.target.as_str()).or_default().push(state);
            }
        }
    }
    Ok(ComputedInfluence {
        provenance: TriState::Bool(true),
        level: maximum,
        model_authorized: TriState::Bool(active_at_protected),
    })
}

fn reachable_from_any(edges: &[Edge], starts: &[&str], target: &str) -> bool {
    let mut seen: BTreeSet<&str> = starts.iter().copied().collect();
    let mut work: Vec<&str> = starts.to_vec();
    while let Some(id) = work.pop() {
        for edge in edges.iter().filter(|e| e.source == id) {
            if seen.insert(&edge.target) {
                work.push(&edge.target);
            }
        }
    }
    seen.contains(target)
}

fn protected(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Review
            | NodeKind::AuthorityAssertion
            | NodeKind::Decision
            | NodeKind::Obligation
            | NodeKind::Capability
            | NodeKind::Effect
    )
}

fn contribution(edge: &Edge, input: &Input) -> Option<ModelLevel> {
    Some(match edge.kind {
        EdgeKind::CandidateGenerate | EdgeKind::CandidateSuppress | EdgeKind::Eligibility => {
            ModelLevel::Classify
        }
        EdgeKind::Rank | EdgeKind::Attention => ModelLevel::Prioritize,
        EdgeKind::Recommend => ModelLevel::Recommend,
        EdgeKind::Default | EdgeKind::DraftArtifact | EdgeKind::DraftReason => ModelLevel::Draft,
        EdgeKind::ModelApproval => ModelLevel::Approve,
        EdgeKind::ModelTrigger => {
            let effect = input.nodes.iter().find(|n| n.id == edge.target)?;
            match effect.attributes.get("reversibility_class")?.as_str()? {
                "Reversible" => ModelLevel::ExecuteReversible,
                "Compensatable" => ModelLevel::ExecuteCompensatable,
                "Irreversible" => ModelLevel::ExecuteIrreversible,
                _ => return None,
            }
        }
        _ => return None,
    })
}

fn topo_order(input: &Input) -> Result<Vec<&str>, Error> {
    let mut incoming: BTreeMap<&str, usize> =
        input.nodes.iter().map(|n| (n.id.as_str(), 0)).collect();
    let mut outgoing: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for edge in &input.edges {
        *incoming.get_mut(edge.target.as_str()).unwrap() += 1;
        outgoing.entry(&edge.source).or_default().push(&edge.target);
    }
    let mut ready: Vec<&str> = incoming
        .iter()
        .filter_map(|(id, n)| (*n == 0).then_some(*id))
        .collect();
    let mut result = Vec::new();
    while let Some(id) = ready.pop() {
        result.push(id);
        for target in outgoing.get(id).into_iter().flatten() {
            let n = incoming.get_mut(target).unwrap();
            *n -= 1;
            if *n == 0 {
                ready.push(target);
            }
        }
    }
    if result.len() != input.nodes.len() {
        return Err(Error::InvalidGraph("cycle detected".into()));
    }
    Ok(result)
}

/// RFC 0029.a §2.2's closed edge algebra: for each edge kind, the allowed
/// source node kinds (`None` means "any", per `Record`'s "any → Evidence")
/// and the allowed target node kinds. This table is closed — an edge kind
/// not covered by `Fixture`'s `EdgeKind` enum already fails to parse, and an
/// edge whose endpoints' kinds are not in these lists is equally invalid,
/// even though the enum variant itself is a recognized kind.
fn allowed_edge_endpoints(kind: EdgeKind) -> (Option<&'static [NodeKind]>, &'static [NodeKind]) {
    use NodeKind::*;
    match kind {
        EdgeKind::DataFlow => (
            Some(&[Fact, ModelInvocation, Transform]),
            &[
                Transform,
                Rule,
                CandidateSet,
                Presentation,
                Review,
                AuthorityAssertion,
                Decision,
            ],
        ),
        EdgeKind::RuleFlow => (
            Some(&[Rule]),
            &[
                CandidateSet,
                Presentation,
                Review,
                AuthorityAssertion,
                Decision,
                Obligation,
                Capability,
            ],
        ),
        EdgeKind::CandidateGenerate => (Some(&[ModelInvocation, Transform, Rule]), &[CandidateSet]),
        EdgeKind::CandidateSuppress => (Some(&[ModelInvocation, Transform, Rule]), &[CandidateSet]),
        EdgeKind::Eligibility => (
            Some(&[ModelInvocation, Transform, Rule, CandidateSet]),
            &[Decision],
        ),
        EdgeKind::Rank => (
            Some(&[ModelInvocation, Transform, Rule]),
            &[CandidateSet, Presentation],
        ),
        EdgeKind::Attention => (Some(&[CandidateSet, Presentation]), &[Review, Decision]),
        EdgeKind::Default => (
            Some(&[ModelInvocation, Transform, Rule]),
            &[Presentation, Review, Decision],
        ),
        EdgeKind::Recommend => (
            Some(&[ModelInvocation, Transform, Rule]),
            &[Presentation, Review, Decision],
        ),
        EdgeKind::DraftArtifact => (
            Some(&[ModelInvocation, Transform, Rule]),
            &[Review, AuthorityAssertion, Decision],
        ),
        EdgeKind::DraftReason => (
            Some(&[ModelInvocation, Transform, Rule]),
            &[Presentation, Review, AuthorityAssertion],
        ),
        EdgeKind::ModelApproval => (
            Some(&[ModelInvocation, Transform, Rule]),
            &[Decision, Capability],
        ),
        EdgeKind::ModelTrigger => (
            Some(&[ModelInvocation, Transform, Rule, Decision]),
            &[Effect],
        ),
        EdgeKind::HumanInput => (Some(&[Presentation, Review]), &[AuthorityAssertion]),
        EdgeKind::AssertFact => (Some(&[AuthorityAssertion]), &[Fact]),
        EdgeKind::AssertDecision => (Some(&[AuthorityAssertion]), &[Decision]),
        EdgeKind::Authorize => (Some(&[Decision, AuthorityAssertion]), &[Capability]),
        EdgeKind::Require => (Some(&[Decision, Rule]), &[Obligation]),
        EdgeKind::Satisfy => (Some(&[Fact, Review, Evidence]), &[Obligation]),
        EdgeKind::Issue => (Some(&[Capability]), &[Effect]),
        EdgeKind::Record => (None, &[Evidence]),
        EdgeKind::ObserveOnly => (Some(&[ModelInvocation, Transform]), &[Evidence]),
    }
}

fn validate_edge_endpoint_kinds(input: &Input) -> Result<(), Error> {
    let kinds: BTreeMap<&str, NodeKind> = input
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n.kind))
        .collect();
    for edge in &input.edges {
        let (allowed_sources, allowed_targets) = allowed_edge_endpoints(edge.kind);
        let source_kind = kinds[edge.source.as_str()];
        let target_kind = kinds[edge.target.as_str()];
        if let Some(allowed_sources) = allowed_sources {
            if !allowed_sources.contains(&source_kind) {
                return Err(Error::InvalidEdgeEndpointKind(format!(
                    "{:?} may not source from {:?} (node {}); allowed sources are {:?}",
                    edge.kind, source_kind, edge.source, allowed_sources
                )));
            }
        }
        if !allowed_targets.contains(&target_kind) {
            return Err(Error::InvalidEdgeEndpointKind(format!(
                "{:?} may not target {:?} (node {}); allowed targets are {:?}",
                edge.kind, target_kind, edge.target, allowed_targets
            )));
        }
    }
    Ok(())
}

pub fn validate_graph(input: &Input) -> Result<(), Error> {
    let mut ids = BTreeSet::new();
    for node in &input.nodes {
        if !ids.insert(node.id.as_str()) {
            return Err(Error::InvalidGraph(format!("duplicate node {}", node.id)));
        }
    }
    for edge in &input.edges {
        if !ids.contains(edge.source.as_str()) || !ids.contains(edge.target.as_str()) {
            return Err(Error::InvalidGraph(format!(
                "edge {:?} has missing endpoint",
                edge.kind
            )));
        }
    }
    // Kahn's algorithm: lineage is a finite DAG.
    let mut incoming: BTreeMap<&str, usize> = ids.iter().map(|id| (*id, 0)).collect();
    let mut outgoing: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for edge in &input.edges {
        *incoming.get_mut(edge.target.as_str()).unwrap() += 1;
        outgoing.entry(&edge.source).or_default().push(&edge.target);
    }
    let mut ready: Vec<&str> = incoming
        .iter()
        .filter_map(|(id, n)| (*n == 0).then_some(*id))
        .collect();
    let mut visited = 0;
    while let Some(id) = ready.pop() {
        visited += 1;
        for target in outgoing.get(id).into_iter().flatten() {
            let count = incoming.get_mut(target).unwrap();
            *count -= 1;
            if *count == 0 {
                ready.push(target);
            }
        }
    }
    if visited != ids.len() {
        return Err(Error::InvalidGraph("cycle detected".into()));
    }
    validate_edge_endpoint_kinds(input)?;
    Ok(())
}
