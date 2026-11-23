use {
    crate::{
        core_editor::{
            cursor::Cursor,
            graphemes::{next_grapheme_boundary, prev_grapheme_boundary},
            line, word,
        },
        enums::{Direction, WordEdge, WordKind},
    },
    std::{convert::From, ops::Range},
    unicode_segmentation::UnicodeSegmentation,
};

/// In memory representation of the entered line(s) including a cursor position to facilitate cursor based editing.
///
/// ## Line-ending contract
///
/// The buffer may contain `\r` — it is **not** guaranteed LF-only. Typing emits
/// `\n`, and the paste and history-recall boundaries normalize CRLF→LF, but
/// other entry points do not: the public `EditCommand::InsertString`, the
/// external-editor (`edit-command-line`) round-trip, and completer-supplied text
/// can all introduce `\r` (e.g. a `\r\n` from a Windows editor). Line, word, and
/// cursor logic therefore treats `\r`/`\r\n` as a terminator throughout (see
/// the `line` and `word` modules and the rest policies). Do not drop those
/// guards on the assumption that the buffer is CR-free.
#[derive(Debug, PartialEq, Eq, Clone, Default)]
pub struct LineBuffer {
    lines: String,
    /// The cursor as a (possibly empty) range. An empty cursor (anchor == head)
    /// is a plain insertion point; a wider one carries the active selection.
    /// Single source of truth — the old `insertion_point` + `selection_anchor`
    /// pair collapsed into this.
    cursor: Cursor,
}

impl From<&str> for LineBuffer {
    fn from(input: &str) -> Self {
        let mut line_buffer = LineBuffer::new();
        line_buffer.insert_str(input);
        line_buffer
    }
}

impl LineBuffer {
    /// Create a line buffer instance
    pub fn new() -> LineBuffer {
        Self::default()
    }

    /// Check to see if the line buffer is empty
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Check if the line buffer is valid utf-8 and the cursor sits on a valid grapheme boundary
    pub fn is_valid(&self) -> bool {
        self.lines.is_char_boundary(self.insertion_point())
            && (self
                .lines
                .grapheme_indices(true)
                .any(|(i, _)| i == self.insertion_point())
                || self.insertion_point() == self.lines.len())
            && std::str::from_utf8(self.lines.as_bytes()).is_ok()
    }


    /// Gets the current edit position — the caret, i.e. the left edge of the
    /// grapheme the cursor covers. For a point (Between) this equals the head;
    /// for a forward block/selection it is one grapheme inward from the head.
    pub fn insertion_point(&self) -> usize {
        self.cursor().caret(&self.lines)
    }

    /// Sets the current edit position. Does not touch the selection anchor —
    /// use [`clear_selection`](Self::clear_selection) explicitly when needed.
    ///
    /// ## Unicode safety:
    /// Not checked, improper use may cause panics in following operations
    pub fn set_insertion_point(&mut self, offset: usize) {
        self.set_head(offset);
    }

    /// Move the head to `head`, preserving whether a selection is active.
    ///
    /// Mirrors the old bare `self.insertion_point = head` field write, which
    /// left the anchor untouched: a point (no selection) drags its degenerate
    /// anchor along so it stays a point, while an active selection keeps its
    /// anchor fixed. ([`Cursor::move_head`] alone would turn a point into a
    /// spurious selection.)
    fn set_head(&mut self, head: usize) {
        self.cursor = if self.cursor.is_empty() {
            Cursor::point(head)
        } else {
            self.cursor.move_head(head)
        };
    }

    /// Returns the cursor as a [`Cursor`] (anchor + head), reflecting any
    /// active selection.
    pub(crate) fn cursor(&self) -> Cursor {
        self.cursor
    }

    /// Sets the cursor to the given [`Cursor`]. An empty cursor (anchor == head)
    /// is a plain insertion point; a non-empty cursor carries a selection.
    ///
    /// ## Unicode safety:
    /// Not checked, improper use may cause panics in following operations
    pub(crate) fn set_cursor(&mut self, cursor: Cursor) {
        self.cursor = cursor;
    }

    /// The current selection anchor, if any. `Some(pos)` while a selection is
    /// active; `None` otherwise.
    pub fn selection_anchor(&self) -> Option<usize> {
        (!self.cursor.is_empty()).then(|| self.cursor.anchor())
    }

    /// Sets the selection anchor without moving the cursor head.
    pub fn set_selection_anchor(&mut self, anchor: Option<usize>) {
        let head = self.cursor.head();
        self.cursor = match anchor {
            Some(anchor) => Cursor::new(anchor, head),
            None => Cursor::point(head),
        };
    }

    /// Clears any active selection. The cursor head is unchanged.
    pub fn clear_selection(&mut self) {
        self.cursor = Cursor::point(self.cursor.head());
    }

    /// Collapse any selection to a zero-width point at the **caret** — the
    /// visible edit position (left edge of the covered grapheme), not the head.
    ///
    /// Editing primitives (`delete_right_grapheme`, `insert_char`, …) anchor on
    /// the caret via [`insertion_point`](Self::insertion_point) but derive their
    /// other end from `head`; those agree only for a point cursor. Collapsing
    /// here before an in-place edit makes `head == caret` so the edit acts on the
    /// grapheme under the caret instead of overshooting into the selection. A
    /// no-op when the cursor is already a point.
    pub(crate) fn collapse_to_caret(&mut self) {
        self.cursor = Cursor::point(self.insertion_point());
    }

    /// The selected byte range, ascending, or `None` for a point cursor.
    pub(crate) fn get_selection(&self) -> Option<(usize, usize)> {
        // `None` exactly when the cursor is empty (head == anchor): with the
        // collapsed `Cursor` storage, `selection_anchor()` is derived from
        // `!is_empty()`, so an anchor on the head is simply no selection.
        self.selection_anchor()?;
        let cursor = self.cursor();

        // Inclusivity is geometric (widened by put_cursor).
        Some((cursor.start(), cursor.end().min(self.len())))
    }

    /// Deletes the selected text, if any, and leaves a point cursor where it was.
    ///
    /// Collapses with [`collapse_to_caret`](Self::collapse_to_caret), not
    /// [`clear_selection`](Self::clear_selection): that one collapses to the
    /// head, which under `Block` sits one grapheme past the visible position.
    pub(crate) fn delete_selection(&mut self) {
        if let Some((start, end)) = self.get_selection() {
            self.clear_range_safe(start..end);
            self.collapse_to_caret();
        }
    }

    /// Moves the cursor head to `pos`. If `select` is true, preserves any
    /// existing selection anchor (or plants one at the current head if none
    /// exists). If `select` is false, clears the selection.
    ///
    /// ## Unicode safety:
    /// Not checked, improper use may cause panics in following operations
    pub fn move_head(&mut self, pos: usize, select: bool) {
        // `select` on a point uses `move_head`, which plants the anchor at the
        // old head (the degenerate anchor) and so opens a selection — exactly
        // the old "plant anchor at insertion_point if none" behavior.
        self.cursor = if select {
            self.cursor.move_head(pos)
        } else {
            Cursor::point(pos)
        };
    }

    /// Output the current line in the multiline buffer
    pub fn get_buffer(&self) -> &str {
        &self.lines
    }

    /// Set to a single line of `buffer` and reset the `InsertionPoint` cursor to the end
    pub fn set_buffer(&mut self, buffer: String) {
        self.lines = buffer;
        self.set_head(self.lines.len());
    }

    /// Calculates the current the user is on
    ///
    /// Zero-based index
    pub fn line(&self) -> usize {
        self.lines[..self.cursor.head()].matches('\n').count()
    }

    /// Calculates the grapheme index in the line the cursor head is on.
    ///
    /// Zero-based index
    pub fn col(&self) -> usize {
        self.lines[self.line_start_index()..self.cursor.head()]
            .grapheme_indices(true)
            .count()
    }

    /// Counts the number of lines in the buffer
    pub fn num_lines(&self) -> usize {
        self.lines.split('\n').count()
    }

    /// Checks to see if the buffer ends with a given character
    pub fn ends_with(&self, c: char) -> bool {
        self.lines.ends_with(c)
    }

    /// Reset the insertion point to the start of the buffer
    pub fn move_to_start(&mut self) {
        self.set_head(0);
    }

    /// Byte offset of the first character on the line containing the cursor.
    ///
    /// Returns 0 for the first line. Pure accessor — does not mutate state.
    pub fn line_start_index(&self) -> usize {
        line::start_of_line(&self.lines, self.cursor.head())
    }

    /// Move the cursor before the first character of the line
    pub fn move_to_line_start(&mut self) {
        self.set_head(self.line_start_index());
    }

    /// Byte offset of the first non-whitespace character on the line
    /// containing the cursor. If the line is entirely blank, returns the
    /// position of its terminating `\n`; if no further non-whitespace or
    /// newline exists, returns the buffer length. Pure accessor — does not
    /// mutate state.
    pub fn line_non_blank_start_index(&self) -> usize {
        let line_start = self.line_start_index();
        self.lines[line_start..]
            .find(|c: char| !c.is_whitespace() || c == '\n')
            .map_or(self.lines.len(), |offset| line_start + offset)
    }

    /// Move the cursor before the first non whitespace character of the line
    pub fn move_to_line_non_blank_start(&mut self) {
        self.set_head(self.line_non_blank_start_index());
    }

    /// Move cursor position to the end of the line
    ///
    /// Insertion will append to the line.
    /// Cursor on top of the potential `\n` or `\r` of `\r\n`
    pub fn move_to_line_end(&mut self) {
        self.set_head(self.find_current_line_end());
    }

    /// Set the insertion point *behind* the last character.
    pub fn move_to_end(&mut self) {
        self.set_head(self.lines.len());
    }

    /// Get the length of the buffer
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// Returns where the current line terminates
    ///
    /// Either:
    /// - end of buffer (`len()`)
    /// - `\n` or `\r\n` (on the first byte)
    pub fn find_current_line_end(&self) -> usize {
        line::end_of_line(&self.lines, self.cursor.head())
    }

    /// Cursor position *behind* the next unicode grapheme to the right
    pub fn grapheme_right_index(&self) -> usize {
        self.grapheme_right_index_from_pos(self.cursor.head())
    }

    /// Cursor position *in front of* the next unicode grapheme to the left
    pub fn grapheme_left_index(&self) -> usize {
        self.grapheme_left_index_from_pos(self.cursor.head())
    }

    /// Cursor position *behind* the next unicode grapheme to the right from the given position
    pub fn grapheme_right_index_from_pos(&self, pos: usize) -> usize {
        next_grapheme_boundary(&self.lines, pos)
    }

    /// Cursor position *behind* the previous unicode grapheme to the left from the given position
    pub(crate) fn grapheme_left_index_from_pos(&self, pos: usize) -> usize {
        prev_grapheme_boundary(&self.lines, pos)
    }

    /// Resolve a word boundary from the cursor through the shared
    /// [`word::locate_word`] resolver — the single definition the old per-flavor
    /// `*_index` scans collapsed into. Returns the bar boundary; these buffer
    /// edits work in gap (bar) terms, so caret geometry doesn't apply.
    fn word_boundary(&self, kind: WordKind, edge: WordEdge, direction: Direction) -> usize {
        word::locate_word(&self.lines, self.cursor.head(), kind, edge, direction)
    }

    /// Returns true if cursor is at the end of the buffer with preceding whitespace.
    fn at_end_of_line_with_preceding_whitespace(&self) -> bool {
        !self.is_empty() // No point checking if empty
        && self.cursor.head() == self.lines.len()
        && self.lines.chars().last().is_some_and(|c| c.is_whitespace())
    }

    /// Cursor position at the end of the current whitespace block.
    fn current_whitespace_end_index(&self) -> usize {
        self.lines[self.cursor.head()..]
            .char_indices()
            .find(|(_, ch)| !ch.is_whitespace())
            .map(|(i, _)| self.cursor.head() + i)
            .unwrap_or(self.lines.len())
    }

    /// Cursor position at the start of the current whitespace block.
    fn current_whitespace_start_index(&self) -> usize {
        self.lines[..self.cursor.head()]
            .char_indices()
            .rev()
            .find(|(_, ch)| !ch.is_whitespace())
            .map(|(i, _)| i + 1)
            .unwrap_or(0)
    }

    /// Returns the range of consecutive whitespace characters that includes
    /// the cursor position. If cursor is at the end of trailing whitespace, includes
    /// that trailing block. Return None if no surrounding whitespace.
    pub(crate) fn current_whitespace_range(&self) -> Option<Range<usize>> {
        let range_start = self.current_whitespace_start_index();
        if self.on_whitespace() {
            let range_end = self.current_whitespace_end_index();
            Some(range_start..range_end)
        } else if self.at_end_of_line_with_preceding_whitespace() {
            Some(range_start..self.cursor.head())
        } else {
            None
        }
    }

    /// Move cursor position *behind* the next unicode grapheme to the right
    pub fn move_right(&mut self) {
        self.set_head(self.grapheme_right_index());
    }

    /// Move cursor position *in front of* the next unicode grapheme to the left
    pub fn move_left(&mut self) {
        self.set_head(self.grapheme_left_index());
    }

    ///Insert a single character at the insertion point and move right
    pub fn insert_char(&mut self, c: char) {
        self.lines.insert(self.cursor.head(), c);
        self.move_right();
    }

    /// Insert `&str` at the cursor position in the current line.
    ///
    /// Sets cursor to end of inserted string
    ///
    /// ## Unicode safety:
    /// Does not validate the incoming string or the current cursor position
    pub fn insert_str(&mut self, string: &str) {
        self.lines.insert_str(self.insertion_point(), string);
        self.set_head(self.insertion_point() + string.len());
    }

    /// Inserts a newline character (`'\n'`) into the buffer at the current
    /// insertion point.
    ///
    /// Only LF is inserted regardless of platform. The painting layer
    /// (`coerce_crlf`) is responsible for converting LF to CRLF when
    /// writing to the terminal in raw mode.
    pub fn insert_newline(&mut self) {
        self.insert_char('\n');
    }

    /// Empty buffer and reset cursor
    pub fn clear(&mut self) {
        self.lines = String::new();
        self.set_head(0);
    }

    /// Clear everything beginning at the cursor to the right/end.
    /// Keeps the cursor at the end.
    pub fn clear_to_end(&mut self) {
        // Truncate at the caret (the visible edit position), not the head: the
        // callers (`cut_from_end`) copy the slice from `insertion_point()`, so a
        // Block/selection cursor (head past the caret) would otherwise leave the
        // covered grapheme behind while reporting it as cut.
        self.lines.truncate(self.insertion_point());
    }

    /// Clear beginning at the cursor up to the end of the line.
    /// Newline character at the end remains.
    pub fn clear_to_line_end(&mut self) {
        // Caret-anchored to match the slice `cut_to_line_end` copies; see
        // `clear_to_end`.
        self.clear_range(self.insertion_point()..self.find_current_line_end());
    }

    /// Clear from the start of the buffer to the cursor.
    /// Keeps the cursor at the beginning of the line/buffer.
    pub fn clear_to_insertion_point(&mut self) {
        self.clear_range(..self.cursor.head());
        self.set_head(0);
    }

    /// Clear all contents between `start` and `end` and change insertion point if necessary.
    ///
    /// If the cursor is located between `start` and `end` it is adjusted to `start`.
    /// If the cursor is located after `end` it is adjusted to stay at its current char boundary.
    pub fn clear_range_safe(&mut self, range: Range<usize>) {
        let (start, end) = if range.start > range.end {
            (range.end, range.start)
        } else {
            (range.start, range.end)
        };
        if self.cursor.head() <= start {
            // No action necessary
        } else if self.cursor.head() < end {
            self.set_head(start);
        } else {
            // Insertion point after end
            self.set_head(self.cursor.head() - (end - start));
        }
        self.clear_range(start..end);
    }

    /// Clear text covered by `range` in the current line
    ///
    /// Safety: Does not change the insertion point/offset and is thus not unicode safe!
    pub(crate) fn clear_range<R>(&mut self, range: R)
    where
        R: std::ops::RangeBounds<usize>,
    {
        self.replace_range(range, "");
    }

    /// Substitute text covered by `range` in the current line
    ///
    /// Safety: Does not change the insertion point/offset and is thus not unicode safe!
    pub fn replace_range<R>(&mut self, range: R, replace_with: &str)
    where
        R: std::ops::RangeBounds<usize>,
    {
        self.lines.replace_range(range, replace_with);
    }

    /// Checks to see if the current edit position is pointing to whitespace
    pub fn on_whitespace(&self) -> bool {
        self.lines[self.cursor.head()..]
            .chars()
            .next()
            .map(char::is_whitespace)
            .unwrap_or(false)
    }

    /// Get the grapheme immediately to the right of the cursor, if any
    pub fn grapheme_right(&self) -> &str {
        &self.lines[self.cursor.head()..self.grapheme_right_index()]
    }

    /// Get the grapheme immediately to the left of the cursor, if any
    pub fn grapheme_left(&self) -> &str {
        &self.lines[self.grapheme_left_index()..self.cursor.head()]
    }

    /// Gets the range of the word the current edit position is pointing to
    pub fn current_word_range(&self) -> Range<usize> {
        // Trailing boundary of the word under/after the cursor (bar geometry).
        let right_index = self.word_boundary(WordKind::Unicode, WordEdge::End, Direction::Forward);
        let left_index = self.lines[..right_index]
            .split_word_bound_indices()
            .rfind(|(_, word)| !is_whitespace_str(word))
            .map(|(i, _)| i)
            .unwrap_or(0);

        left_index..right_index
    }

    /// Range over the line containing byte `pos`.
    ///
    /// Starts on the first character after the previous line feed and is an
    /// exclusive range extending beyond the carriage return and line feed
    /// characters terminating the line.
    fn line_range_at(&self, pos: usize) -> Range<usize> {
        let start = self.lines[..pos].rfind('\n').map_or(0, |offset| offset + 1);
        let end = self.lines[pos..]
            .find('\n')
            .map_or_else(|| self.lines.len(), |i| i + pos + 1);
        start..end
    }

    /// Range over the current line (the one the cursor head is on). See
    /// `line_range_at`.
    pub fn current_line_range(&self) -> Range<usize> {
        self.line_range_at(self.cursor.head())
    }

    /// Replaces the whole word under the cursor with `change` of it and moves
    /// the insertion point past it.
    fn map_current_word(&mut self, change: impl FnOnce(&str) -> String) {
        let change_range = self.current_word_range();
        let changed = change(&self.get_buffer()[change_range.clone()]);
        self.replace_range(change_range, &changed);
        self.set_head(self.word_boundary(WordKind::Unicode, WordEdge::End, Direction::Forward));
    }

    /// Uppercases the current word
    pub fn uppercase_word(&mut self) {
        self.map_current_word(str::to_uppercase);
    }

    /// Lowercases the current word
    pub fn lowercase_word(&mut self) {
        self.map_current_word(str::to_lowercase);
    }

    /// Capitalizes the current word: upper cases its first character and
    /// lower cases the rest. Like [`Self::uppercase_word`] it acts on the
    /// whole word under the cursor and moves the insertion point past it.
    pub fn capitalize_word(&mut self) {
        self.map_current_word(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => {
                    first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
                }
                None => String::new(),
            }
        });
    }

    /// Switches the ASCII case of the current char
    pub fn switchcase_char(&mut self) {
        let insertion_offset = self.insertion_point();
        let right_index = self.grapheme_right_index();

        if right_index > insertion_offset {
            let change_range = insertion_offset..right_index;
            let swapped = self.get_buffer()[change_range.clone()]
                .chars()
                .map(|c| {
                    if c.is_ascii_uppercase() {
                        c.to_ascii_lowercase()
                    } else {
                        c.to_ascii_uppercase()
                    }
                })
                .collect::<String>();
            self.replace_range(change_range, &swapped);
            self.move_right();
        }
    }

    /// Capitalize the character at insertion point (or the first character
    /// following the whitespace at the insertion point) and move the insertion
    /// point right one grapheme.
    pub fn capitalize_char(&mut self) {
        if self.on_whitespace() {
            self.set_head(self.word_boundary(WordKind::Unicode, WordEdge::End, Direction::Forward));
            self.set_head(self.word_boundary(
                WordKind::Unicode,
                WordEdge::Start,
                Direction::Backward,
            ));
        }
        let insertion_offset = self.insertion_point();
        let right_index = self.grapheme_right_index();

        if right_index > insertion_offset {
            let change_range = insertion_offset..right_index;
            let uppercased = self.get_buffer()[change_range.clone()].to_uppercase();
            self.replace_range(change_range, &uppercased);
            self.move_right();
        }
    }

    /// Deletes on grapheme to the left
    pub fn delete_left_grapheme(&mut self) {
        let left_index = self.grapheme_left_index();
        let insertion_offset = self.insertion_point();
        if left_index < insertion_offset {
            self.clear_range(left_index..insertion_offset);
            self.set_head(left_index);
        }
    }

    /// Deletes one grapheme to the right
    pub fn delete_right_grapheme(&mut self) {
        let right_index = self.grapheme_right_index();
        let insertion_offset = self.insertion_point();
        if right_index > insertion_offset {
            self.clear_range(insertion_offset..right_index);
        }
    }

    /// Deletes one word to the left
    pub fn delete_word_left(&mut self) {
        let left_word_index =
            self.word_boundary(WordKind::Unicode, WordEdge::Start, Direction::Backward);
        self.clear_range(left_word_index..self.insertion_point());
        self.set_head(left_word_index);
    }

    /// Deletes one word to the right
    pub fn delete_word_right(&mut self) {
        let right_word_index =
            self.word_boundary(WordKind::Unicode, WordEdge::End, Direction::Forward);
        self.clear_range(self.insertion_point()..right_word_index);
    }

    /// Swaps current word with word on right
    pub fn swap_words(&mut self) {
        let word_1_range = self.current_word_range();
        self.set_head(self.word_boundary(WordKind::Unicode, WordEdge::End, Direction::Forward));
        let word_2_range = self.current_word_range();

        if word_1_range != word_2_range {
            self.set_head(self.word_boundary(
                WordKind::Unicode,
                WordEdge::Start,
                Direction::Backward,
            ));
            let insertion_line = self.get_buffer();
            let word_1 = insertion_line[word_1_range.clone()].to_string();
            let word_2 = insertion_line[word_2_range.clone()].to_string();
            self.replace_range(word_2_range, &word_1);
            self.replace_range(word_1_range, &word_2);
        }
    }

    /// Swaps current grapheme with grapheme on right
    pub fn swap_graphemes(&mut self) {
        let initial_offset = self.insertion_point();

        if initial_offset == 0 {
            self.move_right();
        } else if initial_offset == self.get_buffer().len() {
            self.move_left();
        }

        let updated_offset = self.insertion_point();
        let grapheme_1_start = self.grapheme_left_index();
        let grapheme_2_end = self.grapheme_right_index();

        if grapheme_1_start < updated_offset && grapheme_2_end > updated_offset {
            let grapheme_1 = self.get_buffer()[grapheme_1_start..updated_offset].to_string();
            let grapheme_2 = self.get_buffer()[updated_offset..grapheme_2_end].to_string();
            self.replace_range(updated_offset..grapheme_2_end, &grapheme_1);
            self.replace_range(grapheme_1_start..updated_offset, &grapheme_2);
            self.set_head(grapheme_2_end);
        } else {
            self.set_head(updated_offset);
        }
    }

    /// Visual column of the caret — graphemes from its line start to the caret.
    fn caret_column(&self) -> (Range<usize>, usize) {
        let caret = self.insertion_point();
        let line = self.line_range_at(caret);
        let col = self.lines[line.start..caret].graphemes(true).count();
        (line, col)
    }

    /// Byte offset one line up at the caret's column, or `None` on the first
    /// line. Pure: vertical motion preserves the *caret*'s visual column, so the
    /// column is measured from the caret and the previous line is found by
    /// arithmetic — no cursor mutation, and no head-vs-caret straddle to panic on.
    pub(crate) fn line_up_target(&self) -> Option<usize> {
        if self.is_cursor_at_first_line() {
            return None;
        }
        let (cur, col) = self.caret_column();
        // The previous line is the one ending at this line's start; step back
        // over the `\n` (or `\r\n`) to land in it.
        let prev = self.line_range_at(prev_grapheme_boundary(&self.lines, cur.start));
        let line = &self.lines[prev.clone()];
        // Clamp to the line's last grapheme when it is shorter than the column.
        Some(
            line.grapheme_indices(true)
                .take(col + 1)
                .last()
                .map_or(prev.start, |(i, _)| i + prev.start),
        )
    }

    /// Byte offset one line down at the caret's column, or `None` on the last
    /// line. See [`line_up_target`](Self::line_up_target).
    pub(crate) fn line_down_target(&self) -> Option<usize> {
        if self.is_cursor_at_last_line() {
            return None;
        }
        let (cur, col) = self.caret_column();
        // The current line's exclusive range already starts the next line.
        let next = self.line_range_at(cur.end);
        let line = &self.lines[next.clone()];
        // A short last line (no trailing `\n`) falls back to its content end.
        Some(line.grapheme_indices(true).nth(col).map_or_else(
            || line::end_of_line(&self.lines, next.start),
            |(i, _)| i + next.start,
        ))
    }

    /// Moves one line up, preserving the caret's column.
    pub fn move_line_up(&mut self) {
        if let Some(target) = self.line_up_target() {
            self.set_head(target);
        }
    }

    /// Moves one line down, preserving the caret's column.
    pub fn move_line_down(&mut self) {
        if let Some(target) = self.line_down_target() {
            self.set_head(target);
        }
    }

    /// Checks to see if the cursor is on the first line of the buffer
    pub fn is_cursor_at_first_line(&self) -> bool {
        !self.get_buffer()[0..self.insertion_point()].contains('\n')
    }

    /// Checks to see if the cursor is on the last line of the buffer
    pub fn is_cursor_at_last_line(&self) -> bool {
        !self.get_buffer()[self.insertion_point()..].contains('\n')
    }

    /// Finds index for the first occurrence of a char to the right of offset
    pub fn find_char_right(&self, c: char, current_line: bool) -> Option<usize> {
        // Skip current grapheme
        let char_offset = self.grapheme_right_index();
        let range = if current_line {
            char_offset..self.current_line_range().end
        } else {
            char_offset..self.lines.len()
        };
        self.lines[range].find(c).map(|index| index + char_offset)
    }

    /// Finds index for the first occurrence of a char to the left of offset
    pub fn find_char_left(&self, c: char, current_line: bool) -> Option<usize> {
        let range = if current_line {
            self.current_line_range().start..self.insertion_point()
        } else {
            0..self.insertion_point()
        };
        self.lines[range.clone()].rfind(c).map(|i| i + range.start)
    }

    /// Moves the insertion point until the next char to the right
    pub fn move_right_until(&mut self, c: char, current_line: bool) -> usize {
        if let Some(index) = self.find_char_right(c, current_line) {
            self.set_head(index);
        }

        self.cursor.head()
    }

    /// Moves the insertion point before the next char to the right
    pub fn move_right_before(&mut self, c: char, current_line: bool) -> usize {
        if let Some(index) = self.find_char_right(c, current_line) {
            self.set_head(index);
            self.set_head(self.grapheme_left_index());
        }

        self.cursor.head()
    }

    /// Moves the insertion point until the next char to the left of offset
    pub fn move_left_until(&mut self, c: char, current_line: bool) -> usize {
        if let Some(index) = self.find_char_left(c, current_line) {
            self.set_head(index);
        }

        self.cursor.head()
    }

    /// Moves the insertion point before the next char to the left of offset
    pub fn move_left_before(&mut self, c: char, current_line: bool) -> usize {
        if let Some(index) = self.find_char_left(c, current_line) {
            self.set_head(index + c.len_utf8());
        }

        self.cursor.head()
    }

    /// Deletes until first character to the right of offset
    pub fn delete_right_until_char(&mut self, c: char, current_line: bool) {
        if let Some(index) = self.find_char_right(c, current_line) {
            self.clear_range(self.insertion_point()..index + c.len_utf8());
        }
    }

    /// Deletes before first character to the right of offset
    pub fn delete_right_before_char(&mut self, c: char, current_line: bool) {
        if let Some(index) = self.find_char_right(c, current_line) {
            self.clear_range(self.insertion_point()..index);
        }
    }

    /// Deletes until first character to the left of offset
    pub fn delete_left_until_char(&mut self, c: char, current_line: bool) {
        if let Some(index) = self.find_char_left(c, current_line) {
            self.clear_range(index..self.insertion_point());
            self.set_head(index);
        }
    }

    /// Deletes before first character to the left of offset
    pub fn delete_left_before_char(&mut self, c: char, current_line: bool) {
        if let Some(index) = self.find_char_left(c, current_line) {
            self.clear_range(index + c.len_utf8()..self.insertion_point());
            self.set_head(index + c.len_utf8());
        }
    }

    /// Returns `Some(Range<usize>)` for the range inside the surrounding
    /// `open_char` and `close_char`, or `None` if no pair is found.
    ///
    /// If cursor is positioned just before an opening character, treat it as
    /// being "inside" that pair.
    ///
    /// For symmetric characters (e.g. quotes), the search is restricted to the current line only.
    /// For asymmetric characters (e.g. brackets), the search spans the entire buffer.
    pub(crate) fn range_inside_current_pair(
        &self,
        open_char: char,
        close_char: char,
    ) -> Option<Range<usize>> {
        let only_search_current_line: bool = open_char == close_char;
        let find_range_between_pair_at_position = |pos| {
            self.range_between_matching_pair_at_pos(
                pos,
                only_search_current_line,
                open_char,
                close_char,
            )
        };

        // First try to find pair from current cursor position
        find_range_between_pair_at_position(self.cursor.head()).or_else(|| {
            // Second try, if cursor is positioned just before an opening character,
            // treat it as being "inside" that pair and try from the next position
            self.grapheme_right()
                .starts_with(open_char)
                .then(|| find_range_between_pair_at_position(self.grapheme_right_index()))
                .flatten()
        })
    }

    /// Returns `Some(Range<usize>)` for the range inside the next pair
    /// or `None` if no pair is found
    ///
    /// Search forward from the cursor to find the next occurrence of `open_char`
    /// (including char at cursors current position), then finds its matching
    /// `close_char` and returns the range of text inside those characters.
    /// Note the end of Range is exclusive so the end of the range returned so
    /// the end of the range is index of final char + 1.
    ///
    /// For symmetric characters (e.g. quotes), the search is restricted to the current line only.
    /// For asymmetric characters (e.g. brackets), the search spans the entire buffer.
    pub(crate) fn range_inside_next_pair(
        &self,
        open_char: char,
        close_char: char,
    ) -> Option<Range<usize>> {
        let only_search_current_line: bool = open_char == close_char;

        // Find the next opening character, including the current position
        let open_pair_index = if self.grapheme_right().starts_with(open_char) {
            self.cursor.head()
        } else {
            self.find_char_right(open_char, only_search_current_line)?
        };

        self.range_between_matching_pair_at_pos(
            self.grapheme_right_index_from_pos(open_pair_index),
            only_search_current_line,
            open_char,
            close_char,
        )
    }

    /// Returns `Some(Range<usize>)` for the range inside the pair `open_char`
    /// and `close_char` surrounding the cursor position NOT including the character
    /// at the current cursor position, or `None` if no valid pair is found.
    ///
    /// This is the underlying algorithm used by both `range_inside_current_pair` and
    /// `range_inside_next_pair`.
    /// It uses a forward-first search approach:
    /// 1. Search forward from cursor to find the closing character (ignoring nested pairs)
    /// 2. Search backward from closing to find the matching opening character
    /// 3. Return the range between them
    fn range_between_matching_pair_at_pos(
        &self,
        position: usize,
        only_search_current_line: bool,
        open_char: char,
        close_char: char,
    ) -> Option<Range<usize>> {
        let search_range = if only_search_current_line {
            self.current_line_range()
        } else {
            0..self.lines.len()
        };

        let after_cursor = &self.lines[position..search_range.end];
        let close_pair_index_after_cursor =
            Self::find_index_of_matching_pair(after_cursor, open_char, close_char, false)?;
        let close_char_index_in_buffer = position + close_pair_index_after_cursor;

        let start_to_close_char = &self.lines[search_range.start..close_char_index_in_buffer];

        Self::find_index_of_matching_pair(start_to_close_char, open_char, close_char, true).map(
            |open_char_index_from_start| {
                let open_char_index_in_buffer = search_range.start + open_char_index_from_start;
                (open_char_index_in_buffer + open_char.len_utf8())..close_char_index_in_buffer
            },
        )
    }

    /// Find the index of a pair character that matches the nesting depth at the
    /// start or end of `slice` using depth counting to handle nested pairs.
    /// Helper for [`LineBuffer::range_between_matching_pair_at_pos`]
    ///
    /// If `search_backwards` is false:
    /// Find close_char at same level of nesting as start of slice.
    ///
    /// If `search_backwards` is true:
    /// Find open_char at same level of nesting as end of slice.
    ///
    /// Returns index of the open or closing character that matches the start of slice,
    /// or `None` if not found.
    fn find_index_of_matching_pair(
        slice: &str,
        open_char: char,
        close_char: char,
        search_backwards: bool,
    ) -> Option<usize> {
        let mut depth = 0;
        let mut graphemes: Vec<(usize, &str)> = slice.grapheme_indices(true).collect();

        if search_backwards {
            graphemes.reverse();
        }

        let (target, increment) = if search_backwards {
            (open_char, close_char)
        } else {
            (close_char, open_char)
        };

        for (index, grapheme) in graphemes {
            if let Some(char) = grapheme.chars().next() {
                if char == target {
                    if depth == 0 {
                        return Some(index);
                    }
                    depth -= 1;
                } else if char == increment && index > 0 {
                    depth += 1;
                }
            }
        }
        None
    }

    /// Returns `Some(Range<usize>)` for the range inside pair in `pair_group`
    /// at cursor position including pair of character at current cursor position,
    /// or `None` if cursor is not inside or at a pair included in `pair_group.
    ///
    /// If the opening and closing char in the pair are equal then search is
    /// restricted to the current line.
    ///
    /// If multiple pair types are found in the buffer or line, return the innermost
    /// pair that surrounds the cursor. Handles empty quotes as zero-length ranges inside quote.
    pub(crate) fn range_inside_current_pair_in_group(
        &self,
        matching_pair_group: &[(char, char)],
    ) -> Option<Range<usize>> {
        matching_pair_group
            .iter()
            .filter_map(|(open_char, close_char)| {
                self.range_inside_current_pair(*open_char, *close_char)
            })
            .min_by_key(|range| range.len())
    }

    /// Returns `Some(Range<usize>)` for the range inside the next pair in `pair_group`
    /// or `None` if cursor is not inside a pair included in `pair_group`.
    ///
    /// If the opening and closing char in the pair are equal then search is
    /// restricted to the current line.
    ///
    /// If multiple pair types are found in the buffer or line, return the innermost
    /// pair that surrounds the cursor. Handles empty pairs as zero-length ranges
    /// inside pair (this enables caller to still get the location of the pair).
    pub(crate) fn range_inside_next_pair_in_group(
        &self,
        matching_pair_group: &[(char, char)],
    ) -> Option<Range<usize>> {
        matching_pair_group
            .iter()
            .filter_map(|(open_char, close_char)| {
                self.range_inside_next_pair(*open_char, *close_char)
            })
            .min_by_key(|range| range.start)
    }

    /// Get the range of the current big word (WORD) at cursor position
    pub(crate) fn current_big_word_range(&self) -> Range<usize> {
        // Trailing boundary of the big WORD under/after the cursor (grapheme-safe
        // end, unlike the old `*_end_index + 1` which stepped one *byte*).
        let end = self.word_boundary(WordKind::LongWord, WordEdge::End, Direction::Forward);

        let mut left_index = 0;
        for (i, char) in self.lines[..end].char_indices().rev() {
            if char.is_whitespace() {
                left_index = i + char.len_utf8();
                break;
            }
        }
        left_index..end
    }

    /// Return range of `range` expanded with neighbouring whitespace for "around" operations
    /// Prioritizes whitespace after the word, falls back to whitespace before if none after
    pub(crate) fn expand_range_with_whitespace(&self, range: Range<usize>) -> Range<usize> {
        let end = self.next_non_whitespace_index(range.end);
        let start = if end == range.end {
            self.prev_non_whitespace_index(range.start)
        } else {
            range.start
        };
        start..end
    }

    /// Return next non-whitespace character index after `pos`
    fn next_non_whitespace_index(&self, pos: usize) -> usize {
        self.lines[pos..]
            .char_indices()
            .find(|(_, char)| !char.is_whitespace())
            .map_or(self.lines.len(), |(i, _)| pos + i)
    }

    /// Extend range leftward to include leading whitespace
    fn prev_non_whitespace_index(&self, pos: usize) -> usize {
        self.lines[..pos]
            .char_indices()
            .rev()
            .find(|(_, char)| !char.is_whitespace())
            .map_or(0, |(i, char)| i + char.len_utf8())
    }
}

/// Match any sequence of characters that are considered a word boundary
pub(crate) fn is_whitespace_str(s: &str) -> bool {
    s.chars().all(char::is_whitespace)
}

