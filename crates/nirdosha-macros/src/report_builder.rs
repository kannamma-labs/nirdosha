//! `report_builder! { .. }` — an ad-hoc aggregate report over one
//! `GuardedTable` (RTM's 15.2 "Ad-Hoc Report Builder" archetype).
//!
//! ```ignore
//! nirdosha_rt::report_builder! {
//!     mount: mount_ticket_report,
//!     entity: TicketRow,
//!     table: ticket_table,
//!     path: "/reports/tickets",
//!     title: "Ticket Report",
//!     purpose: "Operations",
//!     access: requires role "Agent",
//!     dimensions: [ status: String, priority: String ],
//! }
//! ```
//!
//! `GET <path>` renders a dimension picker; `POST <path>` runs the
//! report: `GuardedTable::guarded_aggregate` (the **aggregate** guard
//! action — the screen's corpus must carry an `action == "aggregate"`
//! allow record for this resource and purpose) decodes the visible
//! rows with the guard's masks applied, and the handler counts
//! per-group over the requested dimension in-process. Counts only:
//! PII auto-masked in aggregates (15.2's register note), and a masked
//! dimension groups into its mask value rather than leaking the
//! underlying values — the same "mask, then render" order every
//! guarded read already goes through.
//!
//! **There is no unguarded path.** This macro is born after the
//! access-declaration duplication was removed, so `table:` must be a
//! real `GuardedTable` constructor and the guard is the single
//! authority for whether a report runs at all: no allow record for
//! `(subject, action = "aggregate", resource, purpose)` is a plain
//! guard deny. The declared `access:` is vestigial route metadata
//! (OpenAPI summaries), exactly like every other guarded screen.
//!
//! v1 scope, disclosed: one dimension per run, count metric only, and
//! dimensions must be `String`-typed fields of the entity (categorical
//! group keys). Date-range/filters, saved reports, and CSV egress are
//! the archetype's later increments.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::{Ident, LitStr, Token, Type};

use syn::spanned::Spanned;


enum Access {
    Public,
    Role(LitStr),
}

impl Parse for Access {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let ident = Ident::parse_any(input)?;
        if ident == "public" {
            return Ok(Access::Public);
        }
        if ident == "requires" {
            let role_kw = Ident::parse_any(input)?;
            if role_kw != "role" {
                return Err(syn::Error::new(role_kw.span(), "expected `role`"));
            }
            let role: LitStr = input.parse()?;
            if let Err(e) = nirdosha_contract_core::role::role_ident(&role.value(), role.span()) {
                return Err(e);
            }
            return Ok(Access::Role(role));
        }
        Err(syn::Error::new(ident.span(), "expected `public` or `requires role \"...\"`"))
    }
}

pub struct ReportBuilderInput {
    mount: Ident,
    entity: Type,
    table: Ident,
    path: LitStr,
    title: LitStr,
    purpose: LitStr,
    access: Access,
    dimensions: Vec<FieldDef>,
}

struct FieldDef {
    name: Ident,
    ty: Type,
}

impl Parse for ReportBuilderInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut mount: Option<Ident> = None;
        let mut entity: Option<Type> = None;
        let mut table: Option<Ident> = None;
        let mut path: Option<LitStr> = None;
        let mut title: Option<LitStr> = None;
        let mut purpose: Option<LitStr> = None;
        let mut access: Option<Access> = None;
        let mut dimensions: Option<Vec<FieldDef>> = None;

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            input.parse::<Token![:]>()?;
            match key.to_string().as_str() {
                "mount" => mount = Some(input.parse()?),
                "entity" => entity = Some(input.parse()?),
                "table" => table = Some(input.parse()?),
                "path" => path = Some(input.parse()?),
                "title" => title = Some(input.parse()?),
                "purpose" => purpose = Some(input.parse()?),
                "access" => access = Some(input.parse()?),
                "dimensions" => {
                    let content;
                    syn::bracketed!(content in input);
                    let mut fields = Vec::new();
                    while !content.is_empty() {
                        let name: Ident = content.parse()?;
                        content.parse::<Token![:]>()?;
                        let ty: Type = content.parse()?;
                        // v1, disclosed: group keys are collected as
                        // `String` (masked values stay string-shaped by
                        // design), so a non-String dimension would turn
                        // every group into its Display rendering —
                        // refused here rather than silently accepted.
                        if quote!(#ty).to_string() != "String" {
                            return Err(syn::Error::new(ty.span(), "dimensions must be `String`-typed fields in v1 (categorical group keys)"));
                        }
                        fields.push(FieldDef { name, ty });
                        if content.peek(Token![,]) {
                            content.parse::<Token![,]>()?;
                        }
                    }
                    dimensions = Some(fields);
                }
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("expected `mount`, `entity`, `table`, `path`, `title`, `purpose`, `access`, or `dimensions`, found `{other}`"),
                    ))
                }
            }
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }

        let mount = mount.ok_or_else(|| syn::Error::new(input.span(), "`mount:` is required"))?;
        let entity = entity.ok_or_else(|| syn::Error::new(input.span(), "`entity:` is required"))?;
        let table = table.ok_or_else(|| syn::Error::new(input.span(), "`table:` is required (a GuardedTable constructor — there is no unguarded path)"))?;
        let path = path.ok_or_else(|| syn::Error::new(input.span(), "`path:` is required"))?;
        let title = title.ok_or_else(|| syn::Error::new(input.span(), "`title:` is required"))?;
        let purpose = purpose.ok_or_else(|| syn::Error::new(input.span(), "`purpose:` is required (the aggregate policy's purpose)"))?;
        let access = access.unwrap_or(Access::Public);
        let dimensions = dimensions.ok_or_else(|| syn::Error::new(input.span(), "`dimensions: [ name: Type, ... ]` is required (at least one String-typed field)"))?;
        if dimensions.is_empty() {
            return Err(syn::Error::new(input.span(), "report_builder! needs at least one dimension"));
        }
        Ok(ReportBuilderInput { mount, entity, table, path, title, purpose, access, dimensions })
    }
}

pub fn expand(input: TokenStream) -> TokenStream {
    let parsed = match syn::parse::<ReportBuilderInput>(input) {
        Ok(p) => p,
        Err(e) => return e.to_compile_error().into(),
    };

    let ReportBuilderInput { mount, entity, table, path, title, purpose, access, dimensions } = &parsed;
    let dim_names: Vec<String> = dimensions.iter().map(|d| d.name.to_string()).collect();
    let dim_idents: Vec<&Ident> = dimensions.iter().map(|d| &d.name).collect();

    // `entity:` is load-bearing as a type-level contract: it must be a
    // real `GuardedEntity` (the same type `table:` stores) — a mismatch
    // between the two is a compile error here, not a runtime surprise.
    let entity_assert = quote! {
        const _: () = {
            fn assert_guarded_entity<E: ::nirdosha_guard_screens::GuardedEntity>() {}
            fn _check() {
                assert_guarded_entity::<#entity>();
            }
        };
    };

    // The declared `access:` is vestigial route metadata on a guarded
    // screen; both routes resolve an `Auth` and the guard corpus
    // decides — so both routes are `*_with_auth` regardless of it.
    let _ = access;

    let dims_helper = {
        let lits: Vec<TokenStream2> = dim_names.iter().map(|n| quote! { #n }).collect();
        quote! {
            fn __report_dimensions() -> Vec<&'static str> {
                vec![ #(#lits),* ]
            }
        }
    };

    let run_arms = {
        let mut arms = TokenStream2::new();
        for (name, ident) in dim_names.iter().zip(dim_idents.iter()) {
            arms.extend(quote! {
                #name => {
                    for row in rows.iter() {
                        *counts.entry(row.#ident.clone()).or_insert(0) += 1;
                    }
                }
            });
        }
        arms
    };

    quote! {
        #entity_assert
        pub fn #mount(router: ::nirdosha_rt::Router) -> ::nirdosha_rt::Router {
            #dims_helper
            router
                .get_with_auth(#path, #title, |_req, _params, _auth| {
                    nirdosha_rt::Response::html(200, nirdosha_rt::screens::report_form_html(#title, #path, &__report_dimensions()))
                })
                .post_with_auth(#path, "Run report", |req, _params, auth| {
                    let values = req.form_or_json();
                    let Some(dimension) = values.get("dimension").map(|s| s.as_str()).filter(|d| !d.is_empty()) else {
                        return nirdosha_rt::Response::bad_request("pick a dimension");
                    };
                    let rows = match #table().guarded_aggregate(auth, #purpose) {
                        Ok(rows) => rows,
                        Err(e) => return ::nirdosha_guard_screens::guard_error_response(e),
                    };
                    // Counts only, group keys as the guard decoded them:
                    // PII auto-masked in aggregates — a masked dimension
                    // groups into its mask value, and a forbidden
                    // (dropped) field can never be grouped on at all
                    // because the decoded row never carries it.
                    let mut counts: ::std::collections::BTreeMap<String, u64> = ::std::collections::BTreeMap::new();
                    match dimension {
                        #run_arms
                        _ => return ::nirdosha_rt::Response::bad_request("unknown dimension"),
                    }
                    let sorted: Vec<(String, u64)> = counts.into_iter().collect();
                    nirdosha_rt::Response::html(200, nirdosha_rt::screens::aggregate_table_html(#title, #path, dimension, &sorted))
                })
        }
    }
    .into()
}