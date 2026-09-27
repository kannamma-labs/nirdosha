//! RFC 0029 §7.1 catalog-to-IR adapter: the *only* place a screen's
//! declared `[screen.service]` parameters are lowered into the canonical
//! `rfc0029_conformance::Input` graph the admission engine evaluates. No
//! other part of the generator may build that graph — a screen's English
//! `notes`/`name` fields never reach it.

use std::collections::BTreeMap;
use std::fmt;

use rfc0029_conformance::{Bundle, Edge, EdgeKind, Input, Node, NodeKind};
use serde_json::Value;

use crate::service_catalog::ServiceEntry;

/// Fact identifiers this adapter knows how to resolve into a real graph
/// node from static screen metadata alone. A `required_facts` entry
/// outside this set has no live oracle wired at generation time (e.g.
/// `fresh_balance`/`daily_limit` need a real account gateway, not a
/// screens.toml declaration) and is rejected rather than stubbed out —
/// fail-closed, per RFC 0029 §7.1, not a silently-passed check.
const KNOWN_RESOLVABLE_FACTS: &[&str] = &["subject_identity"];

#[derive(Debug)]
pub enum AdapterError {
    UndeclaredParameter { service: String, parameter: String },
    ResourceMismatch { screen_id: String, service: String, expected: String, declared: String },
    NoSubjectRole { screen_id: String, service: String },
    UnresolvedFact { service: String, fact: String },
}

impl fmt::Display for AdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AdapterError::UndeclaredParameter { service, parameter } => write!(
                f,
                "service `{service}`: parameter `{parameter}` is not in this entry's allowed_parameters"
            ),
            AdapterError::ResourceMismatch { screen_id, service, expected, declared } => write!(
                f,
                "screen {screen_id}: data_binding entity `{declared}` does not match service `{service}`'s declared resource `{expected}`"
            ),
            AdapterError::NoSubjectRole { screen_id, service } => write!(
                f,
                "screen {screen_id}: service `{service}` requires the `subject_identity` fact, but the screen declares no role/eligible_roles to bind it to"
            ),
            AdapterError::UnresolvedFact { service, fact } => write!(
                f,
                "service `{service}` requires fact `{fact}`, which this generator has no static resolver for (KNOWN_RESOLVABLE_FACTS: {})",
                KNOWN_RESOLVABLE_FACTS.join(", ")
            ),
        }
    }
}

fn as_table(value: &toml::Value) -> BTreeMap<String, toml::Value> {
    match value {
        toml::Value::Table(t) => t.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        _ => BTreeMap::new(),
    }
}

fn toml_to_json(value: &toml::Value) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

/// Lower one screen's service reference to the canonical `Input` graph.
/// `screen_resource` is the screen's own `data_binding.entities.first()`
/// (if any), cross-checked against the catalog entry's declared
/// `resource` -- the two must agree, neither is derived from the other.
/// `screen_roles` supplies the subject identity the `subject_identity`
/// fact binds to.
pub fn build_input(
    screen_id: &str,
    service_name: &str,
    service_version: &str,
    service_parameters: &toml::Value,
    entry: &ServiceEntry,
    screen_resource: Option<&str>,
    screen_roles: &[String],
) -> Result<Input, AdapterError> {
    for key in as_table(service_parameters).keys() {
        if !entry.allowed_parameters.iter().any(|p| p == key) {
            return Err(AdapterError::UndeclaredParameter {
                service: service_name.to_string(),
                parameter: key.clone(),
            });
        }
    }

    if let Some(declared) = screen_resource {
        if declared != entry.resource {
            return Err(AdapterError::ResourceMismatch {
                screen_id: screen_id.to_string(),
                service: service_name.to_string(),
                expected: entry.resource.clone(),
                declared: declared.to_string(),
            });
        }
    }

    let subject_role = screen_roles
        .first()
        .map(|r| r.split(':').next().unwrap_or(r).to_string());

    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    let mut authority_attrs = BTreeMap::new();
    authority_attrs.insert("resource".to_string(), Value::String(entry.resource.clone()));
    authority_attrs.insert("effect".to_string(), Value::String(entry.effect.clone()));
    authority_attrs.insert(
        "policy_kind".to_string(),
        Value::String(format!("{:?}", entry.policy_kind)),
    );
    if let Some(role) = &subject_role {
        authority_attrs.insert("role".to_string(), Value::String(role.clone()));
    }
    nodes.push(Node { id: "authority".into(), kind: NodeKind::AuthorityAssertion, attributes: authority_attrs });

    for fact in &entry.required_facts {
        if !KNOWN_RESOLVABLE_FACTS.contains(&fact.as_str()) {
            return Err(AdapterError::UnresolvedFact {
                service: service_name.to_string(),
                fact: fact.clone(),
            });
        }
        if fact == "subject_identity" && subject_role.is_none() {
            return Err(AdapterError::NoSubjectRole {
                screen_id: screen_id.to_string(),
                service: service_name.to_string(),
            });
        }
        let id = format!("fact_{fact}");
        let mut attrs = BTreeMap::new();
        attrs.insert("name".to_string(), Value::String(fact.clone()));
        if let Some(role) = &subject_role {
            attrs.insert("subject".to_string(), Value::String(role.clone()));
        }
        nodes.push(Node { id: id.clone(), kind: NodeKind::Fact, attributes: attrs });
        edges.push(Edge { source: id, kind: EdgeKind::Satisfy, target: "authority".into() });
    }

    let mut capability_attrs = BTreeMap::new();
    capability_attrs.insert("effect_class".to_string(), Value::String(entry.effect.clone()));
    nodes.push(Node { id: "capability".into(), kind: NodeKind::Capability, attributes: capability_attrs });
    edges.push(Edge { source: "authority".into(), kind: EdgeKind::Authorize, target: "capability".into() });

    let mut effect_attrs = BTreeMap::new();
    effect_attrs.insert("effect_id".to_string(), Value::String(entry.effect.clone()));
    nodes.push(Node { id: "effect".into(), kind: NodeKind::Effect, attributes: effect_attrs });
    edges.push(Edge { source: "capability".into(), kind: EdgeKind::Issue, target: "effect".into() });

    let mut parameters: BTreeMap<String, Value> = BTreeMap::new();
    parameters.insert(
        "admission_policy".to_string(),
        serde_json::json!({ "effect_class": entry.effect, "maximum_model_level": "None" }),
    );
    for (key, value) in as_table(service_parameters) {
        parameters.insert(format!("screen_param.{key}"), toml_to_json(&value));
    }

    let bundle_id = format!("bundle:{screen_id}:{service_name}:v{service_version}");
    Ok(Input {
        bundle: Bundle { id: bundle_id.clone(), version: 1, hash: format!("symbolic:{bundle_id}") },
        nodes,
        edges,
        catalog_entries: Vec::new(),
        parameters,
    })
}
