use crate::utils::{input::Input, strings::EMPTY_STRING};
use ratatui::widgets::ListState;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Hash)]
pub struct Picker<T> {
    pub(crate) state: ListState,
    pub(crate) relative_state: ListState,
    inverted: bool,
    pub(crate) input: Input,
    pub entries: Arc<Vec<T>>,
    pub total_items: u32,
}

impl<T> Default for Picker<T> {
    fn default() -> Self {
        Self::new(None)
    }
}

impl<T> Picker<T> {
    pub fn new(input: Option<String>) -> Self {
        Self {
            state: ListState::default(),
            relative_state: ListState::default(),
            inverted: false,
            input: Input::new(input.unwrap_or(EMPTY_STRING.to_string())),
            entries: Arc::new(Vec::new()),
            total_items: 0,
        }
    }

    pub(crate) fn offset(&self) -> usize {
        self.selected()
            .unwrap_or(0)
            .saturating_sub(self.relative_selected().unwrap_or(0))
    }

    pub(crate) fn inverted(mut self) -> Self {
        self.inverted = !self.inverted;
        self
    }

    pub(crate) fn reset_selection(&mut self) {
        self.state.select(Some(0));
        self.relative_state.select(Some(0));
    }

    pub(crate) fn reset_input(&mut self) {
        self.input.reset();
    }

    pub(crate) fn select_next(
        &mut self,
        step: u32,
        total_items: usize,
        height: usize,
    ) {
        if self.inverted {
            for _ in 0..step {
                self.inner_prev(total_items, height);
            }
        } else {
            for _ in 0..step {
                self.inner_next(total_items, height);
            }
        }
    }

    pub(crate) fn select_prev(
        &mut self,
        step: u32,
        total_items: usize,
        height: usize,
    ) {
        if self.inverted {
            for _ in 0..step {
                self.inner_next(total_items, height);
            }
        } else {
            for _ in 0..step {
                self.inner_prev(total_items, height);
            }
        }
    }

    fn inner_next(&mut self, total_items: usize, height: usize) {
        let selected = self.selected().unwrap_or(0);
        let relative_selected = self.relative_selected().unwrap_or(0);

        let new_selected = selected.wrapping_add(1) % total_items;
        self.select(Some(new_selected));

        if new_selected == 0 {
            self.relative_select(Some(0));
        } else {
            self.relative_select(Some(
                (relative_selected + 1).min(height - 1),
            ));
        }
    }

    fn inner_prev(&mut self, total_items: usize, height: usize) {
        let selected = self.selected().unwrap_or(0);
        let relative_selected = self.relative_selected().unwrap_or(0);

        let new_selected =
            selected.wrapping_add(total_items - 1) % total_items;
        self.select(Some(new_selected));

        if new_selected == total_items - 1 {
            self.relative_select(Some((height - 1).min(total_items - 1)));
        } else {
            self.relative_select(Some(relative_selected.saturating_sub(1)));
        }
    }
    /// Generic cursor movement helper.
    pub fn move_cursor(
        &mut self,
        movement: Movement,
        step: u32,
        total_items: usize,
        picker_ui_height: usize,
    ) {
        // Early return for empty collections
        if total_items == 0 {
            return;
        }

        match movement {
            Movement::Next => {
                self.select_next(step, total_items, picker_ui_height);
            }
            Movement::Prev => {
                self.select_prev(step, total_items, picker_ui_height);
            }
        }
    }
}

impl<Entry> Picker<Entry> {
    pub(crate) fn selected(&self) -> Option<usize> {
        self.state.selected()
    }

    pub(crate) fn select(&mut self, index: Option<usize>) {
        self.state.select(index);
    }

    fn relative_selected(&self) -> Option<usize> {
        self.relative_state.selected()
    }

    pub(crate) fn relative_select(&mut self, index: Option<usize>) {
        self.relative_state.select(index);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Cursor movement: `Movement::Next` or `Movement::Prev`.
pub enum Movement {
    Next,
    Prev,
}

