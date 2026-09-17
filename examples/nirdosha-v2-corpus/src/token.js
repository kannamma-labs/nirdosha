{
  "_comment": "Theme token file for the Nirdosha UI showcase. Load a custom copy via NIRDOSHA_THEME=/path/to/token.js. All values are optional; missing keys fall back to the embedded modern defaults.",
  "name": "Nirdosha UI Showcase Theme",
  "version": "1.0.0",
  "mode": "light",
  "font": {
    "family": "'Inter', -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif",
    "baseSize": "15px",
    "headingWeight": "650"
  },
  "colors": {
    "bg": "#f8fafc",
    "surface": "#ffffff",
    "surfaceElevated": "#ffffff",
    "surfaceHover": "#f1f5f9",
    "text": "#0f172a",
    "textMuted": "#64748b",
    "textInverse": "#ffffff",
    "primary": "#4f46e5",
    "primaryLight": "#818cf8",
    "primaryDark": "#4338ca",
    "secondary": "#06b6d4",
    "success": "#10b981",
    "warning": "#f59e0b",
    "danger": "#ef4444",
    "info": "#3b82f6",
    "border": "#e2e8f0",
    "shadow": "rgba(15, 23, 42, 0.08)",
    "shadowStrong": "rgba(15, 23, 42, 0.14)"
  },
  "radius": {
    "sm": "6px",
    "md": "12px",
    "lg": "18px",
    "xl": "24px",
    "full": "9999px"
  },
  "shadows": {
    "xs": "0 1px 2px rgba(15, 23, 42, 0.04)",
    "sm": "0 2px 6px rgba(15, 23, 42, 0.06)",
    "md": "0 4px 14px rgba(15, 23, 42, 0.08)",
    "lg": "0 8px 28px rgba(15, 23, 42, 0.10)",
    "xl": "0 16px 48px rgba(15, 23, 42, 0.12)",
    "glow": "0 0 40px rgba(79, 70, 229, 0.15)"
  },
  "animation": {
    "durationFast": "0.15s",
    "durationNormal": "0.3s",
    "durationSlow": "0.5s",
    "easing": "cubic-bezier(0.4, 0, 0.2, 1)",
    "bounce": "cubic-bezier(0.34, 1.56, 0.64, 1)",
    "stagger": "0.05s"
  },
  "layout": {
    "maxWidth": "1320px",
    "navHeight": "64px",
    "cardPadding": "1.5rem",
    "sectionGap": "1.5rem"
  },
  "components": {
    "button": {
      "shadow": "0 4px 14px rgba(79, 70, 229, 0.25)",
      "hoverLift": "translateY(-2px)",
      "hoverShadow": "0 8px 24px rgba(79, 70, 229, 0.35)",
      "activeScale": "scale(0.98)"
    },
    "card": {
      "shadow": "0 4px 14px rgba(15, 23, 42, 0.08)",
      "hoverShadow": "0 8px 28px rgba(15, 23, 42, 0.12)",
      "border": "1px solid rgba(226, 232, 240, 0.8)"
    },
    "input": {
      "shadow": "inset 0 2px 4px rgba(15, 23, 42, 0.04)",
      "focusShadow": "0 0 0 3px rgba(79, 70, 229, 0.15)"
    },
    "table": {
      "rowHover": "#f8fafc",
      "headerBg": "linear-gradient(180deg, #f8fafc 0%, #f1f5f9 100%)"
    },
    "nav": {
      "bg": "rgba(255, 255, 255, 0.85)",
      "backdropBlur": "12px",
      "shadow": "0 4px 20px rgba(15, 23, 42, 0.06)"
    },
    "badge": {
      "success": "#dcfce7",
      "successText": "#166534",
      "warning": "#fef3c7",
      "warningText": "#92400e",
      "danger": "#fee2e2",
      "dangerText": "#991b1b",
      "info": "#dbeafe",
      "infoText": "#1e40af"
    }
  }
}
