/// Truncate a path to only have a set number of path components
///
/// Will truncate a path to only show the last `length` components in a path.
/// If a length of `0` is provided, the path will not be truncated.
/// A value will only be returned if the path has been truncated.
pub fn truncate(dir_string: &str, length: usize) -> Option<String> {
    if length == 0 {
        return None;
    }

    let mut components = dir_string.split('/').collect::<Vec<&str>>();

    // If the first element is "" then there was a leading "/" and we should remove it so we can check the actual count of components
    if components[0].is_empty() {
        components.remove(0);
    }

    if components.len() <= length {
        return None;
    }

    let truncated_components = &components[components.len() - length..];
    Some(truncated_components.join("/"))
}

