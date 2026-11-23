use crate::{
    NuStyle,
    shell_style::{color_from_hex, lookup_style},
    parse_nustyle,
};
use nu_ansi_term::Style;
use console_protocol::{Record, Value};
use std::collections::HashMap;

pub fn lookup_ansi_color_style(s: &str) -> Style {
    if s.starts_with('#') {
        color_from_hex(s)
            .ok()
            .and_then(|c| c.map(|c| c.normal()))
            .unwrap_or_default()
    } else if s.starts_with('{') {
        color_string_to_nustyle(s)
    } else {
        lookup_style(s)
    }
}

pub fn get_color_map(colors: &HashMap<String, Value>) -> HashMap<String, Style> {
    let mut hm: HashMap<String, Style> = HashMap::new();

    for (key, value) in colors {
        parse_map_entry(&mut hm, key, value);
    }

    hm
}

fn parse_map_entry(hm: &mut HashMap<String, Style>, key: &str, value: &Value) {
    let value = match value {
        Value::String { val, .. } => Some(lookup_ansi_color_style(val)),
        Value::Record { val, .. } => {
            get_style_from_value(val).and_then(|ns| parse_nustyle(ns).ok())
        }
        _ => None,
    };
    if let Some(value) = value {
        hm.entry(key.to_owned()).or_insert(value);
    }
}

fn get_style_from_value(record: &Record) -> Option<NuStyle> {
    let mut was_set = false;
    let mut style = NuStyle::from(Style::default());
    for (col, val) in record {
        match col.as_str() {
            "bg" => {
                if let Value::String { val, .. } = val {
                    style.bg = Some(val.clone());
                    was_set = true;
                }
            }
            "fg" => {
                if let Value::String { val, .. } = val {
                    style.fg = Some(val.clone());
                    was_set = true;
                }
            }
            "attr" => {
                if let Value::String { val, .. } = val {
                    style.attr = Some(val.clone());
                    was_set = true;
                }
            }
            _ => (),
        }
    }

    if was_set { Some(style) } else { None }
}

fn color_string_to_nustyle(color_string: &str) -> Style {
    // eprintln!("color_string: {}", &color_string);
    if color_string.is_empty() {
        return Style::default();
    }

    let shell_style = match console_json::from_str::<NuStyle>(color_string) {
        Ok(s) => s,
        Err(_) => return Style::default(),
    };

    parse_nustyle(shell_style).unwrap_or_else(|_| Style::default())
}

