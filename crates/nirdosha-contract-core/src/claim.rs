//! Claim-name/value validation and the `(name, value)` → type-ident
//! convention — the `requires(claim = "department", "cardiology")`
//! sibling to `role.rs`'s own `requires(role = "..")` mapping.
//! Mechanical and total the same way: `PascalCase(name) ++
//! PascalCase(value)`, so a claim declared in `nirdosha_rt::claims!`
//! under any other ident fails to resolve — lying about either
//! component is a build error, in both compilers, the same guarantee
//! `role.rs` already gives a single role name.

use proc_macro2::Span;

pub fn validate_claim_name(name: &str) -> Result<(), String> {
    crate::naming::validate_snake_case(name, "claim name")
}

pub fn validate_claim_value(value: &str) -> Result<(), String> {
    crate::naming::validate_snake_case(value, "claim value")
}

/// `("department", "cardiology")` → ident `DepartmentCardiology`.
pub fn claim_ident(name: &str, value: &str, span: Span) -> syn::Result<proc_macro2::Ident> {
    validate_claim_name(name).map_err(|msg| syn::Error::new(span, format!("invalid claim name: {msg}")))?;
    validate_claim_value(value).map_err(|msg| syn::Error::new(span, format!("invalid claim value: {msg}")))?;
    let ident = format!("{}{}", crate::naming::to_pascal_case(name), crate::naming::to_pascal_case(value));
    Ok(proc_macro2::Ident::new(&ident, span))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_and_value_combine_to_one_pascal_case_ident() {
        let id = claim_ident("department", "cardiology", Span::call_site()).unwrap();
        assert_eq!(id.to_string(), "DepartmentCardiology");
    }

    #[test]
    fn rejects_non_snake_case_either_component() {
        assert!(claim_ident("Department", "cardiology", Span::call_site()).is_err());
        assert!(claim_ident("department", "Cardiology", Span::call_site()).is_err());
        assert!(claim_ident("", "cardiology", Span::call_site()).is_err());
        assert!(claim_ident("department", "", Span::call_site()).is_err());
    }
}
