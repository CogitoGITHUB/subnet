use super::utils::{
    coerce_crlf, escape_control, escape_control_keep_sgr, estimate_required_lines, expand_tabs,
    line_width, resolve_wrap, wrap_position,
};
use crate::{
    menu::{Menu, ReedlineMenu},
    prompt::PromptEditMode,
    Prompt, PromptHistorySearch,
};
use std::borrow::Cow;

/// Aggregate of prompt and input string used by `Painter`
#[derive(Debug)]
pub(crate) struct PromptLines<'prompt> {
    pub(crate) prompt_str_left: Cow<'prompt, str>,
    pub(crate) prompt_str_right: Cow<'prompt, str>,
    pub(crate) prompt_indicator: Cow<'prompt, str>,
    pub(crate) before_cursor: Cow<'prompt, str>,
    pub(crate) after_cursor: Cow<'prompt, str>,
    pub(crate) hint: Cow<'prompt, str>,
    pub(crate) right_prompt_on_last_line: bool,
    /// What the exit path reprints after the cursor, when that is not
    /// `after_cursor`: a history hint drawn in place of trailing auto-pair
    /// closers leaves `after_cursor` empty, but the closers are still in the
    /// buffer.
    pub(crate) exit_after_cursor: Option<Cow<'prompt, str>>,
    /// The [`StatusLine`](crate::StatusLine) text, drawn on its own rows below
    /// the buffer and above any menu. Empty draws none.
    pub(crate) status: Cow<'prompt, str>,
}

impl<'prompt> PromptLines<'prompt> {
    /// Splits the strings before and after the cursor as well as the hint
    /// This vector with the str are used to calculate how many lines are
    /// required to print after the prompt
    pub fn new(
        prompt: &'prompt dyn Prompt,
        prompt_mode: PromptEditMode,
        history_indicator: Option<PromptHistorySearch>,
        before_cursor: &'prompt str,
        after_cursor: &'prompt str,
        hint: &'prompt str,
    ) -> Self {
        let prompt_str_left = prompt.render_prompt_left();
        let prompt_str_right = prompt.render_prompt_right();

        let prompt_indicator = match history_indicator {
            Some(prompt_search) => prompt.render_prompt_history_search_indicator(prompt_search),
            None => prompt.render_prompt_indicator(prompt_mode),
        };

        let before_cursor = coerce_crlf(before_cursor);
        let after_cursor = coerce_crlf(after_cursor);
        let hint = coerce_crlf(hint);
        let right_prompt_on_last_line = prompt.right_prompt_on_last_line();

        Self {
            prompt_str_left,
            prompt_str_right,
            prompt_indicator,
            before_cursor,
            after_cursor,
            hint,
            right_prompt_on_last_line,
            exit_after_cursor: None,
            status: Cow::Borrowed(""),
        }
    }

    /// Set the status line drawn below the buffer. See [`Self::status`].
    ///
    /// Trailing line breaks are dropped: the rows count stops at the last
    /// line, so printing one would move the terminal a row past them, and
    /// scroll it when the status line ends on the bottom row.
    ///
    /// Each line is drawn as it is counted. Control characters show escaped,
    /// as in the buffer, so a `\r` inside a line cannot return to its start
    /// and draw over it. SGR is kept while `use_ansi_coloring` is on, as in a
    /// hint. Tabs become the spaces to the next stop (see [`expand_tabs`]).
    pub(crate) fn with_status(mut self, status: &str, use_ansi_coloring: bool) -> Self {
        let mut drawn = String::with_capacity(status.len());
        // `lines` splits at `\r\n` as well as `\n`, so the `\r` of a line
        // break never reaches the escape.
        for (index, line) in status.trim_end_matches(['\r', '\n']).lines().enumerate() {
            if index > 0 {
                drawn.push_str("\r\n");
            }
            let line = if use_ansi_coloring {
                escape_control_keep_sgr(line)
            } else {
                escape_control(line)
            };
            drawn.push_str(&expand_tabs(&line));
        }
        self.status = Cow::Owned(drawn);
        self
    }

    /// Rows the status line takes, wrapping included. 0 without one.
    pub(crate) fn status_rows(&self, terminal_columns: u16) -> u16 {
        if self.status.is_empty() {
            return 0;
        }
        estimate_required_lines(&self.status, terminal_columns)
            .try_into()
            .unwrap_or(u16::MAX)
    }

    /// Set what the exit path reprints after the cursor in place of
    /// `after_cursor`. See [`Self::exit_after_cursor`].
    pub(crate) fn with_exit_after_cursor(mut self, after_cursor: &'prompt str) -> Self {
        self.exit_after_cursor = Some(coerce_crlf(after_cursor));
        self
    }

    /// Rows to reserve for the prompt and buffer, laid out end to end.
    ///
    /// Floored at [`Self::distance_from_prompt`] plus one: the cursor can rest
    /// on a row no glyph reaches, since a filled row or a trailing newline
    /// leaves it one past the last drawn one, and `menu_start_row` opens the
    /// menu below *that*. Reserving only the rows text occupies runs the menu
    /// past the reservation.
    pub(crate) fn required_lines(
        &self,
        terminal_columns: u16,
        before_cursor: bool,
        menu: Option<&ReedlineMenu>,
    ) -> u16 {
        let mut input =
            self.prompt_str_left.to_string() + &self.prompt_indicator + &self.before_cursor;

        if !before_cursor {
            input += &self.after_cursor;
            if menu.is_none() {
                input += &self.hint;
            }
        }

        let lines = estimate_required_lines(&input, terminal_columns)
            .max(self.distance_from_prompt(terminal_columns) as usize + 1);

        if let Some(menu) = menu {
            lines as u16 + menu.menu_required_lines(terminal_columns)
        } else {
            lines as u16
        }
    }

    /// Rows from the prompt's first row to the row the cursor rests on.
    ///
    /// Places a cursor, so a filled row resolves to the next one. That row is
    /// what `menu_start_row` opens the menu below.
    pub(crate) fn distance_from_prompt(&self, terminal_columns: u16) -> u16 {
        let Some(end) = wrap_position(
            [
                &*self.prompt_str_left,
                &self.prompt_indicator,
                &self.before_cursor,
            ],
            terminal_columns,
        ) else {
            return 0;
        };

        resolve_wrap(end, terminal_columns).1
    }

    /// Calculate the cursor pos, based on the buffer and prompt.
    /// The height is relative to the prompt
    pub(crate) fn cursor_pos(&self, terminal_columns: u16) -> (u16, u16) {
        // If we have a multiline prompt (e.g starship), we expect the cursor to be on the last line
        let prompt_str = format!("{}{}", self.prompt_str_left, self.prompt_indicator);
        // The Cursor position will be relative to this
        let last_prompt_str = prompt_str.lines().last().unwrap_or_default();

        let Some(end) = wrap_position([last_prompt_str, &self.before_cursor], terminal_columns)
        else {
            return (0, 0);
        };

        resolve_wrap(end, terminal_columns)
    }

    /// Rows the prompt occupies, wrapping included. At least 1, since even an
    /// empty prompt owns the row the buffer starts on.
    pub(crate) fn prompt_height(&self, screen_width: u16) -> u16 {
        let complete_prompt = self.prompt_str_left.to_string() + &self.prompt_indicator;
        estimate_required_lines(&complete_prompt, screen_width).max(1) as u16
    }

    /// Estimated width of the line where right prompt will be rendered
    pub(crate) fn estimate_right_prompt_line_width(&self, terminal_columns: u16) -> u16 {
        let first_line_left_prompt = self.prompt_str_left.lines().next();
        let last_line_left_prompt = self.prompt_str_left.lines().last();

        let prompt_lines_total = self.before_cursor.to_string() + &self.after_cursor + &self.hint;
        let prompt_lines_first = prompt_lines_total.lines().next();

        let mut estimate = 0; // space in front of the input

        if self.right_prompt_on_last_line {
            if let Some(last_line_left_prompt) = last_line_left_prompt {
                estimate += line_width(last_line_left_prompt);
                estimate += line_width(&self.prompt_indicator);

                if let Some(prompt_lines_first) = prompt_lines_first {
                    estimate += line_width(prompt_lines_first);
                }
            }
        } else {
            // Render right prompt on the first line
            let required_lines = estimate_required_lines(&self.prompt_str_left, terminal_columns);
            if let Some(first_line_left_prompt) = first_line_left_prompt {
                estimate += line_width(first_line_left_prompt);
            }

            // A single line
            if required_lines == 1 {
                estimate += line_width(&self.prompt_indicator);

                if let Some(prompt_lines_first) = prompt_lines_first {
                    estimate += line_width(prompt_lines_first);
                }
            }
        }

        if estimate > u16::MAX as usize {
            u16::MAX
        } else {
            estimate as u16
        }
    }
}

