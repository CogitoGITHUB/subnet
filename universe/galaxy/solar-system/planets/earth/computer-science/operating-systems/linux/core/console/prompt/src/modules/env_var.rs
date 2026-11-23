use super::{Context, Module};
use std::borrow::Cow;

use crate::config::ModuleConfig;
use crate::configs::env_var::EnvVarConfig;
use crate::formatter::StringFormatter;

/// Creates a module with the value of the chosen environment variable
///
/// Will display the environment variable's value if all of the following criteria are met:
///     - `env_var.disabled` is absent or false
///     - `env_var.variable` is defined
///     - a variable named as the value of `env_var.variable` is defined
pub fn module<'a>(name: Option<&str>, context: &'a Context) -> Option<Module<'a>> {
    let toml_config = match name {
        Some(name) => context
            .config
            .get_config(&["env_var", name])
            .map(Cow::Borrowed),
        None => context
            .config
            .get_module_config("env_var")
            .and_then(filter_config)
            .map(Cow::Owned)
            .map(Some)?,
    };

    let mod_name = match name {
        Some(name) => format!("env_var.{name}"),
        None => "env_var".to_owned(),
    };

    let config = EnvVarConfig::try_load(toml_config.as_deref());
    // Note: Forward config if `Module` ends up needing `config`
    let mut module = Module::new(mod_name, config.description, None);
    if config.disabled {
        return None;
    }

    let variable_name = config.variable.or(name)?;

    let env_value = context.get_env(variable_name);
    let env_value = env_value.as_deref().or(config.default)?;
    let parsed = StringFormatter::new(config.format).and_then(|formatter| {
        formatter
            .map_meta(|var, _| match var {
                "symbol" => Some(config.symbol),
                _ => None,
            })
            .map_style(|variable| match variable {
                "style" => Some(Ok(config.style)),
                _ => None,
            })
            .map(|variable| match variable {
                "env_value" => Some(Ok(env_value)),
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `env_var`:\n{error}");
            return None;
        }
    });

    Some(module)
}

/// Filter `config` to only includes non-table values
/// This filters the top-level table to only include its specific configuration
fn filter_config(config: &toml::Value) -> Option<toml::Value> {
    let o = config
        .as_table()
        .map(|table| {
            table
                .iter()
                .filter(|(_key, val)| !val.is_table())
                .map(|(key, val)| (key.clone(), val.clone()))
                .collect::<toml::value::Table>()
        })
        .filter(|table| !table.is_empty())
        .map(toml::Value::Table);
    log::trace!("Filtered top-level env_var config: {o:?}");
    o
}

