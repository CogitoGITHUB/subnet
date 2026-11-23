use crate::{color_config::lookup_ansi_color_style, color_record_to_nustyle};
use nu_ansi_term::Style;
use console_protocol::{Config, Value, default_color_config};
use std::{collections::HashMap, sync::LazyLock};

/// Default `color_config` map — same source as `Config::default().color_config`
/// (`console_protocol::default_color_config`).
fn default_color_map() -> &'static HashMap<String, Value> {
    static MAP: LazyLock<HashMap<String, Value>> = LazyLock::new(default_color_config);
    &MAP
}

/// Resolve a shape style from a color_config value (string or `{ fg, bg, attr }` record).
fn style_from_color_value(value: &Value) -> Option<Style> {
    match value {
        Value::Record { .. } => Some(color_record_to_nustyle(value)),
        Value::String { val, .. } => Some(lookup_ansi_color_style(val)),
        // null / wrong types treated as unset
        _ => None,
    }
}

/// Default style for a shape key, derived from `default_color_config()` (not a
/// separate hard-coded table). Used when the live config is missing the key or
/// has an unusable value type.
pub fn default_shape_color(shape: &str) -> Style {
    default_color_map()
        .get(shape)
        .and_then(style_from_color_value)
        .unwrap_or_default()
}

pub fn get_shape_color(shape: &str, conf: &Config) -> Style {
    match conf.color_config.get(shape) {
        Some(int_color) => {
            // Shapes do not use color_config closures, currently.
            style_from_color_value(int_color).unwrap_or_else(|| default_shape_color(shape))
        }
        None => default_shape_color(shape),
    }
}

