use chrono::{Datelike, TimeZone, Utc};
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::options::OptionSchema;
use crate::payload::{Body, Status, TimelineData, TimelineEvent};
use crate::theme::{self, ColorKey, Theme};
use crate::time as t;

use super::{Registry, RenderOptions, Renderer, Shape};

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    pub date_format: Option<String>,
}

const COLOR_KEYS: &[ColorKey] = &[
    theme::STATUS_OK,
    theme::STATUS_WARN,
    theme::STATUS_ERROR,
    theme::TEXT_DIM,
    theme::TEXT_SECONDARY,
    theme::TEXT,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "bullet",
        type_hint: "\"●\" | \"•\" | \"▪\" | \"→\" | \"none\"",
        required: false,
        default: Some("none"),
        description: "Glyph prefixed to each event after the time column. `none` (default) shows only the time column and title.",
    },
    OptionSchema {
        name: "max_items",
        type_hint: "positive integer",
        required: false,
        default: Some("all events"),
        description: "Cap on rendered events, from the top. Older events are dropped when the list is longer than the cap.",
    },
    OptionSchema {
        name: "date_format",
        type_hint: "\"relative\" | \"absolute\"",
        required: false,
        default: Some("\"relative\""),
        description: "How the time column is formatted. `relative` shows `2h` / `3d`; `absolute` shows `2026-04-23` (ISO date).",
    },
    OptionSchema {
        name: "align",
        type_hint: "\"left\" | \"center\" | \"right\"",
        required: false,
        default: Some("\"left\""),
        description: "Horizontal alignment of the timeline within its cell.",
    },
];

/// Time-stamped event list. Each row shows a compact relative prefix (`"3h"`, `"2d"`, `"Apr 5"`)
/// computed at draw time against the current clock — cached payloads never freeze a stale
/// "ago" string. Optional `detail` hangs under the title; `status` colours the title.
pub struct ListTimelineRenderer;

const SEPARATOR: &str = " │ ";

impl Renderer for ListTimelineRenderer {
    fn name(&self) -> &str {
        "list_timeline"
    }
    fn description(&self) -> &'static str {
        "Chronological events with a left-aligned time column (relative `3h` / `2d` or absolute ISO date) separated from the title by a vertical bar, and an optional indented detail row underneath. Status colours the title; pick this for commits, releases, notifications."
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::Timeline]
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
        if let Body::Timeline(d) = body {
            render_timeline_at(frame, area, d, opts, Utc::now().timestamp(), theme);
        }
    }
}

fn render_timeline_at(
    frame: &mut Frame,
    area: Rect,
    data: &TimelineData,
    opts: &RenderOptions,
    now: i64,
    theme: &Theme,
) {
    let specific: Options = opts.parse_specific();
    let events: Vec<&TimelineEvent> = data
        .events
        .iter()
        .take(opts.max_items.unwrap_or(usize::MAX))
        .collect();
    let use_absolute = matches!(specific.date_format.as_deref(), Some("absolute"));
    let timezone = opts.timezone.as_deref();
    let locale = opts.locale.as_deref();
    let prefixes: Vec<String> = events
        .iter()
        .map(|e| format_prefix(e.timestamp, now, use_absolute, timezone, locale))
        .collect();
    let prefix_width = prefixes
        .iter()
        .map(|p| p.chars().count())
        .max()
        .unwrap_or(0);
    let bullet = bullet_for(opts.bullet.as_deref());
    let dim = Style::default().fg(theme.text_dim);

    let mut lines: Vec<Line> = Vec::with_capacity(events.len() * 2);
    let mut content_width: usize = 0;
    for (prefix, event) in prefixes.iter().zip(events.iter()) {
        let head = format!("{}{SEPARATOR}{bullet}", pad_left(prefix, prefix_width));
        content_width = content_width.max(head.chars().count() + event.title.chars().count());
        lines.push(Line::from(vec![
            Span::styled(head, dim),
            Span::styled(event.title.clone(), status_style(event.status, theme)),
        ]));
        if let Some(detail) = &event.detail {
            let indent = format!(
                "{}{SEPARATOR}{}",
                " ".repeat(prefix_width),
                " ".repeat(bullet.chars().count())
            );
            content_width = content_width.max(indent.chars().count() + detail.chars().count());
            lines.push(Line::from(vec![
                Span::styled(indent, dim),
                Span::styled(detail.clone(), Style::default().fg(theme.text_secondary)),
            ]));
        }
    }

    let target = align_rect(area, content_width as u16, opts.align.as_deref());
    // Base `theme.text` so event titles without status, and any padding chars around
    // styled spans, inherit the chrome colour instead of the terminal fg.
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().fg(theme.text)),
        target,
    );
}

fn bullet_for(option: Option<&str>) -> String {
    match option {
        None | Some("none") | Some("") => String::new(),
        Some(g) => format!("{g} "),
    }
}

fn format_prefix(
    ts: i64,
    now: i64,
    absolute: bool,
    timezone: Option<&str>,
    locale: Option<&str>,
) -> String {
    if absolute {
        format_iso_date(ts, timezone, locale)
    } else {
        format_relative(ts, now, timezone, locale)
    }
}

fn format_iso_date(ts: i64, timezone: Option<&str>, locale: Option<&str>) -> String {
    let Some(utc) = Utc.timestamp_opt(ts, 0).single() else {
        return "—".into();
    };
    let local = match t::parse_tz(timezone) {
        Some(tz) => utc.with_timezone(&tz).fixed_offset(),
        None => utc.with_timezone(&chrono::Local).fixed_offset(),
    };
    t::format_local(&local, "%Y-%m-%d", locale)
}

fn align_rect(area: Rect, content_width: u16, align: Option<&str>) -> Rect {
    if content_width == 0 || content_width >= area.width {
        return area;
    }
    let offset = match align {
        Some("center") => (area.width - content_width) / 2,
        Some("right") => area.width - content_width,
        _ => return area,
    };
    Rect {
        x: area.x + offset,
        y: area.y,
        width: content_width,
        height: area.height,
    }
}

fn pad_left(s: &str, width: usize) -> String {
    let n = s.chars().count();
    if n >= width {
        return s.to_string();
    }
    let mut out = String::with_capacity(width);
    out.extend(std::iter::repeat_n(' ', width - n));
    out.push_str(s);
    out
}

fn status_style(status: Option<Status>, theme: &Theme) -> Style {
    match status {
        Some(s) => Style::default().fg(super::status_badge::status_color(s, theme)),
        None => Style::default(),
    }
}

/// Relative label for `ts` seen from `now`. Past events shrink to `Ns`/`Nm`/`Nh`/`Nd`/`Nw`;
/// anything older than ~4 weeks falls back to an absolute date so the label stays short. Future
/// events get an `in …` prefix. Same clock for past and future keeps the column width stable.
fn format_relative(ts: i64, now: i64, timezone: Option<&str>, locale: Option<&str>) -> String {
    let delta = now - ts;
    match t::format_relative_compact(delta, locale) {
        Some(s) => s,
        None => format_absolute(ts, now, timezone, locale),
    }
}

fn format_absolute(ts: i64, now: i64, timezone: Option<&str>, locale: Option<&str>) -> String {
    let Some(event_utc) = Utc.timestamp_opt(ts, 0).single() else {
        return "—".into();
    };
    let event = match t::parse_tz(timezone) {
        Some(tz) => event_utc.with_timezone(&tz).fixed_offset(),
        None => event_utc.with_timezone(&chrono::Local).fixed_offset(),
    };
    let Some(now_utc) = Utc.timestamp_opt(now, 0).single() else {
        return t::format_local(&event, "%Y-%m-%d", locale);
    };
    let now_local = match t::parse_tz(timezone) {
        Some(tz) => now_utc.with_timezone(&tz).fixed_offset(),
        None => now_utc.with_timezone(&chrono::Local).fixed_offset(),
    };
    if event.year() == now_local.year() {
        t::format_local(&event, "%b %-d", locale)
    } else {
        t::format_local(&event, "%Y-%m-%d", locale)
    }
}

