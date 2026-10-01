//! The rule governance model: a `RuleDefinition` is versioned, has a real
//! lifecycle (`Draft -> PendingApproval -> Approved -> {Enabled|Disabled}
//! -> Retired`), and only ever becomes the "active" rule realtime/batch
//! consult through [`CtmsRuleGatewayV1`] (see `crate::rule_gateway`) --
//! never mutated directly. This is what turns the CTMS demo from "one
//! hardcoded rule" ([`crate::rules::RuleCatalog`], still real and still
//! used by the original realtime/batch slice) into an actual control
//! plane: rules are drafted, approved by someone other than their author,
//! and only then exposed to evaluation -- with every prior version still
//! auditable.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RuleType {
    AmountThreshold { min_amount_minor: u64 },
    Velocity { window_ms: u64, min_count: usize, min_total_minor: u64 },
    GeoMismatch { allowed_jurisdictions: Vec<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuleStatus {
    Draft,
    PendingApproval,
    Approved,
    Enabled,
    Disabled,
    Retired,
}

fn status_name(status: RuleStatus) -> &'static str {
    match status {
        RuleStatus::Draft => "Draft",
        RuleStatus::PendingApproval => "PendingApproval",
        RuleStatus::Approved => "Approved",
        RuleStatus::Enabled => "Enabled",
        RuleStatus::Disabled => "Disabled",
        RuleStatus::Retired => "Retired",
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleDefinition {
    pub rule_id: String,
    pub version: u32,
    pub name: String,
    pub description: String,
    pub rule_type: RuleType,
    pub currency: String,
    pub jurisdiction: String,
    pub severity: String,
    pub effective_from_ms: u64,
    pub effective_until_ms: Option<u64>,
    pub status: RuleStatus,
    pub author: String,
    pub approved_by: Option<String>,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleStoreError {
    UnknownRule { rule_id: String, version: u32 },
    /// A draft was submitted for `rule_id`/`version` that already exists
    /// -- "rejects overlapping or ambiguous versions."
    VersionAlreadyExists { rule_id: String, version: u32 },
    InvalidRule(&'static str),
    WrongState { expected: &'static str, actual: &'static str },
    /// The approver is the same subject who authored the rule.
    SameActorApproval,
    /// `rollback` was asked to reactivate a version that was never
    /// `Approved`-lineage (i.e. never real).
    RollbackTargetNeverApproved { rule_id: String, version: u32 },
}

impl std::fmt::Display for RuleStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RuleStoreError::UnknownRule { rule_id, version } => write!(f, "no such rule: {rule_id} v{version}"),
            RuleStoreError::VersionAlreadyExists { rule_id, version } => write!(f, "rule {rule_id} v{version} already exists -- versions may not overlap or be ambiguous"),
            RuleStoreError::InvalidRule(reason) => write!(f, "invalid rule: {reason}"),
            RuleStoreError::WrongState { expected, actual } => write!(f, "rule must be {expected} for this action, was {actual}"),
            RuleStoreError::SameActorApproval => write!(f, "a rule's author cannot approve their own rule"),
            RuleStoreError::RollbackTargetNeverApproved { rule_id, version } => write!(f, "cannot roll back {rule_id} to v{version}: that version was never approved"),
        }
    }
}

/// Validates a [`RuleDefinition`]'s own structural invariants -- "validates
/// rule structure" from `CtmsRuleGatewayV1`'s own contract. Called before
/// a draft is ever accepted.
fn validate_structure(rule: &RuleDefinition) -> Result<(), RuleStoreError> {
    if rule.rule_id.trim().is_empty() {
        return Err(RuleStoreError::InvalidRule("rule_id must not be empty"));
    }
    if rule.name.trim().is_empty() {
        return Err(RuleStoreError::InvalidRule("name must not be empty"));
    }
    if rule.currency.trim().is_empty() {
        return Err(RuleStoreError::InvalidRule("currency must not be empty"));
    }
    if rule.jurisdiction.trim().is_empty() {
        return Err(RuleStoreError::InvalidRule("jurisdiction must not be empty"));
    }
    if rule.severity.trim().is_empty() {
        return Err(RuleStoreError::InvalidRule("severity must not be empty"));
    }
    if let Some(until) = rule.effective_until_ms {
        if until <= rule.effective_from_ms {
            return Err(RuleStoreError::InvalidRule("effective_until_ms must be after effective_from_ms"));
        }
    }
    match &rule.rule_type {
        RuleType::AmountThreshold { min_amount_minor } if *min_amount_minor == 0 => Err(RuleStoreError::InvalidRule("min_amount_minor must be greater than zero")),
        RuleType::Velocity { window_ms, min_count, .. } if *window_ms == 0 || *min_count == 0 => Err(RuleStoreError::InvalidRule("velocity window_ms and min_count must be greater than zero")),
        RuleType::GeoMismatch { allowed_jurisdictions } if allowed_jurisdictions.is_empty() => Err(RuleStoreError::InvalidRule("allowed_jurisdictions must not be empty")),
        _ => Ok(()),
    }
}

pub trait RuleStore: Send + Sync {
    fn create_draft(&self, rule: RuleDefinition) -> Result<RuleDefinition, RuleStoreError>;
    fn submit_for_approval(&self, rule_id: &str, version: u32) -> Result<RuleDefinition, RuleStoreError>;
    /// `actor` must differ from the rule's own `author`.
    fn approve(&self, rule_id: &str, version: u32, actor: &str) -> Result<RuleDefinition, RuleStoreError>;
    /// Enables this version and disables whatever other version of
    /// `rule_id` was previously `Enabled` -- "exposes the active immutable
    /// rule snapshot," singular, per `rule_id`.
    fn enable(&self, rule_id: &str, version: u32) -> Result<RuleDefinition, RuleStoreError>;
    fn disable(&self, rule_id: &str, version: u32) -> Result<RuleDefinition, RuleStoreError>;
    fn retire(&self, rule_id: &str, version: u32) -> Result<RuleDefinition, RuleStoreError>;
    /// Disables the currently `Enabled` version (if any) of `rule_id` and
    /// re-enables `to_version`, which must already have been `Approved`
    /// at some point (still present in history, status not `Draft`/
    /// `PendingApproval`).
    fn rollback(&self, rule_id: &str, to_version: u32) -> Result<RuleDefinition, RuleStoreError>;
    fn get(&self, rule_id: &str, version: u32) -> Option<RuleDefinition>;
    /// The one `Enabled` version of `rule_id`, if any -- what realtime and
    /// batch evaluation both consult.
    fn active_snapshot(&self, rule_id: &str) -> Option<RuleDefinition>;
    /// Every version ever created for `rule_id`, oldest first -- "an old
    /// rule version remains auditable."
    fn history(&self, rule_id: &str) -> Vec<RuleDefinition>;
    /// Every currently `Enabled` rule, across all `rule_id`s.
    fn list_active(&self) -> Vec<RuleDefinition>;
}

#[derive(Default)]
struct Inner {
    // (rule_id, version) -> definition
    rules: HashMap<(String, u32), RuleDefinition>,
}

#[derive(Default)]
pub struct InMemoryRuleStore {
    inner: Mutex<Inner>,
}

impl InMemoryRuleStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl RuleStore for InMemoryRuleStore {
    fn create_draft(&self, rule: RuleDefinition) -> Result<RuleDefinition, RuleStoreError> {
        validate_structure(&rule)?;
        let mut inner = self.inner.lock().expect("rule store lock poisoned");
        let key = (rule.rule_id.clone(), rule.version);
        if inner.rules.contains_key(&key) {
            return Err(RuleStoreError::VersionAlreadyExists { rule_id: rule.rule_id, version: rule.version });
        }
        let mut rule = rule;
        rule.status = RuleStatus::Draft;
        rule.approved_by = None;
        inner.rules.insert(key, rule.clone());
        Ok(rule)
    }

    fn submit_for_approval(&self, rule_id: &str, version: u32) -> Result<RuleDefinition, RuleStoreError> {
        let mut inner = self.inner.lock().expect("rule store lock poisoned");
        let rule = inner.rules.get_mut(&(rule_id.to_string(), version)).ok_or_else(|| RuleStoreError::UnknownRule { rule_id: rule_id.to_string(), version })?;
        if rule.status != RuleStatus::Draft {
            return Err(RuleStoreError::WrongState { expected: "Draft", actual: status_name(rule.status) });
        }
        rule.status = RuleStatus::PendingApproval;
        Ok(rule.clone())
    }

    fn approve(&self, rule_id: &str, version: u32, actor: &str) -> Result<RuleDefinition, RuleStoreError> {
        let mut inner = self.inner.lock().expect("rule store lock poisoned");
        let rule = inner.rules.get_mut(&(rule_id.to_string(), version)).ok_or_else(|| RuleStoreError::UnknownRule { rule_id: rule_id.to_string(), version })?;
        if rule.status != RuleStatus::PendingApproval {
            return Err(RuleStoreError::WrongState { expected: "PendingApproval", actual: status_name(rule.status) });
        }
        if rule.author == actor {
            return Err(RuleStoreError::SameActorApproval);
        }
        rule.status = RuleStatus::Approved;
        rule.approved_by = Some(actor.to_string());
        Ok(rule.clone())
    }

    fn enable(&self, rule_id: &str, version: u32) -> Result<RuleDefinition, RuleStoreError> {
        let mut inner = self.inner.lock().expect("rule store lock poisoned");
        {
            let rule = inner.rules.get(&(rule_id.to_string(), version)).ok_or_else(|| RuleStoreError::UnknownRule { rule_id: rule_id.to_string(), version })?;
            if rule.status != RuleStatus::Approved {
                return Err(RuleStoreError::WrongState { expected: "Approved", actual: status_name(rule.status) });
            }
        }
        // Disable any other currently-Enabled version of the same rule_id
        // first -- exactly one active snapshot per rule_id.
        for ((id, v), r) in inner.rules.iter_mut() {
            if id == rule_id && *v != version && r.status == RuleStatus::Enabled {
                r.status = RuleStatus::Disabled;
            }
        }
        let rule = inner.rules.get_mut(&(rule_id.to_string(), version)).expect("checked above");
        rule.status = RuleStatus::Enabled;
        Ok(rule.clone())
    }

    fn disable(&self, rule_id: &str, version: u32) -> Result<RuleDefinition, RuleStoreError> {
        let mut inner = self.inner.lock().expect("rule store lock poisoned");
        let rule = inner.rules.get_mut(&(rule_id.to_string(), version)).ok_or_else(|| RuleStoreError::UnknownRule { rule_id: rule_id.to_string(), version })?;
        if rule.status != RuleStatus::Enabled {
            return Err(RuleStoreError::WrongState { expected: "Enabled", actual: status_name(rule.status) });
        }
        rule.status = RuleStatus::Disabled;
        Ok(rule.clone())
    }

    fn retire(&self, rule_id: &str, version: u32) -> Result<RuleDefinition, RuleStoreError> {
        let mut inner = self.inner.lock().expect("rule store lock poisoned");
        let rule = inner.rules.get_mut(&(rule_id.to_string(), version)).ok_or_else(|| RuleStoreError::UnknownRule { rule_id: rule_id.to_string(), version })?;
        if rule.status == RuleStatus::Retired {
            return Err(RuleStoreError::WrongState { expected: "not already Retired", actual: "Retired" });
        }
        rule.status = RuleStatus::Retired;
        Ok(rule.clone())
    }

    fn rollback(&self, rule_id: &str, to_version: u32) -> Result<RuleDefinition, RuleStoreError> {
        let mut inner = self.inner.lock().expect("rule store lock poisoned");
        {
            let target = inner.rules.get(&(rule_id.to_string(), to_version)).ok_or_else(|| RuleStoreError::UnknownRule { rule_id: rule_id.to_string(), version: to_version })?;
            if matches!(target.status, RuleStatus::Draft | RuleStatus::PendingApproval) {
                return Err(RuleStoreError::RollbackTargetNeverApproved { rule_id: rule_id.to_string(), version: to_version });
            }
        }
        for ((id, v), r) in inner.rules.iter_mut() {
            if id == rule_id && *v != to_version && r.status == RuleStatus::Enabled {
                r.status = RuleStatus::Disabled;
            }
        }
        let target = inner.rules.get_mut(&(rule_id.to_string(), to_version)).expect("checked above");
        target.status = RuleStatus::Enabled;
        Ok(target.clone())
    }

    fn get(&self, rule_id: &str, version: u32) -> Option<RuleDefinition> {
        self.inner.lock().expect("rule store lock poisoned").rules.get(&(rule_id.to_string(), version)).cloned()
    }

    fn active_snapshot(&self, rule_id: &str) -> Option<RuleDefinition> {
        self.inner.lock().expect("rule store lock poisoned").rules.values().find(|r| r.rule_id == rule_id && r.status == RuleStatus::Enabled).cloned()
    }

    fn history(&self, rule_id: &str) -> Vec<RuleDefinition> {
        let mut versions: Vec<RuleDefinition> = self.inner.lock().expect("rule store lock poisoned").rules.values().filter(|r| r.rule_id == rule_id).cloned().collect();
        versions.sort_by_key(|r| r.version);
        versions
    }

    fn list_active(&self) -> Vec<RuleDefinition> {
        self.inner.lock().expect("rule store lock poisoned").rules.values().filter(|r| r.status == RuleStatus::Enabled).cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(rule_id: &str, version: u32, author: &str) -> RuleDefinition {
        RuleDefinition {
            rule_id: rule_id.to_string(),
            version,
            name: "Velocity 24h".to_string(),
            description: "5 debits, 500000 minor, 24h".to_string(),
            rule_type: RuleType::Velocity { window_ms: 86_400_000, min_count: 5, min_total_minor: 500_000 },
            currency: "INR".to_string(),
            jurisdiction: "IN".to_string(),
            severity: "high".to_string(),
            effective_from_ms: 0,
            effective_until_ms: None,
            status: RuleStatus::Draft,
            author: author.to_string(),
            approved_by: None,
            created_at_ms: 0,
        }
    }

    #[test]
    fn a_second_draft_with_the_same_rule_id_and_version_is_rejected() {
        let store = InMemoryRuleStore::new();
        store.create_draft(draft("velocity_24h", 1, "priya")).unwrap();
        let err = store.create_draft(draft("velocity_24h", 1, "priya")).unwrap_err();
        assert_eq!(err, RuleStoreError::VersionAlreadyExists { rule_id: "velocity_24h".to_string(), version: 1 });
    }

    #[test]
    fn an_author_cannot_approve_their_own_rule() {
        let store = InMemoryRuleStore::new();
        store.create_draft(draft("velocity_24h", 1, "priya")).unwrap();
        store.submit_for_approval("velocity_24h", 1).unwrap();
        let err = store.approve("velocity_24h", 1, "priya").unwrap_err();
        assert_eq!(err, RuleStoreError::SameActorApproval);
    }

    #[test]
    fn cannot_enable_a_rule_that_was_never_approved() {
        let store = InMemoryRuleStore::new();
        store.create_draft(draft("velocity_24h", 1, "priya")).unwrap();
        let err = store.enable("velocity_24h", 1).unwrap_err();
        assert_eq!(err, RuleStoreError::WrongState { expected: "Approved", actual: "Draft" });
    }

    #[test]
    fn enabling_a_new_version_disables_the_previously_active_one() {
        let store = InMemoryRuleStore::new();
        for version in [1, 2] {
            store.create_draft(draft("velocity_24h", version, "priya")).unwrap();
            store.submit_for_approval("velocity_24h", version).unwrap();
            store.approve("velocity_24h", version, "arjun").unwrap();
        }
        store.enable("velocity_24h", 1).unwrap();
        assert_eq!(store.active_snapshot("velocity_24h").unwrap().version, 1);
        store.enable("velocity_24h", 2).unwrap();
        let active = store.active_snapshot("velocity_24h").unwrap();
        assert_eq!(active.version, 2);
        assert_eq!(store.get("velocity_24h", 1).unwrap().status, RuleStatus::Disabled);
    }

    #[test]
    fn rollback_reactivates_an_old_approved_version_and_it_stays_auditable() {
        let store = InMemoryRuleStore::new();
        for version in [1, 2] {
            store.create_draft(draft("velocity_24h", version, "priya")).unwrap();
            store.submit_for_approval("velocity_24h", version).unwrap();
            store.approve("velocity_24h", version, "arjun").unwrap();
        }
        store.enable("velocity_24h", 1).unwrap();
        store.enable("velocity_24h", 2).unwrap();

        let rolled_back = store.rollback("velocity_24h", 1).unwrap();
        assert_eq!(rolled_back.status, RuleStatus::Enabled);
        assert_eq!(store.active_snapshot("velocity_24h").unwrap().version, 1);
        assert_eq!(store.get("velocity_24h", 2).unwrap().status, RuleStatus::Disabled);

        let history = store.history("velocity_24h");
        assert_eq!(history.len(), 2, "both versions remain auditable: {history:?}");
    }

    #[test]
    fn rollback_to_a_version_that_was_never_approved_is_rejected() {
        let store = InMemoryRuleStore::new();
        store.create_draft(draft("velocity_24h", 1, "priya")).unwrap();
        let err = store.rollback("velocity_24h", 1).unwrap_err();
        assert_eq!(err, RuleStoreError::RollbackTargetNeverApproved { rule_id: "velocity_24h".to_string(), version: 1 });
    }

    #[test]
    fn an_invalid_rule_structure_is_rejected_before_it_becomes_a_draft() {
        let store = InMemoryRuleStore::new();
        let mut bad = draft("velocity_24h", 1, "priya");
        bad.jurisdiction = String::new();
        let err = store.create_draft(bad).unwrap_err();
        assert_eq!(err, RuleStoreError::InvalidRule("jurisdiction must not be empty"));
    }
}
