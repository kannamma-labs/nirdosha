//! Deterministic v2 source lowering. Unsupported enforcement is a hard error.
use crate::{Graph,Result,Error,hash,schema::{State,Node}};
use serde_json::{json,Value};
use std::collections::{BTreeMap,BTreeSet};

fn ident(s:&str)->Result<String>{syn::parse_str::<syn::Ident>(s).map_err(|_|Error::new("SCHEMA_INVALID",format!("Invalid identifier {s}")))?;Ok(s.into())}
fn name(n:&Node)->Result<String>{ident(&n.symbol.as_ref().ok_or_else(||Error::new("EMISSION_NOT_READY",format!("{} has no scoped symbol",n.id)))?.name)}
fn path(s:&State,id:&str)->Result<String>{let n=s.nodes.get(id).ok_or_else(||Error::new("UNRESOLVED_REFERENCE",id))?;if n.kind=="ExternalSymbol"{let p=n.spec["path"].as_str().ok_or_else(||Error::new("EMISSION_NOT_READY","External symbol path missing"))?;syn::parse_str::<syn::Path>(p).map_err(|e|Error::new("SCHEMA_INVALID",e.to_string()))?;return Ok(p.into())}let sym=n.symbol.as_ref().ok_or_else(||Error::new("EMISSION_NOT_READY","Referenced declaration lacks symbol"))?;let mut parts=vec!["crate".into()];for m in &sym.module_path{parts.push(ident(m)?)}parts.push(name(n)?);Ok(parts.join("::"))}
fn ordered(v:&Value,map:&str,order:&str)->Result<Vec<String>>{let fields=v[map].as_object().ok_or_else(||Error::new("EMISSION_NOT_READY",format!("Missing {map}")))?;let keys:Vec<String>=if let Some(a)=v[order].as_array(){a.iter().map(|x|x.as_str().unwrap_or("").to_owned()).collect()}else{fields.keys().cloned().collect()};if keys.iter().collect::<BTreeSet<_>>()!=fields.keys().collect::<BTreeSet<_>>(){return Err(Error::new("SCHEMA_INVALID","Order must enumerate members exactly"))}Ok(keys)}
fn ty(s:&State,v:&Value)->Result<String>{Ok(match v["tag"].as_str().unwrap_or(""){
    "primitive"=>if v["name"]=="unit"{"()".into()}else{v["name"].as_str().unwrap().into()},
    "generic"=>ident(v["name"].as_str().unwrap())?,
    "named"=>{let base=path(s,v["node_id"].as_str().unwrap())?;let args=v["type_args"].as_array().into_iter().flatten().map(|v|ty(s,v)).collect::<Result<Vec<_>>>()?;if args.is_empty(){base}else{format!("{base}<{}>",args.join(", "))}},
    "tuple"=>format!("({})",v["items"].as_array().unwrap().iter().map(|v|ty(s,v).map(|t|format!("{t},"))).collect::<Result<Vec<_>>>()?.join(" ")),
    "array"=>format!("[{}; {}]",ty(s,&v["inner"])?,v["length"]),
    "reference"=>{if !v["lifetime"].is_null(){return Err(Error::new("UNSUPPORTED_TARGET","Lifetime declarations require a registered generic adapter"))}format!("&{}{}",if v["mutable"]==true{"mut "}else{""},ty(s,&v["inner"])?)},
    "function"=>format!("fn({}) -> {}",v["parameters"].as_array().unwrap().iter().map(|v|ty(s,v)).collect::<Result<Vec<_>>>()?.join(", "),ty(s,&v["result"])?),
    _=>return Err(Error::new("EMISSION_NOT_READY","Unresolved type"))
})}
fn expression(s:&State,v:&Value,bindings:&BTreeMap<String,String>)->Result<String>{Ok(match v["tag"].as_str().unwrap_or(""){
    "literal"=>if v["value"].is_null(){"()".into()}else{serde_json::to_string(&v["value"])?},
    "ref"=>bindings.get(v["binding"].as_str().unwrap()).cloned().ok_or_else(||Error::new("UNRESOLVED_REFERENCE","Expression binding missing"))?,
    "field"=>format!("({}).{}",expression(s,&v["base"],bindings)?,ident(v["field"].as_str().unwrap())?),
    "binary"=>format!("({} {} {})",expression(s,&v["left"],bindings)?,v["op"].as_str().unwrap(),expression(s,&v["right"],bindings)?),
    "not"=>format!("(!{})",expression(s,&v["inner"],bindings)?),
    "call"=>{let id=v["function_id"].as_str().unwrap();let f=&s.nodes[id];let arguments=ordered(&f.spec,"parameters","parameter_order")?.iter().map(|p|expression(s,&v["arguments"][p],bindings)).collect::<Result<Vec<_>>>()?;format!("{}({})",path(s,id)?,arguments.join(", "))},
    _=>return Err(Error::new("UNSUPPORTED_TARGET","Unsupported expression"))
})}
pub(crate) fn body_context(s:&State,n:&Node)->Result<Value>{
    let mut signature=n.spec.clone();signature.as_object_mut().unwrap().remove("body");
    let signature_hash=hash::structured("function-signature",&json!({"symbol":n.symbol,"spec":signature}))?;
    let mut pending=crate::schema::references(&signature);let mut seen=BTreeSet::new();let mut deps=BTreeMap::new();
    while let Some(id)=pending.pop_first(){if !seen.insert(id.clone()) || id==n.id{continue}let d=s.nodes.get(&id).ok_or_else(||Error::new("UNRESOLVED_REFERENCE",&id))?;let mut spec=d.spec.clone();spec.as_object_mut().unwrap().remove("body");pending.extend(crate::schema::references(&spec));deps.insert(id,json!({"symbol":d.symbol,"spec":spec}));}
    Ok(json!({"signature_hash":signature_hash,"dependency_hash":hash::structured("function-dependencies",&json!(deps))?}))
}
fn visibility(v:&Value)->&str{match v.as_str(){Some("pub")=>"pub ",Some("pub(crate)")=>"pub(crate) ",_=>""}}
fn generics(n:&Node)->Result<String>{let names=n.spec["generics"].as_array().into_iter().flatten().map(|v|ident(v.as_str().unwrap())).collect::<Result<Vec<_>>>()?;Ok(if names.is_empty(){String::new()}else{format!("<{}>",names.join(", "))})}
fn declaration(g:&Graph,s:&State,n:&Node)->Result<String>{let spec=&n.spec;let header=format!("{}{}",visibility(&spec["visibility"]),match n.kind.as_str(){"Struct"=>"struct ","Enum"=>"enum ","Function"=>if spec["async"]==true{"async fn "}else{"fn "},_=>return Err(Error::new("UNSUPPORTED_TARGET","Declaration kind"))});let header=format!("{header}{}{}",name(n)?,generics(n)?);
    Ok(match n.kind.as_str(){
        "Struct"=>{let mut out=format!("{header} {{\n");for id in ordered(spec,"fields","field_order")?{let f=&spec["fields"][id];if f.get("default").is_some() || f["policy_ids"].as_array().is_some_and(|a|!a.is_empty()){return Err(Error::new("UNSUPPORTED_TARGET","Field defaults/policies require an enforcing adapter"))}out+=&format!("    {}{}: {},\n",visibility(&f["visibility"]),ident(f["name"].as_str().unwrap())?,ty(s,&f["type"])?) }out+"}\n"},
        "Enum"=>{let mut out=format!("{header} {{\n");for id in ordered(spec,"variants","variant_order")?{let v=&spec["variants"][id];let fields=if v.get("fields").is_some(){ordered(v,"fields","field_order")?}else{vec![]};let fields=fields.iter().map(|id|{let f=&v["fields"][id];let t=ty(s,&f["type"])?;Ok(if v["style"]=="named"{format!("{}: {t}",ident(f["name"].as_str().unwrap())?)}else{t})}).collect::<Result<Vec<_>>>()?;out+=&format!("    {}{},\n",ident(v["name"].as_str().unwrap())?,match v["style"].as_str(){Some("tuple")=>format!("({})",fields.join(", ")),Some("named")=>format!(" {{ {} }}",fields.join(", ")),_=>String::new()});}out+"}\n"},
        "Function"=>{for k in ["contract_ids","policy_ids","effects"]{if spec[k].as_array().is_some_and(|a|!a.is_empty()){return Err(Error::new("UNSUPPORTED_TARGET",format!("{k} needs a checker-backed lowering adapter")))}}
            let mut bindings=BTreeMap::new();let args=ordered(spec,"parameters","parameter_order")?.iter().map(|id|{let p=&spec["parameters"][id];let name=ident(p["name"].as_str().unwrap())?;bindings.insert(id.clone(),name.clone());Ok(format!("{name}: {}",ty(s,&p["type"])?))}).collect::<Result<Vec<_>>>()?;
            let body=&spec["body"];let text=match body["tag"].as_str(){Some("source")=>{let context=body_context(s,n)?;if body["signature_hash"]!=context["signature_hash"] || body["dependency_hash"]!=context["dependency_hash"]{return Err(Error::new("BODY_CONTEXT_STALE",format!("Body for {} needs reconciliation",n.id)))}String::from_utf8(g.blob(body["blob_hash"].as_str().unwrap())?).map_err(|_|Error::new("SCHEMA_INVALID","Body is not UTF-8"))?},Some("plan")=>{let plan=&s.nodes[body["plan_id"].as_str().unwrap()].spec;let mut out=String::new();for step in plan["steps"].as_array().into_iter().flatten(){out+=&format!("{};\n",expression(s,step,&bindings)?)}out+&expression(s,&plan["result"],&bindings)?},_=>return Err(Error::new("EMISSION_NOT_READY",format!("Body slot {} is unimplemented or unsupported",n.id)))};
            syn::parse_str::<syn::Block>(&format!("{{{text}}}")).map_err(|e|Error::new("SCHEMA_INVALID",e.to_string()))?;
            format!("{header}({}) -> {} {{\n{text}\n}}\n",args.join(", "),ty(s,&spec["return_type"])?)
        },_=>unreachable!()
    })
}
#[derive(Default)]struct Module {declarations:Vec<(String,String)>, children:BTreeMap<String,Module>}
fn render(m:&Module,out:&mut String,map:&mut Vec<Value>){for(id,text)in &m.declarations{let start=out.lines().count()+1;out.push_str(text);out.push('\n');map.push(json!({"node_id":id,"start_line":start,"end_line":out.lines().count()}))}for(name,child)in &m.children{out.push_str(&format!("pub mod {name} {{\n"));render(child,out,map);out.push_str("}\n");}}
impl Graph {
    pub fn body_context(&self,id:&str,snapshot:Option<&str>)->Result<Value>{self.check_access(false)?;let(r,view)=if let Some(t)=snapshot{self.snapshot(t)?}else{(self.version()?.revision,"proposed".into())};let s=self.state_view(r,&view)?;let n=s.nodes.get(id).ok_or_else(||Error::new("NOT_FOUND",id))?;if n.kind!="Function"{return Err(Error::new("SCHEMA_INVALID","Expected Function"))}body_context(&s,n)}
    pub fn emit(&self,args:&Value)->Result<Value>{
        self.check_access(false)?;
        let token=if let Some(t)=args["snapshot"].as_str(){t.to_owned()}else{self.new_snapshot("accepted")?};let(r,view)=self.snapshot(&token)?;
        if view!="accepted"{return Err(Error::new("EMISSION_NOT_READY","Emission requires an accepted snapshot"))}
        let s=self.state_view(r,&view)?;crate::schema::validate_state(&s)?;
        let issues=crate::store::acceptance::readiness(&s);if !issues.is_empty(){return Err(Error::new("EMISSION_NOT_READY","Incomplete specification").details(json!({"issues":issues})))}
        let mut root=Module::default();let mut declarations=s.nodes.values().filter(|n|!n.deleted && ["Struct","Enum","Function"].contains(&n.kind.as_str())).collect::<Vec<_>>();
        declarations.sort_by_key(|n|n.symbol.as_ref().map(|s|(s.package_id.clone(),s.module_path.clone(),s.namespace.clone(),s.name.clone())));
        let mut packages=BTreeSet::new();for n in &declarations{if let Some(sym)=&n.symbol{packages.insert(sym.package_id.clone());}}if packages.len()>1{return Err(Error::new("UNSUPPORTED_TARGET","Emit each package through a registered package adapter"))}
        for n in s.nodes.values().filter(|n|!n.deleted){if ["Workflow","Screen","Contract","Policy","ApprovalPolicy"].contains(&n.kind.as_str()){return Err(Error::new("UNSUPPORTED_TARGET",format!("{} needs an enforcing target adapter",n.kind)))}}
        for n in declarations{let text=declaration(self,&s,n)?;let mut module=&mut root;for part in &n.symbol.as_ref().unwrap().module_path{ident(part)?;module=module.children.entry(part.clone()).or_default()}module.declarations.push((n.id.clone(),text));}
        let mut source="// Generated from an accepted Nirdosha graph. Source dialect: v2.\n".to_owned();let mut map=vec![];render(&root,&mut source,&mut map);
        syn::parse_file(&source).map_err(|e|Error::new("EMISSION_NOT_READY",e.to_string()))?;
        if source.len()>512*1024{return Err(Error::new("LIMIT_EXCEEDED","Emitted source exceeds inline limit"))}
        let mut version=self.version()?;version.revision=r;self.check_access(false)?;
        Ok(json!({"graph":version,"dialect":"nirdosha-v2","source":source,"source_hash":hash::bytes(source.as_bytes()),"source_map":map,"emitter_version":"0021-v2/1","build_status":"not_checked","warnings":s.edges.values().filter(|e|!e.deleted&&e.kind=="RELATES_TO").map(|e|json!({"edge_id":e.id,"message":"RELATES_TO has no executable meaning"})).collect::<Vec<_>>()}))
    }
}
