//! Role-name validation and the role-name → type-ident convention.
//!
//! `requires(role = "hr_staff")` resolves to the type
//! `crate::nirdosha_roles::HrStaff`, declared by the `nirdosha_rt::roles!`
//! macro. The mapping is total and mechanical: lowercase snake_case role
//! names become CamelCase type names. A role with no declared type fails
//! to *compile* — the proof token parameter the contract macro injects
//! simply does not resolve — so lying about a role name is also a build
//! error, in both compilers.
//!
//! [`roles!`]'s `role Name;`/`principal Name = "...";` forms (used by
//! catalog-style apps, e.g. RTM's `00_core.nir`, to match `guard_policy!`
//! subject strings verbatim) give the wire name and the type name the
//! *same* bare PascalCase spelling — there is no snake_case form to
//! convert. [`role_ident`] handles both conventions: snake_case first
//! (unchanged from the original mapping — every existing valid
//! snake_case name still resolves exactly as before), falling back to
//! accepting the name as-is when it's already a bare PascalCase
//! identifier. [`validate_role_name`] itself stays snake_case-only — it
//! is also used standalone (see `model.rs`) where the PascalCase
//! fallback does not apply.

use proc_macro2::Span;

pub fn validate_role_name(name: &str) -> Result<(), String> {
    crate::naming::validate_snake_case(name, "role name")
}

/// A bare PascalCase Rust identifier: starts uppercase, every remaining
/// character alphanumeric. Deliberately stricter than "any valid ident"
/// (no leading underscore, no all-lowercase/all-caps) — this is meant to
/// recognize exactly the [`roles!`] `role Name;` shape, not to become a
/// second, looser way to spell a snake_case name.
fn is_bare_pascal_case(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_uppercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric())
}

/// `"hr_staff"` → ident `HrStaff`. `"Analyst"` (already bare PascalCase,
/// e.g. from `nirdosha_rt::roles! { role Analyst; }`) → ident `Analyst`,
/// unconverted.
pub fn role_ident(name: &str, span: Span) -> syn::Result<proc_macro2::Ident> {
    if validate_role_name(name).is_ok() {
        return Ok(proc_macro2::Ident::new(&crate::naming::to_pascal_case(name), span));
    }
    if is_bare_pascal_case(name) {
        return syn::parse_str::<proc_macro2::Ident>(name)
            .map_err(|_| syn::Error::new(span, format!("invalid role name: `{name}` is not a valid Rust identifier")));
    }
    Err(syn::Error::new(
        span,
        format!(
            "invalid role name: `{name}` must be lowercase snake_case (e.g. \"hr_staff\") \
             or a bare PascalCase role type name (e.g. \"Analyst\", matching `nirdosha_rt::roles! {{ role Analyst; }}`)"
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snake_to_camel() {
        let id = role_ident("hr_staff", Span::call_site()).unwrap();
        assert_eq!(id.to_string(), "HrStaff");
        let id = role_ident("manager", Span::call_site()).unwrap();
        assert_eq!(id.to_string(), "Manager");
        let id = role_ident("security_ops_level2", Span::call_site()).unwrap();
        assert_eq!(id.to_string(), "SecurityOpsLevel2");
    }

    #[test]
    fn rejects_non_snake() {
        for bad in ["", "HrStaff", "hr staff", "hr__staff", "hr-Staff", "_hr"] {
            assert!(validate_role_name(bad).is_err(), "`{bad}` should be rejected");
        }
    }

    /// `role_ident` (not `validate_role_name`, which stays snake_case-only
    /// per the test above) accepts a bare PascalCase name unconverted —
    /// the `roles! { role Analyst; }` convention.
    #[test]
    fn accepts_bare_pascal_case_role_type_names() {
        let id = role_ident("Analyst", Span::call_site()).unwrap();
        assert_eq!(id.to_string(), "Analyst");
        let id = role_ident("ComplianceLead", Span::call_site()).unwrap();
        assert_eq!(id.to_string(), "ComplianceLead");
    }

    #[test]
    fn still_rejects_junk_that_is_neither_snake_nor_pascal() {
        for bad in ["", "hr staff", "hr__staff", "hr-Staff", "_hr", "1Bad"] {
            assert!(role_ident(bad, Span::call_site()).is_err(), "`{bad}` should be rejected");
        }
    }
}