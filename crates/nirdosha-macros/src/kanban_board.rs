//! `kanban_board! { .. }` — RFC 0009 Track C's Board/Canvas archetype:
//! a drag-and-drop board grouping an existing datasource's rows by one
//! categorical field. Presentational only — it never regenerates
//! mutation logic. A card's move is a `fetch()` POST (from the one
//! hand-written JS asset `nirdosha_rt::board::board_js` renders) to a
//! move endpoint the app already registered, typically the same
//! `#[contract(requires(role=...))]`-gated function
//! `categorical_actions!` generates, wired to a route by hand exactly
//! as the flagship demo does today for approve/disapprove. The board
//! itself carries no authorization logic — the security boundary is
//! entirely the app's existing move endpoint.
//!
//! ```ignore
//! nirdosha_rt::kanban_board! {
//!     mount: mount_status_board,
//!     entity: Product,
//!     store: product_store,
//!     path: "/board",
//!     access: public,
//!     title_field: name,
//!     column_field: status,
//!     columns: [ "backlog", "in_progress", "done" ],
//!     move_path: "/products/{id}/move/{to}",
//! }
//! ```

use crate::util::expect_keyword;
use nirdosha_contract_core::role::role_ident;
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{bracketed, Ident, LitStr, Token, Type};

enum Access {
    Public,
    Role(LitStr),
}

impl Parse for Access {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        use syn::ext::IdentExt;
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
            // Validated the same way the ident gets built later
            // (`role_ident` at codegen time) -- accepts snake_case wire
            // names and bare PascalCase role type names (RTM's
            // `roles! { role Analyst; }` convention) alike.
            if let Err(e) = nirdosha_contract_core::role::role_ident(&role.value(), role.span()) {
                return Err(e);
            }
            return Ok(Access::Role(role));
        }
        Err(syn::Error::new(ident.span(), "expected `public` or `requires role \"...\"`"))
    }
}

/// Optional `guard: { table: <ident>, purpose: "..." }` -- same shape
/// and rationale as `crud_screens!`'s own `GuardConfig` (see that
/// file's doc comment): when present, the board's view route reads
/// through `nirdosha_guard_screens::GuardedTable::guarded_snapshot`
/// (real per-request policy evaluation, tenant scoping, field masking)
/// instead of `#store().snapshot()`. The move endpoint stays exactly
/// what this macro's own module doc comment already describes --
/// presentational only, hand-wired by the app -- `guard:` only changes
/// where the *cards themselves* come from, not the drag-and-drop
/// mutation path.
struct GuardConfig {
    table: Ident,
    purpose: LitStr,
}

impl Parse for GuardConfig {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        expect_keyword(input, "table")?;
        input.parse::<Token![:]>()?;
        let table: Ident = input.parse()?;
        input.parse::<Token![,]>()?;
        expect_keyword(input, "purpose")?;
        input.parse::<Token![:]>()?;
        let purpose: LitStr = input.parse()?;
        let _ = input.parse::<Token![,]>();
        Ok(GuardConfig { table, purpose })
    }
}

struct KanbanInput {
    mount: Ident,
    // Not used in codegen (card fields are read generically via
    // `.to_string()`, needing no `#entity`-typed code), but required in
    // the DSL for consistency with `crud_screens!`/`wizard!` and so a
    // reader can see at a glance which datasource this board is over.
    #[allow(dead_code)]
    entity: Type,
    store: Ident,
    path: LitStr,
    access: Access,
    title_field: Ident,
    column_field: Ident,
    columns: Vec<LitStr>,
    move_path: LitStr,
    guard: Option<GuardConfig>,
    /// Optional `machine: "Name"` (T-14) — the `workflow!`-registered
    /// state machine (e.g. `"CaseStatus"`) whose real transition graph
    /// gates drags: a card's current column derives its allowed drop
    /// columns from `nirdosha_guard_registry::workflow_allowed_transitions`,
    /// never a second, driftable `transitions:` literal at the call site.
    machine: Option<LitStr>,
}

impl Parse for KanbanInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        expect_keyword(input, "mount")?;
        input.parse::<Token![:]>()?;
        let mount: Ident = input.parse()?;
        input.parse::<Token![,]>()?;

        expect_keyword(input, "entity")?;
        input.parse::<Token![:]>()?;
        let entity: Type = input.parse()?;
        input.parse::<Token![,]>()?;

        expect_keyword(input, "store")?;
        input.parse::<Token![:]>()?;
        let store: Ident = input.parse()?;
        input.parse::<Token![,]>()?;

        expect_keyword(input, "path")?;
        input.parse::<Token![:]>()?;
        let path: LitStr = input.parse()?;
        input.parse::<Token![,]>()?;

        expect_keyword(input, "access")?;
        input.parse::<Token![:]>()?;
        let access: Access = input.parse()?;
        input.parse::<Token![,]>()?;

        expect_keyword(input, "title_field")?;
        input.parse::<Token![:]>()?;
        let title_field: Ident = input.parse()?;
        input.parse::<Token![,]>()?;

        expect_keyword(input, "column_field")?;
        input.parse::<Token![:]>()?;
        let column_field: Ident = input.parse()?;
        input.parse::<Token![,]>()?;

        expect_keyword(input, "columns")?;
        input.parse::<Token![:]>()?;
        let content;
        bracketed!(content in input);
        let mut columns = Vec::new();
        while !content.is_empty() {
            columns.push(content.parse::<LitStr>()?);
            if content.peek(Token![,]) {
                content.parse::<Token![,]>()?;
            }
        }
        if columns.is_empty() {
            return Err(syn::Error::new(content.span(), "kanban_board! needs at least one column"));
        }
        input.parse::<Token![,]>()?;

        expect_keyword(input, "move_path")?;
        input.parse::<Token![:]>()?;
        let move_path: LitStr = input.parse()?;
        if !move_path.value().contains("{id}") || !move_path.value().contains("{to}") {
            return Err(syn::Error::new(move_path.span(), "move_path must contain both `{id}` and `{to}` placeholders"));
        }
        let _ = input.parse::<Token![,]>();

        // Additive-only: every existing `kanban_board!` invocation ends
        // right here (no trailing content), so this only ever fires for
        // an invocation that opted in -- same convention as
        // `crud_screens!`'s identical `guard:` parse. `guard:` and
        // `machine:` are independent opt-ins and may appear in either
        // order, so this loops over trailing keyed clauses instead of
        // checking one fixed keyword once.
        let mut guard = None;
        let mut machine = None;
        loop {
            if !input.peek(Ident) {
                break;
            }
            let fork = input.fork();
            let ahead: Ident = fork.parse()?;
            if ahead == "guard" {
                input.parse::<Ident>()?;
                input.parse::<Token![:]>()?;
                let content;
                syn::braced!(content in input);
                guard = Some(content.parse::<GuardConfig>()?);
                let _ = input.parse::<Token![,]>();
            } else if ahead == "machine" {
                input.parse::<Ident>()?;
                input.parse::<Token![:]>()?;
                machine = Some(input.parse::<LitStr>()?);
                let _ = input.parse::<Token![,]>();
            } else {
                break;
            }
        }

        Ok(KanbanInput { mount, entity, store, path, access, title_field, column_field, columns, move_path, guard, machine })
    }
}

pub fn expand(input: TokenStream) -> TokenStream {
    let parsed = match syn::parse::<KanbanInput>(input) {
        Ok(p) => p,
        Err(e) => return e.to_compile_error().into(),
    };
    expand_parsed(parsed).into()
}

fn expand_parsed(input: KanbanInput) -> TokenStream2 {
    let mount = &input.mount;
    let store = &input.store;
    let path_str = input.path.value();
    let path = quote! { #path_str };
    let title = path_str.trim_start_matches('/').to_string();
    let title_field = &input.title_field;
    let column_field = &input.column_field;
    let columns = &input.columns;
    let move_path = &input.move_path;
    let asset_path_str = format!("{}/board.js", path_str.trim_end_matches('/'));
    let asset_path = quote! { #asset_path_str };

    // Computed once per request, from the real `workflow!`-registered
    // machine -- never a second literal transition list at the call
    // site. `None` (no `machine:` clause) renders exactly as every
    // pre-T-14 board did: no `data-allowed-to`, no graying.
    let allowed_binding = match &input.machine {
        Some(name) => quote! {
            let __nirdosha_allowed: Option<::std::collections::HashMap<String, Vec<String>>> =
                Some(::nirdosha_guard_registry::workflow_allowed_transitions(#name));
        },
        None => quote! {
            let __nirdosha_allowed: Option<::std::collections::HashMap<String, Vec<String>>> = None;
        },
    };

    // `guard:` swaps the card source for a real, per-request guard-policy
    // read (`GuardedTable::guarded_snapshot`) instead of the ungated
    // `#store().snapshot()` -- same posture as `crud_screens!`'s own
    // `guard:` clause on its list route. Rows failing the read policy
    // (e.g. no matching role/purpose grant) surface as the same
    // `guard_error_response` every other guarded route returns, not an
    // empty board indistinguishable from "no cards yet".
    let view_route = if let Some(guard) = &input.guard {
        let table = &guard.table;
        let purpose = &guard.purpose;
        quote! {
            .get_with_auth(#path, #title, |_req, _params, auth| {
                match #table().guarded_snapshot(auth, #purpose) {
                    Ok(matched) => {
                        let cards: Vec<::nirdosha_rt::board::Card> = matched
                            .into_iter()
                            .map(|entity| ::nirdosha_rt::board::Card {
                                id: ::nirdosha_guard_screens::GuardedEntity::row_id(&entity),
                                title: entity.#title_field.to_string(),
                                column: entity.#column_field.to_string(),
                            })
                            .collect();
                        let columns: &[&str] = &[ #(#columns),* ];
                        #allowed_binding
                        ::nirdosha_rt::Response::html(200, ::nirdosha_rt::board::board_html(#title, columns, &cards, #asset_path, __nirdosha_allowed.as_ref()))
                    }
                    Err(e) => ::nirdosha_guard_screens::guard_error_response(e),
                }
            })
        }
    } else {
        let view_body = quote! {
            let cards: Vec<::nirdosha_rt::board::Card> = #store()
                .snapshot()
                .into_iter()
                .map(|(_, entity)| ::nirdosha_rt::board::Card {
                    id: entity.id.to_string(),
                    title: entity.#title_field.to_string(),
                    column: entity.#column_field.to_string(),
                })
                .collect();
            let columns: &[&str] = &[ #(#columns),* ];
            #allowed_binding
            ::nirdosha_rt::Response::html(200, ::nirdosha_rt::board::board_html(#title, columns, &cards, #asset_path, __nirdosha_allowed.as_ref()))
        };
        match &input.access {
            Access::Public => quote! { .get(#path, #title, |_req, _params| { #view_body }) },
            Access::Role(role) => {
                let role_ident = role_ident(&role.value(), role.span()).expect("role name already validated at parse time");
                quote! { .get_gated::<crate::nirdosha_roles::#role_ident>(#path, #title, |_req, _params, _proof| { #view_body }) }
            }
        }
    };

    quote! {
        // `pub`, matching `login!`/`app_shell!`/`crud_screens!`'s generated
        // mount fns -- a private mount fn only ever worked because every
        // existing caller invoked the macro and called the result in the
        // same file; a real multi-module app (screens split one-per-file)
        // needs to call it from outside that module.
        pub fn #mount(router: ::nirdosha_rt::Router) -> ::nirdosha_rt::Router {
            router
                #view_route
                .get(#asset_path, "Board drag-and-drop script", |_req, _params| {
                    ::nirdosha_rt::Response::javascript(200, ::nirdosha_rt::board::board_js(#move_path))
                })
        }
    }
}
