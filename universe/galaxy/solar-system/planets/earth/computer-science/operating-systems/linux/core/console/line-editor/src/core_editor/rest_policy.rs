use std::cmp::Ordering;

use crate::core_editor::{
    graphemes::{
        ensure_grapheme_boundary_next, ensure_grapheme_boundary_prev, next_grapheme_boundary,
        prev_grapheme_boundary,
    },
    Cursor,
};

/// Where the cursor's head may *rest* after an edit, per editor paradigm.
///
/// Applied at the single commit boundary by [`commit`], layered on top of the
/// universal coherence pass ([`recohere`]). Each edit mode maps to one variant;
/// the core never inspects the mode, only the resulting policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RestPolicy {
    /// The head may rest anywhere between graphemes, including past the last one
    /// (at `buf.len()`). Emacs / Vi insert — the ordinary text caret.
    Between,
    /// The head may not rest past the last grapheme: a head at the end is pulled
    /// back onto the final grapheme. Vi normal mode — the cursor invariant
    /// enforced structurally here instead of by a maintenance command.
    OnGrapheme,
    /// The resting cursor always covers exactly one grapheme: a point is widened
    /// onto the grapheme to its right, or — at the buffer end, where there is
    /// none — onto the grapheme to its left. Mirrors Helix's `Range::min_width_1`.
    /// The policy for Vi visual mode, whose selection is always at least the
    /// grapheme it sits on. A line terminator is not a cell: the cursor covers
    /// the last grapheme of the line instead, like vim.
    Block,
    /// [`Block`](RestPolicy::Block) with the terminator counted as a cell, so
    /// `l` walks onto the `\n` and an operator there joins the lines. Helix
    /// normal / select, whose `Range` is over the newline like any other char.
    BlockOverNewline,
}

impl RestPolicy {
    /// Whether the resting cursor covers exactly one grapheme (either block
    /// policy).
    pub(crate) fn is_block(self) -> bool {
        match self {
            RestPolicy::Block => true,
            RestPolicy::BlockOverNewline => true,
            RestPolicy::Between | RestPolicy::OnGrapheme => false,
        }
    }

    /// Whether a line terminator is a cell the caret may rest on. Helix only;
    /// every other mode steps over or clamps before it.
    pub(crate) fn covers_terminator(self) -> bool {
        match self {
            RestPolicy::Between | RestPolicy::OnGrapheme | RestPolicy::Block => false,
            RestPolicy::BlockOverNewline => true,
        }
    }
}

/// Make a cursor *coherent* for `buf`: clamp both ends into `[0, buf.len()]` and
/// snap them to grapheme boundaries, expanding outward so whole graphemes stay
/// covered — the low end floors, the high end ceils, and a point floors while
/// staying a point.
///
/// Universal across edit modes and idempotent, so it is safe to run after any
/// edit. The mode-specific resting rule is layered on top by [`commit`].
pub(crate) fn recohere(buf: &str, c: Cursor) -> Cursor {
    let len = buf.len();

    let head = c.head().min(len);
    let anchor = c.anchor().min(len);

    let (anchor, head) = match anchor.cmp(&head) {
        // point: both floor to keep it a point
        Ordering::Equal => {
            let pos = ensure_grapheme_boundary_prev(buf, anchor);
            (pos, pos)
        }
        // forward: anchor (low) floors, head (high) ceils
        Ordering::Less => (
            ensure_grapheme_boundary_prev(buf, anchor),
            ensure_grapheme_boundary_next(buf, head),
        ),
        // backward: anchor (high) ceils, head (low) floors
        Ordering::Greater => (
            ensure_grapheme_boundary_next(buf, anchor),
            ensure_grapheme_boundary_prev(buf, head),
        ),
    };

    Cursor::new(anchor, head)
}

/// Normalize a cursor at the one commit boundary: first [`recohere`] it (universal
/// coherence), then apply the mode's [`RestPolicy`] resting rule.
///
/// Total and idempotent — `commit(buf, commit(buf, c, p), p) == commit(buf, c, p)`
/// for every input — so it can run after every command without the cursor drifting.
pub(crate) fn commit(buf: &str, c: Cursor, policy: RestPolicy) -> Cursor {
    let c = recohere(buf, c);
    let len = buf.len();
    match policy {
        RestPolicy::Between => c,
        RestPolicy::OnGrapheme => {
            // A *bare* cursor (an empty point) may not rest past the last grapheme
            // of its line: pull it back off the line terminator (or the buffer end)
            // onto that grapheme. Skip when the line is empty — the char before is
            // itself a terminator (or the buffer start), so column 0 is the only
            // cell and pulling back would cross into the previous line. A selection
            // may legitimately cover the final grapheme (head at the boundary,
            // caret one grapheme inward), so its head is left alone — inclusivity
            // is geometric now, not a query-time `+1`.
            let head = c.head();
            let past_last_grapheme = head == len || buf[head..].starts_with(['\n', '\r']);
            let line_has_grapheme = head > 0 && !buf[..head].ends_with(['\n', '\r']);
            if c.is_empty() && past_last_grapheme && line_has_grapheme {
                Cursor::point(prev_grapheme_boundary(buf, head))
            } else {
                c
            }
        }
        RestPolicy::Block => block(buf, c, false),
        RestPolicy::BlockOverNewline => block(buf, c, true),
    }
}

/// The block-caret resting rule, shared by [`RestPolicy::Block`] and the
/// helix-only `BlockOverNewline`.
fn block(buf: &str, c: Cursor, over_newline: bool) -> Cursor {
    // A block cursor always covers exactly one grapheme. Only a resting
    // *point* needs adjusting; an existing selection is already a range.
    if c.is_empty() {
        let head = c.head();
        let next = next_grapheme_boundary(buf, head);
        if next > head && (over_newline || !buf[head..].starts_with(['\n', '\r'])) {
            // widen forward onto the grapheme to the right: [head, next)
            c.move_head(next)
        } else if head > 0 && !buf[..head].ends_with(['\n', '\r']) {
            // at the buffer end there's nothing to the right, so cover the
            // last grapheme instead: [prev, head). Never crosses a line
            // boundary, in either policy — that would move the caret up a
            // line rather than widen it.
            Cursor::new(prev_grapheme_boundary(buf, head), head)
        } else {
            // empty buffer, or an empty line (no grapheme to cover without
            // crossing the newline): stay a zero-width point
            c
        }
    } else {
        c
    }
}

