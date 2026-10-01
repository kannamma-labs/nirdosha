//! Shared support for `requires(expr)`/`ensures(expr)` boolean
//! predicates (issue #68): a small, deliberately restricted expression
//! grammar that both the attribute macro (structural validation at parse
//! time, before a param/type even exists to check against) and
//! `nirdosha-driver` (MIR-local resolution and Z3 discharge at Stage 2)
//! check against — one definition of "supported", not two that can
//! drift apart.
//!
//! Grammar: identifiers, integer/bool literals, arithmetic (`+ - * / %`),
//! comparisons (`== != < <= > >=`), boolean combinators (`&& ||`), unary
//! `-`/`!`, and parens. No field/index/method access, no calls — those
//! need a heavier VC encoding than item 1's shared integer semantics
//! provide today.

use syn::Expr;

/// Parse a predicate source string (as stored in the doc-encoded
/// contract) and check it uses only the supported expression subset.
/// Does *not* resolve identifiers or types — that needs the MIR body a
/// predicate is checked against, only available to `nirdosha-driver`.
pub fn parse_predicate(src: &str) -> Result<Expr, String> {
    let expr: Expr = syn::parse_str(src).map_err(|e| e.to_string())?;
    check_predicate_shape(&expr)?;
    Ok(expr)
}

pub fn check_predicate_shape(expr: &Expr) -> Result<(), String> {
    match expr {
        Expr::Paren(e) => check_predicate_shape(&e.expr),
        Expr::Group(e) => check_predicate_shape(&e.expr),
        Expr::Path(p) if p.path.get_ident().is_some() => Ok(()),
        Expr::Lit(l) => match &l.lit {
            syn::Lit::Int(_) | syn::Lit::Bool(_) => Ok(()),
            _ => Err("requires/ensures literals must be integers or booleans".into()),
        },
        Expr::Unary(u) => match u.op {
            syn::UnOp::Neg(_) | syn::UnOp::Not(_) => check_predicate_shape(&u.expr),
            _ => Err("unsupported unary operator in requires/ensures".into()),
        },
        Expr::Binary(b) => {
            use syn::BinOp::*;
            match b.op {
                Add(_) | Sub(_) | Mul(_) | Div(_) | Rem(_) | Eq(_) | Ne(_) | Lt(_) | Le(_)
                | Gt(_) | Ge(_) | And(_) | Or(_) => {
                    check_predicate_shape(&b.left)?;
                    check_predicate_shape(&b.right)
                }
                _ => Err(
                    "unsupported operator in requires/ensures — only + - * / %, \
                     comparisons (== != < <= > >=), && and || are supported"
                        .into(),
                ),
            }
        }
        _ => Err(
            "unsupported expression in requires/ensures — only identifiers, integer/bool \
             literals, arithmetic, comparisons, &&, ||, unary -/!, and parens are supported"
                .into(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_the_supported_subset() {
        for src in [
            "idx < len",
            "result >= 0",
            "a + b == result",
            "!(x == 0) && y > 1",
            "-x <= 5",
            "true",
        ] {
            parse_predicate(src).unwrap_or_else(|e| panic!("{src}: {e}"));
        }
    }

    #[test]
    fn rejects_field_and_method_access() {
        assert!(parse_predicate("arr.len() > 0").is_err());
        assert!(parse_predicate("point.x > 0").is_err());
    }

    #[test]
    fn rejects_malformed_syntax() {
        assert!(parse_predicate("idx <").is_err());
    }
}
