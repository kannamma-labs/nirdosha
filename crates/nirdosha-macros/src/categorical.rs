//! `categorical_actions! { .. }` — one `#[contract(requires(role = ..))]`-
//! gated function per value of a categorical field (RFC 0020 addendum:
//! "some domains forbid delete" generalizes to "this transition needs
//! this role").
//!
//! ```ignore
//! nirdosha_rt::categorical_actions! {
//!     entity: Product,
//!     store: product_store,
//!     field: is_approved: bool,
//!     actions {
//!         true  => approve_product    requires role "approver",
//!         false => disapprove_product requires role "compliance_officer",
//!     }
//! }
//! ```
//!
//! Road 1 / Road 2, same distinction as `docs/nirdosha-rt-dialect.md`'s
//! "no bespoke parser" rule demands: the generated `approve_product`/
//! `disapprove_product` functions are the *only* thing that decides
//! access — real `#[contract(requires(role = ..))]`, the same
//! unforgeable `RoleProof<R>` mechanism already proven elsewhere. The
//! second thing this macro emits, `role_for_is_approved`, is a
//! **derived, non-authoritative** projection: a real `match` built
//! from the exact same arms, useful for audit logs and UI hints, never
//! consulted for authorization. Because it's a genuine `match` over the
//! field's own type, leaving a value uncovered is `rustc`'s own
//! `error[E0004]: non-exhaustive patterns` — the coverage guarantee is
//! `rustc`, not a scanner.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::{braced, Ident, LitStr, Pat, Token, Type};

struct ActionArm {
    pat: Pat,
    fn_name: Ident,
    role: LitStr,
}

struct CategoricalInput {
    entity: Type,
    store: Ident,
    field: Ident,
    field_ty: Type,
    actions: Vec<ActionArm>,
}

use crate::util::expect_keyword;

impl Parse for CategoricalInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        expect_keyword(input, "entity")?;
        input.parse::<Token![:]>()?;
        let entity: Type = input.parse()?;
        input.parse::<Token![,]>()?;

        expect_keyword(input, "store")?;
        input.parse::<Token![:]>()?;
        let store: Ident = input.parse()?;
        input.parse::<Token![,]>()?;

        expect_keyword(input, "field")?;
        input.parse::<Token![:]>()?;
        let field: Ident = input.parse()?;
        input.parse::<Token![:]>()?;
        let field_ty: Type = input.parse()?;
        input.parse::<Token![,]>()?;

        expect_keyword(input, "actions")?;
        let content;
        braced!(content in input);
        let mut actions = Vec::new();
        while !content.is_empty() {
            let pat = Pat::parse_single(&content)?;
            content.parse::<Token![=>]>()?;
            let fn_name: Ident = content.parse()?;
            expect_keyword(&content, "requires")?;
            expect_keyword(&content, "role")?;
            let role: LitStr = content.parse()?;
            actions.push(ActionArm { pat, fn_name, role });
            if content.peek(Token![,]) {
                content.parse::<Token![,]>()?;
            }
        }
        if actions.is_empty() {
            return Err(syn::Error::new(
                content.span(),
                "categorical_actions! needs at least one `pattern => fn_name requires role \"..\"` arm",
            ));
        }
        // Optional trailing comma after the `actions { .. }` block.
        let _ = input.parse::<Token![,]>();
        Ok(CategoricalInput {
            entity,
            store,
            field,
            field_ty,
            actions,
        })
    }
}

pub fn expand(input: TokenStream) -> TokenStream {
    let parsed = match syn::parse::<CategoricalInput>(input) {
        Ok(p) => p,
        Err(e) => return e.to_compile_error().into(),
    };
    expand_parsed(parsed).into()
}

fn expand_parsed(input: CategoricalInput) -> TokenStream2 {
    let entity = &input.entity;
    let store = &input.store;
    let field = &input.field;
    let field_ty = &input.field_ty;

    // Road 1: authoritative. One independently role-gated function per
    // value, using the exact same `#[contract(requires(role = ..))]`
    // mechanism as everything else in the dialect. The store is the
    // dialect's managed primitive (`nirdosha_rt::prelude::SharedTable`)
    // — the dialect-wide raw-lock deny (`nirdosha-contract-core/src/
    // scan.rs::LOCK_DENIES`) forbids a `Mutex`-backed store, so the
    // generated code goes through `update`'s complete-method critical
    // section, never a hand-written `.lock()`.
    let road1 = input.actions.iter().map(|arm| {
        let pat = &arm.pat;
        let fn_name = &arm.fn_name;
        let role = &arm.role;
        quote! {
            #[nirdosha_rt::contract(requires(role = #role))]
            fn #fn_name(id: i64) -> Result<#entity, &'static str> {
                #store().update(&id, |__nirdosha_entity| {
                    let __nirdosha_entity = __nirdosha_entity.ok_or("not found")?;
                    __nirdosha_entity.#field = #pat;
                    Ok(__nirdosha_entity.clone())
                })
            }
        }
    });

    // Road 2: derived, non-authoritative. A real `match` — an
    // uncovered value is rustc's own E0004, not a lint.
    let match_arms = input.actions.iter().map(|arm| {
        let pat = &arm.pat;
        let role = &arm.role;
        quote! { #pat => #role, }
    });
    let role_for_fn = format_ident!("role_for_{field}");

    quote! {
        #(#road1)*

        /// Derived from the same declaration as the gated functions
        /// above — never consulted for authorization. See RFC 0020
        /// Road 1 / Road 2.
        #[allow(dead_code)]
        fn #role_for_fn(value: #field_ty) -> &'static str {
            match value {
                #(#match_arms)*
            }
        }
    }
}
