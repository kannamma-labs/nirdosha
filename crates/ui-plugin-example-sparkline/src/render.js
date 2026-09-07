// Spliced verbatim into the emitted `<script>` block by
// `ui_gen::generate_with_ui_components` (rfcs/0009 Phase B) -- runs in
// the same page as the rest of `ui_gen_template.html`'s client code, so
// `callFn`/`document`/CSS custom properties like `--md-primary` are all
// real globals already in scope, the same way the std `timeline` widget
// kind's own inline code already assumes.
//
// `node` is the `{type: "widget", kind: "sparkline", source, title,
// entries}` shape `ui_gen.rs::layout_json`'s `Widget` arm emits for
// every widget -- `node.source` names the zero-arg-from-the-client fn
// to call (a `.nir` author writes `sparkline { source: recent_sales
// field: "amount" }`, the same `source` key `timeline` already uses),
// `node.entries.field` names which key of each returned row to plot.
function render_sparkline(node) {
  const width = 120, height = 32;
  const holder = document.createElement("span");
  holder.className = "sparkline-widget";
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", `0 0 ${width} ${height}`);
  svg.setAttribute("width", String(width));
  svg.setAttribute("height", String(height));
  svg.setAttribute("role", "img");
  svg.setAttribute("aria-label", "sparkline" + (node.title ? ": " + node.title : ""));
  holder.appendChild(svg);

  const field = node.entries && node.entries.field;
  if (!node.source || !field) return holder;

  callFn(node.source, {}).then((data) => {
    const rows = Array.isArray(data) ? data : [];
    const values = rows.map((r) => Number(r && r[field]) || 0);
    if (values.length < 2) return;
    const max = Math.max(...values), min = Math.min(...values);
    const span = max - min || 1;
    const stepX = width / (values.length - 1);
    const points = values.map((v, i) => `${i * stepX},${height - ((v - min) / span) * height}`).join(" ");
    const line = document.createElementNS("http://www.w3.org/2000/svg", "polyline");
    line.setAttribute("points", points);
    line.setAttribute("fill", "none");
    line.setAttribute("stroke-width", "2");
    line.style.stroke = "var(--md-primary)";
    svg.appendChild(line);
  });

  return holder;
}
