//! `#[contract(..)]` — checked declarations for the Nirdosha Rust
//! dialect.
//!
//! ```ignore
//! nirdosha_rt::roles! { HrStaff = "hr_staff"; }
//!
//! #[nirdosha_rt::contract(
//!     effects(pure),                        // lying here is a compile error
//!     requires(role = "hr_staff"),          // uncallable without a minted proof
//!     nfr(latency_ms = 50, concurrency_max = 1000),
//! )]
//! fn compute_payroll(records: &[PayRecord]) -> u64 { ... }
//! ```
//!
//! Expansion is 100% ordinary Rust that compiles and runs under a stock
//! toolchain with no verification active:
//!
//! - a `#[doc = "nirdosha:contract {json}"]` attribute carrying the
//!   portable contract encoding (flows into rustdoc JSON for agents and
//!   auditors);
//! - for `requires(role = "..")`, a leading proof parameter
//!   `__nirdosha_role_proof: &::nirdosha_rt::RoleProof<crate::nirdosha_roles::HrStaff>`
//!   — a token with a private constructor that only `nirdosha_rt::Auth`
//!   can mint after a real role check. Without the proof the fn is
//!   *uncallable*, under any compiler;
//! - for `nfr(..)`, a Drop-based guard wrapping the body that records
//!   latency against the declared limit and gates concurrency;
//! - for `effects(pure)`, a local scan of the body for known-impure
//!   operations. A lie is a `compile_error!` — under plain cargo too.
//!
//! Stage 2 (the rustc driver) upgrades the local scan to full
//! interprocedural proof; until then the scan is deliberately
//! over-approximate (see `nirdosha-contract-core/src/scan.rs`).

use nirdosha_contract_core as cc;
use proc_macro::TokenStream;
use quote::quote;
use syn::spanned::Spanned;

#[proc_macro_attribute]
pub fn contract(attr: TokenStream, item: TokenStream) -> TokenStream {
    expand(attr.into(), item).into()
}

fn expand(
    attr: proc_macro2::TokenStream,
    item: TokenStream,
) -> proc_macro2::TokenStream {
    let mut fn_item = match syn::parse::<syn::ItemFn>(item) {
        Ok(f) => f,
        Err(e) => return e.to_compile_error(),
    };
    let contract = match cc::parse::parse_contract(attr) {
        Ok(c) => c,
        Err(e) => return e.to_compile_error(),
    };

    let fn_span = fn_item.sig.fn_token.span;
    let fn_name = fn_item.sig.ident.to_string();

    // --- dialect gates (compile errors under *both* compilers) ---
    if let Some(issue) = contract.validate().first() {
        return syn::Error::new(
            fn_item.sig.ident.span(),
            format!("invalid contract on `{fn_name}`: {issue}"),
        )
        .to_compile_error();
    }
    if fn_item.sig.unsafety.is_some() {
        return syn::Error::new(fn_span, "the nirdosha dialect forbids `unsafe fn`")
            .to_compile_error();
    }
    if fn_item.sig.asyncness.is_some() {
        return syn::Error::new(
            fn_span,
            "async fn contracts arrive with the Stage-2 rustc driver; plain fn only for now",
        )
        .to_compile_error();
    }
    if fn_item.sig.constness.is_some() && contract.nfr.is_some() {
        return syn::Error::new(
            fn_span,
            "const fn cannot carry nfr(..) — the NFR guard needs runtime state",
        )
        .to_compile_error();
    }

    // --- honesty: claiming pure must survive a scan of the body ---
    if contract.claims_pure() {
        let hits = cc::scan::impure_calls_in_block(&fn_item.block);
        if !hits.is_empty() {
            let details = hits
                .iter()
                .map(|h| format!("  · {} ({}) at line {}", h.what, h.why, h.line))
                .collect::<Vec<_>>()
                .join("\n");
            let msg = format!(
                "fn `{fn_name}` claims effects(pure) but the body performs:\n{details}\n\
                 a contract is a checked declaration, not a comment — remove the \
                 impure call or stop claiming purity"
            );
            let mut err = syn::Error::new(fn_span, msg);
            for h in &hits {
                err.combine(syn::Error::new(
                    fn_item.block.span(),
                    format!("impure operation: {} ({})", h.what, h.why),
                ));
            }
            return err.to_compile_error();
        }
    }

    // --- capability injection: requires(role) becomes an unforgeable proof parameter ---
    if let Some(requires) = &contract.requires {
        let ident = match cc::role::role_ident(&requires.role, fn_item.sig.ident.span()) {
            Ok(i) => i,
            Err(e) => return e.to_compile_error(),
        };
        let param: syn::FnArg =
            syn::parse_quote!(__nirdosha_role_proof: &::nirdosha_rt::RoleProof<crate::nirdosha_roles::#ident>);
        fn_item.sig.inputs.insert(0, param);
    }

    // --- runtime injection: nfr(..) wraps the body in a Drop-based guard ---
    if let Some(nfr) = &contract.nfr {
        let name_str = fn_name.clone();
        let latency = match nfr.latency_ms {
            Some(v) => quote!(Some(#v)),
            None => quote!(None),
        };
        let concurrency = match nfr.concurrency_max {
            Some(v) => quote!(Some(#v)),
            None => quote!(None),
        };
        let original = &fn_item.block;
        fn_item.block = syn::parse2(quote! {{
            let __nirdosha_nfr_guard = ::nirdosha_rt::nfr::enter(
                #name_str,
                &::nirdosha_rt::nfr::Limits {
                    latency_ms: #latency,
                    concurrency_max: #concurrency,
                },
            );
            #original
        }})
        .expect("nfr guard wrapper always parses");
    }

    // --- portable encoding: the contract rides into rustdoc as a doc attribute ---
    let doc = contract.doc_string();
    quote! {
        #[doc = #doc]
        #fn_item
    }
}