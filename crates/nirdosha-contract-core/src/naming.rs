//! Shared snake_case validation and PascalCase conversion — the
//! mechanical, total wire-string → Rust-ident mapping `role.rs`
//! (`requires(role = "..")`, one component) and `claim.rs`
//! (`requires(claim = "..", "..")`, two components combined) both
//! build on. Lying about a component (using a name/value with no
//! matching declared type) is a build error in both compilers, because
//! the mapping is mechanical, not looked up — see `role.rs`'s own doc
//! comment for the full reasoning, unchanged here.

pub fn validate_snake_case(value: &str, what: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{what} is empty"));
    }
    for seg in value.split('_') {
        let mut chars = seg.chars();
        match chars.next() {
            Some(first) if first.is_ascii_lowercase() => {}
            _ => return Err(format!("{what} `{value}` must be lowercase snake_case (e.g. \"hr_staff\")")),
        }
        if !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) {
            return Err(format!("{what} `{value}` must be lowercase snake_case (e.g. \"hr_staff\")"));
        }
    }
    Ok(())
}

/// `"hr_staff"` → `"HrStaff"`.
pub fn to_pascal_case(value: &str) -> String {
    value
        .split('_')
        .map(|seg| {
            let mut chars = seg.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snake_to_pascal() {
        assert_eq!(to_pascal_case("hr_staff"), "HrStaff");
        assert_eq!(to_pascal_case("security_ops_level2"), "SecurityOpsLevel2");
    }

    #[test]
    fn rejects_non_snake_case() {
        for bad in ["", "HrStaff", "hr staff", "hr__staff", "hr-Staff", "_hr"] {
            assert!(validate_snake_case(bad, "name").is_err(), "`{bad}` should be rejected");
        }
    }
}
