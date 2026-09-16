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
            if let Err(msg) = nirdosha_contract_core::role::validate_role_name(&role.value()) {
                return Err(syn::Error::new(role.span(), msg));
            }
            return Ok(Access::Role(role));
        }
        Err(syn::Error::new(ident.span(), "expected `public` or `requires role \"...\"`"))
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

        Ok(KanbanInput { mount, entity, store, path, access, title_field, column_field, columns, move_path })
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

    let view_body = quote! {
        let store = #store().lock().unwrap();
        let cards: Vec<::nirdosha_rt::board::Card> = store.values().map(|entity| ::nirdosha_rt::board::Card {
            id: entity.id,
            title: entity.#title_field.to_string(),
            column: entity.#column_field.to_string(),
        }).collect();
        let columns: &[&str] = &[ #(#columns),* ];
        ::nirdosha_rt::Response::html(200, ::nirdosha_rt::board::board_html(#title, columns, &cards, #asset_path))
    };

    let view_route = match &input.access {
        Access::Public => quote! { .get(#path, #title, |_req, _params| { #view_body }) },
        Access::Role(role) => {
            let role_ident = role_ident(&role.value(), role.span()).expect("role name already validated at parse time");
            quote! { .get_gated::<crate::nirdosha_roles::#role_ident>(#path, #title, |_req, _params, _proof| { #view_body }) }
        }
    };

    quote! {
        fn #mount(router: ::nirdosha_rt::Router) -> ::nirdosha_rt::Router {
            router
                #view_route
                .get(#asset_path, "Board drag-and-drop script", |_req, _params| {
                    ::nirdosha_rt::Response::javascript(200, ::nirdosha_rt::board::board_js(#move_path))
                })
        }
    }
}
