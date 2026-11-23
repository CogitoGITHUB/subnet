use unicode_segmentation::UnicodeSegmentation;

/// Truncate a string to only have a set number of characters
///
/// Will truncate a string to only show the last `length` character in the string.
/// If a length of `0` is provided, the string will not be truncated and the original
/// will be returned.
pub fn truncate_text(text: &str, length: usize, truncation_symbol: &str) -> String {
    if length == 0 {
        return String::from(text);
    }

    let truncated_graphemes = get_graphemes(text, length);
    // The truncation symbol should only be added if we truncated
    if length < graphemes_len(text) {
        let truncation_symbol = get_graphemes(truncation_symbol, 1);
        truncated_graphemes + truncation_symbol.as_str()
    } else {
        truncated_graphemes
    }
}

fn get_graphemes(text: &str, length: usize) -> String {
    UnicodeSegmentation::graphemes(text, true)
        .take(length)
        .collect::<Vec<&str>>()
        .concat()
}

fn graphemes_len(text: &str) -> usize {
    UnicodeSegmentation::graphemes(text, true).count()
}

