use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Precondition {
    pub entity_type: String,
    pub id: String,
    pub entity_revision: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Patch {
    pub schema_version: String,
    pub project_id: String,
    pub epoch: String,
    pub mutation_id: String,
    pub preconditions: Vec<Precondition>,
    pub operations: Vec<Value>,
    #[serde(default)]
    pub evidence: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub if_head_revision: Option<u64>,
}

impl Graph {
    pub fn apply(&self, raw: &Value) -> Result<Value> {
        self.transaction(|| self.apply_inner(raw, "direct"))
    }
    pub(crate) fn apply_inner(&self, raw: &Value, binding: &str) -> Result<Value> {
        if serde_json::to_vec(raw)?.len() > 256 * 1024 {
            return Err(Error::new("LIMIT_EXCEEDED", "Patch exceeds 256 KiB"));
        }
        let p: Patch = serde_json::from_value(raw.clone())?;
        let v = self.raw_version()?;
        if p.schema_version != schema::PATCH_SCHEMA
            || p.project_id != v.project_id
            || p.epoch != v.epoch
        {
            return Err(Error::new(
                "SCHEMA_INVALID",
                "Patch schema, project or epoch mismatch",
            ));
        }
        if p.mutation_id.is_empty() || p.mutation_id.len() > 256 || p.operations.len() > 100 {
            return Err(Error::new("LIMIT_EXCEEDED", "Mutation ID/operation limit"));
        }
        let digest = hash::structured("patch", raw)?;
        let receipt:Option<(String,String,String)>=self.conn.query_row("SELECT hash,binding,receipt FROM graph_receipts WHERE actor=?1 AND mutation_id=?2 AND epoch=?3",params![self.access.actor,p.mutation_id,p.epoch],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        if let Some((h, b, r)) = receipt {
            if b != binding {
                return Err(Error::new(
                    "MUTATION_BINDING_CONFLICT",
                    "Mutation belongs to another transport/sequence",
                ));
            }
            if h != digest {
                return Err(Error::new(
                    "IDEMPOTENCY_CONFLICT",
                    "Mutation ID has a different committed payload",
                ));
            }
            return Ok(serde_json::from_str(&r)?);
        }
        if p.if_head_revision.is_some_and(|r| r != v.revision) {
            return Err(Error::new("REVISION_CONFLICT", "Graph head changed")
                .details(json!({"current_revision":v.revision})));
        }
        let before = self.load_state(None)?;
        let mut state = before.clone();
        let mut id_map = BTreeMap::<String, String>::new();
        for op in &p.operations {
            if op["op"] == "node.create" || op["op"] == "edge.create" || op["op"] == "gap.add" {
                let local = required(op, "local_id")?;
                if !local.starts_with('@') || id_map.contains_key(local) {
                    return Err(Error::new(
                        "SCHEMA_INVALID",
                        "Create requires a unique @local_id",
                    ));
                }
                id_map.insert(
                    local.into(),
                    id(if op["op"] == "node.create" {
                        "n"
                    } else if op["op"] == "edge.create" {
                        "e"
                    } else {
                        "g"
                    })?,
                );
            }
        }
        let mut touched = BTreeSet::<(String, String)>::new();
        for raw_op in &p.operations {
            let mut op = raw_op.clone();
            resolve_local(&mut op, &id_map);
            let code = required(&op, "op")?;
            match code {
                "node.create" => {
                    keys(
                        &op,
                        &[
                            "op",
                            "local_id",
                            "kind",
                            "title",
                            "symbol",
                            "spec",
                            "spec_schema_version",
                        ],
                    )?;
                    let key = required(&op, "local_id")?.to_string();
                    let kind = required(&op, "kind")?;
                    if ["Evidence", "AnalysisFinding"].contains(&kind) {
                        return Err(Error::new(
                            "PROTECTED_ENTITY",
                            "Only trusted checker adapters create evidence",
                        ));
                    }
                    let spec = op.get("spec").cloned().unwrap_or(json!({}));
                    let version = op["spec_schema_version"]
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| schema::spec_version(kind));
                    schema::validate_spec(kind, &version, &spec)?;
                    let symbol = op
                        .get("symbol")
                        .filter(|v| !v.is_null())
                        .map(|v| serde_json::from_value(v.clone()))
                        .transpose()?;
                    state.nodes.insert(
                        key.clone(),
                        Node {
                            id: key.clone(),
                            kind: kind.into(),
                            title: required(&op, "title")?.into(),
                            symbol,
                            entity_revision: 0,
                            deleted: false,
                            origin: "agent".into(),
                            spec_schema_version: version,
                            spec,
                            provenance_ids: vec![],
                            source_refs: vec![],
                            protected: false,
                            observation: None,
                        },
                    );
                    touched.insert(("node".into(), key));
                }
                "source.observe" => {
                    keys(&op, &["op", "node_id", "source_ref", "declaration"])?;
                    let key = required(&op, "node_id")?.to_owned();
                    let n = state
                        .nodes
                        .get_mut(&key)
                        .filter(|n| !n.deleted)
                        .ok_or_else(|| Error::new("NOT_FOUND", &key))?;
                    writable(n)?;
                    let source_ref = op
                        .get("source_ref")
                        .cloned()
                        .ok_or_else(|| Error::new("SCHEMA_INVALID", "Missing source_ref"))?;
                    let declaration = op.get("declaration").cloned().unwrap_or(json!({}));
                    // Recorded, not merged/reconciled with `spec` -- RFC
                    // 0021 §5.3's own words: "observations can disagree
                    // with the accepted specification." A later reader
                    // compares the two itself; this operation only ever
                    // records what source parsing actually found.
                    n.observation = Some(json!({
                        "source_ref": source_ref,
                        "declaration": declaration,
                        "observed_at_revision": n.entity_revision,
                    }));
                    touched.insert(("node".into(), key));
                }
                "spec.set" | "spec.unset" | "node.rename" | "body.attach" => {
                    keys(
                        &op,
                        &["op", "node_id", "path", "value", "title", "symbol", "body"],
                    )?;
                    let key = required(&op, "node_id")?.to_owned();
                    let n = state
                        .nodes
                        .get_mut(&key)
                        .filter(|n| !n.deleted)
                        .ok_or_else(|| Error::new("NOT_FOUND", &key))?;
                    writable(n)?;
                    if ["Document", "Chunk"].contains(&n.kind.as_str()) && code.starts_with("spec.")
                    {
                        return Err(Error::new(
                            "PROTECTED_ENTITY",
                            "Document content versions are immutable; supersede them",
                        ));
                    }
                    match code {
                        "node.rename" => {
                            n.title = required(&op, "title")?.into();
                            if let Some(s) = op.get("symbol") {
                                n.symbol = serde_json::from_value(s.clone())?;
                            }
                        }
                        "body.attach" => {
                            let body = op
                                .get("body")
                                .ok_or_else(|| Error::new("SCHEMA_INVALID", "Missing body"))?;
                            if body["tag"] == "source" {
                                let bytes = self.blob(required(body, "blob_hash")?)?;
                                let text = String::from_utf8(bytes).map_err(|_| {
                                    Error::new("SCHEMA_INVALID", "Body must be UTF-8")
                                })?;
                                syn::parse_str::<syn::Block>(&format!("{{{text}}}"))
                                    .map_err(|e| Error::new("SCHEMA_INVALID", e.to_string()))?;
                            }
                            n.spec["body"] = body.clone();
                        }
                        _ => {
                            set_path(
                                &mut n.spec,
                                required(&op, "path")?,
                                if code == "spec.unset" {
                                    None
                                } else {
                                    Some(op.get("value").cloned().ok_or_else(|| {
                                        Error::new("SCHEMA_INVALID", "Missing value")
                                    })?)
                                },
                            )?
                        }
                    }
                    touched.insert(("node".into(), key));
                }
                "edge.create" | "edge.replace" => {
                    keys(
                        &op,
                        &[
                            "op", "local_id", "id", "src", "dst", "kind", "role", "payload",
                        ],
                    )?;
                    let key = required(
                        &op,
                        if code == "edge.create" {
                            "local_id"
                        } else {
                            "id"
                        },
                    )?
                    .to_owned();
                    if code == "edge.replace" {
                        let old = state
                            .edges
                            .get(&key)
                            .ok_or_else(|| Error::new("NOT_FOUND", &key))?;
                        if old.authority != "asserted" {
                            return Err(Error::new(
                                "PROTECTED_ENTITY",
                                "Derived edges are not editable",
                            ));
                        }
                    }
                    let kind = required(&op, "kind")?;
                    if ![
                        "IMPLEMENTS",
                        "RELATES_TO",
                        "SUPPORTED_BY",
                        "SUPERSEDES",
                        "CONTRADICTS",
                    ]
                    .contains(&kind)
                    {
                        return Err(Error::new(
                            "PROTECTED_ENTITY",
                            "Mechanical edges must come from typed specifications",
                        ));
                    }
                    let payload = op.get("payload").cloned().unwrap_or(json!({}));
                    if payload != json!({}) {
                        return Err(Error::new(
                            "SCHEMA_INVALID",
                            "Knowledge edge payload must be empty",
                        ));
                    }
                    state.edges.insert(
                        key.clone(),
                        Edge {
                            id: key.clone(),
                            src: required(&op, "src")?.into(),
                            dst: required(&op, "dst")?.into(),
                            kind: kind.into(),
                            role: op["role"].as_str().map(str::to_owned),
                            payload,
                            authority: "asserted".into(),
                            entity_revision: state.edges.get(&key).map_or(0, |e| e.entity_revision),
                            deleted: false,
                            provenance_ids: vec![],
                        },
                    );
                    touched.insert(("edge".into(), key));
                }
                "entity.tombstone" => {
                    keys(&op, &["op", "entity_type", "id"])?;
                    let key = required(&op, "id")?.to_owned();
                    let kind = required(&op, "entity_type")?;
                    match kind {
                        "node" => {
                            let n = state
                                .nodes
                                .get_mut(&key)
                                .ok_or_else(|| Error::new("NOT_FOUND", &key))?;
                            writable(n)?;
                            n.deleted = true;
                        }
                        "edge" => {
                            let e = state
                                .edges
                                .get_mut(&key)
                                .ok_or_else(|| Error::new("NOT_FOUND", &key))?;
                            if e.authority != "asserted" {
                                return Err(Error::new(
                                    "PROTECTED_ENTITY",
                                    "Cannot delete derived edge",
                                ));
                            }
                            e.deleted = true;
                        }
                        _ => {
                            return Err(Error::new(
                                "SCHEMA_INVALID",
                                "Tombstone requires node or edge",
                            ));
                        }
                    }
                    touched.insert((kind.into(), key));
                }
                "gap.add" => {
                    keys(
                        &op,
                        &[
                            "op",
                            "local_id",
                            "node_id",
                            "member_id",
                            "path_hint",
                            "category",
                            "blocking",
                            "reason",
                        ],
                    )?;
                    let key = required(&op, "local_id")?.to_owned();
                    let node_id = required(&op, "node_id")?.to_owned();
                    state.gaps.insert(
                        key.clone(),
                        Gap {
                            id: key.clone(),
                            node_id,
                            member_id: op["member_id"].as_str().map(str::to_owned),
                            path_hint: op["path_hint"].as_str().unwrap_or("").into(),
                            category: required(&op, "category")?.into(),
                            origin: "agent".into(),
                            status: "open".into(),
                            blocking: op["blocking"].as_bool().unwrap_or(true),
                            reason: required(&op, "reason")?.into(),
                            entity_revision: 0,
                            evidence_refs: p.evidence.clone(),
                            resolution: None,
                            supersedes: None,
                        },
                    );
                    touched.insert(("gap".into(), key));
                }
                "gap.resolve" | "gap.reopen" | "gap.supersede" | "gap.obsolete" => {
                    keys(
                        &op,
                        &[
                            "op",
                            "id",
                            "evidence",
                            "reason",
                            "replacement_id",
                            "path_hint",
                        ],
                    )?;
                    let key = required(&op, "id")?.to_owned();
                    let g = state
                        .gaps
                        .get_mut(&key)
                        .ok_or_else(|| Error::new("NOT_FOUND", &key))?;
                    if g.origin != "agent" {
                        return Err(Error::new(
                            "PROTECTED_ENTITY",
                            "Only owning validator can resolve this gap",
                        ));
                    }
                    if ["obsolete", "superseded"].contains(&g.status.as_str()) {
                        return Err(Error::new("SCHEMA_INVALID", "Gap is terminal"));
                    }
                    let evidence = op["evidence"]
                        .as_array()
                        .filter(|x| !x.is_empty())
                        .ok_or_else(|| {
                            Error::new("SCHEMA_INVALID", "Gap lifecycle change requires evidence")
                        })?;
                    let reason = required(&op, "reason")?;
                    g.status = match code {
                        "gap.resolve" => "resolved",
                        "gap.reopen" => "open",
                        "gap.supersede" => "superseded",
                        _ => "obsolete",
                    }
                    .into();
                    g.resolution = Some(json!({"reason":reason,"evidence":evidence}));
                    g.evidence_refs = evidence.clone();
                    if code == "gap.supersede" {
                        g.supersedes = Some(required(&op, "replacement_id")?.into());
                    }
                    if let Some(path) = op["path_hint"].as_str() {
                        g.path_hint = path.into();
                    }
                    touched.insert(("gap".into(), key));
                }
                _ => {
                    return Err(Error::new(
                        "SCHEMA_INVALID",
                        format!("Unknown operation {code}"),
                    ));
                }
            }
        }
        let mut required_pins = touched.clone();
        for (kind, key) in &touched {
            for s in [&before, &state] {
                match kind.as_str() {
                    "node" => {
                        if let Some(n) = s.nodes.get(key) {
                            for dep in schema::references(&n.spec) {
                                required_pins.insert(("node".into(), dep));
                            }
                        }
                    }
                    "edge" => {
                        if let Some(e) = s.edges.get(key) {
                            required_pins.insert(("node".into(), e.src.clone()));
                            required_pins.insert(("node".into(), e.dst.clone()));
                        }
                    }
                    "gap" => {
                        if let Some(g) = s.gaps.get(key) {
                            required_pins.insert(("node".into(), g.node_id.clone()));
                        }
                    }
                    _ => (),
                }
            }
        }
        let pins: BTreeMap<_, _> = p
            .preconditions
            .iter()
            .map(|p| ((p.entity_type.clone(), p.id.clone()), p.entity_revision))
            .collect();
        if pins.len() != p.preconditions.len() {
            return Err(Error::new("SCHEMA_INVALID", "Duplicate precondition"));
        }
        for ((kind, key), expected) in &pins {
            if entity_revision(&before, kind, key) != Some(*expected) {
                return Err(Error::new("REVISION_CONFLICT",format!("Changed {kind} {key}")).details(json!({"id":key,"entity_type":kind,"current_revision":entity_revision(&before,kind,key)})));
            }
        }
        let missing:Vec<_>=required_pins.iter().filter(|(k,id)|entity_revision(&before,k,id).is_some()&&!pins.contains_key(&(k.clone(),id.clone()))).map(|(k,id)|json!({"entity_type":k,"id":id,"entity_revision":entity_revision(&before,k,id)})).collect();
        if !missing.is_empty() {
            return Err(Error::new(
                "PRECONDITION_REQUIRED",
                "Missing semantic dependency revisions",
            )
            .details(json!({"required":missing})));
        }
        derive_edges(&mut state)?;
        schema::validate_state(&state)?;
        let revision = self.next_revision()?;
        let provenance = id("prov")?;
        // Provenance records only semantic changes: repeated value assignments are no-ops.
        for (k, n) in &mut state.nodes {
            if before.nodes.get(k) != Some(n) {
                n.provenance_ids.push(provenance.clone());
            }
        }
        for (k, e) in &mut state.edges {
            if before.edges.get(k) != Some(e) {
                e.provenance_ids.push(provenance.clone());
            }
        }
        let changes = self.persist_state(&before, &mut state, revision, &p.mutation_id)?;
        if !changes.is_empty() {
            self.conn.execute(
                "INSERT INTO graph_provenance VALUES(?1,?2,?3,?4,?5)",
                params![
                    provenance,
                    revision,
                    self.access.actor,
                    p.mutation_id,
                    serde_json::to_string(&p.evidence)?
                ],
            )?;
            self.reindex_chunks(&state)?;
        }
        let v = self.raw_version()?;
        let receipt = json!({"schema_version":schema::GRAPH_SCHEMA,"project_id":v.project_id,"epoch":v.epoch,"revision":v.revision,"mutation_id":p.mutation_id,"committed":true,"changed_entities":changes.iter().map(|c|json!({"entity_type":c["entity_type"],"id":c["id"],"entity_revision":c["entity_revision"]})).collect::<Vec<_>>(),"id_map":id_map,"change_cursor":format!("{}:{}",v.epoch,v.revision),"remaining_gaps":state.gaps.values().filter(|g|g.status=="open").map(|g|g.id.clone()).collect::<Vec<_>>()});
        self.conn.execute(
            "INSERT INTO graph_receipts VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                self.access.actor,
                p.mutation_id,
                p.epoch,
                digest,
                binding,
                serde_json::to_string(&receipt)?
            ],
        )?;
        Ok(receipt)
    }
    pub(crate) fn reindex_chunks(&self, state: &State) -> Result<()> {
        self.conn.execute("DELETE FROM graph_chunks_fts", [])?;
        for n in state
            .nodes
            .values()
            .filter(|n| !n.deleted && n.kind == "Chunk")
        {
            if let Some(h) = n.spec["blob_hash"].as_str() {
                let content = String::from_utf8(self.blob(h)?)
                    .map_err(|_| Error::new("SCHEMA_INVALID", "Chunk must be UTF-8"))?;
                self.conn.execute(
                    "INSERT INTO graph_chunks_fts VALUES(?1,?2,?3)",
                    params![n.id, n.entity_revision, content],
                )?;
            }
        }
        Ok(())
    }
}

pub(crate) fn entity_revision(s: &State, kind: &str, id: &str) -> Option<u64> {
    match kind {
        "node" => s.nodes.get(id).map(|n| n.entity_revision),
        "edge" => s.edges.get(id).map(|e| e.entity_revision),
        "gap" => s.gaps.get(id).map(|g| g.entity_revision),
        _ => None,
    }
}
pub(crate) fn required<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| Error::new("SCHEMA_INVALID", format!("Missing string {key}")))
}
fn writable(n: &Node) -> Result<()> {
    if n.protected || ["Evidence", "AnalysisFinding"].contains(&n.kind.as_str()) {
        Err(Error::new("PROTECTED_ENTITY", &n.id))
    } else {
        Ok(())
    }
}
fn keys(v: &Value, allowed: &[&str]) -> Result<()> {
    for key in v
        .as_object()
        .ok_or_else(|| Error::new("SCHEMA_INVALID", "Expected object"))?
        .keys()
    {
        if !allowed.contains(&key.as_str()) {
            return Err(Error::new(
                "SCHEMA_INVALID",
                format!("Unknown operation field {key}"),
            ));
        }
    }
    Ok(())
}
fn resolve_local(v: &mut Value, map: &BTreeMap<String, String>) {
    match v {
        Value::String(s) => {
            if let Some(id) = map.get(s) {
                *s = id.clone();
            }
        }
        Value::Array(xs) => {
            for x in xs {
                resolve_local(x, map)
            }
        }
        Value::Object(xs) => {
            for x in xs.values_mut() {
                resolve_local(x, map)
            }
        }
        _ => (),
    }
}
fn set_path(v: &mut Value, path: &str, value: Option<Value>) -> Result<()> {
    if !path.starts_with('/') || path == "/" {
        return Err(Error::new(
            "SCHEMA_INVALID",
            "Path must select a spec member",
        ));
    }
    let parts: Vec<_> = path[1..]
        .split('/')
        .map(|s| s.replace("~1", "/").replace("~0", "~"))
        .collect();
    let mut cur = v;
    for part in &parts[..parts.len() - 1] {
        let map = cur
            .as_object_mut()
            .ok_or_else(|| Error::new("SCHEMA_INVALID", "Path parent is not an object"))?;
        cur = map.entry(part.clone()).or_insert(json!({}));
    }
    let map = cur
        .as_object_mut()
        .ok_or_else(|| Error::new("SCHEMA_INVALID", "Path parent is not an object"))?;
    let key = parts.last().unwrap();
    if let Some(value) = value {
        map.insert(key.clone(), value);
    } else {
        map.remove(key);
    }
    Ok(())
}

pub(crate) fn derive_edges(state: &mut State) -> Result<()> {
    let old = state.edges.clone();
    for e in state
        .edges
        .values_mut()
        .filter(|e| e.authority == "derived")
    {
        e.deleted = true;
    }
    fn walk(owner: &Node, v: &Value, path: &str, out: &mut Vec<(String, String, String)>) {
        match v {
            Value::Object(xs) => {
                for (k, v) in xs {
                    let kind = match k.as_str() {
                        "function_id" => Some(if path.starts_with("/on_entry") {
                            "ON_ENTRY"
                        } else if path.starts_with("/on_exit") {
                            "ON_EXIT"
                        } else {
                            "CALLS"
                        }),
                        "node_id" => Some("USES_TYPE"),
                        "member_ids" | "state_ids" | "transition_ids" | "module_ids" => {
                            Some("CONTAINS")
                        }
                        "contract_ids" | "policy_ids" | "owner_policy_id" | "actor_policy_id" => {
                            Some("CONSTRAINED_BY")
                        }
                        "approval_policy_id" => Some("REQUIRES_APPROVAL"),
                        "entry_function_id" => Some("STARTS"),
                        "entity_id" => Some("DISPLAYS"),
                        _ => None,
                    };
                    if let Some(kind) = kind {
                        if let Some(s) = v.as_str() {
                            out.push((s.into(), kind.into(), format!("{path}/{k}")));
                        } else if let Some(xs) = v.as_array() {
                            for (i, s) in xs.iter().filter_map(Value::as_str).enumerate() {
                                out.push((s.into(), kind.into(), format!("{path}/{k}/{i}")));
                            }
                        }
                    }
                    walk(owner, v, &format!("{path}/{k}"), out);
                }
            }
            Value::Array(xs) => {
                for (i, v) in xs.iter().enumerate() {
                    walk(owner, v, &format!("{path}/{i}"), out);
                }
            }
            _ => (),
        }
    }
    let nodes: Vec<_> = state
        .nodes
        .values()
        .filter(|n| !n.deleted)
        .cloned()
        .collect();
    for n in &nodes {
        let mut refs = vec![];
        walk(n, &n.spec, "", &mut refs);
        for (dst, kind, role) in refs {
            let key = format!(
                "e_{}",
                hash::structured("derived-edge", &json!([n.id, kind, role]))?
            );
            let prev = old.get(&key);
            state.edges.insert(
                key.clone(),
                Edge {
                    id: key,
                    src: n.id.clone(),
                    dst,
                    kind,
                    role: Some(role),
                    payload: json!({}),
                    authority: "derived".into(),
                    entity_revision: prev.map_or(0, |e| e.entity_revision),
                    deleted: false,
                    provenance_ids: prev.map_or_else(Vec::new, |e| e.provenance_ids.clone()),
                },
            );
        }
        if n.kind == "WorkflowTransition" {
            if let (Some(src), Some(dst)) = (
                n.spec["from_state_id"].as_str(),
                n.spec["to_state_id"].as_str(),
            ) {
                let key = format!("e_{}", hash::structured("transition-edge", &json!(n.id))?);
                let prev = old.get(&key);
                state.edges.insert(
                    key.clone(),
                    Edge {
                        id: key,
                        src: src.into(),
                        dst: dst.into(),
                        kind: "TRANSITIONS_TO".into(),
                        role: Some(n.id.clone()),
                        payload: json!({"transition_id":n.id,"event":n.spec["event"]}),
                        authority: "derived".into(),
                        entity_revision: prev.map_or(0, |e| e.entity_revision),
                        deleted: false,
                        provenance_ids: prev.map_or_else(Vec::new, |e| e.provenance_ids.clone()),
                    },
                );
            }
        }
    }
    Ok(())
}
