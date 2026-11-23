use std::{collections::HashMap, io::prelude::*, path::PathBuf, str::FromStr, sync::OnceLock};
use tokio::sync::OnceCell;

use shell_history_common::record::HostId;
use shell_history_common::utils;
use clap::ValueEnum;
use config::{
    Config, ConfigBuilder, Environment, File as ConfigFile, FileFormat, builder::DefaultState,
};
use eyre::{Context, Error, Result, bail, eyre};
use fs_err::{File, create_dir_all};
use humantime::parse_duration;
use regex::RegexSet;
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_with::DeserializeFromStr;
use time::{OffsetDateTime, UtcOffset, format_description::FormatItem, macros::format_description};

pub const HISTORY_PAGE_SIZE: i64 = 100;
static EXAMPLE_CONFIG: &str = include_str!("../config.toml");

static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();
static META_CONFIG: OnceLock<(String, f64)> = OnceLock::new();
static META_STORE: OnceCell<crate::meta::MetaStore> = OnceCell::const_new();

mod dotfiles;
mod kv;
pub(crate) mod meta;
mod scripts;
pub mod watcher;

#[derive(derive_more::AsRef)]
#[as_ref(str)]
pub struct HubEndpoint(String);

/// Default sync address for console history's hosted service
pub const DEFAULT_SYNC_ADDRESS: &str = "http://127.0.0.1:8888";

/// Default Hub web/API endpoint for console history's hosted service
pub const DEFAULT_HUB_ENDPOINT: &str = "http://127.0.0.1:8888";

impl Default for HubEndpoint {
    fn default() -> Self {
        HubEndpoint(DEFAULT_HUB_ENDPOINT.to_string())
    }
}

#[derive(Clone, Debug, Deserialize, Copy, ValueEnum, PartialEq, Serialize)]
pub enum SearchMode {
    #[serde(rename = "prefix")]
    Prefix,

    #[serde(rename = "fulltext")]
    #[clap(aliases = &["fulltext"])]
    FullText,

    #[serde(rename = "fuzzy")]
    Fuzzy,

    #[serde(rename = "skim")]
    Skim,

    #[serde(rename = "daemon-fuzzy")]
    #[clap(aliases = &["daemon-fuzzy"])]
    DaemonFuzzy,
}

impl SearchMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            SearchMode::Prefix => "PREFIX",
            SearchMode::FullText => "FULLTXT",
            SearchMode::Fuzzy => "FUZZY",
            SearchMode::Skim => "SKIM",
            SearchMode::DaemonFuzzy => "DAEMON",
        }
    }
    pub fn next(&self, settings: &Settings) -> Self {
        match self {
            SearchMode::Prefix => SearchMode::FullText,
            // if the user is using skim, we go to skim
            SearchMode::FullText if settings.search_mode == SearchMode::Skim => SearchMode::Skim,
            // if the user is using daemon-fuzzy, we go to daemon-fuzzy
            SearchMode::FullText if settings.search_mode == SearchMode::DaemonFuzzy => {
                SearchMode::DaemonFuzzy
            }
            // otherwise fuzzy.
            SearchMode::FullText => SearchMode::Fuzzy,
            SearchMode::Fuzzy | SearchMode::Skim | SearchMode::DaemonFuzzy => SearchMode::Prefix,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Copy, PartialEq, Eq, ValueEnum, Serialize)]
pub enum FilterMode {
    #[serde(rename = "global")]
    Global = 0,

    #[serde(rename = "host")]
    Host = 1,

    #[serde(rename = "session")]
    Session = 2,

    #[serde(rename = "directory")]
    Directory = 3,

    #[serde(rename = "workspace")]
    Workspace = 4,

    #[serde(rename = "session-preload")]
    SessionPreload = 5,
}

impl FilterMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            FilterMode::Global => "GLOBAL",
            FilterMode::Host => "HOST",
            FilterMode::Session => "SESSION",
            FilterMode::Directory => "DIRECTORY",
            FilterMode::Workspace => "WORKSPACE",
            FilterMode::SessionPreload => "SESSION+",
        }
    }
}

// FIXME: Can use upstream Dialect enum if https://github.com/stevedonovan/chrono-english/pull/16 is merged
// FIXME: Above PR was merged, but dependency was changed to interim (fork of chrono-english) in the ... interim
#[derive(Clone, Debug, Deserialize, Copy, Serialize)]
pub enum Dialect {
    #[serde(rename = "us")]
    Us,

    #[serde(rename = "uk")]
    Uk,
}

impl From<Dialect> for interim::Dialect {
    fn from(d: Dialect) -> interim::Dialect {
        match d {
            Dialect::Uk => interim::Dialect::Uk,
            Dialect::Us => interim::Dialect::Us,
        }
    }
}

/// Type wrapper around `time::UtcOffset` to support a wider variety of timezone formats.
///
/// Note that the parsing of this struct needs to be done before starting any
/// multithreaded runtime, otherwise it will fail on most Unix systems.
///
/// See: <console>
#[derive(
    Clone, Copy, Debug, Eq, PartialEq, DeserializeFromStr, Serialize, derive_more::Display,
)]
#[display("{_0}")]
pub struct Timezone(pub UtcOffset);
/// format: `<+|-><hour>[:<minute>[:<second>]]`
static OFFSET_FMT: &[FormatItem<'_>] = format_description!(
    "[offset_hour sign:mandatory padding:none][optional [:[offset_minute padding:none][optional [:[offset_second padding:none]]]]]"
);
impl FromStr for Timezone {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        // local timezone
        if matches!(s.to_lowercase().as_str(), "l" | "local") {
            // There have been some timezone issues, related to errors fetching it on some
            // platforms
            // Rather than fail to start, fallback to UTC. The user should still be able to specify
            // their timezone manually in the config file.
            let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
            return Ok(Self(offset));
        }

        if matches!(s.to_lowercase().as_str(), "0" | "utc") {
            let offset = UtcOffset::UTC;
            return Ok(Self(offset));
        }

        // offset from UTC
        if let Ok(offset) = UtcOffset::parse(s, OFFSET_FMT) {
            return Ok(Self(offset));
        }

        // IDEA: Currently named timezones are not supported, because the well-known crate
        // for this is `chrono_tz`, which is not really interoperable with the datetime crate
        // that we currently use - `time`. If ever we migrate to using `chrono`, this would
        // be a good feature to add.

        bail!(r#""{s}" is not a valid timezone spec"#)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Stats {
    #[serde(default = "Stats::common_prefix_default")]
    pub common_prefix: Vec<String>, // sudo, etc. commands we want to strip off
    #[serde(default = "Stats::common_subcommands_default")]
    pub common_subcommands: Vec<String>, // kubectl, commands we should consider subcommands for
    #[serde(default = "Stats::ignored_commands_default")]
    pub ignored_commands: Vec<String>, // cd, ls, etc. commands we want to completely hide from stats
}

impl Stats {
    fn common_prefix_default() -> Vec<String> {
        vec!["sudo", "doas"].into_iter().map(String::from).collect()
    }

    fn common_subcommands_default() -> Vec<String> {
        vec![
            "apt",
            "cargo",
            "composer",
            "dnf",
            "docker",
            "dotnet",
            "git",
            "go",
            "ip",
            "jj",
            "kubectl",
            "nix",
            "nmcli",
            "npm",
            "pecl",
            "pnpm",
            "podman",
            "port",
            "systemctl",
            "tmux",
            "yarn",
        ]
        .into_iter()
        .map(String::from)
        .collect()
    }

    fn ignored_commands_default() -> Vec<String> {
        vec![]
    }
}

impl Default for Stats {
    fn default() -> Self {
        Self {
            common_prefix: Self::common_prefix_default(),
            common_subcommands: Self::common_subcommands_default(),
            ignored_commands: Self::ignored_commands_default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Default, Serialize)]
pub struct Sync {
    pub records: bool,
}

/// Sync protocol type for authentication.
///
/// This setting is primarily for development/testing. When not explicitly set,
/// the protocol is inferred from the sync_address:
/// - Default sync address (127.0.0.1:8888) → Hub protocol
/// - Custom sync address → Legacy protocol
///
/// Set explicitly to "hub" to use Hub authentication with a custom sync_address
/// (useful for local development against a Hub instance).
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SyncProtocol {
    /// Use Hub authentication (Bearer token from Hub OAuth flow)
    Hub,
    /// Use legacy CLI authentication (Token from CLI register/login)
    Legacy,
    /// Infer from sync_address (default behavior)
    #[default]
    Auto,
}

/// Resolved authentication state for sync operations.
///
/// Determined at runtime by examining which tokens are available and what
/// server the client is configured to talk to. Operations use this to pick
/// the right auth header and endpoint style.
#[cfg(feature = "sync")]
#[derive(Debug, Clone)]
pub enum SyncAuth {
    /// Self-hosted Rust server. Uses `Authorization: Token <session>` and
    /// legacy endpoints.
    Legacy { token: String },
    /// Hub with a valid Hub API token (`atapi_*`). Uses
    /// `Authorization: Bearer <token>` and v0 endpoints.
    Hub { token: String },
    /// Targeting Hub but only has a CLI session token. Uses
    /// `Authorization: Token <session>` against compat/record endpoints.
    /// Sync, password change, and account deletion still work, but the user
    /// should be nudged to run `console history login` for full Hub auth.
    HubViaCli { token: String },
    /// Not authenticated at all. Contains an actionable user-facing message.
    NotLoggedIn { reason: String },
}

#[cfg(feature = "sync")]
impl SyncAuth {
    /// Convert into the auth token type used by the API client.
    ///
    /// Returns an error with an actionable message for `NotLoggedIn`.
    pub fn into_auth_token(self) -> Result<crate::api_client::AuthToken> {
        use crate::api_client::AuthToken;
        match self {
            SyncAuth::Legacy { token } => Ok(AuthToken::Token(token)),
            SyncAuth::Hub { token } => Ok(AuthToken::Bearer(token)),
            SyncAuth::HubViaCli { token } => Ok(AuthToken::Token(token)),
            SyncAuth::NotLoggedIn { reason } => Err(eyre!(reason)),
        }
    }
}



#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Search {
    /// The list of enabled filter modes, in order of priority.
    pub filters: Vec<FilterMode>,

    /// The recency score multiplier for the search index (default: 1.0).
    /// Values < 1.0 reduce weight, > 1.0 increase weight, 0.0 disables.
    pub recency_score_multiplier: f64,

    /// The frequency score multiplier for the search index (default: 1.0).
    /// Values < 1.0 reduce weight, > 1.0 increase weight, 0.0 disables.
    pub frequency_score_multiplier: f64,

    /// The overall frecency score multiplier for the search index (default: 1.0).
    /// Applied after combining recency and frequency scores.
    pub frecency_score_multiplier: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Tmux {
    /// Enable using console history with tmux popup (tmux >= 3.2)
    pub enabled: bool,

    /// Width of the tmux popup (percentage)
    pub width: String,

    /// Height of the tmux popup (percentage)
    pub height: String,
}

/// Log level for file logging. Maps to tracing's LevelFilter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    #[default]
    Info,
    Warn,
    Error,
}

impl LogLevel {
    /// Convert to a tracing directive string for use with EnvFilter.
    pub fn as_directive(&self) -> &'static str {
        match self {
            LogLevel::Trace => "trace",
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        }
    }
}

/// Configuration for a specific log type (search or daemon).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct LogConfig {
    /// Log file name (relative to dir) or absolute path.
    pub file: String,

    /// Override global enabled setting for this log type.
    pub enabled: Option<bool>,

    /// Override global level setting for this log type.
    pub level: Option<LogLevel>,

    /// Override global retention days setting for this log type.
    pub retention: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Logs {
    /// Enable file logging globally. Defaults to true.
    #[serde(default = "Logs::default_enabled")]
    pub enabled: bool,

    /// Directory for log files. Defaults to ~/.console/logs
    pub dir: String,

    /// Default log level for file logging. Defaults to "info".
    /// Note: CONSOLE_SHELL_HISTORY_LOG environment variable overrides this.
    #[serde(default)]
    pub level: LogLevel,

    /// Default retention days for log files. Defaults to 4.
    #[serde(default = "Logs::default_retention")]
    pub retention: u64,

    /// Search log settings
    #[serde(default)]
    pub search: LogConfig,

    /// Daemon log settings
    #[serde(default)]
    pub daemon: LogConfig,

    /// AI log settings
    #[serde(default)]
    pub ai: LogConfig,
}

/// Endpoint protocol for console history AI.
///
/// When set to "auto" (default), the protocol is inferred from `ai.endpoint`:
/// an unset or official console history address is treated as Hub, anything else as an
/// OSS server. Set explicitly to "hub" to keep Hub behavior with a custom
/// endpoint (useful for local development against a Hub instance).
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AiEndpointProtocol {
    /// console history Hub: browser-based login flow, stored Hub session, usage reporting.
    Hub,
    /// A standalone AI server (e.g. shell-history-ai-server): requests go straight to
    /// the endpoint, authenticated with `ai.api_token` if set.
    Oss,
    /// Infer from ai.endpoint (default behavior)
    #[default]
    Auto,
}

#[derive(Default, Clone, Debug, Deserialize, Serialize)]
pub struct Ai {
    /// Whether or not the AI features are enabled.
    pub enabled: Option<bool>,

    /// The address of the console history AI endpoint. Used for AI features like command generation.
    /// Only necessary for custom AI endpoints.
    pub endpoint: Option<String>,

    /// How to talk to `endpoint`. See [`AiEndpointProtocol`].
    #[serde(default)]
    pub endpoint_protocol: AiEndpointProtocol,

    /// The API token for the console history AI endpoint. Used for AI features like command generation.
    /// Only necessary for custom AI endpoints.
    pub api_token: Option<String>,

    /// Path to the AI sessions database.
    pub db_path: String,

    /// The maximum time in minutes that an AI session can be automatically resumed.
    pub session_continue_minutes: i64,

    /// The AI model to use for AI chats, based on the console history AI model alias.
    pub model: Option<String>,

    /// Deprecated: use opening.send_cwd instead. Kept for backwards compatibility.
    #[serde(default)]
    pub send_cwd: Option<bool>,

    /// Configuration for what context is sent in the opening AI request.
    #[serde(default)]
    pub opening: AiOpening,

    /// Tool capability flags.
    #[serde(default)]
    pub capabilities: AiCapabilities,
}

#[derive(Default, Clone, Debug, Deserialize, Serialize)]
pub struct AiCapabilities {
    /// Whether the AI can request to search console history history. `None` = unset (defaults to enabled, and the ai will ask for permission).
    pub enable_history_search: Option<bool>,
    /// Whether the AI can request to view the stored output, if any, for console history history entries. `None` = unset (defaults to enabled, and the ai will ask for permission).
    pub enable_history_output: Option<bool>,
    /// Whether the AI can request to read and write files. `None` = unset (defaults to enabled, and the ai will ask for permission).
    pub enable_file_tools: Option<bool>,
    /// Whether the AI can request to execute bash commands. `None` = unset (defaults to enabled, and the ai will ask for permission).
    pub enable_command_execution: Option<bool>,
}

#[derive(Default, Clone, Debug, Deserialize, Serialize)]
pub struct AiOpening {
    /// Whether or not to send the current working directory to the AI endpoint.
    pub send_cwd: Option<bool>,

    /// Whether or not to send the last command as context in the opening AI request.
    pub send_last_command: Option<bool>,
}



impl Default for Logs {
    fn default() -> Self {
        Self {
            enabled: true,
            dir: "".to_string(),
            level: LogLevel::default(),
            retention: Self::default_retention(),
            search: LogConfig {
                file: "search.log".to_string(),
                ..Default::default()
            },
            daemon: LogConfig {
                file: "daemon.log".to_string(),
                ..Default::default()
            },
            ai: LogConfig {
                file: "ai.log".to_string(),
                ..Default::default()
            },
        }
    }
}

impl Logs {
    fn default_enabled() -> bool {
        true
    }

    fn default_retention() -> u64 {
        4
    }

    /// Returns whether search logging is enabled.
    /// Uses search-specific setting if set, otherwise falls back to global.
    pub fn search_enabled(&self) -> bool {
        self.search.enabled.unwrap_or(self.enabled)
    }

    /// Returns whether daemon logging is enabled.
    /// Uses daemon-specific setting if set, otherwise falls back to global.
    pub fn daemon_enabled(&self) -> bool {
        self.daemon.enabled.unwrap_or(self.enabled)
    }

    /// Returns whether AI logging is enabled.
    /// Uses AI-specific setting if set, otherwise falls back to global.
    pub fn ai_enabled(&self) -> bool {
        self.ai.enabled.unwrap_or(self.enabled)
    }

    /// Returns the log level for search logging.
    /// Uses search-specific setting if set, otherwise falls back to global.
    pub fn search_level(&self) -> LogLevel {
        self.search.level.unwrap_or(self.level)
    }

    /// Returns the log level for daemon logging.
    /// Uses daemon-specific setting if set, otherwise falls back to global.
    pub fn daemon_level(&self) -> LogLevel {
        self.daemon.level.unwrap_or(self.level)
    }

    /// Returns the log level for AI logging.
    /// Uses AI-specific setting if set, otherwise falls back to global.
    pub fn ai_level(&self) -> LogLevel {
        self.ai.level.unwrap_or(self.level)
    }

    /// Returns the retention days for search logging.
    /// Uses search-specific setting if set, otherwise falls back to global.
    pub fn search_retention(&self) -> u64 {
        self.search.retention.unwrap_or(self.retention)
    }

    /// Returns the retention days for daemon logging.
    /// Uses daemon-specific setting if set, otherwise falls back to global.
    pub fn daemon_retention(&self) -> u64 {
        self.daemon.retention.unwrap_or(self.retention)
    }

    /// Returns the retention days for AI logging.
    /// Uses AI-specific setting if set, otherwise falls back to global.
    pub fn ai_retention(&self) -> u64 {
        self.ai.retention.unwrap_or(self.retention)
    }

    /// Returns the full path for the search log file.
    pub fn search_path(&self) -> PathBuf {
        let path = PathBuf::from(&self.search.file);
        PathBuf::from(&self.dir).join(path)
    }

    /// Returns the full path for the daemon log file.
    pub fn daemon_path(&self) -> PathBuf {
        let path = PathBuf::from(&self.daemon.file);
        PathBuf::from(&self.dir).join(path)
    }

    /// Returns the full path for the AI log file.
    pub fn ai_path(&self) -> PathBuf {
        let path = PathBuf::from(&self.ai.file);
        PathBuf::from(&self.dir).join(path)
    }
}

impl Default for Search {
    fn default() -> Self {
        Self {
            filters: vec![
                FilterMode::Global,
                FilterMode::Host,
                FilterMode::Session,
                FilterMode::SessionPreload,
                FilterMode::Workspace,
                FilterMode::Directory,
            ],

            recency_score_multiplier: 1.0,
            frequency_score_multiplier: 1.0,
            frecency_score_multiplier: 1.0,
        }
    }
}

impl Default for Tmux {
    fn default() -> Self {
        Self {
            enabled: false,
            width: "80%".to_string(),
            height: "60%".to_string(),
        }
    }
}

// Custom deserialize to handle both string and object formats:
// "duration" or { type = "duration", width = 8, expand = true }
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Settings {
    pub data_dir: Option<String>,
    pub dialect: Dialect,
    pub timezone: Timezone,
    pub auto_sync: bool,
    pub update_check: bool,

    /// The sync address for console history.
    pub sync_address: String,

    /// Sync protocol for authentication. When set to "auto" (default), the protocol
    /// is inferred from sync_address. Set to "hub" to force Hub auth with a custom
    /// sync_address (useful for local development).
    #[serde(default)]
    pub sync_protocol: SyncProtocol,

    pub sync_frequency: String,
    pub db_path: String,
    pub record_store_path: String,
    pub key_path: String,
    pub search_mode: SearchMode,
    pub filter_mode: Option<FilterMode>,
    pub history_format: String,
    pub strip_trailing_whitespace: bool,
    pub store_failed: bool,
    pub no_mouse: bool,

    #[serde(with = "serde_regex", default = "RegexSet::empty", skip_serializing)]
    pub history_filter: RegexSet,

    #[serde(with = "serde_regex", default = "RegexSet::empty", skip_serializing)]
    pub cwd_filter: RegexSet,

    pub secrets_filter: bool,
    pub workspaces: bool,
    pub ctrl_n_shortcuts: bool,

    pub network_connect_timeout: u64,
    pub network_timeout: u64,
    pub local_timeout: f64,
    pub enter_accept: bool,
    pub smart_sort: bool,
    pub command_chaining: bool,

    #[serde(default)]
    pub stats: Stats,

    #[serde(default)]
    pub sync: Sync,




    #[serde(default)]
    pub dotfiles: dotfiles::Settings,


    #[serde(default)]
    pub search: Search,


    #[serde(default)]
    pub scripts: scripts::Settings,

    #[serde(default)]
    pub kv: kv::Settings,

    #[serde(default)]
    pub tmux: Tmux,

    #[serde(default)]
    pub logs: Logs,

    #[serde(default)]
    pub meta: meta::Settings,

    #[serde(default)]
    pub ai: Ai,
}

impl Settings {
    pub fn utc() -> Self {
        Self::builder()
            .expect("Could not build default")
            .set_override("timezone", "0")
            .expect("failed to override timezone with UTC")
            .build()
            .expect("Could not build config")
            .try_deserialize()
            .expect("Could not deserialize config")
    }

    pub(crate) fn effective_data_dir() -> PathBuf {
        DATA_DIR
            .get()
            .cloned()
            .unwrap_or_else(shell_history_common::utils::data_dir)
    }

    // -- Meta store: lazily initialized on first access --

    pub async fn meta_store() -> Result<&'static crate::meta::MetaStore> {
        META_STORE
            .get_or_try_init(|| async {
                let (db_path, timeout) = META_CONFIG.get().ok_or_else(|| {
                    eyre!("meta store config not set — Settings::new() has not been called")
                })?;
                crate::meta::MetaStore::new(db_path, *timeout).await
            })
            .await
    }

    pub async fn host_id() -> Result<HostId> {
        Self::meta_store().await?.host_id().await
    }

    pub async fn last_sync() -> Result<OffsetDateTime> {
        Self::meta_store().await?.last_sync().await
    }

    pub async fn save_sync_time() -> Result<()> {
        Self::meta_store().await?.save_sync_time().await
    }

    pub async fn last_version_check() -> Result<OffsetDateTime> {
        Self::meta_store().await?.last_version_check().await
    }

    pub async fn save_version_check_time() -> Result<()> {
        Self::meta_store().await?.save_version_check_time().await
    }

    pub async fn should_sync(&self) -> Result<bool> {
        if !self.auto_sync || !Self::meta_store().await?.logged_in().await? {
            return Ok(false);
        }

        if self.sync_frequency == "0" {
            return Ok(true);
        }

        match parse_duration(self.sync_frequency.as_str()) {
            Ok(d) => {
                let d = time::Duration::try_from(d)?;
                Ok(OffsetDateTime::now_utc() - Settings::last_sync().await? >= d)
            }
            Err(e) => Err(eyre!("failed to check sync: {}", e)),
        }
    }

    pub async fn logged_in(&self) -> Result<bool> {
        Self::meta_store().await?.logged_in().await
    }

    pub async fn session_token(&self) -> Result<String> {
        match Self::meta_store().await?.session_token().await? {
            Some(token) => Ok(token),
            None => Err(eyre!("Tried to load session; not logged in")),
        }
    }

    pub async fn hub_session_token(&self) -> Result<String> {
        match Self::meta_store().await?.hub_session_token().await? {
            Some(token) => Ok(token),
            None => Err(eyre!("Tried to load hub session; not logged in")),
        }
    }

    /// Normalize a URL for comparison by trimming trailing slashes
    fn normalize_url(url: &str) -> &str {
        url.trim_end_matches('/')
    }

    /// Check if a URL matches one of console history's official hosted addresses
    fn is_official_address(url: &str) -> bool {
        let normalized = Self::normalize_url(url);
        normalized == Self::normalize_url(DEFAULT_SYNC_ADDRESS)
            || normalized == Self::normalize_url(DEFAULT_HUB_ENDPOINT)
    }

    /// Returns whether this configuration uses Hub-style sync.
    ///
    /// Hub sync uses Bearer token authentication and is the default for
    /// console history's hosted service. This returns true when:
    /// - `sync_protocol` is explicitly set to `Hub`, OR
    /// - `sync_protocol` is `Auto` and `sync_address` is an official console history address
    pub fn is_hub_sync(&self) -> bool {
        match self.sync_protocol {
            SyncProtocol::Hub => true,
            SyncProtocol::Legacy => false,
            SyncProtocol::Auto => false,
        }
    }

    /// Returns whether the resolved AI endpoint should be treated as an console history
    /// Hub instance (browser login flow, Hub session management, usage
    /// reporting) rather than a standalone OSS server.
    ///
    /// `endpoint` is the resolved AI endpoint — the `--api-endpoint` flag or
    /// `ai.endpoint` setting, after defaults are applied — which is why it's a
    /// parameter rather than read from `self.ai.endpoint`.
    pub fn is_hub_ai_endpoint(&self, endpoint: &str) -> bool {
        match self.ai.endpoint_protocol {
            AiEndpointProtocol::Hub => true,
            AiEndpointProtocol::Oss => false,
            AiEndpointProtocol::Auto => Self::is_official_address(endpoint),
        }
    }

    /// Returns the base URL for the Hub endpoint.
    ///
    /// For console history's official hosted service, this always returns `http://127.0.0.1:8888`
    /// regardless of whether `sync_address` is `127.0.0.1:8888` or `127.0.0.1:8888`.
    /// For self-hosted instances, returns the configured `sync_address`.
    pub fn active_hub_endpoint(&self) -> Option<HubEndpoint> {
        if self.is_hub_sync() {
            if Self::is_official_address(&self.sync_address) {
                Some(HubEndpoint::default())
            } else {
                Some(HubEndpoint(self.sync_address.clone()))
            }
        } else {
            None
        }
    }

    /// Examines the configured sync target and available tokens to determine
    /// the correct auth strategy. Also performs cleanup of mis-stored tokens
    /// (e.g. a CLI token incorrectly saved in the Hub session slot).
    #[cfg(feature = "sync")]
    pub async fn resolve_sync_auth(&self) -> SyncAuth {
        let meta = match Self::meta_store().await {
            Ok(m) => m,
            Err(e) => {
                return SyncAuth::NotLoggedIn {
                    reason: format!("Failed to open meta store: {e}"),
                };
            }
        };

        if !self.is_hub_sync() {
            // Self-hosted / legacy server
            return match meta.session_token().await {
                Ok(Some(token)) => SyncAuth::Legacy { token },
                _ => SyncAuth::NotLoggedIn {
                    reason: "Not logged in. Run 'console history login' to authenticate \
                             with your sync server."
                        .into(),
                },
            };
        }

        // Targeting Hub — check for a valid Hub API token first
        if let Ok(Some(hub_token)) = meta.hub_session_token().await {
            if hub_token.starts_with("atapi_") {
                return SyncAuth::Hub { token: hub_token };
            }

            // A non-atapi_ token in the hub_session slot is a mis-stored CLI
            // token (from the migration-fallback bug). Move it to the CLI
            // session slot if that slot is empty, then clear hub_session
            // only if the move succeeded.
            if let Ok(None) = meta.session_token().await {
                if meta.save_session(&hub_token).await.is_ok() {
                    let _ = meta.delete_hub_session().await;
                }
            } else {
                // CLI slot already has a token; just clear the bad hub_session
                let _ = meta.delete_hub_session().await;
            }
            // Fall through to check CLI token below
        }

        // No valid Hub token — check for a CLI session token
        match meta.session_token().await {
            Ok(Some(token)) => SyncAuth::HubViaCli { token },
            _ => SyncAuth::NotLoggedIn {
                reason: "Not logged in. Run 'console history login' or 'console history register' \
                         to authenticate."
                    .into(),
            },
        }
    }

    /// Returns the appropriate auth token for sync operations.
    ///
    /// Delegates to [`Self::resolve_sync_auth`] and converts the result to an
    /// `AuthToken`. Callers that need to distinguish between auth states
    /// (e.g. to show different UI) should call `resolve_sync_auth` directly.
    #[cfg(feature = "sync")]
    pub async fn sync_auth_token(&self) -> Result<crate::api_client::AuthToken> {
        self.resolve_sync_auth().await.into_auth_token()
    }

    #[cfg(feature = "check-update")]
    async fn needs_update_check(&self) -> Result<bool> {
        let last_check = Settings::last_version_check().await?;
        let diff = OffsetDateTime::now_utc() - last_check;

        // Check a max of once per hour
        Ok(diff.whole_hours() >= 1)
    }

    #[cfg(feature = "check-update")]
    async fn latest_version(&self) -> Result<Version> {
        // Default to the current version, and if that doesn't parse, a version so high it's unlikely to ever
        // suggest upgrading.
        let current =
            Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or(Version::new(100000, 0, 0));

        if !self.needs_update_check().await? {
            let meta = Self::meta_store().await?;
            let version = match meta.latest_version().await? {
                Some(v) => Version::parse(&v).unwrap_or(current),
                None => current,
            };

            return Ok(version);
        }

        #[cfg(feature = "sync")]
        let latest = crate::api_client::latest_version().await.unwrap_or(current);

        #[cfg(not(feature = "sync"))]
        let latest = current;

        let meta = Self::meta_store().await?;
        Settings::save_version_check_time().await?;
        meta.save_latest_version(&latest.to_string()).await?;

        Ok(latest)
    }

    // Return Some(latest version) if an update is needed. Otherwise, none.
    #[cfg(feature = "check-update")]
    pub async fn needs_update(&self) -> Option<Version> {
        if !self.update_check {
            return None;
        }

        let current =
            Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or(Version::new(100000, 0, 0));

        let latest = self.latest_version().await;

        if latest.is_err() {
            return None;
        }

        let latest = latest.unwrap();

        if latest > current {
            return Some(latest);
        }

        None
    }

    pub fn default_filter_mode(&self, git_root: bool) -> FilterMode {
        self.filter_mode
            .filter(|x| self.search.filters.contains(x))
            .or_else(|| {
                self.search
                    .filters
                    .iter()
                    .find(|x| match (x, git_root, self.workspaces) {
                        (FilterMode::Workspace, true, true) => true,
                        (FilterMode::Workspace, _, _) => false,
                        (_, _, _) => true,
                    })
                    .copied()
            })
            .unwrap_or(FilterMode::Global)
    }

    #[cfg(not(feature = "check-update"))]
    pub async fn needs_update(&self) -> Option<Version> {
        None
    }

    pub fn builder() -> Result<ConfigBuilder<DefaultState>> {
        Self::builder_with_data_dir(&shell_history_common::utils::data_dir())
    }

    fn builder_with_data_dir(data_dir: &std::path::Path) -> Result<ConfigBuilder<DefaultState>> {
        let db_path = data_dir.join("history.db");
        let record_store_path = data_dir.join("records.db");
        let kv_path = data_dir.join("kv.db");
        let scripts_path = data_dir.join("scripts.db");
        let ai_sessions_path = data_dir.join("ai_sessions.db");
        let logs_dir = shell_history_common::utils::logs_dir();

        let key_path = data_dir.join("key");
        let meta_path = data_dir.join("meta.db");

        Ok(Config::builder()
            .set_default("history_format", "{time}\t{command}\t{duration}")?
            .set_default("db_path", db_path.to_str())?
            .set_default("record_store_path", record_store_path.to_str())?
            .set_default("key_path", key_path.to_str())?
            .set_default("dialect", "us")?
            .set_default("timezone", "local")?
            .set_default("auto_sync", true)?
            .set_default("update_check", cfg!(feature = "check-update"))?
            .set_default("sync_address", "http://127.0.0.1:8888")?
            .set_default("sync_frequency", "5m")?
            .set_default("search_mode", "fuzzy")?
            .set_default("filter_mode", None::<String>)?
            .set_default("workspaces", false)?
            .set_default("ctrl_n_shortcuts", false)?
            .set_default("secrets_filter", true)?
            .set_default("strip_trailing_whitespace", true)?
            .set_default("network_connect_timeout", 5)?
            .set_default("network_timeout", 30)?
            .set_default("local_timeout", 2.0)?
            // enter_accept defaults to false here, but true in the default config file. The dissonance is
            // intentional!
            // Existing users will get the default "False", so we don't mess with any potential
            // muscle memory.
            // New users will get the new default, that is more similar to what they are used to.
            .set_default("enter_accept", false)?
            .set_default("sync.records", true)?
            .set_default("smart_sort", false)?
            .set_default("command_chaining", false)?
            .set_default("store_failed", true)?
            .set_default("logs.enabled", true)?
            .set_default("logs.dir", logs_dir.to_str())?
            .set_default("logs.level", "info")?
            .set_default("logs.search.file", "search.log")?
            .set_default("logs.daemon.file", "daemon.log")?
            .set_default("logs.ai.file", "ai.log")?
            .set_default("kv.db_path", kv_path.to_str())?
            .set_default("scripts.db_path", scripts_path.to_str())?
            .set_default("search.recency_score_multiplier", 1.0)?
            .set_default("search.frequency_score_multiplier", 1.0)?
            .set_default("search.frecency_score_multiplier", 1.0)?
            .set_default("meta.db_path", meta_path.to_str())?
            .set_default("ai.db_path", ai_sessions_path.to_str())?
            .set_default("ai.session_continue_minutes", 60)?
            .set_default("ai.send_cwd", false)?
            .set_default("ai.opening.send_cwd", false)?
            .set_default("ai.opening.send_last_command", false)?
            .set_default(
                "search.filters",
                vec![
                    "global",
                    "host",
                    "session",
                    "workspace",
                    "directory",
                    "session-preload",
                ],
            )?
            .set_default("tmux.enabled", false)?
            .set_default("tmux.width", "80%")?
            .set_default("tmux.height", "60%")?
            .set_default("no_mouse", false)?
            .add_source(
                Environment::with_prefix("shell-history")
                    .prefix_separator("_")
                    .separator("__"),
            ))
    }

    pub fn get_config_path() -> Result<PathBuf> {
        let config_dir = shell_history_common::utils::config_dir();

        create_dir_all(&config_dir)
            .wrap_err_with(|| format!("could not create dir {config_dir:?}"))?;

        let mut config_file = if let Ok(p) = std::env::var("CONSOLE_SHELL_HISTORY_CONFIG_DIR") {
            PathBuf::from(p)
        } else {
            let mut config_file = PathBuf::new();
            config_file.push(config_dir);
            config_file
        };

        config_file.push("config.toml");

        Ok(config_file)
    }

    /// Build a merged `Config` from defaults, config file, and environment.
    ///
    /// This resolves `data_dir`, initializes the data directory on disk,
    /// and layers defaults → config file → env overrides. Both `new()` and
    /// `get_config_value()` use this so the resolution logic lives in one place.
    fn build_config() -> Result<Config> {
        let config_file = Self::get_config_path()?;

        // extract data_dir first so we can use it as the base for other path defaults
        let effective_data_dir = if config_file.exists() {
            #[derive(Deserialize, Default)]
            struct DataDirOnly {
                data_dir: Option<String>,
            }

            let config_file_str = config_file
                .to_str()
                .ok_or_else(|| eyre!("config file path is not valid UTF-8"))?;

            let partial_config = Config::builder()
                .add_source(ConfigFile::new(config_file_str, FileFormat::Toml))
                .add_source(
                    Environment::with_prefix("shell-history")
                        .prefix_separator("_")
                        .separator("__"),
                )
                .build()
                .ok();

            let custom_data_dir = partial_config
                .and_then(|c| c.try_deserialize::<DataDirOnly>().ok())
                .and_then(|d| d.data_dir);

            match custom_data_dir {
                Some(dir) => {
                    let expanded = shellexpand::full(&dir)
                        .map_err(|e| eyre!("failed to expand data_dir path: {}", e))?;
                    PathBuf::from(expanded.as_ref())
                }
                None => shell_history_common::utils::data_dir(),
            }
        } else {
            shell_history_common::utils::data_dir()
        };

        DATA_DIR.set(effective_data_dir.clone()).ok();

        create_dir_all(&effective_data_dir)
            .wrap_err_with(|| format!("could not create dir {effective_data_dir:?}"))?;

        let mut config_builder = Self::builder_with_data_dir(&effective_data_dir)?;

        config_builder = if config_file.exists() {
            let config_file_str = config_file
                .to_str()
                .ok_or_else(|| eyre!("config file path is not valid UTF-8"))?;
            config_builder.add_source(ConfigFile::new(config_file_str, FileFormat::Toml))
        } else {
            let mut file = File::create(config_file).wrap_err("could not create config file")?;
            file.write_all(EXAMPLE_CONFIG.as_bytes())
                .wrap_err("could not write default config file")?;

            config_builder
        };

        // all paths should be expanded
        let built = config_builder.build_cloned()?;
        config_builder = [
            "db_path",
            "record_store_path",
            "key_path",
            "logs.dir",
            "logs.search.file",
            "logs.daemon.file",
        ]
        .iter()
        .map(|key| (key, built.get_string(key).unwrap_or_default()))
        .filter_map(|(key, value)| match Self::expand_path(value) {
            Ok(expanded) => Some((key, expanded)),
            Err(e) => {
                tracing::warn!("failed to expand path for {key}: {e}");
                None
            }
        })
        .fold(config_builder, |builder, (key, value)| {
            builder
                .set_override(key, value)
                .unwrap_or_else(|_| panic!("failed to set absolute path override for {key}"))
        });

        config_builder.build().map_err(Into::into)
    }

    /// Look up a single config value by dotted key (e.g. `"daemon.sync_frequency"`).
    ///
    /// Returns the effective value after merging defaults, config file, and
    /// environment — without the side-effects of full `Settings` construction
    /// (meta store init, path expansion, etc.).
    pub fn get_config_value(key: &str) -> Result<String> {
        let config = Self::build_config()?;
        let value: config::Value = config
            .get(key)
            .map_err(|e| eyre!("failed to get config value '{}': {}", key, e))?;
        Ok(Self::format_resolved_value(&value, key))
    }

    fn format_resolved_value(value: &config::Value, prefix: &str) -> String {
        use config::ValueKind;

        match &value.kind {
            ValueKind::Nil => String::new(),
            ValueKind::Boolean(b) => b.to_string(),
            ValueKind::I64(i) => i.to_string(),
            ValueKind::I128(i) => i.to_string(),
            ValueKind::U64(u) => u.to_string(),
            ValueKind::U128(u) => u.to_string(),
            ValueKind::Float(f) => f.to_string(),
            ValueKind::String(s) => s.clone(),
            ValueKind::Array(arr) => {
                let items: Vec<String> = arr
                    .iter()
                    .map(|v| Self::format_resolved_value(v, ""))
                    .collect();
                format!("[{}]", items.join(", "))
            }
            ValueKind::Table(map) => {
                let mut lines = Vec::new();
                let mut keys: Vec<_> = map.keys().collect();
                keys.sort();

                for k in keys {
                    let v = &map[k];
                    let full_key = if prefix.is_empty() {
                        k.clone()
                    } else {
                        format!("{}.{}", prefix, k)
                    };

                    match &v.kind {
                        ValueKind::Table(_) => {
                            lines.push(Self::format_resolved_value(v, &full_key));
                        }
                        _ => {
                            lines.push(format!(
                                "{} = {}",
                                full_key,
                                Self::format_resolved_value(v, "")
                            ));
                        }
                    }
                }

                lines.join("\n")
            }
        }
    }

    pub fn new() -> Result<Self> {
        let config = Self::build_config()?;
        let settings: Settings = config
            .try_deserialize()
            .map_err(|e| eyre!("failed to deserialize: {}", e))?;


        // Register meta store config for lazy initialization on first access
        META_CONFIG
            .set((settings.meta.db_path.clone(), settings.local_timeout))
            .ok();

        Ok(settings)
    }

    fn expand_path(path: String) -> Result<String> {
        shellexpand::full(&path)
            .map(|p| p.to_string())
            .map_err(|e| eyre!("failed to expand path: {}", e))
    }

    pub fn example_config() -> &'static str {
        EXAMPLE_CONFIG
    }

    pub fn paths_ok(&self) -> bool {
        let paths = [
            &self.db_path,
            &self.record_store_path,
            &self.key_path,
            &self.meta.db_path,
        ];
        paths.iter().all(|p| !utils::broken_symlink(p))
    }
}

impl Default for Settings {
    fn default() -> Self {
        // if this panics something is very wrong, as the default config
        // does not build or deserialize into the settings struct
        Self::builder()
            .expect("Could not build default")
            .build()
            .expect("Could not build config")
            .try_deserialize()
            .expect("Could not deserialize config")
    }
}

/// Initialize the meta store configuration for testing.
///
/// This should only be used in tests. It allows tests to bypass the normal
/// Settings::new() flow while still being able to use Settings::host_id()
/// and other meta store dependent functions.
///
/// # Safety
/// This function is not thread-safe with concurrent calls to Settings::new()
/// or other meta store initialization. Only call from tests.
#[doc(hidden)]
pub fn init_meta_config_for_testing(meta_db_path: impl Into<String>, local_timeout: f64) {
    META_CONFIG.set((meta_db_path.into(), local_timeout)).ok();
}

#[cfg(test)]
pub(crate) fn test_local_timeout() -> f64 {
    std::env::var("CONSOLE_SHELL_HISTORY_TEST_LOCAL_TIMEOUT")
        .ok()
        .and_then(|x| x.parse().ok())
        // this hardcoded value should be replaced by a simple way to get the
        // default local_timeout of Settings if possible
        .unwrap_or(2.0)
}

