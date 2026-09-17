//! Non-executable workflow authoring validation (RFC 0021.a).
use crate::{Error, Result, schema::{State,Node}};
use serde_json::Value;
use std::collections::BTreeSet;

fn ids(v:&Value)->Vec<&str>{v.as_array().into_iter().flatten().filter_map(Value::as_str).collect()}
fn check_kind(state:&State,id:&str,kinds:&[&str])->Result<()> {
    if !state.nodes.get(id).is_some_and(|n|!n.deleted && kinds.contains(&n.kind.as_str())) {
        return Err(Error::new("UNRESOLVED_REFERENCE",format!("{id} must reference {}",kinds.join(" or "))));
    }
    Ok(())
}
fn references(state:&State,v:&Value)->Result<()> {
    match v {
        Value::Object(o)=>for(k,v)in o {
            let kinds:&[&str]=match k.as_str(){
                "workflow_id"=>&["Workflow"],"initial_state_id"|"from_state_id"|"to_state_id"|"state_ids"=>&["WorkflowState"],
                "transition_ids"=>&["WorkflowTransition"],"function_id"|"entry_function_id"|"handler_ids"=>&["Function","ExternalSymbol"],
                "approval_policy_id"=>&["ApprovalPolicy"],"policy_ids"|"owner_policy_id"|"actor_policy_id"=>&["Policy"],
                "role_ids"=>&["Role"],"claim_ids"=>&["Claim"],"contract_ids"=>&["Contract"],"plan_id"|"update_plan_id"|"startup_plan_id"=>&["Plan"],
                "module_ids"=>&["Module"],"application_id"=>&["Application"],"workflow_ids"=>&["Workflow"],_=>&[]};
            if !kinds.is_empty(){if let Some(id)=v.as_str(){check_kind(state,id,kinds)?}else{for id in ids(v){check_kind(state,id,kinds)?}}}
            references(state,v)?;
        },
        Value::Array(a)=>for v in a{references(state,v)?},_=>()
    }
    Ok(())
}
pub fn validate(state:&State)->Result<()> {
    for n in state.nodes.values().filter(|n|!n.deleted){
        references(state,&n.spec)?;
        let s=&n.spec;
        match n.kind.as_str(){
            "Workflow"=>{
                let members=ids(&s["state_ids"]);let transitions=ids(&s["transition_ids"]);
                if members.len()!=members.iter().collect::<BTreeSet<_>>().len() || transitions.len()!=transitions.iter().collect::<BTreeSet<_>>().len(){return Err(Error::new("SCHEMA_INVALID","Duplicate workflow member"))}
                if let Some(initial)=s["initial_state_id"].as_str(){if !members.contains(&initial){return Err(Error::new("SCHEMA_INVALID","Initial state is not owned by workflow"))}}
                let mut names=BTreeSet::new();
                for id in members.iter().chain(transitions.iter()) {
                    let child=&state.nodes[*id];
                    if child.spec["workflow_id"]!=n.id{return Err(Error::new("SCHEMA_INVALID","Workflow membership and child ownership differ"))}
                    if child.kind=="WorkflowState" && !names.insert(child.title.clone()){return Err(Error::new("SCHEMA_INVALID","Duplicate workflow state name"))}
                }
            },
            "WorkflowState"|"WorkflowTransition"=>{
                if let Some(parent)=s["workflow_id"].as_str(){
                    let p=&state.nodes[parent];let key=if n.kind=="WorkflowState"{"state_ids"}else{"transition_ids"};
                    if !ids(&p.spec[key]).contains(&n.id.as_str()){return Err(Error::new("SCHEMA_INVALID","Child missing from authoritative workflow membership"))}
                }
                if n.kind=="WorkflowTransition" {
                    for key in ["from_state_id","to_state_id"] {if let Some(id)=s[key].as_str(){if state.nodes[id].spec["workflow_id"]!=s["workflow_id"]{return Err(Error::new("SCHEMA_INVALID","Transition crosses workflow boundary"))}}}
                    if let Some(from)=s["from_state_id"].as_str(){let terminal=&state.nodes[from].spec["terminal"];if !terminal.is_null() && terminal!="none" && s["reopen"]!=true{return Err(Error::new("SCHEMA_INVALID","Terminal state requires explicit reopen transition"))}}
                    if s["from_state_id"].is_string() && s["from_state_id"]==s["to_state_id"] && !s["reenter"].is_boolean(){return Err(Error::new("SCHEMA_INVALID","Self-transition must declare reenter"))}
                }
                for key in ["on_entry","on_exit"]{let mut seen=BTreeSet::new();for hook in s[key].as_array().into_iter().flatten(){
                    if !seen.insert(hook["id"].as_str()){return Err(Error::new("SCHEMA_INVALID","Duplicate hook binding ID"))}
                    let f=&state.nodes[hook["function_id"].as_str().unwrap()];validate_hook(f,hook)?;
                }}
            },_=>()
        }
    }
    // Module membership is acyclic. Workflow containment is checked above.
    for n in state.nodes.values().filter(|n|!n.deleted && n.kind=="Module"){
        let mut seen=BTreeSet::new();let mut current=n;
        while let Some(parent)=current.spec["parent_id"].as_str(){
            if !seen.insert(current.id.clone()){return Err(Error::new("SCHEMA_INVALID","Containment cycle"))}
            current=state.nodes.get(parent).ok_or_else(||Error::new("UNRESOLVED_REFERENCE",parent))?;
            if !["Project","Module"].contains(&current.kind.as_str()){return Err(Error::new("SCHEMA_INVALID","Module parent must be Project or Module"))}
        }
    }
    Ok(())
}
fn validate_hook(function:&Node,hook:&Value)->Result<()> {
    if let Some(parameters)=function.spec["parameters"].as_object(){
        let arguments=hook["arguments"].as_object().unwrap();
        if parameters.keys().collect::<BTreeSet<_>>()!=arguments.keys().collect::<BTreeSet<_>>(){return Err(Error::new("SCHEMA_INVALID","Hook argument IDs differ from function parameter IDs"))}
    }
    // Bindings remain authoring data; a compiler adapter must typecheck context values.
    Ok(())
}
