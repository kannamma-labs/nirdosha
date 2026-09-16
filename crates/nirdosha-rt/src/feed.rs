//! Runtime support for `nirdosha_rt::communication_feed!` (RFC 0009
//! Track C's Communication archetype).
//!
//! `refresh_seconds` retains timer-based page refresh. Opt-in
//! `long_poll_seconds` holds a JSON request until a generated POST publishes
//! a change or the bounded wait expires. Router workers keep other requests
//! moving. Notifications are process-local and cover generated POSTs only;
//! direct writes to the backing store must arrange their own notifications.

use crate::screens::{value_display, FieldSpec};
use crate::web::{html_escape, page_shell};

/// A process-local change counter. Wait releases the mutex, and checking the
/// counter under that same mutex prevents a publish-before-wait lost wakeup.
pub struct FeedUpdates {
    revision: std::sync::Mutex<u64>,
    changed: std::sync::Condvar,
}

impl Default for FeedUpdates {
    fn default() -> Self { Self::new() }
}

impl FeedUpdates {
    pub const fn new() -> Self {
        Self { revision: std::sync::Mutex::new(0), changed: std::sync::Condvar::new() }
    }

    pub fn revision(&self) -> u64 { *self.revision.lock().unwrap() }

    pub fn publish(&self) {
        let mut revision = self.revision.lock().unwrap();
        *revision = revision.wrapping_add(1);
        self.changed.notify_all();
    }

    /// Returns immediately for a stale cursor; otherwise waits at most timeout.
    pub fn wait(&self, since: u64, timeout: std::time::Duration) -> u64 {
        let (revision, _) = self.changed.wait_timeout_while(
            self.revision.lock().unwrap(), timeout, |revision| *revision == since,
        ).unwrap();
        *revision
    }
}

/// Adds a long-poll client. Only a changed revision reloads the page; errors
/// back off, and loss of read authorization stops further requests.
pub fn with_long_poll(html: String, api_path: &str, revision: u64) -> String {
    let path = serde_json::to_string(api_path).unwrap().replace('<', "\\u003c");
    let script = format!(r#"<script>(async()=>{{
const path={path}, revision="{revision}";
for(;;){{try{{
const r=await fetch(path+"?since="+revision,{{cache:"no-store"}});
if(r.status===401||r.status===403)return;
if(!r.ok)throw new Error("feed");
const next=r.headers.get("X-Nirdosha-Revision");
await r.text();
if(next!==null&&next!==revision){{location.reload();return;}}
}}catch(_){{await new Promise(resolve=>setTimeout(resolve,1000));}}
}}
}})();</script>"#);
    html.replacen("</body>", &format!("{script}</body>"), 1)
}

/// `GET <path>` — the most recent messages, newest first, plus a plain
/// `<form>` to post a new one (shown regardless of whether the viewer
/// can actually submit — the POST route enforces that; a hidden form
/// isn't a security control, it's just unhelpful UI when the button
/// would 403 anyway, so callers with anonymous posting disabled would
/// still see the form, matching every other screen's honesty about not
/// second-guessing the real gate with client-side hiding).
pub fn feed_html(title: &str, refresh_seconds: Option<u64>, action: &str, post_fields: &[FieldSpec], messages: &[serde_json::Value]) -> String {
    let refresh_meta = match refresh_seconds {
        Some(secs) => format!("<meta http-equiv=\"refresh\" content=\"{secs}\">"),
        None => String::new(),
    };
    let mut items = String::new();
    if messages.is_empty() {
        items.push_str("<p class=\"empty\">No messages yet</p>");
    } else {
        for m in messages {
            let mut fields = String::new();
            for f in post_fields {
                fields.push_str(&format!("<span>{}</span> ", html_escape(&value_display(&m[f.name]))));
            }
            items.push_str(&format!("<p class=\"feed-item\">{fields}</p>"));
        }
    }
    let mut inputs = String::new();
    for f in post_fields {
        inputs.push_str(&format!(
            "<input type=\"{}\" name=\"{}\" placeholder=\"{}\">",
            f.input_type, f.name, html_escape(f.name),
        ));
    }
    let body = format!(
        "<form method=\"post\" action=\"{action}\">{inputs}<button type=\"submit\">Post</button></form><div class=\"feed\">{items}</div>",
    );
    page_shell(title, &refresh_meta, &body)
}
