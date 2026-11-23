//! Colors and border lines for the `tui` widgets, read from `$env.config.tui`.
//!
//! Every color slot is a `color_config`-style value (a color name, `#RRGGBB`,
//! or `{fg, bg, attr}`); see `default_tui` in console-protocol for the defaults.
//! `border_type` names the `table --theme` whose outline borders draw.
//! Table cells follow `color_config` per value type and `LS_COLORS` for path
//! columns, through the converters shared with `explore`.

use lscolors::LsColors;
use console_color_config::{StyleComputer, get_color_map};
use console_explore::style::{
    get_path_style, shell_ansi_color_to_tui_color, shell_style_to_tui, text_style_to_tui_style,
};
use console_protocol::{
    ShellError, TableMode, Value,
    engine::{EngineState, Stack},
    shell_error::generic::GenericError,
};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;
use std::path::Path;

/// The border of widgets without their own `--border` when
/// `$env.config.tui.border_type` is unset: the plain ratatui lines.
const DEFAULT_BORDER: TableMode = TableMode::Single;

/// Read a `--border` flag: one of [`TableMode::tui_border_names`].
/// `$env.config.tui.border_type` is checked the same way when it is set.
pub fn parse_border(value: &Value) -> Result<TableMode, ShellError> {
    let span = value.span();
    let name = value.as_str()?;
    if let Some(mode) = TableMode::from_tui_border(name) {
        return Ok(mode);
    }
    if name.eq_ignore_ascii_case("none") {
        return Err(ShellError::Generic(
            GenericError::new(
                "`none` is not a tui border",
                "a tui border always takes one cell",
                span,
            )
            .with_help("use `frameless` for a border that draws nothing"),
        ));
    }
    Err(ShellError::InvalidValue {
        valid: TableMode::tui_border_names().collect::<Vec<_>>().join(", "),
        actual: name.to_string(),
        span,
    })
}

/// The closest outline a `table --theme` draws, as ratatui border glyphs.
/// A table line with no sides (`compact`, `with_love`) runs through the
/// corners with blank sides, and a theme whose only lines are inner ones
/// (`light`, `psql`, `frameless`) is blank. Every glyph is one cell wide.
fn outline(mode: TableMode) -> border::Set<'static> {
    let custom = |corners: [&'static str; 4], vertical, horizontal| border::Set {
        top_left: corners[0],
        top_right: corners[1],
        bottom_left: corners[2],
        bottom_right: corners[3],
        vertical_left: vertical,
        vertical_right: vertical,
        horizontal_top: horizontal,
        horizontal_bottom: horizontal,
    };
    match mode {
        TableMode::Thin | TableMode::Single => border::PLAIN,
        TableMode::Rounded => border::ROUNDED,
        TableMode::Heavy => border::THICK,
        TableMode::Double => border::DOUBLE,
        TableMode::Reinforced => border::Set {
            top_left: "┏",
            top_right: "┓",
            bottom_left: "┗",
            bottom_right: "┛",
            ..border::PLAIN
        },
        TableMode::Basic | TableMode::BasicCompact => custom(["+"; 4], "|", "-"),
        TableMode::AsciiRounded => custom([".", ".", "'", "'"], "|", "-"),
        TableMode::Dots => custom([".", ".", ":", ":"], ":", "."),
        TableMode::WithLove => custom(["❤"; 4], " ", "❤"),
        TableMode::Markdown => custom(["|"; 4], "|", " "),
        TableMode::Compact => custom(["─"; 4], " ", "─"),
        TableMode::CompactDouble => custom(["═"; 4], " ", "═"),
        TableMode::Restructured => custom(["="; 4], " ", "="),
        TableMode::Light | TableMode::Psql | TableMode::Frameless | TableMode::None => {
            custom([" "; 4], " ", " ")
        }
    }
}

/// A `$env.config.tui` color as a ratatui color. `default` is the terminal's
/// own color (`Color::Reset`); the converter shared with `explore` reads it
/// as "no color", which would let the surface show through instead.
fn config_color(color: nu_ansi_term::Color) -> Color {
    shell_ansi_color_to_tui_color(color).unwrap_or(Color::Reset)
}

/// A `$env.config.tui` style as a ratatui style, reading `default` the way
/// [`config_color`] does.
fn config_style(style: nu_ansi_term::Style) -> Style {
    Style {
        fg: style.foreground.map(config_color),
        bg: style.background.map(config_color),
        ..shell_style_to_tui(style)
    }
}

#[derive(Debug, Clone)]
pub struct Theme {
    /// Fill behind every widget. The default, `Color::Reset`, is the
    /// terminal's own background, so text keeps the terminal's colors on
    /// light and dark terminals alike. Unused when `colored` is false.
    pub surface: Color,
    /// Full-screen fill behind a dialog. Unused when `colored` is false.
    pub backdrop: Color,
    /// `false` when `use_ansi_coloring` is off: no colors, only reverse
    /// video for the selection.
    pub colored: bool,
    title: Style,
    status: Style,
    border: Style,
    border_focused: Style,
    selected: Style,
    header: Style,
    muted: Style,
    highlight: Style,
    tab_active: Style,
    tab_inactive: Style,
    progress: Style,
    button: Style,
    /// `$env.config.tui.border_type`: the border of widgets without their
    /// own `--border`.
    border_type: TableMode,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            surface: Color::Reset,
            backdrop: Color::Reset,
            colored: true,
            title: Style::default()
                .fg(crate::roles::text())
                .bg(crate::roles::dim())
                .add_modifier(Modifier::BOLD),
            status: Style::default().fg(crate::roles::text()).bg(crate::roles::dim()),
            border: Style::default().fg(crate::roles::dim()),
            border_focused: Style::default().fg(crate::roles::text()),
            selected: Style::default().add_modifier(Modifier::REVERSED),
            header: Style::default()
                .fg(crate::roles::text())
                .add_modifier(Modifier::BOLD),
            muted: Style::default().fg(crate::roles::dim()),
            highlight: Style::default()
                .fg(crate::roles::text())
                .add_modifier(Modifier::BOLD),
            tab_active: Style::default()
                .fg(crate::roles::text())
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            tab_inactive: Style::default().fg(crate::roles::dim()),
            progress: Style::default().fg(crate::roles::text()),
            button: Style::default().fg(crate::roles::text()).bg(crate::roles::dim()),
            border_type: DEFAULT_BORDER,
        }
    }
}

impl Theme {
    /// No colors at all, for `use_ansi_coloring = false` or `NO_COLOR`.
    pub fn plain() -> Self {
        Self {
            surface: Color::Reset,
            backdrop: Color::Reset,
            colored: false,
            title: Style::default().add_modifier(Modifier::BOLD),
            status: Style::default(),
            border: Style::default(),
            border_focused: Style::default().add_modifier(Modifier::BOLD),
            selected: Style::default().add_modifier(Modifier::REVERSED),
            header: Style::default().add_modifier(Modifier::BOLD),
            muted: Style::default().add_modifier(Modifier::DIM),
            highlight: Style::default().add_modifier(Modifier::BOLD),
            tab_active: Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            tab_inactive: Style::default(),
            progress: Style::default(),
            button: Style::default().add_modifier(Modifier::REVERSED),
            border_type: DEFAULT_BORDER,
        }
    }

    /// Read `$env.config.tui`. Slots missing from the config, or holding a
    /// value that is not a color or border, keep the built-in default.
    pub fn from_config(engine_state: &EngineState, stack: &Stack) -> Self {
        let config = stack.get_config(engine_state);
        let border_type = config
            .tui
            .get("border_type")
            .and_then(|value| value.as_str().ok())
            .and_then(TableMode::from_tui_border)
            .unwrap_or(DEFAULT_BORDER);
        if !config.use_ansi_coloring.get(engine_state) {
            return Self {
                border_type,
                ..Self::plain()
            };
        }
        let colors = get_color_map(&config.tui);
        let mut theme = Self {
            border_type,
            ..Self::default()
        };
        let slot = |name: &str, target: &mut Style| {
            if let Some(style) = colors.get(name) {
                *target = config_style(*style);
            }
        };
        slot("title_bar", &mut theme.title);
        slot("status_bar", &mut theme.status);
        slot("border", &mut theme.border);
        slot("border_focused", &mut theme.border_focused);
        slot("selected", &mut theme.selected);
        slot("header", &mut theme.header);
        slot("muted", &mut theme.muted);
        slot("highlight", &mut theme.highlight);
        slot("tab_active", &mut theme.tab_active);
        slot("tab_inactive", &mut theme.tab_inactive);
        slot("progress", &mut theme.progress);
        slot("button", &mut theme.button);
        // A fill is one color: a record's `bg`, or a bare color
        // (`surface: default`), which parses as a foreground.
        let fill = |name: &str, target: &mut Color| {
            let style = colors.get(name);
            let color = match config.tui.get(name) {
                Some(Value::String { .. }) => style.and_then(|s| s.foreground),
                _ => style.and_then(|s| s.background),
            };
            if let Some(color) = color {
                *target = config_color(color);
            }
        };
        fill("surface", &mut theme.surface);
        fill("backdrop", &mut theme.backdrop);
        theme
    }

    /// A style painted over the surface: the slot's colors win, the surface
    /// fills in a missing background.
    fn on_surface(&self, style: Style) -> Style {
        self.surface().patch(style)
    }

    pub fn surface(&self) -> Style {
        if self.colored {
            Style::default().bg(self.surface)
        } else {
            Style::default()
        }
    }

    pub fn backdrop(&self) -> Style {
        if self.colored {
            Style::default().bg(self.backdrop).patch(self.muted)
        } else {
            self.muted
        }
    }

    pub fn title(&self) -> Style {
        self.on_surface(self.title)
    }

    pub fn status(&self) -> Style {
        self.on_surface(self.status)
    }

    pub fn border(&self, focused: bool) -> Style {
        self.on_surface(if focused {
            self.border_focused
        } else {
            self.border
        })
    }

    /// Border glyphs for a widget: its own `--border`, or the configured
    /// `border_type`.
    pub fn border_set(&self, own: Option<TableMode>) -> border::Set<'static> {
        outline(own.unwrap_or(self.border_type))
    }

    pub fn selected(&self) -> Style {
        self.selected
    }

    pub fn text(&self) -> Style {
        self.surface()
    }

    pub fn muted(&self) -> Style {
        self.on_surface(self.muted)
    }

    pub fn highlight(&self) -> Style {
        self.on_surface(self.highlight)
    }

    pub fn header(&self) -> Style {
        self.on_surface(self.header)
    }

    pub fn tab(&self, active: bool) -> Style {
        self.on_surface(if active {
            self.tab_active
        } else {
            self.tab_inactive
        })
    }

    pub fn progress(&self) -> Style {
        self.on_surface(self.progress)
    }

    pub fn button(&self, focused: bool) -> Style {
        let base = self.on_surface(self.button);
        if focused {
            base.patch(self.selected)
        } else {
            base
        }
    }

    /// Style a cell from `color_config` for that value's type (`int`,
    /// `filesize`, …).
    pub fn value_cell(&self, value: &Value, computer: &StyleComputer) -> Style {
        if !self.colored {
            return self.text();
        }
        self.on_surface(text_style_to_tui_style(computer.style_primitive(value)))
    }

    /// Style a filesystem path with `LS_COLORS`. Regular files with `fi=0`
    /// get no foreground, the same rule `table` and `explore` use.
    pub fn path_cell(&self, path: &str, cwd: &Path, ls_colors: &LsColors) -> Style {
        if !self.colored {
            return self.text();
        }
        match get_path_style(path, &cwd.to_string_lossy(), ls_colors) {
            Some(style) => self.on_surface(shell_style_to_tui(style)),
            None => self.text(),
        }
    }
}

