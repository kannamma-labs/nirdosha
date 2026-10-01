//! `workspace! { .. }` — T-06's archetype: a composite subject screen
//! (Investigation Workspace, Customer 360, ...) whose panels are each a
//! declared "context need" resolving to one real, independently-capped
//! guarded sub-read — never a bespoke per-screen join. Reuses
//! `nirdosha_rt::workspace::assemble_context` for the real parallel-run,
//! shared-budget, fail-whole-not-partial semantics (see that module's
//! own doc comment); this macro only wires the declared subject table
//! and panel sources through to it, same "macro wires, app crate
//! provides the guarded logic" convention `approval_inbox!`'s
//! `sources:` already established.
//!
//! Always routes via `get_with_auth` (visible to everyone, real
//! authorization deferred to the subject table's own `guard_policy!`
//! evaluation) — the same posture `crud_screens!`'s `guard:` clause
//! already uses, not a second, redundant compile-time role gate.
//!
//! ```ignore
//! nirdosha_rt::workspace! {
//!     mount: mount_investigation_workspace,
//!     path: "/cases/{id}/workspace",
//!     subject: { table: case_table, purpose: "AmlInvestigation", label_fn: case_workspace_header },
//!     budget: { max_rows: 500, max_execution_ms: 2000 },
//!     panels: [
//!         { need: "related_alerts", title: "Related Alerts", render: "table", source: related_alerts_for_case },
//!         { need: "case_transactions", title: "Transactions In Scope", render: "table", source: case_transactions_for_case },
//!         { need: "audit_timeline", title: "Audit Timeline", render: "timeline", source: case_audit_timeline },
//!     ],
//! }
//! ```

use crate::util::expect_keyword;
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{Ident, LitInt, LitStr, Token};

struct Subject {
    table: Ident,
    purpose: LitStr,
    label_fn: Ident,
}

impl Parse for Subject {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let content;
        syn::braced!(content in input);
        expect_keyword(&content, "table")?;
        content.parse::<Token![:]>()?;
        let table: Ident = content.parse()?;
        content.parse::<Token![,]>()?;
        expect_keyword(&content, "purpose")?;
        content.parse::<Token![:]>()?;
        let purpose: LitStr = content.parse()?;
        content.parse::<Token![,]>()?;
        expect_keyword(&content, "label_fn")?;
        content.parse::<Token![:]>()?;
        let label_fn: Ident = content.parse()?;
        let _ = content.parse::<Token![,]>();
        Ok(Subject { table, purpose, label_fn })
    }
}

struct Budget {
    max_rows: LitInt,
    max_execution_ms: LitInt,
}

impl Parse for Budget {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let content;
        syn::braced!(content in input);
        expect_keyword(&content, "max_rows")?;
        content.parse::<Token![:]>()?;
        let max_rows: LitInt = content.parse()?;
        content.parse::<Token![,]>()?;
        expect_keyword(&content, "max_execution_ms")?;
        content.parse::<Token![:]>()?;
        let max_execution_ms: LitInt = content.parse()?;
        let _ = content.parse::<Token![,]>();
        Ok(Budget { max_rows, max_execution_ms })
    }
}

/// One declared context need. `source` must be a real fn
/// `fn(&nirdosha_rt::Auth, &str) -> Result<Vec<serde_json::Value>, String>`
/// (auth, subject_id) — the screen module's own guarded read, never
/// data this macro invents.
struct Panel {
    need: LitStr,
    title: LitStr,
    render: LitStr,
    source: Ident,
}

impl Parse for Panel {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let content;
        syn::braced!(content in input);
        expect_keyword(&content, "need")?;
        content.parse::<Token![:]>()?;
        let need: LitStr = content.parse()?;
        content.parse::<Token![,]>()?;
        expect_keyword(&content, "title")?;
        content.parse::<Token![:]>()?;
        let title: LitStr = content.parse()?;
        content.parse::<Token![,]>()?;
        expect_keyword(&content, "render")?;
        content.parse::<Token![:]>()?;
        let render: LitStr = content.parse()?;
        content.parse::<Token![,]>()?;
        expect_keyword(&content, "source")?;
        content.parse::<Token![:]>()?;
        let source: Ident = content.parse()?;
        let _ = content.parse::<Token![,]>();
        Ok(Panel { need, title, render, source })
    }
}

struct WorkspaceInput {
    mount: Ident,
    path: LitStr,
    subject: Subject,
    budget: Budget,
    panels: Vec<Panel>,
}

impl Parse for WorkspaceInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        expect_keyword(input, "mount")?;
        input.parse::<Token![:]>()?;
        let mount: Ident = input.parse()?;
        input.parse::<Token![,]>()?;

        expect_keyword(input, "path")?;
        input.parse::<Token![:]>()?;
        let path: LitStr = input.parse()?;
        input.parse::<Token![,]>()?;

        expect_keyword(input, "subject")?;
        input.parse::<Token![:]>()?;
        let subject: Subject = input.parse()?;
        input.parse::<Token![,]>()?;

        expect_keyword(input, "budget")?;
        input.parse::<Token![:]>()?;
        let budget: Budget = input.parse()?;
        input.parse::<Token![,]>()?;

        expect_keyword(input, "panels")?;
        input.parse::<Token![:]>()?;
        let bracketed;
        syn::bracketed!(bracketed in input);
        let mut panels = Vec::new();
        while !bracketed.is_empty() {
            panels.push(bracketed.parse::<Panel>()?);
            if bracketed.peek(Token![,]) {
                bracketed.parse::<Token![,]>()?;
            }
        }
        if panels.is_empty() {
            return Err(syn::Error::new(bracketed.span(), "workspace! needs at least one panel"));
        }
        let _ = input.parse::<Token![,]>();

        Ok(WorkspaceInput { mount, path, subject, budget, panels })
    }
}

pub fn expand(input: TokenStream) -> TokenStream {
    let parsed = match syn::parse::<WorkspaceInput>(input) {
        Ok(p) => p,
        Err(e) => return e.to_compile_error().into(),
    };
    expand_parsed(parsed).into()
}

fn expand_parsed(input: WorkspaceInput) -> TokenStream2 {
    let mount = &input.mount;
    let path_str = input.path.value();
    let path = quote! { #path_str };
    let title = path_str.trim_start_matches('/').to_string();

    let subject_table = &input.subject.table;
    let subject_purpose = &input.subject.purpose;
    let subject_label_fn = &input.subject.label_fn;

    let max_rows = &input.budget.max_rows;
    let max_execution_ms = &input.budget.max_execution_ms;

    let need_exprs = input.panels.iter().map(|p| {
        let need = &p.need;
        let panel_title = &p.title;
        let render = &p.render;
        let source = &p.source;
        quote! {
            {
                let __auth = __auth.clone();
                let __sid = __subject_id.clone();
                ::nirdosha_rt::workspace::ContextNeed {
                    need: #need,
                    title: #panel_title,
                    render: #render,
                    source: ::std::boxed::Box::new(move || #source(&__auth, &__sid)),
                }
            }
        }
    });

    quote! {
        pub fn #mount(router: ::nirdosha_rt::Router) -> ::nirdosha_rt::Router {
            router.get_with_auth(#path, #title, |_req, params, __auth| {
                let Some(__subject_id) = params.get("id").map(|s| s.to_string()) else {
                    return ::nirdosha_rt::Response::bad_request("id required");
                };
                let subject_row = match #subject_table().guarded_get(__auth, #subject_purpose, &__subject_id) {
                    Ok(Some(row)) => row,
                    Ok(None) => return ::nirdosha_rt::Response::not_found(),
                    Err(e) => return ::nirdosha_guard_screens::guard_error_response(e),
                };
                let header = #subject_label_fn(&subject_row);
                let needs: ::std::vec::Vec<::nirdosha_rt::workspace::ContextNeed> = ::std::vec![ #(#need_exprs),* ];
                let budget = ::nirdosha_rt::workspace::WorkspaceBudget { max_rows: #max_rows, max_execution_ms: #max_execution_ms };
                match ::nirdosha_rt::workspace::assemble_context(needs, &budget) {
                    Ok(panels) => ::nirdosha_rt::Response::html(200, ::nirdosha_rt::workspace::workspace_html(#title, &header, &panels)),
                    Err(reason) => ::nirdosha_rt::Response::html(422, ::nirdosha_rt::workspace::workspace_context_failed_html(#title, &reason)),
                }
            })
        }
    }
}
