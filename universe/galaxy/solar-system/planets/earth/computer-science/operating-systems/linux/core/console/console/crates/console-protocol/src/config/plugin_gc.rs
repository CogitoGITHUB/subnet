use super::prelude::*;
use crate as console_protocol;
use std::collections::HashMap;

/// Configures when plugins should be stopped if inactive
#[derive(Clone, Debug, Default, IntoValue, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginGcConfigs {
    /// The config to use for plugins not otherwise specified
    pub default: PluginGcConfig,
    /// Specific configs for plugins (by name)
    pub plugins: HashMap<String, PluginGcConfig>,
}

impl PluginGcConfigs {
    /// Get the plugin GC configuration for a specific plugin name. If not specified by name in the
    /// config, this is `default`.
    pub fn get(&self, plugin_name: &str) -> &PluginGcConfig {
        self.plugins.get(plugin_name).unwrap_or(&self.default)
    }
}

impl UpdateFromValue for PluginGcConfigs {
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

        for (col, val) in record.iter() {
            let path = &mut path.push(col);
            match col.as_str() {
                "default" => self.default.update(val, path, errors),
                "plugins" => self.plugins.update(val, path, errors),
                _ => errors.unknown_option(path, val),
            }
        }
    }
}

/// Configures when a plugin should be stopped if inactive
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginGcConfig {
    /// True if the plugin should be stopped automatically
    pub enabled: bool,
    /// When to stop the plugin if not in use for this long (in nanoseconds)
    pub stop_after: i64,
}

impl Default for PluginGcConfig {
    fn default() -> Self {
        PluginGcConfig {
            enabled: true,
            stop_after: 10_000_000_000, // 10sec
        }
    }
}

impl IntoValue for PluginGcConfig {
    fn into_value(self, span: Span) -> Value {
        record! {
            "enabled" => self.enabled.into_value(span),
            "stop_after" => Value::duration(self.stop_after, span),
        }
        .into_value(span)
    }
}

impl UpdateFromValue for PluginGcConfig {
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

        for (col, val) in record.iter() {
            let path = &mut path.push(col);
            match col.as_str() {
                "enabled" => self.enabled.update(val, path, errors),
                "stop_after" => {
                    if let Ok(duration) = val.as_duration() {
                        if duration >= 0 {
                            self.stop_after = duration;
                        } else {
                            errors.invalid_value(path, "a non-negative duration", val);
                        }
                    } else {
                        errors.type_mismatch(path, Type::Duration, val);
                    }
                }
                _ => errors.unknown_option(path, val),
            }
        }
    }
}

