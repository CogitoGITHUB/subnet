//! Semantic colour keys. Renderers look up named keys rather
//! than hard-coding `Color::Green`. The colours come from the
//! style engine (the active NAME.style file), mapped by run.rs.
//!
//! - [`ColorKey`] is a static metadata record a renderer declares
//!   via [`Renderer::color_keys`](crate::render::Renderer::color_keys).
//! - [`Theme`] is the resolved palette passed to every `render()`
//!   call. Callers build it once with [`Theme::from_style`].
//!
//! Tokens: `bg` / `bg_subtle`, `text` / `text_dim` /
//! `text_secondary`, `panel_border` / `panel_title`, `status_ok` /
//! `status_warn` / `status_error`, `accent_today` / `accent_event`,
//! `palette_series` / `palette_heatmap`.
//! `Theme::default()` is only the fallback without a style file.

use ratatui::style::Color;

pub mod run;

/// Metadata describing one semantic colour token a renderer consumes. Lives next to the
/// renderer's declaration so it can't drift out of sync with what the code actually reads.
/// The xtask docs generator renders these into each renderer's reference page.
#[derive(Debug, Clone, Copy)]
pub struct ColorKey {
    /// Token name. Snake_case.
    pub name: &'static str,
    /// One-line human description — rendered verbatim into docs.
    pub description: &'static str,
}

// ── Single-colour keys ─────────────────────────────────────────────────────────────────────
pub const BG: ColorKey = ColorKey {
    name: "bg",
    description: "Viewport background. Painted across the entire splash area before widgets draw. The default is a deep-ocean navy; set `bg = \"reset\"` to inherit the terminal's own background instead (recommended on light terminals).",
};
pub const BG_SUBTLE: ColorKey = ColorKey {
    name: "bg_subtle",
    description: "Secondary surface painted when a `[[row]]` or `[[row.child]]` opts in with `bg = \"subtle\"`. One step off `bg` — for headers, callouts, or any slot that should visually separate from the main content band.",
};
pub const TEXT: ColorKey = ColorKey {
    name: "text",
    description: "Primary body text colour for plain renderers (text_plain, text_ascii, list_plain, animated_typewriter). The top step of the chrome ladder — brighter than `text_secondary`, reads as diffuse daylight on the ocean surface. Set `text = \"reset\"` to inherit the terminal's own foreground instead.",
};
pub const PANEL_BORDER: ColorKey = ColorKey {
    name: "panel_border",
    description: "Panel border colour. Defaults to a muted ocean teal-gray that sits between `bg` and `text_secondary` — visible as structure without competing with the coral `panel_title`. Set `panel_border = \"reset\"` to inherit the terminal's own foreground instead.",
};
pub const PANEL_TITLE: ColorKey = ColorKey {
    name: "panel_title",
    description: "Panel title colour. The coral hero of the Splash palette; set `panel_title = \"reset\"` to inherit the terminal's own foreground.",
};
pub const STATUS_OK: ColorKey = ColorKey {
    name: "status_ok",
    description: "Healthy / passing status (badges, table rows, timeline titles).",
};
pub const STATUS_WARN: ColorKey = ColorKey {
    name: "status_warn",
    description: "Degraded / warning status — visually distinct from both ok and error.",
};
pub const STATUS_ERROR: ColorKey = ColorKey {
    name: "status_error",
    description: "Failing / error status — reserved for attention-grabbing failures.",
};
pub const TEXT_DIM: ColorKey = ColorKey {
    name: "text_dim",
    description: "Barely-visible chrome text: timeline date prefixes, empty-state placeholder, clip hint. Replaces the old `Modifier::DIM` usage — set an explicit colour so rendering stays consistent across terminals (some render DIM as invisible, some as a no-op).",
};
pub const TEXT_SECONDARY: ColorKey = ColorKey {
    name: "text_secondary",
    description: "Slightly-less-dim secondary text, e.g. the detail line under a timeline entry.",
};
pub const ACCENT_TODAY: ColorKey = ColorKey {
    name: "accent_today",
    description: "Calendar \"today\" marker.",
};
pub const ACCENT_EVENT: ColorKey = ColorKey {
    name: "accent_event",
    description: "Calendar event marker and default scatter-plot dot colour.",
};

// ── Multi-colour palettes (`palette_*` family) ─────────────────────────────────────────────
pub const PALETTE_SERIES: ColorKey = ColorKey {
    name: "palette_series",
    description: "Palette cycled per-slice / per-series by chart_pie / chart_line / chart_scatter. TOML takes an array of colours.",
};
pub const PALETTE_HEATMAP: ColorKey = ColorKey {
    name: "palette_heatmap",
    description: "5-step intensity gradient for grid_heatmap (level 0 → level 4). TOML array.",
};

/// Resolved palette handed to each `render()` call. Immutable within a draw cycle.
#[derive(Debug, Clone)]
pub struct Theme {
    pub bg: Color,
    pub bg_subtle: Color,
    pub text: Color,
    pub panel_border: Color,
    pub panel_title: Color,
    pub status_ok: Color,
    pub status_warn: Color,
    pub status_error: Color,
    pub text_dim: Color,
    pub text_secondary: Color,
    pub accent_today: Color,
    pub accent_event: Color,
    pub palette_series: Vec<Color>,
    pub palette_heatmap: Vec<Color>,
}

impl Default for Theme {
    fn default() -> Self {
        // Colors come only from the style roles (run.rs).
        let tx = Color::Rgb(255, 255, 255);
        let dm = Color::Rgb(110, 110, 110);
        let er = Color::Rgb(220, 40, 40);
        let bg = Color::Reset;
        Self {
            bg,
            bg_subtle: bg,
            text: tx,
            panel_border: dm,
            panel_title: tx,
            status_ok: tx,
            status_warn: tx,
            status_error: er,
            text_dim: dm,
            text_secondary: tx,
            accent_today: tx,
            accent_event: tx,
            palette_series: default_series(),
            palette_heatmap: default_heatmap_ramp(),
        }
    }
}

/// Resolve a single-colour theme token by its TOML key (`"text"`, `"panel_title"`, `"status_ok"`
/// etc.). Used by renderers that accept a `color = "<token>"` option; unknown names return
/// `None` so the caller can fall back to the renderer's own default.
pub fn token_color(theme: &Theme, name: &str) -> Option<Color> {
    Some(match name {
        "bg" => theme.bg,
        "bg_subtle" => theme.bg_subtle,
        "text" => theme.text,
        "text_dim" => theme.text_dim,
        "text_secondary" => theme.text_secondary,
        "panel_border" => theme.panel_border,
        "panel_title" => theme.panel_title,
        "status_ok" => theme.status_ok,
        "status_warn" => theme.status_warn,
        "status_error" => theme.status_error,
        "accent_today" => theme.accent_today,
        "accent_event" => theme.accent_event,
        _ => return None,
    })
}

impl Theme {
    /// nth series colour with wrap-around; empty palette falls back to the text color.
    pub fn series_color(&self, i: usize) -> Color {
        if self.palette_series.is_empty() {
            return self.text;
        }
        self.palette_series[i % self.palette_series.len()]
    }

    /// Heatmap level in `0..5`, saturating at both ends. Empty ramp falls back to default.
    pub fn heatmap_level(&self, level: usize) -> Color {
        let ramp = if self.palette_heatmap.is_empty() {
            return default_heatmap_ramp()[level.min(4)];
        } else {
            &self.palette_heatmap
        };
        ramp[level.min(ramp.len() - 1)]
    }

    pub fn from_style() -> Self {
        run::apply(Self::default())
    }
}

/// Series palette: text and dim only.
fn default_series() -> Vec<Color> {
    vec![Color::Rgb(255, 255, 255), Color::Rgb(110, 110, 110)]
}

/// 5-step heatmap ramp: dim then text.
fn default_heatmap_ramp() -> Vec<Color> {
    let d = Color::Rgb(110, 110, 110);
    let t = Color::Rgb(255, 255, 255);
    vec![d, d, d, t, t]
}

