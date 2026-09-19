//! `nirdosha-guard-macros` — compile-time policy / dataset / relation surface.
//!
//! This crate is the canonical syntax front-end for RFC 0023. It emits
//! descriptors into a distributed registry so `cargo nirdosha verify` can
//! check schema/policy coherence, classification consistency, and coverage
//! matrix totality.
//!
//! Current phase: scaffolding. The macros parse their input and emit
//! placeholders; full lowering to Cedar and driver plans is future work.

use proc_macro::TokenStream;
use quote::quote;

/// `policy! { allow "name" when ... }` — canonical policy surface.
#[proc_macro]
pub fn policy(input: TokenStream) -> TokenStream {
    let _ = input;
    quote! {
        // Placeholder: a policy block currently compiles to nothing.
        // In later phases it will emit a static `PolicyDescriptor` and,
        // for the lowerable subset, a compiled `AccessPlan`/`WritePlan`.
    }
    .into()
}

/// `#[dataset(entity = "...", store = "...")]` — binds an entity to a store.
#[proc_macro_attribute]
pub fn dataset(args: TokenStream, input: TokenStream) -> TokenStream {
    let _ = args;
    input
}

/// `#[relation(source = "...", cardinality = N, ttl = ...)]`.
#[proc_macro_attribute]
pub fn relation(args: TokenStream, input: TokenStream) -> TokenStream {
    let _ = args;
    input
}

/// `#[classify(level = ...)]` — marks a type's classification.
#[proc_macro_attribute]
pub fn classify(args: TokenStream, input: TokenStream) -> TokenStream {
    let _ = args;
    input
}

/// `#[mask_transform(name = ...)]` — registers a mask transform.
#[proc_macro_attribute]
pub fn mask_transform(args: TokenStream, input: TokenStream) -> TokenStream {
    let _ = args;
    input
}

/// `#[materialize(relation = ...)]` — Tier 2 materialized column.
#[proc_macro_attribute]
pub fn materialize(args: TokenStream, input: TokenStream) -> TokenStream {
    let _ = args;
    input
}

/// `#[reference(field = ..., store = ..., missing = ...)]`.
#[proc_macro_attribute]
pub fn reference(args: TokenStream, input: TokenStream) -> TokenStream {
    let _ = args;
    input
}

/// `#[invariant(name = ..., schema = ...)]` — pure, schema-typed invariant.
#[proc_macro_attribute]
pub fn invariant(args: TokenStream, input: TokenStream) -> TokenStream {
    let _ = args;
    input
}

/// `#[purpose(code = ..., basis = ..., review = ...)]` — tenant-extensible purpose.
#[proc_macro_attribute]
pub fn purpose(args: TokenStream, input: TokenStream) -> TokenStream {
    let _ = args;
    input
}

/// `audit_sampling! { ... }` — explicit classification → rate table.
#[proc_macro]
pub fn audit_sampling(input: TokenStream) -> TokenStream {
    let _ = input;
    quote! {}.into()
}

/// `audit_rules! { always_full: [...] }`.
#[proc_macro]
pub fn audit_rules(input: TokenStream) -> TokenStream {
    let _ = input;
    quote! {}.into()
}

/// `enumerate! { options from ... }` — scope-injected enumeration.
#[proc_macro]
pub fn enumerate(input: TokenStream) -> TokenStream {
    let _ = input;
    quote! {}.into()
}

/// `break_glass! { ... }`.
#[proc_macro]
pub fn break_glass(input: TokenStream) -> TokenStream {
    let _ = input;
    quote! {}.into()
}

/// `approval_chain! { ... }`.
#[proc_macro]
pub fn approval_chain(input: TokenStream) -> TokenStream {
    let _ = input;
    quote! {}.into()
}
