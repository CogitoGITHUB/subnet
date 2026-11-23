use std::path::{Path, PathBuf};

/// CLI flags that affect *where* configuration files are found.
///
/// Only path-resolution flags belong here. Flags that control *whether* config
/// files are loaded (e.g. `--no-config-file`) stay in `NushellCliArgs`.
///
/// Relative paths are resolved against the provided cwd in
/// [`CliOverrides::from_path_strings`] — that is the single place absolute-ization
/// of CLI path overrides happens.
#[derive(Debug, Clone, Default)]
pub struct CliOverrides {
    /// `--config-home <path>` — override the entire shell config directory
    pub config_home: Option<PathBuf>,

    /// `--config <file>` — override `config.nu` path
    pub config_file: Option<PathBuf>,

    /// `--env-config <file>` — override `env.nu` path
    pub env_file: Option<PathBuf>,

    /// `--plugin-config <file>` — override plugin registry file path
    #[cfg(feature = "plugin")]
    pub plugin_file: Option<PathBuf>,
}

impl CliOverrides {
    /// Build overrides from optional CLI path strings, resolving any relative
    /// path against `cwd`.
    ///
    /// This is the **only** place CLI config path absolute-ization should live.
    ///
    /// Resolution is **logical** (not realpath): tilde is expanded via the
    /// user's home directory, relative segments are joined to `cwd`, and `.` /
    /// `..` are stripped lexically. Symlinks in the path are **not** followed.
    pub fn from_path_strings(
        config_home: Option<&str>,
        config_file: Option<&str>,
        env_file: Option<&str>,
        #[cfg(feature = "plugin")] plugin_file: Option<&str>,
        cwd: &Path,
    ) -> Self {
        Self {
            config_home: config_home.map(|s| absolutize_cli_path(s, cwd)),
            config_file: config_file.map(|s| absolutize_cli_path(s, cwd)),
            env_file: env_file.map(|s| absolutize_cli_path(s, cwd)),
            #[cfg(feature = "plugin")]
            plugin_file: plugin_file.map(|s| absolutize_cli_path(s, cwd)),
        }
    }
}

/// Resolve a CLI path to a logical absolute path against `cwd`.
///
/// Uses [`console_path::expand_path_with`]: expands a leading `~`, joins relative
/// paths to `cwd`, and lexically normalizes `.` / `..`. Does **not**
/// canonicalize or follow symlinks — so `~/.config/shell` becomes
/// `$HOME/.config/shell` even when `$HOME` itself is a symlink chain.
fn absolutize_cli_path(path: &str, cwd: &Path) -> PathBuf {
    console_path::expand_path_with(path, cwd, true)
}

