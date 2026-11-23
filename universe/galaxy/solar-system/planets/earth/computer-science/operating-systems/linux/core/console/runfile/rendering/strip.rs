//! One job: comments and marks stripped here, in one place.
//!
//! strip.rs is the **only** stripper in the package. If another file decides to
//! drop a line, it calls this one; nobody re-implements the rule. It knows two
//! shapes of line and nothing about meaning:
//!   * a block line, "<name> start" or "<name> end",
//!   * a declaration line, "keyword: value" with a registered keyword.
//! Everything else is content and is kept exactly as written. A code body is
//! opaque: inside it every line is kept until the exact line "code end".
//!
//! Comments are the third thing it drops, and the spans come from text/:
//! text/comments.rs says where they are. This file never reads a
//! comment itself, only the byte ranges it must drop, so there is exactly one
//! place that knows what a comment is and one place that removes it.

use crate::buffer::Buffer;
use crate::comments::{find_comments, is_comment_line};
use crate::parser::DECLARATION_KEYWORDS;

/// True when the line is a block line: the two words "<name> start", or the two
/// words "<name> end". Anything else, including the retired "end <name>", is
/// content.
pub fn is_block_line(line: &str) -> bool {
    let trimmed = line.trim_end();
    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    parts.len() == 2 && (parts[1] == "start" || parts[1] == "end")
}

/// True when the line declares something with a registered keyword.
pub fn is_declaration_line(line: &str) -> bool {
    match line.find(":") {
        None => false,
        Some(colon) => {
            let keyword = line[..colon].trim();
            !keyword.is_empty() && DECLARATION_KEYWORDS.contains(&keyword)
        }
    }
}

/// True when the line is anything the reader should never see: a block line, a
/// declaration line, or a line that is only whitespace. A line inside a
/// comment is not judged here, it needs the span, not the shape.
pub fn is_stripped(line: &str) -> bool {
    line.trim().is_empty() || is_block_line(line) || is_declaration_line(line)
}

/// True when the line opens a code block: the exact line "code start", alone at column 0.
fn is_code_start(line: &str) -> bool {
    line.trim_end() == "code start"
}

/// True when the line closes a code block: the exact line "code end", alone at column 0.
fn is_code_end(line: &str) -> bool {
    line.trim_end() == "code end"
}

/// Strip the marks and the comments: keep the content lines, drop everything
/// that is only syntax. Line endings are kept exactly as they came, so a
/// stripped file still has one newline per line it had, and the dropped lines
/// take their own endings with them so the kept lines stay whole. Inside a code
/// block every line is kept, blank lines and marker-looking lines included,
/// except the declarations straight under the start line and the exact end line.
pub fn strip_marks(text: &str) -> String {
    let buffer = Buffer::from_text(text);
    let comments = find_comments(&buffer);
    let mut out = String::new();
    let mut in_code = false;
    let mut at_top = false;
    for line in 1..=buffer.line_count() {
        let start = buffer.line_start(line);
        let end = buffer.line_end(line);
        let shown = buffer.line_text(line);
        // line_bytes keeps the line own ending, so nothing is re-invented
        let kept = std::str::from_utf8(buffer.line_bytes(line)).unwrap_or("");
        if in_code {
            if is_code_end(shown) {
                in_code = false;
                continue;
            }
            if at_top && is_declaration_line(shown) {
                continue;
            }
            at_top = false;
            out.push_str(kept);
            continue;
        }
        if is_comment_line(&comments, start, end) {
            continue;
        }
        if is_code_start(shown) {
            in_code = true;
            at_top = true;
            continue;
        }
        if is_stripped(shown) {
            continue;
        }
        out.push_str(kept);
    }
    out
}

