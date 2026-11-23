use super::{
    edit_stack::EditStack, CaretGeometry, Clipboard, Cursor, LineBuffer, Movement, SelectionExtent,
};
#[cfg(feature = "system_clipboard")]
use crate::core_editor::get_system_clipboard;
use crate::core_editor::graphemes::{next_grapheme_boundary, prev_grapheme_boundary};
use crate::core_editor::resolve::resolve_selection;
use crate::core_editor::{commit, line, operator_span, resolve_motion, RestPolicy};
use crate::enums::{
    EditType, TextObject, TextObjectBracket, TextObjectQuote, TextObjectScope, TextObjectType,
    UndoBehavior,
};
use crate::prompt::PromptEditMode;
use crate::{core_editor::get_local_clipboard, EditCommand};
use crate::{Direction, Granularity, MotionTarget, WordEdge, WordKind};
use std::cmp::{max, min};
use std::ops::{DerefMut, Range};

/// Stateful editor executing changes to the underlying [`LineBuffer`]
///
/// In comparison to the state-less [`LineBuffer`] the [`Editor`] keeps track of
/// the undo/redo history and has facilities for cut/copy/yank/paste
pub struct Editor {
    line_buffer: LineBuffer,
    cut_buffer: Box<dyn Clipboard>,
    #[cfg(feature = "system_clipboard")]
    system_clipboard: Box<dyn Clipboard>,
    edit_stack: EditStack<LineBuffer>,
    last_undo_behavior: UndoBehavior,
    edit_mode: PromptEditMode,
    /// Set when [`sync_edit_mode`](Self::sync_edit_mode) adopts a new rest
    /// policy without committing the cursor; cleared at the commit boundary.
    /// Lets the pre-paint sweep settle a command-less mode transition that
    /// would otherwise never re-normalize under the new policy.
    policy_unsettled: bool,
    /// When `true`, a grapheme left/right motion under a block caret (vi normal)
    /// crosses line terminators — `l` at a line's end lands on the next line's
    /// first grapheme, `h` at column 0 on the previous line's last. When `false`
    /// the motion is clamped to the current line (vim's default `h`/`l`). Bar
    /// carets (emacs, vi insert) always cross regardless, since a bar may rest in
    /// the gap around a `\n`. Defaults to `true`.
    cross_line_cursor: bool,
}

/// Whether an [`EditCommand`] could be applied in the current editor state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditCommandStatus {
    Applied,
    Inapplicable,
}

enum OperatorVerb {
    Cut,
    Copy,
    /// Cut, but a `LineWise` span keeps its line terminators so one blank line
    /// remains — vi's change operator (`cc`/`cj`/`cgg`). Identical to `Cut`
    /// for `CharWise` spans.
    Change,
    Erase,
}

/// Build a word [`MotionTarget`] — the shared shape the legacy `*Word*` command
/// sugar lowers to. The emacs-flavored bindings pass [`WordKind::Unicode`]
/// (UAX-29, proven equivalent to the old `*_index` scans); the big-WORD sugar
/// passes [`WordKind::LongWord`].
fn word_target(kind: WordKind, edge: WordEdge, direction: Direction) -> MotionTarget {
    MotionTarget::Word {
        kind,
        edge,
        direction,
    }
}

impl Default for Editor {
    fn default() -> Self {
        Editor {
            line_buffer: LineBuffer::new(),
            cut_buffer: get_local_clipboard(),
            #[cfg(feature = "system_clipboard")]
            system_clipboard: get_system_clipboard(),
            edit_stack: EditStack::new(),
            last_undo_behavior: UndoBehavior::CreateUndoPoint,
            edit_mode: PromptEditMode::Default,
            policy_unsettled: false,
            cross_line_cursor: true,
        }
    }
}

impl Editor {
    /// Get the current [`LineBuffer`]
    pub const fn line_buffer(&self) -> &LineBuffer {
        &self.line_buffer
    }

    /// Mutable [`LineBuffer`] access, no undo.
    pub(crate) fn line_buffer_mut(&mut self) -> &mut LineBuffer {
        &mut self.line_buffer
    }

    /// Set the [`LineBuffer`] with undo behavior.
    pub(crate) fn set_line_buffer(&mut self, line_buffer: LineBuffer, undo_behavior: UndoBehavior) {
        self.line_buffer = line_buffer;
        self.update_undo_state(undo_behavior);
    }

    pub(crate) fn run_edit_command(&mut self, command: &EditCommand) -> EditCommandStatus {
        let initial_cursor = self.line_buffer.cursor();
        let mut status = EditCommandStatus::Applied;

        match command {
            EditCommand::MoveToStart { select } => self.move_to_start(*select),
            EditCommand::MoveToLineStart { select } => self.move_to_line_start(*select),
            EditCommand::MoveToLineNonBlankStart { select } => {
                self.move_to_line_non_blank_start(*select)
            }
            EditCommand::MoveToEnd { select } => self.move_to_end(*select),
            EditCommand::MoveToLineEnd { select } => self.move_to_line_end(*select),
            EditCommand::MoveToPosition { position, select } => {
                self.move_head_to(*position, *select)
            }
            EditCommand::MoveLineUp { select } => self.move_line_up(*select),
            EditCommand::MoveLineDown { select } => self.move_line_down(*select),
            EditCommand::MoveLeft { select } => self.move_left(*select),
            EditCommand::MoveRight { select } => self.move_right(*select),
            EditCommand::MoveWordLeft { select } => self.move_word_left(*select),
            EditCommand::MoveBigWordLeft { select } => self.move_big_word_left(*select),
            EditCommand::MoveWordRight { select } => self.move_word_right(*select),
            EditCommand::MoveWordRightStart { select } => self.move_word_right_start(*select),
            EditCommand::MoveBigWordRightStart { select } => {
                self.move_big_word_right_start(*select)
            }
            // Deprecated alias: the same word-end target as `MoveWordRight` (#766).
            #[allow(deprecated)]
            EditCommand::MoveWordRightEnd { select } => self.move_word_right(*select),
            EditCommand::MoveBigWordRightEnd { select } => self.move_big_word_right_end(*select),
            EditCommand::Move(t) => {
                let head = self.resolve_head(*t);
                self.move_head_to(head, false);
            }
            // A destination-shaped target has no travel direction for `Span`'s
            // `op_end` to bake inclusivity from: whether its landing grapheme is
            // covered turns on which side of the *anchor* it falls, which only
            // `put_cursor` can see. Helix lowers its own (`gs`) through
            // `put_cursor` in select mode too, so take that path either way.
            EditCommand::Extend(t) if t.direction().is_none() => self.apply_move(*t, true),
            EditCommand::Extend(t) => match self.caret_extent() {
                SelectionExtent::CoverLanding => self.apply_move(*t, true),
                // Backward travel departs from the visible caret, which under a
                // block caret already sits one grapheme inside the head.
                // `extend_span` would park the head on `op_end` and the caret is
                // rendered one grapheme back again, so that offset lands twice
                // and one press moves two cells (#1190). Only while the cursor
                // is still forward: once a press reverses it the caret *is* the
                // head and the offset cancels, which is why the doubling shows
                // up on the first press out of a forward selection and not after.
                //
                // `put_cursor` instead places the head so the caret comes to rest
                // exactly on the resolved target. That is `CoverLanding`'s head
                // placement inside a `Span` mode, which the two are otherwise
                // orthogonal to (see `SelectionExtent`): backward travel is the
                // one direction where `Span` has no inclusivity of its own to
                // encode, since `op_end` and the caret disagree about the same
                // grapheme.
                //
                // Forward keeps `extend_span`, since it departs from the head and
                // has no offset to undo.
                SelectionExtent::Span if t.direction() == Some(Direction::Backward) => {
                    self.apply_move(*t, true)
                }
                SelectionExtent::Span => {
                    // Only forward travel reaches here: the arms above take the
                    // directionless and backward targets. Forward departs from
                    // the head rather than the visible caret, since a forward
                    // block selection puts the caret on the near edge of its head
                    // grapheme and the head on the far edge. Departing from the
                    // caret would make an exclusive forward motion a no-op,
                    // landing on the boundary the previous `Extend` parked the
                    // head on.
                    let geom = self.caret_geometry();
                    let origin = self.line_buffer.cursor().head();
                    let op_end = resolve_motion(self.get_buffer(), origin, *t, geom).op_end;
                    let next =
                        self.line_buffer
                            .cursor()
                            .extend_span(self.get_buffer(), op_end, geom);
                    self.place(next);
                }
            },
            EditCommand::Select(t) => match self.caret_extent() {
                SelectionExtent::CoverLanding => {
                    let origin = self.insertion_point();
                    self.place(Cursor::point(origin));
                    let head = self.resolve_head(*t);
                    self.move_head_to(head, true);
                }
                SelectionExtent::Span => {
                    let geom = self.caret_geometry();
                    let origin = self.insertion_point();
                    let selection = resolve_selection(self.get_buffer(), origin, *t, geom);
                    let selection =
                        if self.edit_mode.rest_policy().is_block() || selection.is_empty() {
                            selection
                        } else {
                            let buf = self.get_buffer();
                            if selection.head() >= selection.anchor() {
                                selection.move_head(prev_grapheme_boundary(buf, selection.head()))
                            } else {
                                Cursor::new(
                                    prev_grapheme_boundary(buf, selection.anchor()),
                                    selection.head(),
                                )
                            }
                        };
                    self.place(selection);
                }
            },
            EditCommand::SelectTextObject(t) => {
                self.select_text_object(*t);
            }
            EditCommand::CollapseSelection(direction) => {
                let cursor = self.line_buffer.cursor();
                let pos = match direction {
                    Direction::Backward => cursor.start(),
                    Direction::Forward => cursor.end(),
                };
                self.place(Cursor::point(pos));
            }
            EditCommand::Cut {
                target,
                granularity,
            } => {
                let sel = operator_span(
                    self.get_buffer(),
                    self.insertion_point(),
                    *target,
                    self.caret_geometry(),
                );
                self.operate(sel, OperatorVerb::Cut, *granularity);
            }
            EditCommand::Copy {
                target,
                granularity,
            } => {
                let sel = operator_span(
                    self.get_buffer(),
                    self.insertion_point(),
                    *target,
                    self.caret_geometry(),
                );
                self.operate(sel, OperatorVerb::Copy, *granularity);
            }
            EditCommand::Change {
                target,
                granularity,
            } => {
                let sel = operator_span(
                    self.get_buffer(),
                    self.insertion_point(),
                    *target,
                    self.caret_geometry(),
                );
                self.operate(sel, OperatorVerb::Change, *granularity);
            }
            EditCommand::Erase(t) => {
                let sel = operator_span(
                    self.get_buffer(),
                    self.insertion_point(),
                    *t,
                    self.caret_geometry(),
                );
                self.operate(sel, OperatorVerb::Erase, Granularity::CharWise);
            }
            EditCommand::InsertChar(c) => self.insert_char(*c),
            EditCommand::Complete => {}
            EditCommand::InsertString(str) => self.insert_str(str),
            EditCommand::InsertPair { open, close } => self.insert_pair(*open, *close),
            EditCommand::InsertNewline => self.insert_newline(),
            EditCommand::InsertNewlineAbove => self.insert_newline_above(),
            EditCommand::InsertNewlineBelow => self.insert_newline_below(),
            EditCommand::ReplaceChar(chr) => self.replace_char(*chr),
            EditCommand::ReplaceChars(n_chars, str) => self.replace_chars(*n_chars, str),
            EditCommand::Backspace => self.backspace(),
            EditCommand::BackspacePair { open, close } => self.backspace_pair(*open, *close),
            EditCommand::Delete => self.delete(),
            EditCommand::CutChar => self.cut_char(),
            EditCommand::CutCharLeft => self.cut_char_left(),
            EditCommand::BackspaceWord => self.line_buffer.delete_word_left(),
            EditCommand::DeleteWord => self.line_buffer.delete_word_right(),
            EditCommand::Clear => self.line_buffer.clear(),
            EditCommand::ClearToLineEnd => self.line_buffer.clear_to_line_end(),
            EditCommand::CutCurrentLine => self.cut_current_line(),
            EditCommand::CutFromStart => self.cut_from_start(),
            EditCommand::CutFromStartLinewise { leave_blank_line } => {
                self.cut_from_start_linewise(*leave_blank_line)
            }
            EditCommand::CutFromLineStart => self.cut_from_line_start(),
            EditCommand::CutFromLineNonBlankStart => self.cut_from_line_non_blank_start(),
            EditCommand::CutToEnd => self.cut_from_end(),
            EditCommand::CutToEndLinewise { leave_blank_line } => {
                self.cut_from_end_linewise(*leave_blank_line)
            }
            EditCommand::CutToLineEnd => self.cut_to_line_end(),
            EditCommand::KillLine => self.kill_line(),
            EditCommand::CutWordLeft => self.cut_word_left(),
            EditCommand::CutBigWordLeft => self.cut_big_word_left(),
            EditCommand::CutWordRight => self.cut_word_right(),
            EditCommand::CutBigWordRight => self.cut_big_word_right(),
            EditCommand::CutWordRightToNext => self.cut_word_right_to_next(),
            EditCommand::CutBigWordRightToNext => self.cut_big_word_right_to_next(),
            EditCommand::PasteCutBufferBefore => self.paste_cut_buffer_before(),
            EditCommand::PasteCutBufferAfter => self.paste_cut_buffer_after(),
            EditCommand::PasteAtSelectionEdge { direction, count } => {
                self.paste_at_selection_edge(*direction, *count)
            }
            EditCommand::UppercaseWord => self.line_buffer.uppercase_word(),
            EditCommand::LowercaseWord => self.line_buffer.lowercase_word(),
            EditCommand::CapitalizeWord => self.line_buffer.capitalize_word(),
            EditCommand::SwitchcaseChar => self.line_buffer.switchcase_char(),
            EditCommand::CapitalizeChar => self.line_buffer.capitalize_char(),
            EditCommand::SwapWords => self.line_buffer.swap_words(),
            EditCommand::SwapGraphemes => self.line_buffer.swap_graphemes(),
            EditCommand::Undo => self.undo(),
            EditCommand::Redo => self.redo(),
            EditCommand::CutRightUntil(c) => self.cut_right_until_char(*c, false, true),
            EditCommand::CutRightBefore(c) => self.cut_right_until_char(*c, true, true),
            EditCommand::MoveRightUntil { c, select } => {
                self.move_right_until_char(*c, false, true, *select)
            }
            EditCommand::MoveRightBefore { c, select } => {
                self.move_right_until_char(*c, true, true, *select)
            }
            EditCommand::CutLeftUntil(c) => self.cut_left_until_char(*c, false, true),
            EditCommand::CutLeftBefore(c) => self.cut_left_until_char(*c, true, true),
            EditCommand::MoveLeftUntil { c, select } => {
                self.move_left_until_char(*c, false, true, *select)
            }
            EditCommand::MoveLeftBefore { c, select } => {
                self.move_left_until_char(*c, true, true, *select)
            }
            EditCommand::SelectAll => self.select_all(),
            EditCommand::SelectLine => self.select_line(),
            EditCommand::CutSelection { granularity } => {
                self.cut_selection_to_cut_buffer(*granularity)
            }
            EditCommand::EraseSelection => self.erase_selection(),
            EditCommand::CopySelection => {
                status = self.copy_selection_to_cut_buffer();
            }
            EditCommand::LowercaseSelection => self.lowercase_selection(),
            EditCommand::UppercaseSelection => self.uppercase_selection(),
            EditCommand::SwitchcaseSelection => self.switchcase_selection(),
            // Same command as `PasteCutBufferBefore`, both names are public
            EditCommand::Paste => self.paste_cut_buffer_before(),
            EditCommand::CopyFromStart => self.copy_from_start(),
            EditCommand::CopyFromStartLinewise => self.copy_from_start_linewise(),
            EditCommand::CopyFromLineStart => self.copy_from_line_start(),
            EditCommand::CopyFromLineNonBlankStart => self.copy_from_line_non_blank_start(),
            EditCommand::CopyToEnd => self.copy_from_end(),
            EditCommand::CopyToEndLinewise => self.copy_from_end_linewise(),
            EditCommand::CopyToLineEnd => self.copy_to_line_end(),
            EditCommand::CopyWordLeft => self.copy_word_left(),
            EditCommand::CopyBigWordLeft => self.copy_big_word_left(),
            EditCommand::CopyWordRight => self.copy_word_right(),
            EditCommand::CopyBigWordRight => self.copy_big_word_right(),
            EditCommand::CopyWordRightToNext => self.copy_word_right_to_next(),
            EditCommand::CopyBigWordRightToNext => self.copy_big_word_right_to_next(),
            EditCommand::CopyRightUntil(c) => self.copy_right_until_char(*c, false, true),
            EditCommand::CopyRightBefore(c) => self.copy_right_until_char(*c, true, true),
            EditCommand::CopyLeftUntil(c) => self.copy_left_until_char(*c, false, true),
            EditCommand::CopyLeftBefore(c) => self.copy_left_until_char(*c, true, true),
            EditCommand::CopyCurrentLine => {
                let range = self.line_buffer.current_line_range();
                let copy_slice = &self.line_buffer.get_buffer()[range];
                if !copy_slice.is_empty() {
                    self.cut_buffer.set(copy_slice, Granularity::LineWise);
                }
            }
            EditCommand::CopyLeft => {
                let insertion_offset = self.line_buffer.insertion_point();
                if insertion_offset > 0 {
                    let left_index = self.line_buffer.grapheme_left_index();
                    let copy_range = left_index..insertion_offset;
                    self.cut_buffer.set(
                        &self.line_buffer.get_buffer()[copy_range],
                        Granularity::CharWise,
                    );
                }
            }
            EditCommand::CopyRight => {
                let insertion_offset = self.line_buffer.insertion_point();
                let right_index = self.line_buffer.grapheme_right_index();
                if right_index > insertion_offset {
                    let copy_range = insertion_offset..right_index;
                    self.cut_buffer.set(
                        &self.line_buffer.get_buffer()[copy_range],
                        Granularity::CharWise,
                    );
                }
            }
            EditCommand::SwapCursorAndAnchor => self
                .line_buffer
                .set_cursor(self.line_buffer.cursor().flip()),
            #[cfg(feature = "system_clipboard")]
            EditCommand::CutSelectionSystem => self.cut_selection_to_system(),
            #[cfg(feature = "system_clipboard")]
            EditCommand::CopySelectionSystem => {
                status = self.copy_selection_to_system();
            }
            #[cfg(feature = "system_clipboard")]
            EditCommand::PasteSystem => self.paste_from_system(),
            EditCommand::CutTextObject { text_object } => self.cut_text_object(*text_object),
            EditCommand::CopyTextObject { text_object } => self.copy_text_object(*text_object),
            EditCommand::AddTextObject { text_object } => self.add_text_object(*text_object),
            EditCommand::RemoveTextObject { text_object } => self.remove_text_object(*text_object),
            EditCommand::ReplaceTextObject { old, new } => self.replace_text_object(*old, *new),
        }
        let leaves_selection = matches!(command.edit_type(), EditType::MoveCursor { select: true })
            || matches!(command, EditCommand::PasteAtSelectionEdge { .. })
            || (matches!(
                command,
                EditCommand::CopySelection
                    | EditCommand::LowercaseSelection
                    | EditCommand::UppercaseSelection
                    | EditCommand::SwitchcaseSelection
            ) && self.edit_mode.retains_selection_after_edit())
            || matches!(
                command,
                EditCommand::ReplaceTextObject { .. }
                    | EditCommand::AddTextObject { .. }
                    | EditCommand::RemoveTextObject { .. }
                    | EditCommand::CutTextObject { .. }
                    | EditCommand::CopyTextObject { .. }
            );
        if !leaves_selection {
            self.clear_selection();
        }

        self.commit_cursor();

        if matches!(command.edit_type(), EditType::MoveCursor { .. })
            && self.line_buffer.cursor() == initial_cursor
        {
            return EditCommandStatus::Inapplicable;
        }

        let new_undo_behavior = match (command, command.edit_type()) {
            (_, EditType::MoveCursor { .. }) => UndoBehavior::MoveCursor,
            (EditCommand::InsertChar(c), EditType::EditText) => UndoBehavior::InsertCharacter(*c),
            (EditCommand::Delete, EditType::EditText) => {
                let deleted_char = self.edit_stack.current().grapheme_right().chars().next();
                UndoBehavior::Delete(deleted_char)
            }
            (EditCommand::Backspace, EditType::EditText) => {
                let deleted_char = self.edit_stack.current().grapheme_left().chars().next();
                UndoBehavior::Backspace(deleted_char)
            }
            (_, EditType::UndoRedo | EditType::NoOp) => UndoBehavior::NoOp,
            (_, _) => UndoBehavior::CreateUndoPoint,
        };

        self.update_undo_state(new_undo_behavior);
        status
    }

    pub(crate) fn clear_selection(&mut self) {
        // Collapse to the caret (the visible position), not merely drop the
        // anchor: under `Block` the stored head sits on the far edge, so dropping
        // the anchor alone would strand the cursor one grapheme past where it
        // shows. Collapsing to `point(caret)` keeps it put; the commit boundary
        // re-widens it under the active policy. The caret is the editor's
        // policy-aware one: `LineBuffer::collapse_to_caret` always reads block
        // geometry, which under a bar steps a forward head back a grapheme.
        let caret = self.insertion_point();
        self.line_buffer.set_cursor(Cursor::point(caret));
    }

    fn operate(&mut self, selection: Cursor, verb: OperatorVerb, granularity: Granularity) {
        // `register` is the span the cut buffer keeps; `delete` is the span that
        // leaves the buffer. They coincide except for a linewise Cut/Copy of the
        // *last* line: the deletion eats the preceding terminator so no blank line
        // is stranded, but the register must hold only the line's content —
        // otherwise a later linewise paste re-introduces that newline as a
        // spurious leading blank line.
        let (register, delete) = match granularity {
            Granularity::CharWise => {
                let r = selection.start()..selection.end();
                (r.clone(), r)
            }
            Granularity::LineWise => {
                let buf = self.get_buffer();
                let s = line::start_of_line(buf, selection.start());
                match verb {
                    // Change keeps the line terminators: only the lines'
                    // content is consumed, so one blank line remains for the
                    // re-entered insert mode. Register and deletion coincide.
                    OperatorVerb::Change => {
                        let r = s..line::end_of_line(buf, selection.end());
                        (r.clone(), r)
                    }
                    // Cut/Copy/Erase consume whole lines including the trailing
                    // `\n`. On the last line (no trailing `\n`) the *deletion*
                    // eats the whole preceding terminator instead so no stray
                    // blank line is left — 2 bytes for a `\r\n`, so the `\r` is
                    // not orphaned (e.g. a CRLF history entry "ab\r\ncd" + `dd`
                    // → "ab"; the buffer can carry CR, see `LineBuffer`'s
                    // line-ending contract). The *register* keeps just `s..e` so
                    // a later linewise paste does not gain a leading blank line.
                    _ => {
                        let e = line::start_of_next_line(buf, selection.end()).unwrap_or(buf.len());
                        let delete_start = if e == buf.len() && s > 0 {
                            if buf[..s].ends_with("\r\n") {
                                s - 2
                            } else {
                                s - 1
                            }
                        } else {
                            s
                        };
                        (s..e, delete_start..e)
                    }
                }
            }
        };

        match verb {
            OperatorVerb::Cut => {
                self.copy_range_with(register, granularity);
                self.line_buffer.clear_range_safe(delete.clone());
                self.line_buffer.set_insertion_point(delete.start);
            }
            // Change's register and deletion coincide, so one range suffices.
            OperatorVerb::Change => self.cut_range_with(delete, granularity),
            OperatorVerb::Copy => self.copy_range_with(register, granularity),
            OperatorVerb::Erase => {
                self.line_buffer.clear_range_safe(delete.clone());
                self.line_buffer.set_insertion_point(delete.start);
            }
        }
    }

    /// Plant or clear a selection anchor explicitly — retained only for test
    /// setup. Production selections open through [`move_head_to`](Self::move_head_to)
    /// (`put_cursor`) or the `Block` min-width-1 commit, neither of which needs
    /// this.

    /// Set the current edit mode
    pub fn set_edit_mode(&mut self, mode: PromptEditMode) {
        // Called on every repaint, so skip the work when nothing relevant moved.
        // `commit_cursor` depends only on the rest policy, and the cursor is
        // already committed under the old one; re-normalize only when the policy
        // actually changes (e.g. Vi insert → normal tightens to `OnGrapheme`).
        let policy_changed = mode.rest_policy() != self.edit_mode.rest_policy();
        self.edit_mode = mode;
        // `sync_edit_mode` may have already adopted this policy without
        // committing (a command-less transition), so `policy_changed` can read
        // false here even though the cursor still owes a settle.
        if policy_changed || self.policy_unsettled {
            self.commit_cursor();
        }
    }

    /// Whether a rest-policy change is awaiting a commit (see
    /// [`policy_unsettled`](Self::policy_unsettled) field).
    pub(crate) fn policy_unsettled(&self) -> bool {
        self.policy_unsettled
    }

    /// Set whether a block-caret left/right motion crosses line terminators (see
    /// the [`cross_line_cursor`](Self::cross_line_cursor) field).
    pub(crate) fn set_cross_line_cursor(&mut self, cross: bool) {
        self.cross_line_cursor = cross;
    }

    /// Adopt `mode`'s rest policy *without* committing the cursor.
    ///
    /// Called at the parse seam, before the events a mode transition emitted
    /// are run, so those commands resolve under the new `RestPolicy` (e.g.
    /// the Esc→normal grapheme step-back reads `OnGrapheme`). The cursor is
    /// deliberately left where insert mode put it: the emitted commands move
    /// and commit it under the new policy, and any no-command transition is
    /// settled by the pre-paint `set_edit_mode`. Committing here would pull a
    /// caret at the line end back a grapheme, double-stepping the Esc move.
    pub fn sync_edit_mode(&mut self, mode: PromptEditMode) {
        if mode.rest_policy() != self.edit_mode.rest_policy() {
            self.policy_unsettled = true;
        }
        self.edit_mode = mode;
    }

    /// Normalize the cursor at the single commit boundary: clamp + grapheme-snap
    /// (universal), then apply the active mode's [`RestPolicy`]. Total and
    /// idempotent, so it is safe to call after any state change — including ones
    /// that move the cursor outside the command path (e.g. history navigation),
    /// which must still settle a vi-normal caret off the line end.
    pub(crate) fn commit_cursor(&mut self) {
        let committed = commit(
            self.line_buffer.get_buffer(),
            self.line_buffer.cursor(),
            self.edit_mode.rest_policy(),
        );
        self.line_buffer.set_cursor(committed);
        self.policy_unsettled = false;
    }

    /// Plain head move with no `put_cursor` geometry — retained only for test
    /// setup. Production motions route through [`move_head_to`](Self::move_head_to)
    /// so selections get the inclusive anchor-flip.

    /// Resolve a motion target to the byte the caret should land on.
    ///
    /// The origin is always the visible caret (Helix `move_horizontally` does the
    /// same: `range.cursor()` is the origin for both Move and Extend). The
    /// returned target is fed to [`Cursor::put_cursor`], which places the head and
    /// flips the anchor as needed — so there is no head-vs-caret origin split.
    fn resolve_head(&self, target: MotionTarget) -> usize {
        let buf = self.line_buffer.get_buffer();
        // Origin is the visible cursor position — `insertion_point()` already
        // resolves that per policy (head for Between, caret for Block).
        let origin = self.insertion_point();
        let head = resolve_motion(buf, origin, target, self.caret_geometry()).head;
        // Only a block-caret grapheme step needs a line policy at the edges; every
        // other target's line-crossing is already fixed by `resolve_motion`, and a
        // bar caret (`Between`) moves freely across the terminator either way.
        // `BlockOverNewline` opts out: the terminator is a cell it may rest on, so
        // the raw step is already the landing and clamping or skipping would put
        // the newline out of reach.
        if let MotionTarget::Grapheme(direction) = target {
            if self.caret_geometry() == CaretGeometry::Block
                && !self.edit_mode.rest_policy().covers_terminator()
            {
                return self.grapheme_line_policy(buf, origin, head, direction);
            }
        }
        head
    }

    /// The block-caret line policy for one grapheme step (`h`/`l` in vi
    /// normal/visual): per [`cross_line_cursor`](Self::cross_line_cursor), either
    /// clamp the landing to the current line, or cross the terminator onto a real
    /// cell on the adjacent line. `origin` is the step's start, `head` its raw
    /// one-grapheme landing.
    ///
    /// This is a *movement-landing* transform only. Operator spans (`d`/`c`/`y`)
    /// deliberately bypass it — they resolve straight through `resolve_motion` and
    /// delete the literal grapheme range, which must not skip the `\n` (e.g. `dl`
    /// deletes the char under the caret, never the line break). So the flag steers
    /// where the caret *rests*, not how far an operator reaches.
    fn grapheme_line_policy(
        &self,
        buf: &str,
        origin: usize,
        head: usize,
        direction: Direction,
    ) -> usize {
        if !self.cross_line_cursor {
            // vim-strict: the caret may not leave the current line.
            return match direction {
                Direction::Backward => head.max(line::start_of_line(buf, origin)),
                Direction::Forward => head.min(line::end_of_line(buf, origin)),
            };
        }
        // Cross the terminator so the caret lands on a real cell, not the `\n`.
        // Forward: skip onto the next line's first grapheme. Backward: step once
        // more onto the previous line's last grapheme — unless that line is *also*
        // a terminator (an empty line), where column 0 is the only cell.
        let is_terminator = |pos: usize| buf[pos..].starts_with(['\r', '\n']);
        if !is_terminator(head) {
            return head;
        }
        match direction {
            Direction::Forward => next_grapheme_boundary(buf, head),
            Direction::Backward => {
                let back = prev_grapheme_boundary(buf, head);
                if is_terminator(back) {
                    head // previous line is empty — rest on its column 0
                } else {
                    back
                }
            }
        }
    }

    /// Caret geometry of the active mode: [`CaretGeometry::Block`] for vi normal
    /// / visual (an inclusive motion lands *on* a grapheme and the operator eats
    /// it), [`CaretGeometry::Bar`] for emacs / vi insert (`Between`, resting on
    /// the trailing boundary). Drives the forward word-end landing and operator
    /// inclusivity in [`resolve_motion`] and the selection extension in
    /// [`Cursor::put_cursor`].
    fn caret_geometry(&self) -> CaretGeometry {
        if self.edit_mode.rest_policy() == RestPolicy::Between {
            CaretGeometry::Bar
        } else {
            CaretGeometry::Block
        }
    }

    /// Place the caret on the grapheme at `target` via [`Cursor::put_cursor`]
    /// (Helix's central op), collapsing the selection unless `select` keeps the
    /// anchor, then normalize at the commit boundary (RestPolicy snap and
    /// selection bookkeeping). The per-mode geometry (inclusive block vs
    /// exclusive bar) rides on [`caret_geometry`](Self::caret_geometry), so
    /// inclusivity is carried by the range itself — there is no
    /// `selection_inclusive` side-channel to maintain.
    ///
    /// The sink for [`CoverLanding`](SelectionExtent::CoverLanding) placement:
    /// every `Move`, and every `Extend` under that extent, funnels here after
    /// its target is resolved via [`resolve_motion`]. [`SelectionExtent::Span`]
    /// extension goes around it through [`Cursor::extend_span`].
    fn move_head_to(&mut self, target: usize, select: bool) {
        let next = self.line_buffer.cursor().put_cursor(
            self.line_buffer.get_buffer(),
            target,
            Movement::select(select),
            self.caret_geometry(),
        );
        self.place(next);
    }

    /// Install an already-placed [`Cursor`] and normalize it at the commit
    /// boundary — the shared tail of every placement strategy ([`put_cursor`]'s
    /// `CoverLanding` via [`move_head_to`](Self::move_head_to), [`extend_span`]'s
    /// `Span`). The strategy decides *where* the caret goes; `place` is *how* it
    /// lands: `set_cursor` then [`commit_cursor`](Self::commit_cursor).
    ///
    /// [`put_cursor`]: Cursor::put_cursor
    /// [`extend_span`]: Cursor::extend_span
    fn place(&mut self, next: Cursor) {
        self.line_buffer.set_cursor(next);
        self.commit_cursor();
    }

    /// The active mode's selection model: how `Extend` places the head (vi-visual
    /// `CoverLanding` vs bar/helix `Span`). Orthogonal to [`caret_geometry`](Self::caret_geometry).
    fn caret_extent(&self) -> SelectionExtent {
        self.edit_mode.selection_extent()
    }

    /// Lower a [`MotionTarget`] onto the cursor (the `Move`/`Extend` path):
    /// resolve the head per the active policy, then place it — collapsing the
    /// selection unless `select` keeps the anchor. The shared sink the legacy
    /// `MoveWord*` sugar funnels through, so they behave identically to an
    /// equivalent `Move`/`Extend` command.
    fn apply_move(&mut self, target: MotionTarget, select: bool) {
        let head = self.resolve_head(target);
        self.move_head_to(head, select);
    }

    /// Lower an operator over a [`MotionTarget`] onto the buffer (the
    /// `Cut`/`Copy` path) at char-wise granularity. The shared sink the legacy
    /// `CutWord*`/`CopyWord*` sugar funnels through: `operator_span`'s `op_end`
    /// already encodes inclusivity, so the consumed range matches the old
    /// hand-built `insertion_point..*_index` ranges.
    fn apply_operator(&mut self, target: MotionTarget, verb: OperatorVerb) {
        let sel = operator_span(
            self.get_buffer(),
            self.insertion_point(),
            target,
            self.caret_geometry(),
        );
        self.operate(sel, verb, Granularity::CharWise);
    }

    pub(crate) fn move_line_up(&mut self, select: bool) {
        if let Some(target) = self.line_buffer.line_up_target() {
            self.move_head_to(target, select);
        }
    }

    pub(crate) fn move_line_down(&mut self, select: bool) {
        if let Some(target) = self.line_buffer.line_down_target() {
            self.move_head_to(target, select);
        }
    }

    /// Get the text of the current [`LineBuffer`]
    pub fn get_buffer(&self) -> &str {
        self.line_buffer.get_buffer()
    }

    /// Edit the [`LineBuffer`] in an undo-safe manner.
    pub fn edit_buffer<F>(&mut self, func: F, undo_behavior: UndoBehavior)
    where
        F: FnOnce(&mut LineBuffer),
    {
        self.update_undo_state(undo_behavior);
        func(&mut self.line_buffer);
    }

    /// Set the text of the current [`LineBuffer`] given the specified [`UndoBehavior`]
    /// Insertion point update to the end of the buffer.
    pub(crate) fn set_buffer(&mut self, buffer: String, undo_behavior: UndoBehavior) {
        self.line_buffer.set_buffer(buffer);
        // History navigation replaces the buffer outside the command path, so
        // normalize the cursor here too (e.g. Vi normal must not sit past the end).
        self.commit_cursor();
        self.update_undo_state(undo_behavior);
    }

    pub(crate) fn insertion_point(&self) -> usize {
        // The visible / edit position is policy-dependent: a `Between` (bar)
        // cursor sits at the head; a `Block` cursor sits at the caret (its left
        // edge). This is the one place that distinction lives — motions, edits
        // and callers all read it from here.
        let cursor = self.line_buffer.cursor();
        if self.edit_mode.rest_policy() == RestPolicy::Between {
            cursor.head()
        } else {
            cursor.caret(self.line_buffer.get_buffer())
        }
    }

    /// Where completion treats the cursor: the *end* of the word under it.
    ///
    /// A caret cursor rests *on* a grapheme and [`Self::insertion_point`]
    /// reports that grapheme's start, so completing there strands it
    /// (`foo` -> `foobaro`). Complete at its far edge instead — where insert
    /// mode sits, since leaving insert steps one grapheme back.
    pub(crate) fn completion_point(&self) -> usize {
        let pos = self.insertion_point();
        if self.edit_mode.rest_policy() == RestPolicy::Between {
            return pos;
        }
        let buf = self.line_buffer.get_buffer();
        // Grapheme-aware: `pos + 1` would split a multi-byte grapheme.
        // Yields `buf.len()` at the end, so no separate guard.
        let next = next_grapheme_boundary(buf, pos);
        // No current policy rests the caret *on* a line terminator (`Block`
        // widens backward off it), but a Helix-faithful policy that does would
        // otherwise pull the newline into the completed span and join the two
        // lines. The word ends at the caret there.
        if buf[pos..next].starts_with(['\n', '\r']) {
            pos
        } else {
            next
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.line_buffer.is_empty()
    }

    pub(crate) fn is_cursor_at_first_line(&self) -> bool {
        self.line_buffer.is_cursor_at_first_line()
    }

    pub(crate) fn is_cursor_at_last_line(&self) -> bool {
        self.line_buffer.is_cursor_at_last_line()
    }

    /// Whether the cursor holds a selection rather than resting as a caret.
    ///
    /// Under a bar caret the resting cursor is an empty point, so any
    /// non-empty cursor is a selection and shape settles it. A block caret
    /// rests as a min-width-1 range, so shape alone cannot tell a resting
    /// caret from a `v`-started one-grapheme selection; only the mode can.
    /// Asking the mode under a bar caret too would miss the shift-selections
    /// emacs and vi insert can hold.
    pub(crate) fn cursor_is_a_selection(&self) -> bool {
        let cursor = self.line_buffer.cursor();
        if self.caret_geometry() == CaretGeometry::Block {
            next_grapheme_boundary(self.get_buffer(), cursor.start()) < cursor.end()
                || (!cursor.is_empty() && self.edit_mode.is_selection_mode())
        } else {
            !cursor.is_empty()
        }
    }

    pub(crate) fn is_cursor_at_buffer_end(&self) -> bool {
        let buf = self.get_buffer();
        // A selection is never a clean end-of-buffer point: accepting a hint
        // here would run through `prepare_append_at_buffer_end` and silently
        // drop it.
        if self.cursor_is_a_selection() {
            return false;
        }
        // Measure from the visible caret: `insertion_point` already resolves
        // the covered grapheme for Block and the trailing gap for Bar, whether
        // the cursor is a point or a min-width-1 block.
        let caret = self.insertion_point();
        if self.caret_geometry() == CaretGeometry::Block {
            next_grapheme_boundary(buf, caret) == buf.len()
        } else {
            caret == buf.len()
        }
    }

    pub(crate) fn reset_undo_stack(&mut self) {
        self.edit_stack.reset();
    }

    pub(crate) fn move_to_start(&mut self, select: bool) {
        self.move_head_to(0, select);
    }

    pub(crate) fn move_to_end(&mut self, select: bool) {
        self.move_head_to(self.line_buffer.len(), select);
    }

    /// Place the edit point *past the last grapheme* (at `len`) so the next
    /// insert appends rather than splitting. A block caret rests one grapheme
    /// inward from the end, so a plain insert there lands *before* the final
    /// character — accepting a trailing history hint must append instead. Does
    /// not commit, so the following `InsertString` reads this position.
    pub(crate) fn prepare_append_at_buffer_end(&mut self) {
        // Collapse to a bare point, don't just move the head: a resting block
        // caret is anchored (a helix "cursor" is a 1-wide selection), and
        // `set_insertion_point` would keep that anchor. The append then runs
        // through `insert_str`'s `delete_selection` and eats the covered
        // grapheme -- accepting the hint "-add" on "ssh" gave "ss-add".
        self.line_buffer
            .set_cursor(Cursor::point(self.line_buffer.len()));
    }

    pub(crate) fn move_to_line_start(&mut self, select: bool) {
        self.move_head_to(self.line_buffer.line_start_index(), select);
    }

    pub(crate) fn move_to_line_non_blank_start(&mut self, select: bool) {
        self.move_head_to(self.line_buffer.line_non_blank_start_index(), select);
    }

    pub(crate) fn move_to_line_end(&mut self, select: bool) {
        self.move_head_to(self.line_buffer.find_current_line_end(), select);
    }

    fn undo(&mut self) {
        let val = self.edit_stack.undo();
        self.line_buffer = val.clone();
    }

    fn redo(&mut self) {
        let val = self.edit_stack.redo();
        self.line_buffer = val.clone();
    }

    pub(crate) fn update_undo_state(&mut self, undo_behavior: UndoBehavior) {
        if matches!(undo_behavior, UndoBehavior::NoOp) {
            self.last_undo_behavior = UndoBehavior::NoOp;
            return;
        }
        if !undo_behavior.create_undo_point_after(&self.last_undo_behavior) {
            self.edit_stack.undo();
        }
        self.edit_stack.insert(self.line_buffer.clone());
        self.last_undo_behavior = undo_behavior;
    }

    // The dedicated `*Linewise` cut/copy methods below back the legacy public
    // `EditCommand` variants only — every builtin binding now lowers through
    // `operate` + `Granularity::LineWise` (with the `Change` verb covering the
    // `leave_blank_line` flavor). Linewise span fixes belong in `operate` /
    // `core_editor::line`, not here.

    fn cut_current_line(&mut self) {
        let deletion_range = self.line_buffer.current_line_range();

        let cut_slice = &self.line_buffer.get_buffer()[deletion_range.clone()];
        if !cut_slice.is_empty() {
            self.cut_buffer.set(cut_slice, Granularity::LineWise);
            self.line_buffer.set_insertion_point(deletion_range.start);
            self.line_buffer.clear_range(deletion_range);
        }
    }

    fn cut_from_start(&mut self) {
        let insertion_offset = self.line_buffer.insertion_point();
        if insertion_offset > 0 {
            self.cut_buffer.set(
                &self.line_buffer.get_buffer()[..insertion_offset],
                Granularity::CharWise,
            );
            self.line_buffer.clear_to_insertion_point();
        }
    }

    fn cut_from_start_linewise(&mut self, leave_blank_line: bool) {
        let insertion_offset = self.line_buffer.insertion_point();
        let end_offset = self.line_buffer.get_buffer()[insertion_offset..]
            .find('\n')
            .map_or(self.line_buffer.len(), |offset| {
                // When leave_blank_line is true, we do **not** add 1 to the offset
                // So there will remain an empty line after the operation
                if leave_blank_line {
                    insertion_offset + offset
                } else {
                    insertion_offset + offset + 1
                }
            });
        if end_offset > 0 {
            self.cut_buffer.set(
                &self.line_buffer.get_buffer()[..end_offset],
                Granularity::LineWise,
            );
            self.line_buffer.clear_range(..end_offset);
            self.line_buffer.move_to_start();
        }
    }

    fn cut_from_line_start(&mut self) {
        let previous_offset = self.line_buffer.insertion_point();
        self.line_buffer.move_to_line_start();
        let deletion_range = self.line_buffer.insertion_point()..previous_offset;
        let cut_slice = &self.line_buffer.get_buffer()[deletion_range.clone()];
        if !cut_slice.is_empty() {
            self.cut_buffer.set(cut_slice, Granularity::CharWise);
            self.line_buffer.clear_range(deletion_range);
        }
    }

    fn cut_from_line_non_blank_start(&mut self) {
        let cursor_pos = self.line_buffer.insertion_point();
        self.line_buffer.move_to_line_non_blank_start();
        let other_pos = self.line_buffer.insertion_point();
        let deletion_range = min(cursor_pos, other_pos)..max(cursor_pos, other_pos);
        self.cut_range(deletion_range);
    }

    fn cut_from_end(&mut self) {
        let cut_slice = &self.line_buffer.get_buffer()[self.line_buffer.insertion_point()..];
        if !cut_slice.is_empty() {
            self.cut_buffer.set(cut_slice, Granularity::CharWise);
            self.line_buffer.clear_to_end();
        }
    }

    fn cut_from_end_linewise(&mut self, leave_blank_line: bool) {
        let buf = self.line_buffer.get_buffer();
        let len = buf.len();
        let nl = buf[..self.line_buffer.insertion_point()].rfind('\n');
        // The register keeps content from the line start only (no leading
        // terminator) so a later linewise paste gains no blank line. The
        // deletion eats the preceding terminator when not leaving a blank line —
        // the whole `\r\n` for CRLF (see `LineBuffer`'s line-ending contract).
        // Same register/delete split as `operate`.
        let register_start = nl.map_or(0, |offset| offset + 1);
        let delete_start = nl.map_or(0, |offset| {
            if leave_blank_line {
                offset + 1
            } else if buf[..offset].ends_with('\r') {
                offset - 1
            } else {
                offset
            }
        });

        if delete_start < len {
            let register_slice = &self.line_buffer.get_buffer()[register_start..];
            if !register_slice.is_empty() {
                self.cut_buffer.set(register_slice, Granularity::LineWise);
            }
            self.line_buffer.set_insertion_point(delete_start);
            self.line_buffer.clear_to_end();
        }
    }

    fn cut_to_line_end(&mut self) {
        let cut_slice = &self.line_buffer.get_buffer()
            [self.line_buffer.insertion_point()..self.line_buffer.find_current_line_end()];
        if !cut_slice.is_empty() {
            self.cut_buffer.set(cut_slice, Granularity::CharWise);
            self.line_buffer.clear_to_line_end();
        }
    }

    fn kill_line(&mut self) {
        if self.line_buffer.insertion_point() == self.line_buffer.find_current_line_end() {
            self.cut_char()
        } else {
            self.cut_to_line_end()
        }
    }

    fn cut_word_left(&mut self) {
        self.apply_operator(
            word_target(WordKind::Unicode, WordEdge::Start, Direction::Backward),
            OperatorVerb::Cut,
        );
    }

    fn cut_big_word_left(&mut self) {
        self.apply_operator(
            word_target(WordKind::LongWord, WordEdge::Start, Direction::Backward),
            OperatorVerb::Cut,
        );
    }

    fn cut_word_right(&mut self) {
        // emacs `M-d`: consume to the current word's trailing boundary (no skip).
        // Under a bar caret the operator span runs `origin..trailing`, matching
        // the old `insertion_point..word_right_index`.
        self.apply_operator(
            word_target(WordKind::Unicode, WordEdge::End, Direction::Forward),
            OperatorVerb::Cut,
        );
    }

    fn cut_big_word_right(&mut self) {
        self.apply_operator(
            word_target(WordKind::LongWord, WordEdge::End, Direction::Forward),
            OperatorVerb::Cut,
        );
    }

    fn cut_word_right_to_next(&mut self) {
        self.apply_operator(
            word_target(WordKind::Unicode, WordEdge::Start, Direction::Forward),
            OperatorVerb::Cut,
        );
    }

    fn cut_big_word_right_to_next(&mut self) {
        self.apply_operator(
            word_target(WordKind::LongWord, WordEdge::Start, Direction::Forward),
            OperatorVerb::Cut,
        );
    }

    fn cut_char(&mut self) {
        if self.line_buffer.selection_anchor().is_some() {
            self.cut_selection_to_cut_buffer(Granularity::CharWise);
        } else {
            let insertion_offset = self.line_buffer.insertion_point();
            let next_char = self.line_buffer.grapheme_right_index();
            self.cut_range(insertion_offset..next_char);
        }
    }

    fn paste_at_selection_edge(&mut self, direction: Direction, count: usize) {
        let at = match direction {
            Direction::Forward => self.line_buffer.cursor().end(),
            Direction::Backward => self.line_buffer.cursor().start(),
        };

        match self.cut_buffer.get() {
            (content, Granularity::CharWise) if !content.is_empty() => {
                let to_paste = content.repeat(count);
                self.line_buffer.set_cursor(Cursor::point(at));
                self.line_buffer.insert_str(&to_paste);
                self.line_buffer
                    .set_cursor(Cursor::new(at, at + to_paste.len()));
            }
            // no consumer for linewise paste yet
            _ => (),
        }
    }

    fn move_right_until_char(
        &mut self,
        c: char,
        before_char: bool,
        current_line: bool,
        select: bool,
    ) {
        // Route through `move_head_to` so a selecting search opens the selection
        // via `put_cursor`; the old `update_selection_anchor` + raw `set_head`
        // path dropped the anchor when starting a selection from a point.
        let Some(found) = self.line_buffer.find_char_right(c, current_line) else {
            // Miss: no movement; only settle the selection per `select`.
            if !select {
                self.clear_selection();
            }
            return;
        };
        let target = if before_char {
            self.line_buffer.grapheme_left_index_from_pos(found)
        } else {
            found
        };
        self.move_head_to(target, select);
    }

    fn move_left_until_char(
        &mut self,
        c: char,
        before_char: bool,
        current_line: bool,
        select: bool,
    ) {
        // See `move_right_until_char`.
        let Some(found) = self.line_buffer.find_char_left(c, current_line) else {
            if !select {
                self.clear_selection();
            }
            return;
        };
        let target = if before_char {
            found + c.len_utf8()
        } else {
            found
        };
        self.move_head_to(target, select);
    }

    fn cut_right_until_char(&mut self, c: char, before_char: bool, current_line: bool) {
        if let Some(index) = self.line_buffer.find_char_right(c, current_line) {
            // Saving the section of the string that will be deleted to be
            // stored into the buffer
            let extra = if before_char { 0 } else { c.len_utf8() };
            let cut_slice =
                &self.line_buffer.get_buffer()[self.line_buffer.insertion_point()..index + extra];

            if !cut_slice.is_empty() {
                self.cut_buffer.set(cut_slice, Granularity::CharWise);

                if before_char {
                    self.line_buffer.delete_right_before_char(c, current_line);
                } else {
                    self.line_buffer.delete_right_until_char(c, current_line);
                }
            }
        }
    }

    fn cut_left_until_char(&mut self, c: char, before_char: bool, current_line: bool) {
        if let Some(index) = self.line_buffer.find_char_left(c, current_line) {
            // Saving the section of the string that will be deleted to be
            // stored into the buffer
            let extra = if before_char { c.len_utf8() } else { 0 };
            let cut_slice =
                &self.line_buffer.get_buffer()[index + extra..self.line_buffer.insertion_point()];

            if !cut_slice.is_empty() {
                self.cut_buffer.set(cut_slice, Granularity::CharWise);

                if before_char {
                    self.line_buffer.delete_left_before_char(c, current_line);
                } else {
                    self.line_buffer.delete_left_until_char(c, current_line);
                }
            }
        }
    }

    fn replace_char(&mut self, character: char) {
        // Visual `r`: replace every grapheme in the selection with `character`,
        // preserving line terminators — vim's `r` over a selection.
        if let Some((start, end)) = self.get_selection() {
            use unicode_segmentation::UnicodeSegmentation;
            let replacement: String = self.line_buffer.get_buffer()[start..end]
                .graphemes(true)
                .map(|g| {
                    if g == "\n" || g == "\r\n" || g == "\r" {
                        g.to_string()
                    } else {
                        character.to_string()
                    }
                })
                .collect();
            self.line_buffer.replace_range(start..end, &replacement);
            self.line_buffer.set_cursor(Cursor::point(start));
            return;
        }
        // Anchor the in-place replace on the caret: under a Block/visual cursor
        // head is one grapheme past the caret, so deleting+inserting without
        // collapsing first would clear two graphemes and corrupt the buffer.
        self.line_buffer.collapse_to_caret();
        let insertion_point = self.line_buffer.insertion_point();
        self.line_buffer.delete_right_grapheme();

        self.line_buffer.insert_char(character);
        self.line_buffer.set_insertion_point(insertion_point);
    }

    fn replace_chars(&mut self, n_chars: usize, string: &str) {
        // See `replace_char`: collapse the selection so the deletes start at the
        // caret rather than overshooting from the head.
        self.line_buffer.collapse_to_caret();
        for _ in 0..n_chars {
            self.line_buffer.delete_right_grapheme();
        }

        self.line_buffer.insert_str(string);
    }

    fn move_left(&mut self, select: bool) {
        let head = self.resolve_head(MotionTarget::Grapheme(Direction::Backward));
        self.move_head_to(head, select);
    }

    fn move_right(&mut self, select: bool) {
        let head = self.resolve_head(MotionTarget::Grapheme(Direction::Forward));
        self.move_head_to(head, select);
    }

    fn select_all(&mut self) {
        let end = self.line_buffer.len();
        self.line_buffer.set_cursor(Cursor::new(0, end));
    }

    /// Helix `x`: snap out to whole lines, or take one more when already there.
    ///
    /// The "already there" test is what no composition of existing commands can
    /// express: [`Select`](EditCommand::Select) re-anchors at the origin and
    /// [`Extend`](EditCommand::Extend) keeps its anchor, so neither can move
    /// both edges to line boundaries *and* notice they were there already.
    fn select_line(&mut self) {
        let buf = self.line_buffer.get_buffer();
        let cursor = self.line_buffer.cursor();
        let (start, end) = (cursor.start(), cursor.end());
        let first = line::start_of_line(buf, start);
        // The last byte covered, which for a point is the position itself. Taken
        // off `end` since that is the exclusive edge and may already be the next
        // line's first byte.
        let last = if end > start {
            prev_grapheme_boundary(buf, end)
        } else {
            start
        };
        // `None` at an unterminated last line, where the buffer end is the edge.
        let after = line::start_of_next_line(buf, last).unwrap_or(buf.len());
        let head = if start == first && end == after {
            line::start_of_next_line(buf, after).unwrap_or(buf.len())
        } else {
            after
        };
        self.place(Cursor::new(first, head));
    }

    #[cfg(feature = "system_clipboard")]
    fn cut_selection_to_system(&mut self) {
        if let Some((start, end)) = self.get_selection() {
            let cut_slice = &self.line_buffer.get_buffer()[start..end];
            self.system_clipboard.set(cut_slice, Granularity::CharWise);
            self.cut_range(start..end);
            self.clear_selection();
        }
    }

    fn cut_selection_to_cut_buffer(&mut self, granularity: Granularity) {
        if let Some((start, end)) = self.get_selection() {
            let sel = Cursor::new(start, end);
            self.operate(sel, OperatorVerb::Cut, granularity);
            self.clear_selection();
        }
    }

    /// Helix `Alt-d`: drop the selection without touching the cut buffer.
    ///
    /// `OperatorVerb::Erase` is the register-free deletion the motion-shaped
    /// `Erase` already uses; only the span differs.
    fn erase_selection(&mut self) {
        if let Some((start, end)) = self.get_selection() {
            let sel = Cursor::new(start, end);
            self.operate(sel, OperatorVerb::Erase, Granularity::CharWise);
            self.clear_selection();
        }
    }

    #[cfg(feature = "system_clipboard")]
    fn copy_selection_to_system(&mut self) -> EditCommandStatus {
        let Some((start, end)) = self.get_selection() else {
            return EditCommandStatus::Inapplicable;
        };

        let cut_slice = &self.line_buffer.get_buffer()[start..end];
        self.system_clipboard.set(cut_slice, Granularity::CharWise);
        EditCommandStatus::Applied
    }

    fn copy_selection_to_cut_buffer(&mut self) -> EditCommandStatus {
        let Some((start, end)) = self.get_selection() else {
            return EditCommandStatus::Inapplicable;
        };

        let cut_slice = &self.line_buffer.get_buffer()[start..end];
        self.cut_buffer.set(cut_slice, Granularity::CharWise);
        EditCommandStatus::Applied
    }

    fn lowercase_selection(&mut self) {
        if let Some((start, end)) = self.get_selection() {
            let lowercase_slice = self.line_buffer.get_buffer()[start..end].to_ascii_lowercase();
            self.line_buffer.replace_range(start..end, &lowercase_slice);
        }
    }

    fn uppercase_selection(&mut self) {
        if let Some((start, end)) = self.get_selection() {
            let uppercase_slice = self.line_buffer.get_buffer()[start..end].to_ascii_uppercase();
            self.line_buffer.replace_range(start..end, &uppercase_slice);
        }
    }

    fn switchcase_selection(&mut self) {
        if let Some((start, end)) = self.get_selection() {
            let switchcase_slice = self.line_buffer.get_buffer()[start..end]
                .chars()
                .map(|ch| {
                    if ch.is_ascii_lowercase() {
                        ch.to_ascii_uppercase()
                    } else {
                        ch.to_ascii_lowercase()
                    }
                })
                .collect::<String>();
            self.line_buffer
                .replace_range(start..end, &switchcase_slice);
        }
    }

    /// If a selection is active returns the selected range, otherwise None.
    /// The range is guaranteed to be ascending.
    pub fn get_selection(&self) -> Option<(usize, usize)> {
        self.line_buffer.get_selection()
    }

    /// The one-grapheme cell the caret rests on inside the active selection,
    /// or `None` when there is no selection, the cell falls outside the
    /// selected range, or the caret geometry is a bar (emacs / vi insert rest
    /// *between* graphemes, so no cell belongs to the cursor).
    ///
    /// This is the cell a distinct cursor style may claim: helix renders its
    /// primary cursor with an own style *inside* the selection, and a flat
    /// selection style (e.g. reverse video) painted over the whole range can
    /// otherwise swallow the terminal cursor entirely.
    pub(crate) fn selection_head_cell(&self) -> Option<(usize, usize)> {
        let (from, to) = self.get_selection()?;
        if self.caret_geometry() != CaretGeometry::Block {
            return None;
        }
        let cell_start = self.insertion_point();
        let cell_end = next_grapheme_boundary(self.get_buffer(), cell_start);
        (cell_start < to && cell_end > from).then_some((cell_start, cell_end))
    }

    fn backspace(&mut self) {
        if self.line_buffer.selection_anchor().is_some() {
            self.line_buffer.delete_selection();
        } else {
            self.line_buffer.delete_left_grapheme();
        }
    }

    /// Cut the grapheme left of the caret into the cut buffer (vi `X`).
    ///
    /// Intended for vi normal mode only, where there is no selection, since
    /// visual `X` is dispatched as a linewise `CutSelection`. Bound directly
    /// elsewhere, a selection could exist, so clear it first: `X` always means
    /// "delete the grapheme before the caret," and clearing avoids leaving a
    /// stale anchor pointing into the mutated buffer. The cut is clamped to
    /// the current line so `X` never crosses a terminator (unconditional,
    /// unlike the `cross_line_cursor`-gated clamp in `resolve_head`).
    fn cut_char_left(&mut self) {
        self.clear_selection();
        let cur_pos = self.line_buffer.insertion_point();
        let left_index = self.line_buffer.grapheme_left_index();
        if left_index < cur_pos && left_index >= self.line_buffer.current_line_range().start {
            self.cut_range(left_index..cur_pos);
        }
    }

    fn delete(&mut self) {
        if self.line_buffer.selection_anchor().is_some() {
            self.line_buffer.delete_selection();
        } else {
            self.line_buffer.delete_right_grapheme();
        }
    }

    fn move_word_left(&mut self, select: bool) {
        self.apply_move(
            word_target(WordKind::Unicode, WordEdge::Start, Direction::Backward),
            select,
        );
    }

    fn move_big_word_left(&mut self, select: bool) {
        self.apply_move(
            word_target(WordKind::LongWord, WordEdge::Start, Direction::Backward),
            select,
        );
    }

    fn move_word_right(&mut self, select: bool) {
        // emacs M-f: end of current word, no skip.
        self.apply_move(
            word_target(WordKind::Unicode, WordEdge::End, Direction::Forward),
            select,
        );
    }

    fn move_word_right_start(&mut self, select: bool) {
        self.apply_move(
            word_target(WordKind::Unicode, WordEdge::Start, Direction::Forward),
            select,
        );
    }

    fn move_big_word_right_start(&mut self, select: bool) {
        self.apply_move(
            word_target(WordKind::LongWord, WordEdge::Start, Direction::Forward),
            select,
        );
    }

    fn move_big_word_right_end(&mut self, select: bool) {
        self.apply_move(
            word_target(WordKind::LongWord, WordEdge::End, Direction::Forward),
            select,
        );
    }

    fn insert_pair(&mut self, open: char, close: char) {
        if let Some((start, end)) = self.get_selection() {
            let selected = self.line_buffer.get_buffer()[start..end].to_string();
            let replacement = format!("{open}{selected}{close}");
            self.line_buffer.replace_range(start..end, &replacement);
            self.line_buffer.set_cursor(Cursor::point(
                start + open.len_utf8() + selected.len() + close.len_utf8(),
            ));
        } else {
            self.line_buffer.insert_char(open);
            let inner = self.line_buffer.insertion_point();
            self.line_buffer.insert_char(close);
            self.line_buffer.set_cursor(Cursor::point(inner));
        }
    }

    /// Replace the text from the caret to the end of its line with
    /// `before_caret` followed by `after_caret`, leaving the caret between
    /// them, as one undo step.
    pub(crate) fn replace_to_line_end(&mut self, before_caret: &str, after_caret: &str) {
        let start = self.insertion_point();
        let end = self.line_buffer.find_current_line_end();
        self.line_buffer
            .replace_range(start..end, &format!("{before_caret}{after_caret}"));
        self.line_buffer
            .set_cursor(Cursor::point(start + before_caret.len()));
        self.commit_cursor();
        self.update_undo_state(UndoBehavior::CreateUndoPoint);
    }

    pub(crate) fn is_auto_pair_closer_at_cursor(&self, close: char) -> bool {
        self.line_buffer.selection_anchor().is_none()
            && self.line_buffer.grapheme_right().starts_with(close)
    }

    pub(crate) fn is_empty_auto_pair_at_cursor(&self, open: char, close: char) -> bool {
        self.line_buffer.selection_anchor().is_none()
            && self.line_buffer.grapheme_left().starts_with(open)
            && self.line_buffer.grapheme_right().starts_with(close)
    }

    fn backspace_pair(&mut self, open: char, close: char) {
        if !self.is_empty_auto_pair_at_cursor(open, close) {
            return;
        }

        self.line_buffer.delete_right_grapheme();
        self.line_buffer.delete_left_grapheme();
    }

    fn insert_char(&mut self, c: char) {
        self.line_buffer.delete_selection();
        self.line_buffer.insert_char(c);
    }

    fn insert_str(&mut self, str: &str) {
        self.line_buffer.delete_selection();
        self.line_buffer.insert_str(str);
    }

    fn insert_newline(&mut self) {
        self.line_buffer.delete_selection();
        self.line_buffer.insert_newline();
    }

    /// Collapse first: `set_insertion_point` is `set_head`, so on an anchored
    /// cursor (every helix one) the stale anchor dragged the caret back onto
    /// the old line.
    fn insert_newline_above(&mut self) {
        self.clear_selection();
        let index = self.line_buffer.find_char_left('\n', false).unwrap_or(0);
        self.line_buffer.set_insertion_point(index);
        self.line_buffer.insert_newline();
    }

    fn insert_newline_below(&mut self) {
        self.clear_selection();
        let index = self
            .line_buffer
            .find_char_right('\n', false)
            .unwrap_or(self.line_buffer.len());
        self.line_buffer.set_insertion_point(index);
        self.line_buffer.insert_newline();
    }

    #[cfg(feature = "system_clipboard")]
    fn paste_from_system(&mut self) {
        insert_clipboard_content_before(&mut self.line_buffer, self.system_clipboard.deref_mut());
    }

    fn paste_cut_buffer_before(&mut self) {
        insert_clipboard_content_before(&mut self.line_buffer, self.cut_buffer.deref_mut());
    }

    fn paste_cut_buffer_after(&mut self) {
        insert_clipboard_content_after(&mut self.line_buffer, self.cut_buffer.deref_mut());
    }

    fn cut_range(&mut self, range: Range<usize>) {
        self.cut_range_with(range, Granularity::CharWise);
    }

    fn cut_range_with(&mut self, range: Range<usize>, granularity: Granularity) {
        if range.start <= range.end {
            self.copy_range_with(range.clone(), granularity);
            self.line_buffer.clear_range_safe(range.clone());
            self.line_buffer.set_insertion_point(range.start);
        }
    }

    fn copy_range(&mut self, range: Range<usize>) {
        self.copy_range_with(range, Granularity::CharWise);
    }

    fn copy_range_with(&mut self, range: Range<usize>, granularity: Granularity) {
        if range.start < range.end {
            let slice = &self.line_buffer.get_buffer()[range];
            self.cut_buffer.set(slice, granularity);
        }
    }

    /// Return the range of the word under the cursor.
    /// A word consists of a sequence of letters, digits and underscores,
    /// separated with white space.
    /// A block of whitespace under the cursor is also treated as a word.
    ///
    /// `text_object_scope` Inner includes only the word itself
    /// while Around also includes trailing whitespace,
    /// or preceding whitespace if there is no trailing whitespace.
    fn word_text_object_range(&self, text_object_scope: TextObjectScope) -> Range<usize> {
        self.line_buffer
            .current_whitespace_range()
            .unwrap_or_else(|| {
                let word_range = self.line_buffer.current_word_range();
                match text_object_scope {
                    TextObjectScope::Inner => word_range,
                    TextObjectScope::Around => {
                        self.line_buffer.expand_range_with_whitespace(word_range)
                    }
                }
            })
    }

    /// Return the range of the WORD under the cursor.
    /// A WORD consists of a sequence of non-blank characters, separated with white space.
    /// A block of whitespace under the cursor is also treated as a word.
    ///
    /// `text_object_scope` Inner includes only the word itself
    /// while Around also includes trailing whitespace,
    /// or preceding whitespace if there is no trailing whitespace.
    fn big_word_text_object_range(&self, text_object_scope: TextObjectScope) -> Range<usize> {
        self.line_buffer
            .current_whitespace_range()
            .unwrap_or_else(|| {
                let big_word_range = self.line_buffer.current_big_word_range();
                match text_object_scope {
                    TextObjectScope::Inner => big_word_range,
                    TextObjectScope::Around => self
                        .line_buffer
                        .expand_range_with_whitespace(big_word_range),
                }
            })
    }

    /// Returns `Some(Range<usize>)` for range inside the character pair in `pair_group`
    /// at or surrounding the cursor, the next pair if no pairs in `pair_group`
    /// surround the cursor, or `None` if there are no pairs from `pair_group` found.
    ///
    /// `text_object_scope` [`TextObjectScope::Inner`] includes only the range inside the pair
    /// whereas [`TextObjectScope::Around`] also includes the surrounding pair characters
    ///
    /// If multiple pair types exist, returns the innermost pair that surrounds
    /// the cursor. Handles empty pair as zero-length ranges inside pair.
    /// For asymmetric pairs like `(` `)` the search is multi-line, however,
    /// for symmetric pairs like `"` `"` the search is restricted to the current line.
    fn matching_pair_group_text_object_range(
        &self,
        text_object_scope: TextObjectScope,
        matching_pair_group: &[(char, char)],
        check_next: bool,
    ) -> Option<Range<usize>> {
        self.line_buffer
            .range_inside_current_pair_in_group(matching_pair_group)
            .or_else(|| {
                if check_next {
                    self.line_buffer
                        .range_inside_next_pair_in_group(matching_pair_group)
                } else {
                    None
                }
            })
            .and_then(|pair_range| match text_object_scope {
                TextObjectScope::Inner => Some(pair_range),
                TextObjectScope::Around => self.expand_range_to_include_pair(pair_range),
            })
    }

    /// Returns `Some(Range<usize>)` for range inside brackets (`()`, `[]`, `{}`)
    /// at or surrounding the cursor, the next pair of brackets if no brackets
    /// surround the cursor, or `None` if there are no brackets found.
    ///
    /// `text_object_scope` [`TextObjectScope::Inner`] includes only the range inside the pair
    /// whereas [`TextObjectScope::Around`] also includes the surrounding pair characters
    ///
    /// If multiple bracket types exist, returns the innermost pair that surrounds
    /// the cursor. Handles empty brackets as zero-length ranges inside brackets.
    /// Includes brackets that span multiple lines.
    fn bracket_text_object_range(
        &self,
        text_object_scope: TextObjectScope,
        brackets_type: TextObjectBracket,
        check_next: bool,
    ) -> Option<Range<usize>> {
        const BRACKET_PAIRS: &[(char, char)] = &[('(', ')'), ('[', ']'), ('{', '}'), ('<', '>')];
        let pairs = match brackets_type {
            TextObjectBracket::Parenthesis => &BRACKET_PAIRS[0..1],
            TextObjectBracket::SquareBracket => &BRACKET_PAIRS[1..2],
            TextObjectBracket::CurlyBracket => &BRACKET_PAIRS[2..3],
            TextObjectBracket::AngleBracket => &BRACKET_PAIRS[3..4],
            TextObjectBracket::All => BRACKET_PAIRS,
        };
        self.matching_pair_group_text_object_range(text_object_scope, pairs, check_next)
    }

    /// Returns `Some(Range<usize>)` for the range inside quotes (`""`, `''` or `\`\`\`)
    /// at the cursor, the next pair of quotes if the cursor is not within quotes,
    /// or `None` if there are no quotes found.
    ///
    /// Quotes are restricted to the current line.
    ///
    /// `text_object_scope` [`TextObjectScope::Inner`] includes only the range inside the pair
    /// whereas [`TextObjectScope::Around`] also includes the surrounding pair characters
    ///
    /// If multiple quote types exist, returns the innermost pair that surrounds
    /// the cursor. Handles empty quotes as zero-length ranges inside quote.
    fn quote_text_object_range(
        &self,
        text_object_scope: TextObjectScope,
        quote_type: TextObjectQuote,
        check_next: bool,
    ) -> Option<Range<usize>> {
        const QUOTE_PAIRS: &[(char, char)] = &[('\'', '\''), ('"', '"'), ('`', '`')];
        let pairs = match quote_type {
            TextObjectQuote::SingleQuote => &QUOTE_PAIRS[0..1],
            TextObjectQuote::DoubleQuote => &QUOTE_PAIRS[1..2],
            TextObjectQuote::Tick => &QUOTE_PAIRS[2..3],
            TextObjectQuote::All => QUOTE_PAIRS,
        };
        self.matching_pair_group_text_object_range(text_object_scope, pairs, check_next)
    }

    /// Get the bounds for a text object operation
    fn text_object_range(&self, text_object: TextObject) -> Option<Range<usize>> {
        match text_object.object_type {
            TextObjectType::Word => Some(self.word_text_object_range(text_object.scope)),
            TextObjectType::BigWord => Some(self.big_word_text_object_range(text_object.scope)),
            TextObjectType::Brackets(brackets_type) => self.bracket_text_object_range(
                text_object.scope,
                brackets_type,
                text_object.check_next,
            ),
            TextObjectType::Quotes(quote_type) => {
                self.quote_text_object_range(text_object.scope, quote_type, text_object.check_next)
            }
            TextObjectType::Pair { left, right } => self.matching_pair_group_text_object_range(
                text_object.scope,
                &[(left, right)],
                text_object.check_next,
            ),
        }
    }

    fn cut_text_object(&mut self, text_object: TextObject) {
        if let Some(range) = self.text_object_range(text_object) {
            self.cut_range(range);
        }
    }

    fn copy_text_object(&mut self, text_object: TextObject) {
        if let Some(range) = self.text_object_range(text_object) {
            self.copy_range(range);
        }
    }

    fn add_text_object(&mut self, text_object: TextObjectType) {
        let Some(pair) = text_object.to_chars() else {
            return;
        };
        let cursor = self.line_buffer.cursor();
        let cursor_forward = cursor.with_direction(super::cursor::Direction::Forward);
        self.line_buffer.set_cursor(cursor_forward);
        self.line_buffer.insert_char(pair.1);
        self.line_buffer.set_cursor(cursor_forward.flip());
        self.line_buffer.insert_char(pair.0);
        self.place(Cursor::new(
            cursor.anchor() + pair.0.len_utf8(),
            cursor.head() + pair.0.len_utf8(),
        ));
    }

    fn remove_text_object(&mut self, text_object: TextObjectType) {
        let Some((left_len, right_len)) = text_object
            .to_chars()
            .map(|(l, r)| (l.len_utf8(), r.len_utf8()))
        else {
            return;
        };
        let cursor = self.line_buffer.cursor();
        self.line_buffer.set_cursor(Cursor::point(cursor.head()));
        let Some(range) = self.text_object_range(TextObject {
            scope: TextObjectScope::Inner,
            object_type: text_object,
            check_next: false,
        }) else {
            self.line_buffer.set_cursor(cursor);
            return;
        };
        let range = range.start - left_len..range.end;
        self.line_buffer
            .clear_range(range.end..range.end + right_len);
        self.line_buffer
            .clear_range(range.start..range.start + left_len);
        let is_range_empty = (range.end - range.start) == left_len;
        let mut anchor = cursor.anchor();
        if cursor.anchor() > range.start || cursor.anchor() == range.start && is_range_empty {
            anchor = anchor.saturating_sub(left_len);
        }
        if cursor.anchor() > range.end {
            anchor = anchor.saturating_sub(right_len);
        }
        let mut head = cursor.head();
        if cursor.head() > range.start || is_range_empty {
            head = head.saturating_sub(left_len);
        }
        if cursor.head() > range.end || cursor.head() == range.end && is_range_empty {
            head = head.saturating_sub(right_len);
        }
        let new_cursor = Cursor::new(anchor, head);
        self.place(new_cursor);
    }
    fn replace_text_object(&mut self, old: TextObjectType, new: TextObjectType) {
        let Some((old_left_len, old_right_len)) =
            old.to_chars().map(|(l, r)| (l.len_utf8(), r.len_utf8()))
        else {
            return;
        };
        let Some((new_left_len, new_right_len)) =
            new.to_chars().map(|(l, r)| (l.len_utf8(), r.len_utf8()))
        else {
            return;
        };
        let cursor = self.line_buffer.cursor();
        self.line_buffer.set_cursor(Cursor::point(cursor.head()));
        let Some(range) = self.text_object_range(TextObject {
            scope: TextObjectScope::Inner,
            object_type: old,
            check_next: false,
        }) else {
            self.line_buffer.set_cursor(cursor);
            return;
        };
        self.remove_text_object(old);
        self.line_buffer.set_cursor(Cursor::new(
            range.start - old_left_len,
            range.end - old_left_len,
        ));
        self.add_text_object(new);

        let (left_diff, right_diff) = (
            old_left_len as isize - new_left_len as isize,
            old_right_len as isize - new_right_len as isize,
        );
        if left_diff == 0 && right_diff == 0 {
            self.place(cursor);
            return;
        }
        let mut anchor = cursor.anchor();
        if cursor.anchor() >= range.start {
            anchor = anchor.saturating_sub_signed(left_diff);
        }
        if cursor.anchor() > range.end {
            anchor = anchor.saturating_sub_signed(right_diff);
        }
        let mut head = cursor.head();
        if cursor.head() >= range.start {
            head = head.saturating_sub_signed(left_diff);
        }
        if cursor.head() > range.end {
            head = head.saturating_sub_signed(right_diff);
        }
        let new_cursor = Cursor::new(anchor, head);
        self.place(new_cursor);
    }

    fn select_text_object(&mut self, text_object: TextObject) {
        if let Some(range) = self.text_object_range(text_object) {
            self.place(Cursor::new(range.start, range.end));
        }
    }

    pub(crate) fn copy_from_start(&mut self) {
        let insertion_offset = self.line_buffer.insertion_point();
        if insertion_offset > 0 {
            self.cut_buffer.set(
                &self.line_buffer.get_buffer()[..insertion_offset],
                Granularity::CharWise,
            );
        }
    }

    pub(crate) fn copy_from_start_linewise(&mut self) {
        let insertion_point = self.line_buffer.insertion_point();
        let end_offset = self.line_buffer.get_buffer()[insertion_point..]
            .find('\n')
            .map_or(self.line_buffer.len(), |offset| insertion_point + offset);
        if end_offset > 0 {
            self.cut_buffer.set(
                &self.line_buffer.get_buffer()[..end_offset],
                Granularity::LineWise,
            );
        }
        self.line_buffer.move_to_start();
    }

    pub(crate) fn copy_from_line_start(&mut self) {
        let previous_offset = self.line_buffer.insertion_point();
        let start_offset = {
            let temp_pos = self.line_buffer.insertion_point();
            self.line_buffer.move_to_line_start();
            let start = self.line_buffer.insertion_point();
            self.line_buffer.set_insertion_point(temp_pos);
            start
        };
        let copy_range = start_offset..previous_offset;
        self.copy_range(copy_range);
    }

    pub(crate) fn copy_from_line_non_blank_start(&mut self) {
        let cursor_pos = self.line_buffer.insertion_point();
        self.line_buffer.move_to_line_non_blank_start();
        let other_pos = self.line_buffer.insertion_point();
        self.line_buffer.set_insertion_point(cursor_pos);
        let copy_range = min(cursor_pos, other_pos)..max(cursor_pos, other_pos);
        self.copy_range(copy_range);
    }

    pub(crate) fn copy_from_end(&mut self) {
        let copy_range = self.line_buffer.insertion_point()..self.line_buffer.len();
        self.copy_range(copy_range);
    }

    pub(crate) fn copy_from_end_linewise(&mut self) {
        self.line_buffer.move_to_line_start();
        let copy_range = self.line_buffer.insertion_point()..self.line_buffer.len();
        if copy_range.start < copy_range.end {
            let slice = &self.line_buffer.get_buffer()[copy_range];
            self.cut_buffer.set(slice, Granularity::LineWise);
        }
    }

    pub(crate) fn copy_to_line_end(&mut self) {
        let copy_range =
            self.line_buffer.insertion_point()..self.line_buffer.find_current_line_end();
        self.copy_range(copy_range);
    }

    pub(crate) fn copy_word_left(&mut self) {
        self.apply_operator(
            word_target(WordKind::Unicode, WordEdge::Start, Direction::Backward),
            OperatorVerb::Copy,
        );
    }

    pub(crate) fn copy_big_word_left(&mut self) {
        self.apply_operator(
            word_target(WordKind::LongWord, WordEdge::Start, Direction::Backward),
            OperatorVerb::Copy,
        );
    }

    pub(crate) fn copy_word_right(&mut self) {
        // emacs forward-word end (no skip) — mirrors `cut_word_right`.
        self.apply_operator(
            word_target(WordKind::Unicode, WordEdge::End, Direction::Forward),
            OperatorVerb::Copy,
        );
    }

    pub(crate) fn copy_big_word_right(&mut self) {
        self.apply_operator(
            word_target(WordKind::LongWord, WordEdge::End, Direction::Forward),
            OperatorVerb::Copy,
        );
    }

    pub(crate) fn copy_word_right_to_next(&mut self) {
        self.apply_operator(
            word_target(WordKind::Unicode, WordEdge::Start, Direction::Forward),
            OperatorVerb::Copy,
        );
    }

    pub(crate) fn copy_big_word_right_to_next(&mut self) {
        self.apply_operator(
            word_target(WordKind::LongWord, WordEdge::Start, Direction::Forward),
            OperatorVerb::Copy,
        );
    }

    pub(crate) fn copy_right_until_char(&mut self, c: char, before_char: bool, current_line: bool) {
        if let Some(index) = self.line_buffer.find_char_right(c, current_line) {
            let extra = if before_char { 0 } else { c.len_utf8() };
            let copy_range = self.line_buffer.insertion_point()..index + extra;
            self.copy_range(copy_range);
        }
    }

    pub(crate) fn copy_left_until_char(&mut self, c: char, before_char: bool, current_line: bool) {
        if let Some(index) = self.line_buffer.find_char_left(c, current_line) {
            let extra = if before_char { c.len_utf8() } else { 0 };
            let copy_range = index + extra..self.line_buffer.insertion_point();
            self.copy_range(copy_range);
        }
    }

    /// Expand the range to include `open_char` and `close_char`
    fn expand_range_to_include_pair(&self, range: Range<usize>) -> Option<Range<usize>> {
        let start = self.line_buffer.grapheme_left_index_from_pos(range.start);
        let end = self.line_buffer.grapheme_right_index_from_pos(range.end);

        Some(start..end)
    }
}

/// Replaces the selection, if any, then inserts the clipboard content before
/// the caret. Linewise content lands above the current line (vi `P`).
fn insert_clipboard_content_before(line_buffer: &mut LineBuffer, clipboard: &mut dyn Clipboard) {
    line_buffer.delete_selection();
    match clipboard.get() {
        (content, Granularity::CharWise) => {
            line_buffer.insert_str(&content);
        }
        (mut content, Granularity::LineWise) => {
            line_buffer.move_to_line_start();
            if !content.ends_with('\n') {
                content.push('\n');
            }
            line_buffer.insert_str(&content);
        }
    }
}

/// Replaces the selection, if any, then inserts the clipboard content after
/// the caret. Linewise content lands below the current line (vi `p`).
fn insert_clipboard_content_after(line_buffer: &mut LineBuffer, clipboard: &mut dyn Clipboard) {
    // After replacing a selection the cursor already sits at the deletion
    // point, so it must NOT skip a grapheme; only the plain no-selection `p`
    // steps past the grapheme under the cursor before inserting.
    let had_selection = line_buffer.selection_anchor().is_some();
    line_buffer.delete_selection();
    match clipboard.get() {
        (content, Granularity::CharWise) => {
            if !had_selection {
                line_buffer.move_right();
            }
            line_buffer.insert_str(&content);
        }
        (mut content, Granularity::LineWise) => {
            if !content.ends_with('\n') {
                content.push('\n');
            }
            let ip = line_buffer.insertion_point();
            match line::start_of_next_line(line_buffer.get_buffer(), ip) {
                // A line exists below: insert at its start so the pasted lines
                // land between current and next — i.e. below the current line.
                Some(next) => {
                    line_buffer.set_insertion_point(next);
                    line_buffer.insert_str(&content);
                }
                // Last line: no line below, so append after the current line's
                // terminator. Drop the trailing `\n` so no blank line is added,
                // otherwise the paste would land *above* (like `P`).
                None => {
                    let trimmed = content.strip_suffix('\n').unwrap_or(&content);
                    if line_buffer.is_empty() {
                        // No current line to append below — insert as-is so an
                        // empty buffer (e.g. after `dd` on the only line) does
                        // not gain a leading blank line.
                        line_buffer.insert_str(trimmed);
                    } else {
                        line_buffer.set_insertion_point(line_buffer.len());
                        line_buffer.insert_str(&format!("\n{trimmed}"));
                    }
                }
            }
        }
    }
}

