use super::{config_update_string_enum, prelude::*};
use crate::{self as console_protocol, ConfigWarning};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, IntoValue, PartialEq, Eq, Serialize, Deserialize)]
pub enum HistoryFileFormat {
    /// Store history as an SQLite database with additional context
    Sqlite,
    /// store history as a plain text file where every line is one command (without any context such as timestamps)
    Plaintext,
}

impl HistoryFileFormat {
    pub fn default_file_name(self) -> std::path::PathBuf {
        match self {
            HistoryFileFormat::Plaintext => "history.txt",
            HistoryFileFormat::Sqlite => "history.sqlite3",
        }
        .into()
    }
}

impl FromStr for HistoryFileFormat {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "sqlite" => Ok(Self::Sqlite),
            "plaintext" => Ok(Self::Plaintext),
            #[cfg(feature = "sqlite")]
            _ => Err("'sqlite' or 'plaintext'"),
            #[cfg(not(feature = "sqlite"))]
            _ => Err("'plaintext'"),
        }
    }
}

impl UpdateFromValue for HistoryFileFormat {
    fn update(&mut self, value: &Value, path: &mut ConfigPath, errors: &mut ConfigErrors) {
        config_update_string_enum(self, value, path, errors);

        #[cfg(not(feature = "sqlite"))]
        if *self == HistoryFileFormat::Sqlite {
            *self = HistoryFileFormat::Plaintext;
            errors.warn(ConfigWarning::IncompatibleOptions {
                label: "SQLite-based history file only supported with the `sqlite` feature, falling back to plain text history", 
                span: value.span(),
                help: "Compile Nushell with `sqlite` feature enabled",
            });
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HistoryPath {
    Default,
    Custom(PathBuf),
    Disabled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryConfig {
    pub max_size: i64,
    pub sync_on_enter: bool,
    pub file_format: HistoryFileFormat,
    pub isolation: bool,
    pub path: HistoryPath,
    pub ignore_space_prefixed: bool,
}

impl IntoValue for HistoryPath {
    fn into_value(self, span: Span) -> Value {
        match self {
            HistoryPath::Default => Value::string("", span),
            HistoryPath::Disabled => Value::nothing(span),
            HistoryPath::Custom(path) => Value::string(path.display().to_string(), span),
        }
    }
}

impl IntoValue for HistoryConfig {
    fn into_value(self, span: Span) -> Value {
        Value::record(
            record! {
                "max_size" => self.max_size.into_value(span),
                "sync_on_enter" => self.sync_on_enter.into_value(span),
                "file_format" => self.file_format.into_value(span),
                "isolation" => self.isolation.into_value(span),
                "path" => self.path.into_value(span),
                "ignore_space_prefixed" => self.ignore_space_prefixed.into_value(span),
            },
            span,
        )
    }
}

impl HistoryConfig {
    /// Resolve the history file path.
    ///
    /// For [`HistoryPath::Default`], joins the history file name under
    /// `config_home`. Callers **must** pass the session's already-resolved
    /// config directory (`EngineState.config_dirs.config_home`) so that CLI
    /// overrides such as `--config-home` are honored.
    ///
    /// Do **not** re-resolve config location from the process environment here —
    /// that would break session isolation.
    ///
    /// # Arguments
    ///
    /// * `config_home` — resolved shell config directory for this session.
    pub fn file_path(&self, config_home: &Path) -> Option<PathBuf> {
        let path = match &self.path {
            HistoryPath::Custom(path) => Some(path.clone()),
            HistoryPath::Disabled => None,
            HistoryPath::Default => {
                let mut history_path = config_home.to_path_buf();
                history_path.push(self.file_format.default_file_name());
                Some(history_path)
            }
        }?;

        if path.is_dir() {
            return Some(path.join(self.file_format.default_file_name()));
        }

        Some(path)
    }
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            max_size: 100_000,
            sync_on_enter: true,
            file_format: HistoryFileFormat::Plaintext,
            isolation: false,
            path: HistoryPath::Default,
            ignore_space_prefixed: true,
        }
    }
}

impl UpdateFromValue for HistoryConfig {
    fn update<'a>(
        &mut self,
        value: &'a Value,
        path: &mut ConfigPath<'a>,
        errors: &mut ConfigErrors,
    ) {
        let Value::Record { val: record, .. } = value else {
            errors.type_mismatch(path, Type::record(), value);
            return;
        };

        // might not be correct if file format was changed away from sqlite rather than isolation,
        // but this is an edge case and the span of the relevant value here should be close enough
        let mut isolation_span = value.span();

        for (col, val) in record.iter() {
            let path = &mut path.push(col);
            match col.as_str() {
                "isolation" => {
                    isolation_span = val.span();
                    let prev = self.isolation;
                    self.isolation.update(val, path, errors);
                    if errors.history_locked_after_startup()
                        && self.isolation != errors.config().history.isolation
                    {
                        self.isolation = prev;
                        errors.locked_after_startup(path, val.span());
                    }
                }
                "sync_on_enter" => self.sync_on_enter.update(val, path, errors),
                "max_size" => {
                    let prev = self.max_size;
                    self.max_size.update(val, path, errors);
                    if errors.history_locked_after_startup()
                        && self.max_size != errors.config().history.max_size
                    {
                        self.max_size = prev;
                        errors.locked_after_startup(path, val.span());
                    }
                }
                "file_format" => {
                    let prev = self.file_format;
                    self.file_format.update(val, path, errors);
                    if errors.history_locked_after_startup()
                        && self.file_format != errors.config().history.file_format
                    {
                        self.file_format = prev;
                        errors.locked_after_startup(path, val.span());
                    }
                }
                "path" => match val {
                    Value::String { val: s, .. } => {
                        let new_path = if s.is_empty() {
                            HistoryPath::Default
                        } else {
                            HistoryPath::Custom(PathBuf::from(s))
                        };

                        if errors.history_locked_after_startup()
                            && new_path != errors.config().history.path
                        {
                            errors.locked_after_startup(path, val.span());
                            continue;
                        }

                        self.path = new_path;
                    }
                    Value::Nothing { .. } => {
                        if errors.history_locked_after_startup()
                            && errors.config().history.path != HistoryPath::Disabled
                        {
                            errors.locked_after_startup(path, val.span());
                            continue;
                        }

                        self.path = HistoryPath::Disabled;
                    }
                    _ => {
                        errors.type_mismatch(path, Type::custom("string or nothing"), val);
                    }
                },
                "ignore_space_prefixed" => self.ignore_space_prefixed.update(val, path, errors),
                _ => errors.unknown_option(path, val),
            }
        }

        // Listing all formats separately in case additional ones are added
        match (self.isolation, self.file_format) {
            (true, HistoryFileFormat::Plaintext) => {
                errors.warn(ConfigWarning::IncompatibleOptions {
                    label: "history isolation only compatible with SQLite format",
                    span: isolation_span,
                    help: r#"disable history isolation, or set $env.config.history.file_format = "sqlite""#,
                });
            }
            (true, HistoryFileFormat::Sqlite) => (),
            (false, _) => (),
        }
    }
}

