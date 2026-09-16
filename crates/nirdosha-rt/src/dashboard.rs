//! Runtime support for `nirdosha_rt::dashboard!` (RFC 0009 Track C,
//! Phase 0): `Metric`/`Chart` widget data assembly and a small,
//! hand-written inline-SVG renderer. No client-side JS — a dashboard
//! is a plain server-rendered page, refreshed via `<meta
//! http-equiv="refresh">` when `refresh_seconds` is set.

use crate::web::{html_escape, page_shell};
use serde_json::Value;

/// A `Metric`/stat function may return `i64` or `f64` — this trait
/// lets the macro call `.into_metric_value()` generically either way,
/// without needing to know the return type at macro-expansion time.
pub trait IntoMetricValue {
    fn into_metric_value(self) -> f64;
}
impl IntoMetricValue for i64 {
    fn into_metric_value(self) -> f64 {
        self as f64
    }
}
impl IntoMetricValue for f64 {
    fn into_metric_value(self) -> f64 {
        self
    }
}

/// A `Chart` function may return the JSON string directly (the
/// corpus's `Json = String` convention) or `Result<Json, E>` for any
/// `E` — this trait covers both without macro-time type knowledge.
pub trait IntoChartData {
    fn into_chart_data(self) -> Value;
}
impl IntoChartData for String {
    fn into_chart_data(self) -> Value {
        serde_json::from_str(&self).unwrap_or_else(|_| Value::Array(vec![]))
    }
}
impl<E> IntoChartData for Result<String, E> {
    fn into_chart_data(self) -> Value {
        match self {
            Ok(s) => s.into_chart_data(),
            Err(_) => Value::Array(vec![]),
        }
    }
}

/// One assembled widget's data — the shape `dashboard!`-generated code
/// builds and both `GET /dashboard.json` and `GET /dashboard` read
/// from the same `Vec<Value>`, so the two can never disagree.
pub fn metric_widget(label: &str, value: f64, target: Option<f64>, alert_below: bool) -> Value {
    let alert = target.is_some_and(|t| if alert_below { value < t } else { value > t });
    serde_json::json!({ "kind": "metric", "label": label, "value": value, "target": target, "alert": alert })
}

pub fn chart_widget(label: &str, mark: &str, data: Value) -> Value {
    serde_json::json!({ "kind": "chart", "label": label, "mark": mark, "data": data })
}

/// A full dashboard page: title, an optional auto-refresh `<meta>`,
/// and every widget rendered in registration order.
pub fn render_dashboard_html(title: &str, refresh_seconds: Option<u64>, widgets: &[Value]) -> String {
    let refresh_meta = match refresh_seconds {
        Some(secs) => format!("<meta http-equiv=\"refresh\" content=\"{secs}\">"),
        None => String::new(),
    };
    let mut body = String::new();
    for widget in widgets {
        body.push_str(&render_widget_html(widget));
    }
    page_shell(title, &refresh_meta, &body)
}

fn render_widget_html(widget: &Value) -> String {
    match widget["kind"].as_str() {
        Some("metric") => {
            let label = widget["label"].as_str().unwrap_or_default();
            let value = widget["value"].as_f64().unwrap_or_default();
            let alert = widget["alert"].as_bool().unwrap_or(false);
            let target_line = match widget["target"].as_f64() {
                Some(t) => format!("<div class=\"target\">target: {t}</div>"),
                None => String::new(),
            };
            format!(
                "<div class=\"widget metric{}\"><div class=\"label\">{}</div><div class=\"value\">{value}</div>{target_line}</div>",
                if alert { " alert" } else { "" },
                html_escape(label),
            )
        }
        Some("chart") => {
            let label = widget["label"].as_str().unwrap_or_default();
            let mark = widget["mark"].as_str().unwrap_or("bar");
            let data = widget["data"].as_array().cloned().unwrap_or_default();
            let svg = match mark {
                "line" => render_line_chart(&data),
                _ => render_bar_chart(&data),
            };
            format!("<div class=\"widget chart\"><div class=\"label\">{}</div>{svg}</div>", html_escape(label))
        }
        _ => String::new(),
    }
}

const CHART_HEIGHT: f64 = 150.0;
const BAR_WIDTH: f64 = 40.0;
const GAP: f64 = 12.0;

fn chart_points(data: &[Value]) -> Vec<(String, f64)> {
    data.iter()
        .map(|d| {
            let label = d["label"].as_str().unwrap_or_default().to_string();
            let value = d["value"].as_f64().unwrap_or(0.0);
            (label, value)
        })
        .collect()
}

pub fn render_bar_chart(data: &[Value]) -> String {
    let points = chart_points(data);
    if points.is_empty() {
        return "<p class=\"empty\">no data</p>".to_string();
    }
    let max = points.iter().map(|(_, v)| *v).fold(0.0_f64, f64::max).max(1.0);
    let width = points.len() as f64 * (BAR_WIDTH + GAP) + GAP;
    let mut svg = format!(
        "<svg width=\"{width}\" height=\"{}\" xmlns=\"http://www.w3.org/2000/svg\">",
        CHART_HEIGHT + 20.0
    );
    for (i, (label, value)) in points.iter().enumerate() {
        let bar_height = (value / max) * CHART_HEIGHT;
        let x = GAP + i as f64 * (BAR_WIDTH + GAP);
        let y = CHART_HEIGHT - bar_height;
        svg.push_str(&format!(
            "<rect x=\"{x}\" y=\"{y}\" width=\"{BAR_WIDTH}\" height=\"{bar_height}\" fill=\"#4a90d9\"/>\
             <text x=\"{}\" y=\"{}\" font-size=\"10\" text-anchor=\"middle\">{}</text>",
            x + BAR_WIDTH / 2.0,
            CHART_HEIGHT + 14.0,
            html_escape(label),
        ));
    }
    svg.push_str("</svg>");
    svg
}

pub fn render_line_chart(data: &[Value]) -> String {
    let points = chart_points(data);
    if points.is_empty() {
        return "<p class=\"empty\">no data</p>".to_string();
    }
    let max = points.iter().map(|(_, v)| *v).fold(0.0_f64, f64::max).max(1.0);
    let step = if points.len() > 1 { 300.0 / (points.len() - 1) as f64 } else { 0.0 };
    let coords: Vec<String> = points
        .iter()
        .enumerate()
        .map(|(i, (_, v))| {
            let x = i as f64 * step;
            let y = CHART_HEIGHT - (v / max) * CHART_HEIGHT;
            format!("{x},{y}")
        })
        .collect();
    let labels: String = points
        .iter()
        .enumerate()
        .map(|(i, (label, _))| {
            let x = i as f64 * step;
            format!("<text x=\"{x}\" y=\"{}\" font-size=\"10\" text-anchor=\"middle\">{}</text>", CHART_HEIGHT + 14.0, html_escape(label))
        })
        .collect();
    format!(
        "<svg width=\"320\" height=\"{}\" xmlns=\"http://www.w3.org/2000/svg\">\
         <polyline points=\"{}\" fill=\"none\" stroke=\"#4a90d9\" stroke-width=\"2\"/>{labels}</svg>",
        CHART_HEIGHT + 20.0,
        coords.join(" "),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_alert_fires_the_right_direction() {
        let below = metric_widget("x", 5.0, Some(10.0), true);
        assert_eq!(below["alert"], true);
        let above = metric_widget("x", 15.0, Some(10.0), false);
        assert_eq!(above["alert"], true);
        let ok = metric_widget("x", 5.0, Some(10.0), false);
        assert_eq!(ok["alert"], false);
    }

    #[test]
    fn chart_data_parses_both_plain_and_result_conventions() {
        let plain: String = "[{\"label\":\"a\",\"value\":1}]".to_string();
        assert_eq!(plain.into_chart_data(), serde_json::json!([{"label":"a","value":1}]));

        let wrapped: Result<String, &'static str> = Ok("[{\"label\":\"b\",\"value\":2}]".to_string());
        assert_eq!(wrapped.into_chart_data(), serde_json::json!([{"label":"b","value":2}]));

        let failed: Result<String, &'static str> = Err("boom");
        assert_eq!(failed.into_chart_data(), serde_json::json!([]));
    }

    #[test]
    fn bar_and_line_charts_render_without_panicking_on_empty_data() {
        assert!(render_bar_chart(&[]).contains("no data"));
        assert!(render_line_chart(&[]).contains("no data"));
    }

    #[test]
    fn dashboard_html_includes_refresh_meta_only_when_set() {
        let html = render_dashboard_html("T", Some(30), &[]);
        assert!(html.contains("http-equiv=\"refresh\" content=\"30\""));
        let html = render_dashboard_html("T", None, &[]);
        assert!(!html.contains("http-equiv"));
    }
}
