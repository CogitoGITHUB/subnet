#[cfg(feature = "_sqlite")]
use rusqlite::ToSql;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    fmt::Display,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Unique ID for the [`HistoryItem`]. More recent items have higher ids than older ones.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct HistoryItemId(pub i64);
impl HistoryItemId {
    /// Create a new `HistoryItemId` value
    pub const fn new(i: i64) -> HistoryItemId {
        HistoryItemId(i)
    }
}

impl Display for HistoryItemId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique ID for the session in which reedline was run to disambiguate different sessions
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistorySessionId(pub i64);
impl HistorySessionId {
    /// Wrap a raw session id, e.g. one read back from the history store.
    pub const fn new(i: i64) -> HistorySessionId {
        HistorySessionId(i)
    }
}

impl Display for HistorySessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(feature = "_sqlite")]
impl ToSql for HistorySessionId {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        Ok(rusqlite::types::ToSqlOutput::Owned(
            rusqlite::types::Value::Integer(self.0),
        ))
    }
}

impl From<HistorySessionId> for i64 {
    fn from(id: HistorySessionId) -> Self {
        id.0
    }
}

/// This trait represents additional arbitrary context to be added to a history (optional, see [`HistoryItem`])
pub trait HistoryItemExtraInfo: Serialize + DeserializeOwned + Default + Send {}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
/// something that is serialized as null and deserialized by ignoring everything
pub struct IgnoreAllExtraInfo;

impl Serialize for IgnoreAllExtraInfo {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        Option::<IgnoreAllExtraInfo>::None.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for IgnoreAllExtraInfo {
    fn deserialize<D>(d: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        serde::de::IgnoredAny::deserialize(d).map(|_| IgnoreAllExtraInfo)
    }
}

impl HistoryItemExtraInfo for IgnoreAllExtraInfo {}

/// A point in time as milliseconds since the Unix epoch (UTC)
///
/// Millisecond precision matches what the sqlite backend stores, so a saved
/// item reads back as equal
#[derive(Clone, Copy, PartialOrd, Ord, Hash, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HistoryTimestamp(i64);

impl HistoryTimestamp {
    /// Wrap a raw millisecond count, e.g. one read back from the history store.
    pub const fn from_millis(millis: i64) -> Self {
        Self(millis)
    }

    /// The milliseconds since the Unix epoch.
    pub const fn to_millis(self) -> i64 {
        self.0
    }

    /// The current system time
    pub fn now() -> Self {
        SystemTime::now().into()
    }
}

// Times before the epoch come out as negative rather than clamped
impl From<SystemTime> for HistoryTimestamp {
    fn from(value: SystemTime) -> Self {
        let millis = match value.duration_since(UNIX_EPOCH) {
            Ok(d) => d.as_millis() as i64,
            Err(e) => -(e.duration().as_millis() as i64),
        };

        Self(millis)
    }
}

/// Represents one run command with some optional additional context
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryItem<ExtraInfo: HistoryItemExtraInfo = IgnoreAllExtraInfo> {
    /// primary key, unique across one history
    pub id: Option<HistoryItemId>,
    /// when this command was started
    pub start_timestamp: Option<HistoryTimestamp>,
    /// the full command line as text
    pub command_line: String,
    /// a unique id for one shell session.
    /// used so the history can be filtered to a single session
    pub session_id: Option<HistorySessionId>,
    /// the hostname the commands were run in
    pub hostname: Option<String>,
    /// the current working directory
    pub cwd: Option<String>,
    /// the duration the command took to complete
    pub duration: Option<Duration>,
    /// the exit status of the command
    pub exit_status: Option<i64>,
    /// arbitrary additional information that might be interesting
    /// NOTE: this attribute is required because of
    /// <https://github.com/rust-lang/rust/issues/41617>
    ///       (see <https://github.com/serde-rs/serde/issues/1296#issuecomment-394056188> for the fix)
    #[serde(deserialize_with = "Option::<ExtraInfo>::deserialize")]
    pub more_info: Option<ExtraInfo>,
}

impl HistoryItem {
    /// create a history item purely from the command line with everything else set to None
    pub fn from_command_line(cmd: impl Into<String>) -> HistoryItem {
        HistoryItem {
            id: None,
            start_timestamp: None,
            command_line: cmd.into(),
            session_id: None,
            hostname: None,
            cwd: None,
            duration: None,
            exit_status: None,
            more_info: None,
        }
    }
}

