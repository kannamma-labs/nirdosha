//! Finite, machine-checkable UI assurance contracts.
//!
//! This is deliberately a *model proof*, not a claim that screenshots prove
//! arbitrary browser behaviour.  A project declares its finite UI states and
//! actions in `.nir/ui-proof.json`; [`verify`] exhaustively checks the model's
//! structural invariants.  A certificate can then bind the declaration and
//! the resulting proof summary to the exact build.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub const SCHEMA: &str = "nirdosha.ui-proof/v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiProofSpec {
    pub schema: String,
    #[serde(default)]
    pub screens: Vec<ScreenSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenSpec {
    pub id: String,
    pub initial_state: String,
    #[serde(default)]
    pub states: Vec<String>,
    #[serde(default)]
    pub actions: Vec<ActionSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionSpec {
    pub id: String,
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub roles: Vec<String>,
    /// Named postconditions supplied by the application's runtime adapter.
    #[serde(default)]
    pub ensures: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiProof {
    pub schema: String,
    pub spec_hash: String,
    pub screens: usize,
    pub states: usize,
    pub actions: usize,
    pub reachable_states: usize,
    pub checked_invariants: Vec<String>,
    pub counterexamples: Vec<String>,
    pub passed: bool,
}

/// Evidence emitted by a real browser adapter. The adapter is intentionally
/// transport-neutral: Playwright, WebDriver, or a future native client can
/// produce the same trace and the verifier remains deterministic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeTrace {
    pub build_hash: String,
    #[serde(default)]
    pub events: Vec<TraceEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEvent {
    pub screen: String,
    pub action: String,
    pub role: String,
    pub before_state: String,
    pub after_state: String,
    #[serde(default)]
    pub observed_ensures: Vec<String>,
    #[serde(default)]
    pub request_hash: Option<String>,
    #[serde(default)]
    pub response_hash: Option<String>,
    #[serde(default)]
    pub audit_event_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceProof {
    pub trace_hash: String,
    pub events: usize,
    pub passed: bool,
    pub counterexamples: Vec<String>,
}

impl UiProofSpec {
    pub fn canonical_hash(&self) -> String {
        let bytes = serde_json::to_vec(self).expect("UI proof spec is serializable");
        hex(&Sha256::digest(bytes))
    }
}

/// Exhaustively checks the finite declaration. A successful result proves the
/// listed invariants for the declared model; it does not silently imply that
/// an uninstrumented browser or backend has been tested.
pub fn verify(spec: &UiProofSpec) -> UiProof {
    let mut errors = Vec::new();
    let mut state_count = 0;
    let mut action_count = 0;
    let mut reachable_count = 0;
    let mut screen_ids = BTreeSet::new();

    if spec.schema != SCHEMA {
        errors.push(format!("schema must be {SCHEMA}"));
    }
    if spec.screens.is_empty() {
        errors.push("at least one screen is required".to_string());
    }

    for screen in &spec.screens {
        if !screen_ids.insert(screen.id.clone()) {
            errors.push(format!("duplicate screen id `{}`", screen.id));
        }
        let states: BTreeSet<_> = screen.states.iter().cloned().collect();
        state_count += states.len();
        if states.is_empty() {
            errors.push(format!("screen `{}` declares no states", screen.id));
        }
        if !states.contains(&screen.initial_state) {
            errors.push(format!(
                "screen `{}` initial state `{}` is undeclared",
                screen.id, screen.initial_state
            ));
        }
        let mut action_ids = BTreeSet::new();
        let mut outgoing: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for action in &screen.actions {
            action_count += 1;
            if !action_ids.insert(action.id.clone()) {
                errors.push(format!(
                    "screen `{}` duplicate action `{}`",
                    screen.id, action.id
                ));
            }
            if !states.contains(&action.from) {
                errors.push(format!(
                    "screen `{}` action `{}` has undeclared source `{}`",
                    screen.id, action.id, action.from
                ));
            }
            if !states.contains(&action.to) {
                errors.push(format!(
                    "screen `{}` action `{}` has undeclared target `{}`",
                    screen.id, action.id, action.to
                ));
            }
            if action.roles.is_empty() {
                errors.push(format!(
                    "screen `{}` action `{}` has no permitted role",
                    screen.id, action.id
                ));
            }
            if action.ensures.is_empty() {
                errors.push(format!(
                    "screen `{}` action `{}` has no postcondition",
                    screen.id, action.id
                ));
            }
            outgoing
                .entry(action.from.clone())
                .or_default()
                .push(action.to.clone());
        }

        // Reachability is an exhaustive finite graph traversal, not a sample.
        let mut seen = BTreeSet::new();
        let mut queue = VecDeque::from([screen.initial_state.clone()]);
        while let Some(state) = queue.pop_front() {
            if !seen.insert(state.clone()) {
                continue;
            }
            for next in outgoing.get(&state).into_iter().flatten() {
                queue.push_back(next.clone());
            }
        }
        reachable_count += seen.len();
        for state in states.difference(&seen) {
            errors.push(format!(
                "screen `{}` state `{state}` is unreachable",
                screen.id
            ));
        }
    }

    let checked_invariants = vec![
        "unique screen and action identities".into(),
        "initial states are declared".into(),
        "all transition endpoints are declared".into(),
        "every action has an explicit role and postcondition".into(),
        "every declared state is reachable from the initial state".into(),
    ];
    UiProof {
        schema: SCHEMA.into(),
        spec_hash: spec.canonical_hash(),
        screens: spec.screens.len(),
        states: state_count,
        actions: action_count,
        reachable_states: reachable_count,
        passed: errors.is_empty(),
        counterexamples: errors,
        checked_invariants,
    }
}

/// Checks browser observations against the already-verified finite model.
/// This does not trust the browser's displayed status by itself: each event
/// must carry the expected transition and all declared postconditions. A
/// production adapter should additionally populate request/response/audit
/// hashes and have an independent oracle validate those artifacts.
pub fn verify_trace(spec: &UiProofSpec, trace: &RuntimeTrace) -> TraceProof {
    let mut errors = Vec::new();
    let screens: BTreeMap<_, _> = spec.screens.iter().map(|s| (s.id.as_str(), s)).collect();
    let mut state_by_screen: BTreeMap<&str, String> = BTreeMap::new();
    for event in &trace.events {
        let Some(screen) = screens.get(event.screen.as_str()) else {
            errors.push(format!("unknown screen `{}`", event.screen));
            continue;
        };
        let Some(action) = screen.actions.iter().find(|a| a.id == event.action) else {
            errors.push(format!(
                "screen `{}` has no action `{}`",
                event.screen, event.action
            ));
            continue;
        };
        let expected_before = state_by_screen
            .entry(event.screen.as_str())
            .or_insert_with(|| screen.initial_state.clone());
        if *expected_before != event.before_state {
            errors.push(format!(
                "screen `{}` action `{}` starts at `{}`, expected `{}`",
                event.screen, event.action, event.before_state, expected_before
            ));
        }
        if action.from != event.before_state || action.to != event.after_state {
            errors.push(format!(
                "screen `{}` action `{}` observed {} -> {}, expected {} -> {}",
                event.screen,
                event.action,
                event.before_state,
                event.after_state,
                action.from,
                action.to
            ));
        }
        if !action.roles.iter().any(|role| role == &event.role) {
            errors.push(format!(
                "role `{}` is not allowed to perform `{}` on `{}`",
                event.role, event.action, event.screen
            ));
        }
        for ensure in &action.ensures {
            if !event
                .observed_ensures
                .iter()
                .any(|observed| observed == ensure)
            {
                errors.push(format!(
                    "screen `{}` action `{}` missing postcondition `{ensure}`",
                    event.screen, event.action
                ));
            }
        }
        state_by_screen.insert(event.screen.as_str(), event.after_state.clone());
    }
    let bytes = serde_json::to_vec(trace).expect("runtime trace is serializable");
    TraceProof {
        trace_hash: hex(&Sha256::digest(bytes)),
        events: trace.events.len(),
        passed: errors.is_empty(),
        counterexamples: errors,
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> UiProofSpec {
        UiProofSpec {
            schema: SCHEMA.into(),
            screens: vec![ScreenSpec {
                id: "task_list".into(),
                initial_state: "empty".into(),
                states: vec!["empty".into(), "loaded".into()],
                actions: vec![ActionSpec {
                    id: "load".into(),
                    from: "empty".into(),
                    to: "loaded".into(),
                    roles: vec!["user".into()],
                    ensures: vec!["results_rendered".into()],
                }],
            }],
        }
    }

    #[test]
    fn finite_model_is_proved() {
        let proof = verify(&spec());
        assert!(proof.passed, "{proof:?}");
        assert_eq!(proof.reachable_states, 2);
    }

    #[test]
    fn undeclared_or_unreachable_state_is_a_counterexample() {
        let mut bad = spec();
        bad.screens[0].states.push("orphan".into());
        let proof = verify(&bad);
        assert!(!proof.passed);
        assert!(
            proof
                .counterexamples
                .iter()
                .any(|e| e.contains("unreachable"))
        );
    }

    #[test]
    fn runtime_trace_must_match_transition_role_and_postcondition() {
        let trace = RuntimeTrace {
            build_hash: "sha256:build".into(),
            events: vec![TraceEvent {
                screen: "task_list".into(),
                action: "load".into(),
                role: "user".into(),
                before_state: "empty".into(),
                after_state: "loaded".into(),
                observed_ensures: vec!["results_rendered".into()],
                request_hash: None,
                response_hash: None,
                audit_event_hash: None,
            }],
        };
        let proof = verify_trace(&spec(), &trace);
        assert!(proof.passed, "{proof:?}");
        let mut bad = trace;
        bad.events[0].role = "guest".into();
        assert!(!verify_trace(&spec(), &bad).passed);
    }
}
