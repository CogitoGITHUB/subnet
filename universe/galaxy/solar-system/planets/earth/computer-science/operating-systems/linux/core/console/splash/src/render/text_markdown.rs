use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    widgets::{Paragraph, Wrap},
};
use tui_markdown::{Options as MdOptions, StyleSheet};

use crate::options::OptionSchema;
use crate::payload::Body;
use crate::theme::{self, ColorKey, Theme};

use super::{Registry, RenderOptions, Renderer, Shape};

const COLOR_KEYS: &[ColorKey] = &[
    theme::TEXT,
    theme::PANEL_TITLE,
    theme::ACCENT_EVENT,
    theme::TEXT_SECONDARY,
    theme::TEXT_DIM,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "align",
    type_hint: "\"left\" | \"center\" | \"right\"",
    required: false,
    default: Some("\"left\""),
    description: "Horizontal alignment of the rendered markdown within its cell.",
}];

/// Markdown renderer wrapping the `tui-markdown` crate. Accepts only `MarkdownTextBlock`
/// — a fetcher emitting that shape is committing to "this string is Markdown source",
/// which is a different semantic claim from `Text` ("this string should be drawn as-is")
/// and `TextBlock` ("these are independent lines"). All styled spans (headings, code,
/// links, blockquote) come from the splashboard `Theme` via [`SplashStyleSheet`] so the
/// markdown reads as one place with the rest of the splash. Syntax highlighting via
/// syntect is intentionally disabled (`tui-markdown` `default-features = false`) to keep
/// fenced code blocks on the same theme tokens.
pub struct TextMarkdownRenderer;

impl Renderer for TextMarkdownRenderer {
    fn name(&self) -> &str {
        "text_markdown"
    }
    fn description(&self) -> &'static str {
        "Wrapped markdown prose with themed headings, inline code, links, and blockquotes. Pick this when the source string carries Markdown syntax that should render as styled spans rather than literal characters."
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::MarkdownTextBlock]
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn color_keys(&self) -> &[ColorKey] {
        COLOR_KEYS
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
        let Body::MarkdownTextBlock(d) = body else {
            return;
        };
        let p = Paragraph::new(parse_markdown(&d.value, theme))
            .style(Style::default().fg(theme.text))
            .alignment(parse_align(opts.align.as_deref()))
            .wrap(Wrap { trim: false });
        frame.render_widget(p, area);
    }
    fn natural_height(
        &self,
        body: &Body,
        _opts: &RenderOptions,
        max_width: u16,
        _registry: &Registry,
    ) -> u16 {
        let Body::MarkdownTextBlock(d) = body else {
            return 1;
        };
        wrapped_line_count(&parse_markdown(&d.value, &Theme::default()), max_width)
    }
}

fn parse_markdown<'a>(source: &'a str, theme: &Theme) -> ratatui::text::Text<'a> {
    let opts = MdOptions::new(SplashStyleSheet::from(theme));
    tui_markdown::from_str_with_options(source, &opts)
}

/// Maps `tui-markdown`'s six style hooks onto splashboard theme tokens.
///
/// - `heading(1..=3)` → `panel_title` (the coral hero) with bold / italic gradient so H1
///   reads loudest. H4-H6 fall back to `text_secondary` italic — section breaks rather
///   than headlines.
/// - `code()` → `accent_event` (cyan-teal). One token covers both inline code spans and
///   fenced code blocks (tui-markdown shares the hook); cyan-teal sits opposite the coral
///   heading so code is visually distinct without competing.
/// - `link()` → `accent_event` + underlined.
/// - `blockquote()` → `text_secondary` italic — quoted material stays muted.
/// - `heading_meta()` / `metadata_block()` → `text_dim`.
#[derive(Debug, Clone)]
struct SplashStyleSheet {
    heading: Color,
    code: Color,
    link: Color,
    secondary: Color,
    dim: Color,
}

impl SplashStyleSheet {
    fn from(theme: &Theme) -> Self {
        Self {
            heading: theme.panel_title,
            code: theme.accent_event,
            link: theme.accent_event,
            secondary: theme.text_secondary,
            dim: theme.text_dim,
        }
    }
}

impl StyleSheet for SplashStyleSheet {
    fn heading(&self, level: u8) -> Style {
        match level {
            1 => Style::default()
                .fg(self.heading)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            2 => Style::default()
                .fg(self.heading)
                .add_modifier(Modifier::BOLD),
            3 => Style::default()
                .fg(self.heading)
                .add_modifier(Modifier::BOLD | Modifier::ITALIC),
            _ => Style::default()
                .fg(self.secondary)
                .add_modifier(Modifier::ITALIC),
        }
    }
    fn code(&self) -> Style {
        Style::default().fg(self.code)
    }
    fn link(&self) -> Style {
        Style::default()
            .fg(self.link)
            .add_modifier(Modifier::UNDERLINED)
    }
    fn blockquote(&self) -> Style {
        Style::default()
            .fg(self.secondary)
            .add_modifier(Modifier::ITALIC)
    }
    fn heading_meta(&self) -> Style {
        Style::default().fg(self.dim)
    }
    fn metadata_block(&self) -> Style {
        Style::default().fg(self.dim)
    }
}

fn wrapped_line_count(text: &ratatui::text::Text<'_>, width: u16) -> u16 {
    let w = width.max(1) as usize;
    text.lines
        .iter()
        .map(|line| {
            let cells: usize = line
                .spans
                .iter()
                .map(|span| span.content.chars().count())
                .sum();
            cells.max(1).div_ceil(w)
        })
        .sum::<usize>()
        .clamp(1, u16::MAX as usize) as u16
}

fn parse_align(s: Option<&str>) -> Alignment {
    match s {
        Some("center") => Alignment::Center,
        Some("right") => Alignment::Right,
        _ => Alignment::Left,
    }
}

