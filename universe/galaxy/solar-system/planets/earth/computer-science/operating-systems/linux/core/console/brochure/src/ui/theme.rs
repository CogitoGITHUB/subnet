//! Colors come only from the console style roles.
use crate::roles;
use ratatui::style::Color;

fn is_light(color: Color) -> bool {
    let Color::Rgb(r, g, b) = color else {
        return false;
    };
    let lin = |c: u8| (c as f32 / 255.0).powf(2.2);
    0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b) > 0.35
}

#[derive(Debug, Clone)]
pub struct ColorTheme {
    pub name: String,
    pub accent: Color,
    pub link: Color,
    pub success: Color,
    pub header: Color,
    pub bg: Color,
    pub bg_dark: Color,
    pub text: Color,
    pub muted: Color,
    pub border: Color,
    pub warning: Color,
    pub code: Color,
    pub sky: Color,
    pub pink: Color,
    pub error: Color,
    pub selection: Color,
    pub code_bg: Color,
}

impl ColorTheme {
    pub fn roles() -> Self {
        let text = roles::text();
        let dim = roles::dim();
        let bg = roles::bg();
        Self {
            name: String::from("console"),
            accent: text,
            link: text,
            success: text,
            header: text,
            bg,
            bg_dark: bg,
            text,
            muted: dim,
            border: dim,
            warning: text,
            code: text,
            sky: dim,
            pink: dim,
            error: roles::err(),
            selection: dim,
            code_bg: bg,
        }
    }

    pub fn category_colors(&self) -> [Color; 8] {
        [
            self.text, self.muted, self.text, self.muted,
            self.text, self.muted, self.text, self.muted,
        ]
    }

    pub fn contrast_color(&self, bg_color: Color) -> Color {
        if is_light(bg_color) {
            self.bg_dark
        } else {
            self.bg
        }
    }

    pub fn syntax_theme(&self) -> &'static str {
        if is_light(self.bg) {
            "InspiredGitHub"
        } else {
            "base16-ocean.dark"
        }
    }
}
