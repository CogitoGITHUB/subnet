use unicode_segmentation::{GraphemeCursor, UnicodeSegmentation};

/// Byte index of the next grapheme boundary at or after `pos`.
///
/// Returns `buf.len()` if there is no grapheme after `pos`.
///
/// # Panics
///
/// Panics if `pos` is not on a UTF-8 character boundary in `buf`.
pub fn next_grapheme_boundary(buf: &str, pos: usize) -> usize {
    debug_assert!(buf.is_char_boundary(pos), "pos must be a char boundary");
    buf[pos..]
        .grapheme_indices(true)
        .nth(1)
        .map(|(i, _)| pos + i)
        .unwrap_or(buf.len())
}

/// Byte index of the previous grapheme boundary before `pos`.
///
/// Returns `0` if there is no grapheme before `pos`.
///
/// # Panics
///
/// Panics if `pos` is not on a UTF-8 character boundary in `buf`.
pub fn prev_grapheme_boundary(buf: &str, pos: usize) -> usize {
    debug_assert!(buf.is_char_boundary(pos), "pos must be a char boundary");
    buf[..pos]
        .grapheme_indices(true)
        .next_back()
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// Whether `pos` sits on a grapheme boundary in `buf`.
///
/// A local check via [`GraphemeCursor`] — the cursor is given the whole buffer
/// as context, so it stays correct for context-sensitive sequences (combining
/// marks, ZWJ emoji) while only examining the bytes around `pos`, never
/// re-segmenting the buffer. Does not panic for off-boundary `pos`; it simply
/// reports `false`.
#[cfg(test)]
fn is_grapheme_boundary(buf: &str, pos: usize) -> bool {
    if pos == 0 || pos == buf.len() {
        return true;
    }
    if !buf.is_char_boundary(pos) {
        return false;
    }
    // Err is unreachable with the whole buffer as the single chunk.
    GraphemeCursor::new(pos, buf.len(), true)
        .is_boundary(buf, 0)
        .unwrap_or(false)
}

/// Snaps `pos` down to the start of the grapheme that contains it (the floor),
/// or returns `pos` unchanged when it already sits on a boundary.
///
/// Total and idempotent — a no-op on an already-aligned position, and never
/// panics (an off-boundary `pos` simply snaps to the enclosing grapheme start;
/// past-the-end clamps to `buf.len()`). Like [`is_grapheme_boundary`], a local
/// [`GraphemeCursor`] check with whole-buffer context — correct for
/// context-sensitive sequences without re-segmenting the buffer.
pub(crate) fn ensure_grapheme_boundary_prev(buf: &str, pos: usize) -> usize {
    // floor to a char boundary first so the cursor seed is valid
    let mut pos = pos.min(buf.len());
    while !buf.is_char_boundary(pos) {
        pos -= 1;
    }
    let mut cursor = GraphemeCursor::new(pos, buf.len(), true);
    match cursor.is_boundary(buf, 0) {
        Ok(true) => pos,
        _ => cursor.prev_boundary(buf, 0).ok().flatten().unwrap_or(0),
    }
}

/// Snaps `pos` up to the end of the grapheme that contains it (the ceiling),
/// or returns `pos` unchanged when it already sits on a boundary.
///
/// Total and idempotent — a no-op on an already-aligned position, and never
/// panics (an off-boundary `pos` simply snaps to the enclosing grapheme end;
/// past-the-end clamps to `buf.len()`). Like [`is_grapheme_boundary`], a local
/// [`GraphemeCursor`] check with whole-buffer context — correct for
/// context-sensitive sequences without re-segmenting the buffer.
pub(crate) fn ensure_grapheme_boundary_next(buf: &str, pos: usize) -> usize {
    // ceil to a char boundary first so the cursor seed is valid (`buf.len()`
    // is always one, so this terminates)
    let mut pos = pos.min(buf.len());
    while !buf.is_char_boundary(pos) {
        pos += 1;
    }
    let mut cursor = GraphemeCursor::new(pos, buf.len(), true);
    match cursor.is_boundary(buf, 0) {
        Ok(true) => pos,
        _ => cursor
            .next_boundary(buf, 0)
            .ok()
            .flatten()
            .unwrap_or(buf.len()),
    }
}

