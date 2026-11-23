use crate::cli::parse_source_entry_delimiter;
use crate::config::ui::InputBarConfig;
use crate::utils::shell::Shell;
use crate::utils::strings::SPACE;
use crate::{
    config::{Keybindings, ui},
    event::Key,
    screen::layout::Orientation,
};
use anyhow::Result;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use serde_with::{OneOrMany, serde_as};
use std::fmt::{self, Display, Formatter};
use std::hash::{Hash, Hasher};
use string_pipeline::MultiTemplate;
use which::which;

#[derive(Debug, Clone)]
pub enum Template {
    StringPipeline(MultiTemplate),
    Raw(String),
}

impl Template {
    pub fn raw(&self) -> &str {
        match self {
            Template::StringPipeline(template) => template.template_string(),
            Template::Raw(raw) => raw,
        }
    }

    pub fn parse(template: &str) -> Result<Self, String> {
        match MultiTemplate::parse(template) {
            Ok(multi_template) => Ok(Template::StringPipeline(multi_template)),
            Err(_) => Ok(Template::Raw(template.to_string())),
        }
    }

    pub fn format(&self, input: &str) -> Result<String> {
        match self {
            Template::StringPipeline(template) => {
                template.format(input).map_err(|e| {
                    anyhow::anyhow!(
                        "Failed to format template '{}' with '{}': {}",
                        self.raw(),
                        input,
                        e
                    )
                })
            }
            Template::Raw(raw) => Ok(raw.replace("{}", input)),
        }
    }
}

impl Display for Template {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "{}", self.raw())
    }
}

impl PartialEq for Template {
    fn eq(&self, other: &Self) -> bool {
        self.raw() == other.raw()
            && matches!(
                (self, other),
                (Template::StringPipeline(_), Template::StringPipeline(_))
                    | (Template::Raw(_), Template::Raw(_))
            )
    }
}

impl Eq for Template {}

impl Hash for Template {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.raw().hash(state);
    }
}

impl Serialize for Template {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.raw())
    }
}

impl<'de> Deserialize<'de> for Template {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Template::parse(&raw).map_err(serde::de::Error::custom)
    }
}

/// A single source command, optionally with a display name.
///
/// Accepts two TOML forms in arrays/single values:
/// - Bare string: `"fd -t f"` — no display name.
/// - Inline table: `{ name = "Default", run = "fd -t f" }` — named.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq)]
#[serde(untagged)]
pub enum SourceCommand {
    Bare(Template),
    Named { name: String, run: Template },
}

impl SourceCommand {
    pub fn template(&self) -> &Template {
        match self {
            Self::Bare(t) | Self::Named { run: t, .. } => t,
        }
    }

    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Named { name, .. } if !name.is_empty() => {
                Some(name.as_str())
            }
            _ => None,
        }
    }
}

impl From<Template> for SourceCommand {
    fn from(template: Template) -> Self {
        Self::Bare(template)
    }
}

#[serde_as]
#[derive(
    Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq, Default,
)]
pub struct CommandSpec {
    #[serde(rename = "command")]
    #[serde_as(as = "OneOrMany<_>")]
    pub inner: Vec<SourceCommand>,
    #[serde(default)]
    pub interactive: bool,
    #[serde(default)]
    pub env: FxHashMap<String, String>,
    /// Optionally override the shell used to execute this command.
    /// If not specified, the shell is detected from the environment.
    #[serde(default)]
    pub shell: Option<Shell>,
}

impl Display for CommandSpec {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(
            f,
            "[{}]",
            self.inner
                .iter()
                .map(|c| c.template().raw())
                .collect::<Vec<_>>()
                .join(";")
        )
    }
}

impl From<Template> for CommandSpec {
    fn from(template: Template) -> Self {
        Self {
            inner: vec![template.into()],
            ..Default::default()
        }
    }
}

impl CommandSpec {
    /// This wraps back to the first command in a circular manner.
    ///
    /// # Panics
    /// If the command spec does not contain any commands.
    pub fn get_nth(&self, index: usize) -> &SourceCommand {
        &self.inner[index % self.inner.len()]
    }
}

/// Execution mode for external actions
#[derive(
    Debug, Clone, Default, serde::Deserialize, serde::Serialize, PartialEq,
)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionMode {
    /// Fork the command as a child process (finder stays open)
    #[default]
    Fork,
    /// Replace the current process with the command (finder exits, command takes over)
    Execute,
}

fn default_separator() -> String {
    SPACE.to_string()
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct ActionSpec {
    #[serde(default)]
    pub description: Option<String>,
    #[serde(flatten)]
    pub command: CommandSpec,
    /// How to execute the command
    #[serde(default)]
    pub mode: ExecutionMode,
    /// Separator to use when formatting multiple entries into the command
    ///
    /// Example: `rm file1+SEPARATOR+file2+SEPARATOR+file3`
    #[serde(default = "default_separator")]
    pub separator: String,
    // TODO: add `requirements` (see `prototypes::BinaryRequirement`)
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ChannelKeyBindings {
    /// Optional channel specific shortcut that, when pressed, switches directly to this channel.
    #[serde(default)]
    pub shortcut: Option<Key>,
    /// Regular action -> binding mappings living at channel level.
    #[serde(flatten)]
    #[serde(default)]
    pub bindings: Keybindings,
}

impl ChannelKeyBindings {
    pub fn channel_shortcut(&self) -> Option<&Key> {
        self.shortcut.as_ref()
    }
}

pub fn remove_enter_keybinding(prototype: &mut ChannelPrototype) {
    if let Some(keybindings) = &mut prototype.keybindings {
        keybindings.bindings.remove(&Key::Enter);
    }
}

#[derive(Default, Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct HistoryConfig {
    /// Whether to use global history for this channel (overrides global setting)
    #[serde(default)]
    pub global_mode: Option<bool>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ChannelPrototype {
    pub metadata: Metadata,
    pub source: SourceSpec,
    #[serde(default)]
    pub preview: Option<PreviewSpec>,
    #[serde(default)]
    pub ui: Option<UiSpec>,
    #[serde(default)]
    pub keybindings: Option<ChannelKeyBindings>,
    /// Watch interval in seconds for automatic reloading (0 = disabled)
    #[serde(default)]
    pub watch: f64,
    #[serde(default)]
    pub history: HistoryConfig,
    #[serde(default)]
    pub actions: FxHashMap<String, ActionSpec>,
}

impl ChannelPrototype {
    pub fn new(name: &str, command: &str) -> Self {
        Self {
            metadata: Metadata {
                name: name.to_string(),
                description: None,
                requirements: vec![],
            },
            source: SourceSpec {
                command: CommandSpec {
                    inner: vec![
                        Template::parse(command)
                            .expect("Failed to parse command")
                            .into(),
                    ],
                    interactive: false,
                    env: FxHashMap::default(),
                    shell: None,
                },
                entry_delimiter: None,
                ansi: false,
                display: None,
                output: None,
                no_sort: false,
                frecency: true,
            },
            preview: None,
            ui: None,
            keybindings: None,
            watch: 0.0,
            history: HistoryConfig::default(),
            actions: FxHashMap::default(),
        }
    }

    pub fn stdin() -> Self {
        Self {
            metadata: Metadata {
                name: "stdin".to_string(),
                description: Some(
                    "A channel that reads from stdin".to_string(),
                ),
                requirements: vec![],
            },
            source: SourceSpec {
                // The source command is unused for stdin channels — data is
                // read directly from process stdin in Rust. We keep a no-op
                // placeholder because the field is required.
                command: CommandSpec {
                    inner: vec![Template::parse("true").unwrap().into()],
                    ..Default::default()
                },
                ..Default::default()
            },
            preview: None,
            ui: None,
            keybindings: None,
            watch: 0.0,
            history: HistoryConfig::default(),
            actions: FxHashMap::default(),
        }
    }

    pub fn with_preview(mut self, preview: Option<PreviewSpec>) -> Self {
        self.preview = preview;
        self
    }
}

impl Display for ChannelPrototype {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "{}", self.metadata.name)
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct Metadata {
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub requirements: Vec<BinaryRequirement>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(transparent)]
pub struct BinaryRequirement {
    pub bin_name: String,
    #[serde(skip)]
    met: bool,
}

impl BinaryRequirement {
    pub fn new(bin_name: &str) -> Self {
        Self {
            bin_name: bin_name.to_string(),
            met: false,
        }
    }

    /// Check if the required binary is available in the system's PATH.
    ///
    /// This method updates the requirement's state in place to reflect whether the binary was
    /// found.
    pub fn init(&mut self) {
        self.met = which(&self.bin_name).is_ok();
    }

    /// Whether the requirement is available in the system's PATH.
    ///
    /// This should be called after `init()`.
    pub fn is_met(&self) -> bool {
        self.met
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct SourceSpec {
    #[serde(flatten)]
    pub command: CommandSpec,
    #[serde(deserialize_with = "deserialize_entry_delimiter", default)]
    pub entry_delimiter: Option<char>,
    #[serde(default)]
    pub ansi: bool,
    #[serde(default)]
    pub display: Option<Template>,
    #[serde(default)]
    pub output: Option<Template>,
    /// Whether to disable sorting and preserve source order.
    /// Defaults to false (sort by match score).
    /// Set to true for pre-sorted data like shell history or git log.
    /// Matches the `--no-sort` CLI flag.
    #[serde(default)]
    pub no_sort: bool,
    /// Whether to use frecency-based ranking for this channel.
    /// Defaults to true. Set to false for channels where frecency doesn't make sense
    /// (e.g., channels with dynamic/random results, or where order is meaningful).
    #[serde(default = "default_frecency")]
    pub frecency: bool,
}

const fn default_frecency() -> bool {
    true
}

/// Just a helper function to adapt cli parsing to serde deserialization.
fn deserialize_entry_delimiter<'de, D>(
    deserializer: D,
) -> Result<Option<char>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    if let Ok(Some(delimiter)) = Option::<String>::deserialize(deserializer) {
        parse_source_entry_delimiter(&delimiter)
            .map(Some)
            .map_err(serde::de::Error::custom)
    } else {
        Ok(None)
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct PreviewSpec {
    #[serde(flatten)]
    pub command: CommandSpec,
    #[serde(default)]
    pub offset: Option<Template>,
    #[serde(default = "cached_default")]
    pub cached: bool,
}

/// Preview caching is enabled by default.
///
/// This makes sense since most channels will benefit from caching by reducing
/// redundant preview command executions when navigating back and forth between entries.
///
/// Users may opt-out for special cases where you want the preview to live update.
fn cached_default() -> bool {
    true
}

impl PreviewSpec {
    pub fn new(command: CommandSpec, offset: Option<Template>) -> Self {
        Self {
            command,
            offset,
            cached: false,
        }
    }

    pub fn from_str_command(command: &str) -> Self {
        Self {
            command: CommandSpec {
                inner: vec![
                    Template::parse(command)
                        .expect("Failed to parse preview command")
                        .into(),
                ],
                interactive: false,
                env: FxHashMap::default(),
                shell: None,
            },
            offset: None,
            cached: false,
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct UiSpec {
    #[serde(default)]
    pub ui_scale: Option<u16>,
    // `layout` is clearer for the user but collides with the overall `Layout` type.
    #[serde(rename = "layout", alias = "orientation", default)]
    pub orientation: Option<Orientation>,
    // Feature-specific configurations
    #[serde(default)]
    pub input_bar: Option<InputBarConfig>,
    #[serde(default)]
    pub preview_panel: Option<ui::PreviewPanelConfig>,
    #[serde(default)]
    pub results_panel: Option<ui::ResultsPanelConfig>,
    #[serde(default)]
    pub status_bar: Option<ui::StatusBarConfig>,
    #[serde(default)]
    pub help_panel: Option<ui::HelpPanelConfig>,
    #[serde(default)]
    pub remote_control: Option<ui::RemoteControlConfig>,
}

pub const DEFAULT_PROTOTYPE_NAME: &str = "files";

impl From<&crate::config::UiConfig> for UiSpec {
    fn from(config: &crate::config::UiConfig) -> Self {
        UiSpec {
            ui_scale: Some(config.ui_scale),
            orientation: Some(config.orientation),
            input_bar: Some(config.input_bar.clone()),
            preview_panel: Some(config.preview_panel.clone()),
            results_panel: Some(config.results_panel.clone()),
            status_bar: Some(config.status_bar.clone()),
            help_panel: Some(config.help_panel.clone()),
            remote_control: Some(config.remote_control.clone()),
        }
    }
}

