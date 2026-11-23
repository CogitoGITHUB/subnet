use crate::color::ColorTheme;
use ratatui::style::Color;

/// Colors come only from the active style (text, dim, error, bg).
#[derive(Debug)]
pub struct ThemeDefinition {
    pub color_theme: ColorTheme,
}

/// The name is ignored: there is one theme, the active style.
pub fn resolve_or_load(_name: &str) -> Result<ThemeDefinition, std::convert::Infallible> {
    let mut color_theme = ColorTheme::default();
    crate::run_style::apply(&mut color_theme);
    Ok(ThemeDefinition {
        color_theme,
    })
}

pub(crate) fn is_dark_color(c: Color) -> bool {
    match c {
        Color::Rgb(r, g, b) => {
            let lum = 0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b);
            lum < 128.0
        }
        Color::White | Color::Gray | Color::LightYellow => false,
        _ => true,
    }
}
