use nirdosha_graph::{Graph, hash, store::Access};
use serde_json::{Value, json};

fn project() -> (tempfile::TempDir, Graph) {
    let d = tempfile::tempdir().unwrap();
    let g = Graph::initialize(
        &d.path().join("project"),
        &d.path().join("host"),
        Access::reviewer("alice"),
    )
    .unwrap();
    (d, g)
}
fn patch(g: &Graph, key: &str, ops: Value, pins: Value) -> Value {
    let v = g.version().unwrap();
    json!({"schema_version":"nirdosha.hi.patch/v1","project_id":v.project_id,"epoch":v.epoch,"mutation_id":key,"preconditions":pins,"operations":ops,"evidence":[]})
}
fn create(g: &Graph, name: &str) -> Value {
    patch(
        g,
        name,
        json!([{"op":"node.create","local_id":"@n","kind":"Struct","title":name,"spec":{"fields":{},"field_order":[]}}]),
        json!([]),
    )
}
fn node(g: &Graph, id: &str) -> Value {
    g.get("node", id, None).unwrap()["entity"].clone()
}
fn pin(g: &Graph, id: &str) -> Value {
    json!({"entity_type":"node","id":id,"entity_revision":node(g,id)["entity_revision"]})
}

#[test]
fn strict_json_and_jcs_have_stable_hashes() {
    assert!(hash::parse(r#"{"x":1,"x":2}"#).is_err());
    assert!(hash::parse(r#"{"x":{"a":1,"a":2}}"#).is_err());
    assert!(hash::parse(r#"{"x":9007199254740992}"#).is_err());
    let a = hash::parse(r#"{"b":2,"a":1.0}"#).unwrap();
    let b = hash::parse(r#"{"a":1,"b":2}"#).unwrap();
    assert_eq!(
        hash::structured("patch", &a).unwrap(),
        hash::structured("patch", &b).unwrap()
    );
    assert_eq!(
        hash::bytes(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn lost_ack_replay_survives_reopen_and_returns_original_identity() {
    let (d, g) = project();
    let p = create(&g, "first");
    let receipt = g.apply(&p).unwrap();
    let original_id = receipt["id_map"]["@n"].clone();
    drop(g);
    let g = Graph::open(
        &d.path().join("project"),
        &d.path().join("host"),
        Access::reviewer("alice"),
    )
    .unwrap();
    g.apply(&create(&g, "other")).unwrap();
    assert_eq!(g.apply(&p).unwrap(), receipt);
    assert_eq!(g.version().unwrap().revision, 2);
    assert_eq!(original_id, receipt["id_map"]["@n"]);
    let mut wrong = p;
    wrong["operations"][0]["title"] = "different".into();
    assert_eq!(g.apply(&wrong).unwrap_err().code, "IDEMPOTENCY_CONFLICT");
}

#[test]
fn malformed_multi_operation_patch_is_atomic() {
    let (_d, g) = project();
    let mut p = create(&g, "first");
    p["operations"].as_array_mut().unwrap().push(json!({"op":"node.create","local_id":"@bad","kind":"Function","title":"oops","spec":{"dialect":"v1"}}));
    assert_eq!(g.apply(&p).unwrap_err().code, "SCHEMA_INVALID");
    assert_eq!(g.version().unwrap().revision, 0);
    assert_eq!(
        g.page(&json!({})).unwrap()["nodes"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn stream_replay_close_order_and_direct_binding_are_enforced() {
    let (_d, g) = project();
    let s = g.stream_open("session").unwrap()["stream_id"]
        .as_str()
        .unwrap()
        .to_string();
    let p = create(&g, "one");
    assert_eq!(g.stream_append(&s, 2, &p).unwrap_err().code, "OUT_OF_ORDER");
    let r = g.stream_append(&s, 1, &p).unwrap();
    assert_eq!(g.apply(&p).unwrap_err().code, "MUTATION_BINDING_CONFLICT");
    g.stream_close(&s, 1, false).unwrap();
    assert_eq!(g.stream_append(&s, 1, &p).unwrap(), r);
    assert_eq!(
        g.stream_append(&s, 2, &create(&g, "two")).unwrap_err().code,
        "STREAM_CLOSED"
    );
}

#[test]
fn revision_conflict_and_noop_do_not_advance_changes() {
    let (_d, g) = project();
    let r = g.apply(&create(&g, "one")).unwrap();
    let id = r["id_map"]["@n"].as_str().unwrap();
    let p = patch(
        &g,
        "noop",
        json!([{"op":"spec.set","node_id":id,"path":"/fields","value":{}}]),
        json!([pin(&g, id)]),
    );
    let ack = g.apply(&p).unwrap();
    assert_eq!(ack["revision"], 1);
    assert!(ack["changed_entities"].as_array().unwrap().is_empty());
    let mut p = patch(
        &g,
        "edit",
        json!([{"op":"node.rename","node_id":id,"title":"renamed"}]),
        json!([pin(&g, id)]),
    );
    g.apply(&p).unwrap();
    p["mutation_id"] = "stale".into();
    assert_eq!(g.apply(&p).unwrap_err().code, "REVISION_CONFLICT");
}

#[test]
fn dependency_pins_required_and_acceptance_never_auto_includes() {
    let (_d, g) = project();
    let p = patch(
        &g,
        "pair",
        json!([
            {"op":"node.create","local_id":"@b","kind":"Struct","title":"B","spec":{"fields":{}}},
            {"op":"node.create","local_id":"@a","kind":"Struct","title":"A","spec":{"fields":{"f":{"name":"b","type":{"tag":"named","node_id":"@b"}}}}}
        ]),
        json!([]),
    );
    let r = g.apply(&p).unwrap();
    let a = r["id_map"]["@a"].as_str().unwrap();
    let b = r["id_map"]["@b"].as_str().unwrap();
    let mut acc = json!({"mutation_id":"accept","base_acceptance_id":null,"selections":[pin(&g,a)],"dependency_pins":[{"id":b,"entity_revision":1}]});
    assert_eq!(
        g.accept(&acc).unwrap_err().code,
        "ACCEPTANCE_DEPENDENCY_MISSING"
    );
    acc["selections"].as_array_mut().unwrap().push(pin(&g, b));
    let accepted = g.accept(&acc).unwrap();
    let p = patch(
        &g,
        "rename_b",
        json!([{"op":"node.rename","node_id":b,"title":"new B"}]),
        json!([pin(&g, b)]),
    );
    g.apply(&p).unwrap();
    let acc = json!({"mutation_id":"next","base_acceptance_id":accepted["acceptance_id"],"selections":[pin(&g,b)],"dependency_pins":[]});
    assert_eq!(
        g.accept(&acc).unwrap_err().code,
        "ACCEPTANCE_DEPENDENCY_MISSING"
    );
    let p = patch(
        &g,
        "rename_a",
        json!([{"op":"node.rename","node_id":a,"title":"new A"}]),
        json!([pin(&g, a)]),
    );
    assert_eq!(g.apply(&p).unwrap_err().code, "PRECONDITION_REQUIRED");
}

#[test]
fn snapshot_is_stable_and_revocation_blocks_cached_access() {
    let (_d, g) = project();
    let r = g.apply(&create(&g, "one")).unwrap();
    let id = r["id_map"]["@n"].as_str().unwrap();
    let token = g.new_snapshot("proposed").unwrap();
    let p = patch(
        &g,
        "rename",
        json!([{"op":"node.rename","node_id":id,"title":"new"}]),
        json!([pin(&g, id)]),
    );
    g.apply(&p).unwrap();
    assert_eq!(
        g.get("node", id, Some(&token)).unwrap()["entity"]["title"],
        "one"
    );
    let export = g.export(Some(&token)).unwrap();
    let artifact = export["artifact_id"].as_str().unwrap();
    g.read_artifact(artifact, 0, 16).unwrap();
    g.revoke_grant("alice:reviewer").unwrap();
    assert_eq!(
        g.read_artifact(artifact, 0, 16).unwrap_err().code,
        "GRANT_REVOKED"
    );
    assert_eq!(g.apply(&p).unwrap_err().code, "GRANT_REVOKED");
}

#[test]
fn full_graph_pages_do_not_silently_stop_at_two_thousand() {
    let (_d, g) = project();
    for batch in 0..21 {
        let ops:Vec<_>=(0..100).map(|i|json!({"op":"node.create","local_id":format!("@n{i}"),"kind":"Requirement","title":format!("n{batch}_{i}"),"spec":{"text":"a requirement"}})).collect();
        g.apply(&patch(&g, &format!("batch{batch}"), json!(ops), json!([])))
            .unwrap();
    }
    let mut args = json!({});
    let mut count = 0;
    loop {
        let p = g.page(&args).unwrap();
        count += p["nodes"].as_array().unwrap().len();
        if p["selection_complete"] == true {
            break;
        }
        args = json!({"cursor":p["next_cursor"],"filter_hash":p["filter_hash"]});
    }
    assert_eq!(count, 2100);
}

#[test]
fn read_access_does_not_create_a_project_and_cannot_write() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("absent");
    assert!(Graph::open(&root, &d.path().join("host"), Access::read("alice")).is_err());
    assert!(!root.exists());
    let (d, g) = project();
    let read = Graph::open(
        &d.path().join("project"),
        &d.path().join("host"),
        Access::read("alice"),
    )
    .unwrap();
    assert_eq!(
        read.apply(&create(&g, "bad")).unwrap_err().code,
        "UNAUTHORIZED"
    );
    assert_eq!(read.page(&json!({})).unwrap()["selection_complete"], true);
}

#[test]
fn path_traversal_and_forged_checker_nodes_are_rejected() {
    let (_d, g) = project();
    assert_eq!(g.blob("../../outside").unwrap_err().code, "SCHEMA_INVALID");
    let mut p = create(&g, "fake");
    p["operations"][0]["kind"] = "AnalysisFinding".into();
    assert_eq!(g.apply(&p).unwrap_err().code, "PROTECTED_ENTITY");
}

#[test]
fn host_journal_recovers_commit_before_ack_and_fences_old_attempt() {
    use nirdosha_graph::journal::{Journal,Frames};
    let (d,g)=project();let path=d.path().join("journal.db");
    let j=Journal::open(&path,"conversation-1",&g).unwrap();
    let prepared=j.prepare(1,"create-entity",create(&g,"one")).unwrap();
    let receipt=g.stream_append(&j.stream_id().unwrap(),1,&prepared).unwrap();
    drop(j); // host died after service commit, before saving tool result
    let j=Journal::open(&path,"conversation-1",&g).unwrap();
    assert!(j.next_attempt().is_err());
    assert_eq!(j.recover(&g).unwrap(),vec![receipt]);
    assert_eq!(g.version().unwrap().revision,1);
    assert_eq!(j.next_attempt().unwrap(),2);
    assert_eq!(j.prepare(1,"late",create(&g,"late")).unwrap_err().code,"REVISION_CONFLICT");
    assert_eq!(j.prepare(2,"create-entity",create(&g,"one")).unwrap(),prepared);
    assert_eq!(j.prepare(2,"create-entity",create(&g,"different")).unwrap_err().code,"IDEMPOTENCY_CONFLICT");
    let mut frames=Frames::default();
    assert!(frames.push(b"{\"operations\":").unwrap().is_empty());
    frames.interrupted();
    assert_eq!(frames.push(b"{\"fresh\":true}\n").unwrap(),vec![json!({"fresh":true})]);
}

#[test]
fn continuation_binds_query_even_when_client_changes_supplied_checksum() {
    let (_d,g)=project();g.apply(&create(&g,"one")).unwrap();g.apply(&create(&g,"two")).unwrap();
    let first=g.page(&json!({"limit":1})).unwrap();
    let changed=hash::structured("query",&json!({"entity_type":"node","kind":null,"text":null,"include_deleted":false})).unwrap();
    assert_eq!(g.page(&json!({"cursor":first["next_cursor"],"filter_hash":changed,"entity_type":"node"})).unwrap_err().code,"SCHEMA_INVALID");
}

#[test]
fn workflow_ownership_terminal_and_literal_quorum_are_validated_atomically() {
    let (_d,g)=project();
    let mut p=patch(&g,"workflow",json!([
        {"op":"node.create","local_id":"@w","kind":"Workflow","title":"Review","spec":{"state_ids":["@a","@b"],"transition_ids":["@t"],"initial_state_id":"@a"}},
        {"op":"node.create","local_id":"@a","kind":"WorkflowState","title":"Draft","spec":{"workflow_id":"@w","terminal":"none"}},
        {"op":"node.create","local_id":"@b","kind":"WorkflowState","title":"Done","spec":{"workflow_id":"@w","terminal":"success"}},
        {"op":"node.create","local_id":"@t","kind":"WorkflowTransition","title":"Submit","spec":{"workflow_id":"@w","from_state_id":"@a","to_state_id":"@b","event":"Submit"}}
    ]),json!([]));
    let mut bad=p.clone();bad["operations"][3]["spec"]["from_state_id"]="@b".into();
    assert_eq!(g.apply(&bad).unwrap_err().code,"SCHEMA_INVALID");
    p["mutation_id"]="valid-workflow".into();let r=g.apply(&p).unwrap();
    let w=r["id_map"]["@w"].as_str().unwrap();
    assert_eq!(node(&g,w)["kind"],"Workflow");
    let bad=patch(&g,"bad-policy",json!([{"op":"node.create","local_id":"@p","kind":"ApprovalPolicy","title":"Impossible","spec":{"stages":[{"id":"review","quorum":3,"slots":{"a":{},"b":{}},"mode":"parallel"}]}}]),json!([]));
    assert_eq!(g.apply(&bad).unwrap_err().code,"SCHEMA_INVALID");
}

#[test]
fn deterministic_accepted_v2_emission_compiles_and_detects_stale_body_context() {
    let (d,g)=project();
    let p=patch(&g,"function",json!([{"op":"node.create","local_id":"@f","kind":"Function","title":"answer","symbol":{"package_id":"app","module_path":[],"namespace":"value","name":"answer"},"spec":{"parameters":{},"parameter_order":[],"return_type":{"tag":"primitive","name":"i64"},"visibility":"pub","body":{"tag":"missing"}}}]),json!([]));
    let receipt=g.apply(&p).unwrap();let id=receipt["id_map"]["@f"].as_str().unwrap();
    let context=g.body_context(id,None).unwrap();let blob=g.put_blob(b"42").unwrap();
    g.apply(&patch(&g,"body",json!([{"op":"body.attach","node_id":id,"body":{"tag":"source","blob_hash":blob,"dialect":"nirdosha-v2","signature_hash":context["signature_hash"],"dependency_hash":context["dependency_hash"]}}]),json!([pin(&g,id)]))).unwrap();
    let accepted=g.accept(&json!({"mutation_id":"accept","base_acceptance_id":null,"selections":[pin(&g,id)],"dependency_pins":[]})).unwrap();
    let one=g.emit(&json!({})).unwrap();let two=g.emit(&json!({})).unwrap();assert_eq!(one,two);
    let path=d.path().join("output.nir");std::fs::write(&path,one["source"].as_str().unwrap()).unwrap();
    let output=std::process::Command::new("rustc").args(["--edition=2024","--crate-type=lib","--crate-name=graph_fixture"]).arg(&path).arg("-o").arg(d.path().join("fixture.rlib")).output().unwrap();
    assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));
    g.apply(&patch(&g,"rename",json!([{"op":"node.rename","node_id":id,"title":"new_answer","symbol":{"package_id":"app","module_path":[],"namespace":"value","name":"new_answer"}}]),json!([pin(&g,id)]))).unwrap();
    assert_eq!(g.emit(&json!({})).unwrap()["source"],one["source"]); // proposals do not leak
    g.accept(&json!({"mutation_id":"accept-again","base_acceptance_id":accepted["acceptance_id"],"selections":[pin(&g,id)],"dependency_pins":[]})).unwrap();
    assert_eq!(g.emit(&json!({})).unwrap_err().code,"BODY_CONTEXT_STALE");
}

#[test]
fn asynchronous_findings_are_protected_and_become_stale() {
    let (_d,g)=project();
    let p=patch(&g,"requirement",json!([{"op":"node.create","local_id":"@r","kind":"Requirement","title":"Cover this","spec":{"text":"Must be implemented"}}]),json!([]));
    let r=g.apply(&p).unwrap();let id=r["id_map"]["@r"].as_str().unwrap();
    let job=g.analyze(&json!({"rules":["requirements.coverage"]})).unwrap();let job=job["job_id"].as_str().unwrap();
    let deadline=std::time::Instant::now()+std::time::Duration::from_secs(5);
    let result=loop{let v=g.analysis_status(job,false).unwrap();if v["state"]!="running"{break v}assert!(std::time::Instant::now()<deadline);std::thread::sleep(std::time::Duration::from_millis(10));};
    assert_eq!(result["state"],"completed","{result}");assert_eq!(result["freshness"],"current");
    let finding=result["result"]["finding_ids"][0].as_str().unwrap();
    assert_eq!(node(&g,finding)["spec"]["classification"],"potential_issue");
    assert_eq!(g.apply(&patch(&g,"forge",json!([{"op":"spec.set","node_id":finding,"path":"/classification","value":"proven_satisfied"}]),json!([pin(&g,finding)]))).unwrap_err().code,"PROTECTED_ENTITY");
    g.apply(&patch(&g,"update",json!([{"op":"spec.set","node_id":id,"path":"/text","value":"Changed obligation"}]),json!([pin(&g,id)]))).unwrap();
    assert_eq!(g.analysis_status(job,false).unwrap()["freshness"],"stale");
    assert_eq!(g.analyze(&json!({"rules":["run:arbitrary-command"]})).unwrap_err().code,"UNSUPPORTED_CHECKER");
}

/// RFC 0021 §5.3 "Observed source": `source.observe` records what a
/// source scan actually found on a node, independent of and possibly
/// disagreeing with the node's own authored `spec` -- the service
/// never reconciles the two automatically.
#[test]
fn source_observe_records_a_disagreeing_observation_without_touching_spec() {
    let (_d, g) = project();
    let p = create(&g, "Account");
    let id = g.apply(&p).unwrap()["id_map"]["@n"].as_str().unwrap().to_string();
    assert_eq!(node(&g, &id)["observation"], Value::Null, "no observation before source.observe");

    let observe = patch(
        &g,
        "observe-1",
        json!([{"op":"source.observe","node_id":id,"source_ref":{"path":"src/account.rs","hash":"deadbeef"},"declaration":{"fields":{"balance_cents":"i64"}}}]),
        json!([pin(&g, &id)]),
    );
    g.apply(&observe).unwrap();
    let observed = node(&g, &id);
    assert_eq!(observed["observation"]["source_ref"]["path"], "src/account.rs");
    assert_eq!(observed["observation"]["declaration"]["fields"]["balance_cents"], "i64");
    // The spec (empty fields, per `create`) is untouched -- an
    // observation can disagree with it, never silently overwrite it.
    assert_eq!(observed["spec"]["fields"], json!({}));

    // A later observation overwrites the prior one (the field is the
    // *current* observed view, not a log -- provenance/history already
    // covers the append-only record via `graph_provenance`).
    let observe_again = patch(
        &g,
        "observe-2",
        json!([{"op":"source.observe","node_id":id,"source_ref":{"path":"src/account.rs","hash":"cafef00d"},"declaration":{"fields":{"balance_cents":"i64","owner":"String"}}}]),
        json!([pin(&g, &id)]),
    );
    g.apply(&observe_again).unwrap();
    assert_eq!(node(&g, &id)["observation"]["source_ref"]["hash"], "cafef00d");
    assert_eq!(node(&g, &id)["observation"]["declaration"]["fields"]["owner"], "String");
}

#[test]
fn source_observe_on_an_unknown_node_is_not_found() {
    let (_d, g) = project();
    let observe = patch(
        &g,
        "observe",
        json!([{"op":"source.observe","node_id":"n_nonexistent","source_ref":{"path":"x.rs"},"declaration":{}}]),
        json!([]),
    );
    assert_eq!(g.apply(&observe).unwrap_err().code, "NOT_FOUND");
}
