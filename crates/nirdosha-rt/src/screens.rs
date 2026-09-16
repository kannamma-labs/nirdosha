//! Runtime support for `nirdosha_rt::crud_screens!` (RFC 0009 Track C):
//! generic List/Detail/Create-Form/Edit-Form/Delete-confirmation
//! rendering, attached to whatever datasource the macro is given. No
//! client-side JS — every form is a plain browser `<form>` POST.

use crate::web::{html_escape, page_shell};
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};

/// A process-global, monotonically increasing id — the same "atomic
/// counter, not a doc-comment convention" shape
/// `nirdosha_v2::new_instance_id` already uses in the corpus, made
/// generic so `crud_screens!` doesn't depend on any one crate's fixture
/// layer.
pub fn next_id() -> i64 {
    static COUNTER: AtomicI64 = AtomicI64::new(0);
    1 + COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Lets `crud_screens!` parse a submitted form/JSON value into any of
/// this dialect's ordinary field types generically — the macro just
/// emits `<#field_ty as ParseField>::parse_field(..)`; a field type
/// with no impl here is an ordinary `rustc` "trait bound not satisfied"
/// error, not a runtime surprise.
pub trait ParseField: Sized {
    fn parse_field(raw: Option<&str>) -> Result<Self, String>;
}

impl ParseField for String {
    fn parse_field(raw: Option<&str>) -> Result<Self, String> {
        Ok(raw.unwrap_or_default().to_string())
    }
}

impl ParseField for i64 {
    fn parse_field(raw: Option<&str>) -> Result<Self, String> {
        raw.unwrap_or_default().trim().parse::<i64>().map_err(|_| "must be a whole number".to_string())
    }
}

impl ParseField for f64 {
    fn parse_field(raw: Option<&str>) -> Result<Self, String> {
        raw.unwrap_or_default().trim().parse::<f64>().map_err(|_| "must be a number".to_string())
    }
}

impl ParseField for bool {
    fn parse_field(raw: Option<&str>) -> Result<Self, String> {
        // HTML checkboxes send "on" when checked and are simply absent
        // (never "false") when unchecked; the JSON API sends "true"/"false".
        Ok(matches!(raw, Some("true") | Some("on") | Some("1")))
    }
}

/// One column/field, as the macro knows it: its name and an HTML
/// `<input type>` hint derived from its Rust type.
#[derive(Clone, Copy)]
pub struct FieldSpec {
    pub name: &'static str,
    pub input_type: &'static str,
}

pub fn value_display(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Converts a serialized entity back into the same key→string shape
/// `Request::form_or_json` produces — used to pre-fill an edit form
/// with the entity's current values.
pub fn row_to_form_values(fields: &[FieldSpec], row: &serde_json::Value) -> HashMap<String, String> {
    fields.iter().map(|f| (f.name.to_string(), value_display(&row[f.name]))).collect()
}

/// `GET <path>` — a table of every row, or an empty-state message with
/// a "Create one" link if the datasource has nothing yet (System
/// State screens, RFC 0009 Track C's archetype 9).
pub fn list_html(
    title: &str,
    base_path: &str,
    fields: &[FieldSpec],
    rows: &[serde_json::Value],
    can_create: bool,
    search: Option<&str>,
) -> String {
    let search_box = format!(
        "<form method=\"get\" action=\"{base_path}\" style=\"margin-bottom:1rem\">\
         <input type=\"text\" name=\"q\" value=\"{}\" placeholder=\"Search {}...\">\
         <button type=\"submit\">Search</button>{}</form>",
        html_escape(search.unwrap_or_default()),
        html_escape(title),
        if search.is_some_and(|s| !s.is_empty()) {
            format!(" <a href=\"{base_path}\">clear</a>")
        } else {
            String::new()
        },
    );
    if rows.is_empty() {
        let cta = if can_create {
            format!(" <a href=\"{base_path}/new\">Create one</a>.")
        } else {
            String::new()
        };
        let empty_msg = if search.is_some_and(|s| !s.is_empty()) {
            "No matching results.".to_string()
        } else {
            format!("No {} yet.{cta}", html_escape(title))
        };
        return page_shell(title, "", &format!("{search_box}<p class=\"empty\">{empty_msg}</p>"));
    }
    let mut table = String::from("<table><thead><tr>");
    for f in fields {
        table.push_str(&format!("<th>{}</th>", html_escape(f.name)));
    }
    table.push_str("<th></th></tr></thead><tbody>");
    for row in rows {
        table.push_str("<tr>");
        for f in fields {
            table.push_str(&format!("<td>{}</td>", html_escape(&value_display(&row[f.name]))));
        }
        let id = row["id"].as_i64().unwrap_or_default();
        table.push_str(&format!("<td><a href=\"{base_path}/{id}\">view</a></td>"));
        table.push_str("</tr>");
    }
    table.push_str("</tbody></table>");
    let new_link = if can_create { format!("<p><a href=\"{base_path}/new\">+ New</a></p>") } else { String::new() };
    let export_link = format!("<p><a href=\"{base_path}/export.csv\">Export CSV</a></p>");
    page_shell(title, "", &format!("{search_box}{new_link}{table}{export_link}"))
}

/// Escapes one CSV field per RFC 4180: wrap in quotes (doubling any
/// embedded quote) whenever the value contains a comma, quote, or
/// newline — left bare otherwise, matching how every spreadsheet
/// import expects an unambiguous field to look.
pub fn csv_escape(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

/// `GET <path>/{id}` — every field as a read row, plus Edit/Delete
/// links when the caller may perform them.
pub fn detail_html(title: &str, base_path: &str, id: i64, fields: &[FieldSpec], row: &serde_json::Value, can_edit: bool, can_delete: bool) -> String {
    let mut rows = String::new();
    for f in fields {
        rows.push_str(&format!("<p><label>{}</label> {}</p>", html_escape(f.name), html_escape(&value_display(&row[f.name]))));
    }
    let mut actions = String::new();
    if can_edit {
        actions.push_str(&format!("<a href=\"{base_path}/{id}/edit\">edit</a> "));
    }
    if can_delete {
        actions.push_str(&format!("<a href=\"{base_path}/{id}/delete\" class=\"danger\">delete</a>"));
    }
    page_shell(title, "", &format!("{rows}<p>{actions}</p><p><a href=\"{base_path}\">back to list</a></p>"))
}

/// `GET <path>` for a `settings_screen!` — like `detail_html` but for a
/// singleton record with no `id`, so the edit link is `edit_path`
/// verbatim rather than `{base_path}/{id}/edit`.
pub fn settings_view_html(title: &str, edit_path: &str, fields: &[FieldSpec], row: &serde_json::Value, can_edit: bool) -> String {
    let mut rows = String::new();
    for f in fields {
        rows.push_str(&format!("<p><label>{}</label> {}</p>", html_escape(f.name), html_escape(&value_display(&row[f.name]))));
    }
    let actions = if can_edit { format!("<a href=\"{edit_path}\">edit</a>") } else { String::new() };
    page_shell(title, "", &format!("{rows}<p>{actions}</p>"))
}

/// `GET <path>/new` or `GET <path>/{id}/edit` — a plain `<form>`
/// (browser-default, `application/x-www-form-urlencoded`), pre-filled
/// with `existing` values on edit, with any `errors` from a failed
/// prior submission shown inline (the actual submitted values, not the
/// stored ones, so a rejected submission doesn't make the user retype
/// everything — the "inline validation" interaction named in RFC 0009
/// Track C's Form archetype).
pub fn form_html(title: &str, action: &str, fields: &[FieldSpec], values: &HashMap<String, String>, errors: &[String]) -> String {
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
    page_shell(
        title,
        "",
        &format!("{error_block}<form method=\"post\" action=\"{action}\">{inputs}<p><button type=\"submit\">Save</button></p></form>"),
    )
}

/// `GET <path>/{id}/delete` — a typed-confirmation page (RFC 0009
/// Track C's Delete/destructive-flow archetype: "type X to confirm",
/// not a bare button), since a plain HTML form has no other way to
/// guard a destructive POST.
pub fn delete_confirm_html(title: &str, action: &str, confirm_word: &str, label: &str) -> String {
    page_shell(
        title,
        "",
        &format!(
            "<p>Delete <strong>{}</strong>? This cannot be undone.</p>\
             <form method=\"post\" action=\"{action}\">\
             <p><label>Type {confirm_word} to confirm</label><input type=\"text\" name=\"confirm\"></p>\
             <p><button type=\"submit\" class=\"danger\">Delete</button> <a href=\"{action}\">cancel</a></p>\
             </form>",
            html_escape(label),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_increasing() {
        let a = next_id();
        let b = next_id();
        assert!(b > a);
    }

    #[test]
    fn parse_field_bool_accepts_html_checkbox_and_json_conventions() {
        assert_eq!(bool::parse_field(Some("on")), Ok(true));
        assert_eq!(bool::parse_field(Some("true")), Ok(true));
        assert_eq!(bool::parse_field(None), Ok(false));
        assert_eq!(bool::parse_field(Some("off")), Ok(false));
    }

    #[test]
    fn parse_field_i64_rejects_garbage() {
        assert!(i64::parse_field(Some("42")).is_ok());
        assert!(i64::parse_field(Some("nope")).is_err());
        assert!(i64::parse_field(None).is_err());
    }

    #[test]
    fn list_html_shows_empty_state_with_create_link() {
        let html = list_html("Products", "/products", &[], &[], true, None);
        assert!(html.contains("No Products yet"));
        assert!(html.contains("/products/new"));
    }

    #[test]
    fn list_html_renders_rows_and_view_links() {
        let fields = [FieldSpec { name: "name", input_type: "text" }];
        let rows = vec![serde_json::json!({"id": 1, "name": "Widget"})];
        let html = list_html("Products", "/products", &fields, &rows, false, None);
        assert!(html.contains("Widget"));
        assert!(html.contains("/products/1"));
        assert!(!html.contains("/products/new"));
    }

    #[test]
    fn list_html_shows_no_matching_results_for_an_empty_search() {
        let html = list_html("Products", "/products", &[], &[], true, Some("xyz"));
        assert!(html.contains("No matching results"));
        assert!(html.contains("value=\"xyz\""));
    }

    #[test]
    fn csv_escape_quotes_only_when_needed() {
        assert_eq!(csv_escape("plain"), "plain");
        assert_eq!(csv_escape("has,comma"), "\"has,comma\"");
        assert_eq!(csv_escape("has\"quote"), "\"has\"\"quote\"");
    }

    #[test]
    fn form_html_escapes_hostile_field_values() {
        let fields = [FieldSpec { name: "name", input_type: "text" }];
        let mut values = HashMap::new();
        values.insert("name".to_string(), "<script>evil()</script>".to_string());
        let html = form_html("New Product", "/products", &fields, &values, &[]);
        assert!(!html.contains("<script>evil"));
        assert!(html.contains("&lt;script&gt;"));
    }
}
