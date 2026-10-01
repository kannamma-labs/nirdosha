//! Structural, exact conformance checking between an extracted workflow
//! spec (the same PRD-extraction JSON shape `.nir`'s
//! `extraction_schema::ExtractedWorkflow` already parses) and a
//! dialect `workflow! { .. }` macro's own declared states/transitions/
//! data fields (issue #73) — a port of `crates/compiler/src/
//! workflow_conformance.rs`'s algorithm, not a reimplementation from
//! scratch: same `Mismatch` shape, same "finite set/relation equality
//! over two already-fully-known structures, no solver needed" reasoning
//! its own module doc gives.
//!
//! Like its `.nir` counterpart, `on_entry`/`on_exit` are compared by
//! *count* only, never by matching an action label against a real
//! function call — the dialect's `workflow!` macro doesn't wire
//! `on_entry`/`on_exit` names to real function calls either (see that
//! macro's own module doc), so there is nothing more than count for
//! this check to verify yet.

use serde::Deserialize;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Deserialize)]
pub struct WorkflowSpec {
    pub name: String,
    #[serde(default)]
    pub data: Vec<SpecDataField>,
    #[serde(default)]
    pub states: Vec<SpecState>,
    #[serde(default)]
    pub transitions: Vec<SpecTransition>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SpecDataField {
    pub field: String,
    #[serde(rename = "type")]
    pub ty: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SpecState {
    pub name: String,
    #[serde(default)]
    pub terminal: bool,
    #[serde(default)]
    pub on_entry: Vec<String>,
    #[serde(default)]
    pub on_exit: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SpecTransition {
    pub from: String,
    pub event: String,
    pub to: String,
    #[serde(default)]
    pub link: bool,
}

/// A `workflow!` macro invocation's own parsed shape, built by the
/// macro from its own `syn::Parse` output before calling
/// [`check_conformance`] — plain owned data, no `syn`/`proc_macro2`
/// dependency here, so this module (and its tests) stay ordinary,
/// fast-compiling Rust.
#[derive(Debug, Clone, Default)]
pub struct DeclaredWorkflow {
    pub data: Vec<(String, String)>,
    pub states: Vec<DeclaredState>,
    /// `(from, event, to, link)`.
    pub transitions: Vec<(String, String, String, bool)>,
}

#[derive(Debug, Clone)]
pub struct DeclaredState {
    pub name: String,
    pub terminal: bool,
    pub on_entry: usize,
    pub on_exit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mismatch {
    MissingDataField { field: String, ty: String },
    ExtraDataField { field: String, ty: String },
    DataFieldTypeMismatch { field: String, spec_ty: String, actual_ty: String },
    MissingState { name: String },
    ExtraState { name: String },
    TerminalFlagMismatch { name: String, spec_terminal: bool, actual_terminal: bool },
    MissingTransition { from: String, event: String, to: String, link: bool },
    ExtraTransition { from: String, event: String, to: String, link: bool },
    OnEntryCountMismatch { state: String, spec: usize, actual: usize },
    OnExitCountMismatch { state: String, spec: usize, actual: usize },
}

impl std::fmt::Display for Mismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Mismatch::MissingDataField { field, ty } => write!(f, "spec data field `{field}: {ty}` has no match in the declared workflow's `data`"),
            Mismatch::ExtraDataField { field, ty } => write!(f, "declared workflow has data field `{field}: {ty}` the spec doesn't mention"),
            Mismatch::DataFieldTypeMismatch { field, spec_ty, actual_ty } => {
                write!(f, "data field `{field}`: spec type `{spec_ty}` != declared type `{actual_ty}`")
            }
            Mismatch::MissingState { name } => write!(f, "spec state `{name}` has no match in the declared workflow"),
            Mismatch::ExtraState { name } => write!(f, "declared workflow has state `{name}` the spec doesn't mention"),
            Mismatch::TerminalFlagMismatch { name, spec_terminal, actual_terminal } => write!(
                f,
                "state `{name}`: spec says terminal={spec_terminal}, declared workflow says terminal={actual_terminal}"
            ),
            Mismatch::MissingTransition { from, event, to, link } => {
                write!(f, "spec transition `{from} --{event}(link={link})--> {to}` has no match in the declared workflow")
            }
            Mismatch::ExtraTransition { from, event, to, link } => {
                write!(f, "declared workflow has transition `{from} --{event}(link={link})--> {to}` the spec doesn't mention")
            }
            Mismatch::OnEntryCountMismatch { state, spec, actual } => {
                write!(f, "state `{state}`: spec lists {spec} `on_entry` action(s), declared workflow has {actual}")
            }
            Mismatch::OnExitCountMismatch { state, spec, actual } => {
                write!(f, "state `{state}`: spec lists {spec} `on_exit` action(s), declared workflow has {actual}")
            }
        }
    }
}

/// Parse a spec file's raw JSON text — the `workflow!` macro's own
/// `syn::parse::Parse` impl reads the file at macro-expansion time and
/// calls this, so `nirdosha-macros` doesn't need its own `serde_json`
/// dependency just for this one JSON shape.
pub fn parse_spec(json: &str) -> Result<WorkflowSpec, String> {
    serde_json::from_str(json).map_err(|e| e.to_string())
}

/// Checks `declared` (a `workflow!` macro's own parsed body) against
/// `spec` — an empty result means every state, every transition, every
/// data field matched exactly; anything else lists precisely what
/// didn't, the same "concrete, actionable diff, not a bare 'doesn't
/// match'" property `workflow_conformance.rs::check_workflow_conformance`
/// documents for itself.
pub fn check_conformance(spec: &WorkflowSpec, declared: &DeclaredWorkflow) -> Vec<Mismatch> {
    let mut mismatches = Vec::new();
    check_data_fields(spec, declared, &mut mismatches);
    check_states(spec, declared, &mut mismatches);
    check_transitions(spec, declared, &mut mismatches);
    mismatches
}

fn check_data_fields(spec: &WorkflowSpec, declared: &DeclaredWorkflow, out: &mut Vec<Mismatch>) {
    let declared_fields: HashMap<&str, &str> =
        declared.data.iter().map(|(name, ty)| (name.as_str(), ty.as_str())).collect();
    let spec_names: HashSet<&str> = spec.data.iter().map(|f| f.field.as_str()).collect();

    for f in &spec.data {
        match declared_fields.get(f.field.as_str()) {
            None => out.push(Mismatch::MissingDataField { field: f.field.clone(), ty: f.ty.clone() }),
            Some(actual_ty) if *actual_ty != f.ty => out.push(Mismatch::DataFieldTypeMismatch {
                field: f.field.clone(),
                spec_ty: f.ty.clone(),
                actual_ty: actual_ty.to_string(),
            }),
            Some(_) => {}
        }
    }
    for (name, ty) in &declared_fields {
        if !spec_names.contains(name) {
            out.push(Mismatch::ExtraDataField { field: name.to_string(), ty: ty.to_string() });
        }
    }
}

fn check_states(spec: &WorkflowSpec, declared: &DeclaredWorkflow, out: &mut Vec<Mismatch>) {
    let declared_states: HashMap<&str, &DeclaredState> =
        declared.states.iter().map(|s| (s.name.as_str(), s)).collect();
    let spec_names: HashSet<&str> = spec.states.iter().map(|s| s.name.as_str()).collect();

    for s in &spec.states {
        match declared_states.get(s.name.as_str()) {
            None => out.push(Mismatch::MissingState { name: s.name.clone() }),
            Some(actual) => {
                if actual.terminal != s.terminal {
                    out.push(Mismatch::TerminalFlagMismatch {
                        name: s.name.clone(),
                        spec_terminal: s.terminal,
                        actual_terminal: actual.terminal,
                    });
                }
                if s.on_entry.len() != actual.on_entry {
                    out.push(Mismatch::OnEntryCountMismatch {
                        state: s.name.clone(),
                        spec: s.on_entry.len(),
                        actual: actual.on_entry,
                    });
                }
                if s.on_exit.len() != actual.on_exit {
                    out.push(Mismatch::OnExitCountMismatch {
                        state: s.name.clone(),
                        spec: s.on_exit.len(),
                        actual: actual.on_exit,
                    });
                }
            }
        }
    }
    for name in declared_states.keys() {
        if !spec_names.contains(name) {
            out.push(Mismatch::ExtraState { name: name.to_string() });
        }
    }
}

fn check_transitions(spec: &WorkflowSpec, declared: &DeclaredWorkflow, out: &mut Vec<Mismatch>) {
    let spec_set: HashSet<(String, String, String, bool)> = spec
        .transitions
        .iter()
        .map(|t| (t.from.clone(), t.event.clone(), t.to.clone(), t.link))
        .collect();
    let declared_set: HashSet<(String, String, String, bool)> = declared.transitions.iter().cloned().collect();

    for (from, event, to, link) in &spec_set {
        if !declared_set.contains(&(from.clone(), event.clone(), to.clone(), *link)) {
            out.push(Mismatch::MissingTransition { from: from.clone(), event: event.clone(), to: to.clone(), link: *link });
        }
    }
    for (from, event, to, link) in &declared_set {
        if !spec_set.contains(&(from.clone(), event.clone(), to.clone(), *link)) {
            out.push(Mismatch::ExtraTransition { from: from.clone(), event: event.clone(), to: to.clone(), link: *link });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_spec() -> WorkflowSpec {
        WorkflowSpec {
            name: "Onboarding".into(),
            data: vec![SpecDataField { field: "applicant_name".into(), ty: "String".into() }],
            states: vec![
                SpecState { name: "Draft".into(), terminal: false, on_entry: vec![], on_exit: vec![] },
                SpecState { name: "Approved".into(), terminal: true, on_entry: vec!["notify".into()], on_exit: vec![] },
            ],
            transitions: vec![SpecTransition { from: "Draft".into(), event: "Submit".into(), to: "Approved".into(), link: false }],
        }
    }

    fn matching_declared() -> DeclaredWorkflow {
        DeclaredWorkflow {
            data: vec![("applicant_name".into(), "String".into())],
            states: vec![
                DeclaredState { name: "Draft".into(), terminal: false, on_entry: 0, on_exit: 0 },
                DeclaredState { name: "Approved".into(), terminal: true, on_entry: 1, on_exit: 0 },
            ],
            transitions: vec![("Draft".into(), "Submit".into(), "Approved".into(), false)],
        }
    }

    #[test]
    fn an_exact_match_reports_no_mismatches() {
        assert!(check_conformance(&sample_spec(), &matching_declared()).is_empty());
    }

    #[test]
    fn a_missing_state_and_transition_are_both_reported() {
        let mut declared = matching_declared();
        declared.states.remove(1);
        declared.transitions.clear();
        let mismatches = check_conformance(&sample_spec(), &declared);
        assert!(mismatches.contains(&Mismatch::MissingState { name: "Approved".into() }));
        assert!(mismatches.contains(&Mismatch::MissingTransition {
            from: "Draft".into(),
            event: "Submit".into(),
            to: "Approved".into(),
            link: false,
        }));
    }

    #[test]
    fn an_extra_declared_state_is_reported() {
        let mut declared = matching_declared();
        declared.states.push(DeclaredState { name: "Rejected".into(), terminal: true, on_entry: 0, on_exit: 0 });
        let mismatches = check_conformance(&sample_spec(), &declared);
        assert!(mismatches.contains(&Mismatch::ExtraState { name: "Rejected".into() }));
    }

    #[test]
    fn a_terminal_flag_mismatch_is_reported() {
        let mut declared = matching_declared();
        declared.states[1].terminal = false;
        let mismatches = check_conformance(&sample_spec(), &declared);
        assert!(mismatches.contains(&Mismatch::TerminalFlagMismatch {
            name: "Approved".into(),
            spec_terminal: true,
            actual_terminal: false,
        }));
    }

    #[test]
    fn an_on_entry_count_mismatch_is_reported() {
        let mut declared = matching_declared();
        declared.states[1].on_entry = 0;
        let mismatches = check_conformance(&sample_spec(), &declared);
        assert!(mismatches.contains(&Mismatch::OnEntryCountMismatch {
            state: "Approved".into(),
            spec: 1,
            actual: 0,
        }));
    }

    #[test]
    fn a_data_field_type_mismatch_is_reported() {
        let mut declared = matching_declared();
        declared.data[0].1 = "i64".into();
        let mismatches = check_conformance(&sample_spec(), &declared);
        assert!(mismatches.contains(&Mismatch::DataFieldTypeMismatch {
            field: "applicant_name".into(),
            spec_ty: "String".into(),
            actual_ty: "i64".into(),
        }));
    }
}
