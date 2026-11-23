use crate::{
    action::{Action, Actions},
    event::{Key, convert_raw_event_to_key},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use rustc_hash::FxHashMap;
use serde::Deserialize;
use std::ops::Deref;
use std::ops::DerefMut;
use std::str::FromStr;
use tracing::debug;

/// A hashmap of keyboard key bindings to actions.
#[derive(Default, Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Keybindings(pub FxHashMap<Key, Actions>);

impl Deref for Keybindings {
    type Target = FxHashMap<Key, Actions>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Keybindings {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl From<Vec<(Key, Action)>> for Keybindings {
    fn from(bindings: Vec<(Key, Action)>) -> Self {
        let mut map = FxHashMap::default();
        for (key, action) in bindings {
            map.insert(key, action.into());
        }
        Keybindings(map)
    }
}

impl Keybindings {
    pub fn new() -> Self {
        Keybindings(FxHashMap::default())
    }

    pub fn merge(self, new: &Keybindings) -> Keybindings {
        merge_keybindings(self, new)
    }
}

/// Merges two binding collections, with new bindings taking precedence.
///
/// # Arguments
///
/// * `bindings` - The base bindings collection (will be consumed)
/// * `new_bindings` - The new bindings to merge in (higher precedence)
///
/// # Returns
///
/// A new `Bindings` collection with merged key mappings.
///
/// # Examples
///
/// ```rust
/// use television::config::keybindings::{Keybindings, merge_keybindings};
/// use television::event::Key;
/// use television::action::Action;
///
/// let base = Keybindings::from(vec![
///     (Key::Enter, Action::ConfirmSelection),
///     (Key::Esc, Action::Quit),
/// ]);
///
/// let custom = Keybindings::from(vec![
///     (Key::Esc, Action::NoOp), // Override quit with no-op
///     (Key::Tab, Action::ToggleSelectionDown), // Add new binding
/// ]);
///
/// let merged = merge_keybindings(base, &custom);
/// assert_eq!(merged.get(&Key::Enter), Some(&Action::ConfirmSelection.into()));
/// assert_eq!(merged.get(&Key::Esc), Some(&Action::NoOp.into()));
/// assert_eq!(merged.get(&Key::Tab), Some(&Action::ToggleSelectionDown.into()));
/// ```
pub fn merge_keybindings(
    mut base: Keybindings,
    new: &Keybindings,
) -> Keybindings {
    debug!("bindings before: {:?}", base.0);

    // Merge new bindings - they take precedence over existing ones
    for (key, actions) in &new.0 {
        base.0.insert(*key, actions.clone());
    }

    debug!("bindings after: {:?}", base.0);

    base
}

/// Parses a string representation of a key event into a `KeyEvent`.
///
/// This function converts human-readable key descriptions (like "ctrl-a", "alt-enter")
/// into crossterm `KeyEvent` structures.
///
/// # Arguments
///
/// * `raw` - String representation of the key (e.g., "ctrl-a", "shift-f1", "esc")
///
/// # Returns
///
/// * `Ok(KeyEvent)` - Successfully parsed key event
/// * `Err(String)` - Parse error with description
///
/// # Supported Modifiers
///
/// - `ctrl-` - Control key
/// - `alt-` - Alt key
/// - `shift-` - Shift key
/// - `cmd-` - Command key (macOS)
/// - `super-` - Super key (Linux/Windows)
///
/// # Examples
///
/// ```rust
/// use television::config::keybindings::parse_key_event;
/// use crossterm::event::{KeyCode, KeyModifiers};
///
/// let event = parse_key_event("ctrl-a").unwrap();
/// assert_eq!(event.code, KeyCode::Char('a'));
/// assert_eq!(event.modifiers, KeyModifiers::CONTROL);
///
/// let event = parse_key_event("alt-enter").unwrap();
/// assert_eq!(event.code, KeyCode::Enter);
/// assert_eq!(event.modifiers, KeyModifiers::ALT);
/// ```
pub fn parse_key_event(raw: &str) -> anyhow::Result<KeyEvent, String> {
    let raw_lower = raw.to_ascii_lowercase();
    let (remaining, modifiers) = extract_modifiers(&raw_lower);
    parse_key_code_with_modifiers(remaining, modifiers)
}

/// Extracts modifier keys from a raw key string.
///
/// This helper function parses modifier prefixes (ctrl-, alt-, shift-, etc.)
/// from a key string and returns the remaining key part along with the
/// extracted modifiers as a `KeyModifiers` bitfield.
///
/// # Arguments
///
/// * `raw` - The raw key string (already lowercased)
///
/// # Returns
///
/// A tuple of (`remaining_key_string`, `extracted_modifiers`)
///
/// # Examples
///
/// ```ignore
/// let (key, mods) = extract_modifiers("ctrl-alt-a");
/// assert_eq!(key, "a");
/// assert!(mods.contains(KeyModifiers::CONTROL | KeyModifiers::ALT));
/// ```
fn extract_modifiers(raw: &str) -> (&str, KeyModifiers) {
    let mut modifiers = KeyModifiers::empty();
    let mut current = raw;

    loop {
        if let Some(rest) = current.strip_prefix("ctrl-") {
            modifiers.insert(KeyModifiers::CONTROL);
            current = rest;
            continue;
        }
        if let Some(rest) = current.strip_prefix("shift-") {
            modifiers.insert(KeyModifiers::SHIFT);
            current = rest;
            continue;
        }
        if let Some(rest) = current.strip_prefix("alt-") {
            modifiers.insert(KeyModifiers::ALT);
            current = rest;
            continue;
        }
        if let Some(rest) = current.strip_prefix("cmd-") {
            modifiers.insert(KeyModifiers::SUPER);
            current = rest;
            continue;
        }
        if let Some(rest) = current.strip_prefix("super-") {
            modifiers.insert(KeyModifiers::SUPER);
            current = rest;
            continue;
        }
        break;
    }

    (current, modifiers)
}

/// Parses a key code string with pre-extracted modifiers into a `KeyEvent`.
///
/// This function handles the actual key code parsing after modifiers have
/// been extracted. It supports named keys (like "esc", "enter") and
/// single character keys.
///
/// # Arguments
///
/// * `raw` - The key string with modifiers already removed
/// * `modifiers` - Pre-extracted modifier keys
///
/// # Returns
///
/// * `Ok(KeyEvent)` - Successfully parsed key event
/// * `Err(String)` - Parse error for unrecognized keys
///
/// # Supported Keys
///
/// - Named keys: esc, enter, left, right, up, down, home, end, etc.
/// - Function keys: f1-f12
/// - Special keys: space, tab, backspace, delete, etc.
/// - Single characters: a-z, 0-9, punctuation
fn parse_key_code_with_modifiers(
    raw: &str,
    mut modifiers: KeyModifiers,
) -> anyhow::Result<KeyEvent, String> {
    use rustc_hash::FxHashMap;
    use std::sync::LazyLock;

    static KEY_CODE_MAP: LazyLock<FxHashMap<&'static str, KeyCode>> =
        LazyLock::new(|| {
            [
                ("esc", KeyCode::Esc),
                ("enter", KeyCode::Enter),
                ("left", KeyCode::Left),
                ("right", KeyCode::Right),
                ("up", KeyCode::Up),
                ("down", KeyCode::Down),
                ("home", KeyCode::Home),
                ("end", KeyCode::End),
                ("pageup", KeyCode::PageUp),
                ("pagedown", KeyCode::PageDown),
                ("backspace", KeyCode::Backspace),
                ("delete", KeyCode::Delete),
                ("insert", KeyCode::Insert),
                ("f1", KeyCode::F(1)),
                ("f2", KeyCode::F(2)),
                ("f3", KeyCode::F(3)),
                ("f4", KeyCode::F(4)),
                ("f5", KeyCode::F(5)),
                ("f6", KeyCode::F(6)),
                ("f7", KeyCode::F(7)),
                ("f8", KeyCode::F(8)),
                ("f9", KeyCode::F(9)),
                ("f10", KeyCode::F(10)),
                ("f11", KeyCode::F(11)),
                ("f12", KeyCode::F(12)),
                ("space", KeyCode::Char(' ')),
                (" ", KeyCode::Char(' ')),
                ("hyphen", KeyCode::Char('-')),
                ("minus", KeyCode::Char('-')),
                ("tab", KeyCode::Tab),
            ]
            .into_iter()
            .collect()
        });

    let c = if let Some(&key_code) = KEY_CODE_MAP.get(raw) {
        key_code
    } else if raw == "backtab" {
        modifiers.insert(KeyModifiers::SHIFT);
        KeyCode::BackTab
    } else if raw.len() == 1 {
        let mut c = raw.chars().next().unwrap();
        if modifiers.contains(KeyModifiers::SHIFT) {
            c = c.to_ascii_uppercase();
        }
        KeyCode::Char(c)
    } else {
        return Err(format!("Unable to parse {raw}"));
    };

    Ok(KeyEvent::new(c, modifiers))
}

/// Converts a `KeyEvent` back to its string representation.
///
/// This function performs the reverse operation of `parse_key_event`,
/// converting a crossterm `KeyEvent` back into a human-readable string
/// format that can be used in configuration files.
///
/// # Arguments
///
/// * `key_event` - The key event to convert
///
/// # Returns
///
/// String representation of the key event (e.g., "ctrl-a", "alt-enter")
///
/// # Examples
///
/// ```rust
/// use television::config::keybindings::key_event_to_string;
/// use crossterm::event::{KeyEvent, KeyCode, KeyModifiers};
///
/// let event = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL);
/// assert_eq!(key_event_to_string(&event), "ctrl-a");
///
/// let event = KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT);
/// assert_eq!(key_event_to_string(&event), "alt-enter");
/// ```
#[allow(dead_code)]
pub fn key_event_to_string(key_event: &KeyEvent) -> String {
    let char;
    let key_code = match key_event.code {
        KeyCode::Backspace => "backspace",
        KeyCode::Enter => "enter",
        KeyCode::Left => "left",
        KeyCode::Right => "right",
        KeyCode::Up => "up",
        KeyCode::Down => "down",
        KeyCode::Home => "home",
        KeyCode::End => "end",
        KeyCode::PageUp => "pageup",
        KeyCode::PageDown => "pagedown",
        KeyCode::Tab => "tab",
        KeyCode::BackTab => "backtab",
        KeyCode::Delete => "delete",
        KeyCode::Insert => "insert",
        KeyCode::F(c) => {
            char = format!("f({c})");
            &char
        }
        KeyCode::Char(' ') => "space",
        KeyCode::Char(c) => {
            char = c.to_string();
            &char
        }
        KeyCode::Esc => "esc",
        KeyCode::Null
        | KeyCode::CapsLock
        | KeyCode::Menu
        | KeyCode::ScrollLock
        | KeyCode::Media(_)
        | KeyCode::NumLock
        | KeyCode::PrintScreen
        | KeyCode::Pause
        | KeyCode::KeypadBegin
        | KeyCode::Modifier(_) => "",
    };

    let mut modifiers = Vec::with_capacity(3);

    if key_event.modifiers.intersects(KeyModifiers::CONTROL) {
        modifiers.push("ctrl");
    }

    if key_event.modifiers.intersects(KeyModifiers::SHIFT) {
        modifiers.push("shift");
    }

    #[cfg(target_os = "macos")]
    if key_event.modifiers.intersects(KeyModifiers::SUPER) {
        modifiers.push("cmd");
    }

    #[cfg(not(target_os = "macos"))]
    if key_event.modifiers.intersects(KeyModifiers::SUPER) {
        modifiers.push("super");
    }

    if key_event.modifiers.intersects(KeyModifiers::ALT) {
        modifiers.push("alt");
    }

    let mut key = modifiers.join("-");

    if !key.is_empty() {
        key.push('-');
    }
    key.push_str(key_code);

    key
}

impl FromStr for Key {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        if raw.chars().filter(|c| *c == '>').count()
            != raw.chars().filter(|c| *c == '<').count()
        {
            return Err(format!("Unable to parse `{raw}`"));
        }
        let raw = if raw.contains("><") {
            raw
        } else {
            let raw = raw.strip_prefix('<').unwrap_or(raw);
            raw.strip_suffix('>').unwrap_or(raw)
        };

        let key_event = parse_key_event(raw)?;
        Ok(convert_raw_event_to_key(key_event))
    }
}

