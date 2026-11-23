use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::Style,
    widgets::Paragraph,
};

use crate::options::OptionSchema;
use crate::payload::{Body, TextBlockData};
use crate::theme::{self, ColorKey, Theme};

use super::{Registry, RenderOptions, Renderer, Shape};

const COLOR_KEYS: &[ColorKey] = &[theme::TEXT];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "bullet",
        type_hint: "\"•\" | \"●\" | \"▪\" | \"→\" | \"none\"",
        required: false,
        default: Some("none"),
        description: "Glyph prefixed to each entry. `none` (default) leaves the text unbulleted; any other string is rendered verbatim followed by a single space.",
    },
    OptionSchema {
        name: "max_items",
        type_hint: "positive integer",
        required: false,
        default: Some("all lines"),
        description: "Cap on rendered entries. Truncates from the end when the block has more lines than the cap.",
    },
    OptionSchema {
        name: "align",
        type_hint: "\"left\" | \"center\" | \"right\"",
        required: false,
        default: Some("\"left\""),
        description: "Horizontal alignment of the whole list within its cell.",
    },
];

/// Multi-line `TextBlock` renderer. Uses ratatui's `Paragraph` so `align = "center"` centres
/// each line individually (matters when entries are very different lengths — e.g. a 3-char
/// "MIT" line in the same block as a 50-char description). Adds list-flavoured options on top:
/// `bullet` glyph prefix and `max_items` cap.
pub struct ListPlainRenderer;

impl Renderer for ListPlainRenderer {
    fn name(&self) -> &str {
        "list_plain"
    }
    fn description(&self) -> &'static str {
        "One line per entry, optionally prefixed with a bullet glyph and capped by `max_items`. Sibling to `text_plain` for `TextBlock` — pick this when you want list semantics (bullet marker, item cap) instead of a paragraph."
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::TextBlock]
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
        if let Body::TextBlock(d) = body {
            render_block(frame, area, d, opts, theme);
        }
    }
    fn natural_height(
        &self,
        body: &Body,
        opts: &RenderOptions,
        _max_width: u16,
        _registry: &Registry,
    ) -> u16 {
        let lines = match body {
            Body::TextBlock(d) => d
                .lines
                .len()
                .min(opts.max_items.unwrap_or(usize::MAX))
                .max(1),
            _ => 1,
        };
        u16::try_from(lines).unwrap_or(u16::MAX)
    }
}

fn render_block(
    frame: &mut Frame,
    area: Rect,
    data: &TextBlockData,
    opts: &RenderOptions,
    theme: &Theme,
) {
    let prefix = bullet_prefix(opts.bullet.as_deref());
    let content = data
        .lines
        .iter()
        .take(opts.max_items.unwrap_or(usize::MAX))
        .map(|l| format!("{prefix}{l}"))
        .collect::<Vec<_>>()
        .join("\n");
    let p = Paragraph::new(content)
        .style(Style::default().fg(theme.text))
        .alignment(parse_align(opts.align.as_deref()));
    frame.render_widget(p, area);
}

fn parse_align(s: Option<&str>) -> Alignment {
    match s {
        Some("center") => Alignment::Center,
        Some("right") => Alignment::Right,
        _ => Alignment::Left,
    }
}

fn bullet_prefix(bullet: Option<&str>) -> String {
    match bullet {
        None | Some("none") | Some("") => String::new(),
        Some(g) => format!("{g} "),
    }
}

