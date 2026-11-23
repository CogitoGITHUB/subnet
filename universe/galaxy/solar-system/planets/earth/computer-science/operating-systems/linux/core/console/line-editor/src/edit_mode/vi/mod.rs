mod command;
mod motion;
mod parser;
mod vi_keybindings;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
pub use vi_keybindings::{
    default_vi_insert_keybindings, default_vi_normal_keybindings, default_vi_visual_keybindings,
};

use super::{is_meta_char, is_plain_char, is_text_char, parse_non_key_event, EditMode};
use crate::{
    edit_mode::{keybindings::Keybindings, vi::parser::parse},
    enums::{EditCommand, EventStatus, ReedlineEvent, ReedlineRawEvent},
    Direction, MotionTarget, PromptEditMode, PromptViMode,
};

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum ViMode {
    Normal,
    Insert,
    Visual,
}

impl From<PromptViMode> for ViMode {
    fn from(mode: PromptViMode) -> Self {
        match mode {
            PromptViMode::Normal => ViMode::Normal,
            PromptViMode::Insert => ViMode::Insert,
            PromptViMode::Visual => ViMode::Visual,
        }
    }
}

/// This parses incoming input `Event`s like a Vi-Style editor
///
/// In every mode an unbound `Alt-<char>` is read as `Esc` followed by
/// `<char>`, the meta convention of readline and zsh: `Alt-k` from insert
/// recalls the previous line, `Alt-I` jumps to the line start and stays in
/// insert. Binding the Alt chord itself takes precedence and is the per-key
/// opt-out.
pub struct Vi {
    cache: Vec<char>,
    insert_keybindings: Keybindings,
    normal_keybindings: Keybindings,
    visual_keybindings: Keybindings,
    mode: ViMode,
    previous: Option<ReedlineEvent>,
    // last f, F, t, T motion for ; and ,
    last_char_search: Option<MotionTarget>,
}

impl Default for Vi {
    fn default() -> Self {
        Self::new(
            default_vi_insert_keybindings(),
            default_vi_normal_keybindings(),
            default_vi_visual_keybindings(),
        )
    }
}

impl Vi {
    /// Creates Vi editor using defined keybindings, one table per mode, in the
    /// order insert, normal, visual.
    ///
    /// Visual reads only its own table, so a binding meant for both goes into
    /// both. [`default_vi_visual_keybindings`] is the normal table with its
    /// navigation keys rebound to extend the selection, so build a custom
    /// visual table on top of it rather than on the normal one.
    pub fn new(
        insert_keybindings: Keybindings,
        normal_keybindings: Keybindings,
        visual_keybindings: Keybindings,
    ) -> Self {
        // Every field is spelled out, since `Default` is built through here.
        Self {
            insert_keybindings,
            normal_keybindings,
            visual_keybindings,
            cache: Vec::new(),
            mode: ViMode::Insert,
            previous: None,
            last_char_search: None,
        }
    }

    /// The keybinding table for the mode the machine is in.
    fn keybindings(&self) -> &Keybindings {
        match self.mode {
            ViMode::Normal => &self.normal_keybindings,
            ViMode::Insert => &self.insert_keybindings,
            ViMode::Visual => &self.visual_keybindings,
        }
    }
}

impl EditMode for Vi {
    fn parse_event(&mut self, event: ReedlineRawEvent) -> ReedlineEvent {
        match event.into() {
            Event::Key(KeyEvent {
                code, modifiers, ..
            }) => self.dispatch_key(modifiers, code),
            event => parse_non_key_event(event),
        }
    }

    fn edit_mode(&self) -> PromptEditMode {
        match self.mode {
            ViMode::Normal => PromptEditMode::Vi(PromptViMode::Normal),
            // Visual maps to its own policy (min-width-1 `Block`) so the commit
            // boundary widens the cursor into a selection on entry.
            ViMode::Visual => PromptEditMode::Vi(PromptViMode::Visual),
            ViMode::Insert => PromptEditMode::Vi(PromptViMode::Insert),
        }
    }

    fn handle_mode_specific_event(&mut self, event: ReedlineEvent) -> EventStatus {
        match event {
            ReedlineEvent::SwitchMode(PromptEditMode::Vi(target)) => {
                // Abandon a half-typed sequence (`3f`), or the next key would
                // be parsed as its argument in the new mode.
                self.cache.clear();
                self.mode = ViMode::from(target);
                EventStatus::Handled
            }
            _ => EventStatus::Inapplicable,
        }
    }
}

/// Append `ReedlineEvent::Repaint` unless the event already ends with one,
/// flattening into an existing `Multiple` rather than nesting it.
fn with_trailing_repaint(event: ReedlineEvent) -> ReedlineEvent {
    match event {
        ReedlineEvent::Repaint => event,
        ReedlineEvent::Multiple(mut events) => {
            if events.last() != Some(&ReedlineEvent::Repaint) {
                events.push(ReedlineEvent::Repaint);
            }
            ReedlineEvent::Multiple(events)
        }
        event => ReedlineEvent::Multiple(vec![event, ReedlineEvent::Repaint]),
    }
}

impl Vi {
    /// Route one keypress: the meta prefix first, then the mode's own grammar
    /// and tables. A method rather than the body of `parse_event` so the meta
    /// path can re-dispatch the stripped key without a raw-event round trip.
    fn dispatch_key(&mut self, modifiers: KeyModifiers, code: KeyCode) -> ReedlineEvent {
        if let Some(event) = self.meta_as_escape(modifiers, code) {
            return event;
        }

        match (self.mode, modifiers, code) {
            // TODO: This guard changes `2v`: the pending count keeps `cache`
            // non-empty, so `v` no longer enters Visual mode. Decide how
            // count-prefixed Visual entry should behave before broadening this
            // special case.
            (ViMode::Normal, KeyModifiers::NONE, KeyCode::Char('v')) if self.cache.is_empty() => {
                self.mode = ViMode::Visual;
                // Entering Visual switches the rest policy to `Block`; the
                // pre-paint commit then widens the cursor into its min-width-1
                // selection. Just repaint — do *not* clear the selection here
                // (e.g. by emitting `Esc`), which would defeat starting one.
                ReedlineEvent::Repaint
            }
            (ViMode::Normal | ViMode::Visual, modifier, KeyCode::Char(c)) => {
                let c = c.to_ascii_lowercase();

                let binding = self.keybindings().find_binding(modifiers, KeyCode::Char(c));
                let is_typeable = is_plain_char(modifier);

                // A pending multi-key motion (e.g. `f<char>`) must be completed
                // before a custom keybinding can claim the next key; otherwise a
                // binding on that second key would hijack the sequence.
                if !self.cache.is_empty() || (binding.is_none() && is_typeable) {
                    self.cache.push(if modifier == KeyModifiers::SHIFT {
                        c.to_ascii_uppercase()
                    } else {
                        c
                    });

                    let res = parse(self.mode, &mut self.cache.iter().peekable());

                    if !res.is_valid() {
                        self.cache.clear();
                        ReedlineEvent::None
                    } else if res.is_complete(self.mode) {
                        let event = res.to_reedline_event(self);
                        let previous_mode = self.mode;
                        let mode_changed = match res.changes_mode(previous_mode) {
                            Some(mode) => {
                                self.mode = mode;
                                mode != previous_mode
                            }
                            None => false,
                        };
                        self.cache.clear();
                        // A mode change must repaint even when the edit is a
                        // no-op (e.g. `a` at line end), mirroring helix's
                        // `Multiple([event, Repaint])` on a mode switch.
                        // `changes_mode` can also report the current mode
                        // (`dw`/`yy` report `Normal` from `Normal`), so only
                        // an actual change gains the trailing `Repaint`.
                        if mode_changed {
                            with_trailing_repaint(event)
                        } else {
                            event
                        }
                    } else {
                        ReedlineEvent::None
                    }
                } else if let Some(event) = binding {
                    event
                } else {
                    ReedlineEvent::None
                }
            }
            (ViMode::Insert, modifier, KeyCode::Char(c)) => {
                // Note. The modifier can also be a combination of modifiers, for
                // example:
                //     KeyModifiers::CONTROL | KeyModifiers::ALT
                //     KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT
                //
                // Mixed modifiers are used by non american keyboards that have extra
                // keys like 'alt gr'. Keep this in mind if in the future there are
                // cases where an event is not being captured
                let c = match modifier {
                    KeyModifiers::NONE => c,
                    _ => c.to_ascii_lowercase(),
                };

                self.keybindings()
                    .find_binding(modifier, KeyCode::Char(c))
                    .unwrap_or_else(|| {
                        if is_text_char(modifier) {
                            ReedlineEvent::Edit(vec![EditCommand::InsertChar(
                                if modifier == KeyModifiers::SHIFT {
                                    c.to_ascii_uppercase()
                                } else {
                                    c
                                },
                            )])
                        } else {
                            ReedlineEvent::None
                        }
                    })
            }
            (_, KeyModifiers::NONE, KeyCode::Esc) => ReedlineEvent::Multiple(self.escape()),
            (ViMode::Normal | ViMode::Visual, _, _) => {
                match self.keybindings().find_binding(modifiers, code) {
                    // A key the sequence has no use for: the binding fires and
                    // abandons what was half typed, as a `SwitchMode` does, so
                    // the next character is not read as a stale argument.
                    Some(event) => {
                        self.cache.clear();
                        event
                    }
                    None => {
                        // Default Enter behavior when no custom binding
                        if modifiers == KeyModifiers::NONE && code == KeyCode::Enter {
                            self.mode = ViMode::Insert;
                            // The normal/visual block caret rests *on* a grapheme;
                            // submitting (or inserting a newline on incomplete input)
                            // acts past it. Release the caret forward like `a`/append
                            // — under the now-`Between` policy — so the trailing edit
                            // (abbreviation expansion or the newline) lands at the line
                            // end, not one grapheme short, which otherwise split the
                            // last word and dropped submit-time abbreviation expansion.
                            ReedlineEvent::Multiple(vec![
                                ReedlineEvent::Edit(vec![EditCommand::MoveRight { select: false }]),
                                ReedlineEvent::Enter,
                            ])
                        } else {
                            ReedlineEvent::None
                        }
                    }
                }
            }
            (ViMode::Insert, _, _) => self
                .keybindings()
                .find_binding(modifiers, code)
                .unwrap_or_else(|| {
                    // Default Enter behavior when no custom binding
                    if modifiers == KeyModifiers::NONE && code == KeyCode::Enter {
                        ReedlineEvent::Enter
                    } else {
                        ReedlineEvent::None
                    }
                }),
        }
    }

    /// Leave for normal mode the way `Esc` does: drop any pending sequence,
    /// let the engine close menus and selections, and when leaving insert step
    /// the caret back onto the last typed grapheme, as vim does.
    fn escape(&mut self) -> Vec<ReedlineEvent> {
        self.cache.clear();
        let leaving_insert = self.mode == ViMode::Insert;
        self.mode = ViMode::Normal;
        let mut events = vec![ReedlineEvent::Esc];
        if leaving_insert {
            events.push(ReedlineEvent::Edit(vec![EditCommand::Move(
                MotionTarget::Grapheme(Direction::Backward),
            )]));
        }
        events.push(ReedlineEvent::Repaint);
        events
    }

    /// `Alt-<char>` as `Esc` then `<char>`, the meta convention of readline
    /// and zsh's vi mode. Terminals without the kitty protocol send the same
    /// bytes for both spellings, so this only makes the two agree.
    ///
    /// Returns `None` where the convention does not apply: a binding on the
    /// Alt chord wins (the per-key opt-out), Alt on a non-character key is
    /// left to the tables, and the Ctrl-Alt pairs some terminals report for
    /// AltGr stay typed text via [`is_text_char`].
    fn meta_as_escape(&mut self, modifiers: KeyModifiers, code: KeyCode) -> Option<ReedlineEvent> {
        let KeyCode::Char(c) = code else {
            return None;
        };
        if !is_meta_char(modifiers) {
            return None;
        }

        // Both mode arms look bindings up by the lowercased char, Shift
        // riding in the modifiers; match that so a user's `Alt-Shift-x`
        // binding is found the same way `Shift-x` is.
        if self
            .keybindings()
            .find_binding(modifiers, KeyCode::Char(c.to_ascii_lowercase()))
            .is_some()
        {
            return None;
        }

        let mut events = self.escape();
        // The stripped key can no longer satisfy `is_meta_char`, so this
        // re-dispatch cannot loop back here.
        match self.dispatch_key(modifiers.difference(KeyModifiers::ALT), code) {
            ReedlineEvent::None => {}
            event => events.push(event),
        }
        Some(ReedlineEvent::Multiple(events))
    }
}

