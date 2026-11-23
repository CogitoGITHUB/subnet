use super::{ConfigErrors, ConfigPath, IntoValue, ShellError, UpdateFromValue, Value};
use crate::{self as console_protocol, FromValue, engine::EngineState};
use serde::{Deserialize, Serialize};
use std::io::IsTerminal;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, IntoValue, Serialize, Deserialize)]
pub enum UseAnsiColoring {
    #[default]
    Auto,
    True,
    False,
}

impl UseAnsiColoring {
    /// Determines whether ANSI colors should be used.
    ///
    /// This method evaluates the `UseAnsiColoring` setting and considers environment variables
    /// (`FORCE_COLOR`, `NO_COLOR`, and `CLICOLOR`) when the value is set to `Auto`.
    /// The configuration value (`UseAnsiColoring`) takes precedence over environment variables, as
    /// it is more direct and internally may be modified to override ANSI coloring behavior.
    ///
    /// Most users should have the default value `Auto` which allows the environment variables to
    /// control ANSI coloring.
    /// However, when explicitly set to `True` or `False`, the environment variables are ignored.
    ///
    /// Behavior based on `UseAnsiColoring`:
    /// - `True`: Forces ANSI colors to be enabled, ignoring terminal support and environment variables.
    /// - `False`: Disables ANSI colors completely.
    /// - `Auto`: Determines whether ANSI colors should be used based on environment variables and terminal support.
    ///
    /// When set to `Auto`, the following environment variables are checked in order:
    /// 1. `FORCE_COLOR`: If set, ANSI colors are always enabled, overriding all other settings.
    /// 2. `NO_COLOR`: If set, ANSI colors are disabled, overriding `CLICOLOR` and terminal checks.
    /// 3. `CLICOLOR`: If set, its value determines whether ANSI colors are enabled (`1` for enabled, `0` for disabled).
    ///
    /// If none of these variables are set, ANSI coloring is enabled only if the standard output is
    /// a terminal.
    ///
    /// By prioritizing the `UseAnsiColoring` value, we ensure predictable behavior and prevent
    /// conflicts with internal overrides that depend on this configuration.
    pub fn get(self, engine_state: &EngineState) -> bool {
        let is_terminal = match self {
            Self::Auto => std::io::stdout().is_terminal(),
            Self::True => return true,
            Self::False => return false,
        };

        let env_value = |env_name| {
            engine_state
                .get_env_var(env_name)
                .and_then(|v| v.coerce_bool().ok())
                .unwrap_or(false)
        };

        if env_value("force_color") {
            return true;
        }

        if env_value("no_color") {
            return false;
        }

        if let Some(cli_color) = engine_state.get_env_var("clicolor")
            && let Ok(cli_color) = cli_color.coerce_bool()
        {
            return cli_color;
        }

        // If the TERM environment variable is set to "dumb", disable ANSI colors
        if let Some(term) = engine_state.get_env_var("term")
            && term.as_str().ok() == Some("dumb")
        {
            return false;
        }

        is_terminal
    }
}

impl From<bool> for UseAnsiColoring {
    fn from(value: bool) -> Self {
        match value {
            true => Self::True,
            false => Self::False,
        }
    }
}

impl FromValue for UseAnsiColoring {
    fn from_value(v: Value) -> Result<Self, ShellError> {
        if let Ok(v) = v.as_bool() {
            return Ok(v.into());
        }

        #[derive(FromValue)]
        enum UseAnsiColoringString {
            Auto = 0,
            True = 1,
            False = 2,
        }

        Ok(match UseAnsiColoringString::from_value(v)? {
            UseAnsiColoringString::Auto => Self::Auto,
            UseAnsiColoringString::True => Self::True,
            UseAnsiColoringString::False => Self::False,
        })
    }
}

impl UpdateFromValue for UseAnsiColoring {
    fn update<'a>(
        &mut self,
        value: &'a Value,
        path: &mut ConfigPath<'a>,
        errors: &mut ConfigErrors,
    ) {
        let Ok(value) = UseAnsiColoring::from_value(value.clone()) else {
            errors.type_mismatch(path, UseAnsiColoring::expected_type(), value);
            return;
        };

        *self = value;
    }
}

