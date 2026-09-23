//! `approval_inbox! { .. }` — T-04's archetype: a real, cross-entity
//! worklist over pending `approval_chain!` escalations, merged from one
//! or more `GuardedTable`s that declare an `escalate to
//! approval(chain ...)` write. Reuses the real quorum/cooling/timeout
//! runtime (`nirdosha_guard_core::approval_chain::ApprovalChainRuntime`,
//! `GuardedTable::list_pending_approvals`) — this macro never tracks
//! escalation state of its own.
//!
//! Renders a merged `GET` list (`nirdosha_rt::approval_inbox::
//! approval_inbox_html`) plus one real, generic action this view itself
//! implements — return-with-reason, which needs no domain-specific
//! field values — and links each row's "Review & approve" out to the
//! entity's OWN already-real propose/confirm route (`detail_path`,
//! `{id}` substituted with the row id `approve()`d escalation ids embed
//! as their `{RESOURCE}:{row_id}:{opened_at}` second segment): that
//! route is what re-derives the exact mutation the original proposal
//! fixed (`GuardedTable::guarded_confirm_escalated_update`'s own doc
//! comment) — a cross-entity list has no way to know those fields
//! generically, so this macro never invents a one-click "Approve"
//! button that would blind-guess them.
//!
//! Guard mode (optional): when **every** `source` declares `purpose:`,
//! the guard is the single authority for who may read each source's
//! escalation worklist — the view route is `get_with_auth` and rows
//! come from `GuardedTable::guarded_list_pending_approvals(auth,
//! purpose)`, the same `read` evaluation `guarded_snapshot` runs (a
//! deny from any source fails the whole view, the same
//! fail-whole-not-partial posture `workspace!` uses). A declared
//! `access:` becomes vestigial route metadata, never a second check.
//! Mixed inboxes — some sources guarded, some not — are refused at
//! expansion: a half-guarded worklist would show rows the guard denies
//! reading.
//!
//! ```ignore
//! nirdosha_rt::approval_inbox! {
//!     mount: mount_case_review_inbox,
//!     path: "/four-eyes",
//!     access: requires role "ComplianceLead",
//!     sources: [
//!         { table: case_table, chain: "case_review", resource: "case", detail_path: "/cases/{id}", purpose: "Operations" },
//!     ],
//! }
//! ```

use crate::util::expect_keyword;
use nirdosha_contract_core::role::role_ident;
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{Ident, LitStr, Token};

enum Access {
    Public,
    Role(Vec<LitStr>),
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
            let mut roles = Vec::new();
            let role: LitStr = input.parse()?;
            if let Err(e) = role_ident(&role.value(), role.span()) {
                return Err(e);
            }
            roles.push(role);
            // Additional roles: `requires role "X" or role "Y" or role "Z"`
            // — an inbox that fans out to several module-scoped inboxes
            // (T-04's own doc comment) is legitimately shared by more than
            // one approving role (e.g. ComplianceLead + Mlro on the unified
            // `/approvals` view), so a single hardcoded role is too narrow.
            while input.peek(syn::Ident) {
                let or_kw = Ident::parse_any(input)?;
                if or_kw != "or" {
                    return Err(syn::Error::new(or_kw.span(), "expected `or role \"...\"`"));
                }
                let role_kw = Ident::parse_any(input)?;
                if role_kw != "role" {
                    return Err(syn::Error::new(role_kw.span(), "expected `role`"));
                }
                let role: LitStr = input.parse()?;
                if let Err(e) = role_ident(&role.value(), role.span()) {
                    return Err(e);
                }
                roles.push(role);
            }
            return Ok(Access::Role(roles));
        }
        Err(syn::Error::new(ident.span(), "expected `public` or `requires role \"...\"` (optionally `or role \"...\"`)"))
    }
}

/// One `GuardedTable` this inbox reads pending escalations from.
/// `resource`/`chain` are declared explicitly (not derived from the
/// table's own `GuardedEntity::RESOURCE`/its corpus `escalate to
/// approval(chain ...)` clause) because a proc-macro over `table`'s bare
/// identifier has no access to either at expansion time — same
/// hand-declared convention `kanban_board!`'s own `columns: [...]` uses
/// for data it could, in principle, read from elsewhere but doesn't.
struct Source {
    table: Ident,
    chain: LitStr,
    resource: LitStr,
    detail_path: LitStr,
    /// The source table's guard purpose. Present on **every** source or
    /// **none** (mixed is a named parse error): a purpose on a source is
    /// the declaration that this inbox's view of that table's
    /// escalations is guard-gated — `guarded_list_pending_approvals`
    /// evaluates the same `read` decision `guarded_snapshot` runs, so a
    /// viewer the guard denies a plain read sees no worklist. An inbox
    /// that is half-guarded would be two inboxes wearing one route.
    purpose: Option<LitStr>,
}

impl Parse for Source {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let content;
        syn::braced!(content in input);
        expect_keyword(&content, "table")?;
        content.parse::<Token![:]>()?;
        let table: Ident = content.parse()?;
        content.parse::<Token![,]>()?;
        expect_keyword(&content, "chain")?;
        content.parse::<Token![:]>()?;
        let chain: LitStr = content.parse()?;
        content.parse::<Token![,]>()?;
        expect_keyword(&content, "resource")?;
        content.parse::<Token![:]>()?;
        let resource: LitStr = content.parse()?;
        content.parse::<Token![,]>()?;
        expect_keyword(&content, "detail_path")?;
        content.parse::<Token![:]>()?;
        let detail_path: LitStr = content.parse()?;
        let _ = content.parse::<Token![,]>();
        // Optional trailing `purpose:` — the guard-mode marker. Absent
        // on every source = the pre-guard-mode shape (fixtures).
        let mut purpose = None;
        {
            use syn::ext::IdentExt;
            let fork = content.fork();
            if let Ok(kw) = syn::Ident::parse_any(&fork) {
                if kw == "purpose" {
                    expect_keyword(&content, "purpose")?;
                    content.parse::<Token![:]>()?;
                    purpose = Some(content.parse()?);
                    let _ = content.parse::<Token![,]>();
                }
            }
        }
        Ok(Source { table, chain, resource, detail_path, purpose })
    }
}

struct ApprovalInboxInput {
    mount: Ident,
    path: LitStr,
    access: Access,
    sources: Vec<Source>,
}

impl Parse for ApprovalInboxInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        expect_keyword(input, "mount")?;
        input.parse::<Token![:]>()?;
        let mount: Ident = input.parse()?;
        input.parse::<Token![,]>()?;

        expect_keyword(input, "path")?;
        input.parse::<Token![:]>()?;
        let path: LitStr = input.parse()?;
        input.parse::<Token![,]>()?;

        expect_keyword(input, "access")?;
        input.parse::<Token![:]>()?;
        let access: Access = input.parse()?;
        input.parse::<Token![,]>()?;

        expect_keyword(input, "sources")?;
        input.parse::<Token![:]>()?;
        let bracketed;
        syn::bracketed!(bracketed in input);
        let mut sources = Vec::new();
        while !bracketed.is_empty() {
            sources.push(bracketed.parse::<Source>()?);
            if bracketed.peek(Token![,]) {
                bracketed.parse::<Token![,]>()?;
            }
        }
        if sources.is_empty() {
            return Err(syn::Error::new(bracketed.span(), "approval_inbox! needs at least one source"));
        }
        let _ = input.parse::<Token![,]>();

        Ok(ApprovalInboxInput { mount, path, access, sources })
    }
}

pub fn expand(input: TokenStream) -> TokenStream {
    let parsed = match syn::parse::<ApprovalInboxInput>(input) {
        Ok(p) => p,
        Err(e) => return e.to_compile_error().into(),
    };
    expand_parsed(parsed).into()
}

fn expand_parsed(input: ApprovalInboxInput) -> TokenStream2 {
    let mount = &input.mount;
    let path_str = input.path.value();
    let path = quote! { #path_str };
    let title = path_str.trim_start_matches('/').to_string();
    let return_path_str = format!("{}/{{resource}}/{{escalation_id}}/return", path_str.trim_end_matches('/'));
    let return_path = quote! { #return_path_str };

    // Guard mode: every source declares `purpose:` (mixed is refused).
    // The guard is then the single authority for who may read each
    // source's escalation worklist; `access:` becomes vestigial route
    // metadata (never a second check) — the same posture
    // `communication_feed!`'s guard clause already established.
    let any_purpose = input.sources.iter().any(|s| s.purpose.is_some());
    let guard_mode = input.sources.iter().all(|s| s.purpose.is_some());
    if any_purpose && !guard_mode {
        let missing: Vec<String> = input
            .sources
            .iter()
            .filter(|s| s.purpose.is_none())
            .map(|s| s.resource.value())
            .collect();
        return syn::Error::new(
            input.path.span(),
            format!(
                "approval_inbox!: source(s) [{}] declare no `purpose:` while others do — an inbox is guard-gated on every source or none; a half-guarded worklist would show rows the guard denies reading",
                missing.join(", ")
            ),
        )
        .to_compile_error();
    }

    // One `row_provider` arm per source: reads that table's pending
    // escalations — guard-gated (`guarded_list_pending_approvals`, a
    // deny from any source fails the whole view) in guard mode, the
    // pre-guard direct read otherwise — and maps `PendingApprovalRow`
    // -> the decoupled `nirdosha_rt::approval_inbox::InboxRow` (see that
    // module's own doc comment for why the mapping happens here, in the
    // app crate's context, and not inside `nirdosha-rt` itself). Keeps
    // only rows on the declared `chain` (a table may carry escalations
    // for chains this inbox doesn't own).
    let row_providers = input.sources.iter().map(|source| {
        let table = &source.table;
        let chain = &source.chain;
        let resource = &source.resource;
        let detail_path = &source.detail_path;
        // The row mapping is identical in both modes; only the fetch
        // differs — guard-gated (one `read` evaluation per source, a
        // deny fails the whole view) or the pre-guard direct read.
        let row_body = quote! {
            if __p.chain != #chain { continue; }
            let __row_id = __p.escalation_id.splitn(3, ':').nth(1).unwrap_or("").to_string();
            __rows.push(::nirdosha_rt::approval_inbox::InboxRow {
                escalation_id: __p.escalation_id.clone(),
                chain: __p.chain.clone(),
                resource: #resource.to_string(),
                proposer: __p.proposer.clone(),
                opened_at: __p.opened_at,
                deadline: __p.deadline,
                quorum: __p.quorum,
                approvals_so_far: __p.approvals_so_far,
                status: __p.status.clone(),
                cooling_ready_at: __p.cooling_ready_at,
                return_reason: __p.return_reason.clone(),
                detail_url: #detail_path.replace("{id}", &__row_id),
                return_url: format!("{}/{}/{}/return", #path_str.trim_end_matches('/'), #resource, __p.escalation_id),
            });
        };
        match &source.purpose {
            Some(purpose) => quote! {
                let __source_rows = match #table().guarded_list_pending_approvals(auth, #purpose) {
                    Ok(rows) => rows,
                    Err(e) => return ::nirdosha_guard_screens::guard_error_response(e),
                };
                for __p in __source_rows {
                    #row_body
                }
            },
            None => quote! {
                for __p in #table().list_pending_approvals() {
                    #row_body
                }
            },
        }
    });

    // The return-with-reason dispatch: `resource` in the URL picks the
    // one source table this escalation can genuinely belong to (an
    // escalation id is only ever real in the table that opened it) —
    // real per-resource routing, not a guess-and-retry across sources.
    let return_arms = input.sources.iter().map(|source| {
        let table = &source.table;
        let chain = &source.chain;
        let resource_str = source.resource.value();
        quote! {
            #resource_str => #table().guarded_return_escalated(auth, #chain, &escalation_id, &reason, now_ms).map_err(::nirdosha_guard_screens::guard_error_response),
        }
    });

    let view_body = quote! {
        let mut __rows: Vec<::nirdosha_rt::approval_inbox::InboxRow> = Vec::new();
        #(#row_providers)*
        __rows.sort_by(|a, b| b.opened_at.cmp(&a.opened_at));
        ::nirdosha_rt::Response::html(200, ::nirdosha_rt::approval_inbox::approval_inbox_html(#title, &__rows))
    };

    let (view_route, role_assertions) = if guard_mode {
        // Guard mode: `*_with_auth` routes only. The guard decides at
        // the data plane (one `read` evaluation per source); `access:`
        // stays vestigial OpenAPI metadata, never a second check.
        (
            quote! {
                .get_with_auth(#path, #title, |_req, _params, auth| {
                    #view_body
                })
            },
            quote! {},
        )
    } else {
        match &input.access {
            Access::Public => (quote! { .get(#path, #title, |_req, _params| { #view_body }) }, quote! {}),
            Access::Role(roles) if roles.len() == 1 => {
                let role = &roles[0];
                let role_ident_tok = role_ident(&role.value(), role.span()).expect("role name already validated at parse time");
                (
                    quote! {
                        .get_gated::<crate::nirdosha_roles::#role_ident_tok>(#path, #title, |_req, _params, _proof| {
                            #view_body
                        })
                    },
                    quote! {},
                )
            }
            Access::Role(roles) => {
                // More than one role: `get_gated::<R>` only ever checks one
                // type parameter, so a real multi-role gate uses `auth.has_role`
                // directly. Each named role still gets the same dead-type-alias
                // existence assertion `app_shell_from_toml!`'s T-10 mechanism
                // uses, so an undeclared/typo'd role here is a named compile
                // error too, not a runtime surprise.
                let role_strs = roles.iter().map(|r| r.value());
                let assertions = roles.iter().map(|role| {
                    let role_ident_tok = role_ident(&role.value(), role.span()).expect("role name already validated at parse time");
                    let alias = quote::format_ident!("__AssertApprovalInboxRoleDeclared_{}", role_ident_tok);
                    quote! {
                        #[allow(dead_code, non_camel_case_types)]
                        type #alias = crate::nirdosha_roles::#role_ident_tok;
                    }
                });
                (
                    quote! {
                        .get_with_auth(#path, #title, |_req, _params, auth| {
                            if !(#(auth.has_role(#role_strs))||*) {
                                return ::nirdosha_rt::Response::forbidden();
                            }
                            #view_body
                        })
                    },
                    quote! { #(#assertions)* },
                )
            }
        }
    };

    quote! {
        #role_assertions

        pub fn #mount(router: ::nirdosha_rt::Router) -> ::nirdosha_rt::Router {
            router
                #view_route
                .post_with_auth(#return_path, "Return escalation with reason", |req, params, auth| {
                    let Some(resource) = params.get("resource").map(|s| s.to_string()) else { return ::nirdosha_rt::Response::bad_request("resource required") };
                    let Some(escalation_id) = params.get("escalation_id").map(|s| s.to_string()) else { return ::nirdosha_rt::Response::bad_request("escalation_id required") };
                    let values = req.form_or_json();
                    let reason = values.get("reason").cloned().unwrap_or_default();
                    let now_ms = (::nirdosha_rt::screens::now_epoch_secs() as u64) * 1000;
                    let result: Result<(), ::nirdosha_rt::Response> = match resource.as_str() {
                        #(#return_arms)*
                        other => Err(::nirdosha_rt::Response::bad_request(&format!("unknown resource `{other}` for this inbox"))),
                    };
                    match result {
                        Ok(()) => ::nirdosha_rt::Response::json(200, &::serde_json::json!({ "status": "returned" })),
                        Err(resp) => resp,
                    }
                })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> syn::Result<ApprovalInboxInput> {
        syn::parse_str(src)
    }

    #[test]
    fn guard_mode_routes_with_auth_and_fetches_via_the_guarded_primitive() {
        let input = parse(
            "mount: mount_x, path: \"/approvals\", access: requires role \"Admin\", \
             sources: [ { table: widget_table, chain: \"c\", resource: \"widget\", detail_path: \"/widgets/{id}\", purpose: \"Ops\" } ]"
        )
        .expect("a fully-guarded inbox must parse");
        let expanded = expand_parsed(input).to_string();
        assert!(expanded.contains("get_with_auth"), "guard-mode view must route via *_with_auth: {expanded}");
        assert!(expanded.contains("guarded_list_pending_approvals"), "guard-mode rows must come from the guarded primitive: {expanded}");
        assert!(expanded.contains("\"Ops\""), "per-source purpose must reach the fetch: {expanded}");
    }

    #[test]
    fn guard_mode_ignores_access_as_vestigial_metadata() {
        // `public` combined with a guarded source is fine: the guard
        // decides who reads at the data plane, so `access:` never
        // becomes a second gate.
        let input = parse(
            "mount: mount_x, path: \"/approvals\", access: public, \
             sources: [ { table: widget_table, chain: \"c\", resource: \"widget\", detail_path: \"/widgets/{id}\", purpose: \"Ops\" } ]"
        )
        .expect("public + guarded source must parse");
        let expanded = expand_parsed(input).to_string();
        assert!(expanded.contains("get_with_auth"), "even public access becomes *_with_auth in guard mode: {expanded}");
        assert!(!expanded.contains(".get("), "no bare public route in guard mode: {expanded}");
    }

    #[test]
    fn mixed_guarded_and_unguarded_sources_are_refused() {
        let input = parse(
            "mount: mount_x, path: \"/approvals\", access: public, \
             sources: [ { table: a_table, chain: \"c\", resource: \"a\", detail_path: \"/a/{id}\", purpose: \"Ops\" }, \
                         { table: b_table, chain: \"c\", resource: \"b\", detail_path: \"/b/{id}\" } ]"
        )
        .expect("mixed sources must still parse (the refusal is at expansion)");
        let expanded = expand_parsed(input).to_string();
        assert!(expanded.contains("half-guarded"), "a half-guarded inbox must be refused by name: {expanded}");
    }

    #[test]
    fn fully_unguarded_back_compat_shape_still_emits_the_pre_guard_read() {
        let input = parse(
            "mount: mount_x, path: \"/approvals\", access: requires role \"Admin\", \
             sources: [ { table: widget_table, chain: \"c\", resource: \"widget\", detail_path: \"/widgets/{id}\" } ]"
        )
        .expect("the pre-guard shape must still parse (fixtures)");
        let expanded = expand_parsed(input).to_string();
        assert!(expanded.contains("list_pending_approvals"), "unguarded sources keep the direct read: {expanded}");
        assert!(!expanded.contains("guarded_list_pending_approvals"), "no guarded fetch without purpose: {expanded}");
        assert!(expanded.contains("get_gated"), "single-role unguarded keeps its real role gate: {expanded}");
    }
}
