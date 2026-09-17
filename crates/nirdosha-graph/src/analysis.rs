//! Allowlisted structural adapters. Findings are service-owned, immutable evidence.
use crate::{Graph,Result,Error,hash,store::{id,now},schema::{self,Node,State}};
use serde_json::{Value,json};
use rusqlite::{params,OptionalExtension};
use std::collections::{BTreeSet,BTreeMap};
pub const RULES:&[&str]=&["structural.readiness","workflow.reachability","workflow.literal_guards","requirements.coverage","function.reachability"];

fn finding(rule:&str,classification:&str,targets:Vec<String>,message:&str,coverage:Value)->Value{json!({"rule_id":rule,"classification":classification,"target_ids":targets,"message":message,"coverage":coverage})}
fn analyze(state:&State,rules:&[String])->Result<Vec<Value>>{
    let mut out=vec![];
    let structural=json!({"modeled_constructs":["typed declarations","explicit references"],"omitted_constructs":["runtime population","business obligations not declared"],"assumptions":[]});
    if rules.iter().any(|r|r=="structural.readiness") {for issue in crate::store::acceptance::readiness(state){out.push(finding("structural.readiness","proven_violation",issue["node_id"].as_str().map(|s|vec![s.into()]).unwrap_or_default(),issue["reason"].as_str().unwrap_or("Incomplete definition"),structural.clone()));}}
    if rules.iter().any(|r|r=="workflow.reachability") {for w in state.nodes.values().filter(|n|!n.deleted&&n.kind=="Workflow"){
        let mut reachable=BTreeSet::new();let mut queue=vec![];
        if let Some(start)=w.spec["initial_state_id"].as_str(){reachable.insert(start.to_owned());queue.push(start.to_owned())}
        while let Some(from)=queue.pop(){for t in state.nodes.values().filter(|n|!n.deleted&&n.kind=="WorkflowTransition"&&n.spec["workflow_id"]==w.id&&n.spec["from_state_id"]==from){if let Some(to)=t.spec["to_state_id"].as_str(){if reachable.insert(to.into()){queue.push(to.into())}}}}
        for state_id in w.spec["state_ids"].as_array().into_iter().flatten().filter_map(Value::as_str){if !reachable.contains(state_id){out.push(finding("workflow.reachability","proven_violation",vec![state_id.into()],"No path from the declared initial state",json!({"modeled_constructs":["declared initial state","transition topology"],"omitted_constructs":["guard feasibility","external instance migration"],"assumptions":["entry only through initial state"]})))}}
    }}
    if rules.iter().any(|r|r=="workflow.literal_guards") {for t in state.nodes.values().filter(|n|!n.deleted&&n.kind=="WorkflowTransition"){
        let guards=t.spec["guard"].as_array().cloned().unwrap_or_default();let mut unknown=false;let mut false_guard=false;
        for guard in &guards{match eval(guard,&json!({})){Ok(Value::Bool(false))=>false_guard=true,Ok(Value::Bool(true))=>(),_=>unknown=true}}
        if false_guard||unknown {out.push(finding("workflow.literal_guards",if false_guard{"proven_violation"}else{"unknown"},vec![t.id.clone()],if false_guard{"A literal guard is false"}else{"Guard depends on unsupported or runtime values"},json!({"modeled_constructs":["literal boolean predicates","integer comparisons","boolean composition"],"omitted_constructs":["symbolic values","calls","solver reasoning"],"assumptions":[]})))}
    }}
    if rules.iter().any(|r|r=="requirements.coverage"){for n in state.nodes.values().filter(|n|!n.deleted&&n.kind=="Requirement"){
        if !state.edges.values().any(|e|!e.deleted&&e.kind=="IMPLEMENTS"&&e.dst==n.id){out.push(finding("requirements.coverage","potential_issue",vec![n.id.clone()],"No declared implementation link",structural.clone()))}
    }}
    if rules.iter().any(|r|r=="function.reachability"){
        let mut reachable=BTreeSet::new();let mut calls:BTreeMap<String,BTreeSet<String>>=BTreeMap::new();
        for n in state.nodes.values().filter(|n|!n.deleted){if n.kind=="Function"&&(n.spec["entry_point"]==true||n.spec["visibility"]=="pub"){reachable.insert(n.id.clone());}
            let refs=schema::references(&n.spec).into_iter().filter(|id|state.nodes.get(id).is_some_and(|n|n.kind=="Function")).collect::<BTreeSet<_>>();
            if ["WorkflowState","Application","Screen"].contains(&n.kind.as_str()){reachable.extend(refs.clone());}calls.insert(n.id.clone(),refs);
        }
        loop{let old=reachable.len();for r in reachable.clone(){if let Some(next)=calls.get(&r){reachable.extend(next.clone());}}if old==reachable.len(){break}}
        for n in state.nodes.values().filter(|n|!n.deleted&&n.kind=="Function"&&!reachable.contains(&n.id)){out.push(finding("function.reachability","potential_issue",vec![n.id.clone()],"No path from declared roots in the modeled call graph",json!({"modeled_constructs":["typed plan calls","application roots","exported functions","workflow hooks"],"omitted_constructs":["calls in preserved source","indirect calls","external callbacks","statement control flow"],"assumptions":["potential issue only; absence of a path is not proof of dead code"]})));}
    }
    if out.len()>1000{return Err(Error::new("LIMIT_EXCEEDED","Analysis result exceeds 1000 findings; narrow the graph"))}Ok(out)
}
/// Shared pure expression evaluator. Unsupported constructs fail closed.
pub fn eval(v:&Value,bindings:&Value)->Result<Value>{
    match v["tag"].as_str().unwrap_or(""){
        "literal"=>Ok(v["value"].clone()),
        "ref"=>bindings.get(v["binding"].as_str().unwrap_or("")).cloned().ok_or_else(||Error::new("UNSUPPORTED_EXPRESSION","Missing binding")),
        "field"=>eval(&v["base"],bindings)?.get(v["field"].as_str().unwrap_or("")).cloned().ok_or_else(||Error::new("UNSUPPORTED_EXPRESSION","Missing field")),
        "not"=>eval(&v["inner"],bindings)?.as_bool().map(|b|json!(!b)).ok_or_else(||Error::new("UNSUPPORTED_EXPRESSION","Expected boolean")),
        "binary"=>{let a=eval(&v["left"],bindings)?;let b=eval(&v["right"],bindings)?;let op=v["op"].as_str().unwrap_or("");
            if op=="=="{return Ok(json!(a==b))}if op=="!="{return Ok(json!(a!=b))}
            if let (Some(a),Some(b))=(a.as_bool(),b.as_bool()){return match op{"&&"=>Ok(json!(a&&b)),"||"=>Ok(json!(a||b)),_=>Err(Error::new("UNSUPPORTED_EXPRESSION","Unsupported boolean operator"))}}
            if let(Some(a),Some(b))=(a.as_i64(),b.as_i64()){return match op{"<"=>Ok(json!(a<b)),"<="=>Ok(json!(a<=b)),">"=>Ok(json!(a>b)),">="=>Ok(json!(a>=b)),_=>Err(Error::new("UNSUPPORTED_EXPRESSION","Arithmetic requires checked target semantics"))}}
            Err(Error::new("UNSUPPORTED_EXPRESSION","Unsupported operand types"))
        },_=>Err(Error::new("UNSUPPORTED_EXPRESSION","Unsupported expression"))
    }
}
impl Graph {
    pub fn analyze(&self,args:&Value)->Result<Value>{
        self.check_access(true)?;let token=args["snapshot"].as_str().map(str::to_owned).map(Ok).unwrap_or_else(||self.new_snapshot("proposed"))?;
        let(r,view)=self.snapshot(&token)?;let rules:Vec<String>=args["rules"].as_array().map(|a|a.iter().map(|r|r.as_str().unwrap_or("").to_owned()).collect()).unwrap_or_else(||RULES.iter().map(|r|r.to_string()).collect());
        if rules.is_empty()||rules.iter().any(|r|!RULES.contains(&r.as_str())){return Err(Error::new("UNSUPPORTED_CHECKER","Only installed rules may run"))}
        let job=id("analysis")?;let request=json!({"snapshot":token,"revision":r,"view":view,"rules":rules});
        self.transaction(||{let active:u64=self.conn.query_row("SELECT COUNT(*) FROM graph_analysis_jobs WHERE state='running'",[],|r|r.get(0))?;if active>=4{return Err(Error::new("BACKPRESSURE","At most four active analyses"))}self.conn.execute("INSERT INTO graph_analysis_jobs VALUES(?1,?2,'running',?3,NULL,?4)",params![job,self.access.grant_id,request.to_string(),now()])?;Ok(())})?;
        let(root,host,access,key)=(self.root.clone(),self.host.clone(),self.access.clone(),job.clone());
        std::thread::spawn(move||{if let Ok(g)=Graph::open(&root,&host,access){if let Err(error)=g.run_analysis(&key,&request){let _=g.conn.execute("UPDATE graph_analysis_jobs SET state='failed',result=?2 WHERE id=?1 AND state='running'",params![key,error.envelope().to_string()]);}}});
        Ok(json!({"job_id":job,"state":"running","input_revision":r}))
    }
    fn run_analysis(&self,job:&str,request:&Value)->Result<()> {
        let state=self.state_view(request["revision"].as_u64().unwrap(),request["view"].as_str().unwrap())?;
        let rules:Vec<String>=serde_json::from_value(request["rules"].clone())?;
        let results=analyze(&state,&rules)?;let inputs=input_manifest(&state)?;
        let manifest=json!({"job_id":job,"request":request,"inputs":inputs,"findings":results,"checker_id":"nirdosha-graph/structural","checker_version":"1"});let artifact=self.put_blob(&serde_json::to_vec(&manifest)?)?;
        self.transaction(||{
            let status:String=self.conn.query_row("SELECT state FROM graph_analysis_jobs WHERE id=?1",[job],|r|r.get(0))?;if status!="running"{return Ok(())}
            let before=self.load_state(None)?;let mut after=before.clone();let mut ids=vec![];
            for mut spec in results.clone(){let key=id("finding")?;spec["input_revision"]=request["revision"].clone();spec["checker_id"]="nirdosha-graph/structural".into();spec["checker_version"]="1".into();spec["evidence_hash"]=artifact.clone().into();spec["scope"]="declared graph only".into();
                let n=Node{id:key.clone(),kind:"AnalysisFinding".into(),title:spec["rule_id"].as_str().unwrap().into(),symbol:None,entity_revision:0,deleted:false,origin:"checker".into(),spec_schema_version:schema::spec_version("AnalysisFinding"),spec,provenance_ids:vec![],source_refs:vec![json!({"inputs":inputs,"snapshot":request["snapshot"],"job_id":job})],protected:true,observation:None};schema::validate_spec(&n.kind,&n.spec_schema_version,&n.spec)?;after.nodes.insert(key.clone(),n);ids.push(key);
            }
            self.persist_state(&before,&mut after,self.next_revision()?,job)?;
            let result=json!({"finding_ids":ids,"manifest_artifact_id":artifact,"inputs":inputs,"checker_id":"nirdosha-graph/structural","checker_version":"1"});
            self.conn.execute("UPDATE graph_analysis_jobs SET state='completed',result=?2 WHERE id=?1",params![job,result.to_string()])?;Ok(())
        })
    }
    pub fn analysis_status(&self,job:&str,cancel:bool)->Result<Value>{
        self.check_access(cancel)?;let row:Option<(String,String,String,Option<String>)>=self.conn.query_row("SELECT grant_id,state,request,result FROM graph_analysis_jobs WHERE id=?1",[job],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        let Some((grant,mut state,request,result))=row else{return Err(Error::new("NOT_FOUND","Analysis job"))};if grant!=self.access.grant_id{return Err(Error::new("UNAUTHORIZED","Job belongs to another grant"))}
        if cancel&&state=="running"{self.conn.execute("UPDATE graph_analysis_jobs SET state='cancelled' WHERE id=?1 AND state='running'",[job])?;state=self.conn.query_row("SELECT state FROM graph_analysis_jobs WHERE id=?1",[job],|r|r.get(0))?;}
        let result:Value=result.map(|r|serde_json::from_str(&r)).transpose()?.unwrap_or(Value::Null);let current=input_manifest(&self.load_state(None)?)?;
        let freshness=if result["inputs"].is_null(){"unknown"}else if result["inputs"]==current{"current"}else{"stale"};
        Ok(json!({"job_id":job,"state":state,"request":serde_json::from_str::<Value>(&request)?,"result":result,"freshness":freshness}))
    }
}
fn input_manifest(s:&State)->Result<Value>{Ok(json!(s.nodes.values().filter(|n|!n.deleted && !["AnalysisFinding","Evidence"].contains(&n.kind.as_str())).map(|n|Ok(json!({"id":n.id,"entity_revision":n.entity_revision,"spec_hash":hash::structured("spec",&n.spec)?}))).collect::<Result<Vec<_>>>()?))}
