use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table},
};

use crate::options::OptionSchema;
use crate::payload::{Body, EntriesData, Entry};
use crate::theme::{self, ColorKey, Theme};

use super::{Registry, RenderOptions, Renderer, Shape};

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    pub layout: Option<String>,
    #[serde(default)]
    pub column_align: Option<Vec<String>>,
}

const COLOR_KEYS: &[ColorKey] = &[
    theme::STATUS_OK,
    theme::STATUS_WARN,
    theme::STATUS_ERROR,
    theme::TEXT,
    theme::TEXT_DIM,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "layout",
        type_hint: "\"rows\" | \"inline\"",
        required: false,
        default: Some("\"rows\""),
        description: "Layout mode. `rows` draws one key/value row per entry; `inline` condenses to a single line of `key: value · key: value`.",
    },
    OptionSchema {
        name: "column_align",
        type_hint: "array of \"left\" | \"center\" | \"right\"",
        required: false,
        default: Some("[\"left\", \"left\"]"),
        description: "Per-column alignment. First entry aligns the key column, second the value column. Ignored in `inline` layout.",
    },
    OptionSchema {
        name: "align",
        type_hint: "\"left\" | \"center\" | \"right\"",
        required: false,
        default: Some("\"left\""),
        description: "Horizontal alignment of the rendered line. Honoured in `inline` layout only — `rows` uses `column_align` per column instead.",
    },
];

/// Key/value table renderer over the `Entries` shape. Internally a ratatui `Table` with two
/// columns (key, value) — the name reflects the widget, not the data, so future alternative
/// renderers for the same shape (card layout, inline chips) don't have to fight over "list".
pub struct GridTableRenderer;

impl Renderer for GridTableRenderer {
    fn name(&self) -> &str {
        "grid_table"
    }
    fn description(&self) -> &'static str {
        "Two-column key/value rows tinted by per-row status, with an `inline` layout that condenses the same entries into a single `key: value · key: value` line. The default look for entry-style payloads (system info, project metadata, env summaries)."
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::Entries]
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
        if let Body::Entries(d) = body {
            let specific: Options = opts.parse_specific();
            match specific.layout.as_deref() {
                Some("inline") => render_inline(frame, area, d, opts, theme),
                _ => render_rows(frame, area, d, &specific, theme),
            }
        }
    }
}

fn render_rows(
    frame: &mut Frame,
    area: Rect,
    data: &EntriesData,
    specific: &Options,
    theme: &Theme,
) {
    let column_align = column_alignments(specific.column_align.as_deref());
    let key_width = data
        .items
        .iter()
        .map(|e| e.key.chars().count())
        .max()
        .unwrap_or(0)
        .min(area.width as usize);
    let widths = [Constraint::Length(key_width as u16), Constraint::Fill(1)];
    let rows = data.items.iter().map(|e| to_row(e, column_align, theme));
    frame.render_widget(
        Table::new(rows, widths).style(Style::default().fg(theme.text)),
        area,
    );
}

fn render_inline(
    frame: &mut Frame,
    area: Rect,
    data: &EntriesData,
    opts: &RenderOptions,
    theme: &Theme,
) {
    let text = Style::default().fg(theme.text);
    let dim = Style::default().fg(theme.text_dim);
    let mut spans: Vec<Span> = Vec::new();
    for (i, entry) in data.items.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", dim));
        }
        let row_style = entry
            .status
            .map(|s| Style::default().fg(super::status_badge::status_color(s, theme)))
            .unwrap_or(text);
        spans.push(Span::styled(entry.key.clone(), row_style));
        if let Some(v) = entry.value.as_deref() {
            spans.push(Span::styled(": ", row_style));
            spans.push(Span::styled(v.to_string(), row_style));
        }
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans))
            .style(text)
            .alignment(parse_align(opts.align.as_deref())),
        area,
    );
}

fn parse_align(s: Option<&str>) -> Alignment {
    match s {
        Some("center") => Alignment::Center,
        Some("right") => Alignment::Right,
        _ => Alignment::Left,
    }
}

fn column_alignments(raw: Option<&[String]>) -> [Alignment; 2] {
    let get = |i: usize, default: Alignment| -> Alignment {
        raw.and_then(|a| a.get(i))
            .map(|s| match s.as_str() {
                "center" => Alignment::Center,
                "right" => Alignment::Right,
                _ => Alignment::Left,
            })
            .unwrap_or(default)
    };
    // Key left-aligned, value right-aligned by default so the value sits flush against
    // the right edge of the cell (fills the panel width). Override via `column_align`.
    [get(0, Alignment::Left), get(1, Alignment::Right)]
}

fn to_row<'a>(item: &'a Entry, aligns: [Alignment; 2], theme: &Theme) -> Row<'a> {
    let key = Cell::from(Line::from(item.key.clone()).alignment(aligns[0]));
    let value = Cell::from(Line::from(item.value.clone().unwrap_or_default()).alignment(aligns[1]));
    let mut row = Row::new(vec![key, value]);
    if let Some(status) = item.status {
        row = row.style(Style::default().fg(super::status_badge::status_color(status, theme)));
    }
    row
}

