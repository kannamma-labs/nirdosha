use nirdosha_workflow::*;
use nirdosha_graph::{schema::{Node,State,spec_version},store::now,Result};
use serde_json::{Value,json};
use std::collections::BTreeSet;
fn node(id:&str,kind:&str,spec:Value)->Node{Node{id:id.into(),kind:kind.into(),title:id.into(),symbol:None,entity_revision:1,deleted:false,origin:"test".into(),spec_schema_version:spec_version(kind),spec,source_refs:vec![],provenance_ids:vec![],protected:false,observation:None}}
fn definition()->State{
    let mut state=State::default();for n in [
        node("w","Workflow",json!({"state_ids":["review","approved"],"transition_ids":["finalize"],"initial_state_id":"review"})),
        node("review","WorkflowState",json!({"workflow_id":"w","terminal":"none","approval_policy_id":"approval"})),
        node("approved","WorkflowState",json!({"workflow_id":"w","terminal":"success","on_entry":[{"id":"notify","function_id":"notify","arguments":{}}]})),
        node("finalize","WorkflowTransition",json!({"workflow_id":"w","from_state_id":"review","to_state_id":"approved","event":"finalize","approval_policy_id":"approval"})),
        node("notify","Function",json!({"parameters":{},"parameter_order":[],"return_type":{"tag":"primitive","name":"unit"},"effects":["outbox"]})),
        node("ops","Role",json!({"name":"operations"})),node("compliance","Role",json!({"name":"compliance"})),
        node("approval","ApprovalPolicy",json!({"identity_basis":"distinct_person","maker_counts":true,"exclude_maker":true,"cross_stage_distinct":true,"decision_lifetime_ms":60000,"rejection":"reject","invalidate_on_change":true,"stages":[{"id":"reviewers","mode":"parallel","quorum":2,"slots":{"operations":{"role_ids":["ops"]},"compliance":{"role_ids":["compliance"]}}}]})),
    ]{state.nodes.insert(n.id.clone(),n);}state
}
fn authority()->Authority{Authority{authority_id:"company-directory".into(),key_id:"company-directory-key-1".into(),audience:"payments".into(),tenant:"company".into(),issuers:BTreeSet::from(["company-login".into()]),max_freshness_seconds:120}}
fn mapping(subject:&str,person:&str)->Mapping{Mapping{mapping_schema_version:nirdosha_workflow::identity::MAPPING_SCHEMA_VERSION.into(),authority_key_id:"company-directory-key-1".into(),audience:"payments".into(),issuer:"company-login".into(),tenant:"company".into(),subject:subject.into(),person:person.into(),roles:BTreeSet::from(["ops".into(),"compliance".into()]),claims:BTreeSet::new(),revision:1,revocation_revision:0,issued_at:now(),expires_at:now()+110,revoked:false,integrity_proof:None}}
fn account(rt:&Runtime,subject:&str,person:&str)->String{let m=mapping(subject,person);rt.install_mapping(&m).unwrap();rt.issue_credential(&m.issuer,&m.tenant,&m.subject,now()+100).unwrap()}
struct Notify{provider_idempotent:bool}
impl Hooks for Notify{fn call(&self,_:&str,_:&Value,_:&mut Value,intents:&mut Vec<Intent>)->Result<()>{intents.push(Intent{action_id:"payment".into(),payload:json!({"notify":true}),provider_idempotent:self.provider_idempotent,depends_on:vec![]});Ok(())}}
fn decision(rt:&Runtime,_token:&str,id:&str,slot:&str,key:&str)->Event{let i=rt.instance(id).unwrap();Event{key:key.into(),instance_id:id.into(),expected_revision:i.revision,kind:"decision".into(),payload:json!({"policy_id":"approval","stage":"reviewers","slot":slot,"decision":"approve","payload_hash":i.payload_hash,"round":i.round})}}
fn finalize(rt:&Runtime,id:&str,key:&str)->Event{Event{key:key.into(),instance_id:id.into(),expected_revision:rt.instance(id).unwrap().revision,kind:"transition".into(),payload:json!({"transition_id":"finalize"})}}
#[test]
fn same_person_accounts_cannot_complete_six_eyes_and_replay_does_not_advance_twice(){
    let d=tempfile::tempdir().unwrap();let path=d.path().join("app.db");let rt=Runtime::open(&path,authority()).unwrap();let def=rt.deploy(&definition()).unwrap();
    let maker=account(&rt,"maker","person-m");let alice=account(&rt,"alice","person-a");let alias=account(&rt,"alice-other","person-a");let bob=account(&rt,"bob","person-b");
    let start=rt.start(&def,"w",&maker,json!({"amount":100}),"start",&NoHooks).unwrap();let id=start["instance"]["id"].as_str().unwrap();
    assert_eq!(rt.event(&maker,&decision(&rt,&maker,id,"operations","maker-decision"),&NoHooks).unwrap_err().code,"MAKER_EXCLUDED");
    rt.event(&alice,&decision(&rt,&alice,id,"operations","a"),&NoHooks).unwrap();rt.event(&alias,&decision(&rt,&alias,id,"compliance","alias"),&NoHooks).unwrap();
    assert_eq!(rt.event(&maker,&finalize(&rt,id,"too-early"),&NoHooks).unwrap_err().code,"QUORUM_NOT_MET");
    rt.event(&bob,&decision(&rt,&bob,id,"compliance","b"),&NoHooks).unwrap();let event=finalize(&rt,id,"final");let result=rt.event(&maker,&event,&Notify{provider_idempotent:true}).unwrap();
    assert_eq!(result["instance"]["state_id"],"approved");drop(rt);
    let rt=Runtime::open(&path,authority()).unwrap();assert_eq!(rt.event(&maker,&event,&NoHooks).unwrap(),result);assert_eq!(rt.outbox(id).unwrap().len(),1);
    let mut different=event.clone();different.payload["other"]=true.into();assert_eq!(rt.event(&maker,&different,&NoHooks).unwrap_err().code,"IDEMPOTENCY_CONFLICT");
    let claimed=rt.claim_outbox(1).unwrap().unwrap();let outbox=claimed["outbox_id"].as_str().unwrap();
    // Provider succeeded, then worker died before persisting its acknowledgment.
    rusqlite::Connection::open(&path).unwrap().execute("UPDATE wf_outbox SET lease_expires=0",[]).unwrap();
    let retry=rt.claim_outbox(1).unwrap().unwrap();assert_eq!(retry["idempotency_key"],claimed["idempotency_key"]);assert_ne!(retry["lease_token"],claimed["lease_token"]);
    assert_eq!(rt.complete_outbox(outbox,claimed["lease_token"].as_str().unwrap(),"succeeded",&json!({"receipt":"provider"})).unwrap_err().code,"REVISION_CONFLICT");
    rt.complete_outbox(outbox,retry["lease_token"].as_str().unwrap(),"succeeded",&json!({"receipt":"provider"})).unwrap();
}
#[test]
fn identity_merge_expiry_and_payload_changes_invalidate_approvals(){
    let d=tempfile::tempdir().unwrap();let rt=Runtime::open(&d.path().join("app.db"),authority()).unwrap();let def=rt.deploy(&definition()).unwrap();let maker=account(&rt,"maker","m");let a=account(&rt,"a","a");let b=account(&rt,"b","b");
    let start=rt.start(&def,"w",&maker,json!({"amount":100}),"start",&NoHooks).unwrap();let id=start["instance"]["id"].as_str().unwrap();
    rt.event(&a,&decision(&rt,&a,id,"operations","a"),&NoHooks).unwrap();rt.event(&b,&decision(&rt,&b,id,"compliance","b"),&NoHooks).unwrap();
    let mut merged=mapping("b","a");merged.revision=2;rt.install_mapping(&merged).unwrap();assert_eq!(rt.event(&maker,&finalize(&rt,id,"merged"),&NoHooks).unwrap_err().code,"QUORUM_NOT_MET");
    merged.person="b".into();merged.revision=3;rt.install_mapping(&merged).unwrap();
    let update=Event{key:"update".into(),instance_id:id.into(),expected_revision:rt.instance(id).unwrap().revision,kind:"update".into(),payload:json!({"amount":200})};rt.event(&maker,&update,&NoHooks).unwrap();
    assert_eq!(rt.event(&maker,&finalize(&rt,id,"old-round"),&NoHooks).unwrap_err().code,"QUORUM_NOT_MET");
    merged.revoked=true;merged.revision=4;merged.revocation_revision=1;rt.install_mapping(&merged).unwrap();assert_eq!(rt.event(&b,&decision(&rt,&b,id,"compliance","revoked"),&NoHooks).unwrap_err().code,"IDENTITY_STALE");
    let mut untrusted=mapping("evil","evil");untrusted.issuer="untrusted".into();assert_eq!(rt.install_mapping(&untrusted).unwrap_err().code,"UNTRUSTED_IDENTITY");
}
/// RFC 0021.b's own words: "A split does not clone a historical
/// decision into multiple approvals." Subject "shared" decides while
/// still mapped to its pre-split person; the authority then splits that
/// person, reassigning "shared" to a brand-new, distinct one. Quorum
/// re-resolves the decision under whichever person "shared" *currently*
/// maps to (never a cached historical value), so the original decision
/// counts as exactly one person either way -- it is never duplicated
/// into a second approval, and there is still exactly one decision row
/// for that subject afterward.
#[test]
fn split_does_not_clone_a_decision_into_multiple_approvals(){
    let d=tempfile::tempdir().unwrap();let path=d.path().join("app.db");let rt=Runtime::open(&path,authority()).unwrap();let def=rt.deploy(&definition()).unwrap();
    let maker=account(&rt,"maker","m");
    let shared=account(&rt,"shared","person-shared-before-split");
    let start=rt.start(&def,"w",&maker,json!({"amount":100}),"start",&NoHooks).unwrap();let id=start["instance"]["id"].as_str().unwrap();
    rt.event(&shared,&decision(&rt,&shared,id,"operations","shared-decides"),&NoHooks).unwrap();
    // Split: "shared" is reassigned to a brand-new person, distinct
    // from both its own pre-split identity and from "other" below.
    let mut split=mapping("shared","person-shared-after-split");split.revision=2;rt.install_mapping(&split).unwrap();
    let other=account(&rt,"other","person-other");
    rt.event(&other,&decision(&rt,&other,id,"compliance","other-decides"),&NoHooks).unwrap();
    let result=rt.event(&maker,&finalize(&rt,id,"final"),&Notify{provider_idempotent:true}).unwrap();
    assert_eq!(result["instance"]["state_id"],"approved");
    // Exactly two decision rows exist for this instance -- one per real
    // `event()` call above. A clone would show up as a third row here.
    let count:i64=rusqlite::Connection::open(&path).unwrap().query_row("SELECT COUNT(*) FROM wf_decisions WHERE instance=?1",[id],|r|r.get(0)).unwrap();
    assert_eq!(count,2,"a split must never duplicate a decision row");
}
#[test]
fn guard_failure_rolls_back_hooks_and_unknown_effect_requires_reconciliation(){
    let d=tempfile::tempdir().unwrap();let path=d.path().join("app.db");let rt=Runtime::open(&path,authority()).unwrap();let mut state=definition();
    // Use an ungated transition to exercise hook transaction and outbox independently.
    state.nodes.get_mut("finalize").unwrap().spec.as_object_mut().unwrap().remove("approval_policy_id");
    state.nodes.get_mut("approved").unwrap().spec["invariants"]=json!([{"tag":"literal","value":false}]);
    let def=rt.deploy(&state).unwrap();let maker=account(&rt,"maker","m");let start=rt.start(&def,"w",&maker,json!({}),"start",&NoHooks).unwrap();let id=start["instance"]["id"].as_str().unwrap();
    assert_eq!(rt.event(&maker,&finalize(&rt,id,"fails"),&Notify{provider_idempotent:false}).unwrap_err().code,"CONDITION_FAILED");assert_eq!(rt.instance(id).unwrap().revision,1);assert!(rt.outbox(id).unwrap().is_empty());
    state.nodes.get_mut("approved").unwrap().spec["invariants"]=json!([]);let changed=rt.deploy(&state).unwrap();let mut admin=mapping("admin","admin");admin.roles.insert("nirdosha.runtime.operator".into());rt.install_mapping(&admin).unwrap();let token=rt.issue_credential(&admin.issuer,&admin.tenant,&admin.subject,now()+90).unwrap();
    rt.migrate_instance("migrate",id,1,&changed,"w","review",json!({}),&token,&NoHooks).unwrap();
    rt.event(&maker,&finalize(&rt,id,"works"),&Notify{provider_idempotent:false}).unwrap();rt.claim_outbox(1).unwrap().unwrap();rusqlite::Connection::open(&path).unwrap().execute("UPDATE wf_outbox SET lease_expires=0",[]).unwrap();assert!(rt.claim_outbox(1).unwrap().is_none());
    let actions=rt.outbox(id).unwrap();assert_eq!(actions[0]["state"],"outcome_unknown");assert_eq!(rt.migrate_instance("blocked",id,rt.instance(id).unwrap().revision,&changed,"w","review",json!({}),&token,&NoHooks).unwrap_err().code,"MIGRATION_BLOCKED");
    rt.reconcile_outbox(actions[0]["id"].as_str().unwrap(),"succeeded",&json!({"operator_verified_receipt":"ok"})).unwrap();
}
