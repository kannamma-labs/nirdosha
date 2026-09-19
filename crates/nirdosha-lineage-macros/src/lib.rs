//! `nirdosha-lineage-macros` — compile-time surface for RFC 0026.

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    parse::{Parse, ParseStream},
    Ident, Result, LitStr,
};

struct LineageQueryInput {
    name: Ident,
}

impl Parse for LineageQueryInput {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let kw: Ident = input.parse()?;
        if kw == "view" {
            let name: Ident = input.parse()?;
            // drain remaining
            while !input.is_empty() {
                let _: proc_macro2::TokenTree = input.parse()?;
            }
            Ok(Self { name })
        } else {
            Ok(Self { name: kw })
        }
    }
}

/// `lineage_query! { view name(...) -> Row { ... }; ... }` — RFC 0026 §9.2.
#[proc_macro]
pub fn lineage_query(input: TokenStream) -> TokenStream {
    let parsed = match syn::parse::<LineageQueryInput>(input) {
        Ok(res) => res,
        Err(err) => return err.into_compile_error().into(),
    };
    let name = &parsed.name;
    let struct_name = format_ident!("{}Query", name);

    quote! {
        pub struct #struct_name {
            pub view_name: &'static str,
        }
        impl #struct_name {
            pub fn new() -> Self {
                Self { view_name: stringify!(#name) }
            }
        }
    }
    .into()
}

/// `data_contract! { ... }` — RFC 0026 §10.1.
#[proc_macro]
pub fn data_contract(input: TokenStream) -> TokenStream {
    let input_str = input.to_string();
    quote! {
        pub const DATA_CONTRACT_SPEC: &'static str = #input_str;
    }
    .into()
}

/// `migration_plan! { ... }` — RFC 0026 §10.2.
#[proc_macro]
pub fn migration_plan(input: TokenStream) -> TokenStream {
    let input_str = input.to_string();
    quote! {
        pub const MIGRATION_PLAN_SPEC: &'static str = #input_str;
    }
    .into()
}

/// `policy_simulation! { ... }` — RFC 0026 §10.3.
#[proc_macro]
pub fn policy_simulation(input: TokenStream) -> TokenStream {
    let input_str = input.to_string();
    quote! {
        pub const POLICY_SIMULATION_SPEC: &'static str = #input_str;
    }
    .into()
}

