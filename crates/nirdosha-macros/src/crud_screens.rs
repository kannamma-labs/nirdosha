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
use syn::{braced, bracketed, Ident, LitStr, Token, Type};

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

struct FieldDef {
    name: Ident,
    ty: Type,
}

/// Optional `guard: { table: <ident>, purpose: "..." }` clause -- see
/// `crates/nirdosha-guard-screens`'s own doc comment for what `table`
/// must return (`&'static GuardedTable<D, Entity>`, any `D: StoreDriver`
/// -- the macro never names `D` itself, it's inferred at the app's own
/// call site) and why a policy id is never named here: which corpus
/// policy applies is resolved by `GuardedTable`'s own evaluator call
/// (role + action + resource + purpose), the same way any other
/// `guard_policy!` consumer works.
///
/// Phase B scope (RTM live-demo plan): wires list/export/JSON-list
/// (Phase A), detail-by-id, edit-form, and update -- all keyed by
/// `GuardedEntity::row_id()`'s real `String` identity (`params.get("id")`
/// used directly, no `i64` parse) rather than the `store:`/`SharedTable`
/// path's synthetic integer id. **`create`/`delete` are deliberately not
/// wired for `guard:` in this phase**: every real corpus create policy
/// surveyed so far is service-principal-driven (e.g. `ingest-create-txn`
/// `for SvcIngest`), not a human `<form>` submission, and no corpus
/// module surveyed so far has a per-row human `delete` policy at all
/// (`retention-purge` is a catalog-level Admin action, not a per-row
/// CRUD delete) -- building a generic guarded create/delete path before
/// a real module needs one would be speculative machinery this phase
/// doesn't add. When a module *does* need guarded create/delete, that's
/// the point to design it against that module's own real shape, not
/// before. `can_create`/`can_delete` are forced `false` for a guarded
/// screen regardless of `input.create`/`input.delete`'s declared
/// access -- see `expand_parsed`'s own comment at the guard branch.
///
/// `update_fields: [name: Type, ...]` is a second, narrower field list,
/// independent of the top-level `fields:` (which stays whatever a
/// list/detail screen wants to *display*). It exists because a real
/// corpus update policy's `field_policy` almost always allows a much
/// smaller set than a screen displays (`analyst-flag-transaction`
/// allows only `analyst_flag`/`analyst_flag_reason` out of a dozen-plus
/// displayed `transaction` columns) -- G1's field-policy check runs
/// against exactly the fields *submitted*, so reusing the display
/// `fields:` list as the update's changed-field set would make every
/// such update fail closed on every field it merely displays. Omitting
/// `update_fields` entirely (M6's current shape: no human role may
/// write a transaction at all) means no edit-form/update route is
/// registered for this screen -- see `expand_parsed`'s guard branch on
/// `edit_form_route`/`update_html_route`/`update_api_route`.
///
/// **There is deliberately no separate `read_only: true` flag.** Delete
/// is *already* never registered for a guarded screen (see above); a
/// screen that also omits both `create_fields:` and `update_fields:` is
/// therefore *already* read-only by construction -- no create, update,
/// or delete route exists at all, not merely hidden behind a UI flag.
/// Adding a flag for something already true by omission would be the
/// exact "speculative machinery before a real module needs it" this
/// file's own module doc comment already argues against for
/// `create`/`delete`. A `guard: { table, purpose }` block with no
/// `create_fields:`/`update_fields:` at all is the read-only-by-
/// construction primitive a screen like 21.1 (Auditor Read-Only
/// Portal) needs -- see `m21_restricted.nir::mount_auditor_portal`,
/// which reuses this exact property by hand-writing its routes
/// (`guarded_snapshot` only, no write handler registered) rather than
/// through this macro, since it also needs the watermark banner
/// (`nirdosha_rt::watermark`) `list_html`'s own output has no hook for.
struct GuardConfig {
    table: Ident,
    purpose: LitStr,
    update_fields: Option<Vec<FieldDef>>,
    /// Like `update_fields`, but for a guarded `create` -- registers
    /// `/{path}/new` and the create submit routes through
    /// `GuardedTable::guarded_insert` instead of leaving them unwired.
    /// Added once a real corpus policy needed it (`60_case_management.nir`'s
    /// `analyst-create-case`, a genuine human `<form>` submission) --
    /// unlike `create`/`delete`'s blanket "no real corpus policy needs
    /// this yet" omission this file's own module doc comment describes,
    /// `create_fields` is opt-in per screen, not a default every guarded
    /// screen gets.
    create_fields: Option<Vec<FieldDef>>,
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

        let mut update_fields = None;
        let mut create_fields = None;
        while input.peek(Ident) {
            let fork = input.fork();
            let ahead: Ident = fork.parse()?;
            let target = if ahead == "update_fields" {
                &mut update_fields
            } else if ahead == "create_fields" {
                &mut create_fields
            } else {
                break;
            };
            input.parse::<Ident>()?;
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
            *target = Some(fields);
            let _ = input.parse::<Token![,]>();
        }

        Ok(GuardConfig { table, purpose, update_fields, create_fields })
    }
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
    guard: Option<GuardConfig>,
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

        // Additive-only: every existing `crud_screens!` invocation ends
        // right here (no trailing content), so this only ever fires for
        // an invocation that opted in.
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
            guard,
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
    let export_path_str = format!("{path_str}/export.csv");
    let export_path = quote! { #export_path_str };

    let title = path_str.trim_start_matches('/').to_string();

    let field_idents: Vec<&Ident> = input.fields.iter().map(|f| &f.name).collect();
    let field_names: Vec<String> = input.fields.iter().map(|f| f.name.to_string()).collect();
    let field_types: Vec<&Type> = input.fields.iter().map(|f| &f.ty).collect();
    let input_types: Vec<&'static str> = input.fields.iter().map(|f| input_type_for(&f.ty)).collect();

    // A guarded screen's editable set -- see `GuardConfig::update_fields`'s
    // own doc comment for why this is independent of the display
    // `fields:` list above. `None` (no `update_fields:` in the `guard:`
    // block) means this screen offers no edit UI at all.
    let update_fields: Option<&[FieldDef]> = input.guard.as_ref().and_then(|g| g.update_fields.as_deref());
    let update_field_idents: Vec<&Ident> = update_fields.map(|fs| fs.iter().map(|f| &f.name).collect()).unwrap_or_default();
    let update_field_names: Vec<String> = update_fields.map(|fs| fs.iter().map(|f| f.name.to_string()).collect()).unwrap_or_default();
    let update_field_types: Vec<&Type> = update_fields.map(|fs| fs.iter().map(|f| &f.ty).collect()).unwrap_or_default();
    let update_input_types: Vec<&'static str> = update_fields.map(|fs| fs.iter().map(|f| input_type_for(&f.ty)).collect()).unwrap_or_default();

    // Same shape as `update_fields` above, for a guarded `create` --
    // `None` (no `create_fields:` in the `guard:` block) means this
    // screen offers no create UI at all, same posture `update_fields`
    // already has for edit.
    let create_fields: Option<&[FieldDef]> = input.guard.as_ref().and_then(|g| g.create_fields.as_deref());
    let create_field_idents: Vec<&Ident> = create_fields.map(|fs| fs.iter().map(|f| &f.name).collect()).unwrap_or_default();
    let create_field_names: Vec<String> = create_fields.map(|fs| fs.iter().map(|f| f.name.to_string()).collect()).unwrap_or_default();
    let create_field_types: Vec<&Type> = create_fields.map(|fs| fs.iter().map(|f| &f.ty).collect()).unwrap_or_default();
    let create_input_types: Vec<&'static str> = create_fields.map(|fs| fs.iter().map(|f| input_type_for(&f.ty)).collect()).unwrap_or_default();

    // A guarded screen's `can_create`/`can_delete` reflect whether
    // `guard.create_fields`/an eventual `delete_fields` equivalent is
    // present, not `input.create`/`input.delete`'s declared access (see
    // `GuardConfig`'s own doc comment -- `create`/`update`/`delete` stay
    // `public` on every guarded RTM invocation today, since no RTM role
    // name can satisfy `crud_screens!`'s own `requires role "..."`
    // grammar; `guard:` is the real authorization for these screens).
    // `delete` has no guarded path yet (no per-row corpus delete policy
    // exists anywhere to build one against) so it stays unconditionally
    // unwired for a guarded screen, same as before this change.
    let can_create = if input.guard.is_some() { create_fields.is_some() } else { matches!(input.create, Access::Public) };
    let can_update = matches!(input.update, Access::Public);
    let can_delete = input.guard.is_none() && matches!(input.delete, Access::Public);

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
            #store().insert(entity.id, entity.clone());
            Ok(entity)
        }

        #update_attr
        fn __update(id: i64, values: &::std::collections::HashMap<String, String>) -> ::std::result::Result<#entity, Vec<String>> {
            let parsed = __parse(values)?;
            #store().update(&id, |entity| match entity {
                Some(entity) => {
                    #( entity.#field_idents = parsed.#field_idents; )*
                    Ok(entity.clone())
                }
                None => Err(vec!["not found".to_string()]),
            })
        }

        #delete_attr
        fn __delete(id: i64) -> bool {
            #store().remove(&id).is_some()
        }

        /// `?q=` filters rows generically across every declared field,
        /// regardless of type — each field's `Display` output is
        /// substring-matched case-insensitively. Shared by the List
        /// screen, its JSON form, and the CSV export, so all three
        /// agree on what a search actually returns.
        fn __matching_rows(req: &::nirdosha_rt::Request) -> Vec<#entity> {
            let q = req.query().get("q").map(|s| s.to_lowercase());
            let mut rows: Vec<#entity> = #store()
                .snapshot()
                .into_iter()
                .map(|(_, e)| e)
                .filter(|e| match &q {
                    None => true,
                    Some(needle) if needle.is_empty() => true,
                    Some(needle) => {
                        let searchable: Vec<String> = vec![ #( e.#field_idents.to_string() ),* ];
                        searchable.iter().any(|v| v.to_lowercase().contains(needle))
                    }
                })
                .collect();
            rows.sort_by_key(|e| e.id);
            rows
        }
    };

    let new_title = format!("New {title}");
    let edit_title = format!("Edit {title}");

    // ---- read: list + detail, HTML + JSON ----
    // When `guard:` is set, list/export go through
    // `nirdosha_guard_screens::GuardedTable::guarded_snapshot` instead of
    // `__matching_rows`/`#store()` -- real per-request policy evaluation
    // (role/action/resource/purpose), tenant scoping, and field masking,
    // not just the `Access::Role` presence check `route()` builds
    // everywhere else in this file. See `GuardConfig`'s own doc comment
    // for exactly what stays unwired in this phase.
    let list_html_route = if let Some(guard) = &input.guard {
        let table = &guard.table;
        let purpose = &guard.purpose;
        quote! {
            .get_with_auth(#path, "List", |req, _params, auth| {
                match #table().guarded_snapshot(auth, #purpose) {
                    Ok(matched) => {
                        let rows: Vec<::serde_json::Value> = matched.iter().map(|e| ::serde_json::to_value(e).unwrap()).collect();
                        let q = req.query().get("q").cloned();
                        ::nirdosha_rt::Response::html(200, ::nirdosha_rt::screens::list_html(#title, #path, &__fields(), &rows, #can_create, q.as_deref()))
                    }
                    Err(e) => ::nirdosha_guard_screens::guard_error_response(e),
                }
            })
        }
    } else {
        route("get", &input.read, &path, "List", |_| {
            quote! {
                let matched = __matching_rows(req);
                let rows: Vec<::serde_json::Value> = matched.iter().map(|e| ::serde_json::to_value(e).unwrap()).collect();
                let q = req.query().get("q").cloned();
                ::nirdosha_rt::Response::html(200, ::nirdosha_rt::screens::list_html(#title, #path, &__fields(), &rows, #can_create, q.as_deref()))
            }
        })
    };
    let export_route = if let Some(guard) = &input.guard {
        let table = &guard.table;
        let purpose = &guard.purpose;
        quote! {
            .get_with_auth(#export_path, "Export CSV", |_req, _params, auth| {
                match #table().guarded_snapshot(auth, #purpose) {
                    Ok(matched) => {
                        let mut csv = String::new();
                        csv.push_str(&[ #(#field_names),* ].join(","));
                        csv.push('\n');
                        for e in &matched {
                            let row = ::serde_json::to_value(e).unwrap();
                            let cells: Vec<String> = vec![ #( ::nirdosha_rt::screens::csv_escape(&::nirdosha_rt::screens::value_display(row.get(#field_names).unwrap_or(&::serde_json::Value::Null))) ),* ];
                            csv.push_str(&cells.join(","));
                            csv.push('\n');
                        }
                        ::nirdosha_rt::Response::csv(200, csv)
                    }
                    Err(e) => ::nirdosha_guard_screens::guard_error_response(e),
                }
            })
        }
    } else {
        route("get", &input.read, &export_path, "Export CSV", |_| {
            quote! {
                let matched = __matching_rows(req);
                let mut csv = String::new();
                csv.push_str(&[ #(#field_names),* ].join(","));
                csv.push('\n');
                for e in &matched {
                    let cells: Vec<String> = vec![ #( ::nirdosha_rt::screens::csv_escape(&e.#field_idents.to_string()) ),* ];
                    csv.push_str(&cells.join(","));
                    csv.push('\n');
                }
                ::nirdosha_rt::Response::csv(200, csv)
            }
        })
    };
    let list_api_route = if let Some(guard) = &input.guard {
        let table = &guard.table;
        let purpose = &guard.purpose;
        quote! {
            .get_with_auth(#api_path, "List (JSON)", |_req, _params, auth| {
                match #table().guarded_snapshot(auth, #purpose) {
                    Ok(matched) => {
                        let rows: Vec<::serde_json::Value> = matched.iter().map(|e| ::serde_json::to_value(e).unwrap()).collect();
                        ::nirdosha_rt::Response::json(200, &::serde_json::json!(rows))
                    }
                    Err(e) => ::nirdosha_guard_screens::guard_error_response(e),
                }
            })
        }
    } else {
        route("get", &input.read, &api_path, "List (JSON)", |_| {
            quote! {
                let matched = __matching_rows(req);
                let rows: Vec<::serde_json::Value> = matched.iter().map(|e| ::serde_json::to_value(e).unwrap()).collect();
                ::nirdosha_rt::Response::json(200, &::serde_json::json!(rows))
            }
        })
    };
    let detail_html_route = if let Some(guard) = &input.guard {
        let table = &guard.table;
        let purpose = &guard.purpose;
        quote! {
            .get_with_auth(#id_path, "Detail", |_req, params, auth| {
                let Some(id) = params.get("id") else { return ::nirdosha_rt::Response::bad_request("id required") };
                match #table().guarded_get(auth, #purpose, id) {
                    Ok(Some(entity)) => {
                        let row = ::serde_json::to_value(&entity).unwrap();
                        ::nirdosha_rt::Response::html(200, ::nirdosha_rt::screens::detail_html(#title, #path, id, &__fields(), &row, #can_update, #can_delete))
                    }
                    Ok(None) => ::nirdosha_rt::Response::not_found(),
                    Err(e) => ::nirdosha_guard_screens::guard_error_response(e),
                }
            })
        }
    } else {
        route("get", &input.read, &id_path, "Detail", |_| {
            quote! {
                let id: i64 = match params.get("id").and_then(|s| s.parse().ok()) { Some(v) => v, None => return ::nirdosha_rt::Response::bad_request("id must be an integer") };
                match #store().get(&id) {
                    Some(entity) => {
                        let row = ::serde_json::to_value(&entity).unwrap();
                        ::nirdosha_rt::Response::html(200, ::nirdosha_rt::screens::detail_html(#title, #path, id, &__fields(), &row, #can_update, #can_delete))
                    }
                    None => ::nirdosha_rt::Response::not_found(),
                }
            }
        })
    };
    let detail_api_route = if let Some(guard) = &input.guard {
        let table = &guard.table;
        let purpose = &guard.purpose;
        quote! {
            .get_with_auth(#api_id_path, "Detail (JSON)", |_req, params, auth| {
                let Some(id) = params.get("id") else { return ::nirdosha_rt::Response::bad_request("id required") };
                match #table().guarded_get(auth, #purpose, id) {
                    Ok(Some(entity)) => ::nirdosha_rt::Response::json(200, &::serde_json::to_value(&entity).unwrap()),
                    Ok(None) => ::nirdosha_rt::Response::not_found(),
                    Err(e) => ::nirdosha_guard_screens::guard_error_response(e),
                }
            })
        }
    } else {
        route("get", &input.read, &api_id_path, "Detail (JSON)", |_| {
            quote! {
                let id: i64 = match params.get("id").and_then(|s| s.parse().ok()) { Some(v) => v, None => return ::nirdosha_rt::Response::bad_request("id must be an integer") };
                match #store().get(&id) {
                    Some(entity) => ::nirdosha_rt::Response::json(200, &::serde_json::to_value(&entity).unwrap()),
                    None => ::nirdosha_rt::Response::not_found(),
                }
            }
        })
    };

    // ---- create: new-form (GET), submit (POST html + POST json) ----
    // Guarded mode only registers these when `guard.create_fields` is
    // set (see `GuardConfig::create_fields`'s own doc comment) --
    // otherwise unregistered entirely, same as before this addition.
    let new_form_route = if input.guard.is_some() {
        if create_fields.is_none() {
            quote! {}
        } else {
            route("get", &Access::Public, &new_path, "New form", |_| {
                quote! {
                    let values: ::std::collections::HashMap<String, String> = ::std::collections::HashMap::new();
                    ::nirdosha_rt::Response::html(200, ::nirdosha_rt::screens::form_html(#new_title, #path, &__guarded_create_fields(), &values, &[]))
                }
            })
        }
    } else {
        route("get", &input.create, &new_path, "New form", |_| {
            quote! {
                let values: ::std::collections::HashMap<String, String> = ::std::collections::HashMap::new();
                ::nirdosha_rt::Response::html(200, ::nirdosha_rt::screens::form_html(#new_title, #path, &__fields(), &values, &[]))
            }
        })
    };
    let create_html_route = if let Some(guard) = &input.guard {
        if create_fields.is_none() {
            quote! {}
        } else {
            let table = &guard.table;
            let purpose = &guard.purpose;
            quote! {
                .post_with_auth(#path, "Create", |req, _params, auth| {
                    let values = req.form_or_json();
                    let entity = match __guarded_parse_create_fields(&values) {
                        Ok(e) => e,
                        Err(errors) => return ::nirdosha_rt::Response::html(400, ::nirdosha_rt::screens::form_html(#new_title, #path, &__guarded_create_fields(), &values, &errors)),
                    };
                    let row_id = ::nirdosha_guard_screens::GuardedEntity::row_id(&entity);
                    let submitted = __guarded_submitted_create_fields(&values);
                    match #table().guarded_insert_checked(auth, #purpose, &submitted, entity) {
                        Ok(_entity) => ::nirdosha_rt::Response::redirect(format!("{}/{}", #path, row_id)),
                        Err(e) => ::nirdosha_guard_screens::guard_error_response(e),
                    }
                })
            }
        }
    } else {
        route("post", &input.create, &path, "Create", |proof| {
            let arg = proof_arg(&proof);
            quote! {
                let values = req.form_or_json();
                match __create(#arg &values) {
                    Ok(entity) => ::nirdosha_rt::Response::redirect(format!("{}/{}", #path, entity.id)),
                    Err(errors) => ::nirdosha_rt::Response::html(400, ::nirdosha_rt::screens::form_html(#new_title, #path, &__fields(), &values, &errors)),
                }
            }
        })
    };
    let create_api_route = if let Some(guard) = &input.guard {
        if create_fields.is_none() {
            quote! {}
        } else {
            let table = &guard.table;
            let purpose = &guard.purpose;
            quote! {
                .post_with_auth(#api_path, "Create (JSON)", |req, _params, auth| {
                    let values = req.form_or_json();
                    let entity = match __guarded_parse_create_fields(&values) {
                        Ok(e) => e,
                        Err(errors) => return ::nirdosha_rt::Response::json(400, &::serde_json::json!({ "errors": errors })),
                    };
                    let submitted = __guarded_submitted_create_fields(&values);
                    match #table().guarded_insert_checked(auth, #purpose, &submitted, entity) {
                        Ok(entity) => ::nirdosha_rt::Response::json(201, &::serde_json::to_value(&entity).unwrap()),
                        Err(e) => ::nirdosha_guard_screens::guard_error_response(e),
                    }
                })
            }
        }
    } else {
        route("post", &input.create, &api_path, "Create (JSON)", |proof| {
            let arg = proof_arg(&proof);
            quote! {
                let values = req.form_or_json();
                match __create(#arg &values) {
                    Ok(entity) => ::nirdosha_rt::Response::json(201, &::serde_json::to_value(&entity).unwrap()),
                    Err(errors) => ::nirdosha_rt::Response::json(400, &::serde_json::json!({ "errors": errors })),
                }
            }
        })
    };

    // ---- update: edit-form (GET), submit (POST html, PUT json) ----
    // Guarded mode parses submitted values into a macro-local
    // `__GuardedFields` struct (just the `fields:` subset, never the
    // full `#entity`) before calling `GuardedTable::guarded_update` --
    // unlike `__parse`, which needs `#entity: Default` to fill in every
    // field a `fields:` entry doesn't cover, a guarded update only ever
    // *merges* the submitted subset onto the row `GuardedTable` fetches
    // internally, so it never needs to construct a whole `#entity` value
    // at all.
    let guarded_fields_helpers = if update_fields.is_some() {
        quote! {
            fn __guarded_update_fields() -> Vec<::nirdosha_rt::screens::FieldSpec> {
                vec![ #( ::nirdosha_rt::screens::FieldSpec { name: #update_field_names, input_type: #update_input_types } ),* ]
            }
            struct __GuardedFields { #( #update_field_idents: #update_field_types, )* }
            fn __guarded_parse_fields(values: &::std::collections::HashMap<String, String>) -> ::std::result::Result<__GuardedFields, Vec<String>> {
                let mut errors: Vec<String> = Vec::new();
                #(
                    let #update_field_idents = match <#update_field_types as ::nirdosha_rt::screens::ParseField>::parse_field(values.get(#update_field_names).map(|s| s.as_str())) {
                        Ok(v) => v,
                        Err(e) => { errors.push(format!("{}: {}", #update_field_names, e)); Default::default() }
                    };
                )*
                if !errors.is_empty() { return Err(errors); }
                Ok(__GuardedFields { #( #update_field_idents ),* })
            }
            // `update_fields:` itself *is* "what this screen may edit" --
            // G1's `field_policy` check runs against exactly this set,
            // not the merged row's full field set (see
            // `GuardedTable::guarded_update`'s own doc comment for why: a
            // real update policy's `forbidden(...)` list names fields
            // that are always present on the merged row whether or not
            // this write touched them).
            fn __guarded_changed_fields() -> ::std::collections::HashSet<String> {
                [ #(#update_field_names),* ].into_iter().map(|s: &str| s.to_string()).collect()
            }
        }
    } else {
        quote! {}
    };
    // Same shape for a guarded `create` -- unlike the update path, this
    // one constructs a whole `#entity` (via `Default`, same as `__parse`
    // does for the ungated path) rather than merging onto a fetched row,
    // since there is no existing row yet.
    let guarded_create_helpers = if create_fields.is_some() {
        quote! {
            fn __guarded_create_fields() -> Vec<::nirdosha_rt::screens::FieldSpec> {
                vec![ #( ::nirdosha_rt::screens::FieldSpec { name: #create_field_names, input_type: #create_input_types } ),* ]
            }
            fn __guarded_parse_create_fields(values: &::std::collections::HashMap<String, String>) -> ::std::result::Result<#entity, Vec<String>> {
                let mut errors: Vec<String> = Vec::new();
                #(
                    let #create_field_idents = match <#create_field_types as ::nirdosha_rt::screens::ParseField>::parse_field(values.get(#create_field_names).map(|s| s.as_str())) {
                        Ok(v) => v,
                        Err(e) => { errors.push(format!("{}: {}", #create_field_names, e)); Default::default() }
                    };
                )*
                if !errors.is_empty() { return Err(errors); }
                Ok(#entity { id: 0, #( #create_field_idents ),*, ..Default::default() })
            }
            // G1's `required(...)` check needs "was this field actually
            // submitted," which a fully-`Default`-filled `#entity` can't
            // represent (every field always has *some* value) --
            // computed from the raw form/JSON keys instead, restricted
            // to this screen's own declared `create_fields:` set so an
            // unrelated extra key in `values` can't masquerade as a
            // real field. A key present but empty (`rationale=`) counts
            // as NOT submitted -- an HTML `<form>` has no way to omit a
            // named input entirely, so "present but blank" is the only
            // signal a browser submission can give for "I didn't fill
            // this in."
            fn __guarded_submitted_create_fields(values: &::std::collections::HashMap<String, String>) -> ::std::collections::HashSet<String> {
                let exempt: &[&str] = <#entity as ::nirdosha_guard_screens::GuardedEntity>::create_field_policy_exempt();
                [ #(#create_field_names),* ]
                    .into_iter()
                    .filter(|name: &&str| values.get(*name).is_some_and(|v| !v.is_empty()))
                    .filter(|name: &&str| !exempt.contains(name))
                    .map(|s: &str| s.to_string())
                    .collect()
            }
        }
    } else {
        quote! {}
    };
    let edit_form_route = if let Some(guard) = &input.guard {
        if update_fields.is_none() {
            quote! {}
        } else {
            let table = &guard.table;
            let purpose = &guard.purpose;
            quote! {
                .get_with_auth(#edit_path, "Edit form", |_req, params, auth| {
                    let Some(id) = params.get("id") else { return ::nirdosha_rt::Response::bad_request("id required") };
                    match #table().guarded_get(auth, #purpose, id) {
                        Ok(Some(entity)) => {
                            let row = ::serde_json::to_value(&entity).unwrap();
                            let values = ::nirdosha_rt::screens::row_to_form_values(&__guarded_update_fields(), &row);
                            ::nirdosha_rt::Response::html(200, ::nirdosha_rt::screens::form_html(#edit_title, &format!("{}/{}/edit", #path, id), &__guarded_update_fields(), &values, &[]))
                        }
                        Ok(None) => ::nirdosha_rt::Response::not_found(),
                        Err(e) => ::nirdosha_guard_screens::guard_error_response(e),
                    }
                })
            }
        }
    } else {
        route("get", &input.update, &edit_path, "Edit form", |_| {
            quote! {
                let id: i64 = match params.get("id").and_then(|s| s.parse().ok()) { Some(v) => v, None => return ::nirdosha_rt::Response::bad_request("id must be an integer") };
                match #store().get(&id) {
                    Some(entity) => {
                        let row = ::serde_json::to_value(&entity).unwrap();
                        let values = ::nirdosha_rt::screens::row_to_form_values(&__fields(), &row);
                        ::nirdosha_rt::Response::html(200, ::nirdosha_rt::screens::form_html(#edit_title, &format!("{}/{}/edit", #path, id), &__fields(), &values, &[]))
                    }
                    None => ::nirdosha_rt::Response::not_found(),
                }
            }
        })
    };
    let update_html_route = if let Some(guard) = &input.guard {
        if update_fields.is_none() {
            quote! {}
        } else {
            let table = &guard.table;
            let purpose = &guard.purpose;
            quote! {
                .post_with_auth(#edit_path, "Update", |req, params, auth| {
                    let Some(id) = params.get("id").map(|s| s.to_string()) else { return ::nirdosha_rt::Response::bad_request("id required") };
                    let values = req.form_or_json();
                    let parsed = match __guarded_parse_fields(&values) {
                        Ok(p) => p,
                        Err(errors) => return ::nirdosha_rt::Response::html(400, ::nirdosha_rt::screens::form_html(#edit_title, &format!("{}/{}/edit", #path, id), &__guarded_update_fields(), &values, &errors)),
                    };
                    let changed = __guarded_changed_fields();
                    match #table().guarded_update(auth, #purpose, &id, &changed, move |entity| { #( entity.#update_field_idents = parsed.#update_field_idents; )* }) {
                        Ok(_entity) => ::nirdosha_rt::Response::redirect(format!("{}/{}", #path, id)),
                        Err(e) => ::nirdosha_guard_screens::guard_error_response(e),
                    }
                })
            }
        }
    } else {
        route("post", &input.update, &edit_path, "Update", |proof| {
            let arg = proof_arg(&proof);
            quote! {
                let id: i64 = match params.get("id").and_then(|s| s.parse().ok()) { Some(v) => v, None => return ::nirdosha_rt::Response::bad_request("id must be an integer") };
                let values = req.form_or_json();
                match __update(#arg id, &values) {
                    Ok(entity) => ::nirdosha_rt::Response::redirect(format!("{}/{}", #path, entity.id)),
                    Err(errors) => ::nirdosha_rt::Response::html(400, ::nirdosha_rt::screens::form_html(#edit_title, &format!("{}/{}/edit", #path, id), &__fields(), &values, &errors)),
                }
            }
        })
    };
    let update_api_route = if let Some(guard) = &input.guard {
        if update_fields.is_none() {
            quote! {}
        } else {
            let table = &guard.table;
            let purpose = &guard.purpose;
            quote! {
                .put_with_auth(#api_id_path, "Update (JSON)", |req, params, auth| {
                    let Some(id) = params.get("id").map(|s| s.to_string()) else { return ::nirdosha_rt::Response::bad_request("id required") };
                    let values = req.form_or_json();
                    let parsed = match __guarded_parse_fields(&values) {
                        Ok(p) => p,
                        Err(errors) => return ::nirdosha_rt::Response::json(400, &::serde_json::json!({ "errors": errors })),
                    };
                    let changed = __guarded_changed_fields();
                    match #table().guarded_update(auth, #purpose, &id, &changed, move |entity| { #( entity.#update_field_idents = parsed.#update_field_idents; )* }) {
                        Ok(entity) => ::nirdosha_rt::Response::json(200, &::serde_json::to_value(&entity).unwrap()),
                        Err(e) => ::nirdosha_guard_screens::guard_error_response(e),
                    }
                })
            }
        }
    } else {
        route("put", &input.update, &api_id_path, "Update (JSON)", |proof| {
            let arg = proof_arg(&proof);
            quote! {
                let id: i64 = match params.get("id").and_then(|s| s.parse().ok()) { Some(v) => v, None => return ::nirdosha_rt::Response::bad_request("id must be an integer") };
                let values = req.form_or_json();
                match __update(#arg id, &values) {
                    Ok(entity) => ::nirdosha_rt::Response::json(200, &::serde_json::to_value(&entity).unwrap()),
                    Err(errors) => ::nirdosha_rt::Response::json(400, &::serde_json::json!({ "errors": errors })),
                }
            }
        })
    };

    // ---- delete: typed-confirmation (GET), submit (POST html, DELETE json) ----
    // Never registered for a guarded screen -- see `GuardConfig`'s own
    // doc comment.
    let delete_confirm_route = if input.guard.is_some() {
        quote! {}
    } else {
        route("get", &input.delete, &delete_path, "Delete confirmation", |_| {
            quote! {
                let id: i64 = match params.get("id").and_then(|s| s.parse().ok()) { Some(v) => v, None => return ::nirdosha_rt::Response::bad_request("id must be an integer") };
                ::nirdosha_rt::Response::html(200, ::nirdosha_rt::screens::delete_confirm_html(#title, &format!("{}/{}/delete", #path, id), "DELETE", &format!("#{id}")))
            }
        })
    };
    let delete_html_route = if input.guard.is_some() {
        quote! {}
    } else {
        route("post", &input.delete, &delete_path, "Delete", |proof| {
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
        })
    };
    let delete_api_route = if input.guard.is_some() {
        quote! {}
    } else {
        route("delete", &input.delete, &api_id_path, "Delete (JSON)", |proof| {
            let arg = proof_arg(&proof);
            quote! {
                let id: i64 = match params.get("id").and_then(|s| s.parse().ok()) { Some(v) => v, None => return ::nirdosha_rt::Response::bad_request("id must be an integer") };
                if __delete(#arg id) { ::nirdosha_rt::Response::no_content() } else { ::nirdosha_rt::Response::not_found() }
            }
        })
    };

    quote! {
        // `pub`, matching `login!`/`app_shell!`'s generated mount fns
        // (all six screen-archetype macros now agree on this — see
        // `dashboard!`/`kanban_board!`/`wizard!`/`settings_screen!`/
        // `communication_feed!`'s own identical fix). A private mount fn
        // only ever worked because every existing caller invoked the
        // macro and called the result in the same file; a real
        // multi-module app (screens split one-per-file, e.g. RTM's
        // `screens/m06_transactions.nir` mounted from a separate
        // `src/bin/serve.nir`) needs to call it from outside
        // that module.
        pub fn #mount(router: ::nirdosha_rt::Router) -> ::nirdosha_rt::Router {
            #fields_fn
            #parse_fn
            #core_fns
            #guarded_fields_helpers
            #guarded_create_helpers

            // Literal-suffix routes (`/new`, `/{id}/edit`, `/{id}/delete`)
            // must be registered before `/{id}` itself: `Router::dispatch`
            // matches in registration order, and a `{id}` wildcard
            // segment matches the literal text "new" just as readily as
            // a real id -- so `/products/new` would otherwise be
            // swallowed by `/products/{id}` and never reached.
            router
                #list_html_route
                #list_api_route
                #export_route
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
