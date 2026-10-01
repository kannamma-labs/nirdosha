//! Runtime support for `nirdosha_rt::wizard!` (RFC 0009 Track C's
//! Workflow/Wizard archetype): a multi-step form whose in-progress
//! answers live server-side, keyed by an opaque per-wizard-run cookie
//! — no client-side JS needed, each step is a plain browser `<form>`
//! POST that redirects to the next step.

use crate::screens::FieldSpec;
use crate::web::{html_escape, page_shell};
use std::collections::HashMap;

/// A demo-grade, unique-but-not-cryptographically-secure id for one
/// in-progress wizard run — same honesty as `web::generate_session_id`,
/// which this deliberately does not share (a wizard run is not a login
/// session; conflating the two would let an unrelated login act as
/// wizard state, or vice versa).
pub fn generate_wizard_run_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{nanos:x}-{count:x}-{:p}", &count)
}

/// `GET <path>/step/<n>` — the form for one step, with a "Step n of
/// total" indicator so the user always knows where they are in the
/// flow (RFC 0009 Track C's Wizard archetype names this explicitly).
pub fn wizard_step_html(
    title: &str, step_no: usize, total_steps: usize, action: &str,
    fields: &[FieldSpec], values: &HashMap<String, String>, errors: &[String],
) -> String {
    let error_block = if errors.is_empty() {
        String::new()
    } else {
        let items: String = errors.iter().map(|e| format!("<li>{}</li>", html_escape(e))).collect();
        format!("<ul class=\"errors\">{items}</ul>")
    };
    let mut inputs = String::new();
    for f in fields {
        let value = values.get(f.name).cloned().unwrap_or_default();
        if f.input_type == "checkbox" {
            let checked = if value == "true" || value == "on" { " checked" } else { "" };
            inputs.push_str(&format!(
                "<p><label>{}</label><input type=\"checkbox\" name=\"{}\"{checked}></p>",
                html_escape(f.name), f.name,
            ));
        } else {
            inputs.push_str(&format!(
                "<p><label>{}</label><input type=\"{}\" name=\"{}\" value=\"{}\"></p>",
                html_escape(f.name), f.input_type, f.name, html_escape(&value),
            ));
        }
    }
    let next_label = if step_no == total_steps { "Finish" } else { "Next" };
    page_shell(
        title,
        "",
        &format!(
            "<p>Step {step_no} of {total_steps}</p>{error_block}\
             <form method=\"post\" action=\"{action}\">{inputs}<p><button type=\"submit\">{next_label}</button></p></form>"
        ),
    )
}

/// `GET <path>/done/<id>` — the completed record, read back from
/// wherever the final step stored it.
pub fn wizard_done_html(title: &str, fields: &[FieldSpec], row: &serde_json::Value) -> String {
    let mut rows = String::new();
    for f in fields {
        rows.push_str(&format!(
            "<p><label>{}</label> {}</p>",
            html_escape(f.name),
            html_escape(&crate::screens::value_display(&row[f.name])),
        ));
    }
    page_shell(title, "", &format!("<p>Done.</p>{rows}"))
}
