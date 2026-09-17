//! Parser for the `#[contract(...)]` attribute token form.
//!
//! Grammar (Stage 1):
//!
//! ```text
//! contract   := clause*
//! clause     := effects "(" ident ("," ident)* ")"
//!             | requires "(" "role" "=" string ")"
//!             | requires "(" "claim" "=" string "," string ")"
//!             | requires "(" predicate ")"
//!             | ensures "(" predicate ")"
//!             | nfr "(" nfr_item ("," nfr_item)* ")"
//!             | resource "(" "kind" "=" string ")"
//! nfr_item   := "latency_ms" "=" number | "concurrency_max" "=" number
//! predicate  := -- the `predicate::check_predicate_shape` subset: idents,
//!                  int/bool literals, arithmetic, comparisons, &&, ||,
//!                  unary -/!, parens (issue #68)
//! ```
//!
//! Unknown clauses are hard errors — in the dialect, a contract is a
//! checked declaration, so a typo can never degrade into a comment.

use crate::model::{Contract, Crud, Ensures, Nfr, Requires, Resource, Sequence};
use proc_macro2::{Delimiter, TokenStream, TokenTree};

pub fn parse_contract(tokens: TokenStream) -> syn::Result<Contract> {
    let trees: Vec<TokenTree> = tokens.into_iter().collect();
    let mut contract = Contract::default();
    let mut i = 0;
    while i < trees.len() {
        // Comma separators between clauses (and a trailing comma) are
        // tolerated — contracts are declarations, not linters' targets.
        if let TokenTree::Punct(p) = &trees[i] {
            if p.as_char() == ',' {
                i += 1;
                continue;
            }
        }
        let (name, span) = match &trees[i] {
            TokenTree::Ident(id) => (id.to_string(), id.span()),
            other => {
                return Err(syn::Error::new(
                    other.span(),
                    "expected a contract clause — effects(..), requires(role = \"..\"), or nfr(..)",
                ))
            }
        };
        let group = match trees.get(i + 1) {
            Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis => g.clone(),
            _ => {
                return Err(syn::Error::new(
                    span,
                    format!("contract clause `{name}` needs a parenthesized argument list, e.g. {name}(..)"),
                ))
            }
        };
        match name.as_str() {
            "effects" => contract.effects = Some(parse_effects(&group)?),
            "requires" => contract.requires = Some(parse_requires(&group)?),
            "ensures" => contract.ensures = Some(parse_ensures(&group)?),
            "nfr" => contract.nfr = Some(parse_nfr(&group)?),
            "crud" => contract.crud = Some(parse_crud(&group)?),
            "resource" => contract.resource = Some(parse_resource(&group)?),
            "sequence" => contract.sequence = Some(parse_sequence(&group)?),
            other => {
                return Err(syn::Error::new(
                    span,
                    format!(
                        "unknown contract clause `{other}` — valid clauses: \
                         effects(pure, io, net, ...), requires(role = \"name\") or \
                         requires(claim = \"name\", \"value\") or \
                         requires(<bool expr>), ensures(<bool expr>), \
                         nfr(latency_ms = N, concurrency_max = N), \
                         crud(op = \"delete\", policy = \"name\"), \
                         resource(kind = \"name\"), \
                         sequence(before = \"name\", after = \"name\")"
                    ),
                ))
            }
        }
        i += 2;
    }
    Ok(contract)
}

fn parse_effects(group: &proc_macro2::Group) -> syn::Result<Vec<String>> {
    let mut effects = Vec::new();
    for tree in group.stream() {
        match tree {
            TokenTree::Ident(id) => effects.push(id.to_string()),
            TokenTree::Punct(p) if p.as_char() == ',' => {}
            other => {
                return Err(syn::Error::new(
                    other.span(),
                    "effects(..) lists effect names: pure, io, net, db, alloc, clock, random, inference",
                ))
            }
        }
    }
    Ok(effects)
}

/// `requires(role = "role_name")` (unforgeable-proof injection),
/// `requires(claim = "claim_name", "claim_value")` (the same, for a
/// claim key/value pair — `ClaimProof`, `role.rs`'s own sibling
/// mechanism in `nirdosha-rt`), or `requires(<bool expr>)` (a numeric
/// precondition, discharged at Stage 2 against the same Z3 VC IR item 1
/// built — issue #68). The three forms are distinguished the only way
/// that doesn't need a keyword: the three-token `role = "..."` shape is
/// tried first, then the five-token `claim = "...", "..."` shape, and
/// anything else in the group is parsed whole as a Rust expression.
fn parse_requires(group: &proc_macro2::Group) -> syn::Result<Requires> {
    let trees: Vec<TokenTree> = group.stream().into_iter().collect();
    if let [TokenTree::Ident(id), TokenTree::Punct(eq), TokenTree::Literal(lit)] = trees.as_slice()
        && id == "role"
        && eq.as_char() == '='
    {
        return match syn::Lit::new(lit.clone()) {
            syn::Lit::Str(s) => Ok(Requires {
                role: Some(s.value()),
                expr: None,
                claim: None,
            }),
            _ => Err(syn::Error::new(
                lit.span(),
                "role names are string literals: requires(role = \"hr_staff\")",
            )),
        };
    }
    if let [TokenTree::Ident(id), TokenTree::Punct(eq), TokenTree::Literal(name_lit), TokenTree::Punct(comma), TokenTree::Literal(value_lit)] = trees.as_slice()
        && id == "claim"
        && eq.as_char() == '='
        && comma.as_char() == ','
    {
        return match (syn::Lit::new(name_lit.clone()), syn::Lit::new(value_lit.clone())) {
            (syn::Lit::Str(name), syn::Lit::Str(value)) => Ok(Requires {
                role: None,
                expr: None,
                claim: Some((name.value(), value.value())),
            }),
            _ => Err(syn::Error::new(
                value_lit.span(),
                "claim names/values are string literals: requires(claim = \"department\", \"cardiology\")",
            )),
        };
    }
    let expr = parse_predicate_group(group, "requires")?;
    Ok(Requires {
        role: None,
        expr: Some(expr),
        claim: None,
    })
}

/// `ensures(<bool expr>)` — a postcondition over `result` and this
/// function's own parameters, checked against every `return` MIR
/// reaches (issue #68).
fn parse_ensures(group: &proc_macro2::Group) -> syn::Result<Ensures> {
    Ok(Ensures {
        expr: parse_predicate_group(group, "ensures")?,
    })
}

/// Parse a clause's whole token group as a single Rust expression,
/// restricted to `nirdosha_contract_core::predicate`'s supported subset,
/// and render it back to its canonical source string — the form that
/// rides in the doc-encoded contract and that `nirdosha-driver`
/// re-parses against real MIR locals.
fn parse_predicate_group(group: &proc_macro2::Group, clause: &str) -> syn::Result<String> {
    let expr: syn::Expr = syn::parse2(group.stream()).map_err(|e| {
        syn::Error::new(
            group.span(),
            format!(
                "{clause}(..) needs `role = \"name\"` (requires only) or a boolean \
                 expression, e.g. {clause}(idx < len): {e}"
            ),
        )
    })?;
    crate::predicate::check_predicate_shape(&expr)
        .map_err(|msg| syn::Error::new(group.span(), format!("{clause}(..) — {msg}")))?;
    Ok(quote::quote!(#expr).to_string())
}

fn parse_nfr(group: &proc_macro2::Group) -> syn::Result<Nfr> {
    let mut nfr = Nfr {
        latency_ms: None,
        error_rate_max: None,
        throughput_min_per_sec: None,
        concurrency_max: None,
    };
    let trees: Vec<TokenTree> = group.stream().into_iter().collect();
    let mut i = 0;
    while i < trees.len() {
        let key = match &trees[i] {
            TokenTree::Ident(id) => id.to_string(),
            other => {
                return Err(syn::Error::new(
                    other.span(),
                    "nfr keys: latency_ms, error_rate_max, throughput_min_per_sec, concurrency_max",
                ))
            }
        };
        match trees.get(i + 1) {
            Some(TokenTree::Punct(p)) if p.as_char() == '=' => {}
            _ => return Err(syn::Error::new(trees[i].span(), "expected `=`")),
        }
        let value = match trees.get(i + 2) {
            Some(TokenTree::Literal(lit)) => lit.clone(),
            _ => return Err(syn::Error::new(trees[i].span(), "nfr values are numbers")),
        };
        let span = trees[i].span();
        match key.as_str() {
            "latency_ms" => {
                if nfr.latency_ms.is_some() {
                    return Err(syn::Error::new(span, "latency_ms declared twice"));
                }
                nfr.latency_ms = Some(parse_number(&value, span)?);
            }
            "error_rate_max" => {
                if nfr.error_rate_max.is_some() {
                    return Err(syn::Error::new(span, "error_rate_max declared twice"));
                }
                nfr.error_rate_max = Some(parse_number(&value, span)?);
            }
            "throughput_min_per_sec" => {
                if nfr.throughput_min_per_sec.is_some() {
                    return Err(syn::Error::new(span, "throughput_min_per_sec declared twice"));
                }
                nfr.throughput_min_per_sec = Some(parse_number(&value, span)?);
            }
            "concurrency_max" => {
                if nfr.concurrency_max.is_some() {
                    return Err(syn::Error::new(span, "concurrency_max declared twice"));
                }
                nfr.concurrency_max = Some(parse_number(&value, span)? as u64);
            }
            other => {
                return Err(syn::Error::new(
                    span,
                    format!("unknown nfr key `{other}` — valid keys: latency_ms, error_rate_max, throughput_min_per_sec, concurrency_max"),
                ))
            }
        }
        i += 3;
        if i < trees.len() {
            match &trees[i] {
                TokenTree::Punct(p) if p.as_char() == ',' => i += 1,
                other => {
                    return Err(syn::Error::new(
                        other.span(),
                        "nfr items are comma-separated: nfr(latency_ms = 50, concurrency_max = 1000)",
                    ))
                }
            }
        }
    }
    Ok(nfr)
}

/// `crud(op = "delete", policy = "financial_us")` — order-flexible,
/// both keys required, same shape as `nfr(..)`'s key=value list.
fn parse_crud(group: &proc_macro2::Group) -> syn::Result<Crud> {
    let mut op: Option<String> = None;
    let mut policy: Option<String> = None;
    let trees: Vec<TokenTree> = group.stream().into_iter().collect();
    let mut i = 0;
    while i < trees.len() {
        let key = match &trees[i] {
            TokenTree::Ident(id) => id.to_string(),
            other => return Err(syn::Error::new(other.span(), "crud keys: op, policy")),
        };
        match trees.get(i + 1) {
            Some(TokenTree::Punct(p)) if p.as_char() == '=' => {}
            _ => return Err(syn::Error::new(trees[i].span(), "expected `=`")),
        }
        let span = trees[i].span();
        let value = match trees.get(i + 2) {
            Some(TokenTree::Literal(lit)) => match syn::Lit::new(lit.clone()) {
                syn::Lit::Str(s) => s.value(),
                _ => return Err(syn::Error::new(lit.span(), "crud values are string literals")),
            },
            _ => return Err(syn::Error::new(span, "crud values are string literals")),
        };
        match key.as_str() {
            "op" => {
                if op.is_some() {
                    return Err(syn::Error::new(span, "op declared twice"));
                }
                op = Some(value);
            }
            "policy" => {
                if policy.is_some() {
                    return Err(syn::Error::new(span, "policy declared twice"));
                }
                policy = Some(value);
            }
            other => {
                return Err(syn::Error::new(
                    span,
                    format!("unknown crud key `{other}` — valid keys: op, policy"),
                ))
            }
        }
        i += 3;
        if i < trees.len() {
            match &trees[i] {
                TokenTree::Punct(p) if p.as_char() == ',' => i += 1,
                other => {
                    return Err(syn::Error::new(
                        other.span(),
                        "crud items are comma-separated: crud(op = \"delete\", policy = \"financial_us\")",
                    ))
                }
            }
        }
    }
    let op = op.ok_or_else(|| syn::Error::new(group.span(), "crud(..) requires `op`"))?;
    let policy = policy.ok_or_else(|| syn::Error::new(group.span(), "crud(..) requires `policy`"))?;
    Ok(Crud { op, policy })
}

/// `resource(kind = "lock")` — single required key, same shape as
/// `requires(role = "...")`'s three-token pattern.
fn parse_resource(group: &proc_macro2::Group) -> syn::Result<Resource> {
    let trees: Vec<TokenTree> = group.stream().into_iter().collect();
    let [TokenTree::Ident(id), TokenTree::Punct(eq), TokenTree::Literal(lit)] = trees.as_slice()
    else {
        return Err(syn::Error::new(
            group.span(),
            "resource(..) currently supports exactly one key: resource(kind = \"lock\")",
        ));
    };
    if id != "kind" || eq.as_char() != '=' {
        return Err(syn::Error::new(
            group.span(),
            "resource(..) currently supports exactly one key: resource(kind = \"lock\")",
        ));
    }
    match syn::Lit::new(lit.clone()) {
        syn::Lit::Str(s) => Ok(Resource { kind: s.value() }),
        _ => Err(syn::Error::new(
            lit.span(),
            "resource kinds are string literals: resource(kind = \"lock\")",
        )),
    }
}

/// `sequence(before = "debit", after = "credit")` — order-flexible,
/// both keys required, same key=value shape as `crud(..)`.
fn parse_sequence(group: &proc_macro2::Group) -> syn::Result<Sequence> {
    let mut before: Option<String> = None;
    let mut after: Option<String> = None;
    let trees: Vec<TokenTree> = group.stream().into_iter().collect();
    let mut i = 0;
    while i < trees.len() {
        let key = match &trees[i] {
            TokenTree::Ident(id) => id.to_string(),
            other => return Err(syn::Error::new(other.span(), "sequence keys: before, after")),
        };
        match trees.get(i + 1) {
            Some(TokenTree::Punct(p)) if p.as_char() == '=' => {}
            _ => return Err(syn::Error::new(trees[i].span(), "expected `=`")),
        }
        let span = trees[i].span();
        let value = match trees.get(i + 2) {
            Some(TokenTree::Literal(lit)) => match syn::Lit::new(lit.clone()) {
                syn::Lit::Str(s) => s.value(),
                _ => return Err(syn::Error::new(lit.span(), "sequence values are string literals")),
            },
            _ => return Err(syn::Error::new(span, "sequence values are string literals")),
        };
        match key.as_str() {
            "before" => {
                if before.is_some() {
                    return Err(syn::Error::new(span, "before declared twice"));
                }
                before = Some(value);
            }
            "after" => {
                if after.is_some() {
                    return Err(syn::Error::new(span, "after declared twice"));
                }
                after = Some(value);
            }
            other => {
                return Err(syn::Error::new(
                    span,
                    format!("unknown sequence key `{other}` — valid keys: before, after"),
                ))
            }
        }
        i += 3;
        if i < trees.len() {
            match &trees[i] {
                TokenTree::Punct(p) if p.as_char() == ',' => i += 1,
                other => {
                    return Err(syn::Error::new(
                        other.span(),
                        "sequence items are comma-separated: sequence(before = \"debit\", after = \"credit\")",
                    ))
                }
            }
        }
    }
    let before = before.ok_or_else(|| syn::Error::new(group.span(), "sequence(..) requires `before`"))?;
    let after = after.ok_or_else(|| syn::Error::new(group.span(), "sequence(..) requires `after`"))?;
    Ok(Sequence { before, after })
}

fn parse_number(lit: &proc_macro2::Literal, span: proc_macro2::Span) -> syn::Result<f64> {
    match syn::Lit::new(lit.clone()) {
        syn::Lit::Int(i) => i.base10_parse::<f64>().map_err(|_| err(span)),
        syn::Lit::Float(f) => f.base10_parse::<f64>().map_err(|_| err(span)),
        _ => Err(err(span)),
    }
}

fn err(span: proc_macro2::Span) -> syn::Error {
    syn::Error::new(span, "expected a number, e.g. nfr(latency_ms = 50)")
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;

    #[test]
    fn parses_all_three_clauses() {
        let c = parse_contract(quote! {
            effects(pure),
            requires(role = "hr_staff"),
            nfr(latency_ms = 50, concurrency_max = 1000)
        })
        .unwrap();
        assert!(c.claims_pure());
        assert_eq!(c.requires.as_ref().unwrap().role.as_deref(), Some("hr_staff"));
        assert_eq!(c.nfr.unwrap().concurrency_max, Some(1000));
    }

    // issue #68: `requires(expr)`/`ensures(expr)` — a numeric pre/post
    // condition, not just a role gate. This is the red test: today
    // `requires(..)` accepts only `role = "..."` and there is no
    // `ensures` clause at all, so both assertions below fail until the
    // parser and model grow the `expr`/`Ensures` support this issue asks for.
    #[test]
    fn parses_requires_and_ensures_expressions() {
        let c = parse_contract(quote! {
            requires(idx < len),
            ensures(result >= 0)
        })
        .unwrap();
        assert_eq!(c.requires.as_ref().unwrap().expr.as_deref(), Some("idx < len"));
        assert_eq!(c.ensures.as_ref().unwrap().expr, "result >= 0");
    }

    #[test]
    fn parses_requires_claim_clause() {
        let c = parse_contract(quote! {
            requires(claim = "department", "cardiology")
        })
        .unwrap();
        assert_eq!(c.requires.as_ref().unwrap().claim, Some(("department".to_string(), "cardiology".to_string())));
        assert_eq!(c.requires.as_ref().unwrap().role, None);
        assert_eq!(c.requires.as_ref().unwrap().expr, None);
    }

    #[test]
    fn requires_claim_needs_two_string_literals() {
        assert!(parse_contract(quote! { requires(claim = "department", cardiology) }).is_err());
    }

    // issue #69: `resource(kind = "..")` — no such clause exists yet, so
    // this is the red test: `parse_contract` errors "unknown contract
    // clause `resource`" until the parser/model grow this clause.
    #[test]
    fn parses_resource_clause() {
        let c = parse_contract(quote! { resource(kind = "lock") }).unwrap();
        assert_eq!(c.resource.unwrap().kind, "lock");
    }

    #[test]
    fn parses_sequence_clause() {
        let c = parse_contract(quote! { sequence(before = "debit", after = "credit") }).unwrap();
        let sequence = c.sequence.unwrap();
        assert_eq!(sequence.before, "debit");
        assert_eq!(sequence.after, "credit");
        // Order-flexible, same as crud(..).
        let c = parse_contract(quote! { sequence(after = "credit", before = "debit") }).unwrap();
        assert_eq!(c.sequence.unwrap().before, "debit");
    }

    #[test]
    fn sequence_clause_requires_both_keys() {
        assert!(parse_contract(quote! { sequence(before = "debit") }).is_err());
        assert!(parse_contract(quote! { sequence(after = "credit") }).is_err());
    }

    #[test]
    fn unknown_clause_is_an_error() {
        let e = parse_contract(quote! { effectz(pure) }).unwrap_err();
        assert!(e.to_string().contains("unknown contract clause"));
    }

    #[test]
    fn unknown_nfr_key_is_an_error() {
        let e = parse_contract(quote! { nfr(latency = 50) }).unwrap_err();
        assert!(e.to_string().contains("unknown nfr key"));
    }

    #[test]
    fn parses_crud_clause_either_key_order() {
        let c = parse_contract(quote! { crud(op = "delete", policy = "financial_us") }).unwrap();
        let crud = c.crud.unwrap();
        assert_eq!(crud.op, "delete");
        assert_eq!(crud.policy, "financial_us");

        let c = parse_contract(quote! { crud(policy = "financial_us", op = "delete") }).unwrap();
        let crud = c.crud.unwrap();
        assert_eq!(crud.op, "delete");
        assert_eq!(crud.policy, "financial_us");
    }

    #[test]
    fn crud_missing_a_key_is_an_error() {
        let e = parse_contract(quote! { crud(op = "delete") }).unwrap_err();
        assert!(e.to_string().contains("requires `policy`"));
    }
}