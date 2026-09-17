//! `landing! { .. }` — the dialect substitute for `.nir`'s `landing { .. }`
//! top-level form (issue #70): first-match-wins rules picking which
//! path an authenticated session redirects to, exactly the `LandingDecl`/
//! `LandingRule` shape `crates/compiler/src/ast.rs` documents, minus the
//! `claim(..)` condition (the dialect's `Auth`/role runtime has no claim
//! concept at all yet — see `docs/DIALECT_FORM_COVERAGE.md`).
//!
//! ```ignore
//! nirdosha_rt::landing! {
//!     role("admin") -> "/admin",
//!     role("hr_staff") -> "/hr",
//!     default -> "/home",
//! }
//! ```
//!
//! expands to a single `landing_path(auth: &nirdosha_rt::Auth) -> &'static str`
//! evaluating the rules in source order via `Auth::has_role`, falling back
//! to `default`'s target when nothing else matches. The fixed name is
//! deliberate: it makes a second `landing!` in the same crate `rustc`'s own
//! duplicate-definition error, the same "at most one per program" property
//! `.nir`'s own parser separately enforces for `LandingDecl`.
//!
//! Validated at macro-expansion time, mirroring `typeck::check_landing`'s
//! own documented rules exactly: at least one rule, exactly one `default`,
//! and `default` must be last (so nothing after it is unreachable).

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{parenthesized, Ident, LitStr, Token};

enum Condition {
    Role(LitStr),
    Default,
}

struct Rule {
    condition: Condition,
    target: LitStr,
    span: proc_macro2::Span,
}

struct LandingInput {
    rules: Vec<Rule>,
}

impl Parse for LandingInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut rules = Vec::new();
        while !input.is_empty() {
            let keyword: Ident = input.parse()?;
            let span = keyword.span();
            let condition = if keyword == "role" {
                let content;
                parenthesized!(content in input);
                let role: LitStr = content.parse()?;
                Condition::Role(role)
            } else if keyword == "default" {
                Condition::Default
            } else {
                return Err(syn::Error::new(
                    span,
                    format!("expected `role(\"..\")` or `default`, found `{keyword}`"),
                ));
            };
            input.parse::<Token![->]>()?;
            let target: LitStr = input.parse()?;
            rules.push(Rule {
                condition,
                target,
                span,
            });
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }
        if rules.is_empty() {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "landing! needs at least one rule, e.g. `default -> \"/home\"`",
            ));
        }
        let default_count = rules
            .iter()
            .filter(|r| matches!(r.condition, Condition::Default))
            .count();
        if default_count == 0 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "landing! needs exactly one `default -> \"..\"` rule",
            ));
        }
        if default_count > 1 {
            return Err(syn::Error::new(
                rules
                    .iter()
                    .filter(|r| matches!(r.condition, Condition::Default))
                    .nth(1)
                    .unwrap()
                    .span,
                "landing! allows exactly one `default` rule",
            ));
        }
        if !matches!(rules.last().unwrap().condition, Condition::Default) {
            let after_default = rules
                .iter()
                .position(|r| matches!(r.condition, Condition::Default))
                .map(|i| rules[i + 1].span)
                .unwrap();
            return Err(syn::Error::new(
                after_default,
                "landing!'s `default` rule must be last — every rule after it is unreachable",
            ));
        }
        Ok(LandingInput { rules })
    }
}

pub fn expand(input: TokenStream) -> TokenStream {
    let parsed = match syn::parse::<LandingInput>(input) {
        Ok(p) => p,
        Err(e) => return e.to_compile_error().into(),
    };
    expand_parsed(parsed).into()
}

fn expand_parsed(input: LandingInput) -> TokenStream2 {
    let arms = input.rules.iter().filter_map(|rule| match &rule.condition {
        Condition::Role(role) => {
            let target = &rule.target;
            Some(quote! {
                if auth.has_role(#role) {
                    return #target;
                }
            })
        }
        Condition::Default => None,
    });
    let default_target = &input
        .rules
        .iter()
        .find(|r| matches!(r.condition, Condition::Default))
        .unwrap()
        .target;

    quote! {
        /// Generated by `nirdosha_rt::landing!` — see its own doc comment.
        #[allow(dead_code)]
        pub fn landing_path(auth: &::nirdosha_rt::Auth) -> &'static str {
            #(#arms)*
            #default_target
        }
    }
}
