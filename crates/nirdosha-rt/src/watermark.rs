//! Watermark overlay for read-only restricted-access screens
//! (`screen.md` module 21's "Auditor Read-Only Portal"/"Regulator
//! Inspection View" convention: an always-on banner naming who is
//! viewing and when, so a screenshot or printout carries its own
//! provenance). Composed onto an already-rendered page the same way
//! `feed::with_long_poll` composes its poll script — a string-level
//! post-process, not a new `page_shell` parameter, so every existing
//! render helper's signature stays untouched.

use crate::web::html_escape;

/// A fixed, non-dismissible banner: "<LABEL> — <user> — <timestamp>".
/// `timestamp` is caller-supplied text, not parsed or validated here —
/// this crate has no date/time-formatting dependency (every other
/// timestamp in this codebase is a raw `now_ms` epoch value; callers
/// wanting a human string format it themselves before calling this).
pub fn watermark_banner(user: &str, label: &str, timestamp: &str) -> String {
    format!(
        "<div style=\"position:sticky;top:0;z-index:1000;background:#7a1f1f;color:#fff;\
         padding:0.5rem 1rem;font-weight:600;text-align:center;letter-spacing:0.02em\">\
         {} — {} — {}</div>",
        html_escape(label),
        html_escape(user),
        html_escape(timestamp),
    )
}

/// Injects a watermark banner immediately after `<body>` in an
/// already-rendered page (`page_shell`/`themed_page_shell_ex` output) —
/// same string-level composition `feed::with_long_poll` uses to inject
/// its poll `<script>`, chosen for the same reason: the banner is
/// orthogonal to what's being viewed, not something every render
/// helper's signature should have to thread through.
pub fn with_watermark(html: String, banner_html: &str) -> String {
    html.replacen("<body>", &format!("<body>{banner_html}"), 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banner_escapes_and_orders_label_user_timestamp() {
        let banner = watermark_banner("<script>alice</script>", "AUDIT READ-ONLY", "1700000000000");
        assert!(!banner.contains("<script>alice"));
        assert!(banner.contains("&lt;script&gt;alice"));
        assert!(banner.contains("AUDIT READ-ONLY"));
        assert!(banner.contains("1700000000000"));
    }

    #[test]
    fn with_watermark_injects_right_after_body_open() {
        let page = "<html><body><div>content</div></body></html>".to_string();
        let out = with_watermark(page, "<div>BANNER</div>");
        assert_eq!(out, "<html><body><div>BANNER</div><div>content</div></body></html>");
    }
}
