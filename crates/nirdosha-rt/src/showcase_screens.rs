//! Runtime HTML renderers for the declarative UI screen archetypes that
//! `crud_screens!`/`dashboard!` do not cover out of the box:
//! custom `screen` + `layout`, `workspace`, `workflow` queue, standalone
//! `visual` pages, and a role-based `landing` page.
//!
//! These are intentionally plain HTML — no client-side JS — so every
//! screen works as a real `Router` route today under plain `cargo`.

use crate::theme::themed_page_shell;
use crate::web::html_escape;

/// One field shown on a custom screen or workspace subject header.
#[derive(Clone)]
pub struct LayoutField {
    pub name: String,
    pub label: String,
    pub value: String,
}

/// One button/action on a custom screen.
#[derive(Clone)]
pub struct LayoutAction {
    pub label: String,
    pub href: String,
    pub style: String,
}

/// A simplified layout tree: row/column/group/tabs/timeline.
/// This is the runtime side of `/// nirdosha:layout { ... }` claims.
#[derive(Clone)]
pub enum LayoutNode {
    Row(Vec<LayoutNode>),
    Column(Vec<LayoutNode>),
    Group { title: String, fields: Vec<String> },
    Tabs(Vec<(String, LayoutNode)>),
    Timeline { source_label: String, events: Vec<TimelineEvent> },
    Divider,
}

#[derive(Clone)]
pub struct TimelineEvent {
    pub ts: i64,
    pub label: String,
    pub detail: String,
}

fn field_value(fields: &[LayoutField], name: &str) -> String {
    fields.iter().find(|f| f.name == name).map(|f| f.value.clone()).unwrap_or_default()
}

fn render_layout_node(node: &LayoutNode, fields: &[LayoutField], actions: &[LayoutAction]) -> String {
    match node {
        LayoutNode::Row(children) => {
            let cols: String = children.iter().map(|c| format!("<div style=\"flex:1;min-width:280px;margin:0.5rem\">{}</div>", render_layout_node(c, fields, actions))).collect();
            format!("<div class=\"nir-card\" style=\"display:flex;flex-wrap:wrap;gap:1rem;align-items:flex-start\">{cols}</div>")
        }
        LayoutNode::Column(children) => {
            let rows: String = children.iter().map(|c| format!("<div style=\"margin:0.5rem 0\">{}</div>", render_layout_node(c, fields, actions))).collect();
            format!("<div class=\"nir-card\" style=\"display:flex;flex-direction:column;gap:0.5rem\">{rows}</div>")
        }
        LayoutNode::Group { title, fields: field_names } => {
            let rows: String = field_names.iter().map(|n| {
                format!(
                    "<p><label>{}</label> {}</p>",
                    html_escape(&fields.iter().find(|f| f.name == *n).map(|f| f.label.clone()).unwrap_or_else(|| n.clone())),
                    html_escape(&field_value(fields, n))
                )
            }).collect();
            format!("<div class=\"nir-card\" style=\"margin:0.5rem 0\"><h4>{}</h4>{rows}</div>", html_escape(title))
        }
        LayoutNode::Tabs(tabs) => {
            let ids: Vec<String> = tabs.iter().enumerate().map(|(i, _)| format!("tab{i}")).collect();
            let labels: String = tabs.iter().enumerate().map(|(i, (label, _))| {
                format!(
                    "<button type=\"button\" class=\"nir-btn tab-btn\" data-target=\"{id}\">{}</button>",
                    html_escape(label),
                    id = ids[i]
                )
            }).collect();
            let bodies: String = tabs.iter().enumerate().map(|(i, (_, node))| {
                format!(
                    "<div class=\"tab-panel\" id=\"{}\" style=\"display:none\">{}</div>",
                    ids[i],
                    render_layout_node(node, fields, actions)
                )
            }).collect();
            let script = r#"<script>
(function(){
  var first = document.querySelector('.tab-btn');
  if(first){ first.classList.add('active'); first.click(); }
  document.querySelectorAll('.tab-btn').forEach(function(btn){
    btn.addEventListener('click', function(){
      var target = this.getAttribute('data-target');
      document.querySelectorAll('.tab-btn').forEach(function(b){ b.classList.remove('active'); });
      this.classList.add('active');
      document.querySelectorAll('.tab-panel').forEach(function(p){ p.style.display = 'none'; });
      document.getElementById(target).style.display = 'block';
    });
  });
})();
</script>"#;
            format!("<div class=\"nir-card\">{labels}{bodies}{script}</div>")
        }
        LayoutNode::Timeline { source_label, events } => {
            let rows: String = events.iter().map(|e| {
                format!(
                    "<li style=\"margin:0.5rem 0\"><strong>{}</strong> <span class=\"nir-text-muted\">({})</span> — {}</li>",
                    html_escape(&e.label),
                    e.ts,
                    html_escape(&e.detail)
                )
            }).collect();
            format!("<div><h4>{}</h4><ul>{rows}</ul></div>", html_escape(source_label))
        }
        LayoutNode::Divider => "<hr style=\"border:none;border-top:1px solid var(--nir-border);margin:1rem 0\">".to_string(),
    }
}

/// Render a custom screen (e.g. the Purchase Order screen) from a
/// layout tree, a field set, and a row of data.
pub fn render_custom_screen(
    title: &str,
    fields: &[LayoutField],
    actions: &[LayoutAction],
    layout: &LayoutNode,
    _row: &serde_json::Value,
) -> String {
    let layout_html = render_layout_node(layout, fields, actions);
    let action_html: String = actions.iter().map(|a| {
        format!(
            "<a href=\"{}\" class=\"nir-btn nir-btn-secondary\">{}</a>",
            html_escape(&a.href), html_escape(&a.label)
        )
    }).collect();
    let action_block = if actions.is_empty() { String::new() } else { format!("<div class=\"nir-card\" style=\"margin:1rem 0\">{action_html}</div>") };
    themed_page_shell(title, "", &format!("{action_block}{layout_html}"))
}

/// One panel in a workspace.
#[derive(Clone)]
pub struct WorkspacePanel {
    pub title: String,
    pub render: String, // "table", "graph", "timeline"
    pub data: serde_json::Value,
}

/// Render a composite workspace page: subject header + panels.
pub fn render_workspace(title: &str, subject_fields: &[LayoutField], panels: &[WorkspacePanel]) -> String {
    let header: String = subject_fields.iter().map(|f| {
        format!("<span style=\"margin-right:1.5rem\"><strong>{}:</strong> {}</span>", html_escape(&f.label), html_escape(&f.value))
    }).collect();
    let panels_html: String = panels.iter().map(|p| {
        let body = match p.render.as_str() {
            "graph" => render_graph(&p.data),
            "heatmap" => render_heatmap(&p.data),
            "timeline" => render_timeline(&p.data),
            _ => render_table(&p.data),
        };
        format!(
            "<div class=\"nir-card\" style=\"margin:0.5rem 0\">\
             <h3>{}</h3>{body}</div>",
            html_escape(&p.title)
        )
    }).collect();
    themed_page_shell(title, "", &format!(
        "<div class=\"nir-card\" style=\"margin-bottom:1rem\">{header}</div>{panels_html}"
    ))
}

/// Render a generic table from a JSON array of objects.
pub fn render_table(data: &serde_json::Value) -> String {
    let rows = match data.as_array() {
        Some(r) if !r.is_empty() => r.clone(),
        _ => return "<p class=\"empty\">No data.</p>".to_string(),
    };
    let headers: Vec<String> = rows[0].as_object().unwrap_or(&serde_json::Map::new()).keys().cloned().collect();
    let mut html = String::from("<table><thead><tr>");
    for h in &headers {
        html.push_str(&format!("<th>{}</th>", html_escape(h)));
    }
    html.push_str("</tr></thead><tbody>");
    for row in &rows {
        html.push_str("<tr>");
        for h in &headers {
            let v = row.get(h).unwrap_or(&serde_json::Value::Null);
            html.push_str(&format!("<td>{}</td>", html_escape(&value_display(v))));
        }
        html.push_str("</tr>");
    }
    html.push_str("</tbody></table>");
    html
}

fn value_display(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Inline-SVG graph renderer.
pub fn render_graph(data: &serde_json::Value) -> String {
    let nodes = data.get("nodes").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let edges = data.get("edges").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    if nodes.is_empty() {
        return "<p class=\"empty\">No graph data.</p>".to_string();
    }
    let cx = 140;
    let cy = 90;
    let radius = 70;
    let n = nodes.len();
    let positions: Vec<(i64, i64, String)> = nodes.iter().enumerate().map(|(i, node)| {
        let angle = 2.0 * std::f64::consts::PI * i as f64 / n.max(1) as f64 - std::f64::consts::FRAC_PI_2;
        let x = (cx as f64 + radius as f64 * angle.cos()).round() as i64;
        let y = (cy as f64 + radius as f64 * angle.sin()).round() as i64;
        let label = node.get("label").and_then(|v| v.as_str()).unwrap_or("").to_string();
        (x, y, label)
    }).collect();

    let mut edge_lines = String::new();
    for e in &edges {
        let src = e.get("source").and_then(|v| v.as_str()).unwrap_or("");
        let tgt = e.get("target").and_then(|v| v.as_str()).unwrap_or("");
        if let Some((x1, y1, _)) = positions.iter().find(|(_, _, l)| *l == src || nodes.iter().any(|n| n.get("id").and_then(|v| v.as_str()).unwrap_or("") == src && n.get("label").and_then(|v| v.as_str()).unwrap_or("") == *l)) {
            let _ = (x1, y1);
        }
        // Find positions by node id match against label for demo
        let src_pos = positions.iter().enumerate().find(|(i, _)| {
            nodes[*i].get("id").and_then(|v| v.as_str()).unwrap_or("") == src ||
            nodes[*i].get("label").and_then(|v| v.as_str()).unwrap_or("") == src
        }).map(|(_, p)| p);
        let tgt_pos = positions.iter().enumerate().find(|(i, _)| {
            nodes[*i].get("id").and_then(|v| v.as_str()).unwrap_or("") == tgt ||
            nodes[*i].get("label").and_then(|v| v.as_str()).unwrap_or("") == tgt
        }).map(|(_, p)| p);
        if let (Some((x1, y1, _)), Some((x2, y2, _))) = (src_pos, tgt_pos) {
            edge_lines.push_str(&format!(
                "<line x1=\"{x1}\" y1=\"{y1}\" x2=\"{x2}\" y2=\"{y2}\" stroke=\"var(--nir-text-muted)\" stroke-width=\"2\"/>"
            ));
        }
    }

    let mut circles = String::new();
    for (x, y, label) in &positions {
        circles.push_str(&format!(
            "<circle cx=\"{x}\" cy=\"{y}\" r=\"24\" fill=\"var(--nir-primary)\"/>\
             <text x=\"{x}\" y=\"{y}\" dy=\"4\" text-anchor=\"middle\" fill=\"white\" font-size=\"10\">{}</text>",
            html_escape(&label.chars().take(3).collect::<String>())
        ));
    }

    format!(
        "<svg class=\"nir-visual-svg\" width=\"300\" height=\"200\" xmlns=\"http://www.w3.org/2000/svg\">\
         {edge_lines}{circles}</svg>"
    )
}

/// Inline-SVG heatmap renderer (12×8 binned grid).
pub fn render_heatmap(data: &serde_json::Value) -> String {
    let points = data.as_array().cloned().unwrap_or_default();
    if points.is_empty() {
        return "<p class=\"empty\">No heatmap data.</p>".to_string();
    }
    let cols = 12;
    let rows = 8;
    let cell_w = 24;
    let cell_h = 20;
    let mut grid = vec![vec![0_i64; cols]; rows];
    for p in &points {
        let lat = p.get("lat").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let lng = p.get("lng").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let weight = p.get("weight").and_then(|v| v.as_i64()).unwrap_or(1);
        let x = ((lng + 180.0) / 360.0 * cols as f64).clamp(0.0, (cols - 1) as f64) as usize;
        let y = ((90.0 - lat) / 180.0 * rows as f64).clamp(0.0, (rows - 1) as f64) as usize;
        grid[y][x] += weight;
    }
    let max = grid.iter().flat_map(|r| r.iter()).copied().max().unwrap_or(1).max(1);
    let mut rects = String::new();
    for y in 0..rows {
        for x in 0..cols {
            let intensity = grid[y][x] as f64 / max as f64;
            let r = (255.0 * (1.0 - intensity)) as u8;
            let b = (255.0 * intensity) as u8;
            rects.push_str(&format!(
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"rgb({r},100,{b})\"/>",
                x * cell_w, y * cell_h, cell_w, cell_h
            ));
        }
    }
    format!(
        "<svg class=\"nir-visual-svg\" width=\"{}\" height=\"{}\" xmlns=\"http://www.w3.org/2000/svg\">{rects}</svg>",
        cols * cell_w, rows * cell_h
    )
}

/// Timeline renderer.
pub fn render_timeline(data: &serde_json::Value) -> String {
    let events = data.as_array().cloned().unwrap_or_default();
    if events.is_empty() {
        return "<p class=\"empty\">No timeline data.</p>".to_string();
    }
    let rows: String = events.iter().map(|e| {
        let ts = e.get("ts").and_then(|v| v.as_i64()).unwrap_or(0);
        let label = e.get("label").and_then(|v| v.as_str()).unwrap_or("");
        let detail = e.get("detail").and_then(|v| v.as_str()).unwrap_or("");
        format!(
            "<li style=\"margin:0.5rem 0\"><strong>{}</strong> <span class=\"nir-text-muted\">({ts})</span> — {}</li>",
            html_escape(label), html_escape(detail)
        )
    }).collect();
    format!("<ul>{rows}</ul>")
}

/// One workflow instance row.
#[derive(Clone)]
pub struct WorkflowInstance {
    pub id: i64,
    pub state: String,
    pub data_label: String,
}

/// Render a workflow queue with a state stepper and per-row actions.
pub fn render_workflow_queue(
    title: &str,
    states: &[String],
    instances: &[WorkflowInstance],
    actions: &[(&str, &str)], // (event, label) pairs valid from the queue
) -> String {
    let stepper = render_stepper(states, "");
    let mut rows = String::new();
    for inst in instances {
        let inst_stepper = render_stepper(states, &inst.state);
        let action_buttons: String = actions.iter().map(|(event, label)| {
            format!(
                "<form method=\"post\" action=\"/workflows/{}/advance\" style=\"display:inline\">\
                 <input type=\"hidden\" name=\"event\" value=\"{}\">\
                 <button type=\"submit\" style=\"margin:0 0.25rem\" class=\"nir-btn\">{}</button></form>",
                inst.id, html_escape(event), html_escape(label)
            )
        }).collect();
        rows.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            inst.id, html_escape(&inst.data_label), inst_stepper, action_buttons
        ));
    }
    let table = if rows.is_empty() {
        "<p class=\"empty\">No pending items.</p>".to_string()
    } else {
        format!(
            "<table><thead><tr><th>ID</th><th>Data</th><th>State</th><th>Actions</th></tr></thead><tbody>{rows}</tbody></table>"
        )
    };
    themed_page_shell(title, "", &format!("<div class=\"nir-card\"><h2>State stepper</h2>{stepper}{table}</div>"))
}

fn render_stepper(states: &[String], current: &str) -> String {
    let mut html = String::from("<div style=\"display:flex;align-items:center;gap:0.25rem;margin:0.5rem 0\">");
    let mut found_current = false;
    let legend_mode = current.is_empty();
    for (i, state) in states.iter().enumerate() {
        let is_current = state == current;
        let is_past = !legend_mode && !found_current && !is_current;
        if is_current {
            found_current = true;
        }
        let color = if legend_mode { "#d1d5db" } else if is_current { "#2563eb" } else if is_past { "#16a34a" } else { "#d1d5db" };
        let _text = if is_current || is_past { "white" } else { "#374151" };
        html.push_str(&format!(
            "<span class=\"nir-badge {}\">{}</span>",
            badge_class_for_state(&color),
            html_escape(state)
        ));
        if i + 1 < states.len() {
            html.push_str("<span class=\"nir-text-muted\">→</span>");
        }
    }
    html.push_str("</div>");
    html
}

/// Render a standalone visual page (graph/heatmap/timeline).
pub fn badge_class_for_state(color: &str) -> &'static str {
    match color {
        "#2563eb" => "nir-badge-info",
        "#16a34a" => "nir-badge-success",
        _ => "nir-badge-warning",
    }
}

pub fn render_visual_page(title: &str, kind: &str, data: &serde_json::Value) -> String {
    let body = match kind {
        "graph" => render_graph(data),
        "heatmap" => render_heatmap(data),
        "timeline" => render_timeline(data),
        _ => "<p>Unknown visual kind.</p>".to_string(),
    };
    themed_page_shell(title, "", &format!("<div class=\"nir-card\">{body}</div>"))
}

/// One landing rule.
#[derive(Clone)]
pub struct LandingRule {
    pub label: String,
    pub href: String,
    pub required_role: Option<String>,
}

/// Render a landing page that shows the entries this identity may use.
pub fn render_landing(title: &str, rules: &[LandingRule], identity_name: &str, roles: &[String]) -> String {
    let greeting = format!("<p>Signed in as <strong>{}</strong>. Choose a screen:</p>", html_escape(identity_name));
    let mut cards = String::new();
    for rule in rules {
        let allowed = rule.required_role.as_ref().map(|r| roles.contains(r)).unwrap_or(true);
        let cls = if allowed { "nir-btn nir-btn-secondary" } else { "nir-btn nir-btn-secondary disabled" };
        let href = if allowed { rule.href.clone() } else { "#".to_string() };
        let badge = if allowed { "" } else { " <span class=\"nir-badge nir-badge-warning\">requires role</span>" };
        cards.push_str(&format!(
            "<a href=\"{}\" class=\"{}\"><strong>{}</strong>{}</a>",
            html_escape(&href), cls, html_escape(&rule.label), badge
        ));
    }
    themed_page_shell(title, "", &format!("{greeting}<div style=\"display:flex;flex-wrap:wrap;gap:0.5rem\">{cards}</div>"))
}
