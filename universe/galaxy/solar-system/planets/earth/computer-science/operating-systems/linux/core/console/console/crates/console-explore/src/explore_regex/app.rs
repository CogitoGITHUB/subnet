//! Application state and logic for the regex explorer.

use crate::explore_regex::colors;
use crate::explore_regex::quick_ref::{QuickRefEntry, get_flattened_entries};
use edtui::{EditorMode, EditorState, Highlight, Index2, Lines, actions::InsertChar};
use fancy_regex::Regex;

/// Which pane currently has input focus.
#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum InputFocus {
    #[default]
    Regex,
    Sample,
    QuickRef,
}

/// Main application state for the regex explorer.
pub struct App {
    /// Which input pane currently has focus (Regex, Sample, or QuickRef)
    pub input_focus: InputFocus,
    /// Editor state for the regex pattern input (single-line)
    pub regex_input: EditorState,
    /// Editor state for the test string input (multi-line)
    pub sample_text: EditorState,
    /// Compiled regex pattern, if valid
    pub compiled_regex: Option<Regex>,
    /// Error message from regex compilation, if invalid
    pub regex_error: Option<String>,
    /// Number of regex matches found in the sample text
    pub match_count: usize,
    // Quick reference panel state
    /// Whether the quick reference panel is currently visible
    pub show_quick_ref: bool,
    /// Whether the help modal is currently visible
    pub show_help: bool,
    /// Index of the currently selected quick reference entry
    pub quick_ref_selected: usize,
    /// Vertical scroll position in the quick reference panel
    pub quick_ref_scroll: usize,
    /// Horizontal scroll position in the quick reference panel
    pub quick_ref_scroll_h: u16,
    /// Height of the quick reference viewport (updated each frame)
    pub quick_ref_view_height: usize,
    /// Width of the quick reference viewport (updated each frame)
    pub quick_ref_view_width: u16,
    /// Flattened list of all quick reference entries
    pub quick_ref_entries: Vec<QuickRefEntry>,
}

impl App {
    pub fn new(input_string: String) -> Self {
        let mut regex_input = EditorState::default();
        regex_input.mode = EditorMode::Insert; // Enable modeless editing

        let mut sample_text = EditorState::new(Lines::from(input_string.as_str()));
        sample_text.mode = EditorMode::Insert; // Enable modeless editing

        let entries = get_flattened_entries();
        let initial_selected = Self::find_next_item(&entries, 0).unwrap_or(0);

        Self {
            input_focus: InputFocus::default(),
            regex_input,
            sample_text,
            compiled_regex: None,
            regex_error: None,
            match_count: 0,
            show_quick_ref: false,
            show_help: false,
            quick_ref_selected: initial_selected,
            quick_ref_scroll: 0,
            quick_ref_scroll_h: 0,
            quick_ref_view_height: 0,
            quick_ref_view_width: 0,
            quick_ref_entries: entries,
        }
    }

    pub fn get_sample_text(&self) -> String {
        self.sample_text.lines.to_string()
    }

    pub fn get_regex_input(&self) -> String {
        self.regex_input.lines.to_string()
    }

    pub fn compile_regex(&mut self) {
        self.compiled_regex = None;
        self.regex_error = None;
        self.match_count = 0;

        let regex_input = self.regex_input.lines.to_string();

        if regex_input.is_empty() {
            return;
        }

        match Regex::new(&regex_input) {
            Ok(regex) => {
                self.compiled_regex = Some(regex);
                self.update_match_count();
            }
            Err(e) => {
                self.regex_error = Some(e.to_string());
            }
        }
    }

    /// Update match count using the already-compiled regex.
    /// This is more efficient than `compile_regex()` when only the sample text changes.
    pub fn update_match_count(&mut self) {
        if let Some(ref regex) = self.compiled_regex {
            let sample_text = self.get_sample_text();
            self.match_count = regex.captures_iter(&sample_text).flatten().count();
        }
    }

    /// Generate edtui Highlights for regex matches.
    pub fn get_highlights(&self) -> Vec<Highlight> {
        let text = self.get_sample_text();
        let Some(regex) = &self.compiled_regex else {
            return Vec::new();
        };

        // Build byte_offset -> (row, col) mapping for Unicode support
        let mut byte_to_pos: Vec<(usize, usize)> = vec![(0, 0); text.len() + 1];
        let (mut row, mut col) = (0, 0);
        for (i, ch) in text.char_indices() {
            for pos in byte_to_pos.iter_mut().skip(i).take(ch.len_utf8()) {
                *pos = (row, col);
            }
            if ch == '\n' {
                row += 1;
                col = 0;
            } else {
                col += 1;
            }
        }
        byte_to_pos[text.len()] = (row, col);

        let mut highlights = Vec::new();
        for cap in regex.captures_iter(&text).flatten() {
            for (group, m) in cap.iter().enumerate() {
                let Some(m) = m else { continue };
                let start = byte_to_pos[m.start()];
                let end = byte_to_pos[m.end().saturating_sub(1)];
                let size = m.end() - m.start();
                highlights.push((
                    size,
                    Highlight::new(
                        Index2::new(start.0, start.1),
                        Index2::new(end.0, end.1),
                        colors::highlight_style(group),
                    ),
                ));
            }
        }

        // Sort by span size (smallest first) so inner groups render last and take precedence
        highlights.sort_by_key(|(size, _)| *size);
        highlights.into_iter().map(|(_, h)| h).collect()
    }

    // ─── Quick Reference Panel ───────────────────────────────────────────

    /// Toggle the quick reference panel visibility.
    pub fn toggle_quick_ref(&mut self) {
        self.show_quick_ref = !self.show_quick_ref;
        self.input_focus = if self.show_quick_ref {
            InputFocus::QuickRef
        } else {
            InputFocus::Regex
        };
    }

    /// Close quick ref panel and return focus to regex input.
    pub fn close_quick_ref(&mut self) {
        self.show_quick_ref = false;
        self.input_focus = InputFocus::Regex;
    }

    /// Toggle the help modal visibility.
    pub fn toggle_help(&mut self) {
        self.show_help = !self.show_help;
    }

    /// Move selection up in the quick reference list.
    pub fn quick_ref_up(&mut self) {
        if let Some(prev) = Self::find_prev_item(&self.quick_ref_entries, self.quick_ref_selected) {
            self.quick_ref_selected = prev;
        }
    }

    /// Move selection down in the quick reference list.
    pub fn quick_ref_down(&mut self) {
        if let Some(next) = Self::find_next_item(&self.quick_ref_entries, self.quick_ref_selected) {
            self.quick_ref_selected = next;
        }
    }

    /// Move selection up by one page.
    pub fn quick_ref_page_up(&mut self) {
        let target = self
            .quick_ref_selected
            .saturating_sub(self.quick_ref_view_height.max(1));
        // Find the nearest selectable item at or after target
        self.quick_ref_selected = Self::find_nearest_item(&self.quick_ref_entries, target)
            .unwrap_or(self.quick_ref_selected);
    }

    /// Move selection down by one page.
    pub fn quick_ref_page_down(&mut self) {
        let target = (self.quick_ref_selected + self.quick_ref_view_height.max(1))
            .min(self.quick_ref_entries.len().saturating_sub(1));
        // Find the nearest selectable item at or before target
        self.quick_ref_selected = Self::find_nearest_item_reverse(&self.quick_ref_entries, target)
            .unwrap_or(self.quick_ref_selected);
    }

    /// Insert the selected quick reference item into the regex input.
    pub fn insert_selected_quick_ref(&mut self) {
        if let Some(QuickRefEntry::Item(item)) = self.quick_ref_entries.get(self.quick_ref_selected)
        {
            // Insert each character of the item at the cursor position
            for ch in item.insert.chars() {
                self.regex_input.execute(InsertChar(ch));
            }
            self.compile_regex();
        }
    }

    /// Scroll quick reference panel left.
    pub fn quick_ref_scroll_left(&mut self) {
        self.quick_ref_scroll_h = self.quick_ref_scroll_h.saturating_sub(4);
    }

    /// Scroll quick reference panel right.
    pub fn quick_ref_scroll_right(&mut self) {
        self.quick_ref_scroll_h = self.quick_ref_scroll_h.saturating_add(4);
    }

    /// Scroll quick reference panel to home (beginning of line).
    pub fn quick_ref_scroll_home(&mut self) {
        self.quick_ref_scroll_h = 0;
    }

    /// Check if an entry at the given index is selectable (i.e., an Item, not a Category).
    fn is_selectable(entries: &[QuickRefEntry], idx: usize) -> bool {
        matches!(entries.get(idx), Some(QuickRefEntry::Item(_)))
    }

    /// Find the next selectable item after (not including) the given index.
    fn find_next_item(entries: &[QuickRefEntry], from: usize) -> Option<usize> {
        ((from + 1)..entries.len()).find(|&i| Self::is_selectable(entries, i))
    }

    /// Find the previous selectable item before (not including) the given index.
    fn find_prev_item(entries: &[QuickRefEntry], from: usize) -> Option<usize> {
        (0..from).rev().find(|&i| Self::is_selectable(entries, i))
    }

    /// Find the nearest selectable item at or after the given index.
    fn find_nearest_item(entries: &[QuickRefEntry], from: usize) -> Option<usize> {
        (from..entries.len()).find(|&i| Self::is_selectable(entries, i))
    }

    /// Find the nearest selectable item at or before the given index.
    fn find_nearest_item_reverse(entries: &[QuickRefEntry], from: usize) -> Option<usize> {
        (0..=from).rev().find(|&i| Self::is_selectable(entries, i))
    }
}

