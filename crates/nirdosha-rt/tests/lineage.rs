//! RFC 0026 scaffold smoke test: the lineage macros parse through
//! `nirdosha_rt`, and the lineage crate is reachable as `nirdosha_rt::lineage`.
//! Expansion (query builders, catalog slices) lands per the RFC's phasing.

nirdosha_rt::lineage_query! {
    view downstream_of(start: nirdosha_rt::lineage::NodeId, depth: u32) -> DownstreamRow {
        node: nirdosha_rt::lineage::NodeId,
        edge_type: nirdosha_rt::lineage::EdgeType,
        policy_version: String [filter],
    };
}

nirdosha_rt::data_contract! {
    contract txn_v2 { entity = "transaction"; owner = "payments-platform"; }
}

#[test]
fn lineage_types_reachable_through_rt() {
    let node = nirdosha_rt::lineage::NodeId::data("txn_events".to_owned(), None);
    assert_eq!(node.kind, nirdosha_rt::lineage::NodeKind::Data);

    // The observation seam exists and is callable (I18 shape, no driver).
    let obs = nirdosha_rt::lineage::LineageObservation::from_context(
        "svc:ingest",
        &"trace-1".to_owned(),
        "user-1",
        "tenant-1",
        "2025.11.4".to_owned(),
        nirdosha_rt::lineage::Purpose("fraud_monitoring".to_owned()),
        nirdosha_rt::lineage::Destination::ApiClient,
        [7u8; 32],
        nirdosha_rt::lineage::PlanFacts {
            edge_type: nirdosha_rt::lineage::EdgeType::Read,
            transformation: nirdosha_rt::lineage::TransformId::Policy("p".to_owned()),
            driver: nirdosha_rt::lineage::DriverRef {
                port: "StreamPort".to_owned(),
                vendor: "kafka".to_owned(),
                version: "0.12.0".to_owned(),
            },
            authority: nirdosha_rt::lineage::Authority::KernelExecution,
            completeness: nirdosha_rt::lineage::FlowCompleteness::Full,
            sampled: false,
            degraded: false,
            sink: nirdosha_guard_core::LineageEntity {
                entity: "txn_events".to_owned(),
                keys: vec![],
            },
        },
    );
    let content = obs.to_audit_content();
    assert_eq!(content["kind"], "lineage");
}
