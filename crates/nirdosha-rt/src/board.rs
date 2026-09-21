//! Runtime support for `nirdosha_rt::kanban_board!` (RFC 0009 Track C's
//! Board/Canvas archetype): a presentational drag-and-drop board over
//! an existing categorical field. This is the one archetype that
//! genuinely needs client-side JS (a `<form>` POST can't express
//! "drag this card to another column"), so it's also this dialect's
//! client-side-interactivity foundation: one small, hand-written,
//! reviewable vanilla-JS asset, no bundler and no framework.
//!
//! `kanban_board!` never regenerates the move/mutation logic itself —
//! it calls back into a move endpoint the app already registered (the
//! same `#[contract(requires(role=...))]`-gated functions
//! `categorical_actions!` generates, wired to a route by hand, exactly
//! as the flagship demo does for approve/disapprove). The board is
//! presentation only: it reads the datasource to lay out cards, and
//! its JS just performs a `fetch()` POST to whatever move path the app
//! provides — the security boundary is unchanged and lives entirely in
//! that existing gated endpoint.

use crate::web::{html_escape, page_shell};
use std::collections::HashMap;

/// One card, as the macro's generated code already knows it: its id,
/// display title, and current column. `id` is `String`, not `i64` --
/// widened for `kanban_board!`'s `guard:` mode, whose rows are keyed by
/// `GuardedEntity::row_id()`'s real string identity (an `AlertId`/
/// `CaseId` newtype, not the `store:`/`SharedTable` path's synthetic
/// integer id). `data-id="{}"` (below) and `board_js`'s JS-side
/// `card.dataset.id` both already treat this as opaque text, so the
/// widening is source-compatible for every existing `i64`-id caller
/// via `.to_string()`.
pub struct Card {
    pub id: String,
    pub title: String,
    pub column: String,
}

/// `GET <path>` — one `<div class="kanban-column">` per declared
/// column, each holding its cards as `draggable="true"` elements. The
/// actual drag-and-drop wiring lives in `board_js`, loaded via a
/// `<script src="{asset_path}">` this function emits.
///
/// `allowed` (T-14, `kanban_board!`'s `machine:` clause) is the real
/// registered workflow's transition graph, keyed by from-state: when
/// present, each card renders its current column's allowed next columns
/// as `data-allowed-to`, and `board_js` grays out — and refuses to
/// accept a drop on — every column not in that list, pre-drop, instead
/// of accepting an illegal drag and failing only when the move endpoint
/// later rejects it. `None` (every pre-T-14 caller) renders exactly as
/// before: no attribute, no graying, every column a legal drop target.
pub fn board_html(title: &str, columns: &[&str], cards: &[Card], asset_path: &str, allowed: Option<&HashMap<String, Vec<String>>>) -> String {
    let mut board = String::new();
    for column in columns {
        let mut cards_html = String::new();
        for card in cards.iter().filter(|c| &c.column == column) {
            let allowed_attr = match allowed {
                Some(map) => {
                    let targets = map.get(&card.column).cloned().unwrap_or_default();
                    format!(" data-allowed-to=\"{}\"", html_escape(&targets.join(",")))
                }
                None => String::new(),
            };
            cards_html.push_str(&format!(
                "<div class=\"kanban-card\" draggable=\"true\" data-id=\"{}\"{}>{}</div>",
                card.id,
                allowed_attr,
                html_escape(&card.title),
            ));
        }
        board.push_str(&format!(
            "<div class=\"kanban-column\" data-column=\"{}\"><h3>{}</h3><div class=\"kanban-cards\">{}</div></div>",
            html_escape(column),
            html_escape(column),
            cards_html,
        ));
    }
    let style = if allowed.is_some() {
        "<style>.kanban-column--disallowed{opacity:.35;pointer-events:none;}</style>"
    } else {
        ""
    };
    let body = format!("{style}<div class=\"kanban-board\">{board}</div><script src=\"{asset_path}\"></script>");
    page_shell(title, "", &body)
}

/// The board's one JS asset. `move_path_template` is a URL containing
/// the literal placeholders `{id}` and `{to}`, substituted here at the
/// browser with the dragged card's id and the column it was dropped
/// on, then POSTed with no body — the move endpoint itself (an
/// existing gated route the app supplies) decides whether the request
/// is authorized and what the move actually means.
pub fn board_js(move_path_template: &str) -> String {
    format!(
        r#"(function() {{
  var template = {template:?};
  var columns = document.querySelectorAll('.kanban-column');
  document.querySelectorAll('.kanban-card').forEach(function(card) {{
    card.addEventListener('dragstart', function(e) {{
      e.dataTransfer.setData('text/plain', card.dataset.id);
      var allowedAttr = card.dataset.allowedTo;
      if (allowedAttr !== undefined) {{
        var allowed = allowedAttr.split(',').filter(Boolean);
        columns.forEach(function(col) {{
          if (allowed.indexOf(col.dataset.column) === -1) {{
            col.classList.add('kanban-column--disallowed');
          }}
        }});
      }}
    }});
    card.addEventListener('dragend', function() {{
      columns.forEach(function(col) {{ col.classList.remove('kanban-column--disallowed'); }});
    }});
  }});
  columns.forEach(function(col) {{
    col.addEventListener('dragover', function(e) {{
      if (col.classList.contains('kanban-column--disallowed')) {{ return; }}
      e.preventDefault();
    }});
    col.addEventListener('drop', function(e) {{
      if (col.classList.contains('kanban-column--disallowed')) {{ return; }}
      e.preventDefault();
      var id = e.dataTransfer.getData('text/plain');
      var to = col.dataset.column;
      var url = template.replace('{{id}}', encodeURIComponent(id)).replace('{{to}}', encodeURIComponent(to));
      fetch(url, {{ method: 'POST' }}).then(function() {{ location.reload(); }});
    }});
  }});
}})();
"#,
        template = move_path_template,
    )
}
