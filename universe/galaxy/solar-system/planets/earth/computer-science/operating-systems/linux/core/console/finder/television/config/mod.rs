use crate::{
    cable::CABLE_DIR_NAME, channels::prototypes::DEFAULT_PROTOTYPE_NAME,
    history::DEFAULT_HISTORY_SIZE, utils::shell::Shell,
};
use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use shell_integration::ShellIntegrationConfig;
use std::{
    env,
    hash::Hash,
    path::{Path, PathBuf},
};
use tracing::{debug, warn};

pub use keybindings::{Keybindings, merge_keybindings};
pub use themes::Theme;
pub use ui::UiConfig;

pub(crate) mod themes;

pub mod keybindings;
pub mod layers;
pub mod shell_integration;
pub mod ui;

const DEFAULT_CONFIG: &str = include_str!("../../.config/config.toml");

#[allow(dead_code, clippy::module_name_repetitions)]
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    #[serde(default = "get_data_dir")]
    pub data_dir: PathBuf,
    #[serde(default = "default_cable_dir")]
    pub cable_dir: PathBuf,
    #[serde(default = "default_tick_rate")]
    pub tick_rate: u64,
    /// The default channel to use when no channel is specified
    #[serde(default = "default_channel")]
    pub default_channel: String,
    /// Maximum number of entries to keep in the global history
    #[serde(default = "default_history_size")]
    pub history_size: usize,
    /// Whether to use global history (all channels) or channel-specific history (default)
    #[serde(default = "default_global_history")]
    pub global_history: bool,
    /// Maximum number of frecency entries to keep per channel
    #[serde(default = "default_frecency_max_entries")]
    pub frecency_max_entries: usize,
    /// Global shell used for executing commands.
    /// If not specified, the shell is detected from the environment.
    /// Channel-specific shell settings override this.
    #[serde(default)]
    pub shell: Option<Shell>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            data_dir: get_data_dir(),
            cable_dir: default_cable_dir(),
            tick_rate: default_tick_rate(),
            default_channel: default_channel(),
            history_size: default_history_size(),
            global_history: default_global_history(),
            frecency_max_entries: default_frecency_max_entries(),
            shell: None,
        }
    }
}

fn default_channel() -> String {
    DEFAULT_PROTOTYPE_NAME.to_string()
}

fn default_history_size() -> usize {
    DEFAULT_HISTORY_SIZE
}

fn default_global_history() -> bool {
    false
}

const DEFAULT_FRECENCY_MAX_ENTRIES: usize = 1000;

fn default_frecency_max_entries() -> usize {
    DEFAULT_FRECENCY_MAX_ENTRIES
}

impl Hash for AppConfig {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.data_dir.hash(state);
        self.tick_rate.hash(state);
        self.history_size.hash(state);
        self.global_history.hash(state);
        self.frecency_max_entries.hash(state);
        self.shell.hash(state);
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Config {
    /// General application configuration
    #[allow(clippy::struct_field_names)]
    #[serde(default, flatten)]
    pub application: AppConfig,
    /// Keybindings configuration
    #[serde(default)]
    pub keybindings: Keybindings,
    /// UI configuration
    #[serde(default)]
    pub ui: UiConfig,
    /// Shell integration configuration
    #[serde(default)]
    pub shell_integration: ShellIntegrationConfig,
}

const PROJECT_NAME: &str = "finder";
pub const CONFIG_FILE_NAME: &str = "config.toml";

pub struct ConfigEnv {
    _data_dir: PathBuf,
    config_dir: PathBuf,
}

impl ConfigEnv {
    pub fn init() -> Result<Self> {
        let data_dir = get_data_dir();
        let config_dir = get_config_dir();
        let cable_dir = config_dir.join(CABLE_DIR_NAME);

        std::fs::create_dir_all(&config_dir)
            .context("Failed creating configuration directory")?;
        std::fs::create_dir_all(&cable_dir)
            .context("Failed creating cable directory")?;
        std::fs::create_dir_all(&data_dir)
            .context("Failed creating data directory")?;

        Ok(Self {
            _data_dir: data_dir,
            config_dir,
        })
    }
}

pub fn default_config_from_file() -> Result<Config> {
    let default_config: Config = toml::from_str(DEFAULT_CONFIG)
        .context("Error parsing default config")?;
    Ok(default_config)
}

const USER_CONFIG_ERROR_MSG: &str = "
╔══════════════════════════════════════════════════════════════════════════════╗
║                                                                              ║
║  If this follows a recent update, it is likely due to a breaking change in   ║
║  the configuration format.                                                   ║
║                                                                              ║
║  Check https://github.com/alexpasmantier/television/releases/latest for the  ║
║  latest release notes.                                                       ║
║                                                                              ║
╚══════════════════════════════════════════════════════════════════════════════╝";

impl Config {
    #[allow(clippy::missing_panics_doc, clippy::missing_errors_doc)]
    pub fn new(
        config_env: &ConfigEnv,
        custom_config_file: Option<&Path>,
    ) -> Result<Self> {
        // Load the default_config values as base defaults
        let default_config: Config = default_config_from_file()?;

        // if a config file exists, load it and merge it with the default configuration
        if config_env.config_dir.join(CONFIG_FILE_NAME).is_file()
            || custom_config_file.is_some()
        {
            let config_file = if let Some(path) = custom_config_file {
                debug!("Using custom configuration file at: {:?}", path);
                path.to_path_buf()
            } else {
                let config_file = config_env.config_dir.join(CONFIG_FILE_NAME);
                debug!(
                    "Using default configuration file at: {:?}",
                    config_file
                );
                config_file
            };

            let user_cfg: Config = Self::load_user_config(&config_file)?;
            debug!("Loaded user configuration: {:?}", user_cfg);

            // merge the user configuration with the default configuration
            let final_cfg =
                Self::merge_with_default(&default_config, user_cfg);

            Ok(final_cfg)
        } else {
            // otherwise, create the default configuration file
            warn!(
                "No config file found at {:?}, creating default configuration file at that location.",
                config_env.config_dir
            );
            // create the default configuration file in the user's config directory
            std::fs::write(
                config_env.config_dir.join(CONFIG_FILE_NAME),
                DEFAULT_CONFIG,
            )?;
            Ok(default_config)
        }
    }

    fn load_user_config(config_file: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(config_file)?;
        let user_cfg: Config = toml::from_str(&contents).context(format!(
            "Error parsing configuration file: {}\n{}",
            config_file.display(),
            USER_CONFIG_ERROR_MSG,
        ))?;
        Ok(user_cfg)
    }

    fn merge_with_default(default: &Config, mut new: Config) -> Config {
        // use default fallback channel as a fallback if user hasn't specified one
        if new.shell_integration.fallback_channel.is_empty() {
            new.shell_integration
                .fallback_channel
                .clone_from(&default.shell_integration.fallback_channel);
        }

        // merge shell integration commands with default commands
        if new.shell_integration.commands.is_empty() {
            new.shell_integration
                .commands
                .clone_from(&default.shell_integration.commands);
        }

        // merge shell integration keybindings with default keybindings
        let mut merged_keybindings =
            default.shell_integration.keybindings.clone();
        merged_keybindings.extend(new.shell_integration.keybindings.clone());
        new.shell_integration.keybindings = merged_keybindings;

        // merge keybindings with default keybindings
        let keybindings =
            merge_keybindings(default.keybindings.clone(), &new.keybindings);
        new.keybindings = keybindings;

        Config {
            application: new.application,
            keybindings: new.keybindings,
            ui: new.ui,
            shell_integration: new.shell_integration,
        }
    }

    pub fn merge_channel_keybindings(&mut self, other: &Keybindings) {
        self.keybindings = merge_keybindings(self.keybindings.clone(), other);
    }
}

pub fn get_data_dir() -> PathBuf {
    // if `FINDER_DATA` is set, use that as the data directory
    let data_folder =
        env::var_os(format!("{}_DATA", PROJECT_NAME.to_uppercase()))
            .map(PathBuf::from)
            .or_else(|| {
                // otherwise, use the XDG data directory
                env::var_os("XDG_DATA_HOME")
                    .map(PathBuf::from)
                    .map(|p| p.join(PROJECT_NAME))
                    .filter(|p| p.is_absolute())
            });

    if let Some(s) = data_folder {
        s
    } else if let Some(proj_dirs) = project_directory() {
        proj_dirs.data_local_dir().to_path_buf()
    } else {
        PathBuf::from("../../../../..").join(".data")
    }
}

pub fn get_config_dir() -> PathBuf {
    // if `FINDER_CONFIG` is set, use that as the finder config directory
    let config_dir =
        env::var_os(format!("{}_CONFIG", PROJECT_NAME.to_uppercase()))
            .map(PathBuf::from)
            .or_else(|| {
                // otherwise, use the XDG config directory + finder
                env::var_os("XDG_CONFIG_HOME")
                    .map(PathBuf::from)
                    .map(|p| p.join(PROJECT_NAME))
                    .filter(|p| p.is_absolute())
            });
    if let Some(s) = config_dir {
        s
    } else if cfg!(unix) {
        // default to ~/.config/finder for unix systems
        if let Some(base_dirs) = directories::BaseDirs::new() {
            base_dirs.home_dir().join(".config").join("finder")
        } else {
            PathBuf::from("../../../../..").join(".config")
        }
    } else if let Some(proj_dirs) = project_directory() {
        proj_dirs.config_local_dir().to_path_buf()
    } else {
        PathBuf::from("../../../../..").join("../../../../../.config")
    }
}

fn default_cable_dir() -> PathBuf {
    get_config_dir().join(CABLE_DIR_NAME)
}

fn project_directory() -> Option<ProjectDirs> {
    ProjectDirs::from("com", "", env!("CARGO_PKG_NAME"))
}

pub fn default_tick_rate() -> u64 {
    50
}

pub use ui::{DEFAULT_PREVIEW_SIZE, DEFAULT_UI_SCALE};

