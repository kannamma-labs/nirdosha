//! G3 (a `filter <scope-fn>()` clause never resolves to a concrete
//! `FilterExpr` -- `PolicyRecord.filter_ref` carries the literal source
//! text, `PolicyRegistration::to_candidate()` never reads it, confirmed
//! against `crates/nirdosha-guard-registry/src/clauses.rs`) and
//! G4/G5's resource-prefix trick (letting `MemStoreDriver`/
//! `PostgresStoreDriver`'s one-row-per-`resource` schema hold more than
//! one row per entity *type*, by keying storage as `"<type>:<row-id>"`
//! and filtering reads with a `Pattern` prefix on the `resource` field
//! itself -- see `nirdosha-guard-mic::match_filter`'s own doc comment
//! confirming `field: ["resource"]` is real, supported pushdown).

use nirdosha_guard_core::{FilterExpr, PatternMatcher, Value};

/// Resolves a `filter_ref` clause name into a concrete `FilterExpr`,
/// against the context this crate has that `to_candidate()` doesn't: a
/// live request's own tenant/subject.
///
/// - `tenant_scope()` always resolves.
/// - `subject_scope()` resolves when the caller supplies
///   `subject_field` (from [`crate::entity::GuardedEntity::
///   subject_scope_field`] -- different per dataset, e.g. `user_id` on
///   `user_profile`; `None` when a dataset hasn't declared one, e.g.
///   `qa_review`'s current schema has no such field at all).
/// - `delegation_scope()` stays unresolved: `nirdosha-guard-core::
///   delegation` has a real `DelegationRegistry::mint`/`authorize`, but
///   nothing threads a delegation-credential id through a live HTTP
///   request/`Auth` yet (no session/header carries one) -- resolving
///   this would mean guessing at plumbing that doesn't exist, not
///   reusing plumbing that does.
/// - `time_range(field, within, retention_window())` also stays
///   unresolved, for a different reason: `MemStoreDriver::match_filter`'s
///   own `FilterExpr::TimeRange` arm always returns `false` (confirmed
///   directly against that match arm) -- resolving the clause text into
///   a real `TimeRange` filter would make every read against it come
///   back silently empty, which is worse than today's explicit
///   fail-closed error. Wiring this needs that driver-level gap closed
///   first, not a workaround here.
///
/// Every unresolved case returns `None`, `Err(GuardScreenError)`'d by
/// the caller (fail closed, matching `MemStoreDriver::query`'s own
/// "refuse an unscoped scan" posture) rather than silently proceeding
/// without the scope the policy asked for.
pub fn resolve_filter_ref(filter_ref: &str, tenant: &str, subject_id: &str, subject_field: Option<&str>) -> Option<FilterExpr> {
    match filter_ref.trim() {
        "tenant_scope()" => Some(FilterExpr::TenantEq { value: Value::Str(tenant.to_string()) }),
        "subject_scope()" => {
            let field = subject_field?;
            Some(FilterExpr::Eq { field: vec![field.to_string()], value: Value::Str(subject_id.to_string()) })
        }
        _ => None,
    }
}

/// The G4/G5 storage key this row occupies: `"<RESOURCE>:<row_id>"`,
/// never the bare `RESOURCE` string alone (which is reserved for policy
/// *matching*, via `EvaluationContext.entity` — see `GuardedTable`'s
/// module doc comment for why the two identities can't be the same
/// value in a multi-row schema).
pub fn storage_key(resource: &str, row_id: &str) -> String {
    format!("{resource}:{row_id}")
}

/// The read-side counterpart: every row whose storage key starts with
/// `"<resource>:"` -- the type discriminator this driver's flat
/// `resource -> payload` schema is otherwise missing.
pub fn resource_prefix_filter(resource: &str) -> FilterExpr {
    FilterExpr::Pattern { field: vec!["resource".into()], matcher: PatternMatcher::Prefix(format!("{resource}:")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tenant_scope_resolves_to_a_tenant_eq_filter() {
        assert_eq!(resolve_filter_ref("tenant_scope()", "tenant-rtm", "u-1", None), Some(FilterExpr::TenantEq { value: Value::Str("tenant-rtm".into()) }));
    }

    #[test]
    fn subject_scope_resolves_when_the_dataset_declares_its_own_subject_field() {
        assert_eq!(
            resolve_filter_ref("subject_scope()", "tenant-rtm", "u-1", Some("user_id")),
            Some(FilterExpr::Eq { field: vec!["user_id".into()], value: Value::Str("u-1".into()) })
        );
    }

    #[test]
    fn subject_scope_fails_closed_when_the_dataset_has_no_subject_field() {
        assert_eq!(resolve_filter_ref("subject_scope()", "tenant-rtm", "u-1", None), None);
    }

    #[test]
    fn unresolved_scope_functions_return_none_not_a_guess() {
        assert_eq!(resolve_filter_ref("delegation_scope()", "tenant-rtm", "u-1", None), None);
        assert_eq!(resolve_filter_ref("time_range(created_at, within, retention_window())", "tenant-rtm", "u-1", None), None);
    }

    #[test]
    fn storage_key_and_prefix_filter_agree() {
        let key = storage_key("transaction", "txn-001");
        let FilterExpr::Pattern { matcher: PatternMatcher::Prefix(prefix), .. } = resource_prefix_filter("transaction") else { panic!("expected Pattern/Prefix") };
        assert!(key.starts_with(&prefix));
    }
}
