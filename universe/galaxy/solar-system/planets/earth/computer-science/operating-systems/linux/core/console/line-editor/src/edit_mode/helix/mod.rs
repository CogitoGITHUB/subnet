mod helix_keybindings;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
pub use helix_keybindings::{
    default_helix_insert_keybindings, default_helix_normal_keybindings,
    default_helix_select_keybindings,
};

use super::{is_plain_char, is_text_char, parse_non_key_event};

use crate::{
    enums::EventStatus, Direction, EditCommand, EditMode, FindStop, Granularity, Keybindings,
    MotionTarget, PromptEditMode, PromptHelixMode, ReedlineEvent, TextObject, TextObjectScope,
    TextObjectType, WordEdge, WordKind,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HelixMode {
    Normal,
    Insert,
    Select,
}

impl From<PromptHelixMode> for HelixMode {
    fn from(mode: PromptHelixMode) -> Self {
        match mode {
            PromptHelixMode::Normal => HelixMode::Normal,
            PromptHelixMode::Insert => HelixMode::Insert,
            PromptHelixMode::Select => HelixMode::Select,
        }
    }
}

/// A prefix key waiting for its argument.
///
/// `Find` and `Replace` take an arbitrary char as data, so no finite key
/// sequence can spell them; `Goto` takes one key from a fixed set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    /// `f`/`F`/`t`/`T` are waiting for the character to find.
    Find {
        direction: Direction,
        stop: FindStop,
    },
    /// `r` is waiting for the replacement character.
    Replace,
    /// `g` is waiting for the goto target (`h`/`l`/`g`/`e`).
    Goto,
    /// `m` is waiting for the matching action
    MatchStepOne,
    /// `m`-`a`/`i`/`s`/`d`/`r` is waiting for the surrounding character
    MatchStepTwo(MatchAction),
    /// `mr` is waiting for the second text object
    MatchReplace(TextObjectType),
}

/// Every parse_event will result in one of three outcomes:
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// Absorb the `ReedlineRawEvent` -> change state -> continue parsing
    Absorb(Pending),
    /// Execute an `Action` matching the completed sequence
    Execute(Action),
    /// Reject a miss-typed sequence
    Reject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verb {
    SelectingMotion(MotionTarget),
    CollapsingMotion(MotionTarget),
    Collapse(Direction),
    Deselect,
    OnSelection(Op),
    Submit,
    ChangeMode,
    Undo,
    Redo,
    Paste(Direction),
    /// Open a blank line below (`Forward`, `o`) or above (`Backward`, `O`).
    OpenLine(Direction),
    /// `%`. Whole-buffer, so both edges move and no [`MotionTarget`] applies.
    SelectAll,
    /// `x`. Selection-shaped rather than motion-shaped: it moves both edges,
    /// which no [`MotionTarget`] can express.
    SelectLine,
    /// `j`/`k`. Does not lower to a [`MotionTarget`]: which of line movement
    /// and history traversal applies is decided by the engine against the
    /// *whole* buffer, above where a motion resolves.
    LineOrHistory(Direction),
    /// `h`/`l`. A grapheme step that, like `j`/`k`, lets an open menu take the
    /// key first in normal mode, and for `l` a history hint before that.
    GraphemeOrMenu(Direction),
    /// `m`. Apply action onto surrounding characters
    Match(Match),
}

/// Every matching action possible
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatchAction {
    /// `i`. Selects inside a text object
    Inner,
    /// `a`. Select around a text object
    Around,
    /// `s`. Insert a text object around the selection
    Set,
    /// `d`. Delete the nearest text object around the cursor head
    Delete,
    /// `r`. Replace a text object by another around the cursor head
    Replace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Match {
    text_object: TextObjectType,
    /// Useful for replace match action
    text_object2: Option<TextObjectType>,
    action: MatchAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Cut,
    Change,
    Yank,
    Replace(char),
    /// `~`. Keeps the selection, like `Yank`, so a further op reuses the span.
    Switchcase,
    /// `` ` ``. Keeps the selection, as `Switchcase` does.
    Lowercase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Action {
    count: usize,
    verb: Verb,
    next_mode: Option<HelixMode>,
}

impl Action {
    /// Emit `cmd` once per count.
    ///
    /// Valid only when repeating the event composes, i.e. the op re-reads
    /// the cursor each time. Motions and undo qualify; paste won't, since
    /// it writes a cursor derived from what it just inserted.
    fn repeated(self, cmd: EditCommand) -> ReedlineEvent {
        ReedlineEvent::Edit(vec![cmd; self.count])
    }
}

/// Shorthand for the `Outcome::Execute(Action { .. })` arms of the key tables.
fn exec(count: usize, verb: Verb, next_mode: Option<HelixMode>) -> Outcome {
    Outcome::Execute(Action {
        count,
        verb,
        next_mode,
    })
}

/// The word-family target of `w`/`b`/`e` and their WORD (`W`/`B`/`E`) twins.
fn word(kind: WordKind, edge: WordEdge, direction: Direction) -> MotionTarget {
    MotionTarget::Word {
        kind,
        edge,
        direction,
    }
}

/// This parses incoming input `Event`s like a Helix/Kakoune-style editor: motions are
/// selection first, lowered onto the editor's [`MotionTarget`](crate::MotionTarget) verb vocabulary.
#[derive(Debug, Clone)]
pub struct Helix {
    /// Keybinding lookup table for insert mode
    insert_keybindings: Keybindings,
    /// Keybinding lookup table for normal mode
    normal_keybindings: Keybindings,
    /// Keybinding lookup table for select mode
    select_keybindings: Keybindings,
    mode: HelixMode,
    /// Count prefix being accumulated (`3w`).
    count: Option<usize>,
    /// Prefix key waiting for its argument (`f`/`r`/`g`/`m`).
    pending: Option<Pending>,
}

impl EditMode for Helix {
    fn parse_event(&mut self, event: crate::ReedlineRawEvent) -> crate::ReedlineEvent {
        match event.into() {
            Event::Key(key) => match self.mode {
                HelixMode::Insert => self.dispatch_insert(key),
                _ => self.dispatch(key),
            },
            event => parse_non_key_event(event),
        }
    }
    fn edit_mode(&self) -> crate::PromptEditMode {
        match self.mode {
            HelixMode::Insert => PromptEditMode::Helix(PromptHelixMode::Insert),
            HelixMode::Normal => PromptEditMode::Helix(PromptHelixMode::Normal),
            HelixMode::Select => PromptEditMode::Helix(PromptHelixMode::Select),
        }
    }
    fn handle_mode_specific_event(&mut self, event: ReedlineEvent) -> EventStatus {
        match event {
            ReedlineEvent::SwitchMode(PromptEditMode::Helix(target)) => {
                // `dispatch` drops the sequence before a bound chord fires, so
                // no binding gets here with one armed. Reset anyway, rather
                // than lean on that guarantee from over here.
                self.pending = None;
                self.count = None;
                self.mode = HelixMode::from(target);
                EventStatus::Handled
            }
            _ => EventStatus::Inapplicable,
        }
    }
}

impl Helix {
    /// Replace the insert-mode keybinding table, keeping the normal-mode
    /// default.
    ///
    /// Layer onto the defaults rather than starting from
    /// [`Keybindings::empty`]: the table is consulted before the state machine
    /// runs, so an empty one silently drops every bound key.
    ///
    ///     # use reedline::{default_helix_insert_keybindings, Helix};
    ///     let mut bindings = default_helix_insert_keybindings();
    ///     // bindings.add_binding(..);
    ///     let helix = Helix::default().with_insert_keybindings(bindings);
    #[must_use]
    pub fn with_insert_keybindings(mut self, keybindings: Keybindings) -> Self {
        self.insert_keybindings = keybindings;
        self
    }

    /// Replace the normal-mode keybinding table, keeping the insert- and
    /// select-mode defaults. Consulted before the state machine, so a binding
    /// here shadows the built-in key of the same name.
    #[must_use]
    pub fn with_normal_keybindings(mut self, keybindings: Keybindings) -> Self {
        self.normal_keybindings = keybindings;
        self
    }

    /// Replace the select-mode keybinding table, keeping the other defaults.
    /// Layer onto [`default_helix_select_keybindings`] rather than the normal
    /// table, or the extending navigation twins (arrows, `Home`/`End`, ...)
    /// fall back to their mode-blind normal-mode behavior.
    #[must_use]
    pub fn with_select_keybindings(mut self, keybindings: Keybindings) -> Self {
        self.select_keybindings = keybindings;
        self
    }

    /// The keybinding table `dispatch` reads, so normal or select.
    fn keybindings(&self) -> &Keybindings {
        match self.mode {
            HelixMode::Select => &self.select_keybindings,
            _ => &self.normal_keybindings,
        }
    }

    fn dispatch(&mut self, key: KeyEvent) -> ReedlineEvent {
        // Insert should never use this code-path.
        debug_assert!(self.mode != HelixMode::Insert);

        // The table comes first, except for a key the machine claims:
        // - `Esc` always reaches it, otherwise modes get stranded;
        // - a plain `1`-`9` starts or continues a count;
        // - a half-typed sequence takes any character, which is its next count
        //   digit or its pending argument, so a binding cannot steal it.
        // Anything else mid-sequence would only be rejected, so a chord bound
        // in the table fires instead of being eaten and abandons the sequence,
        // which is how vi treats its own `cache`.
        let mid_sequence = self.pending.is_some() || self.count.is_some();
        let claimed = match key.code {
            KeyCode::Esc => true,
            KeyCode::Char('1'..='9') if key.modifiers == KeyModifiers::NONE => true,
            KeyCode::Char(_) => mid_sequence && is_text_char(key.modifiers),
            _ => false,
        };
        if !claimed {
            if let Some(event) = self.keybindings().find_binding(key.modifiers, key.code) {
                self.pending = None;
                self.count = None;
                return event;
            }
        }

        let outcome = match (self.pending.take(), key.code) {
            // Handle a pending key event
            (Some(pending), _) => complete_pending(pending, self.count.unwrap_or(1), key),
            // Handle a count modifier
            (None, KeyCode::Char(c @ '0'..='9'))
                if key.modifiers == KeyModifiers::NONE && (c != '0' || self.count.is_some()) =>
            {
                // Cap the count: every consumer repeats O(count) work on one
                // keystroke, so an absurd prefix must not freeze the REPL.
                self.count = Some(
                    self.count
                        .unwrap_or(0)
                        .saturating_mul(10)
                        .saturating_add(c.to_digit(10).unwrap_or(0) as usize)
                        .min(u16::MAX as usize),
                );
                return ReedlineEvent::None;
            }
            // Unbound or claimed, so the helix machine reads it.
            (None, _) => interpret(self.mode, self.count, key),
        };

        match outcome {
            Outcome::Absorb(pending) => {
                self.pending = Some(pending);
                ReedlineEvent::None
            }
            Outcome::Execute(action) => {
                self.count = None;
                let event = lower(action, self.mode);
                if let Some(next_mode) = action.next_mode {
                    self.mode = next_mode;
                }
                event
            }
            Outcome::Reject => {
                self.count = None;
                ReedlineEvent::None
            }
        }
    }
    fn dispatch_insert(&mut self, key: KeyEvent) -> ReedlineEvent {
        // handle esc first since it has to always reach the machine
        if matches!(key.code, KeyCode::Esc) {
            self.mode = HelixMode::Normal;
            return ReedlineEvent::Multiple(vec![ReedlineEvent::Esc, ReedlineEvent::Repaint]);
        }
        if let Some(event) = self
            .insert_keybindings
            .find_binding(key.modifiers, key.code)
        {
            return event;
        }
        match key.code {
            KeyCode::Enter if key.modifiers == KeyModifiers::NONE => ReedlineEvent::Enter,
            KeyCode::Char(ch) if is_text_char(key.modifiers) => {
                ReedlineEvent::Edit(vec![EditCommand::InsertChar(ch)])
            }
            _ => ReedlineEvent::None,
        }
    }
}
impl Default for Helix {
    fn default() -> Self {
        Self {
            insert_keybindings: default_helix_insert_keybindings(),
            normal_keybindings: default_helix_normal_keybindings(),
            select_keybindings: default_helix_select_keybindings(),
            mode: HelixMode::Insert,
            count: None,
            pending: None,
        }
    }
}

/// Complete a pending sequence
fn complete_pending(pending: Pending, count: usize, key: KeyEvent) -> Outcome {
    let ch = match key.code {
        KeyCode::Char(ch) if is_text_char(key.modifiers) => ch,
        _ => return Outcome::Reject,
    };

    match pending {
        Pending::Find { direction, stop } => exec(
            count,
            Verb::SelectingMotion(MotionTarget::Find {
                ch,
                direction,
                stop,
            }),
            None,
        ),
        Pending::Replace => exec(count, Verb::OnSelection(Op::Replace(ch)), None),
        Pending::Goto => {
            let target = match ch {
                'h' => MotionTarget::LineEdge(Direction::Backward),
                'l' => MotionTarget::LineEdge(Direction::Forward),
                'g' => MotionTarget::BufferEdge(Direction::Backward),
                'e' => MotionTarget::BufferEdge(Direction::Forward),
                's' => MotionTarget::LineStartNonBlank,
                _ => return Outcome::Reject,
            };
            exec(count, Verb::CollapsingMotion(target), None)
        }
        Pending::MatchStepOne => {
            let target = match ch {
                'a' => MatchAction::Around,
                'i' => MatchAction::Inner,
                's' => MatchAction::Set,
                'd' => MatchAction::Delete,
                'r' => MatchAction::Replace,
                _ => return Outcome::Reject,
            };
            Outcome::Absorb(Pending::MatchStepTwo(target))
        }
        Pending::MatchStepTwo(action) => {
            let Some(text_object) = TextObjectType::from_char(ch) else {
                return Outcome::Reject;
            };
            if matches!(action, MatchAction::Replace) {
                return Outcome::Absorb(Pending::MatchReplace(text_object));
            }
            if matches!(
                (action, text_object),
                (
                    MatchAction::Set | MatchAction::Delete | MatchAction::Replace,
                    TextObjectType::Word | TextObjectType::BigWord
                )
            ) {
                return Outcome::Reject;
            }
            exec(
                count,
                Verb::Match(Match {
                    text_object,
                    text_object2: None,
                    action,
                }),
                None,
            )
        }
        Pending::MatchReplace(old) => {
            let Some(new) = TextObjectType::from_char(ch) else {
                return Outcome::Reject;
            };

            exec(
                count,
                Verb::Match(Match {
                    text_object: old,
                    text_object2: Some(new),
                    action: MatchAction::Replace,
                }),
                None,
            )
        }
    }
}

/// Interpret a state
///
/// `count` stays `Option` so a typed `1` is distinguishable from no count;
/// only the goto prefix cares.
fn interpret(mode: HelixMode, count: Option<usize>, key: KeyEvent) -> Outcome {
    // Reject any non-typeable char. Alt-modified keys reach the keybinding table
    // in `dispatch` instead, which is where `Alt-d` and ``Alt-` `` are bound.
    if let KeyCode::Char(_) = key.code {
        if !is_plain_char(key.modifiers) {
            return Outcome::Reject;
        }
    }
    // Helix reads `3gg` as "go to line 3", which has no `MotionTarget` yet, so
    // a counted `g` falls through to the reject arm rather than acting as `gg`.
    if key.code == KeyCode::Char('g') && count.is_none() {
        return Outcome::Absorb(Pending::Goto);
    }
    let count = count.unwrap_or(1);
    match key.code {
        KeyCode::Char(ch) => match ch {
            'f' => Outcome::Absorb(Pending::Find {
                direction: Direction::Forward,
                stop: FindStop::On,
            }),
            'F' => Outcome::Absorb(Pending::Find {
                direction: Direction::Backward,
                stop: FindStop::On,
            }),
            't' => Outcome::Absorb(Pending::Find {
                direction: Direction::Forward,
                stop: FindStop::Before,
            }),
            'T' => Outcome::Absorb(Pending::Find {
                direction: Direction::Backward,
                stop: FindStop::Before,
            }),
            'r' => Outcome::Absorb(Pending::Replace),
            'm' => Outcome::Absorb(Pending::MatchStepOne),
            'w' => exec(
                count,
                Verb::SelectingMotion(word(WordKind::Word, WordEdge::Start, Direction::Forward)),
                None,
            ),
            'b' => exec(
                count,
                Verb::SelectingMotion(word(WordKind::Word, WordEdge::Start, Direction::Backward)),
                None,
            ),
            'e' => exec(
                count,
                Verb::SelectingMotion(word(WordKind::Word, WordEdge::End, Direction::Forward)),
                None,
            ),
            'W' => exec(
                count,
                Verb::SelectingMotion(word(
                    WordKind::LongWord,
                    WordEdge::Start,
                    Direction::Forward,
                )),
                None,
            ),
            'B' => exec(
                count,
                Verb::SelectingMotion(word(
                    WordKind::LongWord,
                    WordEdge::Start,
                    Direction::Backward,
                )),
                None,
            ),
            'E' => exec(
                count,
                Verb::SelectingMotion(word(WordKind::LongWord, WordEdge::End, Direction::Forward)),
                None,
            ),
            'l' => exec(count, Verb::GraphemeOrMenu(Direction::Forward), None),
            'h' => exec(count, Verb::GraphemeOrMenu(Direction::Backward), None),
            'j' => exec(count, Verb::LineOrHistory(Direction::Forward), None),
            'k' => exec(count, Verb::LineOrHistory(Direction::Backward), None),
            'x' => exec(count, Verb::SelectLine, None),
            '%' => exec(count, Verb::SelectAll, None),
            '~' => exec(count, Verb::OnSelection(Op::Switchcase), None),
            '`' => exec(count, Verb::OnSelection(Op::Lowercase), None),
            // Insert at the line's first non-blank, append past its last
            // grapheme. Both collapse first: insert mode rests between
            // graphemes, so the block cursor must not survive the switch.
            'I' => exec(
                count,
                Verb::CollapsingMotion(MotionTarget::LineStartNonBlank),
                Some(HelixMode::Insert),
            ),
            'A' => exec(
                count,
                Verb::CollapsingMotion(MotionTarget::LineEdge(Direction::Forward)),
                Some(HelixMode::Insert),
            ),
            'v' => match mode {
                HelixMode::Normal => exec(count, Verb::ChangeMode, Some(HelixMode::Select)),
                HelixMode::Select => exec(count, Verb::ChangeMode, Some(HelixMode::Normal)),
                _ => Outcome::Reject,
            },
            'i' => exec(
                count,
                Verb::Collapse(Direction::Backward),
                Some(HelixMode::Insert),
            ),
            'a' => exec(
                count,
                Verb::Collapse(Direction::Forward),
                Some(HelixMode::Insert),
            ),
            'd' => exec(count, Verb::OnSelection(Op::Cut), Some(HelixMode::Normal)),
            'c' => exec(
                count,
                Verb::OnSelection(Op::Change),
                Some(HelixMode::Insert),
            ),
            'y' => exec(count, Verb::OnSelection(Op::Yank), Some(HelixMode::Normal)),
            'o' => exec(
                count,
                Verb::OpenLine(Direction::Forward),
                Some(HelixMode::Insert),
            ),
            'O' => exec(
                count,
                Verb::OpenLine(Direction::Backward),
                Some(HelixMode::Insert),
            ),
            'u' => exec(count, Verb::Undo, None),
            'U' => exec(count, Verb::Redo, None),
            'p' => exec(
                count,
                Verb::Paste(Direction::Forward),
                Some(HelixMode::Normal),
            ),
            'P' => exec(
                count,
                Verb::Paste(Direction::Backward),
                Some(HelixMode::Normal),
            ),
            _ => Outcome::Reject,
        },
        KeyCode::Enter => exec(count, Verb::Submit, Some(HelixMode::Insert)),
        // Esc deviates from helix, which keeps the selection in normal mode:
        // the single engine-level `Esc` event both dismisses menus and clears
        // the selection, and with `;` not yet bound it is also the only way to
        // drop a selection.
        KeyCode::Esc => match mode {
            HelixMode::Normal => exec(count, Verb::Deselect, None),
            HelixMode::Select => exec(count, Verb::ChangeMode, Some(HelixMode::Normal)),
            HelixMode::Insert => Outcome::Reject,
        },
        _ => Outcome::Reject,
    }
}

/// Lowers an `Action` onto `ReedlineEvent`
fn lower(action: Action, mode: HelixMode) -> ReedlineEvent {
    let event = match action.verb {
        // The two motion verbs differ only in what normal mode does with the
        // span; select mode extends either way.
        Verb::SelectingMotion(target) | Verb::CollapsingMotion(target) => match mode {
            HelixMode::Normal => action.repeated(match action.verb {
                Verb::SelectingMotion(_) => EditCommand::Select(target),
                _ => EditCommand::Move(target),
            }),
            HelixMode::Select => action.repeated(EditCommand::Extend(target)),
            HelixMode::Insert => {
                // unreachable at runtime: dispatch guards against insert mode
                ReedlineEvent::None
            }
        },
        Verb::OnSelection(op) => match op {
            // Helix has no linewise register: `d` and `c` cut exactly the
            // selection, whatever it spans. They differ only in `next_mode`,
            // which `interpret` already set.
            Op::Cut | Op::Change => ReedlineEvent::Edit(vec![EditCommand::CutSelection {
                granularity: Granularity::CharWise,
            }]),
            Op::Yank => ReedlineEvent::Edit(vec![EditCommand::CopySelection]),
            Op::Replace(ch) => ReedlineEvent::Edit(vec![EditCommand::ReplaceChar(ch)]),
            Op::Switchcase => ReedlineEvent::Edit(vec![EditCommand::SwitchcaseSelection]),
            Op::Lowercase => ReedlineEvent::Edit(vec![EditCommand::LowercaseSelection]),
        },
        Verb::Collapse(dir) => ReedlineEvent::Edit(vec![EditCommand::CollapseSelection(dir)]),
        // Only the first open seeks; the rest go *above* the blank line it just
        // made. A repeated `InsertNewlineBelow` would find no `\n` past that
        // line and append at the buffer end, and a plain `InsertNewline` would
        // delete the resting selection, which under `BlockOverNewline` always
        // covers a grapheme.
        Verb::OpenLine(direction) => {
            let first = match direction {
                Direction::Forward => EditCommand::InsertNewlineBelow,
                Direction::Backward => EditCommand::InsertNewlineAbove,
            };
            let mut cmds = vec![first];
            cmds.resize(action.count, EditCommand::InsertNewlineAbove);
            ReedlineEvent::Edit(cmds)
        }
        Verb::Undo => action.repeated(EditCommand::Undo),
        Verb::Redo => action.repeated(EditCommand::Redo),
        Verb::Paste(direction) => ReedlineEvent::Edit(vec![EditCommand::PasteAtSelectionEdge {
            direction,
            count: action.count,
        }]),
        // Each press grows the selection one line, thus a count is just the
        // command repeated: it re-reads the selection every time.
        Verb::SelectAll => ReedlineEvent::Edit(vec![EditCommand::SelectAll]),
        Verb::SelectLine => action.repeated(EditCommand::SelectLine),
        Verb::Match(m) => match m.action {
            MatchAction::Inner => action.repeated(EditCommand::SelectTextObject(TextObject {
                scope: TextObjectScope::Inner,
                object_type: m.text_object,
                check_next: false,
            })),
            MatchAction::Around => action.repeated(EditCommand::SelectTextObject(TextObject {
                scope: TextObjectScope::Around,
                object_type: m.text_object,
                check_next: false,
            })),
            MatchAction::Set => ReedlineEvent::Edit(vec![EditCommand::AddTextObject {
                text_object: m.text_object,
            }]),
            MatchAction::Delete => ReedlineEvent::Edit(vec![EditCommand::RemoveTextObject {
                text_object: m.text_object,
            }]),
            MatchAction::Replace => {
                let Some(new) = m.text_object2 else {
                    // Should not happened, but just in case...
                    return ReedlineEvent::None;
                };
                ReedlineEvent::Edit(vec![EditCommand::ReplaceTextObject {
                    old: m.text_object,
                    new,
                }])
            }
        },
        Verb::Deselect => ReedlineEvent::Multiple(vec![ReedlineEvent::Esc, ReedlineEvent::Repaint]),
        Verb::ChangeMode => ReedlineEvent::None,
        // `Up`/`Down` already carry the whole rule: move by line while another
        // line is there, walk history at the buffer edge, and prefix-search it
        // when the caret sits at the buffer end. A menu takes the keys first, or
        // `j` would move the caret out from under an open one.
        //
        // Select mode extends by line instead and never reaches history, which
        // would replace the buffer the selection is anchored in. `Multiple`
        // carries the count, since `repeated` only multiplies `EditCommand`s.
        Verb::LineOrHistory(direction) => {
            let event = match mode {
                // `MoveLine*`, not `Extend(MotionTarget::Line)`: the target lands
                // on the line *start*, while `line_down_target` keeps the column,
                // so only this reaches the grapheme normal mode would land on.
                HelixMode::Select => ReedlineEvent::Edit(vec![match direction {
                    Direction::Forward => EditCommand::MoveLineDown { select: true },
                    Direction::Backward => EditCommand::MoveLineUp { select: true },
                }]),
                _ => ReedlineEvent::UntilFound(match direction {
                    Direction::Forward => vec![ReedlineEvent::MenuDown, ReedlineEvent::Down],
                    Direction::Backward => vec![ReedlineEvent::MenuUp, ReedlineEvent::Up],
                }),
            };
            ReedlineEvent::Multiple(vec![event; action.count])
        }
        // As `j`/`k`, and as vi's `h`/`l` and the arrow keys: an open menu takes
        // the key first, and `l` takes a history hint before that (#1185). Select
        // mode only extends, since accepting a hint inserts text.
        Verb::GraphemeOrMenu(direction) => {
            let target = MotionTarget::Grapheme(direction);
            if mode == HelixMode::Select {
                action.repeated(EditCommand::Extend(target))
            } else {
                let motion = ReedlineEvent::Edit(vec![EditCommand::Move(target)]);
                let step = ReedlineEvent::UntilFound(match direction {
                    Direction::Forward => vec![
                        ReedlineEvent::HistoryHintComplete,
                        ReedlineEvent::MenuRight,
                        motion,
                    ],
                    Direction::Backward => vec![ReedlineEvent::MenuLeft, motion],
                });
                ReedlineEvent::Multiple(vec![step; action.count])
            }
        }
        // Collapse forward first, as `a` does. The resting selection outlives the
        // `next_mode` flip to insert, so `InsertNewline` on incomplete input
        // opens with `delete_selection` and eats the covered grapheme, and
        // `submit_buffer`'s final repaint leaves the selection highlight in the
        // scrollback. Forward specifically: the break belongs *past* the covered
        // grapheme, where `Deselect` would land before it and vi's `MoveRight`
        // one beyond, since a helix head already sits on the far edge.
        Verb::Submit => {
            return ReedlineEvent::Multiple(vec![
                ReedlineEvent::Edit(vec![EditCommand::CollapseSelection(Direction::Forward)]),
                ReedlineEvent::Enter,
            ]);
        }
    };

    if action.next_mode.is_some() {
        ReedlineEvent::Multiple(vec![event, ReedlineEvent::Repaint])
    } else {
        event
    }
}

