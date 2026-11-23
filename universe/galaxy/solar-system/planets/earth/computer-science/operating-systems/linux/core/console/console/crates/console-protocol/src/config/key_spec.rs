//! The text grammar of a keybinding's `modifier`, `keycode` and `mode`, and of a
//! menu's `layout`, `input_mode` and `output_mode`.
//!
//! The grammar lives here, and not next to reedline, so that an assignment to
//! `$env.config.keybindings` or `$env.config.menus` can be checked when it is
//! made: a list with an entry that can't be read is rejected and the previous
//! list stays, like for any other config value. `console-cli` maps these types onto
//! `crossterm` and reedline, so both read an entry the same way.
//!
//! What is checked here is what the text names. An entry's `event` needs
//! reedline's own types, so it is still read when the prompt is built.

use super::{ParsedKeybinding, ParsedMenu, helper::extract_value, prelude::*};
use crate::Config;
use strum_macros::EnumString;

/// A modifier key of a keybinding, spelled in any case.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumString)]
#[strum(ascii_case_insensitive, serialize_all = "lowercase")]
pub enum KeyModifier {
    Control,
    Shift,
    Alt,
    Super,
    Hyper,
    Meta,
}

/// The key of a keybinding, apart from its modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyCodeSpec {
    Backspace,
    Enter,
    Down,
    Up,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Tab,
    BackTab,
    Delete,
    Insert,
    /// A function key, `f1` to `f35`.
    F(u8),
    /// A character, including `space`.
    Char(char),
    Null,
    Esc,
}

/// The edit mode a keybinding is for, which is the table reedline keeps it in.
///
/// Each is spelled either `vi_normal` or `ViNormal`, in any case, like reedline's
/// `PromptEditModeDiscriminants`, which `switch_mode_target` in `console-cli` reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumString)]
#[strum(ascii_case_insensitive)]
pub enum EditModeName {
    Emacs,
    #[strum(serialize = "ViNormal", serialize = "vi_normal")]
    ViNormal,
    #[strum(serialize = "ViInsert", serialize = "vi_insert")]
    ViInsert,
    #[strum(serialize = "ViVisual", serialize = "vi_visual")]
    ViVisual,
    #[strum(serialize = "HelixNormal", serialize = "helix_normal")]
    HelixNormal,
    #[strum(serialize = "HelixInsert", serialize = "helix_insert")]
    HelixInsert,
    #[strum(serialize = "HelixSelect", serialize = "helix_select")]
    HelixSelect,
}

/// The layout a menu is shown with, spelled in lowercase.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumString)]
#[strum(serialize_all = "lowercase")]
pub enum MenuLayout {
    Columnar,
    List,
    Ide,
    Description,
}

/// How much of the line a menu hands to its completer, spelled in any case.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumString)]
#[strum(ascii_case_insensitive, serialize_all = "snake_case")]
pub enum MenuInputMode {
    Diff,
    CursorPrefix,
    FullBuffer,
}

/// What a menu replaces when a suggestion is taken, spelled in any case.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumString)]
#[strum(ascii_case_insensitive, serialize_all = "snake_case")]
pub enum MenuOutputMode {
    SuggestedSpan,
    FullBuffer,
    ExtendToEnd,
}

const VALID_MODIFIERS: &str = "'control', 'shift', 'alt', 'super', 'hyper', 'meta', or 'none'";
const VALID_KEYCODES: &str = "'backspace', 'enter', 'space', 'down', 'up', 'left', 'right', 'home', 'end', 'pageup', 'pagedown', 'tab', 'backtab', 'delete', 'insert', 'null', 'esc', 'f1' to 'f35', 'char_<char>', or 'char_u<hex code>'";
/// The edit modes a keybinding can name, as the error says.
pub const VALID_KEYBINDING_MODES: &str = "'emacs', 'vi_insert', 'vi_normal', 'vi_visual', 'helix_insert', 'helix_normal', or 'helix_select'";
const VALID_LAYOUTS: &str = "'columnar', 'list', 'ide', or 'description'";
const VALID_INPUT_MODES: &str = "'diff', 'cursor_prefix', or 'full_buffer'";
const VALID_OUTPUT_MODES: &str = "'suggested_span', 'full_buffer', or 'extend_to_end'";

impl ParsedKeybinding {
    /// The modifiers of this entry: `none`, or `_`-joined names such as `control_shift`.
    pub fn modifiers(&self) -> Result<Vec<KeyModifier>, ShellError> {
        let Ok(modifier) = self.modifier.as_str() else {
            return Err(ShellError::RuntimeTypeMismatch {
                expected: Type::String,
                actual: self.modifier.get_type(),
                span: self.modifier.span(),
            });
        };

        if modifier.eq_ignore_ascii_case("none") {
            return Ok(Vec::new());
        }
        modifier
            .split('_')
            .map(|part| {
                part.parse().map_err(|_| ShellError::InvalidValue {
                    valid: VALID_MODIFIERS.into(),
                    actual: format!("'{part}'"),
                    span: self.modifier.span(),
                })
            })
            .collect()
    }

    /// The key of this entry, apart from its modifiers.
    pub fn key_code(&self) -> Result<KeyCodeSpec, ShellError> {
        let Ok(keycode) = self.keycode.as_str() else {
            return Err(ShellError::RuntimeTypeMismatch {
                expected: Type::String,
                actual: self.keycode.get_type(),
                span: self.keycode.span(),
            });
        };
        let span = self.keycode.span();
        let keycode_lower = keycode.to_ascii_lowercase();

        if let Some(rest) = keycode_lower.strip_prefix("char_") {
            let error = |valid: &str, actual: &str| ShellError::InvalidValue {
                valid: valid.into(),
                actual: actual.into(),
                span,
            };

            let mut chars = rest.chars();
            let char = match (chars.next(), chars.next()) {
                (Some(char), None) => char,
                (Some('u'), Some(_)) => {
                    // `rest` starts with the ASCII `u`, so slicing after it is safe.
                    let Ok(code_point) = u32::from_str_radix(&rest[1..], 16) else {
                        return Err(error("a valid hex code", keycode));
                    };
                    char::from_u32(code_point)
                        .ok_or_else(|| error("a valid Unicode code point", keycode))?
                }
                _ => return Err(error("'char_<char>' or 'char_u<hex code>'", keycode)),
            };
            return Ok(KeyCodeSpec::Char(char));
        }

        Ok(match keycode_lower.as_str() {
            "backspace" => KeyCodeSpec::Backspace,
            "enter" => KeyCodeSpec::Enter,
            "space" => KeyCodeSpec::Char(' '),
            "down" => KeyCodeSpec::Down,
            "up" => KeyCodeSpec::Up,
            "left" => KeyCodeSpec::Left,
            "right" => KeyCodeSpec::Right,
            "home" => KeyCodeSpec::Home,
            "end" => KeyCodeSpec::End,
            "pageup" => KeyCodeSpec::PageUp,
            "pagedown" => KeyCodeSpec::PageDown,
            "tab" => KeyCodeSpec::Tab,
            "backtab" => KeyCodeSpec::BackTab,
            "delete" => KeyCodeSpec::Delete,
            "insert" => KeyCodeSpec::Insert,
            name if name.starts_with('f') => name[1..]
                .parse()
                .ok()
                .filter(|num| (1..=35).contains(num))
                .map(KeyCodeSpec::F)
                .ok_or(ShellError::InvalidValue {
                    valid: VALID_KEYCODES.into(),
                    actual: format!("'{keycode}'"),
                    span,
                })?,
            "null" => KeyCodeSpec::Null,
            "esc" | "escape" => KeyCodeSpec::Esc,
            _ => {
                return Err(ShellError::InvalidValue {
                    valid: VALID_KEYCODES.into(),
                    actual: format!("'{keycode}'"),
                    span,
                });
            }
        })
    }

    /// The edit modes this entry binds the key in, in the order written.
    /// `mode` is a name or a list of names, and lists may nest.
    pub fn modes(&self) -> Result<Vec<EditModeName>, ShellError> {
        fn collect(mode: &Value, modes: &mut Vec<EditModeName>) -> Result<(), ShellError> {
            match mode {
                Value::String { val, .. } => {
                    modes.push(val.parse().map_err(|_| ShellError::InvalidValue {
                        valid: VALID_KEYBINDING_MODES.into(),
                        actual: format!("'{val}'"),
                        span: mode.span(),
                    })?);
                }
                Value::List { vals, .. } => {
                    for inner in vals {
                        collect(inner, modes)?;
                    }
                }
                v => {
                    return Err(ShellError::RuntimeTypeMismatch {
                        expected: Type::custom("string or list<string>"),
                        actual: v.get_type(),
                        span: v.span(),
                    });
                }
            }
            Ok(())
        }

        let mut modes = Vec::new();
        collect(&self.mode, &mut modes)?;
        Ok(modes)
    }

    /// Checks what the text of this entry names: its modes, modifier and
    /// keycode. The `event` is not checked, see the module documentation.
    pub(crate) fn validate(&self) -> Result<(), ShellError> {
        self.modes()?;
        self.modifiers()?;
        self.key_code()?;
        Ok(())
    }
}

impl ParsedMenu {
    /// The layout this menu is shown with. Its `type` must be a record with a
    /// `layout` that names one of the four layouts.
    pub fn layout(&self, config: &Config) -> Result<MenuLayout, ShellError> {
        let span = self.r#type.span();
        let Value::Record { val, .. } = &self.r#type else {
            return Err(ShellError::RuntimeTypeMismatch {
                expected: Type::record(),
                actual: self.r#type.get_type(),
                span,
            });
        };

        let layout = extract_value("layout", val, span)?.to_expanded_string("", config);
        layout.parse().map_err(|_| ShellError::InvalidValue {
            valid: VALID_LAYOUTS.into(),
            actual: format!("'{layout}'"),
            span,
        })
    }

    /// How much of the line this menu hands to its completer. `input_mode` names
    /// it and supersedes the legacy `only_buffer_difference`, which is `diff` when
    /// true and `cursor_prefix` when false. One of the two is required.
    pub fn input_mode(&self, config: &Config) -> Result<MenuInputMode, ShellError> {
        match (&self.input_mode, &self.only_buffer_difference) {
            (Some(input_mode), _) => {
                let text = input_mode.to_expanded_string("", config);
                text.parse().map_err(|_| ShellError::InvalidValue {
                    valid: VALID_INPUT_MODES.into(),
                    actual: format!("'{text}'"),
                    span: input_mode.span(),
                })
            }
            (None, Some(only_buffer_difference)) => Ok(if only_buffer_difference.as_bool()? {
                MenuInputMode::Diff
            } else {
                MenuInputMode::CursorPrefix
            }),
            (None, None) => Err(ShellError::MissingRequiredColumn {
                column: "input_mode (or only_buffer_difference)",
                span: self.name.span(),
            }),
        }
    }

    /// What this menu replaces when a suggestion is taken, if `output_mode` is
    /// set. Unset keeps reedline's default, `suggested_span`.
    pub fn output_mode(&self, config: &Config) -> Result<Option<MenuOutputMode>, ShellError> {
        self.output_mode
            .as_ref()
            .map(|output_mode| {
                let text = output_mode.to_expanded_string("", config);
                text.parse().map_err(|_| ShellError::InvalidValue {
                    valid: VALID_OUTPUT_MODES.into(),
                    actual: format!("'{text}'"),
                    span: output_mode.span(),
                })
            })
            .transpose()
    }

    /// Checks what the text of this menu names: its layout, input mode and
    /// output mode. The fields of the layout itself, such as `columns`, are still
    /// read when the prompt is built.
    pub(crate) fn validate(&self, config: &Config) -> Result<(), ShellError> {
        self.layout(config)?;
        self.input_mode(config)?;
        self.output_mode(config)?;
        Ok(())
    }
}

