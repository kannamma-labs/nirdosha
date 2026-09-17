//! RFC 0021.b reference SQLite runtime, separate from the authoring graph.
//! All effects are staged as outbox intents. Host hook adapters are trusted
//! local code and must not perform network/file effects during a transaction.
use nirdosha_graph::{Error,Result,hash,schema::{State,Node},analysis::eval,store::{id,now}};
use rusqlite::{Connection,OptionalExtension,params};
use serde::{Serialize,Deserialize};
use serde_json::{Value,json};
use std::{path::Path,collections::{BTreeMap,BTreeSet}};
pub mod identity;
pub use identity::{Authority,Mapping};
mod outbox;

#[derive(Clone,Debug,Serialize,Deserialize)]
pub struct Instance {pub id:String,pub definition_id:String,pub workflow_id:String,pub state_id:String,pub revision:u64,pub round:u64,pub maker_subject:String,pub data:Value,pub payload_hash:String}
#[derive(Clone,Debug,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {pub key:String,pub instance_id:String,pub expected_revision:u64,pub kind:String,pub payload:Value}
#[derive(Clone,Debug,Serialize,Deserialize)]
pub struct Intent {pub action_id:String,pub payload:Value,pub provider_idempotent:bool,pub depends_on:Vec<String>}
/// Pure/local hook implementations can edit staged JSON and enqueue intents.
/// The runtime publishes neither until all conditions and invariants pass.
pub trait Hooks {fn call(&self,function_id:&str,arguments:&Value,data:&mut Value,intents:&mut Vec<Intent>)->Result<()>;}
pub struct NoHooks;
impl Hooks for NoHooks {fn call(&self,_:&str,_:&Value,_:&mut Value,_:&mut Vec<Intent>)->Result<()>{Err(Error::new("UNSUPPORTED_TARGET","No hook adapter registered"))}}

pub struct Runtime {db:Connection,authority:Authority}
impl Runtime {
    pub fn open(path:&Path,authority:Authority)->Result<Self>{
        if authority.issuers.is_empty()||authority.key_id.is_empty()||authority.audience.is_empty()||authority.max_freshness_seconds==0||authority.max_freshness_seconds>3600{return Err(Error::new("SCHEMA_INVALID","Authority requires issuers, a key_id, an audience and a freshness bound of 1–3600 seconds"))}
        let db=Connection::open(path)?;db.busy_timeout(std::time::Duration::from_secs(2))?;db.execute_batch(SCHEMA)?;
        let config=hash::structured("runtime-authority",&serde_json::to_value(&authority)?)?;
        db.execute("INSERT OR IGNORE INTO wf_config VALUES(1,?1)",[&config])?;
        let existing:String=db.query_row("SELECT hash FROM wf_config WHERE id=1",[],|r|r.get(0))?;if existing!=config{return Err(Error::new("UNTRUSTED_IDENTITY","Authority configuration differs; explicit operator migration required"))}
        Ok(Self{db,authority})
    }
    fn tx<T>(&self,f:impl FnOnce()->Result<T>)->Result<T>{self.db.execute_batch("BEGIN IMMEDIATE")?;match f(){Ok(v)=>{self.db.execute_batch("COMMIT")?;Ok(v)},Err(e)=>{let _=self.db.execute_batch("ROLLBACK");Err(e)}}}
    /// RFC 0021.b's own words: "the runtime capability manifest
    /// therefore distinguishes `distinct_subject` from
    /// `distinct_person(authority_id, contract_version,
    /// freshness_policy)`." Reported with this deployment's *real*
    /// values, not a bare literal -- an emitter/reader checking this
    /// manifest can see exactly which authority and freshness bound
    /// backs the claim, not just that some unspecified one exists.
    pub fn capabilities(&self)->Value {
        let distinct_person=format!("distinct_person(authority_id={}, contract_version={}, freshness_policy={}s)",self.authority.authority_id,identity::MAPPING_SCHEMA_VERSION,self.authority.max_freshness_seconds);
        json!({"adapter":"nirdosha-workflow/sqlite-v1","source_dialect":"nirdosha-v2","capabilities":["guarded_transitions","durable_approval_ledger","distinct_subject",distinct_person,"transactional_hooks","outbox"],"identity_authority":self.authority,"predicate_fragment":["boolean literals","integer comparisons","boolean composition","context field bindings"],"unsupported":["arbitrary predicate calls","automatic finalization","delegation","role vetoes","mandatory slots beyond quorum"]})
    }
    /// Operator deployment API. Definitions are immutable content-addressed data.
    pub fn deploy(&self,state:&State)->Result<String>{nirdosha_graph::schema::validate_state(state)?;
        for n in state.nodes.values().filter(|n|!n.deleted){if n.kind=="ApprovalPolicy"{
            for key in ["identity_basis","stages","exclude_maker","cross_stage_distinct","decision_lifetime_ms","invalidate_on_change","rejection"]{if n.spec.get(key).is_none(){return Err(Error::new("SCHEMA_INVALID",format!("Approval policy requires {key}")))}}
            if n.spec["exclude_maker"]!=true||n.spec["invalidate_on_change"]!=true{return Err(Error::new("UNSUPPORTED_TARGET","Maker exclusion and payload invalidation are mandatory"))}
            if n.spec["decision_lifetime_ms"].as_u64().unwrap_or(0)==0{return Err(Error::new("SCHEMA_INVALID","Decision lifetime must be positive"))}
        }}
        let value=serde_json::to_value(state)?;let key=hash::structured("runtime-definition",&value)?;
        self.db.execute("INSERT OR IGNORE INTO wf_definitions VALUES(?1,?2)",params![key,value.to_string()])?;Ok(key)
    }
    fn definition(&self,key:&str)->Result<State>{let value:String=self.db.query_row("SELECT payload FROM wf_definitions WHERE id=?1",[key],|r|r.get(0))?;Ok(serde_json::from_str(&value)?)}
    pub fn instance(&self,key:&str)->Result<Instance>{let value:Option<String>=self.db.query_row("SELECT payload FROM wf_instances WHERE id=?1",[key],|r|r.get(0)).optional()?;Ok(serde_json::from_str(&value.ok_or_else(||Error::new("NOT_FOUND","Instance"))?)?)}
    fn save(&self,i:&Instance)->Result<()>{self.db.execute("INSERT INTO wf_instances VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload",params![i.id,serde_json::to_string(i)?])?;Ok(())}
    fn receipt(&self,key:&str,digest:&str)->Result<Option<Value>>{let old:Option<(String,String)>=self.db.query_row("SELECT hash,result FROM wf_receipts WHERE key=?1",[key],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;if let Some((h,r))=old{if h!=digest{return Err(Error::new("IDEMPOTENCY_CONFLICT","Event key reused with different input"))}return Ok(Some(serde_json::from_str(&r)?))}Ok(None)}
    fn commit_receipt(&self,key:&str,digest:&str,result:&Value,actor:&str)->Result<()>{self.db.execute("INSERT INTO wf_receipts VALUES(?1,?2,?3)",params![key,digest,result.to_string()])?;self.db.execute("INSERT INTO wf_audit(event_key,actor,result,recorded_at) VALUES(?1,?2,?3,?4)",params![key,actor,result.to_string(),now()])?;Ok(())}
    pub fn start(&self,definition_id:&str,workflow_id:&str,credential:&str,data:Value,key:&str,hooks:&dyn Hooks)->Result<Value>{self.tx(||{
        let(subject,actor)=self.authenticate(credential)?;let digest=hash::structured("runtime-start",&json!([definition_id,workflow_id,subject,data]))?;if let Some(r)=self.receipt(key,&digest)?{return Ok(r)}
        let state=self.definition(definition_id)?;let w=state.nodes.get(workflow_id).filter(|n|n.kind=="Workflow").ok_or_else(||Error::new("NOT_FOUND","Workflow definition"))?;
        let initial=w.spec["initial_state_id"].as_str().ok_or_else(||Error::new("SCHEMA_INVALID","Initial state missing"))?;let n=&state.nodes[initial];
        let mut i=Instance{id:id("instance")?,definition_id:definition_id.into(),workflow_id:workflow_id.into(),state_id:initial.into(),revision:1,round:1,maker_subject:subject.clone(),payload_hash:hash::structured("runtime-review-payload",&data)?,data};
        let mut intents=vec![];let event=json!({});check(&n.spec["entry_conditions"],&context(&i,&actor,&event))?;run_hooks(n,"on_entry",&mut i,&actor,&event,hooks,&mut intents)?;invariants(&state,&i,&actor,&event)?;i.payload_hash=hash::structured("runtime-review-payload",&i.data)?;
        self.save(&i)?;self.save_intents(&i,key,&intents)?;let result=json!({"instance":i});self.commit_receipt(key,&digest,&result,&subject)?;Ok(result)
    })}
    pub fn event(&self,credential:&str,event:&Event,hooks:&dyn Hooks)->Result<Value>{self.tx(||{
        let(subject,actor)=self.authenticate(credential)?;let digest=hash::structured("runtime-event",&json!({"actor":subject,"event":event}))?;if let Some(r)=self.receipt(&event.key,&digest)?{return Ok(r)}
        let mut i=self.instance(&event.instance_id)?;if i.revision!=event.expected_revision{return Err(Error::new("REVISION_CONFLICT","Instance changed").details(json!({"current_revision":i.revision})))}
        let state=self.definition(&i.definition_id)?;let mut intents=vec![];
        match event.kind.as_str(){
            "decision"=>self.record_decision(&state,&i,&subject,&actor,event)?,
            "withdraw"=>{self.db.execute("INSERT INTO wf_decisions(instance,round,policy_id,stage,slot,subject,person,decision,payload_hash,recorded_at,event_key,identity) VALUES(?1,?2,?3,?4,?5,?6,?7,'withdraw',?8,?9,?10,?11)",params![i.id,i.round,required(&event.payload,"policy_id")?,required(&event.payload,"stage")?,required(&event.payload,"slot")?,subject,actor.person,i.payload_hash,now(),event.key,serde_json::to_string(&actor)?])?;},
            "update"=>{if subject!=i.maker_subject{return Err(Error::new("UNAUTHORIZED","Only the maker can revise the payload"))}if state.nodes[&i.state_id].spec["terminal"]!="none"{return Err(Error::new("TRANSITION_REJECTED","Terminal instance cannot be edited"))}i.data=event.payload.clone();i.round=next(i.round)?;i.payload_hash=hash::structured("runtime-review-payload",&i.data)?;invariants(&state,&i,&actor,&event.payload)?;},
            "transition"=>{let t=state.nodes.get(required(&event.payload,"transition_id")?).filter(|n|n.kind=="WorkflowTransition"&&n.spec["workflow_id"]==i.workflow_id&&n.spec["from_state_id"]==i.state_id).ok_or_else(||Error::new("TRANSITION_REJECTED","Transition is not enabled in this state"))?;
                authorize(&state,t.spec["actor_policy_id"].as_str(),&actor)?;
                invariants(&state,&i,&actor,&event.payload)?;check(&state.nodes[&i.state_id].spec["exit_conditions"],&context(&i,&actor,&event.payload))?;check(&t.spec["guard"],&context(&i,&actor,&event.payload))?;
                if let Some(policy)=t.spec["approval_policy_id"].as_str(){self.quorum(&state,&i,policy,None)?;}
                let target=required(&t.spec,"to_state_id")?;let reenter=target!=i.state_id||t.spec["reenter"]==true;
                if reenter{run_hooks(&state.nodes[&i.state_id],"on_exit",&mut i,&actor,&event.payload,hooks,&mut intents)?;}
                if t.spec["update_plan_id"].is_string(){return Err(Error::new("UNSUPPORTED_TARGET","Transactional update plan adapter is not registered"))}
                i.state_id=target.into();if reenter{check(&state.nodes[target].spec["entry_conditions"],&context(&i,&actor,&event.payload))?;run_hooks(&state.nodes[target],"on_entry",&mut i,&actor,&event.payload,hooks,&mut intents)?;}
                invariants(&state,&i,&actor,&event.payload)?;let payload=hash::structured("runtime-review-payload",&i.data)?;if payload!=i.payload_hash && t.spec["approval_policy_id"].is_string(){return Err(Error::new("REVISION_CONFLICT","Approval transition cannot change the reviewed payload"))}if payload!=i.payload_hash{i.round=next(i.round)?;i.payload_hash=payload;}
            },_=>return Err(Error::new("SCHEMA_INVALID","Unknown runtime event kind"))
        }
        i.revision=next(i.revision)?;self.save(&i)?;self.save_intents(&i,&event.key,&intents)?;let result=json!({"instance":i});self.commit_receipt(&event.key,&digest,&result,&subject)?;Ok(result)
    })}
    fn record_decision(&self,s:&State,i:&Instance,subject:&str,actor:&Mapping,e:&Event)->Result<()> {
        let policy_id=required(&e.payload,"policy_id")?;let current=&s.nodes[&i.state_id];
        if current.spec["approval_policy_id"]!=policy_id&&!s.nodes.values().any(|n|n.kind=="WorkflowTransition"&&n.spec["from_state_id"]==i.state_id&&n.spec["approval_policy_id"]==policy_id){return Err(Error::new("UNAUTHORIZED","Approval policy is not active"))}
        let policy=&s.nodes[policy_id];let stage_id=required(&e.payload,"stage")?;let slot=required(&e.payload,"slot")?;let stages=policy.spec["stages"].as_array().ok_or_else(||Error::new("SCHEMA_INVALID","Missing policy stages"))?;
        let index=stages.iter().position(|st|st["id"]==stage_id).ok_or_else(||Error::new("SCHEMA_INVALID","Unknown stage"))?;let stage=&stages[index];let eligibility=stage["slots"].get(slot).ok_or_else(||Error::new("SCHEMA_INVALID","Unknown slot"))?;
        if stage["mode"]=="sequential"&&index>0{self.quorum(s,i,policy_id,Some(index))?;}
        eligible(eligibility,actor)?;let maker=self.mapping(&i.maker_subject)?;if subject==i.maker_subject||(policy.spec["identity_basis"]=="distinct_person"&&maker.person==actor.person){return Err(Error::new("MAKER_EXCLUDED","Maker cannot approve their own request"))}
        if e.payload["payload_hash"]!=i.payload_hash||e.payload["round"]!=i.round{return Err(Error::new("REVISION_CONFLICT","Decision does not cover current payload/round"))}
        let decision=required(&e.payload,"decision")?;if !["approve","reject"].contains(&decision){return Err(Error::new("SCHEMA_INVALID","Decision must approve or reject"))}
        self.db.execute("INSERT INTO wf_decisions(instance,round,policy_id,stage,slot,subject,person,decision,payload_hash,recorded_at,event_key,identity) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",params![i.id,i.round,policy_id,stage_id,slot,subject,actor.person,decision,i.payload_hash,now(),e.key,serde_json::to_string(actor)?])?;Ok(())
    }
    fn quorum(&self,s:&State,i:&Instance,policy_id:&str,until:Option<usize>)->Result<()> {
        let policy=&s.nodes[policy_id];let maker=self.mapping(&i.maker_subject)?;let stages=policy.spec["stages"].as_array().unwrap();
        let mut q=self.db.prepare("SELECT stage,slot,subject,decision,recorded_at FROM wf_decisions WHERE instance=?1 AND round=?2 AND policy_id=?3 AND payload_hash=?4 ORDER BY id")?;
        let records=q.query_map(params![i.id,i.round,policy_id,i.payload_hash],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,u64>(4)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
        let mut effective=BTreeMap::new();for(st,slot,subject,decision,time)in records{effective.insert((st,slot,subject),(decision,time));}
        let mut prior_people=BTreeSet::new();
        for stage in stages.iter().take(until.unwrap_or(stages.len())){
            let mut candidates:BTreeMap<String,BTreeSet<String>>=BTreeMap::new();
            for((st,slot,subject),(decision,time))in &effective{
                if stage["id"]!=*st||now().saturating_sub(*time).saturating_mul(1000)>=policy.spec["decision_lifetime_ms"].as_u64().unwrap(){continue}
                let actor=match self.mapping(subject){Ok(m)=>m,Err(_)=>continue};let Some(eligibility)=stage["slots"].get(slot)else{continue};if eligible(eligibility,&actor).is_err(){continue}
                let person=if policy.spec["identity_basis"]=="distinct_person"{actor.person.clone()}else{subject.clone()};
                if subject==&i.maker_subject||(policy.spec["identity_basis"]=="distinct_person"&&actor.person==maker.person){continue}
                if decision=="reject"&&policy.spec["rejection"]!="collect"{return Err(Error::new("APPROVAL_REJECTED","Rejection requires explicit reject/rework transition"))}
                if decision!="approve"||policy.spec["cross_stage_distinct"]==true&&prior_people.contains(&person){continue}
                candidates.entry(slot.clone()).or_default().insert(person);
            }
            let mut matched=BTreeMap::new();for slot in candidates.keys(){let _=match_slot(slot,&candidates,&mut BTreeSet::new(),&mut matched);}
            let needed=stage["quorum"].as_u64().unwrap()as usize;if matched.len()<needed{return Err(Error::new("QUORUM_NOT_MET","Not enough distinct eligible reviewers"))}
            if policy.spec["cross_stage_distinct"]==true{prior_people.extend(matched.into_keys());}
        }Ok(())
    }
}
fn match_slot(slot:&str,candidates:&BTreeMap<String,BTreeSet<String>>,seen:&mut BTreeSet<String>,matched:&mut BTreeMap<String,String>)->bool{for person in candidates.get(slot).into_iter().flatten(){if !seen.insert(person.clone()){continue}let old=matched.get(person).cloned();if old.is_none()||match_slot(old.as_ref().unwrap(),candidates,seen,matched){matched.insert(person.clone(),slot.into());return true}}false}
fn required<'a>(v:&'a Value,key:&str)->Result<&'a str>{v[key].as_str().ok_or_else(||Error::new("SCHEMA_INVALID",format!("Missing {key}")))}
fn next(r:u64)->Result<u64>{if r>=hash::MAX_INTEGER{Err(Error::new("REVISION_EXHAUSTED","Runtime revision exhausted"))}else{Ok(r+1)}}
fn context(i:&Instance,actor:&Mapping,event:&Value)->Value{json!({"data":i.data,"instance_id":i.id,"event":event,"actor":{"subject":actor.subject,"issuer":actor.issuer,"tenant":actor.tenant,"person":actor.person,"roles":actor.roles,"claims":actor.claims},"round":i.round})}
fn check(predicates:&Value,ctx:&Value)->Result<()>{for p in predicates.as_array().into_iter().flatten(){if eval(p,ctx)?!=true{return Err(Error::new("CONDITION_FAILED","Workflow predicate failed"))}}Ok(())}
fn invariants(s:&State,i:&Instance,actor:&Mapping,event:&Value)->Result<()>{let ctx=context(i,actor,event);check(&s.nodes[&i.workflow_id].spec["invariants"],&ctx)?;check(&s.nodes[&i.state_id].spec["invariants"],&ctx)}
fn eligible(spec:&Value,actor:&Mapping)->Result<()>{for(k,actual)in[("role_ids",&actor.roles),("claim_ids",&actor.claims)]{for id in spec[k].as_array().into_iter().flatten().filter_map(Value::as_str){if !actual.contains(id){return Err(Error::new("UNAUTHORIZED",format!("Missing eligible {k}")))}}}Ok(())}
fn authorize(s:&State,policy:Option<&str>,actor:&Mapping)->Result<()>{if let Some(id)=policy{eligible(&s.nodes[id].spec,actor)?}Ok(())}
fn run_hooks(state:&Node,key:&str,i:&mut Instance,actor:&Mapping,event:&Value,hooks:&dyn Hooks,intents:&mut Vec<Intent>)->Result<()>{for hook in state.spec[key].as_array().into_iter().flatten(){let mut args=serde_json::Map::new();for(k,v)in hook["arguments"].as_object().unwrap(){args.insert(k.clone(),eval(v,&context(i,actor,event))?);}hooks.call(required(hook,"function_id")?,&args.into(),&mut i.data,intents)?;}Ok(())}
const SCHEMA:&str=r#"
PRAGMA journal_mode=WAL;PRAGMA synchronous=FULL;
CREATE TABLE IF NOT EXISTS wf_config(id INTEGER PRIMARY KEY,hash TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS wf_identity(subject_key TEXT PRIMARY KEY,mapping TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS wf_identity_audit(id INTEGER PRIMARY KEY,mapping TEXT NOT NULL,recorded_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS wf_credentials(token_hash TEXT PRIMARY KEY,subject_key TEXT NOT NULL,expires INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS wf_definitions(id TEXT PRIMARY KEY,payload TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS wf_instances(id TEXT PRIMARY KEY,payload TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS wf_receipts(key TEXT PRIMARY KEY,hash TEXT NOT NULL,result TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS wf_audit(id INTEGER PRIMARY KEY,event_key TEXT NOT NULL,actor TEXT NOT NULL,result TEXT NOT NULL,recorded_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS wf_decisions(id INTEGER PRIMARY KEY,instance TEXT NOT NULL,round INTEGER NOT NULL,policy_id TEXT NOT NULL,stage TEXT NOT NULL,slot TEXT NOT NULL,subject TEXT NOT NULL,person TEXT NOT NULL,decision TEXT NOT NULL,payload_hash TEXT NOT NULL,recorded_at INTEGER NOT NULL,event_key TEXT UNIQUE NOT NULL,identity TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS wf_outbox(id TEXT PRIMARY KEY,instance TEXT NOT NULL,event_key TEXT NOT NULL,intent TEXT NOT NULL,state TEXT NOT NULL,lease_token TEXT,lease_expires INTEGER,result TEXT);
CREATE TABLE IF NOT EXISTS wf_deliveries(id INTEGER PRIMARY KEY,outbox_id TEXT NOT NULL,lease_token TEXT NOT NULL,state TEXT NOT NULL,result TEXT,recorded_at INTEGER NOT NULL);
"#;
