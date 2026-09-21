//! The real `cargo nirdosha verify --guard` pipeline
//! (`nirdosha_guard_registry::dump_json()` -> `nirdosha_guard_verify::RegistryView::from_json`
//! -> `verify()`), run against **this crate's own** registered corpus —
//! not the doc-derived copy `crates/nirdosha-rt/tests/rtm_policy_corpus.rs`
//! already covers. Same acceptance bar that file holds itself to.

extern crate rtm;

#[test]
fn rtm_registry_dump_round_trips_through_verify_with_only_expected_findings() {
    let json = nirdosha_guard_registry::dump_json().expect("registry must serialize");
    let registry = nirdosha_guard_verify::RegistryView::from_json(&json).expect("dump must deserialize into RegistryView");
    assert_eq!(registry.policies.len(), nirdosha_guard_registry::POLICIES.len());

    let findings = nirdosha_guard_verify::verify(&registry);
    let by_pass = |pass: &str| findings.iter().filter(|f| f.pass == pass).count();
    for finding in &findings {
        println!("  {} {:?}: {} ({:?})", finding.pass, finding.severity, finding.message, finding.item);
    }

    // V1/V2/V3: every one of the 114 registrations has a well-formed
    // id/action/resource/effect (the `for`/`in-list` parsing plus clause
    // lowering produced valid records for the whole corpus).
    assert_eq!(by_pass("V1"), 0, "V1 findings: {findings:?}");
    assert_eq!(by_pass("V2"), 0, "V2 findings: {findings:?}");
    assert_eq!(by_pass("V3"), 0, "V3 findings: {findings:?}");
    // V4: none of the corpus's real filters/conditions negate a relation.
    assert_eq!(by_pass("V4"), 0, "V4 findings: {findings:?}");
    // V5: needs driver manifests for #[dataset]-attached stores; this
    // corpus never registers any (pure policy/catalog declarations).
    assert_eq!(by_pass("V5"), 0, "V5 findings: {findings:?}");
    // V8 *does* have real work: `stream_port!` registers 3 real ports
    // (txn_in, txn_out, decisions) with no DRIVER_MANIFESTS for any of
    // them (driver installation is separate infra work this policy/catalog
    // corpus never does) — V8 correctly flags all 3. Same expected shape
    // as crates/nirdosha-rt/tests/rtm_policy_corpus.rs's identical check.
    let v8 = findings.iter().filter(|f| f.pass == "V8").collect::<Vec<_>>();
    assert_eq!(v8.len(), 3, "V8 findings: {findings:?}");
    for port in ["txn_in", "txn_out", "decisions"] {
        assert!(v8.iter().any(|f| f.item.as_deref() == Some(port)), "expected a V8 finding for port `{port}`: {findings:?}");
    }
}
