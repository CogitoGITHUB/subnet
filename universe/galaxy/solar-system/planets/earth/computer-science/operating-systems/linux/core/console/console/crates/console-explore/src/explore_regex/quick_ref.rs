//! Quick reference data for the regex explorer.
//!
//! This module provides categorized regex patterns compatible with the
//! fancy-regex crate, similar to regex101.com's quick reference panel.

/// A single quick reference item
#[derive(Clone)]
pub struct QuickRefItem {
    /// The pattern syntax to display (e.g., "\\d")
    pub syntax: &'static str,
    /// Description of what the pattern matches
    pub description: &'static str,
    /// The actual pattern to insert (may differ from syntax for display purposes)
    pub insert: &'static str,
}

/// A category of quick reference items
pub struct QuickRefCategory {
    pub name: &'static str,
    pub items: &'static [QuickRefItem],
}

/// All quick reference categories - patterns are compatible with fancy-regex
pub static QUICK_REF_CATEGORIES: &[QuickRefCategory] = &[
    QuickRefCategory {
        name: "Anchors",
        items: &[
            QuickRefItem {
                syntax: "^",
                description: "Start of string/line",
                insert: "^",
            },
            QuickRefItem {
                syntax: "$",
                description: "End of string/line",
                insert: "$",
            },
            QuickRefItem {
                syntax: "\\b",
                description: "Word boundary",
                insert: "\\b",
            },
            QuickRefItem {
                syntax: "\\B",
                description: "Not a word boundary",
                insert: "\\B",
            },
            QuickRefItem {
                syntax: "\\A",
                description: "Start of string only",
                insert: "\\A",
            },
            QuickRefItem {
                syntax: "\\z",
                description: "End of string only",
                insert: "\\z",
            },
            QuickRefItem {
                syntax: "\\Z",
                description: "End before trailing newlines",
                insert: "\\Z",
            },
            QuickRefItem {
                syntax: "\\G",
                description: "Where previous match ended",
                insert: "\\G",
            },
        ],
    },
    QuickRefCategory {
        name: "Character Classes",
        items: &[
            QuickRefItem {
                syntax: ".",
                description: "Any character except newline",
                insert: ".",
            },
            QuickRefItem {
                syntax: "\\O",
                description: "Any character including newline",
                insert: "\\O",
            },
            QuickRefItem {
                syntax: "\\d",
                description: "Digit [0-9]",
                insert: "\\d",
            },
            QuickRefItem {
                syntax: "\\D",
                description: "Not a digit [^0-9]",
                insert: "\\D",
            },
            QuickRefItem {
                syntax: "\\w",
                description: "Word character [a-zA-Z0-9_]",
                insert: "\\w",
            },
            QuickRefItem {
                syntax: "\\W",
                description: "Not a word character",
                insert: "\\W",
            },
            QuickRefItem {
                syntax: "\\s",
                description: "Whitespace character",
                insert: "\\s",
            },
            QuickRefItem {
                syntax: "\\S",
                description: "Not a whitespace character",
                insert: "\\S",
            },
            QuickRefItem {
                syntax: "\\h",
                description: "Hex digit [0-9A-Fa-f]",
                insert: "\\h",
            },
            QuickRefItem {
                syntax: "\\H",
                description: "Not a hex digit",
                insert: "\\H",
            },
            QuickRefItem {
                syntax: "[abc]",
                description: "Any of a, b, or c",
                insert: "[abc]",
            },
            QuickRefItem {
                syntax: "[^abc]",
                description: "Not a, b, or c",
                insert: "[^abc]",
            },
            QuickRefItem {
                syntax: "[a-z]",
                description: "Character range a-z",
                insert: "[a-z]",
            },
            QuickRefItem {
                syntax: "[A-Z]",
                description: "Character range A-Z",
                insert: "[A-Z]",
            },
            QuickRefItem {
                syntax: "[0-9]",
                description: "Character range 0-9",
                insert: "[0-9]",
            },
        ],
    },
    QuickRefCategory {
        name: "Quantifiers",
        items: &[
            QuickRefItem {
                syntax: "*",
                description: "0 or more (greedy)",
                insert: "*",
            },
            QuickRefItem {
                syntax: "+",
                description: "1 or more (greedy)",
                insert: "+",
            },
            QuickRefItem {
                syntax: "?",
                description: "0 or 1 (greedy)",
                insert: "?",
            },
            QuickRefItem {
                syntax: "{n}",
                description: "Exactly n times",
                insert: "{1}",
            },
            QuickRefItem {
                syntax: "{n,}",
                description: "n or more times",
                insert: "{1,}",
            },
            QuickRefItem {
                syntax: "{n,m}",
                description: "Between n and m times",
                insert: "{1,3}",
            },
            QuickRefItem {
                syntax: "*?",
                description: "0 or more (lazy)",
                insert: "*?",
            },
            QuickRefItem {
                syntax: "+?",
                description: "1 or more (lazy)",
                insert: "+?",
            },
            QuickRefItem {
                syntax: "??",
                description: "0 or 1 (lazy)",
                insert: "??",
            },
            QuickRefItem {
                syntax: "{n,m}?",
                description: "Between n and m (lazy)",
                insert: "{1,3}?",
            },
        ],
    },
    QuickRefCategory {
        name: "Groups & Capturing",
        items: &[
            QuickRefItem {
                syntax: "(...)",
                description: "Capturing group",
                insert: "()",
            },
            QuickRefItem {
                syntax: "(?:...)",
                description: "Non-capturing group",
                insert: "(?:)",
            },
            QuickRefItem {
                syntax: "(?<name>...)",
                description: "Named capturing group",
                insert: "(?<name>)",
            },
            QuickRefItem {
                syntax: "(?P<name>...)",
                description: "Named group (Python style)",
                insert: "(?P<name>)",
            },
            QuickRefItem {
                syntax: "(?>...)",
                description: "Atomic group (no backtrack)",
                insert: "(?>)",
            },
            QuickRefItem {
                syntax: "\\1",
                description: "Backreference to group 1",
                insert: "\\1",
            },
            QuickRefItem {
                syntax: "\\k<name>",
                description: "Backreference to named group",
                insert: "\\k<name>",
            },
            QuickRefItem {
                syntax: "(?P=name)",
                description: "Named backref (Python style)",
                insert: "(?P=name)",
            },
            QuickRefItem {
                syntax: "a|b",
                description: "Match a or b (alternation)",
                insert: "|",
            },
        ],
    },
    QuickRefCategory {
        name: "Lookaround",
        items: &[
            QuickRefItem {
                syntax: "(?=...)",
                description: "Positive lookahead",
                insert: "(?=)",
            },
            QuickRefItem {
                syntax: "(?!...)",
                description: "Negative lookahead",
                insert: "(?!)",
            },
            QuickRefItem {
                syntax: "(?<=...)",
                description: "Positive lookbehind",
                insert: "(?<=)",
            },
            QuickRefItem {
                syntax: "(?<!...)",
                description: "Negative lookbehind",
                insert: "(?<!)",
            },
        ],
    },
    QuickRefCategory {
        name: "Conditionals",
        items: &[
            QuickRefItem {
                syntax: "(?(1)yes|no)",
                description: "If group 1 matched",
                insert: "(?(1)|)",
            },
            QuickRefItem {
                syntax: "(?(<n>)yes|no)",
                description: "If named group matched",
                insert: "(?(<name>)|)",
            },
        ],
    },
    QuickRefCategory {
        name: "Special",
        items: &[
            QuickRefItem {
                syntax: "\\K",
                description: "Reset match start",
                insert: "\\K",
            },
            QuickRefItem {
                syntax: "\\e",
                description: "Escape character (\\x1B)",
                insert: "\\e",
            },
        ],
    },
    QuickRefCategory {
        name: "Escape Sequences",
        items: &[
            QuickRefItem {
                syntax: "\\n",
                description: "Newline",
                insert: "\\n",
            },
            QuickRefItem {
                syntax: "\\r",
                description: "Carriage return",
                insert: "\\r",
            },
            QuickRefItem {
                syntax: "\\t",
                description: "Tab",
                insert: "\\t",
            },
            QuickRefItem {
                syntax: "\\xHH",
                description: "Hex character code",
                insert: "\\x00",
            },
            QuickRefItem {
                syntax: "\\u{HHHH}",
                description: "Unicode code point",
                insert: "\\u{0000}",
            },
            QuickRefItem {
                syntax: "\\\\",
                description: "Literal backslash",
                insert: "\\\\",
            },
            QuickRefItem {
                syntax: "\\.",
                description: "Literal dot",
                insert: "\\.",
            },
            QuickRefItem {
                syntax: "\\*",
                description: "Literal asterisk",
                insert: "\\*",
            },
            QuickRefItem {
                syntax: "\\+",
                description: "Literal plus",
                insert: "\\+",
            },
            QuickRefItem {
                syntax: "\\?",
                description: "Literal question mark",
                insert: "\\?",
            },
            QuickRefItem {
                syntax: "\\^",
                description: "Literal caret",
                insert: "\\^",
            },
            QuickRefItem {
                syntax: "\\$",
                description: "Literal dollar",
                insert: "\\$",
            },
            QuickRefItem {
                syntax: "\\[",
                description: "Literal bracket",
                insert: "\\[",
            },
            QuickRefItem {
                syntax: "\\(",
                description: "Literal parenthesis",
                insert: "\\(",
            },
            QuickRefItem {
                syntax: "\\{",
                description: "Literal brace",
                insert: "\\{",
            },
            QuickRefItem {
                syntax: "\\|",
                description: "Literal pipe",
                insert: "\\|",
            },
        ],
    },
    QuickRefCategory {
        name: "Flags/Modifiers",
        items: &[
            QuickRefItem {
                syntax: "(?i)",
                description: "Case insensitive",
                insert: "(?i)",
            },
            QuickRefItem {
                syntax: "(?m)",
                description: "Multiline (^ $ match lines)",
                insert: "(?m)",
            },
            QuickRefItem {
                syntax: "(?s)",
                description: "Dotall (. matches newlines)",
                insert: "(?s)",
            },
            QuickRefItem {
                syntax: "(?x)",
                description: "Extended (ignore whitespace)",
                insert: "(?x)",
            },
            QuickRefItem {
                syntax: "(?-i)",
                description: "Disable case insensitive",
                insert: "(?-i)",
            },
            QuickRefItem {
                syntax: "(?im)",
                description: "Multiple flags",
                insert: "(?im)",
            },
            QuickRefItem {
                syntax: "(?i:...)",
                description: "Flags for group only",
                insert: "(?i:)",
            },
        ],
    },
    QuickRefCategory {
        name: "Common Patterns",
        items: &[
            QuickRefItem {
                syntax: "\\d+",
                description: "One or more digits",
                insert: "\\d+",
            },
            QuickRefItem {
                syntax: "\\w+",
                description: "One or more word chars",
                insert: "\\w+",
            },
            QuickRefItem {
                syntax: "\\S+",
                description: "One or more non-whitespace",
                insert: "\\S+",
            },
            QuickRefItem {
                syntax: ".*",
                description: "Any characters (greedy)",
                insert: ".*",
            },
            QuickRefItem {
                syntax: ".*?",
                description: "Any characters (lazy)",
                insert: ".*?",
            },
            QuickRefItem {
                syntax: "^.*$",
                description: "Entire line",
                insert: "^.*$",
            },
            QuickRefItem {
                syntax: "(\\w+) \\1",
                description: "Repeated word",
                insert: "(\\w+) \\1",
            },
            QuickRefItem {
                syntax: "(?<!\\S)",
                description: "Start of word (lookbehind)",
                insert: "(?<!\\S)",
            },
            QuickRefItem {
                syntax: "(?!\\S)",
                description: "End of word (lookahead)",
                insert: "(?!\\S)",
            },
        ],
    },
];

/// Represents a flattened entry (either a category header or an item)
#[derive(Clone)]
pub enum QuickRefEntry {
    Category(&'static str),
    Item(QuickRefItem),
}

/// Get a flattened list of all entries (headers and items)
pub fn get_flattened_entries() -> Vec<QuickRefEntry> {
    let mut entries = Vec::new();
    for category in QUICK_REF_CATEGORIES {
        entries.push(QuickRefEntry::Category(category.name));
        for item in category.items {
            entries.push(QuickRefEntry::Item(item.clone()));
        }
    }
    entries
}

