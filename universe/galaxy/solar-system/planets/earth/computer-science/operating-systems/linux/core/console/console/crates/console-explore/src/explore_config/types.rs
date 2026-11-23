//! Type definitions for the explore config TUI application.

use console_protocol::engine::{EngineState, Stack};
use ratatui::style::Color;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// Path through the JSON tree represented as a vector of keys/indices
pub type NodePath = Vec<String>;

/// Mode for the editor pane
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EditorMode {
    Normal,
    Editing,
}

/// Information about a node in the tree
#[derive(Debug, Clone)]
pub struct NodeInfo {
    pub path: NodePath,
    pub value_type: ValueType,
    pub shell_type: Option<NuValueType>,
}

/// JSON value types
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ValueType {
    Null,
    Bool,
    Number,
    String,
    Array,
    Object,
}

/// Nushell-specific value types for display in config mode
#[derive(Debug, Clone, PartialEq)]
pub enum NuValueType {
    Nothing,
    Bool,
    Int,
    Float,
    String,
    List,
    Record,
    Closure,
    Filesize,
    Duration,
    Date,
    Glob,
    CellPath,
    Binary,
    Range,
    Custom(String),
    Error,
}

impl NuValueType {
    pub fn from_nu_value(value: &console_protocol::Value) -> Self {
        match value {
            console_protocol::Value::Nothing { .. } => NuValueType::Nothing,
            console_protocol::Value::Bool { .. } => NuValueType::Bool,
            console_protocol::Value::Int { .. } => NuValueType::Int,
            console_protocol::Value::Float { .. } => NuValueType::Float,
            console_protocol::Value::String { .. } => NuValueType::String,
            console_protocol::Value::List { .. } => NuValueType::List,
            console_protocol::Value::Record { .. } => NuValueType::Record,
            console_protocol::Value::Closure { .. } => NuValueType::Closure,
            console_protocol::Value::Filesize { .. } => NuValueType::Filesize,
            console_protocol::Value::Duration { .. } => NuValueType::Duration,
            console_protocol::Value::Date { .. } => NuValueType::Date,
            console_protocol::Value::Glob { .. } => NuValueType::Glob,
            console_protocol::Value::CellPath { .. } => NuValueType::CellPath,
            console_protocol::Value::Binary { .. } => NuValueType::Binary,
            console_protocol::Value::Range { .. } => NuValueType::Range,
            console_protocol::Value::Custom { val, .. } => NuValueType::Custom(val.type_name()),
            console_protocol::Value::Error { .. } => NuValueType::Error,
        }
    }

    pub fn label(&self) -> &str {
        match self {
            NuValueType::Nothing => "nothing",
            NuValueType::Bool => "bool",
            NuValueType::Int => "int",
            NuValueType::Float => "float",
            NuValueType::String => "string",
            NuValueType::List => "list",
            NuValueType::Record => "record",
            NuValueType::Closure => "closure",
            NuValueType::Filesize => "filesize",
            NuValueType::Duration => "duration",
            NuValueType::Date => "date",
            NuValueType::Glob => "glob",
            NuValueType::CellPath => "cell-path",
            NuValueType::Binary => "binary",
            NuValueType::Range => "range",
            NuValueType::Custom(name) => name,
            NuValueType::Error => "error",
        }
    }

    pub fn color(&self) -> Color {
        match self {
            NuValueType::Nothing => crate::roles::dim(),
            NuValueType::Bool => Color::LightCyan,
            NuValueType::Int => crate::roles::text(),
            NuValueType::Float => crate::roles::text(),
            NuValueType::String => crate::roles::text(),
            NuValueType::List => crate::roles::text(),
            NuValueType::Record => crate::roles::text(),
            NuValueType::Closure => crate::roles::text(),
            NuValueType::Filesize => Color::LightMagenta,
            NuValueType::Duration => Color::LightMagenta,
            NuValueType::Date => Color::LightYellow,
            NuValueType::Glob => Color::LightGreen,
            NuValueType::CellPath => Color::LightBlue,
            NuValueType::Binary => crate::roles::text(),
            NuValueType::Range => crate::roles::text(),
            NuValueType::Custom(_) => crate::roles::text(), // Orange
            NuValueType::Error => crate::roles::err(),
        }
    }
}

impl ValueType {
    pub fn from_value(value: &Value) -> Self {
        match value {
            Value::Null => ValueType::Null,
            Value::Bool(_) => ValueType::Bool,
            Value::Number(_) => ValueType::Number,
            Value::String(_) => ValueType::String,
            Value::Array(_) => ValueType::Array,
            Value::Object(_) => ValueType::Object,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ValueType::Null => "null",
            ValueType::Bool => "boolean",
            ValueType::Number => "number",
            ValueType::String => "string",
            ValueType::Array => "array",
            ValueType::Object => "object",
        }
    }

    pub fn color(&self) -> Color {
        match self {
            ValueType::Null => crate::roles::dim(),
            ValueType::Bool => crate::roles::text(),
            ValueType::Number => crate::roles::text(),
            ValueType::String => crate::roles::text(),
            ValueType::Array => crate::roles::text(),
            ValueType::Object => crate::roles::text(),
        }
    }
}

/// Result from running the app - whether to continue or quit
pub enum AppResult {
    Continue,
    Quit,
}

/// Represents a cursor position within multi-line text
#[derive(Debug, Clone, Copy)]
pub struct CursorPosition {
    /// The line number (0-indexed)
    pub line: usize,
    /// The column within the line (0-indexed)
    pub col: usize,
}

/// Calculate the line and column position of a cursor within multi-line text
///
/// # Arguments
/// * `content` - The text content to analyze
/// * `cursor` - The cursor position as a byte offset
///
/// # Returns
/// A `CursorPosition` with the line and column numbers (both 0-indexed)
pub fn calculate_cursor_position(content: &str, cursor: usize) -> CursorPosition {
    let mut pos = 0;
    let mut cursor_line = 0;
    let mut cursor_col = 0;

    for (line_idx, line) in content.lines().enumerate() {
        if pos + line.len() >= cursor {
            cursor_line = line_idx;
            cursor_col = cursor - pos;
            break;
        }
        pos += line.len() + 1; // +1 for newline
        cursor_line = line_idx + 1;
    }

    CursorPosition {
        line: cursor_line,
        col: cursor_col,
    }
}

/// Focus mode including search
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Focus {
    Tree,
    Editor,
    Search,
}

/// The main application state for the TUI
pub struct App {
    pub tree_state: tui_tree_widget::TreeState<String>,
    pub json_data: Value,
    pub tree_items: Vec<tui_tree_widget::TreeItem<'static, String>>,
    /// Unfiltered tree items - used to restore after clearing search
    pub unfiltered_tree_items: Vec<tui_tree_widget::TreeItem<'static, String>>,
    pub node_map: HashMap<String, NodeInfo>,
    pub focus: Focus,
    pub editor_mode: EditorMode,
    pub editor_content: String,
    pub editor_cursor: usize,
    pub editor_scroll: usize,
    pub selected_identifier: String,
    pub status_message: String,
    pub modified: bool,
    /// In config mode, tracks whether user has confirmed they want to save (via Ctrl+S)
    pub confirmed_save: bool,
    pub output_file: Option<String>,
    pub config_mode: bool,
    /// Type map for preserving Nushell types across tree rebuilds in config mode
    pub shell_type_map: Option<HashMap<String, NuValueType>>,
    pub doc_map: Option<HashMap<String, String>>,
    /// Search/filter state
    pub search_query: String,
    pub search_active: bool,
    /// Engine state for syntax highlighting
    pub engine_state: Arc<EngineState>,
    /// Stack for syntax highlighting
    pub stack: Arc<Stack>,
}
