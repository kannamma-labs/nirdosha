//! Shared rendering for `approval_inbox!` (T-04) -- a real cross-entity
//! worklist over pending `approval_chain!` escalations. Mirrors
//! `board::Card`'s own split: this crate can't depend on
//! `nirdosha-guard-screens` (circular -- that crate depends on this one
//! for `Auth`), so `InboxRow` is a plain, decoupled shape the macro's
//! generated code (which runs in the app crate, where both crates are
//! linked) maps `nirdosha_guard_screens::PendingApprovalRow` into, the
//! same way `kanban_board!`'s generated code maps a `GuardedEntity` into
//! `board::Card`.
//!
//! The actual approve/confirm mutation always stays on the entity's own
//! screen route -- it needs the domain-specific field values the
//! original proposal already fixed (`GuardedTable::
//! guarded_confirm_escalated_update`'s own doc comment: the caller
//! re-derives the identical mutation), which a generic cross-entity list
//! has no way to know. This view only ever links out to that route
//! (`detail_url`). Return-with-reason needs no domain fields at all, so
//! it's a real, generic, inline form here.

use crate::web::{html_escape, page_shell};

#[derive(Debug, Clone)]
pub struct InboxRow {
    pub escalation_id: String,
    pub chain: String,
    pub resource: String,
    pub proposer: String,
    pub opened_at: u64,
    pub deadline: u64,
    pub quorum: u8,
    pub approvals_so_far: u8,
    /// One of "pending" | "cooling" | "approved" | "denied_timeout" | "returned".
    pub status: String,
    pub cooling_ready_at: Option<u64>,
    pub return_reason: Option<String>,
    /// Where "Review & approve" sends the approver -- the entity's own
    /// screen, which has the real form for the domain-specific fields
    /// the original proposal fixed.
    pub detail_url: String,
    /// Where the generic inline "Return" form POSTs -- this view's own
    /// `{path}/{resource}/{escalation_id}/return` route.
    pub return_url: String,
}

pub fn approval_inbox_html(title: &str, rows: &[InboxRow]) -> String {
    let mut body = String::new();
    body.push_str(&format!("<h1>{}</h1>\n", html_escape(title)));
    if rows.is_empty() {
        body.push_str("<p class=\"approval-inbox-empty\">Nothing pending.</p>\n");
    } else {
        body.push_str("<table class=\"approval-inbox\">\n<thead><tr><th>Resource</th><th>Proposer</th><th>Opened</th><th>Deadline</th><th>Quorum</th><th>Status</th><th>Actions</th></tr></thead>\n<tbody>\n");
        for row in rows {
            let status_label = match row.status.as_str() {
                "cooling" => format!("cooling (live at {})", row.cooling_ready_at.unwrap_or(0)),
                "returned" => format!("returned: {}", row.return_reason.clone().unwrap_or_default()),
                other => other.to_string(),
            };
            body.push_str("<tr data-escalation-id=\"");
            body.push_str(&html_escape(&row.escalation_id));
            body.push_str("\">\n");
            body.push_str(&format!("<td>{} ({})</td>\n", html_escape(&row.resource), html_escape(&row.chain)));
            body.push_str(&format!("<td>{}</td>\n", html_escape(&row.proposer)));
            body.push_str(&format!("<td>{}</td>\n", row.opened_at));
            body.push_str(&format!("<td>{}</td>\n", row.deadline));
            body.push_str(&format!("<td>{}/{}</td>\n", row.approvals_so_far, row.quorum));
            body.push_str(&format!("<td data-status=\"{}\">{}</td>\n", html_escape(&row.status), html_escape(&status_label)));
            body.push_str("<td>\n");
            if row.status == "pending" || row.status == "cooling" {
                body.push_str(&format!("<a href=\"{}\">Review &amp; approve</a>\n", html_escape(&row.detail_url)));
                body.push_str(&format!(
                    "<form method=\"post\" action=\"{}\" class=\"approval-inbox-return\"><input type=\"text\" name=\"reason\" placeholder=\"Return reason (required)\" required><button type=\"submit\">Return</button></form>\n",
                    html_escape(&row.return_url)
                ));
            }
            body.push_str("</td>\n</tr>\n");
        }
        body.push_str("</tbody>\n</table>\n");
    }
    page_shell(title, "", &body)
}
