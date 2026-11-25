
use crate::screen::colors::{
    Colorscheme, GeneralColorscheme, HelpColorscheme, InputColorscheme, ModeColorscheme, PreviewColorscheme, ResultsColorscheme,
};
use ratatui::style::Color as RatatuiColor;


pub mod run;

#[derive(Clone, Debug, PartialEq)]
pub enum Color {
    Ansi(ANSIColor),
    Rgb(RGBColor),
}

impl Color {
    pub fn from_str(s: &str) -> Option<Self> {
        if s.starts_with('#') {
            RGBColor::from_str(s).map(Self::Rgb)
        } else {
            match s.to_lowercase().as_str() {
                "black" => Some(Self::Ansi(ANSIColor::Black)),
                "red" => Some(Self::Ansi(ANSIColor::Red)),
                "green" => Some(Self::Ansi(ANSIColor::Green)),
                "yellow" => Some(Self::Ansi(ANSIColor::Yellow)),
                "blue" => Some(Self::Ansi(ANSIColor::Blue)),
                "magenta" => Some(Self::Ansi(ANSIColor::Magenta)),
                "cyan" => Some(Self::Ansi(ANSIColor::Cyan)),
                "white" => Some(Self::Ansi(ANSIColor::White)),
                "bright-black" => Some(Self::Ansi(ANSIColor::BrightBlack)),
                "bright-red" => Some(Self::Ansi(ANSIColor::BrightRed)),
                "bright-green" => Some(Self::Ansi(ANSIColor::BrightGreen)),
                "bright-yellow" => Some(Self::Ansi(ANSIColor::BrightYellow)),
                "bright-blue" => Some(Self::Ansi(ANSIColor::BrightBlue)),
                "bright-magenta" => Some(Self::Ansi(ANSIColor::BrightMagenta)),
                "bright-cyan" => Some(Self::Ansi(ANSIColor::BrightCyan)),
                "bright-white" => Some(Self::Ansi(ANSIColor::BrightWhite)),
                _ => None,
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ANSIColor {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    BrightBlack,
    BrightRed,
    BrightGreen,
    BrightYellow,
    BrightBlue,
    BrightMagenta,
    BrightCyan,
    BrightWhite,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RGBColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl RGBColor {
    pub fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        let s = s.trim_start_matches('#');
        let r = u8::from_str_radix(&s[0..2], 16).ok()?;
        let g = u8::from_str_radix(&s[2..4], 16).ok()?;
        let b = u8::from_str_radix(&s[4..6], 16).ok()?;
        Some(Self { r, g, b })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    // general
    pub background: Option<Color>,
    pub border_fg: Color,
    pub text_fg: Color,
    pub dimmed_text_fg: Color,
    // input
    pub input_text_fg: Color,
    pub result_count_fg: Color,
    // results
    pub result_name_fg: Color,
    pub result_line_number_fg: Color,
    pub result_value_fg: Color,
    pub selection_bg: Color,
    pub selection_fg: Color,
    pub match_fg: Color,
    // preview
    pub preview_title_fg: Color,
    // modes
    pub channel_mode_fg: Color,
    pub channel_mode_bg: Color,
    pub remote_control_mode_fg: Color,
    pub remote_control_mode_bg: Color,
    pub action_picker_mode_fg: Color,
    pub action_picker_mode_bg: Color,
}

impl Theme {
    pub fn from_style() -> Self {
        run::apply(Self::default())
    }
}

impl Default for Theme {
    fn default() -> Self {
        let w = Color::Ansi(ANSIColor::White);
        let k = Color::Ansi(ANSIColor::Black);
        let d = Color::Ansi(ANSIColor::BrightBlack);
        Self {
            background: None,
            border_fg: d.clone(),
            text_fg: w.clone(),
            dimmed_text_fg: d.clone(),
            input_text_fg: w.clone(),
            result_count_fg: w.clone(),
            result_name_fg: w.clone(),
            result_line_number_fg: d.clone(),
            result_value_fg: w.clone(),
            selection_bg: w.clone(),
            selection_fg: k.clone(),
            match_fg: w.clone(),
            preview_title_fg: w.clone(),
            channel_mode_fg: k.clone(),
            channel_mode_bg: w.clone(),
            remote_control_mode_fg: k.clone(),
            remote_control_mode_bg: w.clone(),
            action_picker_mode_fg: k.clone(),
            action_picker_mode_bg: w.clone(),
        }
    }
}

#[allow(clippy::from_over_into)]
impl Into<RatatuiColor> for &RGBColor {
    fn into(self) -> RatatuiColor {
        RatatuiColor::Rgb(self.r, self.g, self.b)
    }
}

#[allow(clippy::from_over_into)]
impl Into<RatatuiColor> for &ANSIColor {
    fn into(self) -> RatatuiColor {
        match self {
            ANSIColor::Black => RatatuiColor::Black,
            ANSIColor::Red => RatatuiColor::Red,
            ANSIColor::Green => RatatuiColor::Green,
            ANSIColor::Yellow => RatatuiColor::Yellow,
            ANSIColor::Blue => RatatuiColor::Blue,
            ANSIColor::Magenta => RatatuiColor::Magenta,
            ANSIColor::Cyan => RatatuiColor::Cyan,
            ANSIColor::White => RatatuiColor::Gray,
            ANSIColor::BrightBlack => RatatuiColor::DarkGray,
            ANSIColor::BrightRed => RatatuiColor::LightRed,
            ANSIColor::BrightGreen => RatatuiColor::LightGreen,
            ANSIColor::BrightYellow => RatatuiColor::LightYellow,
            ANSIColor::BrightBlue => RatatuiColor::LightBlue,
            ANSIColor::BrightMagenta => RatatuiColor::LightMagenta,
            ANSIColor::BrightCyan => RatatuiColor::LightCyan,
            ANSIColor::BrightWhite => RatatuiColor::White,
        }
    }
}

#[allow(clippy::from_over_into)]
impl Into<RatatuiColor> for &Color {
    fn into(self) -> RatatuiColor {
        match self {
            Color::Ansi(ansi) => ansi.into(),
            Color::Rgb(rgb) => rgb.into(),
        }
    }
}

#[allow(clippy::from_over_into)]
impl Into<Colorscheme> for &Theme {
    fn into(self) -> Colorscheme {
        Colorscheme {
            general: self.into(),
            help: self.into(),
            results: self.into(),
            preview: self.into(),
            input: self.into(),
            mode: self.into(),
        }
    }
}

#[allow(clippy::from_over_into)]
impl Into<GeneralColorscheme> for &Theme {
    fn into(self) -> GeneralColorscheme {
        GeneralColorscheme {
            background: self.background.as_ref().map(Into::into),
            border_fg: (&self.border_fg).into(),
        }
    }
}

#[allow(clippy::from_over_into)]
impl Into<HelpColorscheme> for &Theme {
    fn into(self) -> HelpColorscheme {
        HelpColorscheme {
            metadata_field_name_fg: (&self.dimmed_text_fg).into(),
            metadata_field_value_fg: (&self.text_fg).into(),
        }
    }
}

#[allow(clippy::from_over_into)]
impl Into<ResultsColorscheme> for &Theme {
    fn into(self) -> ResultsColorscheme {
        ResultsColorscheme {
            result_fg: (&self.result_name_fg).into(),
            result_selected_bg: (&self.selection_bg).into(),
            result_selected_fg: (&self.selection_fg).into(),
            match_foreground_color: (&self.match_fg).into(),
        }
    }
}

#[allow(clippy::from_over_into)]
impl Into<PreviewColorscheme> for &Theme {
    fn into(self) -> PreviewColorscheme {
        PreviewColorscheme {
            title_fg: (&self.preview_title_fg).into(),
            highlight_bg: (&self.selection_bg).into(),
            content_fg: (&self.text_fg).into(),
            gutter_fg: (&self.dimmed_text_fg).into(),
            gutter_selected_fg: (&self.match_fg).into(),
        }
    }
}

#[allow(clippy::from_over_into)]
impl Into<InputColorscheme> for &Theme {
    fn into(self) -> InputColorscheme {
        InputColorscheme {
            input_fg: (&self.input_text_fg).into(),
            results_count_fg: (&self.result_count_fg).into(),
        }
    }
}

#[allow(clippy::from_over_into)]
impl Into<ModeColorscheme> for &Theme {
    fn into(self) -> ModeColorscheme {
        ModeColorscheme {
            channel: (&self.channel_mode_bg).into(),
            channel_fg: (&self.channel_mode_fg).into(),
            remote_control: (&self.remote_control_mode_bg).into(),
            remote_control_fg: (&self.remote_control_mode_fg).into(),
            action_picker: (&self.action_picker_mode_bg).into(),
            action_picker_fg: (&self.action_picker_mode_fg).into(),
        }
    }
}


/// Role colors for widgets that have no Theme field.
pub fn role_text() -> RatatuiColor {
    run::text_color()
}

pub fn role_error() -> RatatuiColor {
    run::error_color()
}
