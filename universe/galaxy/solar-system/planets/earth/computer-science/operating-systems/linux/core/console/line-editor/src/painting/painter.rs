use crate::painting::utils::estimate_required_lines;
use crate::terminal_extensions::semantic_prompt::{PromptKind, SemanticPromptMarkers};
use crate::{CursorConfig, PromptEditMode};


use {
    super::utils::{
        advance_grapheme, coerce_crlf, deferred_wrap_row, escape_control, line_width,
        plain_menu_text, resolve_wrap, wrap_position,
    },
    crate::{
        menu::{Menu, ReedlineMenu},
        painting::PromptLines,
        utils::environment::{term_is_dumb, var_os},
        Prompt,
    },
    crossterm::{
        cursor::{self, MoveTo, RestorePosition, SavePosition},
        style::{Attribute, Print, ResetColor, SetAttribute},
        terminal::{self, BeginSynchronizedUpdate, Clear, ClearType, EndSynchronizedUpdate},
        QueueableCommand,
    },
    std::io::{Result, Write},
    std::ops::RangeInclusive,
    unicode_segmentation::UnicodeSegmentation,
};

// Returns a string that skips N number of lines with the next offset of lines
// An offset of 0 would return only one line after skipping the required lines
fn skip_buffer_lines(string: &str, skip: usize, offset: Option<usize>) -> &str {
    let (index, limit) = skip_buffer_lines_range(string, skip, offset);

    string[index..limit].trim_end_matches('\n')
}

// `text` of the buffer as it is drawn: the `\r` of a CRLF is the line break's
// own and draws nothing, every other control char shows escaped.
fn escape_as_drawn(text: &str) -> std::borrow::Cow<'_, str> {
    if text.contains("\r\n") {
        escape_control(&text.replace("\r\n", "\n"))
            .into_owned()
            .into()
    } else {
        escape_control(text)
    }
}

// The byte range `skip_buffer_lines` slices, for callers that need to map a
// position back into the unskipped string.
fn skip_buffer_lines_range(string: &str, skip: usize, offset: Option<usize>) -> (usize, usize) {
    let mut matches = string.match_indices('\n');
    let index = if skip == 0 {
        0
    } else {
        matches
            .clone()
            .nth(skip - 1)
            .map(|(index, _)| index + 1)
            .unwrap_or(string.len())
    };

    let limit = match offset {
        Some(offset) => {
            let offset = skip + offset;
            matches
                .nth(offset)
                .map(|(index, _)| index)
                .unwrap_or(string.len())
        }
        None => string.len(),
    };

    (index, limit)
}

/// The writer used by crossterm operations.
///
/// In production this is a buffered stderr handle. During tests it can be
/// backed by a sink, so painting runs normally without spilling escape
/// sequences onto the real terminal: crossterm writes straight to the file
/// descriptor, which bypasses libtest's output capture.
pub enum W {
    /// Buffered stderr — the real terminal.
    // Constructed only in non-test builds; under `cfg(test)` we always use `Sink`.
    #[cfg_attr(test, allow(dead_code))]
    Terminal(std::io::BufWriter<std::io::Stderr>),
    /// Discards all output, used in tests. The second field is the answer to
    /// give a cursor query, or `None` to refuse one.
    /// Captures all output into a buffer so tests can assert on the exact
    /// escape-byte stream the painter emits (not tmux-specific — any
    /// output-level invariant).
    /// Fails once after the given number of bytes, then captures later writes.
    /// Captures like [`W::Capture`], and answers a cursor query from a
    /// [`TestScreen`] fed on each flush, so the answer reflects what a
    /// terminal behind a buffered writer has actually received.
}

impl W {
    /// Writer targeting the real terminal (buffered stderr).
    #[cfg_attr(test, allow(dead_code))]
    pub(crate) fn terminal() -> Self {
        W::Terminal(std::io::BufWriter::new(std::io::stderr()))
    }

    /// Writer that discards everything, for tests that exercise painting
    /// without printing to the terminal.

    /// Like [`W::sink`], but answers a cursor query with `position` rather than
    /// refusing, so a test can drive the paths that measure the cursor instead
    /// of only their "no answer" branch.

    /// Writer that buffers everything written to it, so a test can inspect the
    /// emitted bytes after painting.

    /// Like [`W::capture`], but answering cursor queries from a
    /// `width` by `height` screen; see [`W::Screen`].

    /// The screen a [`W::screen`] writer answers from, to feed it output that
    /// did not come from the painter. Panics on any other writer.

    /// Bytes captured so far. Panics unless this is a [`W::capture`] or
    /// [`W::screen`] writer.
}

impl Write for W {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        match self {
            W::Terminal(w) => w.write(buf),
        }
    }

    fn flush(&mut self) -> Result<()> {
        match self {
            W::Terminal(w) => w.flush(),
        }
    }
}

impl W {
    /// Where the terminal's cursor is, as `(column, row)`.
    ///
    /// The real terminal answers for real, and [`W::screen`] from its emulated
    /// screen. Other test writers answer with whatever
    /// [`W::sink_with_cursor_at`] gave them, and otherwise return an error so
    /// paint paths take their "no answer" branch instead of waiting on a tty
    /// that will never reply.
    pub(crate) fn cursor_position(&self) -> Result<(u16, u16)> {
        match self {
            W::Terminal(_) => cursor::position(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct PainterSuspendedState {
    previous_prompt_rows_range: RangeInclusive<u16>,
    /// Whether the prompt reached the last row of the screen it was captured on.
    /// Recorded here rather than tested at re-use, since by then the screen may have
    /// been resized by whatever ran in between.
    was_flush_at_bottom: bool,
    /// The cell the cursor sat on when the painter was suspended, `None` when the
    /// terminal could not be queried. An in-place return lands on exactly this
    /// cell again; a program that scrolled the screen does not.
    cursor: Option<(u16, u16)>,
}

impl PainterSuspendedState {
    /// Whether the prompt captured in this state can be drawn again where it was,
    /// given the cell the cursor is on now that the host has returned.
    ///
    /// Above the bottom row the stored range decides on its own, as it always has.
    /// A prompt that sat flush against the bottom is the ambiguous case from
    /// nushell/reedline#1130: a program that scrolled the terminal also returns
    /// inside the stored range. The recorded `cursor` tells the two apart, and
    /// nushell/reedline#1196 is what happens when it is not consulted. A cursor
    /// that could not be recorded counts as moved: without a measurement there
    /// is no ground to claim the screen is untouched.
    fn can_reuse_prompt_at(&self, (column, row): (u16, u16)) -> bool {
        let inside_range = self.previous_prompt_rows_range.contains(&row);
        let cursor_unmoved = Some((column, row)) == self.cursor;
        inside_range && (!self.was_flush_at_bottom || cursor_unmoved)
    }
}

/// Screen bounds of the right prompt when it is visible.
#[derive(Debug, Clone, Copy)]
pub struct RightPromptBounds {
    pub row: u16,
    pub start_col: u16,
    pub end_col: u16,
}

#[derive(Debug, Clone)]
pub struct RenderSnapshot {
    pub screen_width: u16,
    pub screen_height: u16,
    pub prompt_start_row: u16,
    pub prompt_height: u16,
    pub large_buffer: bool,
    pub prompt_str_left: String,
    pub prompt_indicator: String,
    pub before_cursor: String,
    pub after_cursor: String,
    pub first_buffer_col: u16,
    pub menu_active: bool,
    pub menu_start_row: Option<u16>,
    pub status_start_row: Option<u16>,
    pub large_buffer_extra_rows_after_prompt: Option<usize>,
    pub large_buffer_offset: Option<usize>,
    pub right_prompt: Option<RightPromptBounds>,
}

#[derive(Debug, PartialEq, Eq)]
enum PromptRowSelector {
    UseExistingPrompt { start_row: u16 },
    MakeNewPrompt { new_row: u16 },
}

/// Query the cursor position unless the terminal explicitly declares itself dumb.
///
/// `TERM=dumb` does not provide cursor-position reporting, so avoid issuing a
/// query that cannot be answered. Otherwise delegate to the painter's writer,
/// which keeps terminal I/O testable.
fn cursor_position_for_term(stdout: &W, is_term_dumb: bool) -> Result<Option<(u16, u16)>> {
    if is_term_dumb {
        Ok(None)
    } else {
        stdout.cursor_position().map(Some)
    }
}

/// The lines an external message prints as. A newline ends a line rather
/// than starting an empty one, so `"a\n"` is one line and `""` still takes a
/// row. A `\r` before a newline goes with it.
fn message_lines(message: &str) -> impl Iterator<Item = &str> {
    message
        .strip_suffix('\n')
        .unwrap_or(message)
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
}

// Selects the row where the next prompt should start on, taking into account whether it should
// re-use a previous prompt.
fn select_prompt_row(
    suspended_state: Option<&PainterSuspendedState>,
    (column, row): (u16, u16), // NOTE: Positions are 0 based here
) -> PromptRowSelector {
    if let Some(painter_state) = suspended_state {
        if painter_state.can_reuse_prompt_at((column, row)) {
            let start_row = *painter_state.previous_prompt_rows_range.start();
            return PromptRowSelector::UseExistingPrompt { start_row };
        }
    }

    // Assumption: if the cursor is not on the zeroth column,
    //   there is content we want to leave intact, thus advance to the next row.
    let new_row = if column > 0 { row + 1 } else { row };
    PromptRowSelector::MakeNewPrompt { new_row }
}

/// Layout values computed once per paint cycle, shared between rendering and snapshot creation.
pub(crate) struct PromptLayout {
    /// Total rows scrolled off the top (before prompt adjustment).
    extra_rows: usize,
    /// Rows scrolled off after subtracting prompt lines.
    extra_rows_after_prompt: usize,
    /// Lines to skip from before_cursor for menu space (large buffer only).
    large_buffer_offset: Option<usize>,

    /// Right prompt bounds (`Some` when rendered).
    right_prompt: Option<RightPromptBounds>,

    /// Row where the menu starts.
    menu_start_row: Option<u16>,

    /// Row where the status line starts, `None` when none is drawn. Below
    /// the buffer and its hint, or, with a menu open, below the cursor's row
    /// and above the menu.
    status_start_row: Option<u16>,

    /// Buffer start column on first visible line.
    first_buffer_col: u16,
}

/// Cached row where the prompt starts on screen, together with its
/// freshness.
///
/// Call [`PromptStartRow::invalidate`] from any new code path that
/// yields the tty or writes content the painter doesn't track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PromptStartRow {
    /// No row known yet (pre-`initialize_prompt_position`).
    Unverified,
    /// Last-known row, but something may have moved the cursor or written
    /// content the painter doesn't model since. The next `repaint_buffer`
    /// must re-query before trusting it.
    Stale(u16),
    /// A resize happened and the terminal reported the *cursor* on
    /// `cursor_row`. Where the prompt starts relative to it depends on how many
    /// rows the prompt and the text before the cursor take at the new width,
    /// which only the next `repaint_buffer` knows; it resolves this with
    /// [`PromptStartRow::resolve_resize`].
    Resized { cursor_row: u16 },
    /// Row matches the terminal as of the last successful query or paint.
    Verified(u16),
}

impl PromptStartRow {
    /// Painter's best understanding of the prompt's screen row,
    /// regardless of freshness. Defaults to `0` when never initialized,
    /// which should only happen before `initialize_prompt_position` runs;
    /// we don't expect that to happen in normal flow.
    pub(crate) fn last_known_row(self) -> u16 {
        match self {
            PromptStartRow::Verified(r) | PromptStartRow::Stale(r) => r,
            // Between a resize and the next paint only the cursor row is known.
            // The prompt starts on it or above it.
            PromptStartRow::Resized { cursor_row } => cursor_row,
            PromptStartRow::Unverified => 0,
        }
    }

    /// Turn a [`PromptStartRow::Resized`] cursor row into the row the prompt
    /// starts on. `lines_before_cursor` is the number of rows from the prompt
    /// start through the cursor's row at the current width, so at least 1.
    /// Every other state is left alone.
    pub(crate) fn resolve_resize(&mut self, lines_before_cursor: u16) {
        if let PromptStartRow::Resized { cursor_row } = self {
            debug_assert!(
                lines_before_cursor >= 1,
                "lines_before_cursor counts the cursor's own row"
            );
            let r = cursor_row.saturating_sub(lines_before_cursor - 1);
            // The row is derived from the cursor position the terminal just
            // reported, and the drift check only repairs a row that sits
            // below the cursor, so there is nothing left to verify.
            *self = PromptStartRow::Verified(r);
        }
    }

    /// Record `row` as freshly verified.
    pub(crate) fn mark_verified(&mut self, row: u16) {
        *self = PromptStartRow::Verified(row);
    }

    /// Demote to stale, preserving the last-known row if any. Idempotent.
    pub(crate) fn invalidate(&mut self) {
        if let PromptStartRow::Verified(r) = *self {
            *self = PromptStartRow::Stale(r);
        }
    }
}

/// Implementation of the output to the terminal
pub struct Painter {
    // Stdout
    stdout: W,
    prompt_start_row: PromptStartRow,
    // The number of lines that the prompt takes up
    prompt_height: u16,
    terminal_size: (u16, u16),
    /// Cached terminal policy, refreshed on resize.
    term_is_dumb: bool,
    last_required_lines: u16,
    large_buffer: bool,
    after_cursor_lines: Option<String>,
    /// The right prompt as last rendered, with the bounds it was drawn at and
    /// its color already applied: the exit erase clears the cursor's row to the
    /// right, and `move_cursor_to_end` has no `prompt` to re-derive the color
    /// from, so both have to be captured during the paint.
    exit_right_prompt: Option<(String, RightPromptBounds)>,
    /// Optional semantic prompt markers for terminal integration (OSC 133/633)
    semantic_markers: Option<Box<dyn SemanticPromptMarkers>>,
    /// Layout computed during the last paint cycle.
    pub(crate) last_layout: Option<PromptLayout>,
    /// Set by `hide_cursor`, cleared by every Show. See [`Painter::show_cursor`].
    cursor_hidden: bool,
    /// The screen height whose last row an external-message commit measured
    /// as the real bottom. Until then a commit that scrolls past the last row
    /// asks where it ended, since a short reported height would put the real
    /// cursor below the counted one (#1206). Keyed on the height rather than
    /// reset: a resize, a new reported size or a measured growth changes the
    /// height, and with it the answer.
    bottom_confirmed_at: Option<u16>,
    /// Rows set aside for the status line of the paint being prepared, before
    /// the menus size themselves so they leave those rows free. That paint
    /// takes them, so none outlive it. See [`Painter::reserve_status_rows`].
    status_rows: u16,
    /// Rows a menu can take below the reserved status line while the whole
    /// block stays on the screen. Meaningless while `status_rows` is 0.
    menu_room: u16,
}

impl Painter {
    pub(crate) fn new(stdout: W) -> Self {
        let term = var_os("TERM");
        let term_is_dumb = term_is_dumb(term.as_deref());
        Painter {
            stdout,
            prompt_start_row: PromptStartRow::Unverified,
            prompt_height: 0,
            terminal_size: (0, 0),
            term_is_dumb,
            last_required_lines: 0,
            large_buffer: false,
            after_cursor_lines: None,
            exit_right_prompt: None,
            semantic_markers: None,
            last_layout: None,
            cursor_hidden: false,
            bottom_confirmed_at: None,
            status_rows: 0,
            menu_room: 0,
        }
    }

    /// Height of the current terminal window.
    ///
    /// A lower bound on the real height, not a reading: the reported size,
    /// raised to fit rows measured by `Painter::measure_cursor_position`.
    pub fn screen_height(&self) -> u16 {
        self.terminal_size.1
    }

    /// Width of the current terminal window
    pub fn screen_width(&self) -> u16 {
        self.terminal_size.0
    }

    /// Sets the semantic prompt markers for terminal integration (OSC 133/633)
    pub fn set_semantic_markers(&mut self, markers: Option<Box<dyn SemanticPromptMarkers>>) {
        self.semantic_markers = markers;
    }

    /// Returns a reference to the semantic prompt markers, if any
    pub fn semantic_markers(&self) -> Option<&dyn SemanticPromptMarkers> {
        self.semantic_markers.as_deref()
    }

    /// Asks the terminal where the cursor is, taking the answer as a floor on
    /// the screen height.
    ///
    /// A terminal only reports rows it has, so a measured row proves the screen
    /// holds at least `row + 1` — outranking a `terminal::size()` shorter than
    /// the window really attached (a serial console reporting `0x0`, a pty left
    /// at 24x80, a missed `SIGWINCH`). Otherwise an anchor at or below the
    /// believed last row saturates [`Painter::remaining_lines`] to `0` and every
    /// repaint walks the prompt up a row. See nushell/reedline#1205.
    ///
    /// No-op when the reported size is right. Only a freshly reported size (a
    /// new `read_line`, or a resize event) lowers the height again, so read it
    /// *after* calling this, not before.
    ///
    /// `Ok(None)` is [`cursor_position_for_term`]'s: no query was made, so
    /// there is no row to grow to.
    fn measure_cursor_position(&mut self) -> Result<Option<(u16, u16)>> {
        let position = cursor_position_for_term(&self.stdout, self.term_is_dumb)?;
        if let Some((_, row)) = position {
            self.grow_screen_height_to_fit_row(row);
        }
        Ok(position)
    }

    /// Reconcile a stale anchor: something yielded the tty since the last paint
    /// (a resize, an external completer, `$EDITOR`, an idle callback) and may
    /// have scrolled our content. Both paths that draw from the anchor run
    /// this first, so they agree on where the block starts. It queries the
    /// cursor, so it has to run before a synchronized frame opens.
    fn reconcile_stale_anchor(&mut self) {
        if let PromptStartRow::Stale(row) = self.prompt_start_row {
            // Cursor above the cached row => content scrolled up while the tty
            // was yielded. Re-anchor to the cursor (ground truth) rather than
            // homing to row 0, which would yank the prompt to the top. The `+1`
            // allows for output that left the cursor on the prompt row.
            // See nushell/reedline#1130.
            let anchor = match self.measure_cursor_position() {
                Ok(Some((_, cursor_row))) if cursor_row + 1 < row => cursor_row,
                _ => row,
            };
            // The measure above grew the height to fit the row it read, but the
            // anchor kept here can be further down than that. Grow to fit the
            // one actually committed to, or `remaining_lines` reads 0 and the
            // paint scrolls for no reason -- the bug this all exists to remove.
            self.grow_screen_height_to_fit_row(anchor);
            self.prompt_start_row.mark_verified(anchor);
        }
    }

    /// Raises the believed screen height so that `row` is on screen.
    fn grow_screen_height_to_fit_row(&mut self, row: u16) {
        self.terminal_size.1 = self.terminal_size.1.max(row.saturating_add(1));
    }

    /// Returns the empty lines from the prompt down, less the rows set aside
    /// for the status line, which draws between the buffer and a menu.
    pub fn remaining_lines_real(&self) -> u16 {
        self.remaining_lines().saturating_sub(self.prompt_height)
    }

    /// Rows a status line can take below a block of `block_rows` while the
    /// whole block stays on the screen. Past that the paint takes the
    /// large-buffer path, which draws no status line.
    pub(crate) fn status_room(&self, block_rows: u16) -> u16 {
        self.screen_height()
            .saturating_sub(block_rows)
            .saturating_sub(1)
    }

    /// Set aside the `rows` of the next paint's status line when they fit in
    /// `room`, from [`Painter::status_room`], so a menu sizing itself from
    /// [`Painter::remaining_lines`] leaves them free. A status line that does
    /// not fit gives way to the buffer and gets none.
    pub(crate) fn reserve_status_rows(&mut self, rows: u16, room: u16) {
        self.status_rows = if rows <= room { rows } else { 0 };
        self.menu_room = room.saturating_sub(self.status_rows);
    }

    /// Rows set aside for the status line of the paint being prepared.
    pub(crate) fn reserved_status_rows(&self) -> u16 {
        self.status_rows
    }

    /// Rows a menu can be cut to below the status line set aside for this
    /// paint, so the whole block stays on the screen, when that still shows
    /// `min_rows` of it.
    ///
    /// `None` without a status line, where a menu taller than the screen takes
    /// the large-buffer path, and when it would be cut shorter than
    /// `min_rows`: the menu keeps its height and takes the status rows back.
    pub(crate) fn menu_room_below_status(&self, min_rows: u16) -> Option<u16> {
        (self.status_rows > 0 && self.menu_room >= min_rows).then_some(self.menu_room)
    }

    /// Returns the empty lines below the prompt once it is scrolled to the top.
    ///
    /// A paint scrolls the prompt up to fit the menu below it, so this is the
    /// room a menu laid out now gets when drawn, even when
    /// [`Painter::remaining_lines_real`] still reads the space before the scroll.
    pub(crate) fn remaining_lines_after_scroll(&self) -> u16 {
        self.screen_height().saturating_sub(self.prompt_height)
    }

    /// Returns the number of lines that are available or can be made available by
    /// stripping the prompt, less the rows set aside for the status line.
    ///
    /// If you want the number of empty lines below the prompt,
    /// use [`Painter::remaining_lines_real`] instead.
    pub fn remaining_lines(&self) -> u16 {
        self.screen_height()
            .saturating_sub(self.prompt_start_row.last_known_row())
            .saturating_sub(self.status_rows)
    }

    /// Computes layout values shared between rendering and snapshot creation.
    ///
    /// `block_rows` is what [`PromptLines::required_lines`] measured for this
    /// paint, before the `status_rows` of its status line. It places the
    /// status line only without a menu, where it is the prompt, the buffer
    /// and the hint.
    fn compute_layout(
        &self,
        lines: &PromptLines,
        menu: Option<&ReedlineMenu>,
        block_rows: u16,
        status_rows: u16,
    ) -> PromptLayout {
        let screen_width = self.screen_width();
        let screen_height = self.screen_height();

        // Large buffer extra rows computation
        let (extra_rows, extra_rows_after_prompt) = if self.large_buffer {
            let prompt_rows_after_first = lines.prompt_height(screen_width) as usize - 1;
            let prompt_indicator_lines = lines.prompt_indicator.lines().count();
            let before_cursor_lines = lines.before_cursor.lines().count();
            let total_lines_before =
                (prompt_rows_after_first + prompt_indicator_lines + before_cursor_lines)
                    .saturating_sub(1);
            let extra = total_lines_before.saturating_sub(screen_height as usize);
            (extra, extra.saturating_sub(prompt_rows_after_first))
        } else {
            (0, 0)
        };

        // Large buffer offset for menu space
        let large_buffer_offset = if self.large_buffer {
            let cursor_distance = lines.distance_from_prompt(screen_width);
            menu.and_then(|menu| {
                if cursor_distance >= screen_height.saturating_sub(1) {
                    let rows = lines
                        .before_cursor
                        .lines()
                        .count()
                        .saturating_sub(extra_rows_after_prompt)
                        .saturating_sub(menu.min_rows() as usize);
                    Some(rows)
                } else {
                    None
                }
            })
        } else {
            None
        };

        // Right prompt layout
        let right_prompt = self.compute_right_prompt(lines, extra_rows);

        // Menu start row
        let menu_start_row = menu.map(|menu| {
            let cursor_distance = lines.distance_from_prompt(screen_width);
            if cursor_distance >= screen_height.saturating_sub(1) {
                screen_height.saturating_sub(menu.min_rows())
            } else {
                self.prompt_start_row.last_known_row() + cursor_distance + 1
            }
        });

        // The status line takes the rows the menu would have started on and
        // pushes the menu below it. Without a menu it follows the whole buffer
        // and the hint, as laid out end to end.
        let status_start_row = (status_rows > 0).then(|| {
            menu_start_row.unwrap_or_else(|| {
                self.prompt_start_row
                    .last_known_row()
                    .saturating_add(block_rows)
            })
        });
        let menu_start_row = menu_start_row.map(|row| row.saturating_add(status_rows));

        // First buffer column
        let first_buffer_col = if self.large_buffer && extra_rows_after_prompt > 0 {
            0
        } else {
            let prompt_line = format!("{}{}", lines.prompt_str_left, lines.prompt_indicator);
            let last_prompt_line = prompt_line.lines().last().unwrap_or_default();
            let width = line_width(last_prompt_line);
            if width > u16::MAX as usize {
                u16::MAX
            } else {
                width as u16
            }
        };

        PromptLayout {
            extra_rows,
            extra_rows_after_prompt,
            large_buffer_offset,
            right_prompt,
            menu_start_row,
            status_start_row,
            first_buffer_col,
        }
    }

    /// Whether the terminal declared itself `TERM=dumb`, as of construction
    /// or the last resize.
    pub(crate) fn term_is_dumb(&self) -> bool {
        self.term_is_dumb
    }

    /// Computes the right prompt position when the terminal can position it.
    fn compute_right_prompt(
        &self,
        lines: &PromptLines,
        extra_rows: usize,
    ) -> Option<RightPromptBounds> {
        if self.term_is_dumb
            || lines.prompt_str_right.is_empty()
            || self.large_buffer && extra_rows > 0
        {
            return None;
        }

        let screen_width = self.screen_width();
        let prompt_length_right = line_width(&lines.prompt_str_right);
        let start_position = screen_width.saturating_sub(prompt_length_right as u16);
        let input_width = lines.estimate_right_prompt_line_width(screen_width);

        if input_width > start_position {
            return None;
        }

        let mut row = self.prompt_start_row.last_known_row();
        if lines.right_prompt_on_last_line {
            row += lines.prompt_height(screen_width) - 1;
        }

        Some(RightPromptBounds {
            row,
            start_col: start_position,
            end_col: start_position.saturating_add(prompt_length_right as u16),
        })
    }

    /// Returns the state necessary before suspending the painter (to run a host command event).
    ///
    /// This state will be used to re-initialize the painter to re-use last prompt if possible.
    pub fn state_before_suspension(&self) -> PainterSuspendedState {
        let start_row = self.prompt_start_row.last_known_row();
        let final_row = start_row + self.last_required_lines;
        PainterSuspendedState {
            previous_prompt_rows_range: start_row..=final_row,
            // `final_row` can overshoot the last visible row for a prompt at the very
            // bottom, so this is `>=` rather than an equality.
            was_flush_at_bottom: final_row >= self.screen_height().saturating_sub(1),
            // One DSR round-trip per suspension is cheap next to whatever the host
            // is about to run, and it turns "might have scrolled" into a fact.
            cursor: cursor_position_for_term(&self.stdout, self.term_is_dumb)
                .ok()
                .flatten(),
        }
    }

    /// Sets the prompt origin position and screen size for a new line editor
    /// invocation
    ///
    /// Not to be used for resizes during a running line editor, use
    /// [`Painter::handle_resize()`] instead
    pub(crate) fn initialize_prompt_position(
        &mut self,
        suspended_state: Option<&PainterSuspendedState>,
    ) -> Result<()> {
        self.initialize_prompt_position_with_size(terminal::size()?, suspended_state)
    }

    /// [`Painter::initialize_prompt_position`] with the size passed in, since
    /// `terminal::size()` needs a tty and so cannot run under test.
    pub(crate) fn initialize_prompt_position_with_size(
        &mut self,
        reported_size: (u16, u16),
        suspended_state: Option<&PainterSuspendedState>,
    ) -> Result<()> {
        // Update the terminal size
        self.terminal_size = {
            // if reported size is 0, 0 -
            // use a default size to avoid divide by 0 panics
            if reported_size == (0, 0) {
                (80, 24)
            } else {
                reported_size
            }
        };
        self.anchor_prompt(suspended_state)
    }

    /// Establishes the prompt's start row for a new line editor invocation by
    /// asking the terminal where the cursor is.
    fn anchor_prompt(&mut self, suspended_state: Option<&PainterSuspendedState>) -> Result<()> {
        // Hide before asking, not only before painting. The exit path leaves
        // the cursor shown at column 0 of the row the next prompt lands on,
        // and the wait for the query's reply is the longest stretch
        // between that newline and the paint, so a fast terminal renders the
        // bare cursor for a frame (#1231). Flushed: the query goes out through
        // crossterm on stdout, unbuffered, and this hide sits in the stderr
        // buffer until then.
        self.hide_cursor()?;
        self.stdout.flush()?;

        // The terminal may not answer the cursor-position query in time
        // (crossterm gives it a fixed 2s): a terminal busy repainting, a
        // multiplexer briefly holding the reply, a slow remote link. That is
        // a transient condition, not a broken terminal, and this was the only
        // query site that still aborted `read_line` on it. Degrade instead:
        // print a newline so the prompt at least starts at column 0 on a row
        // of its own, keep the row as `Stale` so the next paint's drift check
        // asks again (and itself tolerates no answer), and carry on.
        //
        // The substitute row must be the bottom of the screen, never the
        // last-known row. The drift check only re-anchors when the cursor
        // turns out to be *above* the cached row, so a guess can only be
        // repaired if it errs high. The newline just printed has already
        // moved the cursor past the last-known row in the common REPL case,
        // and `clear_from_anchor` erases down from the anchor, so an
        // undershooting guess wipes the output above the prompt with no way
        // to recover. No row is greater than the bottom, so it is the only
        // guess that always lands on the repairable side.
        let position = match self.measure_cursor_position() {
            Ok(Some(position)) => position,
            Ok(None) => {
                // No query was made: do not add a blank line for every prompt.
                self.prompt_start_row =
                    PromptStartRow::Stale(self.screen_height().saturating_sub(1));
                return Ok(());
            }
            Err(_) => {
                // Preserve #1202's newline recovery for a real query failure.
                self.print_crlf()?;
                self.prompt_start_row =
                    PromptStartRow::Stale(self.screen_height().saturating_sub(1));
                return Ok(());
            }
        };
        let prompt_selector = select_prompt_row(suspended_state, position);
        let new_row = match prompt_selector {
            PromptRowSelector::UseExistingPrompt { start_row } => start_row,
            PromptRowSelector::MakeNewPrompt { new_row } => {
                // If we are on the last line and would move beyond the last line, we need to make
                // room for the prompt.
                // Otherwise printing the prompt would scroll off the stored prompt
                // origin, causing issues after repaints.
                //
                // Equality suffices: the height was grown to fit the measured
                // row, so `new_row` is at most `screen_height()`. The other arm
                // skips this guard but is bounded by `select_prompt_row` only
                // re-using a range that contains that row.
                if new_row == self.screen_height() {
                    self.print_crlf()?;
                    new_row.saturating_sub(1)
                } else {
                    new_row
                }
            }
        };
        // Reaching here means the cursor position was measured successfully;
        // the no-measurement path returned above with a stale bottom-row fallback.
        self.prompt_start_row.mark_verified(new_row);
        Ok(())
    }

    /// Mark `prompt_start_row` as possibly out of sync — the next
    /// `repaint_buffer` will re-query the terminal. Call from any path
    /// that lets something other than reedline's rendering move the
    /// cursor (e.g. `$EDITOR`).
    pub(crate) fn invalidate_prompt_start_row(&mut self) {
        self.prompt_start_row.invalidate();
    }

    /// Main painter for the prompt and buffer
    /// It queues all the actions required to print the prompt together with
    /// lines that make the buffer.
    /// Using the prompt lines object in this function it is estimated how the
    /// prompt should scroll up and how much space is required to print all the
    /// lines for the buffer
    ///
    /// Note. The `ScrollUp` operation in `crossterm` deletes lines from the top of
    /// the screen.
    ///
    /// The status line of `lines` draws on the rows
    /// [`Painter::reserve_status_rows`] set aside for this paint, none without.
    pub(crate) fn repaint_buffer(
        &mut self,
        prompt: &dyn Prompt,
        lines: &PromptLines,
        prompt_mode: PromptEditMode,
        menu: Option<&ReedlineMenu>,
        use_ansi_coloring: bool,
        cursor_config: &Option<CursorConfig>,
    ) -> Result<()> {
        // Taken first, so a paint that reserves none, as history search's
        // does, never inherits the rows of the one before.
        let reserved_status_rows = std::mem::take(&mut self.status_rows);

        // Reset any ANSI styling that may have been left by external commands
        // This ensures the prompt is not affected by previous output styling
        // Note: Attribute::Reset (SGR 0) resets all attributes including colors
        self.stdout.queue(SetAttribute(Attribute::Reset))?;

        self.hide_cursor()?;

        let screen_width = self.screen_width();

        self.prompt_height = lines.prompt_height(screen_width);
        let box_lines = box_wrap_prompt_lines(lines, usize::from(screen_width));
        let lines = &box_lines;
        let lines_before_cursor = lines.required_lines(screen_width, true, None);

        // Calibrate prompt start position for multi-line prompt/content before cursor. Check issue #841/#848/#930
        self.prompt_start_row.resolve_resize(lines_before_cursor);

        self.reconcile_stale_anchor();

        // Unreachable in normal flow (initialize_prompt_position runs first);
        // in release, home to row 0 rather than draw over the content there.
        let anchor_uninitialized = self.prompt_start_row == PromptStartRow::Unverified;
        debug_assert!(
            !anchor_uninitialized,
            "repaint_buffer reached before initialize_prompt_position"
        );

        // Distance parameters, computed after reconciling so they reflect the
        // re-anchored row and the height the reconcile may have grown. Reading
        // the height earlier splits the paint across two screens: `large_buffer`
        // would judge against the old, short one and reset the anchor to row 0.
        let screen_height = self.screen_height();
        // Every row from the anchor down: the status rows are among the
        // `required_lines` this has to hold.
        let remaining_lines = screen_height.saturating_sub(self.prompt_start_row.last_known_row());
        let block_rows = lines
            .required_lines(screen_width, false, menu)
            .saturating_add(if menu.is_some() { 0 } else { 1 });
        // The rows were set aside before the menu sized itself. One that could
        // not shrink enough to leave them free takes them back, which keeps
        // the status line out of the large-buffer path.
        let status_rows = if reserved_status_rows <= self.status_room(block_rows) {
            reserved_status_rows
        } else {
            0
        };
        let required_lines = block_rows + status_rows;

        // Cursor queries above must finish before the frame begins. End the frame even if
        // drawing fails, so a recoverable write error cannot leave updates suspended.
        self.begin_frame()?;
        let painted = (|| -> Result<()> {
            // Marking the painter state as larger buffer to avoid animations
            self.large_buffer = required_lines >= screen_height;

            // Moving the start position of the cursor based on the size of the required lines
            if self.large_buffer || anchor_uninitialized {
                for _ in 0..screen_height.saturating_sub(lines_before_cursor) {
                    self.stdout.queue(Print(&coerce_crlf("\n")))?;
                }
                // The reset puts the prompt at row 0; cache is back in sync.
                self.prompt_start_row.mark_verified(0);
            } else if required_lines >= remaining_lines {
                let extra = required_lines.saturating_sub(remaining_lines);
                self.queue_universal_scroll(extra)?;
                let scrolled_row = self.prompt_start_row.last_known_row().saturating_sub(extra);
                self.prompt_start_row.mark_verified(scrolled_row);
            }

            // Moving the cursor to the start of the prompt
            // from this position everything will be printed
            let anchor_row = self.prompt_start_row.last_known_row();
            self.clear_from_anchor(anchor_row)?;

            let layout = self.compute_layout(lines, menu, block_rows, status_rows);

            let margin_cursor_row = if self.large_buffer {
                self.print_large_buffer(prompt, lines, menu, use_ansi_coloring, &layout)?
            } else {
                self.print_small_buffer(prompt, lines, menu, use_ansi_coloring, &layout)?
            };

            self.exit_right_prompt = layout.right_prompt.map(|rp| {
                let text = coerce_crlf(&lines.prompt_str_right);
                let text = if use_ansi_coloring {
                    format!("{}{}", prompt.get_prompt_right_color().prefix(), text)
                } else {
                    text.into_owned()
                };
                (text, rp)
            });

            self.last_layout = Some(layout);

            // The last_required_lines is used to calculate safe range of the current prompt.
            self.last_required_lines = required_lines;

            let after_cursor = lines
                .exit_after_cursor
                .as_ref()
                .unwrap_or(&lines.after_cursor);
            self.after_cursor_lines = if !after_cursor.is_empty() {
                Some(after_cursor.to_string())
            } else {
                None
            };

            // It has to happen *here*, after every print: the position
            // `SavePosition` recorded is also the print head for the text after the
            // cursor, so disambiguating it earlier either writes a glyph onto an
            // unreserved row or overwrites the cell the next row's first grapheme
            // belongs in. Nothing is drawn at this point, so a bare move disturbs
            // neither.
            self.queue_cursor_placement(margin_cursor_row)?;

            let shape = cursor_config
                .as_ref()
                .and_then(|shapes| shapes.shape_for(&prompt_mode));
            if let Some(shape) = shape {
                self.stdout.queue(shape)?;
            }
            self.stdout.queue(cursor::Show)?;
            self.cursor_hidden = false;

            Ok(())
        })();
        let ended = self.end_frame().and_then(|()| self.stdout.flush());
        painted.and(ended)
    }

    /// Captures the current screen layout into a [`RenderSnapshot`] that records
    /// prompt geometry, buffer positions, right-prompt bounds, and menu state.
    /// This snapshot is later used by [`Self::screen_to_buffer_offset`] to map a
    /// terminal (column, row) click coordinate to a byte offset in the editing buffer.
    pub(crate) fn render_snapshot(
        &self,
        lines: &PromptLines,
        menu: Option<&ReedlineMenu>,
        raw_before: &str,
        raw_after: &str,
        layout: &PromptLayout,
    ) -> RenderSnapshot {
        let large_buffer_extra_rows_after_prompt = if self.large_buffer {
            Some(layout.extra_rows_after_prompt)
        } else {
            None
        };
        let large_buffer_offset = layout.large_buffer_offset;

        RenderSnapshot {
            screen_width: self.screen_width(),
            screen_height: self.screen_height(),
            prompt_start_row: self.prompt_start_row.last_known_row(),
            prompt_height: self.prompt_height,
            large_buffer: self.large_buffer,
            prompt_str_left: lines.prompt_str_left.to_string(),
            prompt_indicator: lines.prompt_indicator.to_string(),
            before_cursor: raw_before.to_string(),
            after_cursor: raw_after.to_string(),
            first_buffer_col: layout.first_buffer_col,
            menu_active: menu.is_some(),
            menu_start_row: layout.menu_start_row,
            status_start_row: layout.status_start_row,
            large_buffer_extra_rows_after_prompt,
            large_buffer_offset,
            right_prompt: layout.right_prompt,
        }
    }

    /// Maps a terminal screen coordinate (column, row) to a byte offset in the
    /// combined editing buffer (`before_cursor + after_cursor`).
    ///
    /// Returns `None` when the click lands outside the editable area: above the
    /// prompt, inside the right prompt, on the status line, inside the menu,
    /// or past the end of visible buffer content.
    ///
    /// The algorithm walks grapheme-by-grapheme through the visible portion of
    /// the buffer, tracking the current (row, col) on screen. Wide characters
    /// and line wrapping are accounted for. When the tracked position matches
    /// the target coordinate, the corresponding byte offset is returned.
    pub(crate) fn screen_to_buffer_offset(
        &self,
        snapshot: &RenderSnapshot,
        column: u16,
        row: u16,
    ) -> Option<usize> {
        // Clicks above the prompt are not in the buffer.
        if row < snapshot.prompt_start_row {
            return None;
        }

        // Clicks inside the menu area are not in the buffer.
        if snapshot.menu_active {
            if let Some(menu_start_row) = snapshot.menu_start_row {
                if row >= menu_start_row {
                    return None;
                }
            }
        }

        // Nor are clicks on the status line or below it. It starts past the
        // buffer or, with a menu open, on the rows that erased the text after
        // the cursor.
        if snapshot.status_start_row.is_some_and(|start| row >= start) {
            return None;
        }

        // Clicks inside the right prompt area are not in the buffer.
        if let Some(rp) = &snapshot.right_prompt {
            if row == rp.row && column >= rp.start_col && column < rp.end_col {
                return None;
            }
        }

        // Convert the absolute screen row to a row relative to the prompt start.
        let screen_width = snapshot.screen_width;
        let target_row = row.saturating_sub(snapshot.prompt_start_row);

        // Determine which relative row the buffer content begins on. When the
        // buffer hasn't scrolled, it starts on the last line of the prompt;
        // otherwise it starts at row 0 (the prompt itself has scrolled off).
        let buffer_start_row = if snapshot.large_buffer
            && snapshot.large_buffer_extra_rows_after_prompt.unwrap_or(0) > 0
        {
            0
        } else {
            snapshot.prompt_height.saturating_sub(1)
        };

        // Click landed in the prompt area before any buffer text.
        if target_row < buffer_start_row {
            return None;
        }

        // Compute the visible byte ranges of the before-cursor and after-cursor
        // buffer segments, accounting for lines scrolled off-screen in large
        // buffers and space reserved for menus.
        let (before_start, before_end) = if snapshot.large_buffer {
            skip_buffer_lines_range(
                &snapshot.before_cursor,
                snapshot.large_buffer_extra_rows_after_prompt.unwrap_or(0),
                snapshot.large_buffer_offset,
            )
        } else {
            (0, snapshot.before_cursor.len())
        };
        let before_visible = &snapshot.before_cursor[before_start..before_end];
        let full_before_visible = before_start == 0 && before_end == snapshot.before_cursor.len();

        let (after_start, after_end) = if snapshot.large_buffer {
            if snapshot.menu_active {
                let end = snapshot
                    .after_cursor
                    .find('\n')
                    .unwrap_or(snapshot.after_cursor.len());
                (0, end)
            } else {
                // The same walk `PromptLines::distance_from_prompt` uses, on
                // the snapshot's pieces: a division here would disagree with
                // the layout by a row on a trailing newline or an exactly
                // filled row, and clip `after_cursor` at the wrong place.
                let cursor_distance = wrap_position(
                    [
                        snapshot.prompt_str_left.as_str(),
                        snapshot.prompt_indicator.as_str(),
                        // As drawn, with control chars escaped.
                        &escape_as_drawn(&snapshot.before_cursor),
                    ],
                    screen_width,
                )
                .map_or(0, |end| resolve_wrap(end, screen_width).1);
                let remaining_lines = snapshot.screen_height.saturating_sub(cursor_distance);
                let offset = remaining_lines.saturating_sub(1) as usize;
                skip_buffer_lines_range(&snapshot.after_cursor, 0, Some(offset))
            }
        } else {
            (0, snapshot.after_cursor.len())
        };
        let after_visible = &snapshot.after_cursor[after_start..after_end];
        let full_after_visible = after_start == 0 && after_end == snapshot.after_cursor.len();
        let full_buffer_visible = full_before_visible && full_after_visible;

        // Walk through visible buffer content grapheme-by-grapheme, tracking
        // the screen position. When we hit the target (column, row) we return
        // the corresponding byte offset in the full buffer.
        let mut current_row = buffer_start_row;
        let mut current_col = if current_row == buffer_start_row {
            snapshot.first_buffer_col
        } else {
            0
        };

        let mut check_segment = |segment: &str, base_offset: usize| -> Option<usize> {
            for (index, grapheme) in segment.grapheme_indices(true) {
                // The buffer is drawn with its control chars escaped (`^[`,
                // `M-^[`), so walk what is shown, and a click on any column
                // of an escape maps back to the char it stands for.
                let shown = escape_as_drawn(grapheme);
                for shown in shown.graphemes(true) {
                    // Same rule as the layout walk, so the offset this maps
                    // back to is the grapheme the layout put under the click.
                    // A zero-width grapheme occupies no column and so is never
                    // the target; the base grapheme it combines with already
                    // matched.
                    let (at, width) =
                        advance_grapheme((current_row, current_col), shown, screen_width);
                    (current_row, current_col) = at;

                    if width > 0
                        && current_row == target_row
                        && column >= current_col
                        && column < current_col.saturating_add(width)
                    {
                        return Some(base_offset + index);
                    }

                    current_col = current_col.saturating_add(width);
                }
            }

            None
        };

        if let Some(offset) = check_segment(before_visible, before_start) {
            return Some(offset);
        }

        let after_base = snapshot.before_cursor.len().saturating_add(after_start);
        if let Some(offset) = check_segment(after_visible, after_base) {
            return Some(offset);
        }

        // Click is past all buffer content but still on the last buffer row;
        // treat it as a click at the very end of the buffer.
        if full_buffer_visible && target_row == current_row && column >= current_col {
            return Some(snapshot.before_cursor.len() + snapshot.after_cursor.len());
        }

        None
    }

    /// `printed_before` is what this paint put on screen ahead of the save
    /// below, which decides whether that save landed on the margin. It is only
    /// walked past the early return, so a paint with no right prompt pays
    /// nothing for it.
    fn print_right_prompt<'a>(
        &mut self,
        lines: &PromptLines,
        layout: &PromptLayout,
        printed_before: impl IntoIterator<Item = &'a str>,
    ) -> Result<()> {
        let Some(rp) = &layout.right_prompt else {
            return Ok(());
        };
        let (start_col, row) = (rp.start_col, rp.row);

        let margin_row = self.margin_cursor_row(printed_before);

        self.stdout
            .queue(SavePosition)?
            .queue(cursor::MoveTo(start_col, row))?;

        // Emit right prompt marker (OSC 133;P;k=r)
        if let Some(markers) = &self.semantic_markers {
            self.stdout
                .queue(Print(markers.prompt_start(PromptKind::Right)))?;
        }

        self.stdout
            .queue(Print(&coerce_crlf(&lines.prompt_str_right)))?;

        self.queue_cursor_placement(margin_row)
    }

    /// Put the cursor back where the paint left it: absolutely on the margin,
    /// where the save is ambiguous, and by restoring it everywhere else.
    fn queue_cursor_placement(&mut self, margin_row: Option<u16>) -> Result<()> {
        if self.term_is_dumb {
            // Leave the cursor where the last Print put it, including at a margin.
            return Ok(());
        }
        match margin_row {
            Some(row) => self.stdout.queue(MoveTo(0, row))?,
            None => self.stdout.queue(RestorePosition)?,
        };
        Ok(())
    }

    /// Move to `row` and erase everything below it.
    ///
    /// Clearing while the cursor is on the home cell (0,0) makes tmux's
    /// `scroll-on-clear` (default on) snapshot the whole screen into scrollback
    /// on every repaint, so the prompt/menu piles up in tmux history (#1062).
    /// At row 0 we therefore erase from column 1 to dodge tmux's `cx == 0`
    /// guard, then step back to column 0; the one skipped cell is overwritten by
    /// whatever is printed next. Every other row takes the plain path.
    fn clear_from_anchor(&mut self, row: u16) -> Result<()> {
        if row == 0 {
            self.stdout
                .queue(cursor::MoveTo(1, row))?
                .queue(Clear(ClearType::FromCursorDown))?
                .queue(cursor::MoveTo(0, row))?;
        } else {
            self.stdout
                .queue(cursor::MoveTo(0, row))?
                .queue(Clear(ClearType::FromCursorDown))?;
        }
        Ok(())
    }

    fn print_menu(
        &mut self,
        menu: &dyn Menu,
        use_ansi_coloring: bool,
        layout: &PromptLayout,
    ) -> Result<()> {
        let starting_row = layout.menu_start_row.unwrap_or(0);
        let remaining_lines = self.screen_height().saturating_sub(starting_row);
        let menu_string = menu.menu_string(remaining_lines, use_ansi_coloring);
        // Plain menus uppercase and truncate kept SGR as if it were text, so
        // whatever is left of it goes here, once for every menu.
        let menu_string = if use_ansi_coloring {
            menu_string
        } else {
            plain_menu_text(&menu_string).into_owned()
        };
        self.clear_from_anchor(starting_row)?;
        self.stdout
            .queue(Print(menu_string.trim_end_matches('\n')))?;

        Ok(())
    }

    /// The absolute row the cursor must be moved to, when `printed` (what this
    /// paint emitted before the cursor, in order) ends at the right margin.
    ///
    /// `None` off the margin, where `RestorePosition` is unambiguous. Each print
    /// path answers for its own output rather than the caller reconstructing it:
    /// a large buffer prints line-skipped text, so the rows it occupies are not
    /// the rows the untrimmed buffer would.
    ///
    /// Also `None` past the bottom of the screen: text filling it exactly points
    /// the deferred wrap at a row the terminal has not scrolled into existence,
    /// and a move there gets clamped to the bottom row's first column, a whole
    /// row from the text. Restoring at least lands next to it.
    fn margin_cursor_row<'a>(&self, printed: impl IntoIterator<Item = &'a str>) -> Option<u16> {
        let rows = deferred_wrap_row(printed, self.screen_width())?;
        let row = self.prompt_start_row.last_known_row().saturating_add(rows);
        (row < self.screen_height()).then_some(row)
    }

    fn print_small_buffer(
        &mut self,
        prompt: &dyn Prompt,
        lines: &PromptLines,
        menu: Option<&ReedlineMenu>,
        use_ansi_coloring: bool,
        layout: &PromptLayout,
    ) -> Result<Option<u16>> {
        // Emit prompt start marker (OSC 133;A;k=i for primary prompt)
        if let Some(markers) = &self.semantic_markers {
            self.stdout
                .queue(Print(markers.prompt_start(PromptKind::Primary)))?;
        }

        // print our prompt with color
        if use_ansi_coloring {
            self.stdout
                .queue(Print(prompt.get_prompt_color().prefix()))?;
        }

        self.stdout
            .queue(Print(&coerce_crlf(&lines.prompt_str_left)))?;

        if use_ansi_coloring {
            self.stdout
                .queue(Print(prompt.get_indicator_color().prefix()))?;
        }

        self.stdout
            .queue(Print(&coerce_crlf(&lines.prompt_indicator)))?;

        if use_ansi_coloring {
            self.stdout
                .queue(Print(prompt.get_prompt_right_color().prefix()))?;
        }

        self.print_right_prompt(
            lines,
            layout,
            [&*lines.prompt_str_left, &lines.prompt_indicator],
        )?;

        // Emit command input start marker (OSC 133;B) after prompt (including right prompt)
        if let Some(markers) = &self.semantic_markers {
            self.stdout.queue(Print(markers.command_input_start()))?;
        }

        if use_ansi_coloring {
            self.stdout
                .queue(SetAttribute(Attribute::Reset))?
                .queue(ResetColor)?;
        }

        self.stdout.queue(Print(&lines.before_cursor))?;
        if !self.term_is_dumb {
            self.stdout.queue(SavePosition)?;
        }
        self.stdout.queue(Print(&lines.after_cursor))?;

        let cursor_row = self.margin_cursor_row([
            &*lines.prompt_str_left,
            &lines.prompt_indicator,
            &lines.before_cursor,
        ]);

        if let Some(menu) = menu {
            self.print_status(lines, use_ansi_coloring, layout)?;
            self.print_menu(menu, use_ansi_coloring, layout)?;
        } else {
            self.stdout.queue(Print(&lines.hint))?;
            self.print_box_bottom_edge()?;
            self.print_status(lines, use_ansi_coloring, layout)?;
        }

        Ok(cursor_row)
    }

    /// Draw the status line on the rows [`Painter::compute_layout`] gave it.
    ///
    /// The rows are erased first: with a menu open, text after the cursor
    /// can reach them, the same rows the menu would otherwise draw over.
    fn print_status(
        &mut self,
        lines: &PromptLines,
        use_ansi_coloring: bool,
        layout: &PromptLayout,
    ) -> Result<()> {
        let Some(row) = layout.status_start_row else {
            return Ok(());
        };
        self.clear_from_anchor(row)?;
        self.stdout.queue(Print(&lines.status))?;
        if use_ansi_coloring {
            self.stdout
                .queue(SetAttribute(Attribute::Reset))?
                .queue(ResetColor)?;
        }
        Ok(())
    }

    fn print_large_buffer(
        &mut self,
        prompt: &dyn Prompt,
        lines: &PromptLines,
        menu: Option<&ReedlineMenu>,
        use_ansi_coloring: bool,
        layout: &PromptLayout,
    ) -> Result<Option<u16>> {
        let screen_width = self.screen_width();
        let screen_height = self.screen_height();
        let cursor_distance = lines.distance_from_prompt(screen_width);
        let remaining_lines = screen_height.saturating_sub(cursor_distance);

        let extra_rows = layout.extra_rows;
        let extra_rows_after_prompt = layout.extra_rows_after_prompt;

        // Emit prompt start marker (OSC 133;A;k=i for primary prompt) only if prompt is visible
        if extra_rows == 0 {
            if let Some(markers) = &self.semantic_markers {
                self.stdout
                    .queue(Print(markers.prompt_start(PromptKind::Primary)))?;
            }
        }

        // print our prompt with color
        if use_ansi_coloring {
            self.stdout
                .queue(Print(prompt.get_prompt_color().prefix()))?;
        }

        // In case the prompt is made out of multiple lines, the prompt is split by
        // lines and only the required ones are printed.
        //
        // Sliced rather than run through `skip_buffer_lines`, whose trailing-newline
        // trim would drop the newline that puts the indicator on the row below.
        let (prompt_start, prompt_end) =
            skip_buffer_lines_range(&lines.prompt_str_left, extra_rows, None);
        let prompt_skipped = &lines.prompt_str_left[prompt_start..prompt_end];
        self.stdout.queue(Print(&coerce_crlf(prompt_skipped)))?;

        if extra_rows == 0 {
            if use_ansi_coloring {
                self.stdout
                    .queue(Print(prompt.get_prompt_right_color().prefix()))?;
            }

            // Judged from the *skipped* prompt, which is what reached the screen.
            self.print_right_prompt(lines, layout, [prompt_skipped])?;
        }

        if use_ansi_coloring {
            self.stdout
                .queue(Print(prompt.get_indicator_color().prefix()))?;
        }
        let indicator_skipped =
            skip_buffer_lines(&lines.prompt_indicator, extra_rows_after_prompt, None);
        self.stdout.queue(Print(&coerce_crlf(indicator_skipped)))?;

        // Emit command input start marker (OSC 133;B) after prompt indicator
        if let Some(markers) = &self.semantic_markers {
            self.stdout.queue(Print(markers.command_input_start()))?;
        }

        if use_ansi_coloring {
            self.stdout.queue(ResetColor)?;
        }

        // Selecting the lines before the cursor that will be printed
        let before_cursor_skipped = skip_buffer_lines(
            &lines.before_cursor,
            extra_rows_after_prompt,
            layout.large_buffer_offset,
        );
        self.stdout.queue(Print(before_cursor_skipped))?;
        if !self.term_is_dumb {
            self.stdout.queue(SavePosition)?;
        }

        // Computed from the *skipped* text, which is what reached the screen.
        let cursor_row =
            self.margin_cursor_row([prompt_skipped, indicator_skipped, before_cursor_skipped]);

        if let Some(menu) = menu {
            // TODO: Also solve the difficult problem of displaying (parts of)
            // the content after the cursor with the completion menu
            // This only shows the rest of the line the cursor is on
            if let Some(newline) = lines.after_cursor.find('\n') {
                self.stdout.queue(Print(&lines.after_cursor[0..newline]))?;
            } else {
                self.stdout.queue(Print(&lines.after_cursor))?;
            }
            self.print_menu(menu, use_ansi_coloring, layout)?;
        } else {
            // Selecting lines for the hint
            // The -1 subtraction is done because the remaining lines consider the line where the
            // cursor is located as a remaining line. That has to be removed to get the correct offset
            // for the after-cursor and hint lines
            let offset = remaining_lines.saturating_sub(1) as usize;
            // Selecting lines after the cursor
            let after_cursor_skipped = skip_buffer_lines(&lines.after_cursor, 0, Some(offset));
            self.stdout.queue(Print(after_cursor_skipped))?;
            // Hint lines
            let hint_skipped = skip_buffer_lines(&lines.hint, 0, Some(offset));
            self.stdout.queue(Print(hint_skipped))?;
        }

        Ok(cursor_row)
    }

    /// Updates prompt origin and offset to handle a screen resize event
    pub(crate) fn handle_resize(&mut self, width: u16, height: u16) {
        self.terminal_size = (width, height);
        let term = var_os("TERM");
        self.term_is_dumb = term_is_dumb(term.as_deref());

        self.invalidate_prompt_start_row();

        // `cursor::position()` is blocking and can time out, but a
        // resize happens infrequently enough that we accept the cost.
        // The terminal only reports the cursor, so that is what gets
        // recorded; the next `repaint_buffer` works out the prompt row.
        //
        // Known bug: on iterm2 and kitty, clearing the screen via CMD-K
        // doesn't reset the cursor position — possibly a `position()`
        // bug.
        //
        // Measured, so the row raises a `height` that under-reports: the next
        // paint takes this row as verified and never asks again, which leaves
        // this read as the only proof of the height until the next reported
        // size.
        if let Ok(Some((_, cursor_row))) = self.measure_cursor_position() {
            self.prompt_start_row = PromptStartRow::Resized { cursor_row };
        }
    }

    /// Writes `line` to the terminal followed by `\r\n` and
    /// invalidates the cached prompt anchor since the line scrolls the
    /// terminal independently of the painter.
    pub(crate) fn paint_line(&mut self, line: &str) -> Result<()> {
        // Invalidate up front: a partial write below can still leave
        // bytes in the kernel/tty buffer and displace the cursor.
        self.invalidate_prompt_start_row();
        self.stdout.queue(Print(line))?.queue(Print("\r\n"))?;
        self.stdout.flush()
    }

    /// Queue a hide and remember it, so [`Painter::show_cursor`] can undo it.
    fn hide_cursor(&mut self) -> Result<()> {
        self.stdout.queue(cursor::Hide)?;
        self.cursor_hidden = true;
        Ok(())
    }

    /// Queue the start of a synchronized update: the terminal keeps applying
    /// what follows but draws none of it until [`Painter::end_frame`], so an
    /// erase and the redraw after it show as one frame. A terminal that does
    /// not know the mode ignores it; `TERM=dumb` would print it, so skip it.
    fn begin_frame(&mut self) -> Result<()> {
        if !self.term_is_dumb {
            self.stdout.queue(BeginSynchronizedUpdate)?;
        }
        Ok(())
    }

    /// Queue the end of the frame [`Painter::begin_frame`] opened. Queue it
    /// before the flush that sends the frame and before any cursor query: an
    /// end left in the buffer holds the screen until the terminal times out.
    fn end_frame(&mut self) -> Result<()> {
        if !self.term_is_dumb {
            self.stdout.queue(EndSynchronizedUpdate)?;
        }
        Ok(())
    }

    /// End a frame and send it, for a caller that has nothing to paint after
    /// [`Painter::print_external_message`] left one open. With no frame open
    /// the end resets a mode that is already off, which no terminal shows.
    pub(crate) fn finish_frame(&mut self) -> Result<()> {
        self.end_frame()?;
        self.stdout.flush()
    }

    /// Show the cursor if the painter hid it, for the paths that end without
    /// a paint.
    ///
    /// [`Painter::anchor_prompt`] hides it and the next `repaint_buffer` shows
    /// it again. Anything that hands the terminal back in between (an exit
    /// right after a clear, a failed `read_line`, a drop) goes through here.
    /// Writes nothing when the cursor is already shown, so a host that hid it
    /// on purpose keeps it hidden.
    pub(crate) fn show_cursor(&mut self) -> Result<()> {
        if !self.cursor_hidden {
            return Ok(());
        }
        self.stdout.queue(cursor::Show)?;
        self.cursor_hidden = false;
        self.stdout.flush()
    }

    /// Goes to the beginning of the next line
    ///
    /// Also works in raw mode
    pub(crate) fn print_crlf(&mut self) -> Result<()> {
        self.stdout.queue(Print("\r\n"))?;

        self.stdout.flush()
    }

    /// Clear the screen by printing enough whitespace to start the prompt or
    /// other output back at the first line of the terminal.
    pub(crate) fn clear_screen(&mut self) -> Result<()> {
        self.stdout
            .queue(Clear(ClearType::All))?
            .queue(MoveTo(0, 0))?
            .flush()?;
        self.initialize_prompt_position(None)
    }

    pub(crate) fn clear_scrollback(&mut self) -> Result<()> {
        self.stdout
            .queue(Clear(ClearType::All))?
            .queue(Clear(ClearType::Purge))?
            .queue(MoveTo(0, 0))?
            .flush()?;
        self.initialize_prompt_position(None)
    }

    /// Park the cursor below the entry on the way out of `read_line`.
    ///
    /// Nothing repaints after this, so whatever sits below the cursor is what
    /// the host prints over. A menu or a hint is not user input, so the erase is
    /// unconditional and only the after-cursor text goes back. Text before the
    /// cursor survives, leaving a rejected command on screen as the record of
    /// what was aborted (#1143).
    fn print_box_bottom_edge(&mut self) -> Result<()> {
        let width = self.screen_width();
        if width < 4 {
            return Ok(());
        }
        let edge = "─".repeat(usize::from(width) - 2);
        self.stdout
            .queue(cursor::MoveToColumn(width - 1))?
            .queue(Print("│"))?
            .queue(Print("\r\n"))?
            .queue(Print(format!("╰{edge}╯")))?;
        Ok(())
    }

    pub(crate) fn move_cursor_to_end(&mut self) -> Result<()> {
        self.stdout.queue(Clear(ClearType::FromCursorDown))?;
        if let Some(after_cursor) = &self.after_cursor_lines {
            self.stdout.queue(Print(after_cursor))?;
        }
        if let Some((text, rp)) = &self.exit_right_prompt {
            self.stdout
                .queue(SavePosition)?
                .queue(cursor::MoveTo(rp.start_col, rp.row))?
                .queue(Print(text))?
                .queue(SetAttribute(Attribute::Reset))?
                .queue(ResetColor)?
                .queue(RestorePosition)?;
        }
        self.print_crlf()
    }

    /// Commits `messages` to the screen above the prompt block, one row per
    /// line, so they scroll into history like output printed at a quiet
    /// prompt.
    ///
    /// The block itself, prompt, buffer and anything under
    /// them, is erased from the anchor down and the messages take its place.
    /// `prompt_start_row` names the first free row below them, counted rather
    /// than asked for, and the caller repaints the block there.
    ///
    /// Nothing is flushed, and the synchronized frame opened here is left
    /// open: the caller's repaint closes it and sends the commit and the
    /// redrawn block as one frame. Ending it here, or asking the terminal for
    /// the cursor in between, splits them into two, and the screen shows the
    /// messages without a prompt under them for a frame. A cursor query
    /// cannot go inside the frame either: Alacritty and Rio hold back all
    /// input during one, so the answer only comes at their 150 ms timeout.
    /// Should the repaint never come, that same timeout ends the frame.
    ///
    /// That holds only from a verified anchor: a count is only as good as
    /// the row it starts from. A `Stale` one, as an idle callback leaves it,
    /// is reconciled first, the same way the repaint would. From any other
    /// (a resize is unresolved, the first query went unanswered) the commit
    /// ends its frame, flushes and asks instead, and pays the second frame.
    /// So does a commit that scrolls past the last row of a height not yet
    /// confirmed as the real one: past the reported bottom the count only
    /// holds if the report is right (#1206).
    ///
    /// Starting at the anchor rather than at the cursor is the point. The
    /// anchor is the one row the painter trusts, and every part of the block
    /// sits below it, so there is no height to guess and nothing to leave
    /// behind. The old path counted the block's rows from the buffer's bytes,
    /// disagreed with the painter whenever the left prompt wrapped a line,
    /// and left a copy of the prompt row on screen per batch.
    ///
    /// Each message is split at its newlines here, since every line needs
    /// its own `\r\n` and row count, whatever the source handed over.
    pub(crate) fn print_external_message(&mut self, messages: Vec<String>) -> Result<()> {
        // An idle callback leaves the anchor `Stale` on every poll. Check it
        // the way the repaint after this will, so the messages start where
        // that repaint draws the block, and before the frame opens.
        self.reconcile_stale_anchor();
        let start = self.prompt_start_row.last_known_row();
        let start_verified = matches!(self.prompt_start_row, PromptStartRow::Verified(_));
        // Invalidate before the first queued byte: a `?` early-return below
        // must not leave bytes in the buffer with the cache still claiming
        // `Verified`.
        self.invalidate_prompt_start_row();

        // Hide first, so the cursor is not seen jumping to the anchor. The
        // repaint that follows shows it again; `hide_cursor` records the hide
        // so a path that ends without that repaint can still undo it.
        self.hide_cursor()?;

        self.begin_frame()?;
        let committed = (|| -> Result<()> {
            self.clear_from_anchor(start)?;

            // Count where the cursor lands. A line wider than the screen takes
            // more than one row; an empty one still takes its `\r\n`. Once the
            // cursor is on the last row a newline scrolls instead of moving,
            // hence the cap.
            let height = self.screen_height();
            let max_row = height.saturating_sub(1);
            let mut row = start;
            // Whether the count ran past the last row, so the terminal scrolled.
            // Landing exactly on it takes no scroll and holds at any real height.
            let mut scrolled = false;

            for line in messages.iter().flat_map(|message| message_lines(message)) {
                // max(1) on the width: a mid-resize terminal can report 0 (#842).
                let rows = estimate_required_lines(line, self.screen_width().max(1)).max(1);
                let rows = u16::try_from(rows).unwrap_or(u16::MAX);
                self.stdout.queue(Print(line))?.queue(Print("\r\n"))?;
                // Saturating: a height grown to fit a measured row can sit near
                // `u16::MAX` (#1236).
                let next = row.saturating_add(rows);
                scrolled |= next > max_row;
                row = next.min(max_row);
            }

            let bottom_trusted = !scrolled || self.bottom_confirmed_at == Some(height);
            if !start_verified || !bottom_trusted {
                // Counted from a row nobody checked, or capped at a bottom
                // nobody checked, so ask. Flushed first, so the answer is the
                // cursor after the messages, not before them.
                self.end_frame()?;
                self.stdout.flush()?;
                self.prompt_start_row = match self.measure_cursor_position() {
                    Ok(Some((_, actual))) => {
                        // A scroll that ends on the counted last row means it is
                        // the real one; later commits can trust the cap. Lower
                        // down, the measure has already grown the height.
                        if scrolled && actual == max_row {
                            self.bottom_confirmed_at = Some(height);
                        }
                        PromptStartRow::Verified(actual)
                    }
                    // No answer: the count is all there is. `Stale` keeps the
                    // next paint checking it, which is safe now that the bytes
                    // have gone out.
                    Ok(None) | Err(_) => PromptStartRow::Stale(row),
                };
                return Ok(());
            }

            // The count is the anchor, with no query to check it (see above for
            // why there can be none). It measures the text with the same widths
            // the painter uses for the prompt and buffer, escape sequences
            // stripped, so it is as good as every other row the painter draws.
            // A tab or a message's own cursor movement still fools it: the
            // terminal moves where the count sees zero width.
            //
            // `Verified`, not `Stale`: the lines above are only queued, so the
            // terminal's cursor has not moved yet. A `Stale` row would make the
            // next paint check it against that still-earlier cursor, read it as
            // the prompt having scrolled off the top, and re-anchor by printing a
            // whole screen of newlines, wiping the display (#1005).
            self.prompt_start_row = PromptStartRow::Verified(row);
            Ok(())
        })();
        if committed.is_err() {
            // A write failed with the frame open, and no repaint follows to
            // close it: end it here, or the screen stays held until the
            // terminal's timeout. The first error is the one to report.
            let _ = self.end_frame().and_then(|()| self.stdout.flush());
        }
        committed
    }

    /// Queue scroll of `num` lines to `self.stdout`.
    ///
    /// On some platforms and terminals (e.g. windows terminal, alacritty on windows and linux)
    /// using special escape sequence '\[e<num>S' (provided by [`ScrollUp`]) does not put lines
    /// that go offscreen in scrollback history. This method prints newlines near the edge of screen
    /// (which always works) instead. See [here](https://github.com/nushell/nushell/issues/9166)
    /// for more info on subject.
    ///
    /// ## Note
    /// This method does not return cursor to the original position and leaves it at the first
    /// column of last line. **Be sure to use [`MoveTo`] afterwards if this is not the desired
    /// location**
    fn queue_universal_scroll(&mut self, num: u16) -> Result<()> {
        // If cursor is not near end of screen printing new will not scroll terminal.
        // Move it to the last line to ensure that every newline results in scroll
        self.stdout.queue(MoveTo(0, self.screen_height() - 1))?;
        for _ in 0..num {
            self.stdout.queue(Print(&coerce_crlf("\n")))?;
        }
        Ok(())
    }


    /// Whether the cached anchor is still trusted, so a test can pin which events cost
    /// a re-verify and which keep #1090's query-free path.


    /// What the exit path will reprint after the cursor.

    /// Bytes written so far. Panics unless the painter was built on a
    /// [`W::capture`] writer.
}



fn box_wrap_text(text: &str, start_col: usize, width: usize) -> (String, usize) {
    let limit = width.saturating_sub(2);
    let bar = format!("\x1b[{}G│", width);
    let mut out = String::with_capacity(text.len() + 16);
    let mut col = start_col;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let code = c as u32;
        if code == 27 {
            out.push(c);
            match chars.peek().map(|n| *n as u32) {
                Some(91) => {
                    if let Some(n) = chars.next() {
                        out.push(n);
                    }
                    while let Some(n) = chars.next() {
                        out.push(n);
                        if (0x40..=0x7e).contains(&(n as u32)) {
                            break;
                        }
                    }
                }
                Some(93) => {
                    if let Some(n) = chars.next() {
                        out.push(n);
                    }
                    while let Some(n) = chars.next() {
                        out.push(n);
                        if n as u32 == 7 {
                            break;
                        }
                        if n as u32 == 27 {
                            if let Some(m) = chars.next() {
                                out.push(m);
                            }
                            break;
                        }
                    }
                }
                Some(_) => {
                    if let Some(n) = chars.next() {
                        out.push(n);
                    }
                }
                None => {}
            }
            continue;
        }
        if code == 13 {
            if chars.peek().map(|n| *n as u32) == Some(10) {
                out.push_str(&bar);
            }
            out.push(c);
            col = 0;
            continue;
        }
        if code == 10 {
            out.push(c);
            out.push_str("│ ");
            col = 2;
            continue;
        }
        let mut buffer = [0u8; 4];
        let size = super::utils::line_width(c.encode_utf8(&mut buffer));
        if size > 0 && col + size > limit && col > 2 {
            out.push_str(&bar);
            out.push_str("\r\n│ ");
            col = 2;
        }
        out.push(c);
        col += size;
    }
    (out, col)
}

fn box_wrap_prompt_lines(lines: &PromptLines, width: usize) -> PromptLines<'static> {
    let first = format!("{}{}", lines.prompt_str_left, lines.prompt_indicator);
    let last = first.lines().last().unwrap_or_default();
    let start = super::utils::line_width(last);
    let (before, column) = box_wrap_text(&lines.before_cursor, start, width);
    let (after, column_after) = box_wrap_text(&lines.after_cursor, column, width);
    let (hint, _) = box_wrap_text(&lines.hint, column_after, width);
    PromptLines {
        prompt_str_left: std::borrow::Cow::Owned(lines.prompt_str_left.to_string()),
        prompt_str_right: std::borrow::Cow::Owned(lines.prompt_str_right.to_string()),
        prompt_indicator: std::borrow::Cow::Owned(lines.prompt_indicator.to_string()),
        before_cursor: std::borrow::Cow::Owned(before),
        after_cursor: std::borrow::Cow::Owned(after),
        hint: std::borrow::Cow::Owned(hint),
        right_prompt_on_last_line: lines.right_prompt_on_last_line,
        exit_after_cursor: lines
            .exit_after_cursor
            .as_ref()
            .map(|text| std::borrow::Cow::Owned(text.to_string())),
        status: std::borrow::Cow::Owned(lines.status.to_string()),
    }
}
