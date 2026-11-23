use unicode_segmentation::UnicodeSegmentation;

pub fn remove_last_grapheme(string: &str) -> &str {
    let mut it = UnicodeSegmentation::graphemes(string, true);

    if it.next_back().is_some() {
        it.as_str()
    } else {
        ""
    }
}

