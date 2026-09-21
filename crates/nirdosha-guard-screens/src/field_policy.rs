//! G1/G2 from the RTM live-demo plan: `field_policy` and `requires`
//! conditions never reach `nirdosha-guard-mic`'s live evaluator --
//! `PolicyCandidate` (what `GuardClient::evaluate` actually matches
//! against) has no `field_policy` field at all, and
//! `evaluator::matches_context` never reads `policy.conditions`
//! (confirmed directly against `crates/nirdosha-guard-core/src/
//! evaluator.rs` and `crates/nirdosha-guard-registry/src/lib.rs`'s
//! `to_candidate`/`to_record` split). This module re-checks both against
//! the one thing this crate has that the evaluator doesn't: the
//! decoded, typed row.

use nirdosha_guard_core::{Condition, FieldPolicy, FilterExpr, PatternMatcher, Value};
use nirdosha_guard_registry::PolicyRecord;
use serde_json::Value as Json;
use std::collections::HashSet;

/// Checks a submitted write's field set against `record.field_policy`.
///
/// Semantics (see each corpus policy's own `field_policy { ... }` block
/// for real examples this was derived from, e.g. `20_ingestion.nir`'s
/// `analyst-flag-transaction`): every `required` field must be present;
/// no `forbidden` field may be present; and -- fail-closed, not
/// permissive-by-omission -- once any `allowed`/`required` field is
/// declared at all, a submitted field absent from both `allowed` and
/// `required` is treated as an unlisted-field violation even if it was
/// never named `forbidden` explicitly (a policy that only ever says
/// "these fields, nothing else" shouldn't need to also enumerate every
/// field it *didn't* mean to permit).
pub fn check_field_policy(record: &PolicyRecord, submitted: &HashSet<String>) -> Result<(), Vec<String>> {
    let mut required = Vec::new();
    let mut allowed = HashSet::new();
    let mut forbidden = HashSet::new();
    for fp in &record.field_policy {
        match fp {
            FieldPolicy::Required(path) => {
                if let [name] = path.as_slice() {
                    required.push(name.clone());
                    allowed.insert(name.clone());
                }
            }
            FieldPolicy::Allowed(path) => {
                if let [name] = path.as_slice() {
                    allowed.insert(name.clone());
                }
            }
            FieldPolicy::Forbidden(path) => {
                if let [name] = path.as_slice() {
                    forbidden.insert(name.clone());
                }
            }
            FieldPolicy::Default(..) => {}
        }
    }

    let mut violations = Vec::new();
    for name in &required {
        if !submitted.contains(name) {
            violations.push(format!("missing required field `{name}`"));
        }
    }
    for name in submitted {
        if forbidden.contains(name) {
            violations.push(format!("field `{name}` is forbidden by policy `{}`", record.id));
        }
    }
    if !allowed.is_empty() {
        for name in submitted {
            if !allowed.contains(name) && !forbidden.contains(name) {
                violations.push(format!("field `{name}` is not in policy `{}`'s allowed set", record.id));
            }
        }
    }
    if violations.is_empty() { Ok(()) } else { Err(violations) }
}

/// Checks `record.conditions` against a decoded row's own JSON fields.
///
/// Only `Condition::Expr` (a literal `field(x) == "y"` / `in [...]` /
/// comparison -- the shapes `clauses.rs::lower_condition` actually turns
/// into a `FilterExpr`) is evaluated here. `Condition::Custom` (real
/// `invariant(name)` references and `field(x).transition_allowed()`,
/// per `clauses.rs`'s own doc comment) is handled separately by
/// [`check_custom_conditions`], which needs the typed entity this
/// JSON-generic function doesn't have.
pub fn check_conditions(record: &PolicyRecord, row: &Json) -> Result<(), String> {
    for condition in &record.conditions {
        if let Condition::Expr(filter) = condition {
            if !eval_filter_expr(filter, row) {
                return Err(format!("condition on policy `{}` not satisfied: {filter:?}", record.id));
            }
        }
        // `sar-export`-shaped real gap, found live by this batch's own
        // test: `requires field(status) == "confirmed_fraud"` does NOT
        // lower to `Condition::Expr` the way `clauses.rs`'s own doc
        // comment says a bare `field(x) == "lit"` conjunct should
        // (verified directly: this exact clause lowers to
        // `Condition::Custom(InvariantId("field ( status ) = = ..."))`
        // instead, at least for this corpus's real macro invocations --
        // a `nirdosha-guard-registry` parser question out of this
        // crate's own scope to fix, not something to route around
        // silently). `raw_literal_eq` below recovers the SAME real
        // `field(x)=="y"` semantics generically from that raw text,
        // rather than leaving this condition either silently ignored
        // (a scan/export would leak every row) or perpetually fail-closed
        // (`check_custom_conditions`'s own `check_invariant` fallback
        // would otherwise deny every write matching this shape,
        // regardless of the row's real state).
        if let Condition::Custom(raw) = condition {
            if let Some((field, value)) = raw_literal_eq(&raw.0) {
                let actual = row.get(&field).and_then(Json::as_str);
                if actual != Some(value.as_str()) {
                    return Err(format!("condition on policy `{}` not satisfied: {field} == \"{value}\" (was {actual:?})", record.id));
                }
            }
        }
    }
    Ok(())
}

/// Recovers `(field, literal)` from a `requires field(x) == "y"`
/// conjunct's raw, `clauses.rs`-preserved source text (token-by-token,
/// so `==` prints as two separate `=` tokens joined by a space -- see
/// this fn's one caller for the full context). Returns `None` for
/// anything else (`.transition_allowed()`, a real `invariant(name)`
/// reference, a non-literal RHS) so those keep going through this
/// crate's existing, separate dispatch paths unaffected.
fn raw_literal_eq(raw: &str) -> Option<(String, String)> {
    let compact: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    let rest = compact.strip_prefix("field(")?;
    let (field, rest) = rest.split_once(")==\"")?;
    let value = rest.strip_suffix('"')?;
    Some((field.to_string(), value.to_string()))
}

/// The `Condition::Custom` half of `record.conditions` -- dispatched
/// through [`crate::entity::GuardedEntity`]'s own per-dataset overrides,
/// the only way this crate can reach a real `00_core.nir` invariant fn
/// or `workflow!`-generated transition check without depending on the
/// app crate that declares them. Still fails closed, with a clear
/// reason naming the Phase B gap, for any name the entity's own override
/// doesn't recognize.
///
/// `before`/`after` are the same row on either side of the write this
/// condition gates (equal to each other for a create, where there is no
/// "before"). `requested_status` is `after`'s own encoded value for
/// whichever field a `field(x).transition_allowed()` clause names (in
/// this corpus, always `status` -- confirmed against every occurrence in
/// `examples/rtm/src/*.nir`) -- reading it generically here, rather than
/// hardcoding a field name, would need a `Condition::Custom` shape this
/// crate doesn't have (the field name is only ever present in the raw,
/// unparsed source text `clauses.rs` preserved); requiring the caller to
/// pass the already-known submitted value keeps this function honest
/// about what it doesn't parse out of that raw text.
pub fn check_custom_conditions<E: crate::entity::GuardedEntity>(record: &PolicyRecord, before: &E, after: &E, requested_status: &Json) -> Result<(), String> {
    for condition in &record.conditions {
        let Condition::Custom(invariant) = condition else { continue };
        let raw = invariant.0.as_str();
        if raw.contains("transition_allowed") {
            match before.transition_allowed(requested_status) {
                Some(true) => continue,
                Some(false) => return Err(format!("policy `{}` requires a legal status transition, and the submitted value is not reachable", record.id)),
                None => return Err(format!("policy `{}` requires a status-transition check `transition_allowed` on this dataset hasn't wired (Phase B item)", record.id)),
            }
        }
        match after.check_invariant(raw) {
            Some(true) => continue,
            Some(false) => return Err(format!("policy `{}` requires invariant `{raw}`, which the submitted row fails", record.id)),
            None => return Err(format!("policy `{}` requires invariant `{raw}`, which this dataset hasn't wired (Phase B item)", record.id)),
        }
    }
    Ok(())
}

/// Real read-side counterpart to [`check_field_policy`] -- until this
/// fix, `field_policy { forbidden(...) }` on a READ policy (RTM's
/// `cs-payment-status`/`rm-restriction-view`, `96_restricted_views.nir`)
/// was never enforced at all: only `mask(...)`'s own `FieldMask`s reached
/// [`crate::masking::apply_masks`], so a policy expressing "this reader
/// may only see X" via `field_policy` rather than `mask(...)` returned
/// every field in the clear regardless of `forbidden(...)` naming it.
/// Synthesizes a `Full` [`nirdosha_guard_core::FieldMask`] per forbidden
/// field, reusing the exact same masking pipeline `mask(...)`-granted
/// fields already go through -- not a second, parallel redaction path a
/// screen would need to reason about differently. Scoped to fields the
/// policy names `forbidden` explicitly (both real corpus policies this
/// was built for enumerate every non-allowed field exhaustively) --
/// generalizing to "mask every field not in an `allowed` set" would need
/// each dataset's own full field list, which this JSON-generic module
/// doesn't have; a real, smaller-scope limit, not silently assumed away.
/// `exempt` is the same [`crate::entity::GuardedEntity::field_policy_exempt`]
/// list the write-side check already excludes, so a row's own synthetic
/// `id`/row-identity fields are never masked out from under a screen that
/// needs them to render.
pub fn read_masks_from_field_policy(record: &PolicyRecord, exempt: &[&str]) -> Vec<nirdosha_guard_core::FieldMask> {
    record
        .field_policy
        .iter()
        .filter_map(|fp| match fp {
            FieldPolicy::Forbidden(path) => match path.as_slice() {
                [name] if !exempt.contains(&name.as_str()) => Some(nirdosha_guard_core::FieldMask { field: vec![name.clone()], transform: nirdosha_guard_core::MaskTransform::Full }),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn json_field<'a>(row: &'a Json, field: &[String]) -> Option<&'a Json> {
    let mut current = row;
    for segment in field {
        current = current.get(segment)?;
    }
    Some(current)
}

fn value_matches(expected: &Value, actual: &Json) -> bool {
    match expected {
        Value::Str(s) => actual.as_str() == Some(s.as_str()),
        Value::Int(n) => actual.as_i64() == Some(*n),
        Value::Bool(b) => actual.as_bool() == Some(*b),
        Value::Dec(s) => actual.as_f64().is_some_and(|f| f.to_string() == *s) || actual.as_str() == Some(s.as_str()),
        Value::Null => actual.is_null(),
    }
}

fn pattern_matches(matcher: &PatternMatcher, actual: &str) -> bool {
    match matcher {
        PatternMatcher::Exact(v) => v == actual,
        PatternMatcher::Prefix(v) => actual.starts_with(v.as_str()),
        PatternMatcher::Glob(_) => false, // not needed against row-internal fields today
    }
}

fn eval_filter_expr(filter: &FilterExpr, row: &Json) -> bool {
    match filter {
        FilterExpr::Eq { field, value } => json_field(row, field).is_some_and(|actual| value_matches(value, actual)),
        FilterExpr::In { field, values } => json_field(row, field).is_some_and(|actual| values.iter().any(|v| value_matches(v, actual))),
        FilterExpr::Pattern { field, matcher } => json_field(row, field).and_then(Json::as_str).is_some_and(|actual| pattern_matches(matcher, actual)),
        FilterExpr::And(children) => children.iter().all(|c| eval_filter_expr(c, row)),
        FilterExpr::Or(children) => children.iter().any(|c| eval_filter_expr(c, row)),
        FilterExpr::Not(inner) => !eval_filter_expr(inner, row),
        // Compare needs a typed ordering this JSON-generic evaluator
        // doesn't attempt (same "excluded, not guessed at" posture
        // `nirdosha-guard-mic::match_filter` takes for its own unsupported
        // variants); TenantEq/TimeRange/RelationIn aren't meaningful
        // against a single decoded row's own fields.
        FilterExpr::Compare { .. } | FilterExpr::TenantEq { .. } | FilterExpr::TimeRange { .. } | FilterExpr::RelationIn { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nirdosha_guard_core::InvariantId;

    fn record_with(field_policy: Vec<FieldPolicy>, conditions: Vec<Condition>) -> PolicyRecord {
        PolicyRecord {
            id: "test-policy".into(), effect: nirdosha_guard_registry::Effect::Allow, subjects: vec![], action: "update".into(),
            resource: "widget".into(), purpose: None, caps: vec![], affected_row_cap: None, obligations: vec![], escalation: None,
            field_policy, conditions, filter: None, filter_ref: None, masks: vec![], reason: None, destination: None,
            destination_denied_above: None, grants: vec![], predicate_use: vec![], count_allowed: false, policy_src: String::new(), line: 0,
        }
    }

    #[test]
    fn forbidden_field_present_is_a_violation() {
        let record = record_with(vec![FieldPolicy::Forbidden(vec!["tenant_id".into()])], vec![]);
        let submitted: HashSet<String> = ["tenant_id".into()].into();
        assert!(check_field_policy(&record, &submitted).is_err());
    }

    #[test]
    fn missing_required_field_is_a_violation() {
        let record = record_with(vec![FieldPolicy::Required(vec!["rationale".into()])], vec![]);
        assert!(check_field_policy(&record, &HashSet::new()).is_err());
    }

    #[test]
    fn field_outside_allowed_set_is_a_violation_even_if_not_named_forbidden() {
        let record = record_with(vec![FieldPolicy::Allowed(vec!["status".into()])], vec![]);
        let submitted: HashSet<String> = ["status".into(), "amount".into()].into();
        let result = check_field_policy(&record, &submitted);
        assert!(result.is_err());
        assert!(result.unwrap_err().iter().any(|v| v.contains("amount")));
    }

    #[test]
    fn allowed_and_required_fields_pass() {
        let record = record_with(vec![FieldPolicy::Required(vec!["status".into()]), FieldPolicy::Allowed(vec!["rationale".into()])], vec![]);
        let submitted: HashSet<String> = ["status".into(), "rationale".into()].into();
        assert!(check_field_policy(&record, &submitted).is_ok());
    }

    #[test]
    fn literal_field_equals_condition_is_checked_against_the_row() {
        let record = record_with(vec![], vec![Condition::Expr(FilterExpr::Eq { field: vec!["status".into()], value: Value::Str("draft".into()) })]);
        assert!(check_conditions(&record, &serde_json::json!({ "status": "draft" })).is_ok());
        assert!(check_conditions(&record, &serde_json::json!({ "status": "filed" })).is_err());
    }

    /// `check_conditions` (the JSON-generic, `Condition::Expr`-only half)
    /// silently skips `Condition::Custom` now -- `check_custom_conditions`
    /// below is where that half is actually checked. This is the
    /// counterpart the removed `custom_invariant_condition_fails_closed_
    /// with_a_clear_reason` test used to cover against `check_conditions`
    /// itself, before real dispatch existed.
    #[test]
    fn check_conditions_ignores_custom_conditions() {
        let record = record_with(vec![], vec![Condition::Custom(InvariantId("amount_positive".into()))]);
        assert!(check_conditions(&record, &serde_json::json!({})).is_ok());
    }

    #[derive(Clone, serde::Serialize, serde::Deserialize)]
    struct FakeRow {
        amount: f64,
        status: String,
    }

    impl crate::entity::GuardedEntity for FakeRow {
        const RESOURCE: &'static str = "fake_row";
        fn row_id(&self) -> String {
            self.status.clone()
        }
        fn check_invariant(&self, name: &str) -> Option<bool> {
            match name {
                "amount_positive" => Some(self.amount > 0.0),
                _ => None,
            }
        }
        fn transition_allowed(&self, requested: &Json) -> Option<bool> {
            let requested = requested.as_str()?;
            Some(matches!((self.status.as_str(), requested), ("draft", "in_review") | ("in_review", "filed")))
        }
    }

    #[test]
    fn wired_invariant_condition_dispatches_through_the_entity() {
        let record = record_with(vec![], vec![Condition::Custom(InvariantId("amount_positive".into()))]);
        let bad = FakeRow { amount: -1.0, status: "draft".into() };
        let good = FakeRow { amount: 1.0, status: "draft".into() };
        assert!(check_custom_conditions(&record, &bad, &bad, &Json::Null).is_err());
        assert!(check_custom_conditions(&record, &good, &good, &Json::Null).is_ok());
    }

    #[test]
    fn unwired_invariant_name_fails_closed_with_a_clear_reason() {
        let record = record_with(vec![], vec![Condition::Custom(InvariantId("no_legal_hold".into()))]);
        let row = FakeRow { amount: 1.0, status: "draft".into() };
        let error = check_custom_conditions(&record, &row, &row, &Json::Null).unwrap_err();
        assert!(error.contains("no_legal_hold"));
        assert!(error.contains("Phase B"));
    }

    #[test]
    fn wired_transition_condition_checks_before_state_against_requested_value() {
        let record = record_with(vec![], vec![Condition::Custom(InvariantId("field ( status ) . transition_allowed ( )".into()))]);
        let before = FakeRow { amount: 1.0, status: "draft".into() };
        assert!(check_custom_conditions(&record, &before, &before, &Json::String("in_review".into())).is_ok());
        assert!(check_custom_conditions(&record, &before, &before, &Json::String("filed".into())).is_err());
    }
}
