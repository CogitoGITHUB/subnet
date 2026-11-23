use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::options::OptionSchema;
use crate::payload::{BadgeData, Body, Status};
use crate::theme::{self, ColorKey, Theme};

use super::{Registry, RenderOptions, Renderer, Shape};

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    pub shape: Option<String>,
}

const COLOR_KEYS: &[ColorKey] = &[
    theme::STATUS_OK,
    theme::STATUS_WARN,
    theme::STATUS_ERROR,
    theme::TEXT,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "shape",
        type_hint: "\"rounded\" | \"square\" | \"none\"",
        required: false,
        default: Some("\"none\""),
        description: "Pill framing. `rounded` wraps the label in `( … )`, `square` in `[ … ]`, `none` (default) leaves the bare dot + label.",
    },
    OptionSchema {
        name: "padding",
        type_hint: "cells (u16)",
        required: false,
        default: Some("0"),
        description: "Horizontal padding inside the pill, before the dot and after the label.",
    },
    OptionSchema {
        name: "align",
        type_hint: "\"left\" | \"center\" | \"right\"",
        required: false,
        default: Some("\"left\""),
        description: "Horizontal placement of the pill within its cell.",
    },
];

/// Traffic-light / badge renderer: a single coloured dot (● green / yellow / red) driven by
/// `BadgeData.status`, followed by `BadgeData.label`. One indicator per widget — CI, deploy,
/// SLO, oncall. Rows of badges are a composition concern, handled by the nested layout
/// (`status_row`), not by stuffing multiple statuses into one payload.
pub struct StatusBadgeRenderer;

const DOT: &str = "●";

impl Renderer for StatusBadgeRenderer {
    fn name(&self) -> &str {
        "status_badge"
    }
    fn description(&self) -> &'static str {
        "A coloured dot (green / yellow / red, driven by status) followed by the badge label, optionally framed as a `( … )` or `[ … ]` pill. One indicator per widget — compose rows of badges through layout, not through this renderer."
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::Badge]
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
        if let Body::Badge(d) = body {
            render_badge(frame, area, d, opts, theme);
        }
    }
}

fn render_badge(
    frame: &mut Frame,
    area: Rect,
    data: &BadgeData,
    opts: &RenderOptions,
    theme: &Theme,
) {
    let label_style = Style::default().fg(theme.text);
    let dot_style = Style::default().fg(status_color(data.status, theme));
    let specific: Options = opts.parse_specific();
    let (open, close) = frame_glyphs(specific.shape.as_deref());
    let pad = opts.padding.unwrap_or(0) as usize;
    let spacer: String = " ".repeat(pad);
    let mut spans: Vec<Span> = Vec::with_capacity(7);
    if !open.is_empty() {
        spans.push(Span::styled(format!("{open}{spacer}"), label_style));
    } else if pad > 0 {
        spans.push(Span::styled(spacer.clone(), label_style));
    }
    spans.push(Span::styled(DOT, dot_style));
    spans.push(Span::styled(" ", label_style));
    spans.push(Span::styled(data.label.clone(), label_style));
    if !close.is_empty() {
        spans.push(Span::styled(format!("{spacer}{close}"), label_style));
    } else if pad > 0 {
        spans.push(Span::styled(spacer, label_style));
    }
    let p = Paragraph::new(Line::from(spans)).alignment(parse_align(opts.align.as_deref()));
    frame.render_widget(p, area);
}

fn frame_glyphs(shape: Option<&str>) -> (&'static str, &'static str) {
    match shape {
        Some("rounded") => ("( ", " )"),
        Some("square") => ("[ ", " ]"),
        _ => ("", ""),
    }
}

pub(super) fn status_color(status: Status, theme: &Theme) -> Color {
    match status {
        Status::Ok => theme.status_ok,
        Status::Warn => theme.status_warn,
        Status::Error => theme.status_error,
    }
}

fn parse_align(s: Option<&str>) -> Alignment {
    match s {
        Some("center") => Alignment::Center,
        Some("right") => Alignment::Right,
        _ => Alignment::Left,
    }
}

