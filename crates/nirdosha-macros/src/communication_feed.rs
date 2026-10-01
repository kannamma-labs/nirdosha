//! `communication_feed! { .. }` — RFC 0009 Track C's Communication
//! archetype: an append-only, newest-first feed of posted messages.
//!
//! `refresh_seconds` refreshes on a timer. Alternatively,
//! `long_poll_seconds: 1..=30` waits for a generated POST before refreshing;
//! the JSON endpoint accepts a `since` revision and returns its current
//! revision in `X-Nirdosha-Revision`. Both modes preserve read-role gates.
//!
//! ```ignore
//! nirdosha_rt::communication_feed! {
//!     mount: mount_team_feed,
//!     entity: Message,
//!     store: message_store,
//!     path: "/feed",
//!     fields: [ author: String, body: String ],
//!     post_access: requires role "member",
//!     read_access: public,
//!     refresh_seconds: 5,
//! }
//! ```

use crate::util::expect_keyword;
use nirdosha_contract_core::role::role_ident;
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{braced, bracketed, Ident, LitInt, LitStr, Token, Type};

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

struct FieldDef {
    name: Ident,
    ty: Type,
}

struct FeedInput {
    mount: Ident,
    entity: Type,
    store: Ident,
    path: LitStr,
    fields: Vec<FieldDef>,
    post_access: Access,
    read_access: Access,
    refresh_seconds: Option<LitInt>,
    long_poll_seconds: Option<LitInt>,
    guard: Option<GuardConfig>,
}

impl Parse for FeedInput {
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
            return Err(syn::Error::new(content.span(), "communication_feed! needs at least one field"));
        }
        input.parse::<Token![,]>()?;

        expect_keyword(input, "post_access")?;
        input.parse::<Token![:]>()?;
        let post_access: Access = input.parse()?;
        input.parse::<Token![,]>()?;

        expect_keyword(input, "read_access")?;
        input.parse::<Token![:]>()?;
        let read_access: Access = input.parse()?;
        input.parse::<Token![,]>()?;

        let mut refresh_seconds = None;
        let mut long_poll_seconds = None;
        if input.peek(Ident) {
            let fork = input.fork();
            let ahead: Ident = fork.parse()?;
            if ahead == "refresh_seconds" || ahead == "long_poll_seconds" {
                let mode: Ident = input.parse()?;
                input.parse::<Token![:]>()?;
                let seconds: LitInt = input.parse()?;
                if mode == "refresh_seconds" {
                    refresh_seconds = Some(seconds);
                } else {
                    if !(1..=30).contains(&seconds.base10_parse::<u64>()?) {
                        return Err(syn::Error::new(seconds.span(), "long_poll_seconds must be between 1 and 30"));
                    }
                    long_poll_seconds = Some(seconds);
                }
                if input.peek(Token![,]) { input.parse::<Token![,]>()?; }
            }
        }

        let mut guard = None;
        while input.peek(Ident) {
            let fork = input.fork();
            let ahead: Ident = fork.parse()?;
            if ahead == "guard" && guard.is_none() {
                input.parse::<Ident>()?;
                input.parse::<Token![:]>()?;
                let content;
                braced!(content in input);
                guard = Some(content.parse::<GuardConfig>()?);
                let _ = input.parse::<Token![,]>();
            } else {
                break;
            }
        }

        Ok(FeedInput { mount, entity, store, path, fields, post_access, read_access, refresh_seconds, long_poll_seconds, guard })
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
    let parsed = match syn::parse::<FeedInput>(input) {
        Ok(p) => p,
        Err(e) => return e.to_compile_error().into(),
    };
    expand_parsed(parsed).into()
}

fn expand_parsed(input: FeedInput) -> TokenStream2 {
    let mount = &input.mount;
    let entity = &input.entity;
    let store = &input.store;
    let path_str = input.path.value();
    let path = quote! { #path_str };
    let api_path_str = format!("/api{path_str}");
    let api_path = quote! { #api_path_str };
    let title = path_str.trim_start_matches('/').to_string();
    let refresh = match &input.refresh_seconds {
        Some(n) => quote! { Some(#n) },
        None => quote! { None },
    };
    let publish = input.long_poll_seconds.as_ref().map(|_| quote! { __UPDATES.publish(); });
    let view_revision = input.long_poll_seconds.as_ref().map(|_| quote! { let revision = __UPDATES.revision(); });
    let view_live = input.long_poll_seconds.as_ref().map(|_| quote! {
        let html = ::nirdosha_rt::feed::with_long_poll(html, #api_path, revision);
    });
    let wait = input.long_poll_seconds.as_ref().map(|seconds| quote! {
        let query = _req.query();
        let revision = match query.get("since") {
            Some(since) => match since.parse::<u64>() {
                Ok(since) => __UPDATES.wait(since, ::std::time::Duration::from_secs(#seconds)),
                Err(_) => return ::nirdosha_rt::Response::bad_request("invalid feed revision"),
            },
            None => __UPDATES.revision(),
        };
    });
    let revision_header = input.long_poll_seconds.as_ref().map(|_| quote! {
        response.extra_headers.push(("X-Nirdosha-Revision".into(), revision.to_string()));
        response.extra_headers.push(("Cache-Control".into(), "no-store".into()));
    });

    let field_idents: Vec<&Ident> = input.fields.iter().map(|f| &f.name).collect();
    let field_names: Vec<String> = input.fields.iter().map(|f| f.name.to_string()).collect();
    let field_types: Vec<&Type> = input.fields.iter().map(|f| &f.ty).collect();
    let input_types: Vec<&'static str> = input.fields.iter().map(|f| input_type_for(&f.ty)).collect();

    let post_attr = match &input.post_access {
        Access::Public => quote! {},
        Access::Role(role) => quote! { #[nirdosha_rt::contract(requires(role = #role))] },
    };

    let fields_fn = quote! {
        fn __fields() -> Vec<::nirdosha_rt::screens::FieldSpec> {
            vec![ #( ::nirdosha_rt::screens::FieldSpec { name: #field_names, input_type: #input_types } ),* ]
        }
    };

    let post_fn = quote! {
        #post_attr
        fn __post(values: &::std::collections::HashMap<String, String>) -> ::std::result::Result<#entity, Vec<String>> {
            let mut errors: Vec<String> = Vec::new();
            #(
                let #field_idents = match <#field_types as ::nirdosha_rt::screens::ParseField>::parse_field(values.get(#field_names).map(|s| s.as_str())) {
                    Ok(v) => v,
                    Err(e) => { errors.push(format!("{}: {}", #field_names, e)); Default::default() }
                };
            )*
            if !errors.is_empty() { return Err(errors); }
            let mut entity = #entity { id: 0, #( #field_idents ),*, ..Default::default() };
            entity.id = ::nirdosha_rt::screens::next_id();
            #store().with(|store| {
                store.push(entity.clone());
                store.sort_by_key(|e: &#entity| ::std::cmp::Reverse(e.id));
            });
            #publish
            Ok(entity)
        }
    };

    // ---- optional data-plane guard ----
    // When `guard:` is present, reads and writes go through
    // `GuardedTable` instead of the bare `store`, and the guard is the
    // SINGLE authority for who may read, which rows they see, which
    // fields are masked, and whether a post is allowed. Routes are
    // emitted as `*_with_auth` (an `Auth`, never a role-proof): the
    // declared `read_access`/`post_access` stay vestigial route metadata
    // (OpenAPI docs), not a second check — a role the corpus grants no
    // read policy for is denied by the guard at the data plane.
    if let Some(guard) = &input.guard {
        let table = &guard.table;
        let purpose = &guard.purpose;

        let submitted_fn = quote! {
            fn __guarded_submitted_fields(values: &::std::collections::HashMap<String, String>) -> ::std::collections::HashSet<String> {
                let exempt: &[&str] = <#entity as ::nirdosha_guard_screens::GuardedEntity>::create_field_policy_exempt();
                [ #(#field_names),* ]
                    .into_iter()
                    .filter(|name: &&str| values.get(*name).is_some_and(|v| !v.is_empty()))
                    .filter(|name: &&str| !exempt.contains(name))
                    .map(|s: &str| s.to_string())
                    .collect()
            }
        };

        let guarded_post_fn = quote! {
            fn __guarded_post(auth: &::nirdosha_rt::Auth, values: &::std::collections::HashMap<String, String>) -> ::std::result::Result<#entity, nirdosha_guard_screens::GuardScreenError> {
                let mut errors: Vec<String> = Vec::new();
                #(
                    let #field_idents = match <#field_types as ::nirdosha_rt::screens::ParseField>::parse_field(values.get(#field_names).map(|s| s.as_str())) {
                        Ok(v) => v,
                        Err(e) => { errors.push(format!("{}: {}", #field_names, e)); Default::default() }
                    };
                )*
                if !errors.is_empty() { return Err(nirdosha_guard_screens::GuardScreenError::FieldPolicyViolation(errors)); }
                let mut entity = #entity { id: 0, #( #field_idents ),*, ..Default::default() };
                entity.id = ::nirdosha_rt::screens::next_id();
                let submitted = __guarded_submitted_fields(values);
                #table().guarded_insert_checked(auth, #purpose, &submitted, entity)
                    .map(|e| { #publish e })
            }
        };

        let view_route = quote! {
            .get_with_auth(#path, #title, |_req, _params, auth| {
                #view_revision
                let messages: Vec<::serde_json::Value> = match #table().guarded_snapshot(auth, #purpose) {
                    Ok(rows) => rows.iter().map(|e| ::serde_json::to_value(e).unwrap()).collect(),
                    Err(e) => return ::nirdosha_guard_screens::guard_error_response(e),
                };
                let html = ::nirdosha_rt::feed::feed_html(#title, #refresh, #path, &__fields(), &messages);
                #view_live
                ::nirdosha_rt::Response::html(200, html)
            })
        };

        let api_route = quote! {
            .get_with_auth(#api_path, concat!(#title, " (JSON)"), |_req, _params, auth| {
                #wait
                let messages: Vec<::serde_json::Value> = match #table().guarded_snapshot(auth, #purpose) {
                    Ok(rows) => rows.iter().map(|e| ::serde_json::to_value(e).unwrap()).collect(),
                    Err(e) => return ::nirdosha_guard_screens::guard_error_response(e),
                };
                #[allow(unused_mut)]
                let mut response = ::nirdosha_rt::Response::json(200, &::serde_json::Value::Array(messages));
                #revision_header
                response
            })
        };

        let post_route = quote! {
            .post_with_auth(#path, "Post a message", |req, _params, auth| {
                let values = req.form_or_json();
                match __guarded_post(auth, &values) {
                    Ok(_) => ::nirdosha_rt::Response::redirect(#path),
                    Err(e) => ::nirdosha_guard_screens::guard_error_response(e),
                }
            })
        };

        return quote! {
            pub fn #mount(router: ::nirdosha_rt::Router) -> ::nirdosha_rt::Router {
                static __UPDATES: ::nirdosha_rt::feed::FeedUpdates = ::nirdosha_rt::feed::FeedUpdates::new();
                #fields_fn
                #submitted_fn
                #guarded_post_fn
                router
                    #view_route
                    #api_route
                    #post_route
            }
        };
    }

    let view_body = quote! {
        #view_revision
        let messages: Vec<::serde_json::Value> = #store().with(|store| store.iter().map(|e| ::serde_json::to_value(e).unwrap()).collect());
        let html = ::nirdosha_rt::feed::feed_html(#title, #refresh, #path, &__fields(), &messages);
        #view_live
        ::nirdosha_rt::Response::html(200, html)
    };
    let view_route = match &input.read_access {
        Access::Public => quote! { .get(#path, #title, |_req, _params| { #view_body }) },
        Access::Role(role) => {
            let role_ident = role_ident(&role.value(), role.span()).expect("role name already validated at parse time");
            quote! { .get_gated::<crate::nirdosha_roles::#role_ident>(#path, #title, |_req, _params, _proof| { #view_body }) }
        }
    };

    let api_body = quote! {
        #wait
        let messages: Vec<::serde_json::Value> = #store().with(|store| store.iter().map(|e| ::serde_json::to_value(e).unwrap()).collect());
        #[allow(unused_mut)]
        let mut response = ::nirdosha_rt::Response::json(200, &::serde_json::Value::Array(messages));
        #revision_header
        response
    };
    let api_route = match &input.read_access {
        Access::Public => quote! { .get(#api_path, concat!(#title, " (JSON)"), |_req, _params| { #api_body }) },
        Access::Role(role) => {
            let role_ident = role_ident(&role.value(), role.span()).expect("role name already validated at parse time");
            quote! { .get_gated::<crate::nirdosha_roles::#role_ident>(#api_path, concat!(#title, " (JSON)"), |_req, _params, _proof| { #api_body }) }
        }
    };

    let post_route = match &input.post_access {
        Access::Public => quote! {
            .post(#path, "Post a message", |req, _params| {
                let values = req.form_or_json();
                match __post(&values) {
                    Ok(_) => ::nirdosha_rt::Response::redirect(#path),
                    Err(errors) => ::nirdosha_rt::Response::bad_request(errors.join("; ")),
                }
            })
        },
        Access::Role(role) => {
            let role_ident = role_ident(&role.value(), role.span()).expect("role name already validated at parse time");
            quote! {
                .post_gated::<crate::nirdosha_roles::#role_ident>(#path, "Post a message", |req, _params, proof| {
                    let values = req.form_or_json();
                    match __post(proof, &values) {
                        Ok(_) => ::nirdosha_rt::Response::redirect(#path),
                        Err(errors) => ::nirdosha_rt::Response::bad_request(errors.join("; ")),
                    }
                })
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
            static __UPDATES: ::nirdosha_rt::feed::FeedUpdates = ::nirdosha_rt::feed::FeedUpdates::new();
            #fields_fn
            #post_fn
            router
                #view_route
                #api_route
                #post_route
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FeedInput;

    fn parse(mode: &str) -> syn::Result<FeedInput> {
        syn::parse_str(&format!(
            "mount: mount_feed, entity: Message, store: messages, path: \"/feed\", \
             fields: [body: String], post_access: public, read_access: public, {mode}"
        ))
    }

    #[test]
    fn guard_clause_parses_with_any_access_declaration() {
        let ok = syn::parse_str::<FeedInput>(
            "mount: mount_feed, entity: Message, store: messages, path: \"/feed\", \
             fields: [body: String], post_access: requires role \"member\", \
             read_access: requires role \"member\", \
             guard: { table: message_table, purpose: \"Operations\" }"
        );
        assert!(ok.is_ok());
        let guarded = ok.unwrap();
        assert!(guarded.guard.is_some());

        // `public` read/post combined with `guard:` is fine: guard-mode
        // routes are `*_with_auth` and the guard is the single authority,
        // so the declared access stays vestigial route metadata.
        let public_guard = syn::parse_str::<FeedInput>(
            "mount: mount_feed, entity: Message, store: messages, path: \"/feed\", \
             fields: [body: String], post_access: public, read_access: public, \
             guard: { table: message_table, purpose: \"Operations\" }"
        );
        assert!(public_guard.is_ok());
    }

    #[test]
    fn guard_clause_requires_table_and_purpose() {
        let bad = syn::parse_str::<FeedInput>(
            "mount: mount_feed, entity: Message, store: messages, path: \"/feed\", \
             fields: [body: String], post_access: requires role \"member\", \
             read_access: requires role \"member\", \
             guard: { table: message_table }"
        );
        assert!(bad.is_err(), "accepted guard without purpose");
    }

    #[test]
    fn long_poll_wait_is_bounded_and_modes_are_exclusive() {
        assert!(parse("long_poll_seconds: 1,").is_ok());
        assert!(parse("long_poll_seconds: 30,").is_ok());
        for mode in [
            "long_poll_seconds: 0,",
            "long_poll_seconds: 31,",
            "long_poll_seconds: 1, refresh_seconds: 5,",
            "refresh_seconds: 5, long_poll_seconds: 1,",
            "unknown_mode: 1,",
        ] {
            assert!(parse(mode).is_err(), "accepted {mode}");
        }
    }
}
