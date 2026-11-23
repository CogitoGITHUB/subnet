//! Tree building, path operations, and CLI tree printing utilities.

use crate::explore_config::types::{NodeInfo, NuValueType, ValueType};
use serde_json::Value;
use std::collections::HashMap;
use tui_tree_widget::TreeItem;

/// Check if a JSON value is a leaf node (not an object or array)
pub fn is_leaf(value: &Value) -> bool {
    !value.is_object() && !value.is_array()
}

/// Render a leaf value as a string
pub fn render_leaf(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "<unserializable>".to_string())
}

/// Print a JSON tree structure to stdout (for CLI mode)
pub fn print_json_tree(value: &Value, prefix: &str, is_tail: bool, key: Option<&str>) {
    if let Some(k) = key {
        let connector = if is_tail { "└── " } else { "├── " };
        let leaf_part = if is_leaf(value) {
            format!(" {}", render_leaf(value))
        } else {
            String::new()
        };
        println!("{}{}{}:{}", prefix, connector, k, leaf_part);
    }

    if !is_leaf(value) {
        let branch = if is_tail { "    " } else { "│   " };
        let child_prefix = if key.is_none() {
            prefix.to_string()
        } else {
            format!("{}{}", prefix, branch)
        };

        match value {
            Value::Object(map) => {
                let mut entries: Vec<(&str, &Value)> =
                    map.iter().map(|(kk, vv)| (kk.as_str(), vv)).collect();
                entries.sort_by_key(|(kk, _)| *kk);
                for (idx, &(kk, vv)) in entries.iter().enumerate() {
                    let child_tail = idx == entries.len() - 1;
                    print_json_tree(vv, &child_prefix, child_tail, Some(kk));
                }
            }
            Value::Array(arr) => {
                for (idx, vv) in arr.iter().enumerate() {
                    let child_tail = idx == arr.len() - 1;
                    let idx_str = idx.to_string();
                    print_json_tree(vv, &child_prefix, child_tail, Some(&idx_str));
                }
            }
            _ => {}
        }
    }
}

/// Filter tree items based on a search query.
/// Returns a new tree with only items (and their ancestors) that match the query.
/// The search is case-insensitive and matches against the item's identifier (path).
pub fn filter_tree_items(
    items: &[TreeItem<'static, String>],
    query: &str,
) -> Vec<TreeItem<'static, String>> {
    let query_lower = query.to_lowercase();
    filter_tree_items_recursive(items, &query_lower)
}

/// Recursively filter tree items based on a search query.
///
/// This function traverses the tree and includes items that either:
/// 1. Have an identifier that contains the query string (case-insensitive match)
/// 2. Have descendants that match the query (ancestor preservation)
///
/// # Arguments
/// * `items` - The tree items to filter
/// * `query` - The lowercase search query to match against identifiers
///
/// # Returns
/// A new vector of tree items containing only matching items and their ancestors.
/// - Leaf items that match are cloned directly
/// - Parent items with matching children are rebuilt with only the filtered children
/// - Parent items that match but have no matching children are shown as collapsed leaves
///
/// # Note
/// If rebuilding a parent with filtered children fails (e.g., due to duplicate identifiers),
/// the item is included as a collapsed leaf to ensure no matches are silently dropped.
fn filter_tree_items_recursive(
    items: &[TreeItem<'static, String>],
    query: &str,
) -> Vec<TreeItem<'static, String>> {
    let mut result = Vec::new();

    for item in items {
        let identifier = item.identifier().clone();
        let identifier_lower = identifier.to_lowercase();

        // Check if this item's identifier matches the query
        let self_matches = identifier_lower.contains(query);

        // Recursively filter children
        let filtered_children = filter_tree_items_recursive(item.children(), query);

        // Include this item if it matches OR if any of its children matched
        if self_matches || !filtered_children.is_empty() {
            if item.children().is_empty() {
                // Leaf item - just clone it
                result.push(item.clone());
            } else if !filtered_children.is_empty() {
                // Has matching children - rebuild with filtered children
                match rebuild_item_with_children(item, filtered_children) {
                    Ok(new_item) => result.push(new_item),
                    Err(_) => {
                        // Fallback: if rebuild fails, include as collapsed leaf
                        // This ensures matching items aren't silently dropped
                        result.push(TreeItem::new_leaf(identifier, format_collapsed_label(item)));
                    }
                }
            } else {
                // Self matches but has children that don't match
                // Include as a leaf (collapsed view of matching parent)
                result.push(TreeItem::new_leaf(identifier, format_collapsed_label(item)));
            }
        }
    }

    result
}

/// Rebuild a tree item with new children, preserving the display format
fn rebuild_item_with_children(
    original: &TreeItem<'static, String>,
    new_children: Vec<TreeItem<'static, String>>,
) -> Result<TreeItem<'static, String>, std::io::Error> {
    let identifier = original.identifier().clone();
    // Extract the display text by getting the height (number of lines) and reconstructing
    // Since we can't access the text directly, use the identifier as a base for the label
    // The original label format is preserved in the clone, so we just need the identifier
    // to build a similar looking label
    let label = format_parent_label(&identifier, new_children.len());
    TreeItem::new(identifier, label, new_children)
}

/// Format a label for a parent node based on identifier
fn format_parent_label(identifier: &str, child_count: usize) -> String {
    // Extract the key name from the identifier (last part after the last dot)
    let key = identifier
        .rsplit('.')
        .next()
        .unwrap_or(identifier)
        .trim_start_matches('[')
        .trim_end_matches(']');
    format!("{} {{{} keys}}", key, child_count)
}

/// Format a display label for a tree item shown in collapsed form.
///
/// This is used when a parent item matches the search query but none of its
/// children match. The item is displayed as a leaf node with a label indicating
/// how many children it contains.
///
/// # Arguments
/// * `item` - The tree item to create a collapsed label for
///
/// # Returns
/// A string label in the format:
/// - `"key {N keys}"` for items with children (where N is the child count)
/// - `"key"` for leaf items (no children)
///
/// # Example
/// For an item with identifier `"color_config.string"` and 3 children,
/// this returns `"string {3 keys}"`.
fn format_collapsed_label(item: &TreeItem<'static, String>) -> String {
    let identifier = item.identifier();
    let key = identifier
        .rsplit('.')
        .next()
        .unwrap_or(identifier)
        .trim_start_matches('[')
        .trim_end_matches(']');
    let child_count = item.children().len();
    if child_count > 0 {
        format!("{} {{{} keys}}", key, child_count)
    } else {
        key.to_string()
    }
}

/// Build tree items for the TUI tree widget
pub fn build_tree_items(
    json_data: &Value,
    node_map: &mut HashMap<String, NodeInfo>,
    shell_type_map: &Option<HashMap<String, NuValueType>>,
    doc_map: &Option<HashMap<String, String>>,
) -> Vec<TreeItem<'static, String>> {
    build_tree_items_recursive(
        json_data,
        node_map,
        shell_type_map,
        doc_map,
        Vec::new(),
        String::new(),
    )
}

fn build_tree_items_recursive(
    value: &Value,
    node_map: &mut HashMap<String, NodeInfo>,
    shell_type_map: &Option<HashMap<String, NuValueType>>,
    doc_map: &Option<HashMap<String, String>>,
    current_path: Vec<String>,
    parent_id: String,
) -> Vec<TreeItem<'static, String>> {
    match value {
        Value::Object(map) => {
            // Sort keys alphabetically
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by_key(|(k, _)| *k);

            entries
                .into_iter()
                .map(|(key, val)| {
                    let mut path = current_path.clone();
                    path.push(key.clone());

                    let identifier = if parent_id.is_empty() {
                        key.clone()
                    } else {
                        format!("{}.{}", parent_id, key)
                    };

                    let value_type = ValueType::from_value(val);
                    let shell_type = shell_type_map
                        .as_ref()
                        .and_then(|m| m.get(&identifier).cloned());

                    // Check if documentation exists for this path
                    let config_path = path.join(".");
                    let has_doc = doc_map
                        .as_ref()
                        .is_some_and(|m| m.contains_key(&config_path))
                        || should_suppress_doc_warning(&path);

                    node_map.insert(
                        identifier.clone(),
                        NodeInfo {
                            path: path.clone(),
                            value_type,
                            shell_type,
                        },
                    );

                    let display = format_tree_label(key, val, has_doc, doc_map.is_some());

                    if is_leaf(val) {
                        TreeItem::new_leaf(identifier, display)
                    } else {
                        let children = build_tree_items_recursive(
                            val,
                            node_map,
                            shell_type_map,
                            doc_map,
                            path,
                            identifier.clone(),
                        );
                        TreeItem::new(identifier, display, children)
                            .expect("all item identifiers are unique")
                    }
                })
                .collect()
        }
        Value::Array(arr) => arr
            .iter()
            .enumerate()
            .map(|(idx, val)| {
                let mut path = current_path.clone();
                path.push(idx.to_string());

                let identifier = if parent_id.is_empty() {
                    format!("[{}]", idx)
                } else {
                    format!("{}[{}]", parent_id, idx)
                };

                let value_type = ValueType::from_value(val);
                let shell_type = shell_type_map
                    .as_ref()
                    .and_then(|m| m.get(&identifier).cloned());

                // Check if documentation exists for this path
                let config_path = path.join(".");
                let has_doc = doc_map
                    .as_ref()
                    .is_some_and(|m| m.contains_key(&config_path))
                    || should_suppress_doc_warning(&path);

                node_map.insert(
                    identifier.clone(),
                    NodeInfo {
                        path: path.clone(),
                        value_type,
                        shell_type,
                    },
                );

                let display = format_array_item_label(idx, val, has_doc, doc_map.is_some());

                if is_leaf(val) {
                    TreeItem::new_leaf(identifier, display)
                } else {
                    let children = build_tree_items_recursive(
                        val,
                        node_map,
                        shell_type_map,
                        doc_map,
                        path,
                        identifier.clone(),
                    );
                    TreeItem::new(identifier, display, children)
                        .expect("all item identifiers are unique")
                }
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Escape control characters that would cause multi-line rendering in tree labels
fn escape_for_display(s: &str) -> String {
    s.replace('\r', "\\r")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

/// Check if a path should suppress the "missing documentation" warning.
/// This is used for user-defined list items like keybindings and menus entries,
/// where individual items won't have documentation.
fn should_suppress_doc_warning(path: &[String]) -> bool {
    // Suppress warnings for any nested items under keybindings or menus
    // e.g., ["keybindings", "0", "name"] or ["menus", "1", "source"]
    if path.len() >= 2 {
        let first = path[0].as_str();
        if first == "keybindings" || first == "menus" {
            return true;
        }
    }
    false
}

fn format_tree_label(key: &str, value: &Value, has_doc: bool, is_config_mode: bool) -> String {
    let doc_marker = if is_config_mode && !has_doc {
        "⚠ "
    } else {
        ""
    };
    // Escape control characters in key to prevent multi-line tree items
    let safe_key = escape_for_display(key);
    match value {
        Value::Null => format!("{}{}: null", doc_marker, safe_key),
        Value::Bool(b) => format!("{}{}: {}", doc_marker, safe_key, b),
        Value::Number(n) => format!("{}{}: {}", doc_marker, safe_key, n),
        Value::String(s) => {
            // Truncate safely at char boundary, not byte boundary
            // Use nth() to check if string has more than 40 chars without counting all chars
            let needs_truncation = s.chars().nth(40).is_some();
            let preview = if needs_truncation {
                let truncated: String = s.chars().take(37).collect();
                format!("{}...", truncated)
            } else {
                s.clone()
            };
            format!(
                "{}{}: \"{}\"",
                doc_marker,
                safe_key,
                escape_for_display(&preview)
            )
        }
        Value::Array(arr) => format!("{}{} [{} items]", doc_marker, safe_key, arr.len()),
        Value::Object(obj) => format!("{}{} {{{} keys}}", doc_marker, safe_key, obj.len()),
    }
}

fn format_array_item_label(
    idx: usize,
    value: &Value,
    has_doc: bool,
    is_config_mode: bool,
) -> String {
    let doc_marker = if is_config_mode && !has_doc {
        "⚠ "
    } else {
        ""
    };
    match value {
        Value::Null => format!("{}[{}]: null", doc_marker, idx),
        Value::Bool(b) => format!("{}[{}]: {}", doc_marker, idx, b),
        Value::Number(n) => format!("{}[{}]: {}", doc_marker, idx, n),
        Value::String(s) => {
            // Truncate safely at char boundary, not byte boundary
            // Use nth() to check if string has more than 40 chars without counting all chars
            let needs_truncation = s.chars().nth(40).is_some();
            let preview = if needs_truncation {
                let truncated: String = s.chars().take(37).collect();
                format!("{}...", truncated)
            } else {
                s.clone()
            };
            format!(
                "{}[{}]: \"{}\"",
                doc_marker,
                idx,
                escape_for_display(&preview)
            )
        }
        Value::Array(arr) => format!("{}[{}] [{} items]", doc_marker, idx, arr.len()),
        Value::Object(obj) => format!("{}[{}] {{{} keys}}", doc_marker, idx, obj.len()),
    }
}

/// Get a value at a specific path in the JSON tree
pub fn get_value_at_path<'a>(value: &'a Value, path: &[String]) -> Option<&'a Value> {
    let mut current = value;
    for part in path {
        match current {
            Value::Object(map) => {
                current = map.get(part)?;
            }
            Value::Array(arr) => {
                let idx: usize = part.parse().ok()?;
                current = arr.get(idx)?;
            }
            _ => return None,
        }
    }
    Some(current)
}

/// Set a value at a specific path in the JSON tree
pub fn set_value_at_path(value: &mut Value, path: &[String], new_value: Value) -> bool {
    if path.is_empty() {
        *value = new_value;
        return true;
    }

    let mut current = value;
    for (i, part) in path.iter().enumerate() {
        if i == path.len() - 1 {
            // Last part, set the value
            match current {
                Value::Object(map) => {
                    map.insert(part.clone(), new_value);
                    return true;
                }
                Value::Array(arr) => {
                    if let Ok(idx) = part.parse::<usize>()
                        && idx < arr.len()
                    {
                        arr[idx] = new_value;
                        return true;
                    }
                    return false;
                }
                _ => return false,
            }
        } else {
            // Navigate deeper
            match current {
                Value::Object(map) => {
                    if let Some(next) = map.get_mut(part) {
                        current = next;
                    } else {
                        return false;
                    }
                }
                Value::Array(arr) => {
                    if let Ok(idx) = part.parse::<usize>() {
                        if idx < arr.len() {
                            current = &mut arr[idx];
                        } else {
                            return false;
                        }
                    } else {
                        return false;
                    }
                }
                _ => return false,
            }
        }
    }
    false
}

