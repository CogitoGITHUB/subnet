use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::options::OptionSchema;
use crate::payload::{BarsData, Body};
use crate::theme::{self, ColorKey, Theme};

use super::{Registry, RenderOptions, Renderer, Shape};

const COLOR_KEYS: &[ColorKey] = &[theme::TEXT, theme::TEXT_DIM];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "max_items",
        type_hint: "positive integer",
        required: false,
        default: Some("all bars"),
        description: "Cap on rendered ranks. Keeps the top N by value when the input has more bars than the cap.",
    },
    OptionSchema {
        name: "style",
        type_hint: "\"number\" | \"medal\" | \"none\"",
        required: false,
        default: Some("\"number\""),
        description: "Rank-prefix glyph. `number` (default) emits `1.` / `2.` / `3.`. `medal` emits 🥇 / 🥈 / 🥉 for the top three then numbers for the rest. `none` omits the prefix.",
    },
    OptionSchema {
        name: "align",
        type_hint: "\"left\" | \"center\" | \"right\"",
        required: false,
        default: Some("\"left\""),
        description: "Horizontal alignment of the whole ranking within its cell.",
    },
];

const MEDALS: [&str; 3] = ["🥇", "🥈", "🥉"];
const COLUMN_GAP: &str = "  ";

/// Rank-prefixed list view for the `Bars` shape. Preserves the fetcher's bar order — sort is
/// the fetcher's responsibility, not the renderer's, so that fetchers emitting recency-sorted
/// (`steam_owned_games sort = "recent"`), alphabetical, or any non-value-desc order display as
/// intended. Use `style = "medal"` to highlight the top three with 🥇/🥈/🥉.
pub struct ListRankingRenderer;

impl Renderer for ListRankingRenderer {
    fn name(&self) -> &str {
        "list_ranking"
    }
    fn description(&self) -> &'static str {
        "Rank-prefixed table preserving the fetcher's bar order: rank prefix (`1.` / `2.` numbers, 🥇/🥈/🥉 medals, or none), label, and right-aligned value column. The text-first sibling of `chart_bar` — pick this when the values matter more than their relative bar lengths. Sort is the fetcher's responsibility; `max_items` truncates from the head."
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::Bars]
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
        if let Body::Bars(d) = body {
            render_ranking(frame, area, d, opts, theme);
        }
    }
}

fn render_ranking(
    frame: &mut Frame,
    area: Rect,
    data: &BarsData,
    opts: &RenderOptions,
    theme: &Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let rows = capped_rows(data, opts.max_items);
    let style = opts.style.as_deref().unwrap_or("number");
    let prefixes: Vec<String> = (0..rows.len()).map(|i| rank_prefix(i, style)).collect();
    let widths = column_widths(&prefixes, &rows);
    // Stretch the label column so the value sits flush against the right edge of
    // the cell instead of leaving dead space, and truncate over-long labels so the
    // value doesn't get clipped off the right.
    let target = align_rect(area, widths.total() as u16, opts.align.as_deref());
    let prefix_part = if widths.prefix > 0 {
        widths.prefix + 1
    } else {
        0
    };
    let fixed = prefix_part + COLUMN_GAP.len() + widths.value;
    let label_width = (target.width as usize).saturating_sub(fixed);
    let effective = Widths {
        prefix: widths.prefix,
        label: label_width,
        value: widths.value,
    };
    let lines: Vec<Line> = rows
        .iter()
        .zip(prefixes.iter())
        .map(|(row, prefix)| compose_line(prefix, &row.label, &row.display, effective, theme))
        .collect();
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().fg(theme.text)),
        target,
    );
}

/// Row view. `display` is what the right column actually prints — `Bar.value_label` when
/// present, otherwise the numeric value stringified.
struct Row {
    label: String,
    display: String,
}

/// Build rows in the fetcher's bar order. `max_items` truncates from the head — sort is the
/// fetcher's job (lastfm's `chart.getTop*` is server-ranked, steam's `_owned_games` sorts by
/// playtime or recency per its `sort` option, basic_bars carries the user-authored order).
fn capped_rows(data: &BarsData, cap: Option<usize>) -> Vec<Row> {
    let mut rows: Vec<Row> = data
        .bars
        .iter()
        .map(|b| Row {
            label: b.label.clone(),
            display: b.value_label.clone().unwrap_or_else(|| b.value.to_string()),
        })
        .collect();
    if let Some(n) = cap {
        rows.truncate(n);
    }
    rows
}

fn rank_prefix(index: usize, style: &str) -> String {
    match style {
        "none" => String::new(),
        "medal" => MEDALS
            .get(index)
            .map(|m| (*m).to_string())
            .unwrap_or_else(|| format!("{}.", index + 1)),
        _ => format!("{}.", index + 1),
    }
}

#[derive(Debug, Clone, Copy)]
struct Widths {
    prefix: usize,
    label: usize,
    value: usize,
}

impl Widths {
    fn total(self) -> usize {
        let prefix_part = if self.prefix > 0 { self.prefix + 1 } else { 0 };
        prefix_part + self.label + COLUMN_GAP.len() + self.value
    }
}

fn column_widths(prefixes: &[String], rows: &[Row]) -> Widths {
    Widths {
        prefix: prefixes.iter().map(|p| display_width(p)).max().unwrap_or(0),
        label: rows
            .iter()
            .map(|r| r.label.chars().count())
            .max()
            .unwrap_or(0),
        value: rows
            .iter()
            .map(|r| r.display.chars().count())
            .max()
            .unwrap_or(0),
    }
}

fn compose_line<'a>(
    prefix: &str,
    label: &str,
    value: &str,
    widths: Widths,
    theme: &Theme,
) -> Line<'a> {
    let mut spans: Vec<Span<'a>> = Vec::with_capacity(4);
    if widths.prefix > 0 {
        let pad = widths.prefix.saturating_sub(display_width(prefix));
        spans.push(Span::styled(
            format!("{}{prefix} ", " ".repeat(pad)),
            Style::default().fg(theme.text_dim),
        ));
    }
    spans.push(Span::raw(fit_label(label, widths.label)));
    spans.push(Span::raw(COLUMN_GAP));
    spans.push(Span::raw(pad_left(value, widths.value)));
    Line::from(spans)
}

/// Pad with trailing spaces if `label` fits, else truncate from the right (with a single
/// `…` ellipsis) so the value column stays visible. Empty width returns empty.
fn fit_label(label: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let n = label.chars().count();
    if n <= width {
        let mut out = String::with_capacity(width);
        out.push_str(label);
        out.extend(std::iter::repeat_n(' ', width - n));
        return out;
    }
    if width == 1 {
        return "…".to_string();
    }
    let take = width - 1;
    let mut out: String = label.chars().take(take).collect();
    out.push('…');
    out
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

// Medal codepoints render two cells in every wcwidth-aware terminal; everything else we emit
// here is ASCII (digits, dot, padding spaces). A lookup table keeps the column maths honest
// without pulling in a unicode-width dependency for one renderer.
fn display_width(s: &str) -> usize {
    s.chars()
        .map(|c| {
            if matches!(c, '🥇' | '🥈' | '🥉') {
                2
            } else {
                1
            }
        })
        .sum()
}

