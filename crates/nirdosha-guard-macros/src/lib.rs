//! `nirdosha-guard-macros` — compile-time policy / dataset / relation surface.
//!
//! Syntax front-end for RFC 0023. Emits descriptors into distributed
//! registry slices (`POLICIES`, `CATALOG`, `ROLES`, `APPROVAL_CHAINS`, `WORKFLOWS`, etc.)
//! so `cargo nirdosha verify` can run Gate 2 checks.

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    parse::{Parse, ParseStream},
    Ident, LitStr, Result, Token,
};

struct PolicyInput {
    effect: Ident,
    id: LitStr,
    action: LitStr,
    resource: LitStr,
    purpose: Option<LitStr>,
    clauses: String,
}

impl Parse for PolicyInput {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let effect: Ident = input.parse()?;
        if effect != "allow" && effect != "deny" {
            return Err(syn::Error::new(effect.span(), "expected `allow` or `deny`"));
        }
        let id: LitStr = input.parse()?;

        // Optional `for Role, Role...`
        if input.peek(Ident) && input.fork().parse::<Ident>()?.to_string() == "for" {
            let _: Ident = input.parse()?;
            while !input.peek(Ident)
                || input
                    .fork()
                    .parse::<Ident>()
                    .map(|value| value.to_string() != "when")
                    .unwrap_or(false)
            {
                let _: Ident = input.parse()?;
                if input.peek(Token![,]) {
                    let _: Token![,] = input.parse()?;
                } else {
                    break;
                }
            }
        }

        let when: Ident = input.parse()?;
        if when != "when" {
            return Err(syn::Error::new(when.span(), "expected `when`"));
        }
        let action_name: Ident = input.parse()?;
        if action_name != "action" {
            return Err(syn::Error::new(action_name.span(), "expected `action`"));
        }
        let _: Token![==] = input.parse()?;
        let action: LitStr = input.parse()?;

        let mut resource = LitStr::new("*", proc_macro2::Span::call_site());
        let mut purpose = None;

        if input.peek(Token![&&]) {
            let _: Token![&&] = input.parse()?;
            let resource_name: Ident = input.parse()?;
            if resource_name == "resource" {
                let _: Token![==] = input.parse()?;
                resource = input.parse()?;
            }
        }

        let mut clauses_tokens = Vec::new();

        // Parse remaining tokens for purpose or details/clauses
        while !input.is_empty() {
            if input.peek(Ident) {
                let kw: Ident = input.parse()?;
                if kw == "purpose" && input.peek(syn::token::Paren) {
                    let content;
                    syn::parenthesized!(content in input);
                    if let Ok(p) = content.parse::<LitStr>() {
                        purpose = Some(p);
                    }
                } else {
                    clauses_tokens.push(kw.to_string());
                }
            } else {
                let tt: proc_macro2::TokenTree = input.parse()?;
                clauses_tokens.push(tt.to_string());
            }
        }

        Ok(Self {
            effect,
            id,
            action,
            resource,
            purpose,
            clauses: clauses_tokens.join(" "),
        })
    }
}

fn policy_impl(input: TokenStream) -> TokenStream {
    let parsed = match syn::parse::<PolicyInput>(input) {
        Ok(value) => value,
        Err(error) => return error.into_compile_error().into(),
    };
    let id = parsed.id.value();
    let static_name = format_ident!(
        "__NIRDOSHA_POLICY_{}",
        id.chars()
            .map(|ch| if ch.is_ascii_alphanumeric() {
                ch.to_ascii_uppercase()
            } else {
                '_'
            })
            .collect::<String>()
    );
    let effect = if parsed.effect == "allow" {
        quote!(::nirdosha_guard_registry::Effect::Allow)
    } else {
        quote!(::nirdosha_guard_registry::Effect::Deny)
    };
    let id_lit = parsed.id;
    let action = parsed.action;
    let resource = parsed.resource;
    let purpose_expr = match parsed.purpose {
        Some(p) => quote!(Some(#p)),
        None => quote!(None),
    };
    let clauses_expr = if parsed.clauses.is_empty() {
        quote!(None)
    } else {
        let clauses = parsed.clauses;
        quote!(Some(#clauses))
    };

    quote! {
        #[used]
        #[::nirdosha_guard_registry::linkme::distributed_slice(::nirdosha_guard_registry::POLICIES)]
        #[linkme(crate = ::nirdosha_guard_registry::linkme)]
        static #static_name: ::nirdosha_guard_registry::PolicyRegistration = ::nirdosha_guard_registry::PolicyRegistration {
            id: #id_lit,
            effect: #effect,
            action: #action,
            resource: #resource,
            purpose: #purpose_expr,
            clauses_json: #clauses_expr,
            source: module_path!(),
            line: line!(),
        };
    }
    .into()
}

/// `policy! { allow "name" when ... }` — canonical policy surface.
#[proc_macro]
pub fn policy(input: TokenStream) -> TokenStream {
    policy_impl(input)
}

/// Alias for `policy!`.
#[proc_macro]
pub fn guard_policy(input: TokenStream) -> TokenStream {
    policy_impl(input)
}

fn attribute_impl(kind: &'static str, args: TokenStream, input: TokenStream) -> TokenStream {
    let item = match syn::parse::<syn::Item>(input.clone()) {
        Ok(item) => item,
        Err(error) => return error.into_compile_error().into(),
    };
    let name = match &item {
        syn::Item::Const(item) => item.ident.clone(),
        syn::Item::Enum(item) => item.ident.clone(),
        syn::Item::Fn(item) => item.sig.ident.clone(),
        syn::Item::Struct(item) => item.ident.clone(),
        syn::Item::Type(item) => item.ident.clone(),
        _ => {
            return syn::Error::new_spanned(item, "guard attribute requires a named item")
                .into_compile_error()
                .into()
        }
    };
    let source = args.to_string();
    let static_name = format_ident!("__NIRDOSHA_{}_{}", kind.to_ascii_uppercase(), name);
    let kind_lit = kind;
    let source_lit = source;
    quote! {
        #item
        #[used]
        #[::nirdosha_guard_registry::linkme::distributed_slice(::nirdosha_guard_registry::CATALOG)]
        #[linkme(crate = ::nirdosha_guard_registry::linkme)]
        static #static_name: ::nirdosha_guard_registry::CatalogRegistration = ::nirdosha_guard_registry::CatalogRegistration {
            kind: #kind_lit,
            name: stringify!(#name),
            source: #source_lit,
        };
    }
    .into()
}

/// `#[dataset(entity = "...", store = "...")]` — binds an entity to a store.
#[proc_macro_attribute]
pub fn dataset(args: TokenStream, input: TokenStream) -> TokenStream {
    attribute_impl("dataset", args, input)
}

/// `#[relation(source = "...", cardinality = N, ttl = ...)]`.
#[proc_macro_attribute]
pub fn relation(args: TokenStream, input: TokenStream) -> TokenStream {
    attribute_impl("relation", args, input)
}

/// `#[classify(level = ...)]` — marks a type's classification.
#[proc_macro_attribute]
pub fn classify(args: TokenStream, input: TokenStream) -> TokenStream {
    attribute_impl("classify", args, input)
}

/// `#[mask_transform(name = ...)]` — registers a mask transform.
#[proc_macro_attribute]
pub fn mask_transform(args: TokenStream, input: TokenStream) -> TokenStream {
    attribute_impl("mask_transform", args, input)
}

/// `#[materialize(relation = ...)]` — Tier 2 materialized column.
#[proc_macro_attribute]
pub fn materialize(args: TokenStream, input: TokenStream) -> TokenStream {
    attribute_impl("materialize", args, input)
}

/// `#[reference(field = ..., store = ..., missing = ...)]`.
#[proc_macro_attribute]
pub fn reference(args: TokenStream, input: TokenStream) -> TokenStream {
    attribute_impl("reference", args, input)
}

/// `#[invariant(name = ..., schema = ...)]` — pure, schema-typed invariant.
#[proc_macro_attribute]
pub fn invariant(args: TokenStream, input: TokenStream) -> TokenStream {
    attribute_impl("invariant", args, input)
}

/// `#[purpose(code = ..., basis = ..., review = ...)]` — tenant-extensible purpose.
#[proc_macro_attribute]
pub fn purpose(args: TokenStream, input: TokenStream) -> TokenStream {
    attribute_impl("purpose", args, input)
}

/// `guard_roles! { RoleA, RoleB, ... }`.
#[proc_macro]
pub fn guard_roles(input: TokenStream) -> TokenStream {
    let input_str = input.to_string();
    let static_name = format_ident!("__NIRDOSHA_ROLES_REGISTRATION");
    quote! {
        #[used]
        #[::nirdosha_guard_registry::linkme::distributed_slice(::nirdosha_guard_registry::CATALOG)]
        #[linkme(crate = ::nirdosha_guard_registry::linkme)]
        static #static_name: ::nirdosha_guard_registry::CatalogRegistration = ::nirdosha_guard_registry::CatalogRegistration {
            kind: "roles",
            name: "guard_roles",
            source: #input_str,
        };
    }
    .into()
}

/// `approval_chain! { name { ... } }`.
#[proc_macro]
pub fn approval_chain(input: TokenStream) -> TokenStream {
    let input_str = input.to_string();
    let static_name = format_ident!("__NIRDOSHA_APPROVAL_CHAIN_REG");
    quote! {
        #[used]
        #[::nirdosha_guard_registry::linkme::distributed_slice(::nirdosha_guard_registry::CATALOG)]
        #[linkme(crate = ::nirdosha_guard_registry::linkme)]
        static #static_name: ::nirdosha_guard_registry::CatalogRegistration = ::nirdosha_guard_registry::CatalogRegistration {
            kind: "approval_chain",
            name: "approval_chain",
            source: #input_str,
        };
    }
    .into()
}

/// `workflow! { machine Name { ... } }`.
#[proc_macro]
pub fn workflow(input: TokenStream) -> TokenStream {
    let input_str = input.to_string();
    let static_name = format_ident!("__NIRDOSHA_WORKFLOW_REG");
    quote! {
        #[used]
        #[::nirdosha_guard_registry::linkme::distributed_slice(::nirdosha_guard_registry::CATALOG)]
        #[linkme(crate = ::nirdosha_guard_registry::linkme)]
        static #static_name: ::nirdosha_guard_registry::CatalogRegistration = ::nirdosha_guard_registry::CatalogRegistration {
            kind: "workflow",
            name: "workflow",
            source: #input_str,
        };
    }
    .into()
}

/// `break_glass! { grant name { ... } }`.
#[proc_macro]
pub fn break_glass(input: TokenStream) -> TokenStream {
    let input_str = input.to_string();
    let static_name = format_ident!("__NIRDOSHA_BREAK_GLASS_REG");
    quote! {
        #[used]
        #[::nirdosha_guard_registry::linkme::distributed_slice(::nirdosha_guard_registry::CATALOG)]
        #[linkme(crate = ::nirdosha_guard_registry::linkme)]
        static #static_name: ::nirdosha_guard_registry::CatalogRegistration = ::nirdosha_guard_registry::CatalogRegistration {
            kind: "break_glass",
            name: "break_glass",
            source: #input_str,
        };
    }
    .into()
}

/// `audit_sampling! { ... }` — explicit classification → rate table.
#[proc_macro]
pub fn audit_sampling(input: TokenStream) -> TokenStream {
    let input_str = input.to_string();
    let static_name = format_ident!("__NIRDOSHA_AUDIT_SAMPLING_REG");
    quote! {
        #[used]
        #[::nirdosha_guard_registry::linkme::distributed_slice(::nirdosha_guard_registry::CATALOG)]
        #[linkme(crate = ::nirdosha_guard_registry::linkme)]
        static #static_name: ::nirdosha_guard_registry::CatalogRegistration = ::nirdosha_guard_registry::CatalogRegistration {
            kind: "audit_sampling",
            name: "audit_sampling",
            source: #input_str,
        };
    }
    .into()
}

/// `audit_rules! { always_full: [...] }`.
#[proc_macro]
pub fn audit_rules(input: TokenStream) -> TokenStream {
    let input_str = input.to_string();
    let static_name = format_ident!("__NIRDOSHA_AUDIT_RULES_REG");
    quote! {
        #[used]
        #[::nirdosha_guard_registry::linkme::distributed_slice(::nirdosha_guard_registry::CATALOG)]
        #[linkme(crate = ::nirdosha_guard_registry::linkme)]
        static #static_name: ::nirdosha_guard_registry::CatalogRegistration = ::nirdosha_guard_registry::CatalogRegistration {
            kind: "audit_rules",
            name: "audit_rules",
            source: #input_str,
        };
    }
    .into()
}

/// `enumerate! { options from ... }` — scope-injected enumeration.
#[proc_macro]
pub fn enumerate(input: TokenStream) -> TokenStream {
    let input_str = input.to_string();
    let static_name = format_ident!("__NIRDOSHA_ENUMERATE_REG");
    quote! {
        #[used]
        #[::nirdosha_guard_registry::linkme::distributed_slice(::nirdosha_guard_registry::CATALOG)]
        #[linkme(crate = ::nirdosha_guard_registry::linkme)]
        static #static_name: ::nirdosha_guard_registry::CatalogRegistration = ::nirdosha_guard_registry::CatalogRegistration {
            kind: "enumerate",
            name: "enumerate",
            source: #input_str,
        };
    }
    .into()
}

