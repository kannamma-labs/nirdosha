//! `workspace!` archetype (T-06/B11): a declarative composite screen that
//! reads N independently-capped "context needs" (related alerts, case
//! transactions, entity 360, explainability, audit timeline) and merges
//! them under one shared row/time budget — never one giant join. Each
//! need is its own guarded sub-read (a real fn the screen module hand-
//! declares, same "macro wires, app crate provides the guarded logic"
//! convention `approval_inbox!`'s `sources:` already established); a
//! sub-read that fails, or a merged total that would exceed `budget`,
//! fails the WHOLE assembly with a named reason — nothing ever renders
//! partially (`screens-plan.md`'s B11 Performance note, made real, not
//! decorative).
//!
//! Supersedes `showcase_screens::{render_workspace, render_custom_screen,
//! LayoutNode}` — dead code with zero call sites anywhere in this corpus
//! before this ticket, the hand-built "unverified HTML" trees
//! `screens-plan.md` names as the anti-pattern this archetype retires.

use crate::showcase_screens::{render_graph, render_table, render_timeline};
use crate::theme::themed_page_shell;
use crate::web::html_escape;

/// Per-request row/time cap shared across every declared need. This is
/// an ADDITIONAL cap layered on top of whatever `row_cap`/`max_execution`
/// each need's own `guard_policy!` already enforces at the
/// `GuardedTable` layer — this one exists so a workspace composed of
/// several individually-legal reads still cannot produce an unbounded
/// merged response.
#[derive(Debug, Clone, Copy)]
pub struct WorkspaceBudget {
    pub max_rows: usize,
    pub max_execution_ms: u64,
}

/// One declared context need. `source` is a real guarded read (never
/// fabricated data) the screen module supplies; this type is only the
/// generic envelope `workspace!`'s expansion wires it through.
pub struct ContextNeed {
    pub need: &'static str,
    pub title: &'static str,
    /// "table" | "timeline" | "graph" — which `showcase_screens` renderer
    /// paints this panel's rows.
    pub render: &'static str,
    pub source: Box<dyn FnOnce() -> Result<Vec<serde_json::Value>, String> + Send>,
}

#[derive(Debug, Clone)]
pub struct ContextPanel {
    pub need: String,
    pub title: String,
    pub render: String,
    pub rows: Vec<serde_json::Value>,
}

/// Runs every declared need's guarded sub-read in parallel — one OS
/// thread per need. A workspace has a handful of declared needs at
/// most (this corpus's largest is 3), so thread-per-need is the
/// honest, simplest reading of "run in parallel" at this scale, not a
/// thread pool nothing here needs. Then merges under `budget`: a
/// sub-read error, OR a running row total that would exceed
/// `budget.max_rows`, OR total wall time exceeding
/// `budget.max_execution_ms`, fails the WHOLE assembly with a named
/// reason — never a partial render.
pub fn assemble_context(needs: Vec<ContextNeed>, budget: &WorkspaceBudget) -> Result<Vec<ContextPanel>, String> {
    let started = std::time::Instant::now();

    let results: Vec<(&'static str, &'static str, &'static str, Result<Vec<serde_json::Value>, String>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = needs
            .into_iter()
            .map(|n| {
                let need = n.need;
                let title = n.title;
                let render = n.render;
                scope.spawn(move || (need, title, render, (n.source)()))
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_else(|_| ("unknown", "unknown", "table", Err("context need panicked".to_string()))))
            .collect()
    });

    let elapsed_ms = started.elapsed().as_millis() as u64;
    if elapsed_ms > budget.max_execution_ms {
        return Err(format!(
            "workspace context assembly exceeded its time budget ({elapsed_ms}ms > {}ms) — refused, not partially rendered",
            budget.max_execution_ms
        ));
    }

    let mut panels = Vec::with_capacity(results.len());
    let mut total_rows = 0usize;
    for (need, title, render, result) in results {
        let rows = result.map_err(|reason| format!("context need `{need}` failed: {reason}"))?;
        total_rows += rows.len();
        if total_rows > budget.max_rows {
            return Err(format!(
                "context need `{need}` pushed the merged workspace read to {total_rows} rows, exceeding the budget of {} — whole read refused, never rendered partially",
                budget.max_rows
            ));
        }
        panels.push(ContextPanel { need: need.to_string(), title: title.to_string(), render: render.to_string(), rows });
    }
    Ok(panels)
}

pub fn workspace_html(title: &str, subject_header: &[(String, String)], panels: &[ContextPanel]) -> String {
    let header: String = subject_header
        .iter()
        .map(|(k, v)| format!("<span style=\"margin-right:1.5rem\"><strong>{}:</strong> {}</span>", html_escape(k), html_escape(v)))
        .collect();
    let panels_html: String = panels
        .iter()
        .map(|p| {
            let data = serde_json::Value::Array(p.rows.clone());
            let body = match p.render.as_str() {
                "graph" => render_graph(&data),
                "timeline" => render_timeline(&data),
                _ => render_table(&data),
            };
            format!("<div class=\"nir-card\" style=\"margin:0.5rem 0\"><h3>{}</h3>{body}</div>", html_escape(&p.title))
        })
        .collect();
    themed_page_shell(title, "", &format!("<div class=\"nir-card\" style=\"margin-bottom:1rem\">{header}</div>{panels_html}"))
}

/// Rendered when [`assemble_context`] returns `Err` — the named reason
/// is shown, never a partially-filled workspace.
pub fn workspace_context_failed_html(title: &str, reason: &str) -> String {
    themed_page_shell(
        title,
        "",
        &format!("<div class=\"nir-card\"><p class=\"empty\">Context assembly refused: {}</p></div>", html_escape(reason)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn need(need: &'static str, rows: usize) -> ContextNeed {
        ContextNeed {
            need,
            title: need,
            render: "table",
            source: Box::new(move || Ok((0..rows).map(|i| serde_json::json!({ "i": i })).collect())),
        }
    }

    #[test]
    fn merges_panels_under_budget() {
        let budget = WorkspaceBudget { max_rows: 100, max_execution_ms: 5000 };
        let panels = assemble_context(vec![need("a", 3), need("b", 4)], &budget).unwrap();
        assert_eq!(panels.len(), 2);
        assert_eq!(panels.iter().map(|p| p.rows.len()).sum::<usize>(), 7);
    }

    #[test]
    fn a_need_exceeding_the_shared_budget_fails_the_whole_assembly_with_a_named_reason() {
        let budget = WorkspaceBudget { max_rows: 5, max_execution_ms: 5000 };
        let err = assemble_context(vec![need("small", 3), need("huge", 100)], &budget).unwrap_err();
        assert!(err.contains("exceeding the budget of 5"), "reason must name the budget: {err}");
    }

    #[test]
    fn one_failing_need_fails_the_whole_assembly_not_a_partial_render() {
        let budget = WorkspaceBudget { max_rows: 100, max_execution_ms: 5000 };
        let failing = ContextNeed { need: "flaky", title: "Flaky", render: "table", source: Box::new(|| Err("upstream denied".to_string())) };
        let err = assemble_context(vec![need("ok", 2), failing], &budget).unwrap_err();
        assert!(err.contains("flaky") && err.contains("upstream denied"), "reason must name the failing need: {err}");
    }
}
