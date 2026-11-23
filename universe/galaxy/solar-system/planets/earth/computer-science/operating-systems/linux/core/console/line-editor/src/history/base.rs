use super::HistoryItemId;
use crate::{
    core_editor::LineBuffer, history::HistoryTimestamp, HistoryItem, HistorySessionId, Result,
};

/// Browsing modes for a [`History`]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryNavigationQuery {
    /// `bash` style browsing through the history. Contained `LineBuffer` is used to store the state of manual entry before browsing through the history
    Normal(LineBuffer),
    /// Search for entries starting with a particular string.
    PrefixSearch(String),
    /// Full exact search for all entries containing a string.
    SubstringSearch(String),
    // Suffix Search
    // Fuzzy Search
}

/// Ways to search for a particular command line in the [`History`]
// todo: merge with [HistoryNavigationQuery]
pub enum CommandLineSearch {
    /// Command line starts with the same string
    Prefix(String),
    /// Command line contains the string
    Substring(String),
    /// Command line is the string.
    ///
    /// Useful to gather statistics
    Exact(String),
}

/// Defines how to traverse the history when executing a [`SearchQuery`]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SearchDirection {
    /// From the most recent entry backward
    Backward,
    /// From the least recent entry forward
    Forward,
}

/// A typed expected value for [`SearchFilter::more_info_json`] filters.
///
/// Each variant maps to a JSON scalar type and produces a type-precise SQL comparison
/// using `json_type()` to avoid collisions between JSON booleans, integers, and strings.
/// Arrays and objects are intentionally unsupported.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum JsonFilterValue {
    /// Match a JSON `null` at the given path.
    Null,
    /// Match a JSON boolean (`true` or `false`) at the given path.
    Bool(bool),
    /// Match a JSON integer at the given path. Does not match JSON `true`/`false` or strings.
    Integer(i64),
    /// Match a JSON floating-point number at the given path.
    Real(f64),
    /// Match a JSON string at the given path. Does not match numbers or booleans.
    Text(String),
}

/// Defines additional filters for querying the [`History`]
pub struct SearchFilter {
    /// Query for the command line content
    pub command_line: Option<CommandLineSearch>,
    /// Considered implementation detail for now
    pub(crate) not_command_line: Option<String>, // to skip the currently shown value in up-arrow navigation
    /// Filter based on the executing systems hostname
    pub hostname: Option<String>,
    /// Exact filter for the working directory
    pub cwd_exact: Option<String>,
    /// Prefix filter for the working directory
    pub cwd_prefix: Option<String>,
    /// Filter whether the command completed
    pub exit_successful: Option<bool>,
    /// Filter on the session id
    pub session: Option<HistorySessionId>,
    /// Filter by JSON path in the `more_info` column using SQLite's `json_type()` /
    /// `json_extract()`.
    ///
    /// Each entry is `(json_path, expected_value)`. The [`JsonFilterValue`] variant
    /// determines the exact SQL comparison used, preventing collisions between JSON
    /// booleans, integers, and strings (e.g. `Bool(true)` will not match integer `1`
    /// or string `"true"`).
    ///
    /// Rows where `more_info` is SQL `NULL` or the JSON path does not exist will not match.
    /// Only evaluated by `SqliteBackedHistory::search_with_extra` (`sqlite` feature);
    /// the non-typed [`History`] trait methods and
    /// [`crate::FileBackedHistory`] ignore this field.
    pub more_info_json: Option<Vec<(String, JsonFilterValue)>>,
}

impl SearchFilter {
    /// Create a search filter with a [`CommandLineSearch`]
    pub fn from_text_search(
        cmd: CommandLineSearch,
        session: Option<HistorySessionId>,
    ) -> SearchFilter {
        let mut s = SearchFilter::anything(session);
        s.command_line = Some(cmd);
        s
    }

    /// Create a search filter with a [`CommandLineSearch`] and `cwd`
    pub fn from_text_search_cwd(
        cwd: String,
        cmd: CommandLineSearch,
        session: Option<HistorySessionId>,
    ) -> SearchFilter {
        let mut s = SearchFilter::anything(session);
        s.command_line = Some(cmd);
        s.cwd_exact = Some(cwd);
        s
    }

    /// anything within this session
    pub fn anything(session: Option<HistorySessionId>) -> SearchFilter {
        SearchFilter {
            command_line: None,
            not_command_line: None,
            hostname: None,
            cwd_exact: None,
            cwd_prefix: None,
            exit_successful: None,
            session,
            more_info_json: None,
        }
    }
}

/// Query for search in the potentially rich [`History`]
pub struct SearchQuery {
    /// Direction to search in
    pub direction: SearchDirection,
    /// if given, only get results after/before this time (depending on direction)
    pub start_time: Option<HistoryTimestamp>,
    /// if given, only get results after/before this time (depending on direction)
    pub end_time: Option<HistoryTimestamp>,
    /// if given, only get results after/before this id (depending on direction)
    pub start_id: Option<HistoryItemId>,
    /// if given, only get results after/before this id (depending on direction)
    pub end_id: Option<HistoryItemId>,
    /// How many results to get
    pub limit: Option<i64>,
    /// Additional filters defined with [`SearchFilter`]
    pub filter: SearchFilter,
}

/// Currently `pub` ways to construct a query
impl SearchQuery {
    /// all that contain string in reverse chronological order
    pub fn all_that_contain_rev(contains: String) -> SearchQuery {
        SearchQuery {
            direction: SearchDirection::Backward,
            start_time: None,
            end_time: None,
            start_id: None,
            end_id: None,
            limit: None,
            filter: SearchFilter::from_text_search(CommandLineSearch::Substring(contains), None),
        }
    }

    /// Get the most recent entry matching [`SearchFilter`]
    pub const fn last_with_search(filter: SearchFilter) -> SearchQuery {
        SearchQuery {
            direction: SearchDirection::Backward,
            start_time: None,
            end_time: None,
            start_id: None,
            end_id: None,
            limit: Some(1),
            filter,
        }
    }

    /// Get the most recent entry starting with the `prefix`
    pub fn last_with_prefix(prefix: String, session: Option<HistorySessionId>) -> SearchQuery {
        SearchQuery::last_with_search(SearchFilter::from_text_search(
            CommandLineSearch::Prefix(prefix),
            session,
        ))
    }

    /// Get the most recent entry starting with the `prefix` and `cwd`
    pub fn last_with_prefix_and_cwd(
        prefix: String,
        cwd: String,
        session: Option<HistorySessionId>,
    ) -> SearchQuery {
        let prefix = CommandLineSearch::Prefix(prefix);
        SearchQuery::last_with_search(SearchFilter::from_text_search_cwd(cwd, prefix, session))
    }

    /// Query to get all entries in the given [`SearchDirection`]
    pub fn everything(
        direction: SearchDirection,
        session: Option<HistorySessionId>,
    ) -> SearchQuery {
        SearchQuery {
            direction,
            start_time: None,
            end_time: None,
            start_id: None,
            end_id: None,
            limit: None,
            filter: SearchFilter::anything(session),
        }
    }
}

/// Represents a history file or database
/// Data could be stored e.g. in a plain text file, in a `JSONL` file, in a `SQLite` database
pub trait History: Send {
    /// save a history item to the database
    /// if given id is None, a new id is created and set in the return value
    /// if given id is Some, the existing entry is updated
    fn save(&mut self, h: HistoryItem) -> Result<HistoryItem>;
    /// load a history item by its id
    fn load(&self, id: HistoryItemId) -> Result<HistoryItem>;

    /// count the results of a query
    fn count(&self, query: SearchQuery) -> Result<i64>;
    /// return the total number of history items
    fn count_all(&self) -> Result<i64> {
        self.count(SearchQuery::everything(SearchDirection::Forward, None))
    }
    /// return the results of a query
    fn search(&self, query: SearchQuery) -> Result<Vec<HistoryItem>>;

    /// update an item atomically
    fn update(
        &mut self,
        id: HistoryItemId,
        updater: &dyn Fn(HistoryItem) -> HistoryItem,
    ) -> Result<()>;
    /// delete all history items
    fn clear(&mut self) -> Result<()>;
    /// remove an item from this history
    fn delete(&mut self, h: HistoryItemId) -> Result<()>;
    /// ensure that this history is written to disk
    fn sync(&mut self) -> std::io::Result<()>;
    /// get the history session id
    fn session(&self) -> Option<HistorySessionId>;
}

