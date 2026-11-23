use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::options::OptionSchema;
use crate::payload::{Body, RatioData};
use crate::theme::{self, ColorKey, Theme};

use super::{Registry, RenderOptions, Renderer, Shape};

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    pub segments: Option<u16>,
}

const COLOR_KEYS: &[ColorKey] = &[
    theme::TEXT,
    theme::TEXT_DIM,
    theme::STATUS_OK,
    theme::STATUS_WARN,
    theme::STATUS_ERROR,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "label",
        type_hint: "string",
        required: false,
        default: Some("payload.label"),
        description: "Optional prefix shown before the LED bar, e.g. `\"VOL\"` → `VOL ▰▰▰▱▱ 60%`. Falls back to `RatioData.label`; omitted when neither is set.",
    },
    OptionSchema {
        name: "segments",
        type_hint: "u16",
        required: false,
        default: Some("5"),
        description: "Number of LED segments. Defaults to the canonical 5-LED silhouette; raise for finer granularity (e.g. 10) when there's room. Values below 1 are clamped to 1.",
    },
    OptionSchema {
        name: "tone",
        type_hint: "\"neutral\" | \"fill\" | \"drain\"",
        required: false,
        default: Some("\"neutral\""),
        description: "How segment colour follows the value. `neutral` is single `theme.text`. `fill` treats the value as how-full (low → status_error, high → status_ok) — right for VU / quota progress. `drain` inverts (high → status_error) — right for usage-style readouts.",
    },
    OptionSchema {
        name: "value_format",
        type_hint: "\"percent\" | \"fraction\" | \"both\"",
        required: false,
        default: Some("\"percent\""),
        description: "Suffix format. `fraction` and `both` require `RatioData.denominator` — otherwise they fall back to `percent`.",
    },
];

const DEFAULT_SEGMENTS: u16 = 5;
const FILLED: &str = "▰";
const EMPTY: &str = "▱";

/// 5-LED segmented bar for `Ratio`. Discrete chunks instead of a continuous fill — the
/// retro-hardware sibling of `gauge_line`. Tone follows `gauge_battery` semantics: neutral
/// (default) for theme-colour, `fill` for low-is-bad, `drain` for high-is-bad.
pub struct GaugeSegmentRenderer;

impl Renderer for GaugeSegmentRenderer {
    fn name(&self) -> &str {
        "gauge_segment"
    }
    fn description(&self) -> &'static str {
        "Discrete LED-style bar for `Ratio`: by default five chunky pip blocks that light up in steps rather than a continuous fill. Pick this over `gauge_line` when you want a retro hardware look or coarser readability at a glance."
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::Ratio]
    }
    fn color_keys(&self) -> &[ColorKey] {
        COLOR_KEYS
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn render(
        &self,
        frame: &mut Frame,
        area: Rect,
        body: &Body,
        opts: &RenderOptions,
        theme: &Theme,
        _registry: &Registry,
    ) {
        if let Body::Ratio(d) = body {
            render_segment(frame, area, d, opts, theme);
        }
    }
}

fn render_segment(
    frame: &mut Frame,
    area: Rect,
    data: &RatioData,
    opts: &RenderOptions,
    theme: &Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let specific: Options = opts.parse_specific();
    let ratio = data.value.clamp(0.0, 1.0);
    let prefix = resolve_prefix(opts, data);
    let suffix = format_value(ratio, data.denominator, opts.value_format.as_deref());
    let segments = resolve_segment_count(specific.segments);
    let filled = (f64::from(segments) * ratio).round() as u16;
    let fill = tone_color(ratio, opts.tone.as_deref(), theme);
    let row = Rect::new(area.x, area.y + area.height / 2, area.width, 1);
    let line = Line::from(compose_spans(
        &prefix, &suffix, filled, segments, fill, theme,
    ));
    frame.render_widget(Paragraph::new(line), row);
}

fn compose_spans<'a>(
    prefix: &str,
    suffix: &str,
    filled: u16,
    segments: u16,
    fill: ratatui::style::Color,
    theme: &Theme,
) -> Vec<Span<'a>> {
    let mut spans = Vec::with_capacity(5);
    if !prefix.is_empty() {
        spans.push(Span::styled(
            format!("{prefix} "),
            Style::default().fg(theme.text),
        ));
    }
    spans.push(Span::styled(
        FILLED.repeat(filled as usize),
        Style::default().fg(fill),
    ));
    spans.push(Span::styled(
        EMPTY.repeat(segments.saturating_sub(filled) as usize),
        Style::default().fg(theme.text_dim),
    ));
    spans.push(Span::styled(
        format!(" {suffix}"),
        Style::default().fg(theme.text),
    ));
    spans
}

fn resolve_prefix(opts: &RenderOptions, data: &RatioData) -> String {
    opts.label
        .clone()
        .or_else(|| data.label.clone())
        .unwrap_or_default()
}

fn resolve_segment_count(opt: Option<u16>) -> u16 {
    opt.unwrap_or(DEFAULT_SEGMENTS).max(1)
}

fn format_value(ratio: f64, denominator: Option<u64>, mode: Option<&str>) -> String {
    let pct = (ratio * 100.0).round() as u64;
    match (mode.unwrap_or("percent"), denominator) {
        ("fraction", Some(d)) => format!("{} of {d}", numerator(ratio, d)),
        ("both", Some(d)) => format!("{pct}% ({} of {d})", numerator(ratio, d)),
        _ => format!("{pct}%"),
    }
}

fn numerator(ratio: f64, denominator: u64) -> u64 {
    (ratio * denominator as f64).round() as u64
}

fn tone_color(ratio: f64, tone: Option<&str>, theme: &Theme) -> ratatui::style::Color {
    match tone.unwrap_or("neutral") {
        "fill" => level_color(ratio, theme),
        "drain" => level_color(1.0 - ratio, theme),
        _ => theme.text,
    }
}

fn level_color(ratio: f64, theme: &Theme) -> ratatui::style::Color {
    if ratio < 0.20 {
        theme.status_error
    } else if ratio < 0.50 {
        theme.status_warn
    } else {
        theme.status_ok
    }
}

