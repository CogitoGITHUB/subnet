use std::{collections::HashSet, ops::Deref};

use crate::{
    history::SearchQuery, menu_functions::parse_selection_char, Completer, CompletionResult,
    History, HistoryItem, Result, Span, Suggestion,
};

const SELECTION_CHAR: char = '!';

// The HistoryCompleter is created just before updating the menu
// It pulls data from the object that contains access to the History
pub(crate) struct HistoryCompleter<'menu>(&'menu dyn History);

fn search_unique(
    completer: &HistoryCompleter,
    line: &str,
) -> Result<impl Iterator<Item = HistoryItem>> {
    let parsed = parse_selection_char(line, SELECTION_CHAR);
    let values = completer.0.search(SearchQuery::all_that_contain_rev(
        parsed.remainder.to_string(),
    ))?;

    let mut seen_matching_command_lines = HashSet::new();
    Ok(values
        .into_iter()
        .filter(move |value| seen_matching_command_lines.insert(value.command_line.clone())))
}

impl Completer for HistoryCompleter<'_> {
    fn complete(&mut self, line: &str, pos: usize) -> CompletionResult {
        let suggestions = match search_unique(self, line) {
            Err(_) => vec![],
            Ok(search_results) => search_results
                .map(|value| self.create_suggestion(line, pos, value.command_line.deref()))
                .collect(),
        };
        CompletionResult::fresh(suggestions)
    }
}

impl<'menu> HistoryCompleter<'menu> {
    pub fn new(history: &'menu dyn History) -> Self {
        Self(history)
    }

    /// Assumes `line.len() <= pos` (i.e. `line` is the cursor-prefix slice).
    /// Update this span calculation before HistoryMenu opts into `InputMode::FullBuffer`,
    /// where `line` would be the entire buffer and `pos - line.len()` would underflow.
    fn create_suggestion(&self, line: &str, pos: usize, value: &str) -> Suggestion {
        let span = Span {
            start: pos - line.len(),
            end: pos,
        };

        Suggestion {
            value: value.to_string(),
            description: None,
            style: None,
            extra: None,
            span,
            append_whitespace: false,
            ..Default::default()
        }
    }
}

