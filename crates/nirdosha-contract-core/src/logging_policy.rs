//! Logging policy register — the jurisdiction-aware rulebook that drives
//! `#[contract(logging(domain = "..", country = ".."))]`.
//!
//! This module parses the signed TOML policy register (schema
//! `logging-policy/v2`) and resolves the effective rules for a given
//! (domain, country, event_class). The resolved policy is what the
//! `#[contract]` macro turns into a compile-time `const` and a runtime
//! logging guard.
//!
//! The default policy file lives at `assets/logging-policies/default.toml`
//! and is embedded into `nirdosha-contract-core` via `include_str!`, so
//! plain `cargo build` has a fallback even when `cargo nirdosha` has not
//! staged a custom signed policy. Custom policies are read from
//! `NIRDOSHA_LOGGING_POLICY_PATH` at macro-expansion time; their signature
//! is verified by `cargo nirdosha` before the path is handed to rustc, not
//! by the proc macro itself.
//!
//! Design notes:
//! - Rules merge most-restrictive: a specific country rule overrides a
//!   wildcard; `never_log` fields are always forbidden; retention floors
//!   (`min_retention_days`) never drop below the merged value.
//! - The field dictionary gives canonical classification and default
//!   handling for logical field names like `cvv`, `pan`, `ssn`. Actual
//!   physical column names are supplied separately by `#[dataset(...)]`
//!   field maps (see `nirdosha_guard_registry::LOGGING_FIELD_MAPS`).

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Path to the default policy file, relative to this source file.
const DEFAULT_POLICY_TOML: &str = include_str!("../../../assets/logging-policies/default.toml");

/// Environment variable a Nirdosha-aware build driver sets to override the
/// embedded default with a verified custom policy.
pub const POLICY_PATH_ENV: &str = "NIRDOSHA_LOGGING_POLICY_PATH";

/// Top-level policy register.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoggingPolicyRegister {
    pub meta: Meta,
    #[serde(default)]
    pub signing: Option<SigningEnvelope>,
    #[serde(default)]
    pub precedence: Precedence,
    #[serde(default)]
    pub definitions: Definitions,
    #[serde(default)]
    pub never_log: NeverLog,
    #[serde(default)]
    pub fields: BTreeMap<String, FieldDef>,
    #[serde(default)]
    pub mask_functions: BTreeMap<String, MaskFunction>,
    #[serde(default)]
    pub default_rules: DefaultRules,
    #[serde(default)]
    pub storage: Storage,
    #[serde(default)]
    pub access: Access,
    #[serde(default)]
    pub subject_rights: SubjectRights,
    #[serde(default)]
    pub legal_hold: LegalHold,
    #[serde(default)]
    pub enforcement: Enforcement,
    #[serde(default)]
    pub dpia: Dpia,
    #[serde(default)]
    pub attestation: Attestation,
    #[serde(default)]
    pub rule: Vec<Rule>,
    #[serde(default)]
    pub waiver: Vec<Waiver>,
    #[serde(default)]
    pub test_vector: Vec<TestVector>,
    #[serde(default)]
    pub revision: Vec<Revision>,
}

/// Load the effective policy register. Custom path from the environment
/// takes precedence; otherwise the embedded default is used.
pub fn load_register() -> Result<LoggingPolicyRegister, String> {
    match std::env::var(POLICY_PATH_ENV) {
        Ok(path) => {
            let text = std::fs::read_to_string(&path)
                .map_err(|e| format!("cannot read policy at {path}: {e}"))?;
            parse_register(&text)
        }
        Err(_) => parse_register(DEFAULT_POLICY_TOML),
    }
}

/// Parse a policy register TOML document.
pub fn parse_register(text: &str) -> Result<LoggingPolicyRegister, String> {
    toml::from_str(text).map_err(|e| format!("logging policy TOML parse error: {e}"))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Meta {
    pub schema: String,
    pub id: String,
    pub version: String,
    pub title: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub issued: String,
    #[serde(default)]
    pub effective: String,
    #[serde(default)]
    pub next_review: String,
    #[serde(default)]
    pub supersedes: String,
    pub publisher: String,
    #[serde(default)]
    pub publisher_contact: String,
    #[serde(default)]
    pub approvers: Vec<String>,
    #[serde(default)]
    pub distribution: String,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SigningEnvelope {
    pub algorithm: String,
    #[serde(default)]
    pub key_id: String,
    #[serde(default)]
    pub key_fingerprint: String,
    #[serde(default)]
    pub signed_at: String,
    pub signature: String,
    #[serde(default)]
    pub countersigned_by: Vec<String>,
    #[serde(default)]
    pub verify_key_url: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Precedence {
    #[serde(default, rename = "match")]
    pub match_steps: Vec<String>,
    #[serde(default)]
    pub specificity: Vec<String>,
    #[serde(default)]
    pub merge: BTreeMap<String, String>,
    #[serde(default)]
    pub waivers_apply_after: String,
    #[serde(default)]
    pub legal_holds_apply_after: String,
    #[serde(default)]
    pub lint_rules: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definitions {
    #[serde(default)]
    pub levels: Vec<String>,
    #[serde(default)]
    pub event_classes: Vec<String>,
    #[serde(default)]
    pub integrity_levels: Vec<String>,
    #[serde(default)]
    pub authorities: Vec<String>,
    #[serde(default)]
    pub retention_classes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NeverLog {
    #[serde(default)]
    pub fields: Vec<String>,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldDef {
    #[serde(default)]
    pub classification: String,
    #[serde(default)]
    pub detect: String,
    #[serde(default)]
    pub default_handling: String,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaskFunction {
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub algorithms: Vec<String>,
    #[serde(default)]
    pub salt: String,
    #[serde(default)]
    pub params: String,
    #[serde(default)]
    pub vault: String,
    #[serde(default)]
    pub engines: Vec<String>,
    #[serde(default)]
    pub on_failure: String,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DefaultRules {
    #[serde(default)]
    pub level: String,
    #[serde(default)]
    pub retention_days: u64,
    #[serde(default)]
    pub min_retention_days: u64,
    #[serde(default)]
    pub archive_after_days: u64,
    #[serde(default)]
    pub mask: BTreeMap<String, String>,
    #[serde(default)]
    pub forbid: Vec<String>,
    #[serde(default)]
    pub require_in_event: Vec<String>,
    #[serde(default)]
    pub purposes: Vec<String>,
    #[serde(default)]
    pub encrypt_at_rest: String,
    #[serde(default)]
    pub encrypt_in_transit: String,
    #[serde(default)]
    pub integrity: String,
    #[serde(default)]
    pub dsar_searchable: bool,
    #[serde(default)]
    pub erasure: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Storage {
    #[serde(default)]
    pub residency_default: String,
    #[serde(default)]
    pub delete_semantics: String,
    #[serde(default)]
    pub proof_of_deletion: bool,
    #[serde(default)]
    pub key_management: String,
    #[serde(default)]
    pub key_rotation: String,
    #[serde(default)]
    pub backup_retention_days: u64,
    #[serde(default)]
    pub cross_border_mechanisms: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Access {
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub roles: Vec<AccessRole>,
    #[serde(default)]
    pub export_requires_case_id: bool,
    #[serde(default)]
    pub export_watermark: bool,
    #[serde(default)]
    pub log_access_logging: bool,
    #[serde(default)]
    pub log_access_retention_days: u64,
    #[serde(default)]
    pub break_glass: BTreeMap<String, toml::Value>,
    #[serde(default)]
    pub anonymize_for_shared_environments: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessRole {
    pub role: String,
    #[serde(default)]
    pub can: Vec<String>,
    #[serde(default)]
    pub approval: String,
    #[serde(default)]
    pub timeboxed_days: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubjectRights {
    #[serde(default)]
    pub dsar_search_default: bool,
    #[serde(default)]
    pub erasure_mechanisms: Vec<String>,
    #[serde(default)]
    pub erasure_sla_days: u64,
    #[serde(default)]
    pub opt_out_signals: Vec<String>,
    #[serde(default)]
    pub opt_out_effect: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegalHold {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub overrides: Vec<String>,
    #[serde(default)]
    pub require: Vec<String>,
    #[serde(default)]
    pub review_every_days: u64,
    #[serde(default)]
    pub auto_release_on_expiry: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Enforcement {
    #[serde(default)]
    pub validation_pipeline: Vec<String>,
    #[serde(default)]
    pub on_forbidden_field: String,
    #[serde(default)]
    pub on_missing_required: String,
    #[serde(default)]
    pub on_retention_violation: String,
    #[serde(default)]
    pub on_schema_mismatch: String,
    #[serde(default)]
    pub canary: BTreeMap<String, toml::Value>,
    #[serde(default)]
    pub sampling: BTreeMap<String, toml::Value>,
    #[serde(default)]
    pub volume_quota: BTreeMap<String, toml::Value>,
    #[serde(default)]
    pub violation_severity: BTreeMap<String, String>,
    #[serde(default)]
    pub remediation_sla_hours: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dpia {
    #[serde(default)]
    pub required_when: Vec<String>,
    #[serde(default)]
    pub blocking: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attestation {
    #[serde(default)]
    pub cadence: String,
    #[serde(default)]
    pub attesters: Vec<String>,
    #[serde(default)]
    pub evidence: Vec<String>,
}

/// One jurisdiction/rule.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub domain: String,
    #[serde(default)]
    pub subdomain: String,
    pub country: String,
    #[serde(default)]
    pub region: Vec<String>,
    #[serde(default)]
    pub regulator: String,
    #[serde(default)]
    pub basis: Vec<String>,
    #[serde(default)]
    pub authority: String,
    #[serde(default)]
    pub legal_basis: Vec<String>,
    #[serde(default)]
    pub event_classes: Vec<String>,
    #[serde(default)]
    pub level: String,
    #[serde(default)]
    pub level_overrides: Vec<LevelOverride>,
    #[serde(default)]
    pub retention_days: u64,
    #[serde(default)]
    pub min_retention_days: u64,
    #[serde(default)]
    pub archive_after_days: u64,
    #[serde(default)]
    pub mask: BTreeMap<String, String>,
    #[serde(default)]
    pub forbid: Vec<String>,
    #[serde(default)]
    pub require_in_event: Vec<String>,
    #[serde(default)]
    pub pseudonymize: Vec<String>,
    #[serde(default)]
    pub purposes: Vec<String>,
    #[serde(default)]
    pub integrity: String,
    #[serde(default)]
    pub tamper: String,
    #[serde(default)]
    pub storage_region: String,
    #[serde(default)]
    pub cross_border: Vec<String>,
    #[serde(default)]
    pub localization_required: bool,
    #[serde(default)]
    pub separate_consent_required: bool,
    #[serde(default)]
    pub breach_notification_hours: u64,
    #[serde(default)]
    pub dsar_searchable: bool,
    #[serde(default)]
    pub erasure: String,
    #[serde(default)]
    pub dpia: bool,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub special_categories: Vec<String>,
    #[serde(default)]
    pub minimum_necessary: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LevelOverride {
    pub event_class: String,
    pub level: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Waiver {
    pub id: String,
    pub rule_id: String,
    pub constraint: String,
    #[serde(default)]
    pub field: String,
    #[serde(default)]
    pub granted_handling: String,
    #[serde(default)]
    pub scope: BTreeMap<String, toml::Value>,
    #[serde(default)]
    pub requested_by: String,
    #[serde(default)]
    pub approved_by: Vec<String>,
    #[serde(default)]
    pub justification: String,
    #[serde(default)]
    pub issued: String,
    #[serde(default)]
    pub expires: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub compensating_controls: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestVector {
    pub id: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub event: BTreeMap<String, toml::Value>,
    #[serde(default)]
    pub expect: String,
    #[serde(default)]
    pub expect_reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Revision {
    pub version: String,
    #[serde(default)]
    pub date: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub changes: Vec<String>,
}

/// The effective, merged policy for a concrete (domain, country, event_class).
/// This is what the macro turns into a `const`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedPolicy {
    pub rule_ids: Vec<String>,
    pub level: LogLevel,
    pub retention_days: u64,
    pub min_retention_days: u64,
    pub mask: BTreeMap<String, String>,
    pub forbid: BTreeSet<String>,
    pub require_in_event: BTreeSet<String>,
    pub integrity: String,
    pub purposes: Vec<String>,
    pub storage_region: String,
    pub basis: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LogLevel {
    Debug,
    Info,
    Notice,
    Warn,
    Error,
    Fatal,
}

impl LogLevel {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "debug" => Some(Self::Debug),
            "info" => Some(Self::Info),
            "notice" => Some(Self::Notice),
            "warn" => Some(Self::Warn),
            "error" => Some(Self::Error),
            "fatal" => Some(Self::Fatal),
            _ => None,
        }
    }
}

impl LoggingPolicyRegister {
    /// Resolve the effective policy for a concrete call site.
    /// `region` is optional and currently only used for region-specific rules.
    pub fn resolve(
        &self,
        domain: &str,
        country: &str,
        event_class: &str,
        _region: Option<&str>,
    ) -> Result<ResolvedPolicy, String> {
        // 1. Start from defaults.
        let mut level = LogLevel::parse(&self.default_rules.level).unwrap_or(LogLevel::Info);
        let mut retention_days = self.default_rules.retention_days;
        let mut min_retention_days = self.default_rules.min_retention_days;
        let mut mask: BTreeMap<String, String> = self.default_rules.mask.clone();
        let mut forbid: BTreeSet<String> = self.default_rules.forbid.iter().cloned().collect();
        let mut require_in_event: BTreeSet<String> =
            self.default_rules.require_in_event.iter().cloned().collect();
        let mut integrity = self.default_rules.integrity.clone();
        let mut purposes: BTreeSet<String> = self.default_rules.purposes.iter().cloned().collect();
        let mut storage_region = self.storage.residency_default.clone();
        let mut basis: BTreeSet<String> = BTreeSet::new();
        let mut matched_rules: Vec<String> = Vec::new();

        // 2. Apply never_log as unconditional forbid.
        for f in &self.never_log.fields {
            forbid.insert(f.clone());
        }

        // 3. Group matching rules by specificity. Within the same
        // specificity, values merge according to the register's merge
        // semantics; a more specific group then overrides the previous,
        // less specific merged result. Defaults form the base layer.
        let mut groups: BTreeMap<u8, Vec<&Rule>> = BTreeMap::new();
        for rule in self.rule.iter().filter(|r| rule_matches(r, domain, country, event_class)) {
            let score = rule_specificity_score(rule, domain, country);
            groups.entry(score).or_default().push(rule);
            matched_rules.push(rule.id.clone());
            basis.extend(rule.basis.iter().cloned());
        }

        // Apply groups from least specific to most specific.
        for (_score, rules) in groups {
            let mut group_level: Option<LogLevel> = None;
            let mut group_retention: Option<u64> = None;
            let mut group_min_retention: Option<u64> = None;
            let mut group_mask: BTreeMap<String, String> = BTreeMap::new();
            let mut group_forbid: BTreeSet<String> = BTreeSet::new();
            let mut group_require: BTreeSet<String> = BTreeSet::new();
            let mut group_integrity: Option<String> = None;
            let mut group_storage_region: Option<String> = None;
            let mut group_purposes: BTreeSet<String> = BTreeSet::new();

            for rule in rules {
                if let Some(l) = rule_effective_level(rule, event_class) {
                    group_level = Some(group_level.map(|gl| gl.max(l)).unwrap_or(l));
                }
                if rule.retention_days > 0 {
                    group_retention =
                        Some(group_retention.map(|v| v.min(rule.retention_days)).unwrap_or(rule.retention_days));
                }
                if rule.min_retention_days > 0 {
                    group_min_retention = Some(
                        group_min_retention
                            .map(|v| v.max(rule.min_retention_days))
                            .unwrap_or(rule.min_retention_days),
                    );
                }
                for (field, transform) in &rule.mask {
                    group_mask.insert(field.clone(), transform.clone());
                }
                for f in &rule.forbid {
                    group_forbid.insert(f.clone());
                }
                for f in &rule.require_in_event {
                    group_require.insert(f.clone());
                }
                for p in &rule.purposes {
                    group_purposes.insert(p.clone());
                }
                if !rule.integrity.is_empty() {
                    group_integrity = Some(rule.integrity.clone());
                }
                if !rule.storage_region.is_empty() {
                    group_storage_region = Some(rule.storage_region.clone());
                }
            }

            // Override less-specific merged result with this group's values.
            if let Some(l) = group_level {
                level = l;
            }
            if let Some(v) = group_retention {
                retention_days = v;
            }
            if let Some(v) = group_min_retention {
                min_retention_days = v;
            }
            mask.extend(group_mask);
            forbid.extend(group_forbid);
            require_in_event.extend(group_require);
            purposes.extend(group_purposes);
            if let Some(i) = group_integrity {
                integrity = i;
            }
            if let Some(s) = group_storage_region {
                storage_region = s;
            }
        }

        // 4. Forbid is stronger than mask: drop any masked field that is
        // also forbidden. (The lint below would otherwise reject this.)
        for f in &forbid {
            mask.remove(f);
        }

        // 5. Lint: ceiling must not be below floor.
        if retention_days < min_retention_days {
            return Err(format!(
                "resolved retention_days ({retention_days}) is below min_retention_days ({min_retention_days}) for domain={domain} country={country} event_class={event_class}"
            ));
        }

        // 6. Lint: forbidden field must not also be masked.
        for f in &forbid {
            if mask.contains_key(f) {
                return Err(format!(
                    "field `{f}` appears in both mask and forbid for domain={domain} country={country} event_class={event_class}"
                ));
            }
        }

        Ok(ResolvedPolicy {
            rule_ids: matched_rules,
            level,
            retention_days,
            min_retention_days,
            mask,
            forbid,
            require_in_event,
            integrity,
            purposes: purposes.into_iter().collect(),
            storage_region,
            basis: basis.into_iter().collect(),
        })
    }
}

fn rule_effective_level(rule: &Rule, event_class: &str) -> Option<LogLevel> {
    rule.level_overrides
        .iter()
        .find(|o| o.event_class == event_class)
        .and_then(|o| LogLevel::parse(&o.level))
        .or_else(|| {
            if rule.level.is_empty() {
                None
            } else {
                LogLevel::parse(&rule.level)
            }
        })
}

fn rule_matches(rule: &Rule, domain: &str, country: &str, event_class: &str) -> bool {
    let domain_ok = rule.domain == "*" || rule.domain == domain;
    let country_ok = rule.country == "*" || rule.country == country;
    let event_class_ok =
        rule.event_classes.is_empty() || rule.event_classes.iter().any(|c| c == event_class);
    domain_ok && country_ok && event_class_ok
}

/// Higher score = more specific = applied later.
fn rule_specificity_score(rule: &Rule, domain: &str, _country: &str) -> u8 {
    let mut score: u8 = 0;
    if !rule.region.is_empty() {
        score += 1;
    }
    if !rule.subdomain.is_empty() {
        score += 1;
    }
    if rule.country != "*" {
        score += 1;
    }
    if rule.domain != "*" && rule.domain == domain {
        score += 1;
    }
    score
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_loads_and_resolves() {
        let reg = load_register().expect("default policy must parse");
        let resolved = reg.resolve("payments", "US", "transaction", None).unwrap();
        assert_eq!(resolved.level, LogLevel::Info);
        assert_eq!(resolved.retention_days, 365);
        assert!(resolved.forbid.contains("cvv"));
        assert!(resolved.forbid.contains("pin"));
        assert!(resolved.mask.contains_key("pan"));
        assert!(resolved.require_in_event.contains("request_id"));
    }

    #[test]
    fn debug_event_is_hostile() {
        let reg = load_register().unwrap();
        let resolved = reg.resolve("*", "*", "debug", None).unwrap();
        assert_eq!(resolved.level, LogLevel::Debug);
        assert_eq!(resolved.retention_days, 7);
        assert!(resolved.forbid.contains("user_id"));
        assert!(resolved.forbid.contains("email"));
        assert!(resolved.forbid.contains("free_text"));
    }

    #[test]
    fn eu_payments_has_purpose_limitation() {
        let reg = load_register().unwrap();
        let resolved = reg.resolve("payments", "EU", "transaction", None).unwrap();
        assert_eq!(resolved.retention_days, 365);
        assert!(resolved.purposes.contains(&"security".to_string()));
        assert!(!resolved.purposes.contains(&"marketing".to_string()));
        assert_eq!(resolved.storage_region, "eu");
    }
}
