//! Conversion utilities between Nu values and JSON, and config documentation parsing.

use crate::explore_config::types::NuValueType;
use console_config::ConfigFileKind;
use console_protocol::engine::EngineState;
use console_protocol::{ShellError, shell_error::generic::GenericError};
use serde_json::Value;
use std::collections::HashMap;
use std::error::Error;

/// Convert a console_protocol::Value to a serde_json::Value
/// This properly handles closures by converting them to their string representation
#[allow(clippy::only_used_in_recursion)]
pub fn shell_value_to_json(
    engine_state: &EngineState,
    value: &console_protocol::Value,
    span: console_protocol::Span,
) -> Result<Value, ShellError> {
    Ok(match value {
        console_protocol::Value::Bool { val, .. } => Value::Bool(*val),
        console_protocol::Value::Int { val, .. } => Value::Number((*val).into()),
        console_protocol::Value::Float { val, .. } => serde_json::Number::from_f64(*val)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        console_protocol::Value::String { val, .. } => Value::String(val.clone()),
        console_protocol::Value::Nothing { .. } => Value::Null,
        console_protocol::Value::List { vals, .. } => {
            let json_vals: Result<Vec<_>, _> = vals
                .iter()
                .map(|v| shell_value_to_json(engine_state, v, span))
                .collect();
            Value::Array(json_vals?)
        }
        console_protocol::Value::Record { val, .. } => {
            let mut map = serde_json::Map::new();
            for (k, v) in val.iter() {
                map.insert(k.clone(), shell_value_to_json(engine_state, v, span)?);
            }
            Value::Object(map)
        }
        console_protocol::Value::Closure { val, .. } => {
            // Convert closure to its string representation instead of serializing internal structure
            let closure_string =
                val.coerce_into_string(engine_state, value.span())
                    .map_err(|e| {
                        ShellError::Generic(
                            GenericError::new(
                                "Failed to convert closure to string",
                                "",
                                value.span(),
                            )
                            .with_inner([e]),
                        )
                    })?;
            Value::String(closure_string.to_string())
        }
        console_protocol::Value::Filesize { val, .. } => Value::Number(val.get().into()),
        console_protocol::Value::Duration { val, .. } => Value::Number((*val).into()),
        console_protocol::Value::Date { val, .. } => Value::String(val.to_string()),
        console_protocol::Value::Glob { val, .. } => Value::String(val.to_string()),
        console_protocol::Value::CellPath { val, .. } => {
            let parts: Vec<Value> = val
                .members
                .iter()
                .map(|m| match m {
                    console_protocol::ast::PathMember::String { val, .. } => Value::String(val.clone()),
                    console_protocol::ast::PathMember::Int { val, .. } => {
                        Value::Number((*val as i64).into())
                    }
                })
                .collect();
            Value::Array(parts)
        }
        console_protocol::Value::Binary { val, .. } => Value::Array(
            val.iter()
                .map(|b| Value::Number((*b as i64).into()))
                .collect(),
        ),
        console_protocol::Value::Range { .. } => Value::Null,
        console_protocol::Value::Error { error, .. } => {
            return Err(*error.clone());
        }
        console_protocol::Value::Custom { val, .. } => {
            let collected = val.to_base_value(value.span())?;
            shell_value_to_json(engine_state, &collected, span)?
        }
    })
}

/// Parse the doc_config.nu file to extract documentation for each config path
/// Returns a HashMap mapping config paths (e.g., "history.file_format") to their documentation
pub fn parse_config_documentation() -> HashMap<String, String> {
    let doc_content = ConfigFileKind::Config.doc();
    let mut doc_map = HashMap::new();
    let mut current_comments: Vec<String> = Vec::new();

    for line in doc_content.lines() {
        let trimmed = line.trim();

        if trimmed.is_empty() {
            // Empty lines clear the comment buffer - this ensures section headings
            // (which are separated from actual documentation by blank lines)
            // don't get included in the documentation for settings
            current_comments.clear();
        } else if trimmed.starts_with('#') {
            // Collect comment lines (strip the leading # and space)
            let comment = trimmed.trim_start_matches('#').trim();
            if !comment.is_empty() {
                current_comments.push(comment.to_string());
            }
        } else if trimmed.starts_with("$env.config.") {
            // This is a config setting line
            // Extract the path (everything between "$env.config." and " =" or end of relevant part)
            if let Some(path) = extract_config_path(trimmed)
                && !current_comments.is_empty()
            {
                // Join all collected comments as the documentation
                let doc = current_comments.join("\n");
                doc_map.insert(path, doc);
            }
            // Clear comments after processing a setting
            current_comments.clear();
        } else {
            // Non-comment, non-config, non-empty line - might be code examples, clear comments
            current_comments.clear();
        }
    }

    doc_map
}

/// Extract the config path from a line like "$env.config.history.file_format = ..."
/// Returns the path without "$env.config." prefix (e.g., "history.file_format")
pub fn extract_config_path(line: &str) -> Option<String> {
    let line = line.trim();
    if !line.starts_with("$env.config.") {
        return None;
    }

    // Remove "$env.config." prefix
    let rest = &line["$env.config.".len()..];

    // Find where the path ends (at '=' or end of line for bare references)
    let path_end = rest.find(['=', ' ']).unwrap_or(rest.len());

    let path = rest[..path_end].trim();
    if path.is_empty() {
        None
    } else {
        Some(path.to_string())
    }
}

/// Build a map of path identifiers to NuValueType for tracking original shell types
pub fn build_nu_type_map(
    value: &console_protocol::Value,
    current_path: Vec<String>,
    type_map: &mut HashMap<String, NuValueType>,
) {
    let identifier = path_to_identifier(&current_path);

    if !identifier.is_empty() {
        type_map.insert(identifier.clone(), NuValueType::from_nu_value(value));
    }

    match value {
        console_protocol::Value::Record { val, .. } => {
            for (k, v) in val.iter() {
                let mut path = current_path.clone();
                path.push(k.clone());
                build_nu_type_map(v, path, type_map);
            }
        }
        console_protocol::Value::List { vals, .. } => {
            for (idx, v) in vals.iter().enumerate() {
                let mut path = current_path.clone();
                path.push(idx.to_string());
                build_nu_type_map(v, path, type_map);
            }
        }
        _ => {}
    }
}

/// Build a map of path identifiers to original Nu values for types that can't be roundtripped
/// (like Closures, Dates, Ranges, etc.)
pub fn build_original_value_map(
    value: &console_protocol::Value,
    current_path: Vec<String>,
    value_map: &mut HashMap<String, console_protocol::Value>,
) {
    let identifier = path_to_identifier(&current_path);

    // Store values that can't be roundtripped through JSON
    if !identifier.is_empty() {
        match value {
            console_protocol::Value::Closure { .. }
            | console_protocol::Value::Date { .. }
            | console_protocol::Value::Range { .. } => {
                value_map.insert(identifier.clone(), value.clone());
            }
            _ => {}
        }
    }

    match value {
        console_protocol::Value::Record { val, .. } => {
            for (k, v) in val.iter() {
                let mut path = current_path.clone();
                path.push(k.clone());
                build_original_value_map(v, path, value_map);
            }
        }
        console_protocol::Value::List { vals, .. } => {
            for (idx, v) in vals.iter().enumerate() {
                let mut path = current_path.clone();
                path.push(idx.to_string());
                build_original_value_map(v, path, value_map);
            }
        }
        _ => {}
    }
}

/// Convert a path vector to an identifier string (e.g., ["history", "file_format"] -> "history.file_format")
fn path_to_identifier(path: &[String]) -> String {
    if path.is_empty() {
        String::new()
    } else {
        path.iter()
            .enumerate()
            .map(|(i, p)| {
                if p.parse::<usize>().is_ok() {
                    format!("[{}]", p)
                } else if i == 0 {
                    p.clone()
                } else {
                    format!(".{}", p)
                }
            })
            .collect::<String>()
    }
}

/// Convert a serde_json::Value to a console_protocol::Value (simple version without type info)
#[allow(dead_code)]
pub fn json_to_nu_value(
    json: &Value,
    span: console_protocol::Span,
) -> Result<console_protocol::Value, Box<dyn Error>> {
    json_to_nu_value_with_types(json, span, &None, &None, Vec::new())
}

/// Convert a serde_json::Value to a console_protocol::Value, using type information to preserve
/// original Nu types like Duration, Filesize, and Closure
pub fn json_to_nu_value_with_types(
    json: &Value,
    span: console_protocol::Span,
    type_map: &Option<HashMap<String, NuValueType>>,
    original_values: &Option<HashMap<String, console_protocol::Value>>,
    current_path: Vec<String>,
) -> Result<console_protocol::Value, Box<dyn Error>> {
    let identifier = path_to_identifier(&current_path);
    let original_type = type_map.as_ref().and_then(|m| m.get(&identifier));

    Ok(match json {
        Value::Null => console_protocol::Value::nothing(span),
        Value::Bool(b) => console_protocol::Value::bool(*b, span),
        Value::Number(n) => {
            // Check if we need to convert to a special type based on original
            if let Some(orig_type) = original_type {
                match orig_type {
                    NuValueType::Duration => {
                        if let Some(i) = n.as_i64() {
                            return Ok(console_protocol::Value::duration(i, span));
                        }
                    }
                    NuValueType::Filesize => {
                        if let Some(i) = n.as_i64() {
                            return Ok(console_protocol::Value::filesize(i, span));
                        }
                    }
                    _ => {}
                }
            }
            // Default number handling
            if let Some(i) = n.as_i64() {
                console_protocol::Value::int(i, span)
            } else if let Some(f) = n.as_f64() {
                console_protocol::Value::float(f, span)
            } else {
                return Err(format!("Unsupported number: {}", n).into());
            }
        }
        Value::String(s) => {
            // Check if we need to restore an original value that can't be roundtripped
            if let Some(orig_type) = original_type {
                match orig_type {
                    NuValueType::Closure | NuValueType::Date | NuValueType::Range => {
                        // Try to get the original value - closures, dates, and ranges
                        // can't be reconstructed from their string representation
                        if let Some(original_values_map) = original_values
                            && let Some(original_value) = original_values_map.get(&identifier)
                        {
                            // Return the original value since we can't reconstruct these types
                            return Ok(original_value.clone());
                        }
                        // If no original value found, keep as string
                        // This will likely cause a config error, but that's the expected behavior
                        // since the user modified something that can't be properly converted
                    }
                    NuValueType::Glob => {
                        return Ok(console_protocol::Value::glob(s.clone(), false, span));
                    }
                    _ => {}
                }
            }
            console_protocol::Value::string(s.clone(), span)
        }
        Value::Array(arr) => {
            // Check if this was originally binary data
            if let Some(NuValueType::Binary) = original_type {
                let bytes: Result<Vec<u8>, _> = arr
                    .iter()
                    .map(|v| {
                        v.as_i64()
                            .and_then(|i| u8::try_from(i).ok())
                            .ok_or("Invalid byte value")
                    })
                    .collect();
                if let Ok(bytes) = bytes {
                    return Ok(console_protocol::Value::binary(bytes, span));
                }
            }

            // Check if this was originally a CellPath
            if let Some(NuValueType::CellPath) = original_type {
                use console_protocol::ast::PathMember;
                use console_protocol::casing::Casing;
                let members: Result<Vec<PathMember>, _> = arr
                    .iter()
                    .map(|v| match v {
                        Value::String(s) => Ok(PathMember::String {
                            val: s.clone(),
                            span,
                            optional: false,
                            casing: Casing::Sensitive,
                        }),
                        Value::Number(n) => {
                            if let Some(i) = n.as_u64() {
                                Ok(PathMember::Int {
                                    val: i as usize,
                                    span,
                                    optional: false,
                                })
                            } else {
                                Err("Invalid cell path member")
                            }
                        }
                        _ => Err("Invalid cell path member"),
                    })
                    .collect();
                if let Ok(members) = members {
                    return Ok(console_protocol::Value::cell_path(
                        console_protocol::ast::CellPath { members },
                        span,
                    ));
                }
            }

            // Regular array/list
            let values: Result<Vec<_>, _> = arr
                .iter()
                .enumerate()
                .map(|(idx, v)| {
                    let mut path = current_path.clone();
                    path.push(idx.to_string());
                    json_to_nu_value_with_types(v, span, type_map, original_values, path)
                })
                .collect();
            console_protocol::Value::list(values?, span)
        }
        Value::Object(obj) => {
            let mut record = console_protocol::Record::new();
            for (k, v) in obj {
                let mut path = current_path.clone();
                path.push(k.clone());
                record.push(
                    k.clone(),
                    json_to_nu_value_with_types(v, span, type_map, original_values, path)?,
                );
            }
            console_protocol::Value::record(record, span)
        }
    })
}

