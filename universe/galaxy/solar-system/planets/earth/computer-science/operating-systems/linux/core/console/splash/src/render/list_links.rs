//! `list_links` — `LinkedTextBlock` rendered one row per line, wrapped in OSC 8 hyperlinks where
//! the row carries a `url`. Rows without a url render as plain text.
//!
//! How the OSC 8 plumbing works: ratatui's `Buffer` stores per-cell symbol strings that the
//! crossterm backend prints verbatim. We collapse the whole linked row into the first cell's
//! symbol — `ESC ] 8 ; ; URL ESC \\ <text> ESC ] 8 ; ; ESC \\` — pin its diff width to 1 via
//! `CellDiffOption::ForcedWidth(1)`, and mark the trailing cells with `CellDiffOption::Skip` so
//! ratatui's diff doesn't try to redraw them. The `ForcedWidth(1)` matters because ratatui 0.30's
//! diff iterator advances by the cell's computed `cell_width` for non-skip cells; without the
//! pin, the printable characters inside the escape sequence (URL, visible text) would be counted
//! as visible columns and the iterator would jump far past the trailing cells — even spilling
//! into the next row — so subsequent rows would never be diffed and would render blank.
//!
//! The `skip = true` trick only buys real-terminal correctness: `TestBackend` doesn't
//! simulate the terminal's char-rendering of the OSC 8 visible portion, so the skipped cells
//! stay at their default empty state in its buffer. The runtime's off-screen scrollback
//! capture (`render_full_into_buffer`) targets `TestBackend` and would inherit that broken
//! state — we wrap that draw in [`without_osc8`] so the renderer emits plain text for the
//! capture, while the on-screen `CrosstermBackend` pass keeps the OSC 8 wrap.

use std::cell::Cell as StdCell;
use std::num::NonZeroU16;

use ratatui::{Frame, buffer::Buffer, buffer::CellDiffOption, layout::Rect, style::Style};

use crate::options::OptionSchema;
use crate::payload::{Body, LinkedLine, LinkedTextBlockData};
use crate::theme::{self, ColorKey, Theme};

use super::{Registry, RenderOptions, Renderer, Shape};

const COLOR_KEYS: &[ColorKey] = &[theme::TEXT];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "max_items",
    type_hint: "positive integer",
    required: false,
    default: Some("all rows"),
    description: "Cap on rendered rows. Truncates from the end when the input has more rows than the cap.",
}];

/// Renders a `LinkedTextBlock` body one row per line, wrapping rows whose `url` is `Some(_)` in
/// an OSC 8 hyperlink so modern terminals (iTerm2, kitty, WezTerm, recent Windows Terminal, etc.)
/// surface the row as a clickable link. Rows without a url render as plain text. Pairs with feed
/// fetchers (`hackernews_top`, `github_my_prs`, `github_recent_releases`, …) where each row has a
/// canonical "open this" target.
pub struct ListLinksRenderer;

impl Renderer for ListLinksRenderer {
    fn name(&self) -> &str {
        "list_links"
    }
    fn description(&self) -> &'static str {
        "One row per entry, with rows that carry a URL wrapped in OSC-8 hyperlinks so modern terminals make them clickable. Use this for feed-style payloads (HN headlines, recent PRs, releases) where each row has a canonical destination."
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::LinkedTextBlock]
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
        if let Body::LinkedTextBlock(d) = body {
            render_links(frame, area, d, opts, theme);
        }
    }
}

fn render_links(
    frame: &mut Frame,
    area: Rect,
    data: &LinkedTextBlockData,
    opts: &RenderOptions,
    theme: &Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let cap = opts.max_items.unwrap_or(usize::MAX);
    let style = Style::default().fg(theme.text);
    let buf = frame.buffer_mut();
    data.items
        .iter()
        .take(cap)
        .take(area.height as usize)
        .enumerate()
        .for_each(|(i, line)| {
            let y = area.y + i as u16;
            write_row(buf, area.x, y, area.width, line, style);
        });
}

fn write_row(buf: &mut Buffer, x: u16, y: u16, max_width: u16, line: &LinkedLine, style: Style) {
    let (end_x, _) = buf.set_stringn(x, y, &line.text, max_width as usize, style);
    let drawn = end_x.saturating_sub(x);
    if drawn == 0 {
        return;
    }
    if osc8_suppressed() {
        return;
    }
    if let Some(url) = line.url.as_deref() {
        wrap_link(buf, x, y, end_x, url, style);
    }
}

thread_local! {
    /// Suppresses the OSC 8 wrap inside the closure passed to [`without_osc8`]. The runtime's
    /// off-screen capture path (`render_full_into_buffer`) targets `TestBackend`, which holds
    /// raw cell symbols verbatim and doesn't interpret escape sequences — putting the OSC 8
    /// wrap there would only pollute the buffer that later gets sliced into the inline
    /// scrollback. The on-screen `CrosstermBackend` pass runs without this flag so user-facing
    /// frames keep the clickable hyperlinks.
    static OSC8_SUPPRESSED: StdCell<bool> = const { StdCell::new(false) };
}

/// Run `f` with OSC 8 hyperlink wrapping disabled. Idempotent and re-entrant: nested calls
/// stack the suppression and restore the prior value on exit. Panic-safe: an RAII guard
/// restores the prior value via `Drop`, so a panic inside `f` doesn't leak the suppressed
/// state into subsequent unrelated draws on the same thread.
pub fn without_osc8<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    let _guard = SuppressionGuard::engage();
    f()
}

struct SuppressionGuard {
    prior: bool,
}

impl SuppressionGuard {
    fn engage() -> Self {
        let prior = OSC8_SUPPRESSED.with(|cell| cell.replace(true));
        Self { prior }
    }
}

impl Drop for SuppressionGuard {
    fn drop(&mut self) {
        OSC8_SUPPRESSED.with(|cell| cell.set(self.prior));
    }
}

fn osc8_suppressed() -> bool {
    OSC8_SUPPRESSED.with(StdCell::get)
}

/// Re-export of the OSC 8 wrapping helper so sibling renderers (`list_cards`) can reuse the
/// diff-option plumbing without duplicating the escape-sequence dance.
pub(crate) fn wrap_osc8(buf: &mut Buffer, x: u16, y: u16, end_x: u16, url: &str, style: Style) {
    if osc8_suppressed() {
        return;
    }
    wrap_link(buf, x, y, end_x, url, style);
}

const FORCED_WIDTH_1: CellDiffOption = CellDiffOption::ForcedWidth(NonZeroU16::new(1).unwrap());

fn wrap_link(buf: &mut Buffer, x: u16, y: u16, end_x: u16, url: &str, style: Style) {
    let visible: String = (x..end_x)
        .filter_map(|col| buf.cell((col, y)).map(|c| c.symbol().to_string()))
        .collect();
    if let Some(cell) = buf.cell_mut((x, y)) {
        cell.set_symbol(&format!("\x1b]8;;{url}\x1b\\{visible}\x1b]8;;\x1b\\"));
        cell.set_style(style);
        cell.set_diff_option(FORCED_WIDTH_1);
    }
    for col in (x + 1)..end_x {
        if let Some(cell) = buf.cell_mut((col, y)) {
            cell.set_diff_option(CellDiffOption::Skip);
        }
    }
}

