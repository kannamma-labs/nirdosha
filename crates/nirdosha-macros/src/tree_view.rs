//! `tree_view! { .. }` — a hierarchical view over one `GuardedTable`
//! holding a self-referencing (parent/child) entity. RTM's 5.6
//! "Related Parties / UBO" archetype: the interim is a flat table; the
//! tree render needs exactly one macro and nothing else.
//!
//! ```ignore
//! nirdosha_rt::tree_view! {
//!     mount: mount_relation_tree,
//!     entity: RelationRow,
//!     table: relation_table,
//!     path: "/relations/tree",
//!     title: "Related Parties / UBO",
//!     purpose: "Operations",
//!     access: requires role "Analyst",
//!     id_field: relation_id,
//!     parent_field: parent_id,
//!     label_field: name,
//! }
//! ```
//!
//! `GET <path>` reads the visible rows once through
//! `GuardedTable::guarded_snapshot` (the **read** guard action — the
//! screen's corpus must carry an `action == "read"` allow record), then
//! nests them in-process via `screens::tree_view_html`: children
//! grouped under `parent_field`, roots are rows whose parent is empty
//! or not itself a visible row. Rendering is deterministic (siblings
//! sorted by id) and cycle-safe (a parent loop renders a `(cycle)`
//! marker instead of recursing forever).
//!
//! **There is no unguarded path**, and the guard is the single
//! authority for what the tree may show: the snapshot is decoded with
//! the guard's masks applied, so a masked id/parent/label field nests
//! into its mask value, and a dropped field cannot be nested on at all
//! (its rows render as unresolved roots, honestly). The declared
//! `access:` is vestigial route metadata (OpenAPI summaries), exactly
//! like every other guarded screen.
//!
//! v1 scope, disclosed: expand-all rendering (no collapse chrome), no
//! client-side JS, depth-capped at 24 levels per branch.

use proc_macro::TokenStream;
use quote::quote;
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::{Ident, LitStr, Token, Type};

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

pub struct TreeViewInput {
    mount: Ident,
    entity: Type,
    table: Ident,
    path: LitStr,
    title: LitStr,
    purpose: LitStr,
    access: Access,
    id_field: Ident,
    parent_field: Ident,
    label_field: Ident,
}

impl Parse for TreeViewInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut mount: Option<Ident> = None;
        let mut entity: Option<Type> = None;
        let mut table: Option<Ident> = None;
        let mut path: Option<LitStr> = None;
        let mut title: Option<LitStr> = None;
        let mut purpose: Option<LitStr> = None;
        let mut access: Option<Access> = None;
        let mut id_field: Option<Ident> = None;
        let mut parent_field: Option<Ident> = None;
        let mut label_field: Option<Ident> = None;

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
                "id_field" => id_field = Some(input.parse()?),
                "parent_field" => parent_field = Some(input.parse()?),
                "label_field" => label_field = Some(input.parse()?),
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("expected `mount`, `entity`, `table`, `path`, `title`, `purpose`, `access`, `id_field`, `parent_field`, or `label_field`, found `{other}`"),
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
        let purpose = purpose.ok_or_else(|| syn::Error::new(input.span(), "`purpose:` is required (the read policy's purpose)"))?;
        let access = access.unwrap_or(Access::Public);
        let id_field = id_field.ok_or_else(|| syn::Error::new(input.span(), "`id_field:` is required"))?;
        let parent_field = parent_field.ok_or_else(|| syn::Error::new(input.span(), "`parent_field:` is required"))?;
        let label_field = label_field.ok_or_else(|| syn::Error::new(input.span(), "`label_field:` is required"))?;
        Ok(TreeViewInput { mount, entity, table, path, title, purpose, access, id_field, parent_field, label_field })
    }
}

pub fn expand(input: TokenStream) -> TokenStream {
    let parsed = match syn::parse::<TreeViewInput>(input) {
        Ok(p) => p,
        Err(e) => return e.to_compile_error().into(),
    };

    let TreeViewInput { mount, entity, table, path, title, purpose, access, id_field, parent_field, label_field } = &parsed;

    // `entity:` is load-bearing as a type-level contract, same posture
    // as report_builder!: the type must be a real GuardedEntity (the
    // same type `table:` stores) — a mismatch is a compile error here.
    let entity_assert = quote! {
        const _: () = {
            fn assert_guarded_entity<E: ::nirdosha_guard_screens::GuardedEntity>() {}
            fn _check() {
                assert_guarded_entity::<#entity>();
            }
        };
    };

    // The declared `access:` is vestigial route metadata on a guarded
    // screen; the route resolves an `Auth` and the guard corpus decides.
    let _ = access;

    let id_name = id_field.to_string();
    let parent_name = parent_field.to_string();
    let label_name = label_field.to_string();

    quote! {
        #entity_assert
        pub fn #mount(router: ::nirdosha_rt::Router) -> ::nirdosha_rt::Router {
            router
                .get_with_auth(#path, #title, |_req, _params, auth| {
                    // One guarded read; everything below is pure
                    // presentation over what the guard let through
                    // (masks already applied at decode).
                    let rows = match #table().guarded_snapshot(auth, #purpose) {
                        Ok(rows) => rows,
                        Err(e) => return ::nirdosha_guard_screens::guard_error_response(e),
                    };
                    let values: Vec<::serde_json::Value> = rows.iter().map(|r| ::serde_json::to_value(r).unwrap()).collect();
                    nirdosha_rt::Response::html(200, nirdosha_rt::screens::tree_view_html(
                        #title, &values, #id_name, #parent_name, #label_name,
                    ))
                })
        }
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_invocation_parses() {
        let ok = syn::parse_str::<TreeViewInput>(
            "mount: mount_relation_tree, entity: RelationRow, table: relation_table, \
             path: \"/relations/tree\", title: \"Related Parties / UBO\", \
             purpose: \"Operations\", access: requires role \"Analyst\", \
             id_field: relation_id, parent_field: parent_id, label_field: name",
        );
        assert!(ok.is_ok());
        let parsed = ok.unwrap();
        assert_eq!(parsed.id_field.to_string(), "relation_id");
    }

    #[test]
    fn access_defaults_to_public() {
        let ok = syn::parse_str::<TreeViewInput>(
            "mount: mount_tree, entity: Row, table: t, path: \"/t\", \
             title: \"Tree\", purpose: \"Operations\", \
             id_field: id, parent_field: parent, label_field: name",
        );
        assert!(ok.is_ok());
    }

    #[test]
    fn unknown_key_is_a_parse_error() {
        let bad = syn::parse_str::<TreeViewInput>(
            "mount: mount_tree, entity: R, table: t, path: \"/t\", \
             title: \"T\", purpose: \"Operations\", \
             id_field: id, parent_field: parent, label_field: name, columns: [x]",
        );
        assert!(bad.is_err());
    }

    #[test]
    fn missing_fields_are_required() {
        let bad = syn::parse_str::<TreeViewInput>(
            "mount: mount_tree, entity: R, table: t, path: \"/t\", \
             title: \"Tree\", purpose: \"Operations\"",
        );
        assert!(bad.is_err(), "id_field/parent_field/label_field are required");
    }
}