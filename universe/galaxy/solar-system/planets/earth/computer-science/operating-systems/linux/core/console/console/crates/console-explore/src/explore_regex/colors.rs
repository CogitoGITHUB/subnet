//! Color scheme and style helpers for the regex explorer UI.

use ratatui::style::{Color, Modifier, Style};

use crate::roles;

pub fn accent() -> Color {
    roles::text()
}
pub fn success() -> Color {
    roles::text()
}
pub fn error() -> Color {
    roles::err()
}
pub fn warning() -> Color {
    roles::err()
}
pub fn bg_dark() -> Color {
    roles::bg()
}
pub fn fg_primary() -> Color {
    Color::Reset
}
pub fn fg_muted() -> Color {
    roles::dim()
}
pub fn fg_light() -> Color {
    roles::text()
}

/// Match group style: cycles over the roles only.
pub fn highlight_style(group: usize) -> Style {
    match group % 3 {
        0 => Style::new().bg(roles::text()).fg(roles::bg()),
        1 => Style::new().bg(roles::dim()).fg(roles::text()),
        _ => Style::new().bg(roles::err()).fg(roles::text()),
    }
}

/// Style presets for common UI elements
pub mod styles {
    use super::*;

    /// Style for focused/active elements
    pub fn focused() -> Style {
        Style::new().fg(fg_primary()).bold()
    }

    /// Style for unfocused/inactive elements
    pub fn unfocused() -> Style {
        Style::new().fg(fg_muted())
    }

    /// Style for the focus indicator ("> ")
    pub fn focus_indicator() -> Style {
        Style::new().fg(fg_primary())
    }

    /// Style for muted separator text
    pub fn separator() -> Style {
        Style::new().fg(fg_muted())
    }

    /// Style for status badges (the brackets)
    pub fn status_bracket() -> Style {
        Style::new().fg(fg_muted())
    }

    /// Style for success status text
    pub fn status_success() -> Style {
        Style::new().fg(success())
    }

    /// Style for error status text
    pub fn status_error() -> Style {
        Style::new().fg(error())
    }

    /// Style for warning status text
    pub fn status_warning() -> Style {
        Style::new().fg(warning())
    }

    /// Style for category headers in quick reference
    pub fn category_header() -> Style {
        Style::new()
            .fg(accent())
            .bold()
            .add_modifier(Modifier::UNDERLINED)
    }

    /// Style for selected item (inverted colors)
    pub fn selected() -> Style {
        Style::new().fg(bg_dark()).bg(accent())
    }

    /// Style for selected item text (bold variant)
    pub fn selected_bold() -> Style {
        selected().bold()
    }

    /// Border style for focused panels
    pub fn border_focused() -> Style {
        Style::new().fg(accent())
    }

    /// Border style for focused panels with error state
    pub fn border_error() -> Style {
        Style::new().fg(error())
    }

    /// Border style for unfocused panels
    pub fn border_unfocused() -> Style {
        Style::new().fg(fg_muted())
    }

    /// Style for modal help description text (brighter than separator)
    pub fn modal_desc() -> Style {
        Style::new().fg(fg_light())
    }
}
