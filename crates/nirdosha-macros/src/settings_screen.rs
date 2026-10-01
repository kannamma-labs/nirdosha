//! `settings_screen! { .. }` — RFC 0009 Track C's Settings/Configuration
//! archetype: a single, always-present record (no list, no id, no
//! delete) — "the current settings," not a collection. Reuses
//! `crud_screens!`'s `ParseField`/validation machinery, but the
//! datasource is a bare `fn() -> &'static SharedCell<Entity>` (the
//! managed singleton, `nirdosha_rt::prelude::SharedCell`), not a keyed
//! `SharedTable<i64, Entity>` — there's exactly one row, so there's
//! no id to key it by. (Pre-deny, this was a raw `Mutex<Entity>`;
//! the dialect-wide raw-lock deny — `nirdosha-contract-core/src/
//! scan.rs::LOCK_DENIES`, issue #77 — moved it onto the managed
//! primitive, the same move `Chan`'s queue made.)
//!
//! ```ignore
//! nirdosha_rt::settings_screen! {
//!     mount: mount_app_settings,
//!     entity: AppSettings,
//!     store: app_settings_store,
//!     path: "/settings",
//!     fields: [ site_name: String, maintenance_mode: bool ],
//!     access: requires role "admin",
//! }
//! ```

use crate::util::expect_keyword;
use nirdosha_contract_core::role::role_ident;
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{braced, bracketed, Ident, LitStr, Token, Type};

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

struct GuardConfig {
    table: Ident,
    purpose: LitStr,
    row_id: LitStr,
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
        let mut row_id = syn::LitStr::new("singleton", purpose.span());
        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
            if input.peek(Ident) {
                let fork = input.fork();
                let ahead: Ident = fork.parse()?;
                if ahead == "row_id" {
                    input.parse::<Ident>()?;
                    input.parse::<Token![:]>()?;
                    row_id = input.parse()?;
                    let _ = input.parse::<Token![,]>();
                }
            }
        }
        Ok(GuardConfig { table, purpose, row_id })
    }
}

struct FieldDef {
    name: Ident,
    ty: Type,
}

struct SettingsInput {
    mount: Ident,
    entity: Type,
    store: Ident,
    path: LitStr,
    fields: Vec<FieldDef>,
    access: Access,
    guard: Option<GuardConfig>,
}

impl Parse for SettingsInput {
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
            return Err(syn::Error::new(content.span(), "settings_screen! needs at least one field"));
        }
        input.parse::<Token![,]>()?;

        expect_keyword(input, "access")?;
        input.parse::<Token![:]>()?;
        let access: Access = input.parse()?;
        let _ = input.parse::<Token![,]>();

        let mut guard = None;
        if input.peek(Ident) {
            let fork = input.fork();
            let ahead: Ident = fork.parse()?;
            if ahead == "guard" {
                input.parse::<Ident>()?;
                input.parse::<Token![:]>()?;
                let content;
                braced!(content in input);
                guard = Some(content.parse::<GuardConfig>()?);
                let _ = input.parse::<Token![,]>();
            }
        }

        Ok(SettingsInput { mount, entity, store, path, fields, access, guard })
    }
}

fn input_type_for(ty: &Type) -> &'static str {
    match quote!(#ty).to_string().as_str() {
        "bool" => "checkbox",
        "i64" | "i32" | "u32" | "u64" | "f64" | "f32" | "usize" | "isize" => "number",
        _ => "text",
    }
}

pub fn expand(input: TokenStream) -> TokenStream {
    let parsed = match syn::parse::<SettingsInput>(input) {
        Ok(p) => p,
        Err(e) => return e.to_compile_error().into(),
    };
    expand_parsed(parsed).into()
}

fn expand_parsed(input: SettingsInput) -> TokenStream2 {
    let mount = &input.mount;
    let entity = &input.entity;
    let store = &input.store;
    let path_str = input.path.value();
    let path = quote! { #path_str };
    let api_path_str = format!("/api{path_str}");
    let api_path = quote! { #api_path_str };
    let edit_path_str = format!("{}/edit", path_str.trim_end_matches('/'));
    let edit_path = quote! { #edit_path_str };
    let title = path_str.trim_start_matches('/').to_string();

    let field_idents: Vec<&Ident> = input.fields.iter().map(|f| &f.name).collect();
    let field_names: Vec<String> = input.fields.iter().map(|f| f.name.to_string()).collect();
    let field_types: Vec<&Type> = input.fields.iter().map(|f| &f.ty).collect();
    let input_types: Vec<&'static str> = input.fields.iter().map(|f| input_type_for(&f.ty)).collect();

    let access_attr = match &input.access {
        Access::Public => quote! {},
        Access::Role(role) => quote! { #[nirdosha_rt::contract(requires(role = #role))] },
    };

    let fields_fn = quote! {
        fn __fields() -> Vec<::nirdosha_rt::screens::FieldSpec> {
            vec![ #( ::nirdosha_rt::screens::FieldSpec { name: #field_names, input_type: #input_types } ),* ]
        }
    };

    // ---- optional data-plane guard ----
    // When `guard:` is present, the singleton is stored in a
    // `GuardedTable` keyed by `row_id` (default `"singleton"`), and the
    // guard is the SINGLE authority for who may read it and whether the
    // update is allowed. Routes are emitted as `*_with_auth` (an `Auth`,
    // never a role-proof): the declared `access` stays vestigial route
    // metadata (OpenAPI docs), not a second check — a role the corpus
    // grants no policy for is denied by the guard at the data plane.
    if let Some(guard) = &input.guard {
        let table = &guard.table;
        let purpose = &guard.purpose;
        let row_id = &guard.row_id;

        let changed_fields_fn = quote! {
            fn __changed_fields() -> ::std::collections::HashSet<String> {
                [ #(#field_names),* ].into_iter().map(|s: &str| s.to_string()).collect()
            }
        };

        let view_route = quote! {
            .get_with_auth(#path, #title, |_req, _params, auth| {
                match #table().guarded_get(auth, #purpose, #row_id) {
                    Ok(Some(entity)) => {
                        let row = ::serde_json::to_value(&entity).unwrap();
                        ::nirdosha_rt::Response::html(200, ::nirdosha_rt::screens::settings_view_html(#title, #edit_path, &__fields(), &row, true))
                    }
                    Ok(None) => ::nirdosha_rt::Response::not_found(),
                    Err(e) => ::nirdosha_guard_screens::guard_error_response(e),
                }
            })
        };

        let view_api_route = quote! {
            .get_with_auth(#api_path, concat!(#title, " (JSON)"), |_req, _params, auth| {
                match #table().guarded_get(auth, #purpose, #row_id) {
                    Ok(Some(entity)) => ::nirdosha_rt::Response::json(200, &::serde_json::to_value(&entity).unwrap()),
                    Ok(None) => ::nirdosha_rt::Response::not_found(),
                    Err(e) => ::nirdosha_guard_screens::guard_error_response(e),
                }
            })
        };

        let edit_form_route = quote! {
            .get_with_auth(#edit_path, "Edit form", |_req, _params, auth| {
                match #table().guarded_get(auth, #purpose, #row_id) {
                    Ok(Some(entity)) => {
                        let row = ::serde_json::to_value(&entity).unwrap();
                        let values = ::nirdosha_rt::screens::row_to_form_values(&__fields(), &row);
                        ::nirdosha_rt::Response::html(200, ::nirdosha_rt::screens::form_html(#title, #edit_path, &__fields(), &values, &[]))
                    }
                    Ok(None) => ::nirdosha_rt::Response::not_found(),
                    Err(e) => ::nirdosha_guard_screens::guard_error_response(e),
                }
            })
        };

        let update_html_route = quote! {
            .post_with_auth(#edit_path, "Update", |req, _params, auth| {
                let values = req.form_or_json();
                let mut errors: Vec<String> = Vec::new();
                #(
                    let #field_idents = match <#field_types as ::nirdosha_rt::screens::ParseField>::parse_field(values.get(#field_names).map(|s| s.as_str())) {
                        Ok(v) => v,
                        Err(e) => { errors.push(format!("{}: {}", #field_names, e)); Default::default() }
                    };
                )*
                if !errors.is_empty() {
                    return ::nirdosha_rt::Response::html(400, ::nirdosha_rt::screens::form_html(#title, #edit_path, &__fields(), &values, &errors));
                }
                let changed = __changed_fields();
                match #table().guarded_update(auth, #purpose, #row_id, &changed, move |entity| { #( entity.#field_idents = #field_idents; )* }) {
                    Ok(_) => ::nirdosha_rt::Response::redirect(#path),
                    Err(e) => ::nirdosha_guard_screens::guard_error_response(e),
                }
            })
        };

        let update_api_route = quote! {
            .put_with_auth(#api_path, "Update (JSON)", |req, _params, auth| {
                let values = req.form_or_json();
                let mut errors: Vec<String> = Vec::new();
                #(
                    let #field_idents = match <#field_types as ::nirdosha_rt::screens::ParseField>::parse_field(values.get(#field_names).map(|s| s.as_str())) {
                        Ok(v) => v,
                        Err(e) => { errors.push(format!("{}: {}", #field_names, e)); Default::default() }
                    };
                )*
                if !errors.is_empty() {
                    return ::nirdosha_rt::Response::json(400, &::serde_json::json!({ "errors": errors }));
                }
                let changed = __changed_fields();
                match #table().guarded_update(auth, #purpose, #row_id, &changed, move |entity| { #( entity.#field_idents = #field_idents; )* }) {
                    Ok(entity) => ::nirdosha_rt::Response::json(200, &::serde_json::to_value(&entity).unwrap()),
                    Err(e) => ::nirdosha_guard_screens::guard_error_response(e),
                }
            })
        };

        return quote! {
            pub fn #mount(router: ::nirdosha_rt::Router) -> ::nirdosha_rt::Router {
                #fields_fn
                #changed_fields_fn
                router
                    #view_route
                    #view_api_route
                    #edit_form_route
                    #update_html_route
                    #update_api_route
            }
        };
    }

    let update_fn = quote! {
        #access_attr
        fn __update(values: &::std::collections::HashMap<String, String>) -> ::std::result::Result<#entity, Vec<String>> {
            let mut errors: Vec<String> = Vec::new();
            #(
                let #field_idents = match <#field_types as ::nirdosha_rt::screens::ParseField>::parse_field(values.get(#field_names).map(|s| s.as_str())) {
                    Ok(v) => v,
                    Err(e) => { errors.push(format!("{}: {}", #field_names, e)); Default::default() }
                };
            )*
            if !errors.is_empty() { return Err(errors); }
            #store().with(|current| {
                #( current.#field_idents = #field_idents; )*
                Ok(current.clone())
            })
        }
    };

    let view_route = route("get", &input.access, &path, &title, |_| {
        quote! {
            let entity = #store().get_clone();
            let row = ::serde_json::to_value(&entity).unwrap();
            ::nirdosha_rt::Response::html(200, ::nirdosha_rt::screens::settings_view_html(#title, #edit_path, &__fields(), &row, true))
        }
    });
    let view_api_route = route("get", &input.access, &api_path, &format!("{title} (JSON)"), |_| {
        quote! {
            let entity = #store().get_clone();
            ::nirdosha_rt::Response::json(200, &::serde_json::to_value(&entity).unwrap())
        }
    });
    let edit_form_route = route("get", &input.access, &edit_path, "Edit form", |_| {
        quote! {
            let entity = #store().get_clone();
            let row = ::serde_json::to_value(&entity).unwrap();
            let values = ::nirdosha_rt::screens::row_to_form_values(&__fields(), &row);
            ::nirdosha_rt::Response::html(200, ::nirdosha_rt::screens::form_html(#title, #edit_path, &__fields(), &values, &[]))
        }
    });
    let update_html_route = route("post", &input.access, &edit_path, "Update", |proof| {
        let arg = match &proof {
            Some(p) => quote! { #p, },
            None => quote! {},
        };
        quote! {
            let values = req.form_or_json();
            match __update(#arg &values) {
                Ok(_) => ::nirdosha_rt::Response::redirect(#path),
                Err(errors) => ::nirdosha_rt::Response::html(400, ::nirdosha_rt::screens::form_html(#title, #edit_path, &__fields(), &values, &errors)),
            }
        }
    });
    let update_api_route = route("put", &input.access, &api_path, "Update (JSON)", |proof| {
        let arg = match &proof {
            Some(p) => quote! { #p, },
            None => quote! {},
        };
        quote! {
            let values = req.form_or_json();
            match __update(#arg &values) {
                Ok(entity) => ::nirdosha_rt::Response::json(200, &::serde_json::to_value(&entity).unwrap()),
                Err(errors) => ::nirdosha_rt::Response::json(400, &::serde_json::json!({ "errors": errors })),
            }
        }
    });

    quote! {
        // `pub`, matching `login!`/`app_shell!`/`crud_screens!`'s generated
        // mount fns -- a private mount fn only ever worked because every
        // existing caller invoked the macro and called the result in the
        // same file; a real multi-module app (screens split one-per-file)
        // needs to call it from outside that module.
        pub fn #mount(router: ::nirdosha_rt::Router) -> ::nirdosha_rt::Router {
            #fields_fn
            #update_fn
            router
                #view_route
                #view_api_route
                #edit_form_route
                #update_html_route
                #update_api_route
        }
    }
}

fn route(method: &str, access: &Access, path: &TokenStream2, summary: &str, body_fn: impl Fn(Option<TokenStream2>) -> TokenStream2) -> TokenStream2 {
    let (plain, gated) = match method {
        "get" => (quote!(get), quote!(get_gated)),
        "post" => (quote!(post), quote!(post_gated)),
        "put" => (quote!(put), quote!(put_gated)),
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

#[cfg(test)]
mod tests {
    use super::SettingsInput;

    #[test]
    fn guard_clause_parses_with_defaults() {
        let parsed = syn::parse_str::<SettingsInput>(
            "mount: mount_app_settings, entity: AppSettings, store: app_settings_store, path: \"/settings\", \
             fields: [site_name: String], access: requires role \"admin\", \
             guard: { table: app_settings_table, purpose: \"Administration\" }"
        );
        assert!(parsed.is_ok());
        let input = parsed.unwrap();
        assert!(input.guard.is_some());
        assert_eq!(input.guard.unwrap().row_id.value(), "singleton");
    }

    #[test]
    fn guard_clause_accepts_custom_row_id() {
        let parsed = syn::parse_str::<SettingsInput>(
            "mount: mount_app_settings, entity: AppSettings, store: app_settings_store, path: \"/settings\", \
             fields: [site_name: String], access: requires role \"admin\", \
             guard: { table: app_settings_table, purpose: \"Administration\", row_id: \"current\" }"
        );
        assert!(parsed.is_ok());
        assert_eq!(parsed.unwrap().guard.unwrap().row_id.value(), "current");
    }

    #[test]
    fn guard_clause_requires_table_and_purpose() {
        let parsed = syn::parse_str::<SettingsInput>(
            "mount: mount_app_settings, entity: AppSettings, store: app_settings_store, path: \"/settings\", \
             fields: [site_name: String], access: requires role \"admin\", \
             guard: { table: app_settings_table }"
        );
        assert!(parsed.is_err());
    }
}
