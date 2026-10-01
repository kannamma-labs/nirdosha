//! Compile-time-checked compliance policies — "this domain forbids
//! delete" as a `rustc` guarantee, not a doc comment a scanner might
//! or might not have been run against.
//!
//! The design goal, same one `role.rs` states for `RoleProof`: the
//! guarantee must be real `rustc`, not tooling. A policy declares a
//! `const` list of forbidden CRUD operations; `#[nirdosha_rt::contract(
//! crud_op = "delete", policy = "financial_us")]` on a function emits a
//! `const _: () = assert!(..)` referencing that list. `rustc`'s own
//! const-evaluator — not a separate scanner — refuses the build if the
//! named policy forbids the named operation. No unstable features: a
//! `const fn` reading an associated `const` through a generic bound,
//! and a stable-since-1.57 `const`-context `panic!`/`assert!`.

/// A compliance policy in the application's vocabulary. Declared via
/// [`crate::policy!`].
pub trait Policy {
    const NAME: &'static str;
    /// CRUD operation names (`"create"`/`"read"`/`"update"`/`"delete"`)
    /// this policy forbids outright.
    const FORBIDDEN_OPS: &'static [&'static str];
    /// Data-protection concerns (`"at_rest"`/`"in_transit"`) this
    /// policy requires encryption for. Declared for forward
    /// reference — nothing in this crate enforces it yet (no crypto,
    /// no TLS primitive exists in `nirdosha-rt` today); see RFC 0020.
    const ENCRYPTED_CONCERNS: &'static [&'static str];
}

const fn str_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

const fn contains(haystack: &[&str], needle: &str) -> bool {
    let mut i = 0;
    while i < haystack.len() {
        if str_eq(haystack[i], needle) {
            return true;
        }
        i += 1;
    }
    false
}

/// Whether policy `P` forbids operation `op` (e.g. `"delete"`).
///
/// A free `const fn`, not a trait method: reading `P::FORBIDDEN_OPS`
/// through the generic bound is an ordinary associated-const lookup
/// (stable, no `const_trait_impl` needed) — only trait *methods*, not
/// trait *constants*, require that unstable feature to be callable in
/// a `const` context.
pub const fn crud_forbidden<P: Policy>(op: &str) -> bool {
    contains(P::FORBIDDEN_OPS, op)
}

/// Whether policy `P` requires encryption for `concern` (`"at_rest"`
/// or `"in_transit"`). See [`Policy::ENCRYPTED_CONCERNS`]'s doc: not
/// enforced by anything in this crate yet.
pub const fn requires_encryption<P: Policy>(concern: &str) -> bool {
    contains(P::ENCRYPTED_CONCERNS, concern)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FinancialUs;
    impl Policy for FinancialUs {
        const NAME: &'static str = "financial_us";
        const FORBIDDEN_OPS: &'static [&'static str] = &["delete"];
        const ENCRYPTED_CONCERNS: &'static [&'static str] = &["at_rest", "in_transit"];
    }

    struct NoRestrictions;
    impl Policy for NoRestrictions {
        const NAME: &'static str = "no_restrictions";
        const FORBIDDEN_OPS: &'static [&'static str] = &[];
        const ENCRYPTED_CONCERNS: &'static [&'static str] = &[];
    }

    // Compiles at all only if these are genuinely const-evaluable.
    const FINANCIAL_FORBIDS_DELETE: bool = crud_forbidden::<FinancialUs>("delete");
    const FINANCIAL_FORBIDS_CREATE: bool = crud_forbidden::<FinancialUs>("create");
    const _: () = assert!(FINANCIAL_FORBIDS_DELETE);
    const _: () = assert!(!FINANCIAL_FORBIDS_CREATE);

    #[test]
    fn forbidden_ops_are_checked_by_name() {
        assert!(crud_forbidden::<FinancialUs>("delete"));
        assert!(!crud_forbidden::<FinancialUs>("create"));
        assert!(!crud_forbidden::<FinancialUs>("read"));
        assert!(!crud_forbidden::<NoRestrictions>("delete"));
    }

    #[test]
    fn encrypted_concerns_are_checked_by_name() {
        assert!(requires_encryption::<FinancialUs>("at_rest"));
        assert!(requires_encryption::<FinancialUs>("in_transit"));
        assert!(!requires_encryption::<NoRestrictions>("at_rest"));
    }
}
