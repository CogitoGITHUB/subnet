use crate::{History, HistoryNavigationQuery, HistorySessionId};

use super::base::CommandLineSearch;
use super::base::SearchDirection;
use super::base::SearchFilter;
use super::HistoryItem;
use super::SearchQuery;
use crate::Result;

/// Interface of a stateful navigation via [`HistoryNavigationQuery`].
#[derive(Debug)]
pub struct HistoryCursor {
    query: HistoryNavigationQuery,
    current: Option<HistoryItem>,
    skip_dupes: bool,
    session: Option<HistorySessionId>,
}

impl HistoryCursor {
    pub fn new(query: HistoryNavigationQuery, session: Option<HistorySessionId>) -> HistoryCursor {
        HistoryCursor {
            query,
            current: None,
            skip_dupes: true,
            session,
        }
    }

    /// This moves the cursor backwards respecting the navigation query that is set
    /// - Results in a no-op if the cursor is at the initial point
    pub fn back(&mut self, history: &dyn History) -> Result<()> {
        self.navigate_in_direction(history, SearchDirection::Backward)
    }

    /// This moves the cursor forwards respecting the navigation-query that is set
    /// - Results in a no-op if the cursor is at the latest point
    pub fn forward(&mut self, history: &dyn History) -> Result<()> {
        self.navigate_in_direction(history, SearchDirection::Forward)
    }

    /// Moves the cursor to the oldest entry the navigation query matches.
    /// Returns whether the cursor moved, so a jump from the oldest entry or
    /// on a history without a match is a no-op.
    pub fn jump_to_oldest(&mut self, history: &dyn History) -> Result<bool> {
        let mut oldest = history.search(SearchQuery {
            direction: SearchDirection::Forward,
            ..SearchQuery::last_with_search(self.get_query_filter())
        })?;
        let Some(oldest) = oldest.pop() else {
            return Ok(false);
        };
        if self.current.as_ref().map(|e| e.id) == Some(oldest.id) {
            return Ok(false);
        }
        self.current = Some(oldest);
        Ok(true)
    }

    /// Moves the cursor past the newest entry, where the navigation started.
    pub fn past_newest(&mut self) {
        self.current = None;
    }

    fn get_query_filter(&self) -> SearchFilter {
        match self.query.clone() {
            HistoryNavigationQuery::Normal(_) => SearchFilter::anything(self.session),
            HistoryNavigationQuery::PrefixSearch(prefix) => {
                SearchFilter::from_text_search(CommandLineSearch::Prefix(prefix), self.session)
            }
            HistoryNavigationQuery::SubstringSearch(substring) => SearchFilter::from_text_search(
                CommandLineSearch::Substring(substring),
                self.session,
            ),
        }
    }

    fn get_search_filter(&self) -> SearchFilter {
        let filter = self.get_query_filter();
        if let (true, Some(current)) = (self.skip_dupes, &self.current) {
            SearchFilter {
                not_command_line: Some(current.command_line.clone()),
                ..filter
            }
        } else {
            filter
        }
    }
    fn navigate_in_direction(
        &mut self,
        history: &dyn History,
        direction: SearchDirection,
    ) -> Result<()> {
        if direction == SearchDirection::Forward && self.current.is_none() {
            // if searching forward but we don't have a starting point, assume we are at the end
            return Ok(());
        }
        let start_id = self.current.as_ref().and_then(|e| e.id);
        let mut next = history.search(SearchQuery {
            start_id,
            end_id: None,
            start_time: None,
            end_time: None,
            direction,
            limit: Some(1),
            filter: self.get_search_filter(),
        })?;
        if next.len() == 1 {
            self.current = Some(next.swap_remove(0));
        } else if direction == SearchDirection::Forward {
            // no result and searching forward: we are at the end
            self.current = None;
        }
        Ok(())
    }

    /// Returns the string (if present) at the cursor
    pub fn string_at_cursor(&self) -> Option<String> {
        self.current.as_ref().map(|e| e.command_line.to_string())
    }

    /// Poll the current [`HistoryNavigationQuery`] mode
    pub fn get_navigation(&self) -> HistoryNavigationQuery {
        self.query.clone()
    }
}

