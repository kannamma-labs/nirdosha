//! V2 declaration grammar and lexical cross-references. This establishes
//! declaration consistency, not runtime durability/authorization or proofs.
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use syn::{spanned::Spanned, visit::Visit, Item};

#[derive(Debug, Clone, Serialize)]
pub struct Declaration {
    pub kind: String,
    pub item: String,
    pub line: u32,
    pub payload: Value,
    pub evidence: &'static str,
}

#[derive(Debug, Clone)]
pub struct Issue { pub line: u32, pub message: String }

#[derive(Default)]
pub struct Declarations { pub items: Vec<Declaration>, pub issues: Vec<Issue> }

pub fn inspect(file: &syn::File) -> Declarations {
    let mut output = Declarations::default();
    inspect_scope(&file.items, "", &mut output);
    output
}

fn inspect_scope(items: &[Item], namespace: &str, output: &mut Declarations) {
    let functions: BTreeMap<String, &syn::ItemFn> = items.iter().filter_map(|item| match item {
        Item::Fn(f) => Some((f.sig.ident.to_string(), f)), _ => None,
    }).collect();
    let structs: BTreeMap<String, &syn::ItemStruct> = items.iter().filter_map(|item| match item {
        Item::Struct(s) => Some((s.ident.to_string(), s)), _ => None,
    }).collect();
    for item in items {
        let (name, attrs, structure, function) = match item {
            Item::Fn(f) => (f.sig.ident.to_string(), &f.attrs, None, Some(f)),
            Item::Struct(s) => (s.ident.to_string(), &s.attrs, Some(s), None),
            Item::Enum(e) => (e.ident.to_string(), &e.attrs, None, None),
            Item::Mod(m) => {
                if let Some((_, items)) = &m.content { inspect_scope(items, &format!("{namespace}{}::", m.ident), output); }
                (m.ident.to_string(), &m.attrs, None, None)
            }
            _ => continue,
        };
        let docs: Vec<(u32, String)> = attrs.iter().filter_map(|attr| {
            if !attr.path().is_ident("doc") { return None; }
            let syn::Meta::NameValue(value) = &attr.meta else { return None; };
            let syn::Expr::Lit(literal) = &value.value else { return None; };
            let syn::Lit::Str(text) = &literal.lit else { return None; };
            Some((attr.span().start().line as u32, text.value()))
        }).collect();
        let mut index = 0;
        let mut seen = BTreeSet::new();
        while index < docs.len() {
            let (line, doc) = &docs[index];
            index += 1;
            let Some(rest) = doc.trim().strip_prefix("nirdosha:") else { continue; };
            let split = rest.find(char::is_whitespace).unwrap_or(rest.len());
            let kind = &rest[..split];
            if kind == "contract" { continue; }
            let mut json = rest[split..].trim().to_string();
            // Consecutive doc lines can carry a multiline JSON object.
            while serde_json::from_str::<Value>(&json).is_err_and(|e| e.is_eof()) && index < docs.len() && !docs[index].1.trim().starts_with("nirdosha:") {
                json.push('\n'); json.push_str(docs[index].1.trim()); index += 1;
            }
            let result = (|| {
                if !seen.insert(kind) { return Err(format!("duplicate nirdosha:{kind} on {name}")); }
                let payload: Value = serde_json::from_str(&json).map_err(|e| format!("malformed nirdosha:{kind} JSON: {e}"))?;
                validate(kind, &name, &payload, structure, function, &structs, &functions)?;
                Ok(payload)
            })();
            match result {
                Ok(payload) => output.items.push(Declaration { kind: kind.into(), item: format!("{namespace}{name}"), line: *line, payload, evidence: "schema_and_lexical_references_only" }),
                Err(message) => output.issues.push(Issue { line: *line, message }),
            }
        }
    }
}

fn object<'a>(value: &'a Value, allowed: &[&str]) -> Result<&'a Map<String, Value>, String> {
    let object = value.as_object().ok_or("declaration must be a JSON object")?;
    if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("unknown declaration key `{key}`"));
    }
    Ok(object)
}
fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or_else(|| format!("`{key}` must be a nonempty string"))
}
fn optional_string(value: &Value, key: &str) -> Result<(), String> { if value.get(key).is_some() { string(value, key)?; } Ok(()) }
fn array<'a>(value: &'a Value, key: &str) -> Result<&'a [Value], String> {
    match value.get(key) { None => Ok(&[]), Some(v) => v.as_array().map(Vec::as_slice).ok_or_else(|| format!("`{key}` must be an array")) }
}
fn function_ref<'a>(functions: &'a BTreeMap<String, &syn::ItemFn>, name: &str) -> Result<&'a syn::ItemFn, String> {
    functions.get(name).copied().ok_or_else(|| format!("unknown or nonlocal function `{name}` in declaration"))
}
fn field_names(s: &syn::ItemStruct) -> BTreeSet<String> { s.fields.iter().filter_map(|f| f.ident.as_ref().map(ToString::to_string)).collect() }

fn validate(kind: &str, name: &str, value: &Value, structure: Option<&syn::ItemStruct>, function: Option<&syn::ItemFn>, structs: &BTreeMap<String, &syn::ItemStruct>, functions: &BTreeMap<String, &syn::ItemFn>) -> Result<(), String> {
    match kind {
        "screen" => {
            object(value, &["for", "title", "fields", "actions"])?;
            let structure = structure.ok_or("screen must decorate its data struct")?;
            if string(value, "for")? != name { return Err("screen `for` must name the decorated struct".into()); }
            optional_string(value, "title")?;
            let fields = field_names(structure);
            if let Some(overrides) = value.get("fields") {
                for (field, options) in overrides.as_object().ok_or("fields must be an object")? {
                    if !fields.contains(field) { return Err(format!("unknown screen field `{field}`")); }
                    object(options, &["label", "pattern", "min", "max", "render", "readonly", "format"])?;
                    for key in ["label", "pattern", "render", "format"] { optional_string(options, key)?; }
                    for key in ["min", "max"] { if options.get(key).is_some_and(|v| !v.is_number()) { return Err(format!("{key} must be numeric")); } }
                    if options.get("readonly").is_some_and(|v| !v.is_boolean()) { return Err("readonly must be boolean".into()); }
                }
            }
            for action in array(value, "actions")? { check_action(action, functions)?; }
        }
        "dashboard" => {
            object(value, &["tiles", "charts"])?;
            for (key, reference) in [("tiles", "stat"), ("charts", "chart")] {
                for metric in array(value, key)? {
                    object(metric, &["label", reference])?; string(metric, "label")?;
                    let f = function_ref(functions, string(metric, reference)?)?;
                    if !f.sig.inputs.is_empty() { return Err("dashboard functions must have no parameters".into()); }
                    if let syn::ReturnType::Default = f.sig.output { return Err("dashboard functions must return a value".into()); }
                }
            }
        }
        "workflow" => {
            object(value, &["name", "data", "states"])?; string(value, "name")?;
            let structure = structure.ok_or("workflow must decorate its data struct")?;
            if let Some(data) = value.get("data") {
                object(data, &["struct", "fields"])?;
                if string(data, "struct")? != name { return Err("workflow data must name the decorated struct".into()); }
                let fields = data.get("fields").and_then(Value::as_object).ok_or("workflow data fields must be an object")?;
                if fields.keys().cloned().collect::<BTreeSet<_>>() != field_names(structure) { return Err("workflow data fields differ from the Rust struct".into()); }
                for (field, ty) in fields {
                    let expected = ty.as_str().ok_or("workflow field type must be a string")?;
                    let actual = &structure.fields.iter().find(|f| f.ident.as_ref().is_some_and(|id| id == field)).unwrap().ty;
                    if let syn::Type::Path(path) = actual {
                        if !path.path.is_ident(expected) { return Err(format!("workflow field `{field}` type differs from declaration")); }
                    } else { return Err("workflow complex field type is unsupported".into()); }
                }
            }
            let states = value.get("states").and_then(Value::as_object).filter(|s| !s.is_empty()).ok_or("workflow needs nonempty states")?;
            for state in states.values() {
                object(state, &["terminal", "owner", "on_entry", "on_exit", "transitions"])?;
                optional_string(state, "owner")?;
                if state.get("terminal").is_some_and(|v| !v.is_boolean()) { return Err("terminal must be boolean".into()); }
                for hook in ["on_entry", "on_exit"] { if state.get(hook).is_some() { function_ref(functions, string(state, hook)?)?; } }
                if let Some(transitions) = state.get("transitions") {
                    let transitions = transitions.as_object().ok_or("transitions must be an object")?;
                    if state["terminal"] == true && !transitions.is_empty() { return Err("terminal state cannot have transitions".into()); }
                    for target in transitions.values() {
                        let target = target.as_str().ok_or("transition target must be a state name")?;
                        if !states.contains_key(target) { return Err(format!("unknown workflow target state `{target}`")); }
                    }
                }
            }
        }
        "transact" => {
            object(value, &["precheck", "network", "verify", "commit", "compensate", "log", "retry", "timeout"])?;
            let f = function.ok_or("transact must decorate a function")?;
            let mut calls = Calls::default(); calls.visit_block(&f.block);
            for slot in ["precheck", "network", "verify", "commit", "compensate", "log"] {
                if value.get(slot).is_some() || ["network", "verify", "commit"].contains(&slot) {
                    let callee = string(value, slot)?;
                    function_ref(functions, callee)?;
                    if !calls.0.contains(callee) { return Err(format!("transaction body never calls declared {slot} `{callee}`")); }
                }
            }
            // Presence is necessary, not sufficient. No claim of path/order
            // correctness or durability is emitted by this syntactic pass.
        }
        "validate" => {
            object(value, &["fn", "pre", "post"])?;
            let f = function.ok_or("validate must decorate a function")?;
            if string(value, "fn")? != name { return Err("validate must name its decorated function".into()); }
            let _ = f;
            for key in ["pre", "post"] {
                for expression in array(value, key)? {
                    let expression = expression.as_str().ok_or("predicate must be a string")?;
                    syn::parse_str::<syn::Expr>(expression).map_err(|e| format!("invalid predicate: {e}"))?;
                }
            }
        }
        "serve" => {
            object(value, &["requires_public", "routes"])?;
            function.ok_or("serve must decorate a function")?;
            if value.get("requires_public").is_some_and(|v| !v.is_boolean()) { return Err("requires_public must be boolean".into()); }
            for route in array(value, "routes")? { if !route.as_str().is_some_and(|s| s.starts_with('/')) { return Err("route must start with /".into()); } }
        }
        "visual" => { object(value, &["label", "render"])?; string(value,"label")?; string(value,"render")?; function.ok_or("visual must decorate a function")?; }
        "nav" => { object(value, &["module"])?; string(value, "module")?; }
        "schema" => { object(value, &["auto_migrate"])?; structure.ok_or("schema must decorate a struct")?; if !value["auto_migrate"].is_boolean() { return Err("auto_migrate must be boolean".into()); } }
        "role_mapping" => { object(value, &[])?; let s = structure.ok_or("role_mapping must decorate a struct")?; if field_names(s) != ["id", "app_role", "idp_role"].into_iter().map(str::to_owned).collect() { return Err("role_mapping fields must be id, app_role, idp_role".into()); } }
        "field" => {
            object(value, &["struct", "field", "requires"])?;
            let s = structs.get(string(value, "struct")?).ok_or("unknown field-gated struct")?;
            if !field_names(s).contains(string(value,"field")?) { return Err("unknown gated field".into()); }
            object(&value["requires"], &["role"])?; string(&value["requires"], "role")?;
        }
        "workspace" => {
            object(value, &["name", "title", "subject", "panels"])?;
            string(value,"name")?; string(value,"title")?;
            if !structs.contains_key(string(value,"subject")?) { return Err("unknown workspace subject".into()); }
            for panel in array(value,"panels")? {
                object(panel, &["label", "source", "render", "actions"])?;
                string(panel,"label")?; function_ref(functions, string(panel,"source")?)?;
                for action in array(panel,"actions")? { check_action(action,functions)?; }
            }
        }
        "layout" => { object(value, &["tree"])?; check_layout(&value["tree"], functions)?; }
        _ => return Err(format!("unknown nirdosha declaration kind `{kind}`")),
    }
    Ok(())
}

fn check_action(action: &Value, functions: &BTreeMap<String, &syn::ItemFn>) -> Result<(), String> {
    object(action, &["label", "fn", "style", "confirm", "show_result"])?;
    string(action,"label")?; function_ref(functions,string(action,"fn")?)?;
    for key in ["style", "confirm"] { optional_string(action,key)?; }
    Ok(())
}

fn check_layout(node: &Value, functions: &BTreeMap<String, &syn::ItemFn>) -> Result<(), String> {
    let fields = object(node, &["row", "column", "group", "fields", "divider", "tabs", "tab", "timeline", "action"])?;
    if fields.is_empty() { return Err("empty layout node".into()); }
    for key in ["row", "column", "tabs"] { for child in array(node,key)? { check_layout(child,functions)?; } }
    for key in ["group", "tab", "action"] { optional_string(node,key)?; }
    for field in array(node,"fields")? { if !field.is_string() { return Err("layout field must be a string".into()); } }
    if let Some(timeline) = node.get("timeline") { object(timeline,&["source"])?; function_ref(functions,string(timeline,"source")?)?; }
    Ok(())
}

#[derive(Default)]
struct Calls(BTreeSet<String>);
impl<'ast> Visit<'ast> for Calls {
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = &*call.func { self.0.insert(path.path.segments.iter().map(|s| s.ident.to_string()).collect::<Vec<_>>().join("::")); }
        syn::visit::visit_expr_call(self,call);
    }
}
