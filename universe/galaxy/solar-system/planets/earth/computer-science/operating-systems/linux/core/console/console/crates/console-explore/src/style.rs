//! Conversions between shell styles and ratatui styles, shared with `console-tui`.
//!
//! `explore` and the `tui` commands both draw with ratatui, so the mapping
//! from `$env.config.color_config`, `LS_COLORS`, and ANSI escapes lives here
//! once.

pub use crate::explore::shell_common::{create_lscolors, get_path_style};
pub use crate::explore::views::colored_text_widget::style_to_tui as ansi_style_to_tui;
pub use crate::explore::views::util::{
    shell_ansi_color_to_tui_color, shell_style_to_tui, text_style_to_tui_style,
};
