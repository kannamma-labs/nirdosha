//! Runtime theming for `nirdosha_rt`-served HTML screens.
//!
//! A theme is loaded from a JSON/JS-like file at startup and converted
//! into a block of CSS custom properties + keyframes + base rules. The
//! default shipped theme makes every generated screen look modern,
//! with soft shadows, rounded corners, and subtle lift/hover/focus
//! animations.

use crate::web::html_escape;
use std::sync::{Arc, Mutex};

static THEME: Mutex<Option<Arc<Theme>>> = Mutex::new(None);

/// One loaded theme.
#[derive(Clone, Debug, Default)]
pub struct Theme {
    pub values: serde_json::Map<String, serde_json::Value>,
}

impl Theme {
    /// Load a theme from a JSON string. Any parse error yields the
    /// default modern theme so the app never serves a broken page.
    pub fn from_json(text: &str) -> Self {
        match serde_json::from_str::<serde_json::Value>(text) {
            Ok(serde_json::Value::Object(map)) => Theme { values: map },
            _ => Theme::default_modern(),
        }
    }

    pub fn default_modern() -> Self {
        // Embedded from this crate's own source tree (issue #75 item
        // 3: a self-contained binary's default assets must not
        // `include_str!` reach outside its own crate directory into a
        // sibling example — that breaks the moment this crate is used,
        // vendored, or published independently of the workspace's
        // examples/ tree). `examples/nirdosha-v2-corpus/src/token.js`
        // is a worked *override* example, loaded at runtime via
        // `NIRDOSHA_THEME`, not the compile-time default.
        let text = include_str!("default_theme.json");
        Self::from_json(text)
    }

    pub fn get(&self, path: &str) -> Option<&serde_json::Value> {
        let mut cursor: &serde_json::Map<String, serde_json::Value> = &self.values;
        let parts: Vec<&str> = path.split('.').collect();
        for (i, key) in parts.iter().enumerate() {
            match cursor.get(*key) {
                Some(serde_json::Value::Object(m)) if i + 1 < parts.len() => cursor = m,
                Some(v) if i + 1 == parts.len() => return Some(v),
                _ => return None,
            }
        }
        None
    }

    pub fn string(&self, path: &str, fallback: &str) -> String {
        self.get(path)
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| fallback.to_string())
    }

    pub fn css(&self) -> String {
        let s = |path: &str, fallback: &str| self.string(path, fallback);
        format!(
            "{base}",
            base = CSS_BASE
                .replace("{bg}", &s("colors.bg", "#f8fafc"))
                .replace("{surface}", &s("colors.surface", "#ffffff"))
                .replace("{surfaceElevated}", &s("colors.surfaceElevated", "#ffffff"))
                .replace("{surfaceHover}", &s("colors.surfaceHover", "#f1f5f9"))
                .replace("{text}", &s("colors.text", "#0f172a"))
                .replace("{textMuted}", &s("colors.textMuted", "#64748b"))
                .replace("{textInverse}", &s("colors.textInverse", "#ffffff"))
                .replace("{primary}", &s("colors.primary", "#4f46e5"))
                .replace("{primaryLight}", &s("colors.primaryLight", "#818cf8"))
                .replace("{primaryDark}", &s("colors.primaryDark", "#4338ca"))
                .replace("{secondary}", &s("colors.secondary", "#06b6d4"))
                .replace("{success}", &s("colors.success", "#10b981"))
                .replace("{warning}", &s("colors.warning", "#f59e0b"))
                .replace("{danger}", &s("colors.danger", "#ef4444"))
                .replace("{info}", &s("colors.info", "#3b82f6"))
                .replace("{border}", &s("colors.border", "#e2e8f0"))
                .replace("{shadow}", &s("colors.shadow", "rgba(15, 23, 42, 0.08)"))
                .replace("{shadowStrong}", &s("colors.shadowStrong", "rgba(15, 23, 42, 0.14)"))
                .replace("{radius_sm}", &s("radius.sm", "6px"))
                .replace("{radius_md}", &s("radius.md", "12px"))
                .replace("{radius_lg}", &s("radius.lg", "18px"))
                .replace("{radius_xl}", &s("radius.xl", "24px"))
                .replace("{radius_full}", &s("radius.full", "9999px"))
                .replace("{shadow_xs}", &s("shadows.xs", "0 1px 2px rgba(15, 23, 42, 0.04)"))
                .replace("{shadow_sm}", &s("shadows.sm", "0 2px 6px rgba(15, 23, 42, 0.06)"))
                .replace("{shadow_md}", &s("shadows.md", "0 4px 14px rgba(15, 23, 42, 0.08)"))
                .replace("{shadow_lg}", &s("shadows.lg", "0 8px 28px rgba(15, 23, 42, 0.10)"))
                .replace("{shadow_xl}", &s("shadows.xl", "0 16px 48px rgba(15, 23, 42, 0.12)"))
                .replace("{shadow_glow}", &s("shadows.glow", "0 0 40px rgba(79, 70, 229, 0.15)"))
                .replace("{dur_fast}", &s("animation.durationFast", "0.15s"))
                .replace("{dur_normal}", &s("animation.durationNormal", "0.3s"))
                .replace("{dur_slow}", &s("animation.durationSlow", "0.5s"))
                .replace("{easing}", &s("animation.easing", "cubic-bezier(0.4, 0, 0.2, 1)"))
                .replace("{bounce}", &s("animation.bounce", "cubic-bezier(0.34, 1.56, 0.64, 1)"))
                .replace("{stagger}", &s("animation.stagger", "0.05s"))
                .replace("{font}", &s("font.family", "sans-serif"))
                .replace("{base_size}", &s("font.baseSize", "15px"))
                .replace("{heading_weight}", &s("font.headingWeight", "650"))
                .replace("{max_width}", &s("layout.maxWidth", "1320px"))
                .replace("{nav_height}", &s("layout.navHeight", "64px"))
                .replace("{card_padding}", &s("layout.cardPadding", "1.5rem"))
                .replace("{section_gap}", &s("layout.sectionGap", "1.5rem"))
                .replace("{card_border}", &s("components.card.border", "1px solid rgba(226, 232, 240, 0.8)"))
                .replace("{nav_bg}", &s("components.nav.bg", "rgba(255, 255, 255, 0.85)"))
                .replace("{nav_blur}", &s("components.nav.backdropBlur", "12px"))
                .replace("{nav_shadow}", &s("components.nav.shadow", "0 4px 20px rgba(15, 23, 42, 0.06)"))
                .replace("{btn_shadow}", &s("components.button.shadow", "0 4px 14px rgba(79, 70, 229, 0.25)"))
                .replace("{btn_hover_lift}", &s("components.button.hoverLift", "translateY(-2px)"))
                .replace("{btn_hover_shadow}", &s("components.button.hoverShadow", "0 8px 24px rgba(79, 70, 229, 0.35)"))
                .replace("{btn_active_scale}", &s("components.button.activeScale", "scale(0.98)"))
                .replace("{input_shadow}", &s("components.input.shadow", "inset 0 2px 4px rgba(15, 23, 42, 0.04)"))
                .replace("{input_focus_shadow}", &s("components.input.focusShadow", "0 0 0 3px rgba(79, 70, 229, 0.15)"))
                .replace("{table_header_bg}", &s("components.table.headerBg", "linear-gradient(180deg, #f8fafc 0%, #f1f5f9 100%)"))
                .replace("{table_row_hover}", &s("components.table.rowHover", "#f8fafc"))
                .replace("{badge_success}", &s("components.badge.success", "#dcfce7"))
                .replace("{badge_success_text}", &s("components.badge.successText", "#166534"))
                .replace("{badge_warning}", &s("components.badge.warning", "#fef3c7"))
                .replace("{badge_warning_text}", &s("components.badge.warningText", "#92400e"))
                .replace("{badge_danger}", &s("components.badge.danger", "#fee2e2"))
                .replace("{badge_danger_text}", &s("components.badge.dangerText", "#991b1b"))
                .replace("{badge_info}", &s("components.badge.info", "#dbeafe"))
                .replace("{badge_info_text}", &s("components.badge.infoText", "#1e40af"))
        )
    }
}

const CSS_BASE: &str = r#"
:root {
  --nir-bg: {bg}; --nir-surface: {surface}; --nir-surface-elevated: {surfaceElevated}; --nir-surface-hover: {surfaceHover};
  --nir-text: {text}; --nir-text-muted: {textMuted}; --nir-text-inverse: {textInverse};
  --nir-primary: {primary}; --nir-primary-light: {primaryLight}; --nir-primary-dark: {primaryDark};
  --nir-secondary: {secondary}; --nir-success: {success}; --nir-warning: {warning}; --nir-danger: {danger}; --nir-info: {info};
  --nir-border: {border}; --nir-shadow: {shadow}; --nir-shadow-strong: {shadowStrong};
  --nir-radius-sm: {radius_sm}; --nir-radius-md: {radius_md}; --nir-radius-lg: {radius_lg}; --nir-radius-xl: {radius_xl}; --nir-radius-full: {radius_full};
  --nir-shadow-xs: {shadow_xs}; --nir-shadow-sm: {shadow_sm}; --nir-shadow-md: {shadow_md}; --nir-shadow-lg: {shadow_lg}; --nir-shadow-xl: {shadow_xl}; --nir-shadow-glow: {shadow_glow};
  --nir-anim-fast: {dur_fast}; --nir-anim-normal: {dur_normal}; --nir-anim-slow: {dur_slow}; --nir-easing: {easing}; --nir-bounce: {bounce}; --nir-stagger: {stagger};
  --nir-font: {font}; --nir-base-size: {base_size}; --nir-heading-weight: {heading_weight};
  --nir-max-width: {max_width}; --nir-nav-height: {nav_height}; --nir-card-padding: {card_padding}; --nir-section-gap: {section_gap};
}

@keyframes fadeInDown { from { opacity: 0; transform: translateY(-12px); } to { opacity: 1; transform: translateY(0); } }
@keyframes fadeInUp { from { opacity: 0; transform: translateY(16px); } to { opacity: 1; transform: translateY(0); } }
@keyframes scaleIn { from { opacity: 0; transform: scale(0.96); } to { opacity: 1; transform: scale(1); } }
@keyframes shimmer { 0% { background-position: -200% 0; } 100% { background-position: 200% 0; } }
@keyframes pulseSoft { 0%, 100% { opacity: 1; } 50% { opacity: 0.7; } }

* { box-sizing: border-box; }
::-webkit-scrollbar { width: 8px; height: 8px; }
::-webkit-scrollbar-track { background: var(--nir-surface-hover); border-radius: var(--nir-radius-full); }
::-webkit-scrollbar-thumb { background: var(--nir-border); border-radius: var(--nir-radius-full); }
::-webkit-scrollbar-thumb:hover { background: var(--nir-text-muted); }
::selection { background: var(--nir-primary-light); color: var(--nir-text-inverse); }
html { scroll-behavior: smooth; }
body { font-family: var(--nir-font); font-size: var(--nir-base-size); margin: 0; padding: 0; color: var(--nir-text); background: var(--nir-bg); background-image: radial-gradient(circle at 10% 20%, rgba(79,70,229,0.04) 0%, transparent 20%), radial-gradient(circle at 90% 80%, rgba(6,182,212,0.04) 0%, transparent 20%); line-height: 1.6; -webkit-font-smoothing: antialiased; }
@media (prefers-reduced-motion: reduce) { *, *::before, *::after { animation-duration: 0.01ms !important; animation-iteration-count: 1 !important; transition-duration: 0.01ms !important; } }
@media (max-width: 768px) { .nir-shell { padding: 0.75rem; } .nir-nav { flex-wrap: wrap; height: auto; padding: 0.5rem; } .nir-nav a { padding: 0.35rem 0.5rem; font-size: 0.85rem; } }
.nir-shell { max-width: var(--nir-max-width); margin: 0 auto; padding: var(--nir-section-gap); animation: fadeInUp var(--nir-anim-slow) var(--nir-easing); }
.nir-card, .nir-metric { animation: scaleIn var(--nir-anim-normal) var(--nir-easing) backwards; }
.nir-shell .nir-card:nth-child(1) { animation-delay: 0s; }
.nir-shell .nir-card:nth-child(2) { animation-delay: var(--nir-stagger); }
.nir-shell .nir-card:nth-child(3) { animation-delay: calc(2 * var(--nir-stagger)); }
.nir-shell .nir-card:nth-child(4) { animation-delay: calc(3 * var(--nir-stagger)); }
.nir-shell .nir-card:nth-child(5) { animation-delay: calc(4 * var(--nir-stagger)); }
.nir-card { position: relative; overflow: hidden; background: var(--nir-surface); border: {card_border}; border-radius: var(--nir-radius-lg); padding: var(--nir-card-padding); box-shadow: var(--nir-shadow-md); transition: transform var(--nir-anim-fast) var(--nir-easing), box-shadow var(--nir-anim-fast) var(--nir-easing); }
.nir-card:hover { box-shadow: var(--nir-shadow-lg), 0 0 30px rgba(79,70,229,0.08); transform: translateY(-2px); }
.nir-card h3 { font-size: 1.1rem; margin-bottom: 0.75rem; }
.nir-card h4 { font-size: 1rem; margin-bottom: 0.5rem; color: var(--nir-text-muted); }
.nir-card::after { content: ""; position: absolute; inset: 0; border-radius: var(--nir-radius-lg); pointer-events: none; box-shadow: inset 0 1px 1px rgba(255,255,255,0.6); opacity: 0.6; background: linear-gradient(135deg, rgba(255,255,255,0.4) 0%, transparent 50%); }
h1, h2, h3, h4 { font-weight: var(--nir-heading-weight); margin: 0 0 1rem 0; letter-spacing: -0.02em; position: relative; }
h1::after, h2::after { content: ""; display: block; width: 48px; height: 4px; margin-top: 0.6rem; background: linear-gradient(90deg, var(--nir-primary), var(--nir-secondary)); border-radius: 2px; }
a { color: var(--nir-primary); text-decoration: none; transition: color var(--nir-anim-fast) var(--nir-easing), outline-offset var(--nir-anim-fast); outline: 2px solid transparent; outline-offset: 2px; }
a:hover { color: var(--nir-primary-dark); text-decoration: underline; }
a:focus-visible { outline: 2px solid var(--nir-primary-light); outline-offset: 2px; border-radius: var(--nir-radius-sm); }

.nir-nav { position: sticky; top: 0; z-index: 100; height: var(--nir-nav-height); display: flex; align-items: center; gap: 0.25rem; padding: 0 1.5rem; background: {nav_bg}; backdrop-filter: blur({nav_blur}); -webkit-backdrop-filter: blur({nav_blur}); box-shadow: {nav_shadow}; border-bottom: 1px solid var(--nir-border); animation: fadeInDown var(--nir-anim-normal) var(--nir-easing); }
.nir-nav a { padding: 0.5rem 0.85rem; border-radius: var(--nir-radius-md); color: var(--nir-text-muted); font-weight: 500; font-size: 0.92rem; transition: all var(--nir-anim-fast) var(--nir-easing); position: relative; }
.nir-nav a:hover { background: var(--nir-surface-hover); color: var(--nir-text); text-decoration: none; transform: translateY(-1px); }
.nir-nav a:focus-visible { outline: 2px solid var(--nir-primary-light); outline-offset: -2px; border-radius: var(--nir-radius-md); }
.nir-nav a::after { content: ""; position: absolute; left: 50%; bottom: 2px; width: 0; height: 2px; background: var(--nir-primary); transition: width var(--nir-anim-fast), left var(--nir-anim-fast); border-radius: 1px; }
.nir-nav a:hover::after { width: 70%; left: 15%; }
.nir-nav a.nir-active::after { display: none; }
.nir-nav a.nir-active { background: linear-gradient(135deg, var(--nir-primary), var(--nir-primary-light)); color: var(--nir-text-inverse); box-shadow: var(--nir-shadow-sm), var(--nir-shadow-glow); }
.nir-nav-spacer { flex: 1; }
.nir-nav-menu { position: relative; height: 100%; display: flex; align-items: center; }
.nir-nav-menu::before { content: ""; position: absolute; top: 100%; left: 0; right: 0; height: 0.4rem; }
.nir-nav-top { padding: 0.5rem 0.85rem; border-radius: var(--nir-radius-md); color: var(--nir-text-muted); font-weight: 500; font-size: 0.92rem; cursor: pointer; user-select: none; transition: all var(--nir-anim-fast) var(--nir-easing); white-space: nowrap; }
.nir-nav-top::after { content: " \25BE"; font-size: 0.7em; opacity: 0.6; }
.nir-nav-menu:hover .nir-nav-top, .nir-nav-menu:focus-within .nir-nav-top { background: var(--nir-surface-hover); color: var(--nir-text); }
.nir-nav-dropdown { display: none; flex-direction: column; position: absolute; top: 100%; left: 0; min-width: 200px; padding: 0.5rem; margin-top: 0.4rem; background: var(--nir-surface); border: {card_border}; border-radius: var(--nir-radius-lg); box-shadow: var(--nir-shadow-lg); z-index: 150; animation: fadeInDown var(--nir-anim-fast) var(--nir-easing); }
.nir-nav-menu:hover .nir-nav-dropdown, .nir-nav-menu:focus-within .nir-nav-dropdown { display: flex; }
.nir-nav-dropdown a { padding: 0.55rem 0.75rem; }

button, .nir-btn { display: inline-flex; align-items: center; justify-content: center; gap: 0.4rem; padding: 0.55rem 1.1rem; border: none; border-radius: var(--nir-radius-md); background: linear-gradient(135deg, var(--nir-primary), var(--nir-primary-light)); color: var(--nir-text-inverse); font-weight: 600; font-size: 0.92rem; cursor: pointer; box-shadow: {btn_shadow}; transition: transform var(--nir-anim-fast) var(--nir-easing), box-shadow var(--nir-anim-fast) var(--nir-easing), filter var(--nir-anim-fast); }
button:hover, .nir-btn:hover { transform: {btn_hover_lift}; box-shadow: {btn_hover_shadow}; filter: brightness(1.05); }
button:active, .nir-btn:active { transform: {btn_active_scale}; filter: brightness(0.98); }
.nir-btn-secondary { background: var(--nir-surface); color: var(--nir-primary); border: 1px solid var(--nir-border); box-shadow: var(--nir-shadow-xs); }
.nir-btn-secondary:hover { background: var(--nir-surface-hover); box-shadow: var(--nir-shadow-sm); }
.nir-btn.tab-btn { background: var(--nir-surface); color: var(--nir-text); border: 1px solid var(--nir-border); box-shadow: var(--nir-shadow-xs); margin-right: 0.25rem; }
.nir-btn.tab-btn:hover { background: var(--nir-surface-hover); }
.nir-btn.tab-btn.active { background: var(--nir-primary); color: var(--nir-text-inverse); }
.tab-panel { padding-top: 1rem; }
.nir-btn.disabled { opacity: 0.5; cursor: not-allowed; filter: grayscale(0.6); transform: none !important; }
.nir-btn.disabled:hover { transform: none !important; box-shadow: var(--nir-shadow-xs) !important; }
.nir-btn-danger { background: linear-gradient(135deg, var(--nir-danger), #f87171); box-shadow: 0 4px 14px rgba(239,68,68,0.25); }
button:focus-visible, .nir-btn:focus-visible { outline: 2px solid var(--nir-primary-light); outline-offset: 2px; border-radius: var(--nir-radius-md); }
.nir-fab { position: fixed; bottom: 2rem; right: 2rem; border-radius: var(--nir-radius-full); width: 3.5rem; height: 3.5rem; padding: 0; font-size: 1.5rem; box-shadow: var(--nir-shadow-lg); z-index: 90; }
.nir-fab:hover { transform: translateY(-4px) rotate(90deg); }
.nir-tooltip { position: relative; cursor: help; border-bottom: 1px dashed var(--nir-border); }
.nir-tooltip::after { content: attr(data-tip); position: absolute; bottom: 120%; left: 50%; transform: translateX(-50%); background: var(--nir-text); color: var(--nir-text-inverse); padding: 0.4rem 0.6rem; border-radius: var(--nir-radius-md); font-size: 0.75rem; white-space: nowrap; opacity: 0; pointer-events: none; transition: opacity var(--nir-anim-fast); box-shadow: var(--nir-shadow-md); }
.nir-tooltip:hover::after { opacity: 1; }
.feed-item { margin: 0; }
.nir-toast { position: fixed; top: 1rem; right: 1rem; background: var(--nir-surface); border: {card_border}; border-radius: var(--nir-radius-lg); padding: 1rem 1.25rem; box-shadow: var(--nir-shadow-lg); animation: fadeInDown var(--nir-anim-normal) var(--nir-easing); z-index: 200; max-width: 320px; }

.nir-input, input[type=text], input[type=number], input[type=password], input[type=email], input[type=search], select, textarea { padding: 0.55rem 0.85rem; border: 1px solid var(--nir-border); border-radius: var(--nir-radius-md); background: var(--nir-surface); color: var(--nir-text); font-size: 0.95rem; box-shadow: {input_shadow}; transition: border-color var(--nir-anim-fast), box-shadow var(--nir-anim-fast); }
input:focus, select:focus, textarea:focus { outline: none; border-color: var(--nir-primary); box-shadow: {input_focus_shadow}; transform: translateY(-1px); }

table { width: 100%; border-collapse: separate; border-spacing: 0; border-radius: var(--nir-radius-lg); overflow: hidden; background: var(--nir-surface); box-shadow: var(--nir-shadow-sm); animation: fadeInUp var(--nir-anim-normal) var(--nir-easing); }
th { background: {table_header_bg}; color: var(--nir-text-muted); font-weight: 600; font-size: 0.78rem; text-transform: uppercase; letter-spacing: 0.04em; padding: 0.8rem 1rem; border-bottom: 1px solid var(--nir-border); }
td { padding: 0.85rem 1rem; border-bottom: 1px solid var(--nir-border); transition: background var(--nir-anim-fast); }
tr:hover td { background: {table_row_hover}; }
tr { transition: transform var(--nir-anim-fast); }
tr:hover { transform: scale(1.003); }
tr:last-child td { border-bottom: none; }

.nir-metric { background: var(--nir-surface); border: {card_border}; border-radius: var(--nir-radius-lg); padding: 1.25rem 1.5rem; min-width: 170px; box-shadow: var(--nir-shadow-md); transition: box-shadow var(--nir-anim-fast), transform var(--nir-anim-fast); }
.nir-metric:hover { box-shadow: var(--nir-shadow-lg), 0 0 30px rgba(79,70,229,0.08); transform: translateY(-2px); }
.nir-metric.important { animation: pulseSoft 2s var(--nir-easing) infinite; }
.nir-metric .value { font-size: 2.4rem; font-weight: 700; background: linear-gradient(135deg, var(--nir-primary), var(--nir-secondary)); -webkit-background-clip: text; -webkit-text-fill-color: transparent; }
.nir-metric .label { color: var(--nir-text-muted); font-size: 0.85rem; font-weight: 500; }
.nir-chart { background: var(--nir-surface); border: {card_border}; border-radius: var(--nir-radius-lg); padding: 1.25rem 1.5rem; box-shadow: var(--nir-shadow-md); }
.nir-chart .label { color: var(--nir-text-muted); font-size: 0.85rem; font-weight: 500; margin-bottom: 0.5rem; }

.nir-badge { display: inline-flex; align-items: center; box-shadow: 0 1px 2px rgba(15, 23, 42, 0.05); transition: transform var(--nir-anim-fast); }
.nir-badge:hover { transform: translateY(-1px); } padding: 0.25rem 0.7rem; border-radius: var(--nir-radius-full); font-size: 0.78rem; font-weight: 600; letter-spacing: 0.01em; }
.nir-badge-success { background: {badge_success}; color: {badge_success_text}; }
.nir-badge-warning { background: {badge_warning}; color: {badge_warning_text}; }
.nir-badge-danger { background: {badge_danger}; color: {badge_danger_text}; }
.nir-badge-info { background: {badge_info}; color: {badge_info_text}; }

.danger { color: var(--nir-danger); }
.nir-empty { color: var(--nir-text-muted); font-style: italic; padding: 2rem; text-align: center; }
.nir-errors, .errors { color: var(--nir-danger); background: {badge_danger}; padding: 0.75rem 1rem; border-radius: var(--nir-radius-md); }
.errors li { margin: 0.25rem 0; }

form p { margin: 0.9rem 0; }
form[method="get"] { display: flex; align-items: center; gap: 0.5rem; flex-wrap: wrap; }
form[method="get"] p { margin: 0; }
form label { display: block; font-weight: 500; font-size: 0.9rem; color: var(--nir-text-muted); margin-bottom: 0.35rem; }

.nir-landing-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(260px, 1fr)); gap: 1rem; }
.nir-landing-card { display: block; background: var(--nir-surface); border: {card_border}; border-radius: var(--nir-radius-lg); padding: 1.25rem; box-shadow: var(--nir-shadow-sm); transition: all var(--nir-anim-fast) var(--nir-easing); color: var(--nir-text); }
.nir-landing-card:hover { text-decoration: none; transform: translateY(-3px); box-shadow: var(--nir-shadow-lg); }
.nir-landing-card h3 { margin: 0 0 0.35rem; color: var(--nir-primary); }
.nir-landing-card p { margin: 0; color: var(--nir-text-muted); font-size: 0.9rem; }
.nir-landing-card.disabled { opacity: 0.55; cursor: not-allowed; filter: grayscale(0.5); }
.nir-landing-card.disabled:hover { transform: none; box-shadow: var(--nir-shadow-sm); }
.nir-landing-card.disabled { opacity: 0.55; cursor: not-allowed; filter: grayscale(0.5); }
.nir-landing-card.disabled:hover { transform: none; box-shadow: var(--nir-shadow-sm); }

.nir-chart svg rect { transition: opacity var(--nir-anim-fast); }
.nir-chart:hover { box-shadow: var(--nir-shadow-lg), 0 0 30px rgba(79,70,229,0.08); }
.nir-chart svg rect:hover { opacity: 0.8; }
.nir-chart svg, .nir-visual svg, .nir-visual-svg { background: var(--nir-surface); border: {card_border}; border-radius: var(--nir-radius-lg); box-shadow: var(--nir-shadow-sm); }

.nir-skeleton { border: none; background: linear-gradient(90deg, var(--nir-surface-hover) 25%, var(--nir-surface) 50%, var(--nir-surface-hover) 75%); background-size: 200% 100%; animation: shimmer 1.5s infinite; border-radius: var(--nir-radius-md); height: 1rem; margin: 0.5rem 0; }

.nir-login-card { max-width: 380px; margin: 6rem auto; background: var(--nir-surface); border: {card_border}; border-radius: var(--nir-radius-xl); padding: 2rem; box-shadow: var(--nir-shadow-lg); animation: scaleIn var(--nir-anim-normal) var(--nir-bounce); transition: box-shadow var(--nir-anim-fast), transform var(--nir-anim-fast); }
.nir-login-card:hover { box-shadow: var(--nir-shadow-xl), 0 0 40px rgba(79,70,229,0.12); transform: translateY(-3px); }
"#;

/// Load the global theme from a file path if one is provided, otherwise
/// use the embedded default modern theme. Call once at startup.
pub fn load(path: Option<&std::path::Path>) {
    let theme = match path {
        Some(p) => match std::fs::read_to_string(p) {
            Ok(text) => Theme::from_json(&text),
            Err(_) => Theme::default_modern(),
        },
        None => Theme::default_modern(),
    };
    if let Ok(mut guard) = THEME.lock() {
        *guard = Some(Arc::new(theme));
    }
}

/// Get a clone of the currently loaded theme.
pub fn current() -> Arc<Theme> {
    if let Ok(guard) = THEME.lock() {
        if let Some(theme) = guard.as_ref() {
            return theme.clone();
        }
    }
    Arc::new(Theme::default_modern())
}

/// Build a complete page using the loaded theme.
pub fn themed_page_shell(title: &str, nav: &str, body: &str) -> String {
    themed_page_shell_ex(title, nav, "", body)
}

/// Build a complete page with extra `<head>` content (e.g. refresh meta).
pub fn themed_page_shell_ex(title: &str, nav: &str, extra_head: &str, body: &str) -> String {
    let theme = current();
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{}</title>\
         <link rel=\"preconnect\" href=\"https://fonts.googleapis.com\"><link rel=\"preconnect\" href=\"https://fonts.gstatic.com\" crossorigin>\
         <link href=\"https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&display=swap\" rel=\"stylesheet\">\
         {extra_head}<style>{}</style></head><body>{}<div class=\"nir-shell\">{}</div></body></html>",
        html_escape(title),
        theme.css(),
        nav,
        body
    )
}
