use super::{Menu, MenuBuilder, MenuEvent, MenuSettings};
use crate::{
    core_editor::Editor,
    menu_functions::{
        available_lines, get_match_indices, resolve_completer_input, scroll_offset_usize,
        style_suggestion, truncate_with_ansi, CompletionDisplay, CompletionPhase,
    },
    painting::Painter,
    Completer, Suggestion,
};
use nu_ansi_term::ansi::RESET;
use std::num::NonZeroUsize;

/// The traversal direction of the menu
#[derive(Debug, PartialEq, Eq)]
pub enum TraversalDirection {
    /// Traverse horizontally
    Horizontal,
    /// Traverse vertically
    Vertical,
}

/// Default values used as reference for the menu. These values are set during
/// the initial declaration of the menu and are always kept as reference for the
/// changeable [`ColumnDetails`]
struct DefaultColumnDetails {
    /// Number of columns that the menu will have
    pub columns: u16,
    /// Column width
    pub col_width: Option<usize>,
    /// Column padding
    pub col_padding: usize,
    /// Traversal direction
    pub traversal_dir: TraversalDirection,
}

impl Default for DefaultColumnDetails {
    fn default() -> Self {
        Self {
            columns: 4,
            col_width: None,
            col_padding: 2,
            traversal_dir: TraversalDirection::Horizontal,
        }
    }
}

/// Represents the actual column conditions of the menu. These conditions change
/// since they need to accommodate possible different line sizes for the column values
#[derive(Default)]
struct ColumnDetails {
    /// Number of columns that the menu will have
    pub columns: u16,
    /// Column width
    pub col_width: usize,
    /// Number of rows that fit on screen, used as the page size
    pub visible_rows: u16,
}

/// Menu to present suggestions in a columnar fashion
/// It presents a description of the suggestion if available
pub struct ColumnarMenu {
    /// Menu settings
    settings: MenuSettings,
    /// Columnar menu active status
    active: bool,
    /// Default column details that are set when creating the menu
    /// These values are the reference for the working details
    default_details: DefaultColumnDetails,
    /// Number of minimum rows that are displayed when
    /// the required lines is larger than the available lines
    min_rows: u16,
    /// Working column details keep changing based on the collected values
    working_details: ColumnDetails,
    /// Suggestions currently displayed and their derived display metrics
    completions: CompletionDisplay,
    /// Where the menu stands between activation and a final answer about the
    /// line on screen: visibility, what may be decided from the values, and
    /// whether an empty menu means "still computing" or "no records".
    phase: CompletionPhase,
    /// Column position
    col_pos: usize,
    /// row position in the menu. Starts from 0
    row_pos: usize,
    /// Number of rows that are skipped when printing,
    /// depending on selected value and terminal height
    skip_rows: usize,
    /// Rows the menu is cut to so the status line above it stays on the
    /// screen, `None` without one. Set on every layout from
    /// `Painter::menu_room_below_status`.
    status_cap: Option<u16>,
    /// Event sent to the menu
    event: Option<MenuEvent>,
    /// String collected after the menu is activated
    input: Option<String>,
}

impl Default for ColumnarMenu {
    fn default() -> Self {
        Self {
            settings: MenuSettings::default().with_name("columnar_menu"),
            active: false,
            default_details: DefaultColumnDetails::default(),
            min_rows: 3,
            working_details: ColumnDetails::default(),
            completions: CompletionDisplay::default(),
            phase: CompletionPhase::Unasked,
            col_pos: 0,
            row_pos: 0,
            skip_rows: 0,
            status_cap: None,
            event: None,
            input: None,
        }
    }
}

// Menu configuration functions
impl MenuBuilder for ColumnarMenu {
    fn settings_mut(&mut self) -> &mut MenuSettings {
        &mut self.settings
    }
}

// Menu specific configuration functions
impl ColumnarMenu {
    /// Menu builder with new columns value
    #[must_use]
    pub fn with_columns(mut self, columns: u16) -> Self {
        self.default_details.columns = columns;
        self
    }

    /// Menu builder with new column width value
    #[must_use]
    pub fn with_column_width(mut self, col_width: Option<usize>) -> Self {
        self.default_details.col_width = col_width;
        self
    }

    /// Menu builder with new column width value
    #[must_use]
    pub fn with_column_padding(mut self, col_padding: usize) -> Self {
        self.default_details.col_padding = col_padding;
        self
    }

    /// Menu builder with new traversal direction value
    #[must_use]
    pub fn with_traversal_direction(mut self, direction: TraversalDirection) -> Self {
        self.default_details.traversal_dir = direction;
        self
    }
}

// Menu functionality
impl ColumnarMenu {
    /// Move menu cursor to the next element
    fn move_next(&mut self) {
        let new_index = self.index() + 1;

        let new_index = if new_index >= self.get_values().len() {
            0
        } else {
            new_index
        };

        (self.row_pos, self.col_pos) = self.position_from_index(new_index);
    }

    /// Move menu cursor to the previous element
    fn move_previous(&mut self) {
        let new_index = match self.index().checked_sub(1) {
            Some(index) => index,
            None => self.completions.suggestions().len().saturating_sub(1),
        };

        (self.row_pos, self.col_pos) = self.position_from_index(new_index);
    }

    /// Move menu cursor up
    fn move_up(&mut self) {
        self.row_pos = match self.row_pos.checked_sub(1) {
            Some(index) => index,
            None => self.get_last_row_at_col(self.col_pos),
        }
    }

    /// Move menu cursor down
    fn move_down(&mut self) {
        let new_row = self.row_pos + 1;
        self.row_pos = if new_row > self.get_last_row_at_col(self.col_pos) {
            0
        } else {
            new_row
        }
    }

    /// Move menu cursor one page down, stopping at the last row of the column
    fn move_next_page(&mut self) {
        self.row_pos = self
            .row_pos
            .saturating_add(usize::from(self.working_details.visible_rows.max(1)))
            .min(self.get_last_row_at_col(self.col_pos));
    }

    /// Move menu cursor one page up, stopping at the first row
    fn move_previous_page(&mut self) {
        self.row_pos = self
            .row_pos
            .saturating_sub(usize::from(self.working_details.visible_rows.max(1)));
    }

    /// Move menu cursor left
    fn move_left(&mut self) {
        self.col_pos = if let Some(col) = self.col_pos.checked_sub(1) {
            col
        } else {
            self.get_last_col_at_row(self.row_pos)
        }
    }

    /// Move menu cursor right
    fn move_right(&mut self) {
        let new_col = self.col_pos + 1;
        self.col_pos = if new_col > self.get_last_col_at_row(self.row_pos) {
            0
        } else {
            new_col
        }
    }

    /// Calculates row and column positions from an index
    fn position_from_index(&self, index: usize) -> (usize, usize) {
        match self.default_details.traversal_dir {
            TraversalDirection::Vertical => {
                let rows = self.grid_rows().get();
                (index % rows, index / rows)
            }
            TraversalDirection::Horizontal => {
                let cols = self.grid_cols().get();
                (index / cols, index % cols)
            }
        }
    }

    /// Calculates the last row containing a value for the specified column
    fn get_last_row_at_col(&self, col_pos: usize) -> usize {
        let num_values = self.get_values().len();
        match self.default_details.traversal_dir {
            TraversalDirection::Vertical => {
                if col_pos >= self.grid_cols().get() - 1 {
                    // Last column, might not be full
                    let mod_val = num_values % self.grid_rows();
                    if mod_val == 0 {
                        // Full column
                        self.grid_rows().get().saturating_sub(1)
                    } else {
                        // Column with last row empty
                        mod_val.saturating_sub(1)
                    }
                } else {
                    // Full column
                    self.grid_rows().get().saturating_sub(1)
                }
            }
            TraversalDirection::Horizontal => {
                let mod_val = num_values % self.grid_cols();
                if mod_val > 0 && col_pos >= mod_val {
                    // Column with last row empty
                    self.grid_rows().get().saturating_sub(2)
                } else {
                    // Full column
                    self.grid_rows().get().saturating_sub(1)
                }
            }
        }
    }

    /// Calculates the last column containing a value for the specified row
    fn get_last_col_at_row(&self, row_pos: usize) -> usize {
        let num_values = self.get_values().len();
        match self.default_details.traversal_dir {
            TraversalDirection::Vertical => {
                let mod_val = num_values % self.grid_rows();
                if mod_val > 0 && row_pos >= mod_val {
                    // Row with last column empty
                    self.get_used_cols().saturating_sub(2)
                } else {
                    // Full row
                    self.get_used_cols().saturating_sub(1)
                }
            }
            TraversalDirection::Horizontal => {
                if row_pos >= self.grid_rows().get() - 1 {
                    // Last row, might not be full
                    let mod_val = num_values % self.grid_cols();
                    if mod_val == 0 {
                        // Full row
                        self.get_used_cols().saturating_sub(1)
                    } else {
                        // Row with some columns empty
                        mod_val.saturating_sub(1)
                    }
                } else {
                    // Full row
                    self.get_used_cols().saturating_sub(1)
                }
            }
        }
    }

    /// Menu index based on column and row position
    fn index(&self) -> usize {
        match self.default_details.traversal_dir {
            TraversalDirection::Vertical => self.col_pos * self.grid_rows().get() + self.row_pos,
            TraversalDirection::Horizontal => self.row_pos * self.grid_cols().get() + self.col_pos,
        }
    }

    /// Calculates how many rows the menu will use
    ///
    /// The *layout* answer, so it may be 0 while a completion is still in
    /// flight; grid arithmetic asks [`grid_rows`](Self::grid_rows) instead.
    fn get_rows(&self) -> u16 {
        if self.get_values().is_empty() && self.phase.awaiting_results() {
            // No reason to save space if we're waiting for results.
            0
        } else {
            self.grid_rows().get().min(u16::MAX as usize) as u16
        }
    }

    /// How many rows the value grid has, at least 1 by construction: `%` and
    /// `/` by it cannot panic and need no per-site zero check.
    fn grid_rows(&self) -> NonZeroUsize {
        let values = self.get_values().len();
        NonZeroUsize::new(values.div_ceil(usize::from(self.get_cols())))
            .unwrap_or(NonZeroUsize::MIN)
    }

    /// Calculates how many columns will be used to display values
    fn get_used_cols(&self) -> usize {
        self.grid_cols().get()
    }

    /// The column counterpart of [`grid_rows`](Self::grid_rows), likewise at
    /// least 1 by construction.
    fn grid_cols(&self) -> NonZeroUsize {
        let values = self.get_values().len();
        let cols = match self.default_details.traversal_dir {
            TraversalDirection::Vertical => values.div_ceil(self.grid_rows().get()),
            TraversalDirection::Horizontal => usize::from(self.get_cols()).min(values),
        };
        NonZeroUsize::new(cols).unwrap_or(NonZeroUsize::MIN)
    }

    /// Returns working details col width
    fn get_width(&self) -> usize {
        self.working_details.col_width
    }

    fn no_records_msg(&self, use_ansi_coloring: bool) -> String {
        let msg = "NO RECORDS FOUND";
        if use_ansi_coloring {
            format!(
                "{}{}{}",
                self.settings.color.selected_text_style.prefix(),
                msg,
                RESET
            )
        } else {
            msg.to_string()
        }
    }

    /// Returns working details columns
    fn get_cols(&self) -> u16 {
        self.working_details.columns.max(1)
    }

    /// Width of the suggestion at `index`, 0 when out of range.
    ///
    /// `display_widths` is built from the same `values` in
    /// [`CompletionDisplay::new`], so the index holds today; 0 keeps a
    /// mismatch painting (unpadded) instead of aborting mid-repaint.
    fn display_width(&self, index: usize) -> usize {
        self.completions
            .display_widths
            .get(index)
            .copied()
            .unwrap_or(0)
    }

    /// Creates default string that represents one suggestion from the menu
    pub fn create_string(
        &self,
        suggestion: &Suggestion,
        index: usize,
        use_ansi_coloring: bool,
    ) -> String {
        match use_ansi_coloring {
            true => self.format_ansi(suggestion, index),
            false => self.format_plain(suggestion, index),
        }
    }

    /// Horizontal split of a menu row into the suggestion column and the
    /// description that follows it.
    fn column_split(&self, terminal_width: usize) -> (usize, usize) {
        let maximum_left_size =
            self.completions.longest_suggestion + self.default_details.col_padding;
        let left_text_size = terminal_width.min(maximum_left_size);
        let description_size = terminal_width.saturating_sub(left_text_size);

        (left_text_size, description_size)
    }

    /// Handles plain-text formatting.
    fn format_plain(&self, suggestion: &Suggestion, index: usize) -> String {
        let is_selected = index == self.index();
        let terminal_width = self.get_width();
        let display_value = suggestion.display_value();

        // Calculate the remaining space after the suggestion text
        let empty_space = terminal_width.saturating_sub(self.display_width(index));
        let marker = if is_selected { ">" } else { "" };
        let description = suggestion.description.as_deref().unwrap_or("");

        let mut formatted_line = if description.is_empty() {
            // If there is no description, pad with empty space to fill the terminal width.
            let padding_width = empty_space.saturating_sub(marker.len());
            format!("{marker}{display_value}{:padding_width$}", "")
        } else {
            // The left column is capped at `left_text_size` and the description
            // takes only what's left, so the row can't exceed the terminal
            let (left_text_size, description_size) = self.column_split(terminal_width);
            let padding_width = left_text_size.saturating_sub(marker.len());
            let mut base_line = format!("{marker}{display_value:padding_width$}");

            base_line.extend(description.chars().take(description_size).map(|character| {
                if character == '\n' {
                    ' '
                } else {
                    character
                }
            }));
            base_line
        };

        if is_selected {
            // Modifies in-place. Prevents unicode edge cases as well.
            formatted_line.make_ascii_uppercase();
        }

        formatted_line
    }

    /// Handles ANSI-colored formatting.
    fn format_ansi(&self, suggestion: &Suggestion, index: usize) -> String {
        let is_selected = index == self.index();
        let terminal_width = self.get_width();
        let display_value = suggestion.display_value();
        let display_width = self.display_width(index);
        let color_settings = &self.settings.color;

        // TODO(ysthakur): let the user strip quotes, rather than doing it here
        // Natively trim standard quote characters using a character array match.
        let base_string = self
            .completions
            .shortest_base_string
            .trim_start_matches(['`', '\'', '"']);
        let match_indices =
            get_match_indices(display_value, &suggestion.match_indices, base_string);

        // Calculate spatial boundaries for the suggestion text and its description.
        let (left_text_size, description_size) = self.column_split(terminal_width);

        // Resolve ANSI styles based on selection state.
        let text_style = suggestion
            .style
            .as_ref()
            .unwrap_or(&color_settings.text_style);
        let match_style = if is_selected {
            &color_settings.selected_match_style
        } else {
            &color_settings.match_style
        };

        let selected_style = is_selected.then_some(&color_settings.selected_text_style);

        // Truncate and apply ANSI highlighting to the primary suggestion text.
        let truncated_value = truncate_with_ansi(display_value, left_text_size);
        let styled_value = style_suggestion(
            &truncated_value,
            &match_indices,
            text_style,
            match_style,
            selected_style,
        );

        // Extract the description, but only if it's long enough
        // to be visible (> 3).
        let description = suggestion
            .description
            .as_deref()
            .filter(|_| description_size > 3)
            .unwrap_or("");

        if description.is_empty() {
            let padding_width = terminal_width.saturating_sub(display_width);
            return format!("{styled_value}{RESET}{:padding_width$}", "");
        }

        // Prepare the description by filtering out newlines
        // and applying truncation and paint.
        let padding_size = left_text_size.saturating_sub(display_width);
        let cleaned_description: String = description
            .chars()
            .filter(|&character| character != '\n')
            .collect();
        let painted_description = color_settings
            .description_style
            .paint(truncate_with_ansi(&cleaned_description, description_size));

        match is_selected {
            true => format!(
                "{styled_value}{RESET}{}{}{:padding_size$}{painted_description}{RESET}",
                text_style.prefix(),
                color_settings.selected_text_style.prefix(),
                ""
            ),
            false => format!(
                "{styled_value}{:padding_size$}{RESET}{painted_description}{RESET}",
                ""
            ),
        }
    }

    /// Apply a queued menu event, refreshing the values or moving the selection
    fn apply_event(
        &mut self,
        event: MenuEvent,
        editor: &mut Editor,
        completer: &mut dyn Completer,
    ) {
        match event {
            MenuEvent::Activate(updated) | MenuEvent::Edit(updated) => {
                self.reload(updated, editor, completer)
            }
            MenuEvent::Deactivate => {}
            // No values, no grid: nothing to move over.
            _ if self.get_values().is_empty() => {}
            MenuEvent::NextElement => self.move_next(),
            MenuEvent::PreviousElement => self.move_previous(),
            MenuEvent::MoveUp => self.move_up(),
            MenuEvent::MoveDown => self.move_down(),
            MenuEvent::MoveLeft => self.move_left(),
            MenuEvent::MoveRight => self.move_right(),
            MenuEvent::NextPage => self.move_next_page(),
            MenuEvent::PreviousPage => self.move_previous_page(),
        }
    }

    /// Recompute the completion box geometry from the current suggestions,
    /// selection, and terminal size. Runs on every repaint.
    pub fn recompute_layout(&mut self, painter: &Painter) {
        let screen_width = painter.screen_width() as usize;

        // If there is at least one suggestion that contains a description, then the layout
        // is changed to one column to fit the description
        let has_descriptions = self
            .get_values()
            .iter()
            .any(|suggestion| suggestion.description.is_some());

        if has_descriptions {
            self.working_details.columns = 1;
            self.working_details.col_width = screen_width;
        } else {
            // If no default width is found, then the total screen width is used to estimate
            // the column width based on the default number of columns
            // Floored: `with_columns(0)` is representable.
            let default_width = self
                .default_details
                .col_width
                .unwrap_or_else(|| screen_width / self.default_details.columns.max(1) as usize);

            // Adjusting the working width of the column based the max line width found
            // in the menu values
            let required_width =
                self.completions.longest_suggestion + self.default_details.col_padding;

            // Floored: 0 on a 0-width terminal, and `possible_columns`
            // divides by this.
            self.working_details.col_width =
                required_width.max(default_width).min(screen_width).max(1);

            // The working columns is adjusted based on possible number of columns
            // that could be fitted in the screen with the calculated column width
            let possible_columns = (screen_width / self.working_details.col_width) as u16;
            self.working_details.columns =
                possible_columns.min(self.default_details.columns).max(1);
        }

        // A grid taller than the screen would push the status line out, so it
        // scrolls within the rows left below it instead.
        self.status_cap = painter.menu_room_below_status(self.min_rows());
        let available = available_lines(
            painter,
            self.min_rows(),
            self.status_cap.unwrap_or(u16::MAX),
        );
        self.working_details.visible_rows = available;
        self.skip_rows = scroll_offset_usize(self.row_pos, self.skip_rows, usize::from(available));
    }
}

impl Menu for ColumnarMenu {
    /// Menu settings
    fn settings(&self) -> &MenuSettings {
        &self.settings
    }

    /// Active status
    fn is_active(&self) -> bool {
        self.active
    }

    /// The columnar menu can to quick complete if there is only one element
    fn can_quick_complete(&self) -> bool {
        true
    }

    /// The columnar menu can try to find the common string and replace it
    /// in the given line buffer
    fn can_partially_complete(
        &mut self,
        values_updated: bool,
        editor: &mut Editor,
        completer: &mut dyn Completer,
    ) -> bool {
        // If the values were already updated (e.g. quick completions are true)
        // there is no need to update the values from the menu
        if !values_updated {
            self.update_values(editor, completer);
        }

        // Stale results can't splice wrong span
        if self.completions.common_prefix(editor) {
            // Recalculate spans after replacement
            self.update_values(editor, completer);

            true
        } else {
            false
        }
    }

    fn set_active(&mut self, active: bool) {
        self.active = active;
    }

    fn clear_input(&mut self) {
        self.input = None;
    }

    fn on_activate(&mut self) {
        // Reset completions on activation
        self.completions = CompletionDisplay::default();
        self.phase.on_activate();
    }

    /// Queue menu event
    fn menu_event(&mut self, event: MenuEvent) {
        self.handle_menu_event(&event);
        self.event = Some(event);
    }

    /// Updates menu values
    fn reset_position(&mut self) {
        self.col_pos = 0;
        self.row_pos = 0;
    }

    fn update_values(&mut self, editor: &mut Editor, completer: &mut dyn Completer) {
        let (input, pos) = resolve_completer_input(editor, &mut self.input, &self.settings);

        let (result, base_ranges) = completer.complete_with_base_ranges(&input, pos);
        self.phase.note(&result);
        if let Some(completions) = CompletionDisplay::from_result(result, &base_ranges, editor) {
            self.completions = completions;
            self.reset_position();
        }
    }

    /// The working details for the menu changes based on the size of the lines
    /// collected from the completer
    fn update_working_details(
        &mut self,
        editor: &mut Editor,
        completer: &mut dyn Completer,
        painter: &Painter,
    ) {
        if let Some(event) = self.event.take() {
            self.apply_event(event, editor, completer);
        }

        self.recompute_layout(painter);
    }

    /// Replace buffer at span via completion display
    fn replace_in_buffer(&self, editor: &mut Editor) {
        self.completions
            .accept(self.index(), editor, self.settings.output_mode);
    }

    /// Minimum rows that should be displayed by the menu
    fn min_rows(&self) -> u16 {
        self.get_rows().min(self.min_rows)
    }

    /// Gets values from filler that will be displayed in the menu
    fn get_values(&self) -> &[Suggestion] {
        self.completions.suggestions()
    }

    fn results_are_provisional(&self) -> bool {
        self.phase.provisional()
    }

    fn is_awaiting_first_answer(&self) -> bool {
        self.phase.awaiting_first_answer()
    }

    fn selection_at(
        &self,
        column: u16,
        menu_row: u16,
        _terminal_columns: u16,
        available_lines: u16,
    ) -> Option<usize> {
        if self.get_values().is_empty() || self.get_width() == 0 {
            return None;
        }

        let columns = self.grid_cols().get();
        let rows = self.grid_rows().get();

        let visible_rows = rows.min(available_lines as usize);
        let menu_column = column as usize / self.get_width();
        if menu_column >= columns
            || menu_row as usize >= visible_rows
            || self.skip_rows + menu_row as usize >= rows
        {
            return None;
        }

        let index = match self.default_details.traversal_dir {
            TraversalDirection::Vertical => menu_column * rows + self.skip_rows + menu_row as usize,
            TraversalDirection::Horizontal => {
                (self.skip_rows + menu_row as usize) * columns + menu_column
            }
        };
        if index >= self.get_values().len() {
            return None;
        }

        Some(index)
    }

    fn set_selection(&mut self, index: usize) {
        if index >= self.get_values().len() {
            return;
        }

        let (row_pos, col_pos) = self.position_from_index(index);
        self.row_pos = row_pos;
        self.col_pos = col_pos;
    }

    fn menu_required_lines(&self, _terminal_columns: u16) -> u16 {
        self.get_rows().min(self.status_cap.unwrap_or(u16::MAX))
    }

    fn menu_string(&self, available_lines: u16, use_ansi_coloring: bool) -> String {
        // No more rows than `menu_required_lines` booked: the painter offers
        // every row down to the bottom of the screen.
        let available_lines = available_lines.min(self.status_cap.unwrap_or(u16::MAX));
        if self.get_values().is_empty() {
            if self.phase.awaiting_results() {
                // A background completion is still running; draw nothing rather
                // than flashing "NO RECORDS FOUND" before the results land.
                String::new()
            } else {
                self.no_records_msg(use_ansi_coloring)
            }
        } else {
            // It seems that crossterm prefers to have a complete string ready to be printed
            // rather than looping through the values and printing multiple things
            // This reduces the flickering when printing the menu
            match self.default_details.traversal_dir {
                TraversalDirection::Vertical => {
                    // `grid_rows`, since `step_by(0)` panics: the type proves
                    // what the enclosing non-empty branch implies.
                    let num_rows = self.grid_rows().get();
                    let rows_to_draw = num_rows.min(available_lines.into());
                    let mut menu_string = String::new();
                    for line in 0..rows_to_draw {
                        let skip_value = self.skip_rows + line;
                        let row_string: String = self
                            .get_values()
                            .iter()
                            .enumerate()
                            .skip(skip_value)
                            .step_by(num_rows)
                            .take(self.get_cols().into())
                            .map(|(index, suggestion)| {
                                self.create_string(suggestion, index, use_ansi_coloring)
                            })
                            .collect();
                        menu_string.push_str(&row_string);
                        menu_string.push_str("\r\n");
                    }
                    menu_string
                }
                TraversalDirection::Horizontal => {
                    // Widened to usize so the products cannot overflow u16.
                    let available_values =
                        usize::from(available_lines) * usize::from(self.get_cols());
                    let skip_values = self.skip_rows * self.grid_cols().get();

                    self.get_values()
                        .iter()
                        .skip(skip_values)
                        .take(available_values)
                        .enumerate()
                        .map(|(index, suggestion)| {
                            // Correcting the enumerate index based on the number of skipped values
                            let index = index + skip_values;
                            let column = index % self.get_cols() as usize;

                            let end_of_line =
                                if column == self.get_cols().saturating_sub(1) as usize {
                                    "\r\n"
                                } else {
                                    ""
                                };
                            format!(
                                "{}{}",
                                self.create_string(suggestion, index, use_ansi_coloring),
                                end_of_line
                            )
                        })
                        .collect()
                }
            }
        }
    }
}

