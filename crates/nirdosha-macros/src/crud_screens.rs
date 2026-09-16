//! `crud_screens! { .. }` — RFC 0009 Track C: List/Detail/Create-Form/
//! Edit-Form/Delete-confirmation, attached to a real datasource, for
//! both a browser (`<form>` POSTs, server-rendered HTML) and a JSON
//! API caller — the same generated functions serve both. A typo'd
//! field name is an ordinary `rustc` "no such field" error; a field
//! type with no `ParseField` impl is an ordinary "trait bound not
//! satisfied" error. Nothing here is checked by a separate tool.
//!
//! ```ignore
//! nirdosha_rt::crud_screens! {
//!     mount: mount_product_screens,
//!     entity: Product,
//!     store: product_store,
//!     path: "/products",
//!     fields: [ name: String, price_cents: i64, cost_cents: i64 ],
//!     create: requires role "admin",
//!     read: public,
//!     update: requires role "admin",
//!     delete: requires role "admin",
//! }
//! ```
//!
//! `Entity` must derive `Clone`, `Default`, and `serde::Serialize` —
//! `Default` covers every struct field this macro's `fields:` list
//! doesn't mention (e.g. a field a `categorical_actions!` transition
//! owns instead), the same way `..Default::default()` works in
//! ordinary hand-written Rust.

use crate::util::expect_keyword;
use nirdosha_contract_core::role::role_ident;
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::{bracketed, Ident, LitStr, Token, Type};

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
            if let Err(msg) = nirdosha_contract_core::role::validate_role_name(&role.value()) {
                return Err(syn::Error::new(role.span(), msg));
            }
            return Ok(Access::Role(role));
        }
        Err(syn::Error::new(ident.span(), "expected `public` or `requires role \"...\"`"))
    }
}

struct FieldDef {
    name: Ident,
    ty: Type,
}

struct CrudScreensInput {
    mount: Ident,
    entity: Type,
    store: Ident,
    path: LitStr,
    fields: Vec<FieldDef>,
    create: Access,
    read: Access,
    update: Access,
    delete: Access,
}

impl Parse for CrudScreensInput {
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

        expect_keyword(input, "fields")?;
        input.parse::<Token![:]>()?;
        let content;
        bracketed!(content in input);
        let mut fields = Vec::new();
        while !content.is_empty() {
            let name: Ident = content.parse()?;
            content.parse::<Token![:]>()?;
            let ty: Type = content.parse()?;
            fields.push(FieldDef { name, ty });
            if content.peek(Token![,]) {
                content.parse::<Token![,]>()?;
            }
        }
        if fields.is_empty() {
            return Err(syn::Error::new(content.span(), "crud_screens! needs at least one field"));
        }
        input.parse::<Token![,]>()?;

        expect_keyword(input, "create")?;
        input.parse::<Token![:]>()?;
        let create: Access = input.parse()?;
        input.parse::<Token![,]>()?;

        expect_keyword(input, "read")?;
        input.parse::<Token![:]>()?;
        let read: Access = input.parse()?;
        input.parse::<Token![,]>()?;

        expect_keyword(input, "update")?;
        input.parse::<Token![:]>()?;
        let update: Access = input.parse()?;
        input.parse::<Token![,]>()?;

        expect_keyword(input, "delete")?;
        input.parse::<Token![:]>()?;
        let delete: Access = input.parse()?;
        let _ = input.parse::<Token![,]>();

        Ok(CrudScreensInput {
            mount,
            entity,
            store,
            path,
            fields,
            create,
            read,
            update,
            delete,
        })
    }
}

fn input_type_for(ty: &Type) -> &'static str {
    match quote!(#ty).to_string().as_str() {
        "bool" => "checkbox",
        "i64" | "i32" | "u32" | "u64" | "f64" | "f32" | "usize" | "isize" => "number",
        _ => "text",
    }
}

/// Builds one `.get(..)`/`.get_gated::<R>(..)` (etc.) registration.
/// `body_fn` receives `Some(quote!(proof))` when the route is gated
/// (so the caller can thread the real proof into a core function) or
/// `None` when public — the one place gated vs. public code paths
/// actually diverge.
fn route(method: &str, access: &Access, path: &TokenStream2, summary: &str, body_fn: impl Fn(Option<TokenStream2>) -> TokenStream2) -> TokenStream2 {
    let (plain, gated) = match method {
        "get" => (quote!(get), quote!(get_gated)),
        "post" => (quote!(post), quote!(post_gated)),
        "put" => (quote!(put), quote!(put_gated)),
        "delete" => (quote!(delete), quote!(delete_gated)),
        _ => unreachable!("internal: unknown HTTP method {method}"),
    };
    match access {
        Access::Public => {
            let body = body_fn(None);
            quote! { .#plain(#path, #summary, |req, params| { #body }) }
        }
        Access::Role(role) => {
            let role_ident = role_ident(&role.value(), role.span()).expect("role name already validated at parse time");
            let body = body_fn(Some(quote!(proof)));
            quote! { .#gated::<crate::nirdosha_roles::#role_ident>(#path, #summary, |req, params, proof| { #body }) }
        }
    }
}

/// `proof,` when gated (to splice as a leading call argument), empty
/// when public — `#proof_ref` is whatever `route`'s `body_fn` was handed.
fn proof_arg(proof_ref: &Option<TokenStream2>) -> TokenStream2 {
    match proof_ref {
        Some(p) => quote! { #p, },
        None => quote! {},
    }
}

pub fn expand(input: TokenStream) -> TokenStream {
    let parsed = match syn::parse::<CrudScreensInput>(input) {
        Ok(p) => p,
        Err(e) => return e.to_compile_error().into(),
    };
    expand_parsed(parsed).into()
}

fn expand_parsed(input: CrudScreensInput) -> TokenStream2 {
    let mount = &input.mount;
    let entity = &input.entity;
    let store = &input.store;
    let path_str = input.path.value();
    let path = quote! { #path_str };
    let api_path_str = format!("/api{path_str}");
    let api_path = quote! { #api_path_str };
    let id_path_str = format!("{path_str}/{{id}}");
    let id_path = quote! { #id_path_str };
    let api_id_path_str = format!("/api{path_str}/{{id}}");
    let api_id_path = quote! { #api_id_path_str };
    let new_path_str = format!("{path_str}/new");
    let new_path = quote! { #new_path_str };
    let edit_path_str = format!("{path_str}/{{id}}/edit");
    let edit_path = quote! { #edit_path_str };
    let delete_path_str = format!("{path_str}/{{id}}/delete");
    let delete_path = quote! { #delete_path_str };

    let title = path_str.trim_start_matches('/').to_string();

    let field_idents: Vec<&Ident> = input.fields.iter().map(|f| &f.name).collect();
    let field_names: Vec<String> = input.fields.iter().map(|f| f.name.to_string()).collect();
    let field_types: Vec<&Type> = input.fields.iter().map(|f| &f.ty).collect();
    let input_types: Vec<&'static str> = input.fields.iter().map(|f| input_type_for(&f.ty)).collect();

    let can_create = matches!(input.create, Access::Public);
    let can_update = matches!(input.update, Access::Public);
    let can_delete = matches!(input.delete, Access::Public);

    let fields_fn = quote! {
        fn __fields() -> Vec<::nirdosha_rt::screens::FieldSpec> {
            vec![ #( ::nirdosha_rt::screens::FieldSpec { name: #field_names, input_type: #input_types } ),* ]
        }
    };

    let parse_fn = quote! {
        fn __parse(values: &::std::collections::HashMap<String, String>) -> ::std::result::Result<#entity, Vec<String>> {
            let mut errors: Vec<String> = Vec::new();
            #(
                let #field_idents = match <#field_types as ::nirdosha_rt::screens::ParseField>::parse_field(values.get(#field_names).map(|s| s.as_str())) {
                    Ok(v) => v,
                    Err(e) => { errors.push(format!("{}: {}", #field_names, e)); Default::default() }
                };
            )*
            if !errors.is_empty() { return Err(errors); }
            Ok(#entity { id: 0, #( #field_idents ),*, ..Default::default() })
        }
    };

    let create_attr = match &input.create {
        Access::Public => quote! {},
        Access::Role(role) => quote! { #[nirdosha_rt::contract(requires(role = #role))] },
    };
    let update_attr = match &input.update {
        Access::Public => quote! {},
        Access::Role(role) => quote! { #[nirdosha_rt::contract(requires(role = #role))] },
    };
    let delete_attr = match &input.delete {
        Access::Public => quote! {},
        Access::Role(role) => quote! { #[nirdosha_rt::contract(requires(role = #role))] },
    };

    // The #[contract(..)] macro (if the operation is role-gated) injects
    // its own proof parameter at position 0 automatically -- these core
    // fn signatures below never declare one themselves.
    let core_fns = quote! {
        #create_attr
        fn __create(values: &::std::collections::HashMap<String, String>) -> ::std::result::Result<#entity, Vec<String>> {
            let mut entity = __parse(values)?;
            entity.id = ::nirdosha_rt::screens::next_id();
            #store().lock().unwrap().insert(entity.id, entity.clone());
            Ok(entity)
        }

        #update_attr
        fn __update(id: i64, values: &::std::collections::HashMap<String, String>) -> ::std::result::Result<#entity, Vec<String>> {
            let parsed = __parse(values)?;
            let mut store = #store().lock().unwrap();
            match store.get_mut(&id) {
                Some(entity) => {
                    #( entity.#field_idents = parsed.#field_idents; )*
                    Ok(entity.clone())
                }
                None => Err(vec!["not found".to_string()]),
            }
        }

        #delete_attr
        fn __delete(id: i64) -> bool {
            #store().lock().unwrap().remove(&id).is_some()
        }
    };

    let new_title = format!("New {title}");
    let edit_title = format!("Edit {title}");

    // ---- read: list + detail, HTML + JSON ----
    let list_html_route = route("get", &input.read, &path, "List", |_| {
        quote! {
            let mut rows: Vec<::serde_json::Value> = #store().lock().unwrap().values().map(|e| ::serde_json::to_value(e).unwrap()).collect();
            rows.sort_by_key(|r| r["id"].as_i64().unwrap_or(0));
            ::nirdosha_rt::Response::html(200, ::nirdosha_rt::screens::list_html(#title, #path, &__fields(), &rows, #can_create))
        }
    });
    let list_api_route = route("get", &input.read, &api_path, "List (JSON)", |_| {
        quote! {
            let mut rows: Vec<::serde_json::Value> = #store().lock().unwrap().values().map(|e| ::serde_json::to_value(e).unwrap()).collect();
            rows.sort_by_key(|r| r["id"].as_i64().unwrap_or(0));
            ::nirdosha_rt::Response::json(200, &::serde_json::json!(rows))
        }
    });
    let detail_html_route = route("get", &input.read, &id_path, "Detail", |_| {
        quote! {
            let id: i64 = match params.get("id").and_then(|s| s.parse().ok()) { Some(v) => v, None => return ::nirdosha_rt::Response::bad_request("id must be an integer") };
            match #store().lock().unwrap().get(&id) {
                Some(entity) => {
                    let row = ::serde_json::to_value(entity).unwrap();
                    ::nirdosha_rt::Response::html(200, ::nirdosha_rt::screens::detail_html(#title, #path, id, &__fields(), &row, #can_update, #can_delete))
                }
                None => ::nirdosha_rt::Response::not_found(),
            }
        }
    });
    let detail_api_route = route("get", &input.read, &api_id_path, "Detail (JSON)", |_| {
        quote! {
            let id: i64 = match params.get("id").and_then(|s| s.parse().ok()) { Some(v) => v, None => return ::nirdosha_rt::Response::bad_request("id must be an integer") };
            match #store().lock().unwrap().get(&id) {
                Some(entity) => ::nirdosha_rt::Response::json(200, &::serde_json::to_value(entity).unwrap()),
                None => ::nirdosha_rt::Response::not_found(),
            }
        }
    });

    // ---- create: new-form (GET), submit (POST html + POST json) ----
    let new_form_route = route("get", &input.create, &new_path, "New form", |_| {
        quote! {
            let values: ::std::collections::HashMap<String, String> = ::std::collections::HashMap::new();
            ::nirdosha_rt::Response::html(200, ::nirdosha_rt::screens::form_html(#new_title, #path, &__fields(), &values, &[]))
        }
    });
    let create_html_route = route("post", &input.create, &path, "Create", |proof| {
        let arg = proof_arg(&proof);
        quote! {
            let values = req.form_or_json();
            match __create(#arg &values) {
                Ok(entity) => ::nirdosha_rt::Response::redirect(format!("{}/{}", #path, entity.id)),
                Err(errors) => ::nirdosha_rt::Response::html(400, ::nirdosha_rt::screens::form_html(#new_title, #path, &__fields(), &values, &errors)),
            }
        }
    });
    let create_api_route = route("post", &input.create, &api_path, "Create (JSON)", |proof| {
        let arg = proof_arg(&proof);
        quote! {
            let values = req.form_or_json();
            match __create(#arg &values) {
                Ok(entity) => ::nirdosha_rt::Response::json(201, &::serde_json::to_value(&entity).unwrap()),
                Err(errors) => ::nirdosha_rt::Response::json(400, &::serde_json::json!({ "errors": errors })),
            }
        }
    });

    // ---- update: edit-form (GET), submit (POST html, PUT json) ----
    let edit_form_route = route("get", &input.update, &edit_path, "Edit form", |_| {
        quote! {
            let id: i64 = match params.get("id").and_then(|s| s.parse().ok()) { Some(v) => v, None => return ::nirdosha_rt::Response::bad_request("id must be an integer") };
            match #store().lock().unwrap().get(&id) {
                Some(entity) => {
                    let row = ::serde_json::to_value(entity).unwrap();
                    let values = ::nirdosha_rt::screens::row_to_form_values(&__fields(), &row);
                    ::nirdosha_rt::Response::html(200, ::nirdosha_rt::screens::form_html(#edit_title, &format!("{}/{}/edit", #path, id), &__fields(), &values, &[]))
                }
                None => ::nirdosha_rt::Response::not_found(),
            }
        }
    });
    let update_html_route = route("post", &input.update, &edit_path, "Update", |proof| {
        let arg = proof_arg(&proof);
        quote! {
            let id: i64 = match params.get("id").and_then(|s| s.parse().ok()) { Some(v) => v, None => return ::nirdosha_rt::Response::bad_request("id must be an integer") };
            let values = req.form_or_json();
            match __update(#arg id, &values) {
                Ok(entity) => ::nirdosha_rt::Response::redirect(format!("{}/{}", #path, entity.id)),
                Err(errors) => ::nirdosha_rt::Response::html(400, ::nirdosha_rt::screens::form_html(#edit_title, &format!("{}/{}/edit", #path, id), &__fields(), &values, &errors)),
            }
        }
    });
    let update_api_route = route("put", &input.update, &api_id_path, "Update (JSON)", |proof| {
        let arg = proof_arg(&proof);
        quote! {
            let id: i64 = match params.get("id").and_then(|s| s.parse().ok()) { Some(v) => v, None => return ::nirdosha_rt::Response::bad_request("id must be an integer") };
            let values = req.form_or_json();
            match __update(#arg id, &values) {
                Ok(entity) => ::nirdosha_rt::Response::json(200, &::serde_json::to_value(&entity).unwrap()),
                Err(errors) => ::nirdosha_rt::Response::json(400, &::serde_json::json!({ "errors": errors })),
            }
        }
    });

    // ---- delete: typed-confirmation (GET), submit (POST html, DELETE json) ----
    let delete_confirm_route = route("get", &input.delete, &delete_path, "Delete confirmation", |_| {
        quote! {
            let id: i64 = match params.get("id").and_then(|s| s.parse().ok()) { Some(v) => v, None => return ::nirdosha_rt::Response::bad_request("id must be an integer") };
            ::nirdosha_rt::Response::html(200, ::nirdosha_rt::screens::delete_confirm_html(#title, &format!("{}/{}/delete", #path, id), "DELETE", &format!("#{id}")))
        }
    });
    let delete_html_route = route("post", &input.delete, &delete_path, "Delete", |proof| {
        let arg = proof_arg(&proof);
        quote! {
            let id: i64 = match params.get("id").and_then(|s| s.parse().ok()) { Some(v) => v, None => return ::nirdosha_rt::Response::bad_request("id must be an integer") };
            let values = req.form_or_json();
            if values.get("confirm").map(|s| s.as_str()) != Some("DELETE") {
                return ::nirdosha_rt::Response::html(400, ::nirdosha_rt::screens::delete_confirm_html(#title, &format!("{}/{}/delete", #path, id), "DELETE", &format!("#{id}")));
            }
            __delete(#arg id);
            ::nirdosha_rt::Response::redirect(#path)
        }
    });
    let delete_api_route = route("delete", &input.delete, &api_id_path, "Delete (JSON)", |proof| {
        let arg = proof_arg(&proof);
        quote! {
            let id: i64 = match params.get("id").and_then(|s| s.parse().ok()) { Some(v) => v, None => return ::nirdosha_rt::Response::bad_request("id must be an integer") };
            if __delete(#arg id) { ::nirdosha_rt::Response::no_content() } else { ::nirdosha_rt::Response::not_found() }
        }
    });

    quote! {
        fn #mount(router: ::nirdosha_rt::Router) -> ::nirdosha_rt::Router {
            #fields_fn
            #parse_fn
            #core_fns

            // Literal-suffix routes (`/new`, `/{id}/edit`, `/{id}/delete`)
            // must be registered before `/{id}` itself: `Router::dispatch`
            // matches in registration order, and a `{id}` wildcard
            // segment matches the literal text "new" just as readily as
            // a real id -- so `/products/new` would otherwise be
            // swallowed by `/products/{id}` and never reached.
            router
                #list_html_route
                #list_api_route
                #new_form_route
                #edit_form_route
                #delete_confirm_route
                #detail_html_route
                #detail_api_route
                #create_html_route
                #create_api_route
                #update_html_route
                #update_api_route
                #delete_html_route
                #delete_api_route
        }
    }
}
