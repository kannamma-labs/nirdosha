//! Runtime support for `nirdosha_rt::communication_feed!` (RFC 0009
//! Track C's Communication archetype).
//!
//! **This is honest client-side polling, not real server push.** True
//! push needs `Router` to hold connections open (`Arc<Mutex<..>>` +
//! thread-per-connection via `nirdosha_rt::prelude::spawn`/`join`,
//! replacing the current `Rc<RefCell<..>>`, one-connection-at-a-time
//! model) — a genuine concurrency-model change to already-shipped code,
//! not a new feature layered on top. Until that's decided and done,
//! `communication_feed!` reuses `dashboard!`'s own `<meta
//! http-equiv="refresh">` mechanism: the browser reloads the whole page
//! on a timer, same as any dashboard. It *feels* live at human
//! timescales; it is not a persistent connection, and this module's
//! name says "feed," not "socket" or "push," on purpose.

use crate::screens::{value_display, FieldSpec};
use crate::web::{html_escape, page_shell};

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
