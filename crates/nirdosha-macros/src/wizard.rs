//! `wizard! { .. }` — RFC 0009 Track C's Workflow/Wizard archetype: a
//! multi-step form, each step its own `GET`/`POST {path}/step/N`, with
//! in-progress answers held server-side (keyed by an opaque cookie) so
//! no client-side JS is needed. The final step assembles the full
//! entity and inserts it into the same kind of datasource
//! `crud_screens!` uses.
//!
//! Each step's fields are fixed at compile time (one literal route per
//! step, not one `{n}` wildcard route with runtime-dispatched field
//! lists) so every field's `ParseField` check is an ordinary `rustc`
//! trait-bound check, the same guarantee `crud_screens!` gives.
//!
//! ```ignore
//! nirdosha_rt::wizard! {
//!     mount: mount_onboarding_wizard,
//!     entity: Employee,
//!     store: employee_store,
//!     path: "/onboarding",
//!     access: public,
//!     steps: [
//!         { name: "Basics", fields: [ name: String, department: String ] },
//!         { name: "Compensation", fields: [ salary: f64 ] },
//!     ],
//! }
//! ```

use crate::util::expect_keyword;
use nirdosha_contract_core::role::role_ident;
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use std::collections::HashSet;
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

fn parse_field_list(input: ParseStream) -> syn::Result<Vec<FieldDef>> {
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
    Ok(fields)
}

struct StepDef {
    name: LitStr,
    fields: Vec<FieldDef>,
}

impl Parse for StepDef {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let content;
        braced!(content in input);
        expect_keyword(&content, "name")?;
        content.parse::<Token![:]>()?;
        let name: LitStr = content.parse()?;
        content.parse::<Token![,]>()?;
        expect_keyword(&content, "fields")?;
        content.parse::<Token![:]>()?;
        let fields = parse_field_list(&content)?;
        if fields.is_empty() {
            return Err(syn::Error::new(name.span(), "each wizard step needs at least one field"));
        }
        let _ = content.parse::<Token![,]>();
        Ok(StepDef { name, fields })
    }
}

struct WizardInput {
    mount: Ident,
    entity: Type,
    store: Ident,
    path: LitStr,
    access: Access,
    steps: Vec<StepDef>,
}

impl Parse for WizardInput {
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

        expect_keyword(input, "steps")?;
        input.parse::<Token![:]>()?;
        let content;
        bracketed!(content in input);
        let mut steps = Vec::new();
        while !content.is_empty() {
            steps.push(content.parse::<StepDef>()?);
            if content.peek(Token![,]) {
                content.parse::<Token![,]>()?;
            }
        }
        if steps.len() < 2 {
            return Err(syn::Error::new(content.span(), "wizard! needs at least two steps (a single step is a plain form, not a wizard)"));
        }
        let _ = input.parse::<Token![,]>();

        let mut seen: HashSet<String> = HashSet::new();
        for step in &steps {
            for f in &step.fields {
                let name = f.name.to_string();
                if !seen.insert(name.clone()) {
                    return Err(syn::Error::new(f.name.span(), format!("field `{name}` appears in more than one step")));
                }
            }
        }

        Ok(WizardInput { mount, entity, store, path, access, steps })
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
    let parsed = match syn::parse::<WizardInput>(input) {
        Ok(p) => p,
        Err(e) => return e.to_compile_error().into(),
    };
    expand_parsed(parsed).into()
}

fn expand_parsed(input: WizardInput) -> TokenStream2 {
    let mount = &input.mount;
    let entity = &input.entity;
    let store = &input.store;
    let path_str = input.path.value();
    let base_path = quote! { #path_str };
    let cookie_name = format!("nirdosha_wizard_{}", path_str.trim_start_matches('/').replace('/', "_"));
    let total_steps = input.steps.len();

    let access_attr = match &input.access {
        Access::Public => quote! {},
        Access::Role(role) => quote! { #[nirdosha_rt::contract(requires(role = #role))] },
    };

    let all_field_idents: Vec<&Ident> = input.steps.iter().flat_map(|s| s.fields.iter().map(|f| &f.name)).collect();
    let all_field_names: Vec<String> = input.steps.iter().flat_map(|s| s.fields.iter().map(|f| f.name.to_string())).collect();
    let all_field_types: Vec<&Type> = input.steps.iter().flat_map(|s| s.fields.iter().map(|f| &f.ty)).collect();
    let all_input_types: Vec<&'static str> = input.steps.iter().flat_map(|s| s.fields.iter().map(|f| input_type_for(&f.ty))).collect();

    let finalize_fn = quote! {
        #access_attr
        fn __finalize(values: &::std::collections::HashMap<String, String>) -> #entity {
            #(
                let #all_field_idents = <#all_field_types as ::nirdosha_rt::screens::ParseField>::parse_field(values.get(#all_field_names).map(|s| s.as_str()))
                    .expect("wizard state holds only canonical, already-validated values");
            )*
            let mut entity = #entity { id: 0, #( #all_field_idents ),*, ..Default::default() };
            entity.id = ::nirdosha_rt::screens::next_id();
            #store().lock().unwrap().insert(entity.id, entity.clone());
            entity
        }

        fn __all_fields() -> Vec<::nirdosha_rt::screens::FieldSpec> {
            vec![ #( ::nirdosha_rt::screens::FieldSpec { name: #all_field_names, input_type: #all_input_types } ),* ]
        }

        fn __wizard_state() -> &'static ::std::sync::Mutex<::std::collections::HashMap<String, ::std::collections::HashMap<String, String>>> {
            static STATE: ::std::sync::OnceLock<::std::sync::Mutex<::std::collections::HashMap<String, ::std::collections::HashMap<String, String>>>> = ::std::sync::OnceLock::new();
            STATE.get_or_init(|| ::std::sync::Mutex::new(::std::collections::HashMap::new()))
        }
    };

    let mut step_items = Vec::new();
    let mut step_routes = Vec::new();

    for (i, step) in input.steps.iter().enumerate() {
        let step_no = i + 1;
        let is_last = step_no == total_steps;
        let step_path_str = format!("{}/step/{step_no}", path_str.trim_end_matches('/'));
        let step_path = quote! { #step_path_str };
        let step_title = step.name.value();
        let step_field_names: Vec<String> = step.fields.iter().map(|f| f.name.to_string()).collect();
        let step_field_types: Vec<&Type> = step.fields.iter().map(|f| &f.ty).collect();
        let step_input_types: Vec<&'static str> = step.fields.iter().map(|f| input_type_for(&f.ty)).collect();

        let fields_fn_name = quote::format_ident!("__fields_step_{step_no}");
        let validate_fn_name = quote::format_ident!("__validate_step_{step_no}");

        step_items.push(quote! {
            fn #fields_fn_name() -> Vec<::nirdosha_rt::screens::FieldSpec> {
                vec![ #( ::nirdosha_rt::screens::FieldSpec { name: #step_field_names, input_type: #step_input_types } ),* ]
            }

            fn #validate_fn_name(values: &::std::collections::HashMap<String, String>) -> ::std::result::Result<::std::collections::HashMap<String, String>, Vec<String>> {
                let mut errors: Vec<String> = Vec::new();
                let mut canonical: ::std::collections::HashMap<String, String> = ::std::collections::HashMap::new();
                #(
                    match <#step_field_types as ::nirdosha_rt::screens::ParseField>::parse_field(values.get(#step_field_names).map(|s| s.as_str())) {
                        Ok(v) => { canonical.insert(#step_field_names.to_string(), v.to_string()); }
                        Err(e) => errors.push(format!("{}: {}", #step_field_names, e)),
                    }
                )*
                if !errors.is_empty() { return Err(errors); }
                Ok(canonical)
            }
        });

        let next_step_path_str = if is_last {
            format!("{}/done", path_str.trim_end_matches('/'))
        } else {
            format!("{}/step/{}", path_str.trim_end_matches('/'), step_no + 1)
        };

        let get_body = quote! {
            let existing: ::std::collections::HashMap<String, String> = req.cookie(#cookie_name)
                .and_then(|sid| __wizard_state().lock().unwrap().get(&sid).cloned())
                .unwrap_or_default();
            ::nirdosha_rt::Response::html(200, ::nirdosha_rt::wizard::wizard_step_html(#step_title, #step_no, #total_steps, #step_path, &#fields_fn_name(), &existing, &[]))
        };

        let post_body_last = |proof: Option<TokenStream2>| {
            let finalize_arg = match &proof {
                Some(p) => quote! { #p, },
                None => quote! {},
            };
            quote! {
                let values = req.form_or_json();
                match #validate_fn_name(&values) {
                    Err(errors) => ::nirdosha_rt::Response::html(400, ::nirdosha_rt::wizard::wizard_step_html(#step_title, #step_no, #total_steps, #step_path, &#fields_fn_name(), &values, &errors)),
                    Ok(canonical) => {
                        let sid = req.cookie(#cookie_name).unwrap_or_default();
                        let mut merged = __wizard_state().lock().unwrap().get(&sid).cloned().unwrap_or_default();
                        merged.extend(canonical);
                        let entity = __finalize(#finalize_arg &merged);
                        __wizard_state().lock().unwrap().remove(&sid);
                        let mut resp = ::nirdosha_rt::Response::redirect(format!("{}/{}", #next_step_path_str, entity.id));
                        resp.extra_headers.push(("Set-Cookie".to_string(), format!("{}=; Path=/; Max-Age=0", #cookie_name)));
                        resp
                    }
                }
            }
        };

        let post_body_middle = quote! {
            let values = req.form_or_json();
            match #validate_fn_name(&values) {
                Err(errors) => ::nirdosha_rt::Response::html(400, ::nirdosha_rt::wizard::wizard_step_html(#step_title, #step_no, #total_steps, #step_path, &#fields_fn_name(), &values, &errors)),
                Ok(canonical) => {
                    let sid = req.cookie(#cookie_name).unwrap_or_else(::nirdosha_rt::wizard::generate_wizard_run_id);
                    let mut state = __wizard_state().lock().unwrap();
                    let entry = state.entry(sid.clone()).or_default();
                    entry.extend(canonical);
                    drop(state);
                    let mut resp = ::nirdosha_rt::Response::redirect(#next_step_path_str);
                    resp.extra_headers.push(("Set-Cookie".to_string(), format!("{}={}; HttpOnly; Path=/", #cookie_name, sid)));
                    resp
                }
            }
        };

        step_routes.push(route("get", &input.access, &step_path, &format!("Wizard: {step_title}"), |_| get_body.clone()));
        if is_last {
            step_routes.push(route("post", &input.access, &step_path, &format!("Wizard: {step_title} (submit)"), post_body_last));
        } else {
            step_routes.push(route("post", &input.access, &step_path, &format!("Wizard: {step_title} (submit)"), |_| post_body_middle.clone()));
        }
    }

    let done_path_str = format!("{}/done/{{id}}", path_str.trim_end_matches('/'));
    let done_path = quote! { #done_path_str };
    let done_route = route("get", &input.access, &done_path, "Wizard complete", |_| {
        quote! {
            let id: i64 = match params.get("id").and_then(|s| s.parse().ok()) {
                Some(id) => id,
                None => return ::nirdosha_rt::Response::bad_request("id must be an integer"),
            };
            match #store().lock().unwrap().get(&id) {
                Some(entity) => {
                    let row = ::serde_json::to_value(entity).unwrap();
                    ::nirdosha_rt::Response::html(200, ::nirdosha_rt::wizard::wizard_done_html("Done", &__all_fields(), &row))
                }
                None => ::nirdosha_rt::Response::not_found(),
            }
        }
    });

    let start_route = route("get", &input.access, &base_path, "Start wizard", |_| {
        let first_step_path = format!("{}/step/1", path_str.trim_end_matches('/'));
        quote! { ::nirdosha_rt::Response::redirect(#first_step_path) }
    });

    quote! {
        fn #mount(router: ::nirdosha_rt::Router) -> ::nirdosha_rt::Router {
            #finalize_fn
            #( #step_items )*
            router
                #start_route
                #( #step_routes )*
                #done_route
        }
    }
}

fn route(method: &str, access: &Access, path: &TokenStream2, summary: &str, body_fn: impl Fn(Option<TokenStream2>) -> TokenStream2) -> TokenStream2 {
    let (plain, gated) = match method {
        "get" => (quote!(get), quote!(get_gated)),
        "post" => (quote!(post), quote!(post_gated)),
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
