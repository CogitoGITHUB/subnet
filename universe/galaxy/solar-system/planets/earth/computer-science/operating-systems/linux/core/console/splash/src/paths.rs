#![allow(dead_code)]

use std::path::PathBuf;

/// All splashboard state lives under a single dotted directory in the user's home:
/// `$HOME/.splash/`. Keeps settings, dashboards, trust store, cache, and ReadStore data
/// in one place — same pattern as per-directory `.splash/` configs. Overridable via the
/// `SPLASH_HOME` env var for tests, CI, or power users who want to relocate.
///
/// Rationale over the platform-specific `dirs` crate: for a CLI tool, `~/Library/...` on
/// macOS is awkward (Unix-flavored OS, dotfile conventions are the norm for terminal
/// tools). `$HOME/.splash/` works the same on Linux, macOS, and Windows, and mirrors
/// the per-directory convention users already know.
pub fn splashboard_home() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("SPLASH_HOME") {
        return Some(PathBuf::from(p));
    }
    dirs::home_dir().map(|h| h.join(".splash"))
}

/// User preferences (`[general]`). Always lives in HOME — per-dir overrides are
/// out of scope for 0.x.
pub fn settings_path() -> Option<PathBuf> {
    splashboard_home().map(|d| d.join("settings.toml"))
}

/// Sensitive env vars (`GH_TOKEN`, `TODOIST_TOKEN`, etc.). Kept separate from `settings.toml`
/// so dotfiles-in-git users can git-ignore this one file without losing the rest of their
/// splashboard config.
pub fn secrets_path() -> Option<PathBuf> {
    splashboard_home().map(|d| d.join("secrets.toml"))
}

/// Default dashboard when no per-dir dashboard applies and the CWD isn't a project root.
pub fn home_dashboard_path() -> Option<PathBuf> {
    splashboard_home().map(|d| d.join("home.dashboard.toml"))
}

/// Default project dashboard used when the CWD is a git repo root without a per-dir dashboard.
pub fn project_dashboard_path() -> Option<PathBuf> {
    splashboard_home().map(|d| d.join("project.dashboard.toml"))
}

pub fn trust_store_path() -> Option<PathBuf> {
    splashboard_home().map(|d| d.join("trust.toml"))
}

pub fn cache_dir() -> Option<PathBuf> {
    splashboard_home().map(|d| d.join("cache"))
}

pub fn read_store_dir() -> Option<PathBuf> {
    splashboard_home().map(|d| d.join("store"))
}

/// Directory for the `tracing` file appender. Created lazily by the logging init path so
/// invocations that never log don't touch the disk.
pub fn logs_dir() -> Option<PathBuf> {
    splashboard_home().map(|d| d.join("logs"))
}

/// Shared mutex guarding any test that mutates `SPLASH_HOME`. Without this, parallel
/// tests in different modules can see each other's temporary values through the process-wide
/// env var. Exposed at crate level so other test modules (`fetcher::read_store`, etc.) can
/// lock the same mutex.
pub static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

