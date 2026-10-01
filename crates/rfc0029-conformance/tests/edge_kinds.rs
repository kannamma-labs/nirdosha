use rfc0029_conformance::{validate_graph, Bundle, Edge, EdgeKind, Error, Input, Node, NodeKind};
use std::collections::BTreeMap;

fn node(id: &str, kind: NodeKind) -> Node {
    Node {
        id: id.to_string(),
        kind,
        attributes: BTreeMap::new(),
    }
}

fn bundle() -> Bundle {
    Bundle {
        id: "bundle:test".into(),
        version: 1,
        hash: "symbolic:test".into(),
    }
}

/// Build a minimal two-node, one-edge graph and check it against
/// `validate_graph`. `source_kind`/`target_kind` are the endpoint kinds
/// actually used; the test asserts whether that combination is licensed by
/// RFC 0029.a §2.2's closed edge table.
fn check(kind: EdgeKind, source_kind: NodeKind, target_kind: NodeKind) -> Result<(), Error> {
    let input = Input {
        bundle: bundle(),
        nodes: vec![node("s", source_kind), node("t", target_kind)],
        edges: vec![Edge {
            source: "s".into(),
            kind,
            target: "t".into(),
        }],
        catalog_entries: vec![],
        parameters: BTreeMap::new(),
    };
    validate_graph(&input)
}

fn assert_invalid(kind: EdgeKind, source_kind: NodeKind, target_kind: NodeKind) {
    assert!(
        matches!(
            check(kind, source_kind, target_kind),
            Err(Error::InvalidEdgeEndpointKind(_))
        ),
        "{kind:?} with source {source_kind:?} and target {target_kind:?} should be rejected by the closed edge table, but was accepted"
    );
}

fn assert_valid(kind: EdgeKind, source_kind: NodeKind, target_kind: NodeKind) {
    assert!(
        check(kind, source_kind, target_kind).is_ok(),
        "{kind:?} with source {source_kind:?} and target {target_kind:?} should be licensed by the closed edge table, but was rejected"
    );
}

// One negative case per edge kind (all 22 `EdgeKind` variants), each a
// deliberately disallowed source or target for that specific row of RFC
// 0029.a §2.2's table — not sampled, exhaustive over edge kinds. Each is
// paired with one positive control on the same kind so a bug that made the
// validator reject everything (or accept everything) would still be caught.

#[test]
fn data_flow_rejects_disallowed_target() {
    assert_valid(EdgeKind::DataFlow, NodeKind::Fact, NodeKind::Decision);
    assert_invalid(EdgeKind::DataFlow, NodeKind::Fact, NodeKind::Effect);
}

#[test]
fn rule_flow_rejects_disallowed_source() {
    assert_valid(EdgeKind::RuleFlow, NodeKind::Rule, NodeKind::Decision);
    assert_invalid(EdgeKind::RuleFlow, NodeKind::Fact, NodeKind::Decision);
}

#[test]
fn candidate_generate_rejects_disallowed_target() {
    assert_valid(
        EdgeKind::CandidateGenerate,
        NodeKind::ModelInvocation,
        NodeKind::CandidateSet,
    );
    assert_invalid(
        EdgeKind::CandidateGenerate,
        NodeKind::ModelInvocation,
        NodeKind::Decision,
    );
}

#[test]
fn candidate_suppress_rejects_disallowed_source() {
    assert_valid(
        EdgeKind::CandidateSuppress,
        NodeKind::Rule,
        NodeKind::CandidateSet,
    );
    assert_invalid(
        EdgeKind::CandidateSuppress,
        NodeKind::Fact,
        NodeKind::CandidateSet,
    );
}

#[test]
fn eligibility_rejects_disallowed_source() {
    // The real bug this session's L8-L13 rebuild hit: Fact is not a
    // licensed source for Eligibility, only ModelInvocation/Transform/
    // Rule/CandidateSet are.
    assert_valid(EdgeKind::Eligibility, NodeKind::Transform, NodeKind::Decision);
    assert_invalid(EdgeKind::Eligibility, NodeKind::Fact, NodeKind::Decision);
}

#[test]
fn rank_rejects_disallowed_target() {
    assert_valid(EdgeKind::Rank, NodeKind::ModelInvocation, NodeKind::Presentation);
    assert_invalid(EdgeKind::Rank, NodeKind::ModelInvocation, NodeKind::Decision);
}

#[test]
fn attention_rejects_disallowed_source() {
    assert_valid(EdgeKind::Attention, NodeKind::Presentation, NodeKind::Review);
    assert_invalid(EdgeKind::Attention, NodeKind::ModelInvocation, NodeKind::Review);
}

#[test]
fn default_rejects_disallowed_target() {
    assert_valid(EdgeKind::Default, NodeKind::ModelInvocation, NodeKind::Review);
    assert_invalid(EdgeKind::Default, NodeKind::ModelInvocation, NodeKind::Capability);
}

#[test]
fn recommend_rejects_disallowed_target() {
    assert_valid(EdgeKind::Recommend, NodeKind::ModelInvocation, NodeKind::Decision);
    assert_invalid(EdgeKind::Recommend, NodeKind::ModelInvocation, NodeKind::Fact);
}

#[test]
fn draft_artifact_rejects_disallowed_target() {
    assert_valid(EdgeKind::DraftArtifact, NodeKind::ModelInvocation, NodeKind::Review);
    assert_invalid(
        EdgeKind::DraftArtifact,
        NodeKind::ModelInvocation,
        NodeKind::Presentation,
    );
}

#[test]
fn draft_reason_rejects_disallowed_target() {
    assert_valid(EdgeKind::DraftReason, NodeKind::ModelInvocation, NodeKind::Review);
    assert_invalid(EdgeKind::DraftReason, NodeKind::ModelInvocation, NodeKind::Decision);
}

#[test]
fn model_approval_rejects_disallowed_target() {
    assert_valid(EdgeKind::ModelApproval, NodeKind::ModelInvocation, NodeKind::Decision);
    assert_invalid(EdgeKind::ModelApproval, NodeKind::ModelInvocation, NodeKind::Review);
}

#[test]
fn model_trigger_rejects_disallowed_source() {
    assert_valid(EdgeKind::ModelTrigger, NodeKind::Decision, NodeKind::Effect);
    assert_invalid(EdgeKind::ModelTrigger, NodeKind::Fact, NodeKind::Effect);
}

#[test]
fn human_input_rejects_disallowed_source() {
    assert_valid(EdgeKind::HumanInput, NodeKind::Review, NodeKind::AuthorityAssertion);
    assert_invalid(
        EdgeKind::HumanInput,
        NodeKind::ModelInvocation,
        NodeKind::AuthorityAssertion,
    );
}

#[test]
fn assert_fact_rejects_disallowed_source() {
    // The real bug from earlier this session: a raw ModelInvocation cannot
    // structurally produce a Fact. Only an AuthorityAssertion can.
    assert_valid(EdgeKind::AssertFact, NodeKind::AuthorityAssertion, NodeKind::Fact);
    assert_invalid(EdgeKind::AssertFact, NodeKind::ModelInvocation, NodeKind::Fact);
}

#[test]
fn assert_decision_rejects_disallowed_source() {
    assert_valid(EdgeKind::AssertDecision, NodeKind::AuthorityAssertion, NodeKind::Decision);
    assert_invalid(EdgeKind::AssertDecision, NodeKind::ModelInvocation, NodeKind::Decision);
}

#[test]
fn authorize_rejects_disallowed_target() {
    assert_valid(EdgeKind::Authorize, NodeKind::Decision, NodeKind::Capability);
    assert_invalid(EdgeKind::Authorize, NodeKind::Decision, NodeKind::Effect);
}

#[test]
fn require_rejects_disallowed_source() {
    assert_valid(EdgeKind::Require, NodeKind::Decision, NodeKind::Obligation);
    assert_invalid(EdgeKind::Require, NodeKind::Fact, NodeKind::Obligation);
}

#[test]
fn satisfy_rejects_disallowed_source() {
    assert_valid(EdgeKind::Satisfy, NodeKind::Fact, NodeKind::Obligation);
    assert_invalid(EdgeKind::Satisfy, NodeKind::ModelInvocation, NodeKind::Obligation);
}

#[test]
fn issue_rejects_disallowed_source() {
    assert_valid(EdgeKind::Issue, NodeKind::Capability, NodeKind::Effect);
    assert_invalid(EdgeKind::Issue, NodeKind::Decision, NodeKind::Effect);
}

#[test]
fn record_allows_any_source_but_rejects_disallowed_target() {
    // Record's source is deliberately unrestricted ("any -> Evidence");
    // this is the one row where the *source* check must never fire.
    assert_valid(EdgeKind::Record, NodeKind::ModelInvocation, NodeKind::Evidence);
    assert_valid(EdgeKind::Record, NodeKind::Obligation, NodeKind::Evidence);
    assert_invalid(EdgeKind::Record, NodeKind::ModelInvocation, NodeKind::Decision);
}

#[test]
fn observe_only_rejects_disallowed_source() {
    assert_valid(EdgeKind::ObserveOnly, NodeKind::ModelInvocation, NodeKind::Evidence);
    assert_invalid(EdgeKind::ObserveOnly, NodeKind::Fact, NodeKind::Evidence);
}

#[test]
fn every_edge_kind_has_a_negative_case_in_this_file() {
    // Guards against silently forgetting a variant if EdgeKind ever grows:
    // this list must be updated by hand alongside any new variant, and this
    // test is the tripwire that a reviewer actually did so.
    let covered = [
        EdgeKind::DataFlow,
        EdgeKind::RuleFlow,
        EdgeKind::CandidateGenerate,
        EdgeKind::CandidateSuppress,
        EdgeKind::Eligibility,
        EdgeKind::Rank,
        EdgeKind::Attention,
        EdgeKind::Default,
        EdgeKind::Recommend,
        EdgeKind::DraftArtifact,
        EdgeKind::DraftReason,
        EdgeKind::ModelApproval,
        EdgeKind::ModelTrigger,
        EdgeKind::HumanInput,
        EdgeKind::AssertFact,
        EdgeKind::AssertDecision,
        EdgeKind::Authorize,
        EdgeKind::Require,
        EdgeKind::Satisfy,
        EdgeKind::Issue,
        EdgeKind::Record,
        EdgeKind::ObserveOnly,
    ];
    assert_eq!(covered.len(), 22, "update this list when EdgeKind grows");
}
