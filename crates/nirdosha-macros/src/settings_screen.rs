//! `settings_screen! { .. }` — RFC 0009 Track C's Settings/Configuration
//! archetype: a single, always-present record (no list, no id, no
//! delete) — "the current settings," not a collection. Reuses
//! `crud_screens!`'s `ParseField`/validation machinery, but the
//! datasource is a bare `fn() -> &'static Mutex<Entity>`, not a
//! `Mutex<HashMap<i64, Entity>>` — there's exactly one row, so there's
//! no id to key it by.
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

        Ok(SettingsInput { mount, entity, store, path, fields, access })
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
            let mut current = #store().lock().unwrap();
            #( current.#field_idents = #field_idents; )*
            Ok(current.clone())
        }
    };

    let view_route = route("get", &input.access, &path, &title, |_| {
        quote! {
            let entity = #store().lock().unwrap().clone();
            let row = ::serde_json::to_value(&entity).unwrap();
            ::nirdosha_rt::Response::html(200, ::nirdosha_rt::screens::settings_view_html(#title, #edit_path, &__fields(), &row, true))
        }
    });
    let view_api_route = route("get", &input.access, &api_path, &format!("{title} (JSON)"), |_| {
        quote! {
            let entity = #store().lock().unwrap().clone();
            ::nirdosha_rt::Response::json(200, &::serde_json::to_value(&entity).unwrap())
        }
    });
    let edit_form_route = route("get", &input.access, &edit_path, "Edit form", |_| {
        quote! {
            let entity = #store().lock().unwrap().clone();
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
        fn #mount(router: ::nirdosha_rt::Router) -> ::nirdosha_rt::Router {
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
