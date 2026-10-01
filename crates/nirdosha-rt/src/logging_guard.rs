//! Jurisdiction-aware logging guard.
//!
//! The `#[contract(logging(domain = "..", country = ".."))]` macro injects
//! a `Drop`-based guard around the function body. The guard carries the
//! resolved policy (`Policy`) and, on every exit path, records one metadata
//! event to the existing flight-recorder sink (`NIRDOSHA_NFR_LOG_FILE`).
//!
//! The policy itself contains *logical* compliance field names (`cvv`,
//! `pan`, ...). At runtime, when the caller asks to scrub an event value,
//! the guard resolves those logical names to physical column names using the
//! `LOGGING_FIELD_MAPS` distributed slice populated by `#[dataset(...)]`.
//!
//! Honest limitations (first slice):
//! - Field maps are resolved at runtime, not at macro-expansion time. A
//!   missing map means the logical name is treated as a literal physical
//!   path.
//! - Masks are applied to `serde_json::Value` only; custom Rust structs
//!   must be serialized first.
//! - The guard does not yet enforce retention or storage-region at runtime;
//!   those travel in the policy metadata for downstream collectors.

use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// Resolved policy the macro embeds as a `const`.
#[derive(Debug, Clone, Copy)]
pub struct Policy {
    pub rule_ids: &'static str,
    pub domain: &'static str,
    pub country: &'static str,
    pub level: Level,
    pub retention_days: u64,
    /// Logical field names that must be masked, plus the mask transform
    /// string from the policy (e.g. "partial(6,4)").
    pub mask_fields: &'static [(&'static str, &'static str)],
    /// Logical field names that must never appear in a logged event.
    pub forbid_fields: &'static [&'static str],
    /// Logical field names that must be present in a logged event.
    pub require_fields: &'static [&'static str],
    pub basis: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Debug,
    Info,
    Notice,
    Warn,
    Error,
    Fatal,
}

impl Policy {
    /// True if the policy permits an event at `event_level` to be emitted.
    pub fn is_enabled(&self, event_level: Level) -> bool {
        event_level >= self.level
    }
}

/// Enter a guarded function. Called only by code the `#[contract]` macro
/// injects. `entity` is the optional dataset entity name used to resolve
/// logical field names to physical paths at scrub time.
pub fn enter(function: &'static str, policy: &'static Policy, entity: Option<&'static str>) -> Guard {
    let _ = start_time();
    push_active(policy, entity);
    Guard {
        function,
        policy,
        entity,
        started: Instant::now(),
    }
}

pub struct Guard {
    function: &'static str,
    policy: &'static Policy,
    entity: Option<&'static str>,
    started: Instant,
}

impl Drop for Guard {
    fn drop(&mut self) {
        let latency_ms = self.started.elapsed().as_secs_f64() * 1_000.0;
        record_metadata(self.function, self.policy, self.entity, latency_ms);
        pop_active();
    }
}

/// Record a function-call metadata event. Emits to the same sinks the NFR
/// guard uses: stderr if `NIRDOSHA_NFR_LOG=1`, and the file configured by
/// `NIRDOSHA_NFR_LOG_FILE`.
fn record_metadata(
    function: &str,
    policy: &Policy,
    entity: Option<&str>,
    latency_ms: f64,
) {
    let event = serde_json::json!({
        "kind": "logging_guard_metadata",
        "function": function,
        "domain": policy.domain,
        "country": policy.country,
        "level": level_name(policy.level),
        "retention_days": policy.retention_days,
        "entity": entity,
        "rule_ids": policy.rule_ids,
        "basis": policy.basis,
        "latency_ms": latency_ms,
    });
    let line = serde_json::to_string(&event).unwrap_or_default();
    if !line.is_empty() {
        if std::env::var_os("NIRDOSHA_NFR_LOG").is_some_and(|v| !v.is_empty()) {
            eprintln!("{line}");
        }
        if let Some(path) = std::env::var_os("NIRDOSHA_NFR_LOG_FILE") {
            if !path.is_empty() {
                use std::io::Write as _;
                if let Ok(mut file) =
                    std::fs::OpenOptions::new().create(true).append(true).open(path)
                {
                    let _ = writeln!(file, "{line}");
                }
            }
        }
    }
}

fn level_name(level: Level) -> &'static str {
    match level {
        Level::Debug => "debug",
        Level::Info => "info",
        Level::Notice => "notice",
        Level::Warn => "warn",
        Level::Error => "error",
        Level::Fatal => "fatal",
    }
}

fn start_time() -> Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    *START.get_or_init(Instant::now)
}

// -----------------------------------------------------------------------------
// Active guard stack (thread-local) so `emit` can inherit the current policy.
// -----------------------------------------------------------------------------

struct Active {
    policy: &'static Policy,
    entity: Option<&'static str>,
}

thread_local! {
    static ACTIVE: std::cell::RefCell<Vec<Active>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn push_active(policy: &'static Policy, entity: Option<&'static str>) {
    ACTIVE.with(|v| v.borrow_mut().push(Active { policy, entity }));
}

fn pop_active() {
    ACTIVE.with(|v| {
        v.borrow_mut().pop();
    });
}

/// Return the most recently entered policy/entity pair, if any.
pub fn active_policy() -> Option<(&'static Policy, Option<&'static str>)> {
    ACTIVE.with(|v| {
        let borrow = v.borrow();
        borrow.last().map(|a| (a.policy, a.entity))
    })
}

// -----------------------------------------------------------------------------
// Field-map cache (process-wide) built from the distributed slice.
// -----------------------------------------------------------------------------

type FieldMapByConcept = BTreeMap<String, Vec<Vec<String>>>;
type FieldMapCache = BTreeMap<String, FieldMapByConcept>;

fn field_map_cache() -> &'static Mutex<FieldMapCache> {
    static CACHE: OnceLock<Mutex<FieldMapCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn physical_paths_for(entity: &str, concept: &str) -> Vec<Vec<String>> {
    let mut cache = field_map_cache().lock().unwrap();
    let map = cache.entry(entity.to_string()).or_insert_with(|| {
        let mut m: BTreeMap<String, Vec<Vec<String>>> = BTreeMap::new();
        for reg in nirdosha_guard_registry::LOGGING_FIELD_MAPS.iter().filter(|r| r.entity == entity) {
            m.entry(reg.concept.to_string())
                .or_default()
                .push(reg.physical.iter().map(|s| s.to_string()).collect());
        }
        m
    });
    map.get(concept).cloned().unwrap_or_default()
}

// -----------------------------------------------------------------------------
// Event scrubbing
// -----------------------------------------------------------------------------

/// Result of scrubbing an event payload against the active policy.
#[derive(Debug, Clone)]
pub enum ScrubResult {
    Clean(serde_json::Value),
    ForbiddenField { field: String, rule_ids: String },
    MissingRequired { field: String, rule_ids: String },
}

/// Scrub `payload` using the active policy and the optional entity's field
/// map. If no guard is active, returns the payload unchanged (`Clean`).
pub fn scrub(payload: serde_json::Value) -> ScrubResult {
    let Some((policy, entity)) = active_policy() else {
        return ScrubResult::Clean(payload);
    };
    scrub_with(policy, entity, payload)
}

/// Scrub a payload with an explicit policy/entity. Used by tests and by
/// callers that want to scrub outside an active guard.
pub fn scrub_with(
    policy: &Policy,
    entity: Option<&str>,
    mut payload: serde_json::Value,
) -> ScrubResult {
    // 1. Check forbidden fields.
    for concept in policy.forbid_fields {
        let paths = resolve_paths(entity, concept);
        for path in &paths {
            if has_path(&payload, path) {
                return ScrubResult::ForbiddenField {
                    field: path.join("."),
                    rule_ids: policy.rule_ids.to_string(),
                };
            }
        }
    }

    // 2. Apply masks.
    for (concept, transform) in policy.mask_fields {
        let paths = resolve_paths(entity, concept);
        for path in &paths {
            apply_mask(&mut payload, path, transform);
        }
    }

    // 3. Check required fields.
    for concept in policy.require_fields {
        let paths = resolve_paths(entity, concept);
        let found = paths.iter().any(|p| has_path(&payload, p));
        if !found {
            return ScrubResult::MissingRequired {
                field: concept.to_string(),
                rule_ids: policy.rule_ids.to_string(),
            };
        }
    }

    ScrubResult::Clean(payload)
}

fn resolve_paths(entity: Option<&str>, concept: &str) -> Vec<Vec<String>> {
    if let Some(entity) = entity {
        let mapped = physical_paths_for(entity, concept);
        if !mapped.is_empty() {
            return mapped;
        }
    }
    vec![vec![concept.to_string()]]
}

fn has_path(value: &serde_json::Value, path: &[String]) -> bool {
    let mut current = value;
    for segment in path {
        match current.get(segment) {
            Some(v) => current = v,
            None => return false,
        }
    }
    !current.is_null()
}

fn apply_mask(value: &mut serde_json::Value, path: &[String], transform: &str) {
    if path.is_empty() {
        return;
    }
    if path.len() == 1 {
        if let Some(v) = value.get_mut(&path[0]) {
            *v = do_mask(v, transform);
        }
        return;
    }
    let mut current = value;
    for segment in &path[..path.len() - 1] {
        match current.get_mut(segment) {
            Some(serde_json::Value::Object(_)) | Some(serde_json::Value::Array(_)) => current = current.get_mut(segment).unwrap(),
            _ => return,
        }
    }
    if let Some(v) = current.get_mut(&path[path.len() - 1]) {
        *v = do_mask(v, transform);
    }
}

fn do_mask(v: &serde_json::Value, transform: &str) -> serde_json::Value {
    let s = match v {
        serde_json::Value::String(s) => s.as_str(),
        _ => return serde_json::Value::String("[MASKED]".to_string()),
    };
    serde_json::Value::String(mask_string(s, transform))
}

fn mask_string(s: &str, transform: &str) -> String {
    if transform == "redact" || transform.starts_with("hash") || transform.starts_with("scrub") {
        "[REDACTED]".to_string()
    } else if transform.starts_with("partial(") {
        let inner = transform.strip_prefix("partial(").and_then(|t| t.strip_suffix(")")).unwrap_or("");
        let parts: Vec<&str> = inner.split(',').collect();
        if parts.len() == 2 {
            if let (Ok(first), Ok(last)) = (parts[0].parse::<usize>(), parts[1].parse::<usize>()) {
                let len = s.chars().count();
                if first + last >= len || first + last == 0 {
                    return "[REDACTED]".to_string();
                }
                let prefix: String = s.chars().take(first).collect();
                let suffix: String = s.chars().skip(len - last).collect();
                let middle_len = len.saturating_sub(first + last);
                return format!("{}{}{}", prefix, "*".repeat(middle_len), suffix);
            }
        }
        "[REDACTED]".to_string()
    } else if transform.starts_with("truncate(") {
        let inner = transform.strip_prefix("truncate(").and_then(|t| t.strip_suffix(")")).unwrap_or("");
        if let Ok(n) = inner.parse::<usize>() {
            if n == 0 {
                return "".to_string();
            }
            let len = s.chars().count();
            if n >= len {
                return s.to_string();
            }
            return s.chars().take(n).collect();
        }
        "[REDACTED]".to_string()
    } else {
        // tokenize, pseudonymize, generalize, drop-field, aggregate, etc.
        format!("[MASKED:{}]", transform)
    }
}

/// Convenience: serialize and scrub a typed payload against the active
/// policy. Returns `Err` if the payload contains a forbidden field or a
/// required field is missing.
pub fn emit<T: Serialize>(level: Level, event_class: &'static str, payload: &T) -> Result<(), ScrubError> {
    let Some((policy, entity)) = active_policy() else {
        return Ok(());
    };
    if !policy.is_enabled(level) {
        return Ok(());
    }
    let value = serde_json::to_value(payload).map_err(|e| ScrubError::Serialize(e.to_string()))?;
    match scrub_with(policy, entity, value) {
        ScrubResult::Clean(scrubbed) => {
            let event = serde_json::json!({
                "event_class": event_class,
                "level": level_name(level),
                "domain": policy.domain,
                "country": policy.country,
                "payload": scrubbed,
            });
            record_event(&event);
            Ok(())
        }
        ScrubResult::ForbiddenField { field, rule_ids } => Err(ScrubError::ForbiddenField { field, rule_ids }),
        ScrubResult::MissingRequired { field, rule_ids } => Err(ScrubError::MissingRequired { field, rule_ids }),
    }
}

#[derive(Debug, Clone)]
pub enum ScrubError {
    Serialize(String),
    ForbiddenField { field: String, rule_ids: String },
    MissingRequired { field: String, rule_ids: String },
}

fn record_event(event: &serde_json::Value) {
    let line = serde_json::to_string(event).unwrap_or_default();
    if line.is_empty() {
        return;
    }
    if std::env::var_os("NIRDOSHA_NFR_LOG").is_some_and(|v| !v.is_empty()) {
        eprintln!("{line}");
    }
    if let Some(path) = std::env::var_os("NIRDOSHA_NFR_LOG_FILE") {
        if !path.is_empty() {
            use std::io::Write as _;
            if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(file, "{line}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_policy() -> Policy {
        Policy {
            rule_ids: "R-test",
            domain: "payments",
            country: "US",
            level: Level::Info,
            retention_days: 365,
            mask_fields: &[("pan", "partial(6,4)")],
            forbid_fields: &["cvv"],
            require_fields: &["timestamp", "actor"],
            basis: "test",
        }
    }

    #[test]
    fn partial_mask_works() {
        assert_eq!(mask_string("4111111111111111", "partial(6,4)"), "411111******1111");
    }

    #[test]
    fn redact_masks_fully() {
        assert_eq!(mask_string("secret", "redact"), "[REDACTED]");
    }

    #[test]
    fn scrub_detects_forbidden_field() {
        let policy = test_policy();
        let payload = serde_json::json!({ "cvv": "123" });
        match scrub_with(&policy, None, payload) {
            ScrubResult::ForbiddenField { field, .. } => assert_eq!(field, "cvv"),
            other => panic!("expected forbidden, got {other:?}"),
        }
    }

    #[test]
    fn scrub_detects_missing_required_field() {
        let policy = test_policy();
        let payload = serde_json::json!({ "pan": "4111111111111111" });
        match scrub_with(&policy, None, payload) {
            ScrubResult::MissingRequired { field, .. } => assert!(field == "timestamp" || field == "actor"),
            other => panic!("expected missing required, got {other:?}"),
        }
    }

    #[test]
    fn scrub_applies_mask() {
        let policy = test_policy();
        let payload = serde_json::json!({
            "pan": "4111111111111111",
            "timestamp": "t",
            "actor": "svc",
        });
        match scrub_with(&policy, None, payload) {
            ScrubResult::Clean(v) => assert_eq!(v["pan"].as_str(), Some("411111******1111")),
            other => panic!("expected clean, got {other:?}"),
        }
    }
}
