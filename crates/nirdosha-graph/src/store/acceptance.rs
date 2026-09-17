use super::mutations::{Precondition, derive_edges, entity_revision};
use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Accept {
    pub mutation_id: String,
    pub base_acceptance_id: Option<String>,
    pub selections: Vec<Precondition>,
    #[serde(default)]
    pub removals: Vec<Removal>,
    pub dependency_pins: Vec<Pin>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Removal {
    pub entity_type: String,
    pub id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pin {
    pub id: String,
    pub entity_revision: u64,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Manifest {
    pub state: State,
    pub dependency_pins: BTreeMap<String, BTreeMap<String, u64>>,
}

impl Graph {
    pub(crate) fn accepted_manifest(&self, key: &str) -> Result<Manifest> {
        let s: Option<String> = self
            .conn
            .query_row(
                "SELECT manifest FROM graph_acceptances WHERE id=?1",
                [key],
                |r| r.get(0),
            )
            .optional()?;
        Ok(serde_json::from_str(&s.ok_or_else(|| {
            Error::new("NOT_FOUND", "Acceptance manifest not found")
        })?)?)
    }
    pub(crate) fn accepted_state_at(&self, revision: u64) -> Result<State> {
        let s:Option<String>=self.conn.query_row("SELECT manifest FROM graph_acceptances WHERE revision<=?1 ORDER BY revision DESC LIMIT 1",[revision],|r|r.get(0)).optional()?;
        Ok(if let Some(s) = s {
            serde_json::from_str::<Manifest>(&s)?.state
        } else {
            State::default()
        })
    }
    fn acceptance_candidate(&self, a: &Accept) -> Result<Manifest> {
        let head: Option<String> = self.conn.query_row(
            "SELECT accepted_id FROM graph_meta WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        if a.base_acceptance_id != head {
            return Err(Error::new("REVISION_CONFLICT", "Accepted head changed")
                .details(json!({"current_acceptance_id":head})));
        }
        let mut m = if let Some(key) = &head {
            self.accepted_manifest(key)?
        } else {
            Manifest::default()
        };
        let proposals = self.load_state(None)?;
        let selected: BTreeSet<_> = a
            .selections
            .iter()
            .map(|s| (s.entity_type.clone(), s.id.clone()))
            .collect();
        if selected.len() != a.selections.len() {
            return Err(Error::new(
                "SCHEMA_INVALID",
                "Repeated acceptance selection",
            ));
        }
        for r in &a.removals {
            match r.entity_type.as_str() {
                "node" => {
                    m.state.nodes.remove(&r.id);
                    m.dependency_pins.remove(&r.id);
                }
                "edge" => {
                    m.state.edges.remove(&r.id);
                }
                "gap" => {
                    m.state.gaps.remove(&r.id);
                }
                _ => return Err(Error::new("SCHEMA_INVALID", "Invalid removal kind")),
            }
        }
        for sel in &a.selections {
            if entity_revision(&proposals, &sel.entity_type, &sel.id) != Some(sel.entity_revision) {
                return Err(Error::new(
                    "REVISION_CONFLICT",
                    format!("Selection changed: {}", sel.id),
                ));
            }
            match sel.entity_type.as_str() {
                "node" => {
                    let n = proposals.nodes.get(&sel.id).unwrap();
                    if n.deleted {
                        return Err(Error::new(
                            "ACCEPTANCE_INVALID",
                            "Cannot select a tombstone",
                        ));
                    }
                    m.state.nodes.insert(n.id.clone(), n.clone());
                }
                "edge" => {
                    let e = proposals.edges.get(&sel.id).unwrap();
                    if e.authority == "derived" || e.deleted {
                        return Err(Error::new(
                            "ACCEPTANCE_INVALID",
                            "Select owner specs, not derived/deleted edges",
                        ));
                    }
                    m.state.edges.insert(e.id.clone(), e.clone());
                }
                "gap" => {
                    let g = proposals.gaps.get(&sel.id).unwrap();
                    m.state.gaps.insert(g.id.clone(), g.clone());
                }
                _ => {
                    return Err(Error::new(
                        "SCHEMA_INVALID",
                        "Unknown selection entity type",
                    ));
                }
            }
        }
        let pins: BTreeMap<_, _> = a
            .dependency_pins
            .iter()
            .map(|p| (p.id.clone(), p.entity_revision))
            .collect();
        if pins.len() != a.dependency_pins.len() {
            return Err(Error::new("SCHEMA_INVALID", "Duplicate dependency pin"));
        }
        for n in m.state.nodes.values() {
            let refs = schema::references(&n.spec);
            let mut newpins = BTreeMap::new();
            for r in refs {
                let expected = if selected.contains(&("node".into(), n.id.clone())) {
                    pins.get(&r).copied()
                } else {
                    m.dependency_pins
                        .get(&n.id)
                        .and_then(|p| p.get(&r))
                        .copied()
                };
                let actual = m.state.nodes.get(&r).map(|n| n.entity_revision);
                if expected.is_none() || actual != expected {
                    return Err(Error::new(
                        "ACCEPTANCE_DEPENDENCY_MISSING",
                        format!("{} requires exact selection/pin for {r}", n.id),
                    )
                    .details(
                        json!({"owner":n.id,"dependency":r,"expected":expected,"selected":actual}),
                    ));
                }
                newpins.insert(r, expected.unwrap());
            }
            if selected.contains(&("node".into(), n.id.clone())) {
                m.dependency_pins.insert(n.id.clone(), newpins);
            }
            for g in proposals.gaps.values().filter(|g| g.node_id == n.id) {
                if selected.contains(&("node".into(), n.id.clone()))
                    && m.state.gaps.get(&g.id).map(|g| g.entity_revision) != Some(g.entity_revision)
                {
                    return Err(Error::new(
                        "ACCEPTANCE_DEPENDENCY_MISSING",
                        format!("Select or retain exact gap revision {}", g.id),
                    ));
                }
            }
        }
        m.state.edges.retain(|_, e| e.authority != "derived");
        derive_edges(&mut m.state)?;
        schema::validate_state(&m.state)
            .map_err(|e| Error::new("ACCEPTANCE_INVALID", e.to_string()))?;
        let issues = readiness(&m.state);
        if !issues.is_empty() {
            return Err(Error::new(
                "ACCEPTANCE_INVALID",
                "Specification has unresolved obligations",
            )
            .details(json!({"issues":issues})));
        }
        Ok(m)
    }
    pub fn accept(&self, args: &Value) -> Result<Value> {
        self.check_accept()?;
        self.transaction(||{
            self.check_accept()?;let a:Accept=serde_json::from_value(args.clone())?;let v=self.raw_version()?;
            let digest=hash::structured("acceptance",args)?;
            let old:Option<(String,String,String)>=self.conn.query_row("SELECT hash,binding,receipt FROM graph_receipts WHERE actor=?1 AND mutation_id=?2 AND epoch=?3",params![self.access.actor,a.mutation_id,v.epoch],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
            if let Some((h,b,r))=old{if b!="accept"{return Err(Error::new("MUTATION_BINDING_CONFLICT","Mutation already bound"));}if h!=digest{return Err(Error::new("IDEMPOTENCY_CONFLICT","Acceptance payload differs"));}return Ok(serde_json::from_str(&r)?);}
            let manifest=self.acceptance_candidate(&a)?;let revision=self.next_revision()?;let key=id("accept")?;
            self.conn.execute("INSERT INTO graph_acceptances VALUES(?1,?2,?3)",params![key,revision,serde_json::to_string(&manifest)?])?;
            self.conn.execute("UPDATE graph_meta SET accepted_id=?1,revision=?2",params![key,revision])?;
            let result=json!({"acceptance_id":key,"graph":self.raw_version()?,"mutation_id":a.mutation_id,"specification_ready":true,"body_slots":manifest.state.nodes.values().filter(|n|n.kind=="Function"&&(n.spec["body"].is_null()||n.spec["body"]["tag"]=="missing")).map(|n|n.id.clone()).collect::<Vec<_>>()});
            self.conn.execute("INSERT INTO graph_changes VALUES(?1,?2)",params![revision,serde_json::to_string(&json!({"revision":revision,"acceptance_id":key,"changes":[]}))?])?;
            self.conn.execute("INSERT INTO graph_receipts VALUES(?1,?2,?3,?4,'accept',?5)",params![self.access.actor,a.mutation_id,v.epoch,digest,serde_json::to_string(&result)?])?;
            Ok(result)
        })
    }
    pub fn validate(&self, args: &Value) -> Result<Value> {
        self.check_access(false)?;
        if args["mode"] == "acceptance_preview" {
            let a: Accept = serde_json::from_value(args["acceptance"].clone())?;
            self.acceptance_candidate(&a)?;
            return Ok(json!({"valid":true,"advisory":true}));
        }
        let (r, view) = if let Some(s) = args["snapshot"].as_str() {
            self.snapshot(s)?
        } else {
            (
                self.raw_version()?.revision,
                args["view"].as_str().unwrap_or("proposed").into(),
            )
        };
        let state = self.state_view(r, &view)?;
        schema::validate_state(&state)?;
        let issues = readiness(&state);
        let unsupported:Vec<_>=state.nodes.values().filter(|n|!n.deleted&&["Workflow","Screen"].contains(&n.kind.as_str())).map(|n|json!({"id":n.id,"code":"UNSUPPORTED_TARGET","reason":"Execution adapter must be explicitly installed"})).collect();
        let missing_bodies: Vec<_> = state
            .nodes
            .values()
            .filter(|n| {
                !n.deleted
                    && n.kind == "Function"
                    && (n.spec["body"].is_null() || n.spec["body"]["tag"] == "missing")
            })
            .map(|n| n.id.clone())
            .collect();
        self.check_access(false)?;
        Ok(
            json!({"revision":r,"specification_ready":issues.is_empty(),"emission_ready":issues.is_empty()&&unsupported.is_empty()&&missing_bodies.is_empty(),"issues":issues,"unsupported":unsupported,"missing_bodies":missing_bodies}),
        )
    }
}

pub(crate) fn readiness(s: &State) -> Vec<Value> {
    let mut issues = vec![];
    for g in s.gaps.values().filter(|g| g.blocking && g.status == "open") {
        issues.push(json!({"id":g.id,"node_id":g.node_id,"reason":g.reason}));
    }
    for n in s.nodes.values().filter(|n| !n.deleted) {
        let fields: &[&str] = match n.kind.as_str() {
            "Function" => &["parameters", "return_type"],
            "Struct" => &["fields"],
            "Enum" => &["variants"],
            "Workflow" => &["initial_state_id", "state_ids", "transition_ids"],
            "WorkflowState" => &["workflow_id", "terminal"],
            "WorkflowTransition" => &["workflow_id", "from_state_id", "to_state_id", "event"],
            "ApprovalPolicy" => &["identity_basis", "stages", "exclude_maker"],
            "Document" => &["blob_hash"],
            "Chunk" => &["document_id", "blob_hash", "ordinal"],
            _ => &[],
        };
        for k in fields {
            if n.spec.get(*k).is_none_or(Value::is_null) {
                issues.push(json!({"node_id":n.id,"path":format!("/{k}"),"reason":"required authoring information missing"}));
            }
        }
        fn holes(v: &Value, path: &str, out: &mut Vec<Value>, id: &str) {
            match v {
                Value::Object(xs) => {
                    for (k, v) in xs {
                        if k == "type" && v.is_null() {
                            out.push(json!({"node_id":id,"path":format!("{path}/{k}"),"reason":"unresolved field/parameter type"}));
                        }
                        holes(v, &format!("{path}/{k}"), out, id);
                    }
                }
                Value::Array(xs) => {
                    for (i, v) in xs.iter().enumerate() {
                        holes(v, &format!("{path}/{i}"), out, id);
                    }
                }
                _ => (),
            }
        }
        holes(&n.spec, "", &mut issues, &n.id);
    }
    issues
}
