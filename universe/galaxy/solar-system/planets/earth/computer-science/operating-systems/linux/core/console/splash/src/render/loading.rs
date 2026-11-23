//! Loading placeholder drawn for widgets that are fetchable but have no payload yet.
//!
//! Rendered directly from `layout::draw` instead of going through `render_payload`, so a widget
//! whose real payload hasn't landed still shows a consistent "still working" indicator rather
//! than a blank slot. Animates on the same cadence as animated renderers — the runtime's
//! `ANIMATION_WINDOW` gives the spinner at least a couple of full cycles even on a fast path.

use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::Line,
    widgets::Paragraph,
};

use crate::theme::Theme;

use super::Shape;

/// Braille spinner frames, 10 steps per full cycle. Advances on a ~100ms cadence, which lines
/// up with `FRAME_TICK` (50ms) so every other redraw shows a new frame.
const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Shape-agnostic entry point. For now every shape renders a centered spinner; per-shape
/// skeletons (dim rows for `Entries`, flat baseline for `NumberSeries`, empty grid for
/// `Heatmap`, …) can replace this in a follow-up without changing the call sites.
pub fn render_loading(frame: &mut Frame, area: Rect, _shape: Shape, theme: &Theme) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    render_spinner(frame, area, theme);
}

fn render_spinner(frame: &mut Frame, area: Rect, theme: &Theme) {
    let spinner = current_spinner_frame();
    let label = format!("{spinner} loading");
    let style = Style::default()
        .fg(theme.text_dim)
        .add_modifier(Modifier::ITALIC);
    let top_pad = area.height.saturating_sub(1) / 2;
    let inner = Rect {
        x: area.x,
        y: area.y + top_pad,
        width: area.width,
        height: 1,
    };
    let paragraph = Paragraph::new(Line::from(label).style(style)).alignment(Alignment::Center);
    frame.render_widget(paragraph, inner);
}

fn current_spinner_frame() -> &'static str {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let idx = ((millis / 100) as usize) % SPINNER_FRAMES.len();
    SPINNER_FRAMES[idx]
}

