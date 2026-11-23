use {
    crate::{
        app::AppContext,
        skin::roles::tm_theme_xml,
    },
    once_cell::sync::Lazy,
    std::path::Path,
    syntect::{
        easy::{
            HighlightLines,
            HighlightOptions,
        },
        highlighting::{
            Theme,
            ThemeSet,
        },
        parsing::SyntaxSet,
    },
};

static SYNTAXES: &[u8] = include_bytes!("../../resources/syntect/syntaxes.bin");

pub static SYNTAXER: Lazy<Syntaxer> = Lazy::new(Syntaxer::default);

/// wrap heavy to initialize syntect things
pub struct Syntaxer {
    pub syntax_set: SyntaxSet,
    /// the only theme, built from the four style roles
    pub theme: Theme,
}
impl Default for Syntaxer {
    fn default() -> Self {
        let xml = tm_theme_xml();
        let mut cursor = std::io::Cursor::new(xml.into_bytes());
        let theme = ThemeSet::load_from_reader(&mut cursor).unwrap_or_default();
        Self {
            syntax_set: time!(
                Debug,
                syntect::dumps::from_uncompressed_data(SYNTAXES).unwrap()
            ),
            theme,
        }
    }
}

impl Syntaxer {
    pub fn highlighter_for(
        &self,
        path: &Path,
        _con: &AppContext,
    ) -> Option<HighlightLines<'_>> {
        path.extension()
            .and_then(|e| e.to_str())
            .and_then(|ext| self.syntax_set.find_syntax_by_extension(ext))
            .map(|syntax| {
                let options = HighlightOptions {
                    ignore_errors: true,
                };
                HighlightLines::new(syntax, &self.theme, options)
            })
    }
}
