//! Parser for the `#[contract(...)]` attribute token form.
//!
//! Grammar (Stage 1):
//!
//! ```text
//! contract   := clause*
//! clause     := effects "(" ident ("," ident)* ")"
//!             | requires "(" "role" "=" string ")"
//!             | nfr "(" nfr_item ("," nfr_item)* ")"
//! nfr_item   := "latency_ms" "=" number | "concurrency_max" "=" number
//! ```
//!
//! Unknown clauses are hard errors — in the dialect, a contract is a
//! checked declaration, so a typo can never degrade into a comment.

use crate::model::{Contract, Nfr, Requires};
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
            "nfr" => contract.nfr = Some(parse_nfr(&group)?),
            other => {
                return Err(syn::Error::new(
                    span,
                    format!(
                        "unknown contract clause `{other}` — valid clauses: \
                         effects(pure, io, net, ...), requires(role = \"name\"), \
                         nfr(latency_ms = N, concurrency_max = N)"
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

fn parse_requires(group: &proc_macro2::Group) -> syn::Result<Requires> {
    let trees: Vec<TokenTree> = group.stream().into_iter().collect();
    if trees.len() != 3 {
        return Err(syn::Error::new(
            group.span(),
            "requires(..) currently supports exactly one key: requires(role = \"role_name\")",
        ));
    }
    match &trees[0] {
        TokenTree::Ident(id) if id == "role" => {}
        other => {
            return Err(syn::Error::new(
                other.span(),
                "requires(..) currently supports exactly one key: requires(role = \"role_name\")",
            ))
        }
    }
    match &trees[1] {
        TokenTree::Punct(p) if p.as_char() == '=' => {}
        other => return Err(syn::Error::new(other.span(), "expected `=`")),
    }
    match &trees[2] {
        TokenTree::Literal(lit) => match syn::Lit::new(lit.clone()) {
            syn::Lit::Str(s) => Ok(Requires { role: s.value() }),
            _ => Err(syn::Error::new(
                trees[2].span(),
                "role names are string literals: requires(role = \"hr_staff\")",
            )),
        },
        other => Err(syn::Error::new(other.span(), "role names are string literals")),
    }
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
        assert_eq!(c.requires.as_ref().unwrap().role, "hr_staff");
        assert_eq!(c.nfr.unwrap().concurrency_max, Some(1000));
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
}