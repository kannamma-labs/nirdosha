//! Role-name validation and the role-name → type-ident convention.
//!
//! `requires(role = "hr_staff")` resolves to the type
//! `crate::nirdosha_roles::HrStaff`, declared by the `nirdosha_rt::roles!`
//! macro. The mapping is total and mechanical: lowercase snake_case role
//! names become CamelCase type names. A role with no declared type fails
//! to *compile* — the proof token parameter the contract macro injects
//! simply does not resolve — so lying about a role name is also a build
//! error, in both compilers.

use proc_macro2::Span;

pub fn validate_role_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("role name is empty".into());
    }
    for seg in name.split('_') {
        let mut chars = seg.chars();
        match chars.next() {
            Some(first) if first.is_ascii_lowercase() => {}
            _ => {
                return Err(format!(
                    "role name `{name}` must be lowercase snake_case (e.g. \"hr_staff\")"
                ))
            }
        }
        if !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) {
            return Err(format!(
                "role name `{name}` must be lowercase snake_case (e.g. \"hr_staff\")"
            ));
        }
    }
    Ok(())
}

/// `"hr_staff"` → ident `HrStaff`.
pub fn role_ident(name: &str, span: Span) -> syn::Result<proc_macro2::Ident> {
    validate_role_name(name).map_err(|msg| syn::Error::new(span, format!("invalid role name: {msg}")))?;
    let camel: String = name
        .split('_')
        .map(|seg| {
            let mut chars = seg.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect();
    Ok(proc_macro2::Ident::new(&camel, span))
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
}