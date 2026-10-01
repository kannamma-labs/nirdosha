//! One registry drives both wire JSON Schema and validation. Unknown keys fail.
use crate::{Error, Result, hash};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const GRAPH_SCHEMA: &str = "nirdosha.hi.graph/v2";
pub const PATCH_SCHEMA: &str = "nirdosha.hi.patch/v1";
pub const KINDS: &[&str] = &[
    "Project",
    "Module",
    "Struct",
    "Enum",
    "Function",
    "Contract",
    "Policy",
    "Role",
    "Claim",
    "Screen",
    "Workflow",
    "WorkflowState",
    "WorkflowTransition",
    "ApprovalPolicy",
    "Application",
    "Plan",
    "Requirement",
    "Decision",
    "Document",
    "Chunk",
    "Test",
    "ExternalSymbol",
    "Evidence",
    "AnalysisFinding",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Symbol {
    pub package_id: String,
    pub module_path: Vec<String>,
    pub namespace: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub symbol: Option<Symbol>,
    pub entity_revision: u64,
    pub deleted: bool,
    pub origin: String,
    pub spec_schema_version: String,
    pub spec: Value,
    pub provenance_ids: Vec<String>,
    pub source_refs: Vec<Value>,
    pub protected: bool,
    /// RFC 0021 §5.3 "Observed source": parsed declarations, source
    /// hashes and spans, recorded via the `source.observe` patch
    /// operation. `None` until a source sync ever observes this node.
    /// Deliberately loose `Value`, same convention `spec`/`payload`
    /// already use — not a `TypedSpec` kind, not schema-registry
    /// validated, just a `{source_ref, declaration,
    /// observed_at_revision}` record. Can disagree with `spec`; the
    /// service never reconciles the two automatically (RFC's own
    /// words: "observations can disagree with the accepted
    /// specification").
    pub observation: Option<Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Edge {
    pub id: String,
    pub src: String,
    pub dst: String,
    pub kind: String,
    pub role: Option<String>,
    pub payload: Value,
    pub authority: String,
    pub entity_revision: u64,
    pub deleted: bool,
    pub provenance_ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Gap {
    pub id: String,
    pub node_id: String,
    pub member_id: Option<String>,
    pub path_hint: String,
    pub category: String,
    pub origin: String,
    pub status: String,
    pub blocking: bool,
    pub reason: String,
    pub entity_revision: u64,
    pub evidence_refs: Vec<Value>,
    pub resolution: Option<Value>,
    pub supersedes: Option<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    pub nodes: BTreeMap<String, Node>,
    pub edges: BTreeMap<String, Edge>,
    pub gaps: BTreeMap<String, Gap>,
}

pub fn spec_version(kind: &str) -> String {
    format!("nirdosha.hi.spec/{kind}/v1")
}
fn text() -> Value {
    json!({"type":"string"})
}
fn boolean() -> Value {
    json!({"type":"boolean"})
}
fn integer() -> Value {
    json!({"type":"integer","minimum":0,"maximum":hash::MAX_INTEGER})
}
fn enumeration(xs: &[&str]) -> Value {
    json!({"type":"string","enum":xs})
}
fn list(shape: Value) -> Value {
    json!({"type":"array","items":shape})
}
fn map(shape: Value) -> Value {
    json!({"type":"object","additionalProperties":shape})
}
fn nullable(shape: Value) -> Value {
    json!({"anyOf":[{"type":"null"},shape]})
}
fn object(fields: &[(&str, Value)], required: &[&str]) -> Value {
    json!({"type":"object","properties":fields.iter().map(|(k,v)|(k.to_string(),v.clone())).collect::<serde_json::Map<_,_>>(),"required":required,"additionalProperties":false})
}
fn reference(name: &str) -> Value {
    json!({"$ref":format!("#/$defs/{name}")})
}
fn tag(name: &str, fields: &[(&str, Value)], required: &[&str]) -> Value {
    let mut fs = vec![("tag", json!({"const":name}))];
    fs.extend_from_slice(fields);
    let mut req = vec!["tag"];
    req.extend_from_slice(required);
    object(&fs, &req)
}

pub fn definitions() -> Value {
    let ty = json!({"anyOf":[
        tag("primitive",&[("name",enumeration(&["bool","i8","i16","i32","i64","i128","u8","u16","u32","u64","u128","usize","isize","f32","f64","char","String","str","unit"]))],&["name"]),
        tag("named",&[("node_id",text()),("type_args",list(reference("TypeRef")))],&["node_id"]),
        tag("generic",&[("name",text())],&["name"]),
        tag("tuple",&[("items",list(reference("TypeRef")))],&["items"]),
        tag("array",&[("inner",reference("TypeRef")),("length",integer())],&["inner","length"]),
        tag("reference",&[("inner",reference("TypeRef")),("mutable",boolean()),("lifetime",text())],&["inner","mutable"]),
        tag("function",&[("parameters",list(reference("TypeRef"))),("result",reference("TypeRef"))],&["parameters","result"])
    ]});
    let expr = json!({"anyOf":[
        tag("literal",&[("value",json!({"type":["string","number","boolean","null"]}))],&["value"]),
        tag("ref",&[("binding",text())],&["binding"]),
        tag("field",&[("base",reference("Expression")),("field",text())],&["base","field"]),
        tag("binary",&[("op",enumeration(&["+","-","*","/","%","==","!=","<",">","<=",">=","&&","||"])),("left",reference("Expression")),("right",reference("Expression"))],&["op","left","right"]),
        tag("not",&[("inner",reference("Expression"))],&["inner"]),
        tag("call",&[("function_id",text()),("arguments",map(reference("Expression")))],&["function_id","arguments"])
    ]});
    let field = object(
        &[
            ("name", text()),
            ("type", nullable(reference("TypeRef"))),
            ("visibility", enumeration(&["private", "pub", "pub(crate)"])),
            ("default", reference("Expression")),
            ("policy_ids", list(text())),
        ],
        &["name", "type"],
    );
    let hook = object(
        &[
            ("id", text()),
            ("function_id", text()),
            ("arguments", map(reference("Expression"))),
        ],
        &["id", "function_id", "arguments"],
    );
    let body = json!({"anyOf":[tag("missing",&[],&[]),tag("source",&[("blob_hash",text()),("dialect",json!({"const":"nirdosha-v2"})),("signature_hash",text()),("dependency_hash",text())],&["blob_hash","dialect","signature_hash","dependency_hash"]),tag("plan",&[("plan_id",text())],&["plan_id"]),tag("template",&[("template_id",text()),("version",integer()),("bindings",map(reference("Expression")))],&["template_id","version","bindings"])]});
    json!({"TypeRef":ty,"Expression":expr,"Field":field,"Hook":hook,"BodySlot":body})
}

fn build_spec_schema(kind: &str) -> Result<Value> {
    let fields = match kind {
        "Project" => vec![
            ("dialect", json!({"const":"nirdosha-v2"})),
            ("module_ids", list(text())),
            ("application_id", nullable(text())),
            ("toolchain", text()),
            ("required_checks", list(text())),
        ],
        "Module" => vec![
            ("parent_id", nullable(text())),
            ("member_ids", list(text())),
            ("path", text()),
            ("visibility", enumeration(&["private", "pub", "pub(crate)"])),
        ],
        "Struct" => vec![
            ("fields", map(reference("Field"))),
            ("field_order", list(text())),
            ("generics", list(text())),
            ("visibility", enumeration(&["private", "pub", "pub(crate)"])),
        ],
        "Enum" => vec![
            (
                "variants",
                map(object(
                    &[
                        ("name", text()),
                        ("fields", map(reference("Field"))),
                        ("field_order", list(text())),
                        ("style", enumeration(&["unit", "tuple", "named"])),
                    ],
                    &["name", "style"],
                )),
            ),
            ("variant_order", list(text())),
            ("generics", list(text())),
            ("visibility", enumeration(&["private", "pub", "pub(crate)"])),
        ],
        "Function" => vec![
            ("parameters", map(reference("Field"))),
            ("parameter_order", list(text())),
            ("return_type", nullable(reference("TypeRef"))),
            ("generics", list(text())),
            ("async", boolean()),
            ("visibility", enumeration(&["private", "pub", "pub(crate)"])),
            ("effects", list(text())),
            ("contract_ids", list(text())),
            ("policy_ids", list(text())),
            ("body", reference("BodySlot")),
            ("entry_point", boolean()),
        ],
        "Contract" => vec![
            ("function_id", text()),
            ("pre", list(reference("Expression"))),
            ("post", list(reference("Expression"))),
            ("invariants", list(reference("Expression"))),
            ("required_checker", text()),
            ("description", text()),
        ],
        "Policy" => vec![
            ("role_ids", list(text())),
            ("claim_ids", list(text())),
            ("nfr", map(integer())),
            ("description", text()),
        ],
        "Role" | "Claim" => vec![
            ("name", text()),
            ("description", text()),
            ("implies_ids", list(text())),
        ],
        "Workflow" => vec![
            ("data_type", reference("TypeRef")),
            ("initial_state_id", nullable(text())),
            ("state_ids", list(text())),
            ("transition_ids", list(text())),
            ("invariants", list(reference("Expression"))),
            ("required_runtime_capabilities", list(text())),
        ],
        "WorkflowState" => vec![
            ("workflow_id", text()),
            ("entry_conditions", list(reference("Expression"))),
            ("exit_conditions", list(reference("Expression"))),
            ("invariants", list(reference("Expression"))),
            ("on_entry", list(reference("Hook"))),
            ("on_exit", list(reference("Hook"))),
            ("owner_policy_id", nullable(text())),
            ("approval_policy_id", nullable(text())),
            (
                "terminal",
                enumeration(&["none", "success", "rejection", "cancellation"]),
            ),
        ],
        "WorkflowTransition" => vec![
            ("workflow_id", text()),
            ("from_state_id", text()),
            ("to_state_id", text()),
            ("event", text()),
            ("actor_policy_id", nullable(text())),
            ("guard", list(reference("Expression"))),
            ("approval_policy_id", nullable(text())),
            ("update_plan_id", nullable(text())),
            ("reenter", boolean()),
            ("reopen", boolean()),
            ("timeout_ms", integer()),
        ],
        "ApprovalPolicy" => vec![
            (
                "identity_basis",
                enumeration(&["distinct_person", "distinct_subject"]),
            ),
            ("maker_counts", boolean()),
            ("exclude_maker", boolean()),
            ("cross_stage_distinct", boolean()),
            (
                "stages",
                list(object(
                    &[
                        ("id", text()),
                        ("quorum", integer()),
                        (
                            "slots",
                            map(object(
                                &[("role_ids", list(text())), ("claim_ids", list(text()))],
                                &[],
                            )),
                        ),
                        ("mode", enumeration(&["parallel", "sequential"])),
                    ],
                    &["id", "quorum", "slots", "mode"],
                )),
            ),
            ("decision_lifetime_ms", integer()),
            ("rejection", enumeration(&["reject", "rework", "collect"])),
            ("invalidate_on_change", boolean()),
        ],
        "Application" => vec![
            ("entry_function_id", text()),
            ("startup_plan_id", nullable(text())),
            ("workflow_ids", list(text())),
            ("handler_ids", list(text())),
        ],
        "Screen" => vec![
            ("entity_id", text()),
            ("archetype", text()),
            (
                "fields",
                map(object(
                    &[("field_id", text()), ("label", text())],
                    &["field_id"],
                )),
            ),
            ("actions", list(reference("Hook"))),
            ("policy_ids", list(text())),
        ],
        "Plan" => vec![
            ("steps", list(reference("Expression"))),
            ("result", reference("Expression")),
        ],
        "Requirement" | "Decision" => vec![
            ("text", text()),
            ("source_ids", list(text())),
            ("supersedes_id", nullable(text())),
            ("resolved", boolean()),
        ],
        "Document" => vec![
            ("blob_hash", text()),
            ("source_path", text()),
            ("previous_document_id", nullable(text())),
        ],
        "Chunk" => vec![
            ("document_id", text()),
            ("blob_hash", text()),
            ("ordinal", integer()),
            ("span_start", integer()),
            ("span_end", integer()),
        ],
        "ExternalSymbol" => vec![
            ("path", text()),
            ("package", text()),
            ("version", text()),
            ("parameters", map(reference("Field"))),
            ("parameter_order", list(text())),
            ("return_type", reference("TypeRef")),
            ("type_symbol", boolean()),
        ],
        "Test" => vec![
            ("target_ids", list(text())),
            ("body", reference("BodySlot")),
            ("expected", reference("Expression")),
        ],
        "Evidence" | "AnalysisFinding" => vec![
            ("rule_id", text()),
            (
                "classification",
                enumeration(&[
                    "proven_violation",
                    "proven_satisfied",
                    "potential_issue",
                    "unknown",
                ]),
            ),
            ("target_ids", list(text())),
            ("input_revision", integer()),
            ("checker_id", text()),
            ("checker_version", text()),
            ("scope", text()),
            ("message", text()),
            ("evidence_hash", text()),
            ("coverage", object(&[("modeled_constructs",list(text())),("omitted_constructs",list(text())),("assumptions",list(text()))],&["modeled_constructs","omitted_constructs","assumptions"])),
        ],
        _ => {
            return Err(Error::new(
                "SCHEMA_INVALID",
                format!("Unknown node kind {kind}"),
            ));
        }
    };
    let mut s = object(&fields, &[]);
    s["$defs"] = definitions();
    s["$id"] = spec_version(kind).into();
    s["$schema"] = "https://json-schema.org/draft/2020-12/schema".into();
    Ok(s)
}

fn schema_ref(kind: &str) -> Result<&'static Value> {
    static REGISTRY: std::sync::OnceLock<BTreeMap<String, Value>> = std::sync::OnceLock::new();
    REGISTRY
        .get_or_init(|| {
            KINDS
                .iter()
                .map(|kind| {
                    (
                        kind.to_string(),
                        build_spec_schema(kind).expect("registry kind"),
                    )
                })
                .collect()
        })
        .get(kind)
        .ok_or_else(|| Error::new("SCHEMA_INVALID", format!("Unknown kind {kind}")))
}
pub fn spec_schema(kind: &str) -> Result<Value> {
    Ok(schema_ref(kind)?.clone())
}

pub fn validate_spec(kind: &str, version: &str, spec: &Value) -> Result<()> {
    if version != spec_version(kind) {
        return Err(Error::new("SPEC_SCHEMA_UNSUPPORTED", version));
    }
    if serde_json::to_vec(spec)?.len() > 128 * 1024 {
        return Err(Error::new("LIMIT_EXCEEDED", "spec exceeds 128 KiB"));
    }
    let s = schema_ref(kind)?;
    validate(s, spec, s, "", 0)?;
    for (collection, order) in [
        ("fields", "field_order"),
        ("parameters", "parameter_order"),
        ("variants", "variant_order"),
    ] {
        if let Some(m) = spec.get(collection).and_then(Value::as_object) {
            if let Some(xs) = spec.get(order).and_then(Value::as_array) {
                let ids: BTreeSet<_> = xs.iter().filter_map(Value::as_str).collect();
                if ids.len() != xs.len() || ids != m.keys().map(String::as_str).collect() {
                    return Err(Error::new(
                        "SCHEMA_INVALID",
                        format!("{order} must list every {collection} member exactly once"),
                    ));
                }
            }
        }
    }
    if kind == "ApprovalPolicy" {
        if spec.get("exclude_maker") == Some(&json!(false)) {
            return Err(Error::new(
                "SCHEMA_INVALID",
                "Multi-person approval excludes its maker from reviewer slots",
            ));
        }
        if let Some(stages) = spec["stages"].as_array() {
            let mut ids = BTreeSet::new();
            for stage in stages {
                let q = stage["quorum"].as_u64().unwrap_or(0);
                let slots = stage["slots"].as_object().map_or(0, |x| x.len());
                if q == 0 || q > slots as u64 || !ids.insert(stage["id"].as_str()) {
                    return Err(Error::new(
                        "SCHEMA_INVALID",
                        "Stage quorum/identity is invalid",
                    ));
                }
            }
        }
    }
    Ok(())
}

fn validate(s: &Value, v: &Value, root: &Value, path: &str, depth: usize) -> Result<()> {
    let invalid = || {
        Error::new("SCHEMA_INVALID", format!("Invalid value at {path}"))
            .details(json!({"path":path}))
    };
    if depth > 64 {
        return Err(Error::new("LIMIT_EXCEEDED", "spec nesting exceeds 64"));
    }
    if let Some(r) = s["$ref"].as_str() {
        return validate(
            root.pointer(&r[1..]).ok_or_else(invalid)?,
            v,
            root,
            path,
            depth + 1,
        );
    }
    if let Some(xs) = s["anyOf"].as_array() {
        return if xs
            .iter()
            .any(|s| validate(s, v, root, path, depth + 1).is_ok())
        {
            Ok(())
        } else {
            Err(invalid())
        };
    }
    if let Some(c) = s.get("const") {
        if c != v {
            return Err(invalid());
        }
    }
    if let Some(xs) = s["enum"].as_array() {
        if !xs.contains(v) {
            return Err(invalid());
        }
    }
    let types = match &s["type"] {
        Value::String(s) => vec![s.as_str()],
        Value::Array(xs) => xs.iter().filter_map(Value::as_str).collect(),
        _ => vec![],
    };
    if !types.is_empty()
        && !types.iter().any(|t| match *t {
            "object" => v.is_object(),
            "array" => v.is_array(),
            "string" => v.is_string(),
            "integer" => v.as_u64().is_some(),
            "number" => v.is_number(),
            "boolean" => v.is_boolean(),
            "null" => v.is_null(),
            _ => false,
        })
    {
        return Err(invalid());
    }
    if let Some(n) = v.as_u64() {
        if s["minimum"].as_u64().is_some_and(|x| n < x)
            || s["maximum"].as_u64().is_some_and(|x| n > x)
        {
            return Err(invalid());
        }
    }
    if let Some(o) = v.as_object() {
        if let Some(req) = s["required"].as_array() {
            for k in req {
                if !o.contains_key(k.as_str().unwrap_or("")) {
                    return Err(invalid());
                }
            }
        }
        for (k, v) in o {
            let sub = s["properties"]
                .get(k)
                .or_else(|| s.get("additionalProperties").filter(|x| x.is_object()));
            if let Some(sub) = sub {
                validate(sub, v, root, &format!("{path}/{k}"), depth + 1)?;
            } else if s["additionalProperties"] == false {
                return Err(Error::new(
                    "SCHEMA_INVALID",
                    format!("Unknown field {path}/{k}"),
                ));
            }
        }
    }
    if let Some(xs) = v.as_array() {
        if let Some(sub) = s.get("items") {
            for (i, v) in xs.iter().enumerate() {
                validate(sub, v, root, &format!("{path}/{i}"), depth + 1)?;
            }
        }
    }
    Ok(())
}

/// Semantic references from typed fields, never arbitrary prose/source strings.
pub fn references(spec: &Value) -> BTreeSet<String> {
    fn walk(v: &Value, out: &mut BTreeSet<String>) {
        match v {
            Value::Object(xs) => {
                for (k, v) in xs {
                    if matches!(
                        k.as_str(),
                        "node_id"
                            | "function_id"
                            | "plan_id"
                            | "parent_id"
                            | "application_id"
                            | "workflow_id"
                            | "initial_state_id"
                            | "from_state_id"
                            | "to_state_id"
                            | "owner_policy_id"
                            | "approval_policy_id"
                            | "actor_policy_id"
                            | "update_plan_id"
                            | "entry_function_id"
                            | "startup_plan_id"
                            | "entity_id"
                            | "document_id"
                            | "previous_document_id"
                            | "supersedes_id"
                    ) {
                        if let Some(s) = v.as_str() {
                            out.insert(s.into());
                        }
                    } else if matches!(
                        k.as_str(),
                        "module_ids"
                            | "member_ids"
                            | "contract_ids"
                            | "policy_ids"
                            | "role_ids"
                            | "claim_ids"
                            | "implies_ids"
                            | "state_ids"
                            | "transition_ids"
                            | "workflow_ids"
                            | "handler_ids"
                            | "source_ids"
                            | "target_ids"
                    ) {
                        if let Some(xs) = v.as_array() {
                            for s in xs.iter().filter_map(Value::as_str) {
                                out.insert(s.into());
                            }
                        }
                    }
                    walk(v, out);
                }
            }
            Value::Array(xs) => {
                for x in xs {
                    walk(x, out);
                }
            }
            _ => (),
        }
    }
    let mut out = BTreeSet::new();
    walk(spec, &mut out);
    out
}

pub fn validate_state(state: &State) -> Result<()> {
    let mut symbols = BTreeSet::new();
    for n in state.nodes.values().filter(|n| !n.deleted) {
        validate_spec(&n.kind, &n.spec_schema_version, &n.spec)?;
        if let Some(s) = &n.symbol {
            if !["type", "value", "macro"].contains(&s.namespace.as_str())
                || syn::parse_str::<syn::Ident>(&s.name).is_err()
            {
                return Err(Error::new("SCHEMA_INVALID", "Invalid symbol"));
            }
            if !symbols.insert(
                serde_jcs::to_string(s).map_err(|e| Error::new("SCHEMA_INVALID", e.to_string()))?,
            ) {
                return Err(Error::new("SCHEMA_INVALID", "Duplicate scoped symbol"));
            }
        }
        for r in if ["AnalysisFinding","Evidence"].contains(&n.kind.as_str()) { BTreeSet::new() } else { references(&n.spec) } {
            if !state.nodes.get(&r).is_some_and(|n| !n.deleted) {
                return Err(Error::new(
                    "UNRESOLVED_REFERENCE",
                    format!("{} references {r}", n.id),
                ));
            }
        }
    }
    crate::workflow::validate(state)?;
    for e in state.edges.values().filter(|e| !e.deleted) {
        for r in [&e.src, &e.dst] {
            if !state.nodes.get(r).is_some_and(|n| !n.deleted) {
                return Err(Error::new(
                    "UNRESOLVED_REFERENCE",
                    format!("Edge {} references {r}", e.id),
                ));
            }
        }
    }
    for g in state
        .gaps
        .values()
        .filter(|g| g.status == "open" || g.status == "resolved")
    {
        let n = state
            .nodes
            .get(&g.node_id)
            .filter(|n| !n.deleted)
            .ok_or_else(|| Error::new("GAP_TARGET_INVALID", &g.id))?;
        if !g.path_hint.is_empty() && n.spec.pointer(&g.path_hint).is_none() {
            return Err(Error::new("GAP_TARGET_INVALID", &g.id));
        }
    }
    Ok(())
}
