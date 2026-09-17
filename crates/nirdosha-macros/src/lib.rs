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
//!   operations. A lie is a `compile_error!` — under plain cargo too;
//! - for `crud(op = "..", policy = "..")`, a sibling `const _: () =
//!   assert!(..)` item cross-checking the named
//!   [`nirdosha_rt::policy!`]-declared policy's forbidden-operations
//!   list. `rustc`'s own const-evaluator refuses the build if the
//!   policy forbids the named operation — see RFC 0020 and
//!   `docs/nirdosha-rt-dialect.md`'s "no bespoke parser, ever" rule.
//!
//! Stage 2 (the rustc driver) upgrades the local scan to full
//! interprocedural proof; until then the scan is deliberately
//! over-approximate (see `nirdosha-contract-core/src/scan.rs`).

mod categorical;
mod communication_feed;
mod crud_screens;
mod dashboard;
mod kanban_board;
mod landing;
mod settings_screen;
mod util;
mod wizard;
mod workflow;

use nirdosha_contract_core as cc;
use proc_macro::TokenStream;
use quote::quote;
use syn::spanned::Spanned;

#[proc_macro_attribute]
pub fn contract(attr: TokenStream, item: TokenStream) -> TokenStream {
    expand(attr.into(), item).into()
}

/// See `categorical::expand`'s module doc — one `#[contract(requires(
/// role = ..))]`-gated function per categorical value, plus a derived,
/// non-authoritative `role_for_<field>` projection.
#[proc_macro]
pub fn categorical_actions(input: TokenStream) -> TokenStream {
    categorical::expand(input)
}

/// See `dashboard::expand`'s module doc — RFC 0009 Track C Phase 0.
#[proc_macro]
pub fn dashboard(input: TokenStream) -> TokenStream {
    dashboard::expand(input)
}

/// See `crud_screens::expand`'s module doc — RFC 0009 Track C's
/// CRUD/List/Detail/Form/Delete archetypes, attached to a datasource.
#[proc_macro]
pub fn crud_screens(input: TokenStream) -> TokenStream {
    crud_screens::expand(input)
}

/// See `settings_screen::expand`'s module doc — RFC 0009 Track C's
/// Settings archetype: a single always-present record, no list/create/
/// delete, attached to a singleton datasource.
#[proc_macro]
pub fn settings_screen(input: TokenStream) -> TokenStream {
    settings_screen::expand(input)
}

/// See `wizard::expand`'s module doc — RFC 0009 Track C's Workflow/
/// Wizard archetype: a multi-step form with server-side, per-run state.
#[proc_macro]
pub fn wizard(input: TokenStream) -> TokenStream {
    wizard::expand(input)
}

/// See `kanban_board::expand`'s module doc — RFC 0009 Track C's
/// Board/Canvas archetype: a presentational drag-and-drop board.
#[proc_macro]
pub fn kanban_board(input: TokenStream) -> TokenStream {
    kanban_board::expand(input)
}

/// See `communication_feed::expand`'s module doc — RFC 0009 Track C's
/// Communication archetype: an append-only feed with timer refresh or
/// opt-in bounded long polling (`long_poll_seconds: 1..=30`).
#[proc_macro]
pub fn communication_feed(input: TokenStream) -> TokenStream {
    communication_feed::expand(input)
}

/// See `landing::expand`'s module doc — issue #70's substitute for
/// `.nir`'s `landing { .. }` top-level form: first-match-wins
/// `role(..) -> target` rules picking a post-login redirect, with a
/// required `default` catch-all.
#[proc_macro]
pub fn landing(input: TokenStream) -> TokenStream {
    landing::expand(input)
}

/// See `workflow::expand`'s module doc — issue #70's "item 4" general
/// state-machine substitute for `.nir`'s `workflow { .. }` top-level
/// form, and issue #73's macro-time conformance check against an
/// extracted spec via its optional `against = "spec.json"` clause.
#[proc_macro]
pub fn workflow(input: TokenStream) -> TokenStream {
    workflow::expand(input)
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
    if let Some(role) = contract.requires.as_ref().and_then(|r| r.role.as_deref()) {
        let ident = match cc::role::role_ident(role, fn_item.sig.ident.span()) {
            Ok(i) => i,
            Err(e) => return e.to_compile_error(),
        };
        let param: syn::FnArg =
            syn::parse_quote!(__nirdosha_role_proof: &::nirdosha_rt::RoleProof<crate::nirdosha_roles::#ident>);
        fn_item.sig.inputs.insert(0, param);
    }

    // --- capability injection: requires(claim) becomes an unforgeable proof parameter ---
    // Same mechanism as `requires(role)` above, mirrored exactly — the
    // one difference is the type ident is mechanically derived from
    // *two* strings (`cc::claim::claim_ident`), not one.
    if let Some((name, value)) = contract.requires.as_ref().and_then(|r| r.claim.as_ref()) {
        let ident = match cc::claim::claim_ident(name, value, fn_item.sig.ident.span()) {
            Ok(i) => i,
            Err(e) => return e.to_compile_error(),
        };
        let param: syn::FnArg =
            syn::parse_quote!(__nirdosha_claim_proof: &::nirdosha_rt::ClaimProof<crate::nirdosha_claims::#ident>);
        fn_item.sig.inputs.insert(0, param);
    }

    // --- requires(expr)/ensures(expr) (issue #68): a dead sibling fn with
    // this fn's own original parameter list (and, for ensures, a `result:
    // <ReturnType>` parameter) whose body is just the predicate. It is
    // never called -- Stage 2's MIR pass does the real Z3 proof -- but it
    // forces plain rustc to type-check the predicate against this fn's
    // real parameter/return types right now, at macro-expansion time: an
    // undeclared identifier or a non-boolean expression is a real,
    // well-spanned compile_error!-grade diagnostic under *both*
    // compilers, exactly like every other contract clause here.
    let original_inputs = fn_item.sig.inputs.clone();
    let generics = &fn_item.sig.generics;
    let where_clause = &fn_item.sig.generics.where_clause;
    let mut predicate_checkers = Vec::new();
    if let Some(expr_src) = contract.requires.as_ref().and_then(|r| r.expr.as_deref()) {
        let expr: syn::Expr = match syn::parse_str(expr_src) {
            Ok(e) => e,
            Err(e) => return e.to_compile_error(),
        };
        let checker_name =
            quote::format_ident!("__nirdosha_requires_check_{}", fn_item.sig.ident);
        predicate_checkers.push(quote! {
            #[allow(dead_code, unused_variables)]
            fn #checker_name #generics(#original_inputs) -> bool #where_clause { #expr }
        });
    }
    if let Some(ensures) = &contract.ensures {
        let expr: syn::Expr = match syn::parse_str(&ensures.expr) {
            Ok(e) => e,
            Err(e) => return e.to_compile_error(),
        };
        let ret_ty: syn::Type = match &fn_item.sig.output {
            syn::ReturnType::Default => syn::parse_quote!(()),
            syn::ReturnType::Type(_, ty) => (**ty).clone(),
        };
        let checker_name =
            quote::format_ident!("__nirdosha_ensures_check_{}", fn_item.sig.ident);
        predicate_checkers.push(quote! {
            #[allow(dead_code, unused_variables)]
            fn #checker_name #generics(#original_inputs, result: #ret_ty) -> bool #where_clause { #expr }
        });
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

    // --- policy cross-check: crud(op, policy) becomes a real const assertion,
    // not a doc-comment scanner. rustc's own const-evaluator refuses the
    // build if the named policy forbids the named operation — see
    // docs/nirdosha-rt-dialect.md's "no bespoke parser, ever" rule.
    let crud_assertion = match &contract.crud {
        Some(crud) => {
            let policy_ident = match cc::role::role_ident(&crud.policy, fn_item.sig.ident.span()) {
                Ok(i) => i,
                Err(e) => return e.to_compile_error(),
            };
            let op_str = &crud.op;
            let message = format!(
                "policy `{}` forbids the `{op_str}` operation, but `{fn_name}` declares crud(op = \"{op_str}\", policy = \"{}\")",
                crud.policy, crud.policy
            );
            quote! {
                const _: () = assert!(
                    !::nirdosha_rt::crud_forbidden::<crate::nirdosha_policies::#policy_ident>(#op_str),
                    #message
                );
            }
        }
        None => quote!(),
    };

    // --- portable encoding: the contract rides into rustdoc as a doc attribute ---
    let doc = contract.doc_string();
    quote! {
        #crud_assertion
        #(#predicate_checkers)*
        #[doc = #doc]
        #fn_item
    }
}
