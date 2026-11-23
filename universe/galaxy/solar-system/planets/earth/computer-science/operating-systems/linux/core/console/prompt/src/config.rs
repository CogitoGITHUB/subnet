use crate::configs::Palette;
use crate::context::Context;

use crate::utils;
use crate::utils::serde::{ValueDeserializer, ValueRef};
use nu_ansi_term::Color;
use serde::{
    Deserialize, Deserializer, Serialize, de::Error as SerdeError, de::value::Error as ValueError,
};

use std::borrow::Cow;
use std::clone::Clone;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::io::ErrorKind;

use toml::Value;

/// Root config of a module.
pub trait ModuleConfig<'a, E>
where
    Self: Default,
    E: SerdeError,
{
    /// Construct a `ModuleConfig` from a toml value.
    fn from_config<V: Into<ValueRef<'a>>>(config: V) -> Result<Self, E>;

    /// Loads the TOML value into the config.
    /// Missing values are set to their default values.
    /// On error, logs an error message.
    fn load<V: Into<ValueRef<'a>>>(config: V) -> Self {
        match Self::from_config(config) {
            Ok(config) => config,
            Err(e) => {
                log::warn!("Failed to load config value: {e}");
                Self::default()
            }
        }
    }

    /// Helper function that will call `ModuleConfig::from_config(config)` if config is Some,
    /// or `ModuleConfig::default()` if config is None.
    fn try_load<V: Into<ValueRef<'a>>>(config: Option<V>) -> Self {
        config.map(Into::into).map(Self::load).unwrap_or_default()
    }
}

impl<'a, T: Deserialize<'a> + Default> ModuleConfig<'a, ValueError> for T {
    /// Create `ValueDeserializer` wrapper and use it to call `Deserialize::deserialize` on it.
    fn from_config<V: Into<ValueRef<'a>>>(config: V) -> Result<Self, ValueError> {
        let config = config.into();
        let deserializer = ValueDeserializer::new(config);
        T::deserialize(deserializer).or_else(|err| {
            // If the error is an unrecognized key, print a warning and run
            // deserialize ignoring that error. Otherwise, just return the error
            if err.to_string().contains("Unknown key") {
                log::warn!("{err}");
                let deserializer2 = ValueDeserializer::new(config).with_allow_unknown_keys();
                T::deserialize(deserializer2)
            } else {
                Err(err)
            }
        })
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[cfg_attr(
    feature = "config-schema",
    derive(schemars::JsonSchema),
    schemars(deny_unknown_fields)
)]
#[serde(untagged)]
pub enum Either<A, B> {
    First(A),
    Second(B),
}

/// A wrapper around `Vec<T>` that implements `ModuleConfig`, and either
/// accepts a value of type `T` or a list of values of type `T`.
#[derive(Clone, Default, Serialize)]
pub struct VecOr<T>(pub Vec<T>);

impl<'de, T> Deserialize<'de> for VecOr<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let either = Either::<Vec<T>, T>::deserialize(deserializer)?;
        match either {
            Either::First(v) => Ok(Self(v)),
            Either::Second(s) => Ok(Self(vec![s])),
        }
    }
}

#[cfg(feature = "config-schema")]
impl<T> schemars::JsonSchema for VecOr<T>
where
    T: schemars::JsonSchema + Sized,
{
    fn schema_name() -> Cow<'static, str> {
        // `Either::<T, Vec<T>>::schema_name()` is not unique per `T`; nested `VecOr`s must not share a `$defs` entry.
        Cow::Owned(format!("VecOr_{}", T::schema_name()))
    }

    fn schema_id() -> Cow<'static, str> {
        Cow::Owned(format!("{}::VecOr<{}>", module_path!(), T::schema_id()))
    }

    fn json_schema(generator: &mut schemars::generate::SchemaGenerator) -> schemars::Schema {
        Either::<T, Vec<T>>::json_schema(generator)
    }
}

/// Root config of starship.
#[derive(Default)]
pub struct StarshipConfig {
    pub config: Option<toml::Table>,
}

impl StarshipConfig {
    /// Initialize the Config struct
    pub fn initialize(config_file_path: Option<&OsStr>) -> Self {
        Self::config_from_file(config_file_path)
            .map(|config| Self {
                config: Some(config),
            })
            .unwrap_or_default()
    }

    /// Create a config from a starship configuration file
    fn config_from_file(config_file_path: Option<&OsStr>) -> Option<toml::Table> {
        let toml_content = Self::read_config_content_as_str(config_file_path)?;

        match toml::from_str(&toml_content) {
            Ok(parsed) => {
                log::debug!("Config parsed: {parsed:?}");
                Some(parsed)
            }
            Err(error) => {
                log::error!("Unable to parse the config file: {error}");
                None
            }
        }
    }

    pub fn read_config_content_as_str(config_file_path: Option<&OsStr>) -> Option<String> {
        if config_file_path.is_none() {
            log::debug!(
                "Unable to determine `config_file_path`. Perhaps `utils::home_dir` is not defined on your platform?"
            );
            return None;
        }
        let config_file_path = config_file_path.as_ref().unwrap();
        match utils::read_file(config_file_path) {
            Ok(content) => {
                log::trace!("Config file content: \"\n{content}\"");
                Some(content)
            }
            Err(e) => {
                let level = if e.kind() == ErrorKind::NotFound {
                    log::Level::Debug
                } else {
                    log::Level::Error
                };

                log::log!(level, "Unable to read config file content: {e}");
                None
            }
        }
    }

    /// Get the subset of the table for a module by its name
    pub fn get_module_config(&self, module_name: &str) -> Option<&Value> {
        let module_config = self.get_config(&[module_name]);
        if module_config.is_some() {
            log::debug!("Config found for \"{module_name}\": {module_config:?}");
        }
        module_config
    }

    /// Get the value of the config in a specific path
    pub fn get_config(&self, path: &[&str]) -> Option<&Value> {
        let mut prev_table = self.config.as_ref()?;

        assert_ne!(
            path.len(),
            0,
            "Starship::get_config called with an empty path"
        );

        let (table_options, _) = path.split_at(path.len() - 1);

        // Assumes all keys except the last in path has a table
        for option in table_options {
            if let Some(value) = prev_table.get(*option) {
                if let Some(value) = value.as_table() {
                    prev_table = value;
                } else {
                    log::trace!(
                        "No config found for \"{}\": \"{}\" is not a table",
                        path.join("."),
                        option
                    );
                    return None;
                }
            } else if prev_table.contains_key(*option) {
                log::trace!(
                    "No config found for \"{}\": \"{}\" is not a table",
                    path.join("."),
                    option
                );
                return None;
            }
        }

        let last_option = path.last().unwrap();
        let value = prev_table.get(*last_option);
        if value.is_none() {
            log::trace!(
                "No config found for \"{}\": Option \"{}\" not found",
                path.join("."),
                last_option
            );
        }
        value
    }

    /// Get the subset of the table for a custom module by its name
    pub fn get_custom_module_config(&self, module_name: &str) -> Option<&Value> {
        let module_config = self.get_config(&["custom", module_name]);
        if module_config.is_some() {
            log::debug!("Custom config found for \"{module_name}\": {module_config:?}");
        }
        module_config
    }

    /// Get the table of all the registered custom modules, if any
    pub fn get_custom_modules(&self) -> Option<&toml::value::Table> {
        self.get_config(&["custom"])?.as_table()
    }
    /// Get the table of all the registered `env_var` modules, if any
    pub fn get_env_var_modules(&self) -> Option<&toml::value::Table> {
        self.get_config(&["env_var"])?.as_table()
    }
}

/// Deserialize a style string in the starship format with serde
pub fn deserialize_style<'de, D>(de: D) -> Result<Style, D::Error>
where
    D: Deserializer<'de>,
{
    Cow::<'_, str>::deserialize(de).and_then(|s| {
        parse_style_string(s.as_ref(), None).ok_or_else(|| D::Error::custom("Invalid style string"))
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum PrevColor {
    Fg,
    Bg,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
/// Wrapper for `nu_ansi_term::Style` that supports referencing the previous style's foreground/background color.
pub struct Style {
    style: nu_ansi_term::Style,
    bg: Option<PrevColor>,
    fg: Option<PrevColor>,
}

impl Style {
    pub fn to_ansi_style(&self, prev: Option<&nu_ansi_term::Style>) -> nu_ansi_term::Style {
        let Some(prev_style) = prev else {
            return self.style;
        };

        let mut current = self.style;

        if let Some(prev_color) = self.bg {
            match prev_color {
                PrevColor::Fg => current.background = prev_style.foreground,
                PrevColor::Bg => current.background = prev_style.background,
            }
        }

        if let Some(prev_color) = self.fg {
            match prev_color {
                PrevColor::Fg => current.foreground = prev_style.foreground,
                PrevColor::Bg => current.foreground = prev_style.background,
            }
        }

        current
    }

    fn map_style<F>(&self, f: F) -> Self
    where
        F: FnOnce(&nu_ansi_term::Style) -> nu_ansi_term::Style,
    {
        Self {
            style: f(&self.style),
            ..*self
        }
    }

    fn fg(&self, prev_color: PrevColor) -> Self {
        Self {
            fg: Some(prev_color),
            ..*self
        }
    }

    fn bg(&self, prev_color: PrevColor) -> Self {
        Self {
            bg: Some(prev_color),
            ..*self
        }
    }
}

impl From<nu_ansi_term::Style> for Style {
    fn from(value: nu_ansi_term::Style) -> Self {
        Self {
            style: value,
            ..Default::default()
        }
    }
}

impl From<nu_ansi_term::Color> for Style {
    fn from(value: nu_ansi_term::Color) -> Self {
        Self {
            style: value.into(),
            ..Default::default()
        }
    }
}

/** Parse a style string which represents an ansi style. Valid tokens in the style
 string include the following:
 - 'fg:<color>'    (specifies that the color read should be a foreground color)
 - 'bg:<color>'    (specifies that the color read should be a background color)
 - 'underline'
 - 'bold'
 - 'italic'
 - 'inverted'
 - 'blink'
 - '`prev_fg`'        (specifies the color should be the previous foreground color)
 - '`prev_bg`'        (specifies the color should be the previous background color)
 - '<color>'       (see the `parse_color_string` doc for valid color strings)
*/
pub fn parse_style_string(style_string: &str, context: Option<&Context>) -> Option<Style> {
    style_string
        .split_whitespace()
        .try_fold(Style::default(), |style, token| {
            let token = token.to_lowercase();

            // Check for FG/BG identifiers and strip them off if appropriate
            // If col_fg is true, color the foreground. If it's false, color the background.
            let (token, col_fg) = if token.as_str().starts_with("fg:") {
                (token.trim_start_matches("fg:").to_owned(), true)
            } else if token.as_str().starts_with("bg:") {
                (token.trim_start_matches("bg:").to_owned(), false)
            } else {
                (token, true) // Bare colors are assumed to color the foreground
            };

            match token.as_str() {
                "underline" => Some(style.map_style(nu_ansi_term::Style::underline)),
                "bold" => Some(style.map_style(nu_ansi_term::Style::bold)),
                "italic" => Some(style.map_style(nu_ansi_term::Style::italic)),
                "dimmed" => Some(style.map_style(nu_ansi_term::Style::dimmed)),
                "inverted" => Some(style.map_style(nu_ansi_term::Style::reverse)),
                "blink" => Some(style.map_style(nu_ansi_term::Style::blink)),
                "hidden" => Some(style.map_style(nu_ansi_term::Style::hidden)),
                "strikethrough" => Some(style.map_style(nu_ansi_term::Style::strikethrough)),

                "prev_fg" if col_fg => Some(style.fg(PrevColor::Fg)),
                "prev_fg" => Some(style.bg(PrevColor::Fg)),

                "prev_bg" if col_fg => Some(style.fg(PrevColor::Bg)),
                "prev_bg" => Some(style.bg(PrevColor::Bg)),

                // When the string is supposed to be a color:
                // Decide if we yield none, reset background or set color.
                color_string => {
                    if color_string == "none" && col_fg {
                        None // fg:none yields no style.
                    } else {
                        // Either bg or valid color or both.
                        let parsed = parse_color_string(
                            color_string,
                            context.and_then(|x| {
                                get_palette(
                                    &x.root_config.palettes,
                                    x.root_config.palette.as_deref(),
                                )
                            }),
                        );
                        // bg + invalid color = reset the background to default.
                        if !col_fg && parsed.is_none() {
                            let mut new_style = style;
                            new_style.style.background = Option::None;
                            Some(new_style)
                        } else {
                            // Valid color, apply color to either bg or fg
                            parsed.map(|ansi_color| {
                                if col_fg {
                                    style.map_style(|s| s.fg(ansi_color))
                                } else {
                                    style.map_style(|s| s.on(ansi_color))
                                }
                            })
                        }
                    }
                }
            }
        })
}

/** Parse a string that represents a color setting, returning None if this fails
 There are three valid color formats:
  - #RRGGBB      (a hash followed by an RGB hex)
  - u8           (a number from 0-255, representing an ANSI color)
  - colstring    (one of the 16 predefined color strings or a custom user-defined color)
*/
fn parse_color_string(
    color_string: &str,
    palette: Option<&Palette>,
) -> Option<nu_ansi_term::Color> {
    // Colors come only from the console style roles.
    // Only the meaning is kept: error, dim, text.
    let s = color_string.to_lowercase();
    let known = matches!(
        s.as_str(),
        "black" | "red" | "green" | "yellow" | "blue"
            | "purple" | "cyan" | "white" | "bright-black"
            | "bright-red" | "bright-green" | "bright-yellow"
            | "bright-blue" | "bright-purple" | "bright-cyan"
            | "bright-white"
    );
    let hex = s.len() == 7
        && s.starts_with('#')
        && s[1..].chars().all(|c| c.is_ascii_hexdigit());
    let user = palette
        .and_then(|p| p.get(color_string))
        .is_some();
    if !(known || hex || user || s.parse::<u8>().is_ok()) {
        log::debug!("Could not parse color: {color_string}");
        return None;
    }
    Some(match s.as_str() {
        "red" | "bright-red" => crate::roles::err(),
        "black" | "bright-black" => crate::roles::dim(),
        _ => crate::roles::text(),
    })
}

fn get_palette<'a>(
    palettes: &'a HashMap<String, Palette>,
    palette_name: Option<&str>,
) -> Option<&'a Palette> {
    if let Some(palette_name) = palette_name {
        let palette = palettes.get(palette_name);
        if palette.is_some() {
            log::trace!("Found color palette: {palette_name}");
        } else {
            log::warn!("Could not find color palette: {palette_name}");
        }
        palette
    } else {
        log::trace!("No color palette specified, using defaults");
        None
    }
}

