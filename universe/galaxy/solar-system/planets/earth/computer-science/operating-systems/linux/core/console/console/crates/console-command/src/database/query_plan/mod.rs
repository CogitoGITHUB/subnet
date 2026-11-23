use crate::database::values::sqlite::SQLiteQueryBuilder;
use console_protocol::{PipelineData, ShellError, Span, Value};

/// A lazy query plan that can be composed and executed by filter commands.
///
/// This is the deepened seam between filter commands (which want to push down
/// operations like limit, select, count) and database backends (which implement
/// the actual query execution). Each variant wraps a concrete query builder.
///
/// Callers interact with this enum through its methods instead of downcasting
/// to a concrete type. Adding a new backend means adding a variant here;
/// filter commands do not change.
pub enum QueryPlan {
    /// SQLite-based lazy query (via `SQLiteQueryBuilder`).
    Sqlite(SQLiteQueryBuilder),
}

impl QueryPlan {
    /// Try to extract a `QueryPlan` from a `&dyn Any` reference.
    ///
    /// This is the single place where `downcast_ref` happens. All filter
    /// commands call this instead of importing and downcasting to a concrete
    /// query builder type.
    pub fn try_from_any(val: &dyn std::any::Any) -> Option<Self> {
        val.downcast_ref::<SQLiteQueryBuilder>()
            .map(|b| Self::Sqlite(b.clone()))
    }

    /// Apply a LIMIT to the query plan.
    pub fn with_limit(self, limit: i64) -> Self {
        match self {
            Self::Sqlite(b) => Self::Sqlite(b.with_limit(limit)),
        }
    }

    /// Apply an OFFSET to the query plan.
    pub fn with_offset(self, offset: i64) -> Self {
        match self {
            Self::Sqlite(b) => Self::Sqlite(b.with_offset(offset)),
        }
    }

    /// Apply a DISTINCT to the query plan.
    pub fn with_distinct(self) -> Self {
        match self {
            Self::Sqlite(b) => Self::Sqlite(b.with_distinct()),
        }
    }

    /// Apply an ORDER BY to the query plan.
    pub fn with_order_by(self, order_by: String) -> Self {
        match self {
            Self::Sqlite(b) => Self::Sqlite(b.with_order_by(order_by)),
        }
    }

    /// Project the output to a subset of columns.
    ///
    /// Returns `None` if the projection cannot be expressed (e.g. complex
    /// column expressions), allowing callers to fall back to in-memory
    /// processing.
    pub fn project_output_columns(&self, columns: &[String]) -> Option<Self> {
        match self {
            Self::Sqlite(b) => b.project_output_columns(columns).map(Self::Sqlite),
        }
    }

    /// Execute the query and return the result as `PipelineData`.
    pub fn execute(&self, call_span: Span) -> Result<PipelineData, ShellError> {
        match self {
            Self::Sqlite(b) => b.execute(call_span),
        }
    }

    /// Count the number of rows without fetching them.
    pub fn count(&self, call_span: Span) -> Result<i64, ShellError> {
        match self {
            Self::Sqlite(b) => b.count(call_span),
        }
    }

    /// Human-readable type name for error messages.
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Sqlite(_) => "lazy query",
        }
    }

    /// Convert back into a `Value::Custom` for pipeline propagation.
    pub fn into_value(self, span: Span) -> Value {
        match self {
            Self::Sqlite(b) => Value::custom(Box::new(b), span),
        }
    }
}

