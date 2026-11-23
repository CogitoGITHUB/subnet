//! Line-boundary math for `&str` buffers — the shared substrate every
//! line-aware consumer resolves through.
//!
//! Motions (`resolve_motion`), the linewise operator snap (`Editor::operate`),
//! and the [`LineBuffer`](super::LineBuffer) accessors all delegate here, so
//! line-start/line-end semantics (including CRLF handling) live in one place
//! instead of drifting across hand-rolled `find`/`rfind` copies.

/// Byte offset of the first character of the line containing `pos`.
///
/// Returns 0 for the first line.
pub(crate) fn start_of_line(buf: &str, pos: usize) -> usize {
    buf[..pos].rfind('\n').map_or(0, |i| i + 1)
    // str is guaranteed to be utf8, thus \n is safe to assume 1 byte long
}

/// Byte offset where the line containing `pos` ends, *excluding* the line
/// terminator: the position of the `\n` — or of the `\r` in a `\r\n` pair —
/// or `buf.len()` when the line is unterminated.
pub(crate) fn end_of_line(buf: &str, pos: usize) -> usize {
    match buf[pos..].find('\n') {
        None => buf.len(),
        Some(i) => {
            let newline = pos + i;
            if newline > 0 && buf.as_bytes().get(newline - 1) == Some(&b'\r') {
                newline - 1
            } else {
                newline
            }
        }
    }
}

/// Byte offset of the first non-whitespace character on the line containing
/// `pos`, or `None` when the line is blank.
///
/// Bounding the search to the line is what makes the `None` reachable.
/// [`LineBuffer::line_non_blank_start_index`](super::LineBuffer::line_non_blank_start_index)
/// searches on into the buffer and settles for the terminator, so it *moves* on
/// a blank line. The bound also keeps a `\r\n`'s `\r` out of the haystack.
pub(crate) fn first_non_blank(buf: &str, pos: usize) -> Option<usize> {
    let start = start_of_line(buf, pos);
    buf[start..end_of_line(buf, pos)]
        .find(|c: char| !c.is_whitespace())
        // `find` reports into the slice, not the buffer.
        .map(|offset| start + offset)
}

/// Byte offset just past the `\n` terminating the line containing `pos`, or
/// `None` when the line is unterminated (there is no line below).
pub(crate) fn start_of_next_line(buf: &str, pos: usize) -> Option<usize> {
    buf[pos..].find('\n').map(|i| pos + i + 1)
}

