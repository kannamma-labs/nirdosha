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

/// One card, as the macro's generated code already knows it: its id,
/// display title, and current column.
pub struct Card {
    pub id: i64,
    pub title: String,
    pub column: String,
}

/// `GET <path>` — one `<div class="kanban-column">` per declared
/// column, each holding its cards as `draggable="true"` elements. The
/// actual drag-and-drop wiring lives in `board_js`, loaded via a
/// `<script src="{asset_path}">` this function emits.
pub fn board_html(title: &str, columns: &[&str], cards: &[Card], asset_path: &str) -> String {
    let mut board = String::new();
    for column in columns {
        let mut cards_html = String::new();
        for card in cards.iter().filter(|c| &c.column == column) {
            cards_html.push_str(&format!(
                "<div class=\"kanban-card\" draggable=\"true\" data-id=\"{}\">{}</div>",
                card.id,
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
    let body = format!("<div class=\"kanban-board\">{board}</div><script src=\"{asset_path}\"></script>");
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
  document.querySelectorAll('.kanban-card').forEach(function(card) {{
    card.addEventListener('dragstart', function(e) {{
      e.dataTransfer.setData('text/plain', card.dataset.id);
    }});
  }});
  document.querySelectorAll('.kanban-column').forEach(function(col) {{
    col.addEventListener('dragover', function(e) {{ e.preventDefault(); }});
    col.addEventListener('drop', function(e) {{
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
