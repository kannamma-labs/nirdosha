//! Transport-independent project MCP handlers; language tools remain separate.
use crate::{Error,Graph,Result,schema};
use serde_json::{Value,json};

fn s()->Value{json!({"type":"string"})}
fn n()->Value{json!({"type":"integer","minimum":0})}
fn obj(fields:&[(&str,Value)],required:&[&str])->Value{json!({"type":"object","properties":fields.iter().map(|(k,v)|(k.to_string(),v.clone())).collect::<serde_json::Map<_,_>>(),"required":required,"additionalProperties":false})}
pub fn tools(graph:&Graph)->Value {
    let mut definitions=vec![
        ("graph_analysis_status","Read a checker run and its current freshness",obj(&[("job_id",s())],&["job_id"])),
        ("graph_emit","Deterministically lower an accepted snapshot to v2 source",obj(&[("snapshot",s())],&[])),
        ("graph_body_context","Read signature/dependency hashes required for a body slot",obj(&[("id",s()),("snapshot",s())],&["id"])),
        ("graph_schema","Read supported graph/spec schemas and capability limits",obj(&[("kind",s())],&[])),
        ("graph_get_node","Retrieve one exact node",obj(&[("id",s()),("snapshot",s())],&["id"])),
        ("graph_get_edge","Retrieve one exact relationship",obj(&[("id",s()),("snapshot",s())],&["id"])),
        ("graph_get_graph","Read every selected entity through consistent bounded pages",obj(&[("view",s()),("snapshot",s()),("cursor",s()),("filter_hash",s()),("limit",n()),("include_deleted",json!({"type":"boolean"}))],&[])),
        ("graph_query","Query nodes, edges or gaps without arbitrary SQL",obj(&[("view",s()),("snapshot",s()),("cursor",s()),("filter_hash",s()),("limit",n()),("entity_type",s()),("kind",s()),("text",s()),("include_deleted",json!({"type":"boolean"}))],&[])),
        ("graph_neighbors","Read a bounded typed neighborhood",obj(&[("node_id",s()),("snapshot",s()),("depth",n()),("direction",s()),("edge_kinds",json!({"type":"array","items":s()}))],&["node_id"])),
        ("graph_get_changes","Read complete committed transaction groups",obj(&[("after_cursor",s()),("limit",n())],&[])),
        ("graph_export","Export a complete immutable graph metadata snapshot",obj(&[("snapshot",s())],&[])),
        ("graph_read_artifact","Read artifact bytes in bounded base64 chunks",obj(&[("artifact_id",s()),("offset",n()),("length",n())],&["artifact_id"])),
        ("graph_validate","Check structure, acceptance readiness and declared emission support",obj(&[("snapshot",s()),("view",s()),("mode",s()),("acceptance",json!({"type":"object"}))],&[])),
        ("graph_stream_status","Recover the next committed authoring sequence",obj(&[("stream_id",s())],&["stream_id"])),
    ];
    if graph.access.author {definitions.extend([
        ("graph_analyze","Run installed bounded structural checker rules",obj(&[("snapshot",s()),("rules",json!({"type":"array","items":s()}))],&[])),
        ("graph_analysis_cancel","Cancel a pending checker run",obj(&[("job_id",s())],&["job_id"])),
        ("graph_apply","Atomically apply a typed patch; reuse its mutation ID on retry",patch_schema()),
        ("graph_stream_open","Open/recover an actor-bound authoring stream",obj(&[("client_key",s())],&["client_key"])),
        ("graph_stream_append","Commit one complete replay-safe authoring frame",obj(&[("stream_id",s()),("sequence",n()),("patch",patch_schema())],&["stream_id","sequence","patch"])),
        ("graph_stream_close","Close or cancel a stream without undoing committed facts",obj(&[("stream_id",s()),("final_sequence",n()),("cancel",json!({"type":"boolean"}))],&["stream_id","final_sequence"])),
        ("graph_put_artifact","Persist complete UTF-8 source/evidence bytes by hash",obj(&[("content",s())],&["content"])),
    ]);}
    if graph.access.accept{definitions.push(("graph_accept","Accept an explicit exact dependency-consistent specification set",obj(&[("mutation_id",s()),("base_acceptance_id",json!({"type":["string","null"]})),("selections",json!({"type":"array","items":precondition_schema()})),("removals",json!({"type":"array","items":obj(&[("entity_type",s()),("id",s())],&["entity_type","id"])})),("dependency_pins",json!({"type":"array","items":obj(&[("id",s()),("entity_revision",n())],&["id","entity_revision"])}))],&["mutation_id","base_acceptance_id","selections","dependency_pins"])));}
    json!({"tools":definitions.into_iter().map(|(name,description,input)|json!({"name":name,"description":description,"inputSchema":input,"annotations":{"readOnlyHint":!matches!(name,"graph_apply"|"graph_accept"|"graph_stream_open"|"graph_stream_append"|"graph_stream_close"|"graph_put_artifact")}})).collect::<Vec<_>>()})
}
fn precondition_schema()->Value{obj(&[("entity_type",s()),("id",s()),("entity_revision",n())],&["entity_type","id","entity_revision"])}
pub fn patch_schema()->Value {
    obj(&[("schema_version",json!({"const":schema::PATCH_SCHEMA})),("project_id",s()),("epoch",s()),("mutation_id",s()),("preconditions",json!({"type":"array","items":precondition_schema()})),("operations",json!({"type":"array","maxItems":100,"items":{"type":"object","required":["op"],"properties":{"op":{"enum":["node.create","node.rename","spec.set","spec.unset","body.attach","source.observe","edge.create","edge.replace","entity.tombstone","gap.add","gap.resolve","gap.reopen","gap.supersede","gap.obsolete"]}}}})),("evidence",json!({"type":"array"})),("if_head_revision",n())],&["schema_version","project_id","epoch","mutation_id","preconditions","operations"])
}
pub fn call(graph:&Graph,name:&str,args:&Value)->Value {
    match execute(graph,name,args){Ok(v)=>json!({"isError":false,"structuredContent":v,"content":[{"type":"text","text":v.to_string()}]}),Err(e)=>{let v=e.envelope();json!({"isError":true,"structuredContent":v,"content":[{"type":"text","text":v.to_string()}]})}}
}
pub fn execute(graph:&Graph,name:&str,args:&Value)->Result<Value>{
    graph.check_access(false)?;
    let text=|key:&str|args[key].as_str().ok_or_else(||Error::new("SCHEMA_INVALID",format!("Missing {key}")));
    let number=|key:&str|args[key].as_u64().ok_or_else(||Error::new("SCHEMA_INVALID",format!("Missing unsigned integer {key}")));
    match name {
        "graph_schema"=>{let specs=if let Some(k)=args["kind"].as_str(){json!({k:schema::spec_schema(k)?})}else{schema::KINDS.iter().map(|k|Ok((k.to_string(),schema::spec_schema(k)?))).collect::<Result<serde_json::Map<_,_>>>()?.into()};Ok(json!({"graph":graph.version()?,"specs":specs,"patch_schema":patch_schema(),"source_dialect":"nirdosha-v2","limits":{"page_entities":200,"page_bytes":524288,"patch_bytes":262144,"operations":100},"runtime_capabilities":[],"checker_capabilities":["structural"]}))},
        "graph_get_node"=>graph.get("node",text("id")?,args["snapshot"].as_str()),
        "graph_get_edge"=>graph.get("edge",text("id")?,args["snapshot"].as_str()),
        "graph_get_graph"|"graph_query"=>graph.page(args),
        "graph_neighbors"=>graph.neighbors(args),
        "graph_get_changes"=>graph.changes(args["after_cursor"].as_str(),args["limit"].as_u64().unwrap_or(100)as usize),
        "graph_export"=>graph.export(args["snapshot"].as_str()),
        "graph_read_artifact"=>graph.read_artifact(text("artifact_id")?,args["offset"].as_u64().unwrap_or(0)as usize,args["length"].as_u64().unwrap_or(65536)as usize),
        "graph_emit"=>graph.emit(args),"graph_body_context"=>graph.body_context(text("id")?,args["snapshot"].as_str()),
        "graph_analyze"=>graph.analyze(args),"graph_analysis_status"=>graph.analysis_status(text("job_id")?,false),"graph_analysis_cancel"=>graph.analysis_status(text("job_id")?,true),
        "graph_apply"=>graph.apply(args),"graph_accept"=>graph.accept(args),"graph_validate"=>graph.validate(args),
        "graph_stream_open"=>graph.stream_open(text("client_key")?),
        "graph_stream_status"=>graph.stream_status(text("stream_id")?),
        "graph_stream_append"=>graph.stream_append(text("stream_id")?,number("sequence")?,&args["patch"]),
        "graph_stream_close"=>graph.stream_close(text("stream_id")?,number("final_sequence")?,args["cancel"].as_bool().unwrap_or(false)),
        "graph_put_artifact"=>Ok(json!({"artifact_id":graph.put_blob(text("content")?.as_bytes())?})),
        _=>Err(Error::new("SCHEMA_INVALID",format!("Unknown graph tool {name}"))),
    }
}

pub fn resources(graph:&Graph)->Result<Value>{let v=graph.version()?;Ok(json!({"resources":[{"uri":format!("nirdosha-hi://{}/graph/head",v.project_id),"name":"Project graph head","mimeType":"application/json"}]}))}
pub fn templates(graph:&Graph)->Result<Value>{let v=graph.version()?;Ok(json!({"resourceTemplates": ([("nodes/{node_id}","Node"),("edges/{edge_id}","Edge"),("snapshots/{snapshot_id}/manifest","Snapshot"),("artifacts/{artifact_id}/manifest","Artifact")].iter().map(|(path,name)|json!({"uriTemplate":format!("nirdosha-hi://{}/{path}",v.project_id),"name":name,"mimeType":"application/json"})).collect::<Vec<_>>())}))}
pub fn resource_read(graph:&Graph,uri:&str)->Result<Value>{
    let v=graph.version()?;let prefix=format!("nirdosha-hi://{}/",v.project_id);let path=uri.strip_prefix(&prefix).ok_or_else(||Error::new("UNAUTHORIZED","Resource belongs to another project"))?;
    let value=if path=="graph/head"{json!({"graph":v})}else if let Some(id)=path.strip_prefix("nodes/"){graph.get("node",id,None)?}else if let Some(id)=path.strip_prefix("edges/"){graph.get("edge",id,None)?}else if let Some(id)=path.strip_prefix("snapshots/").and_then(|x|x.strip_suffix("/manifest")){let(r,view)=graph.snapshot(id)?;json!({"snapshot":id,"revision":r,"view":view})}else if let Some(id)=path.strip_prefix("artifacts/").and_then(|x|x.strip_suffix("/manifest")){let mut value=graph.read_artifact(id,0,1)?;value.as_object_mut().unwrap().remove("bytes_base64");value}else{return Err(Error::new("NOT_FOUND","Resource not found"));};
    graph.check_access(false)?;Ok(json!({"contents":[{"uri":uri,"mimeType":"application/json","text":value.to_string()}]}))
}
