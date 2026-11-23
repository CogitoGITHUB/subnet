use crate::core_editor::graphemes::{next_grapheme_boundary, prev_grapheme_boundary};

/// The direction a cursor extends in.
///
/// `Forward` when `head >= anchor`, `Backward` when `head < anchor`.
///
// Part of the selection vocabulary that lands incrementally: no caller until
// selection-aware motions (Vi visual / Helix) are wired through `Cursor`.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Head is at or after anchor.
    Forward,
    /// Head is before anchor.
    Backward,
}

/// Selection intent of a motion — whether it carries the anchor along or drops
/// it. Mirrors Helix's `Movement`; the readable face of `put_cursor`'s old
/// `extend: bool`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Movement {
    /// Collapse to a point at the target — an ordinary (non-selecting) motion.
    Move,
    /// Keep the anchor fixed and move only the head — extend the selection.
    Extend,
}

impl Movement {
    /// `Extend` when selecting, `Move` otherwise — the bridge from the boolean
    /// `select` flag the edit commands still carry.
    pub(crate) fn select(select: bool) -> Self {
        if select {
            Movement::Extend
        } else {
            Movement::Move
        }
    }
}

/// Where the caret sits relative to a grapheme — the geometry axis shared by
/// motion resolution ([`resolve_motion`](super::resolve_motion)) and selection
/// extension ([`Cursor::put_cursor`]).
///
/// - `Block` (vi normal / visual): the caret sits *on* a grapheme. A forward
///   word-end lands on the last char, and an inclusive motion's operator or
///   selection covers it.
/// - `Bar` (emacs / vi-insert): the caret sits *between* graphemes, on the
///   trailing boundary, so the same motion is exclusive.
///
/// The readable face of the `block`/`inclusive` boolean that used to thread
/// through the motion path under three different names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CaretGeometry {
    Block,
    Bar,
}

impl CaretGeometry {
    /// `true` for `Block` — the caret is on a grapheme, so inclusive motions
    /// cover it. The boolean the geometry-sensitive internals still branch on.
    pub(crate) fn is_inclusive(self) -> bool {
        matches!(self, CaretGeometry::Block)
    }
}

/// How a mode's `Extend` grows a selection — the selection-model axis, chosen
/// per edit mode by [`PromptEditMode::selection_extent`](crate::PromptEditMode).
/// Orthogonal to [`CaretGeometry`]: geometry is *where the caret rests*, extent
/// is *how far a motion drags the head*.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SelectionExtent {
    /// Vi visual: the block cursor sweeps the grapheme it lands *on*, so head
    /// covers the motion target: `vw` over `"foo bar"` selects `"foo b"`.
    CoverLanding,
    /// Helix: the selection is the gap-indexed operator span (`op_end`), so the
    /// head stops at the motion boundary: `w` selects `"foo "`, caret on the
    /// space.
    Span,
}

/// Keep the anchor's grapheme covered when a block selection reverses direction
/// across it (Helix `Range::put_cursor`): the anchor hops to the far edge of its
/// grapheme so it stays inside the range. A bar (exclusive) selection is half-open
/// `[min, max)`, so its anchor never moves.
fn flip_anchor(buf: &str, anchor: usize, old_head: usize, new_head: usize, block: bool) -> usize {
    if !block {
        anchor
    } else if old_head >= anchor && new_head < anchor {
        next_grapheme_boundary(buf, anchor)
    } else if old_head < anchor && new_head >= anchor {
        prev_grapheme_boundary(buf, anchor)
    } else {
        anchor
    }
}

/// A cursor as a (possibly empty) range over a buffer.
///
/// Uses gap indexing — `anchor` and `head` represent positions *between* bytes,
/// not bytes themselves. The range covered is left-inclusive and right-
/// exclusive, regardless of anchor/head ordering.
///
/// A "point" cursor (`anchor == head`) is the degenerate empty range and
/// behaves like a plain insertion point. Wider cursors model selections.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    /// The anchor of the cursor: the side that doesn't move when extending.
    anchor: usize,
    /// The head of the cursor: moved when extending or moving.
    head: usize,
}

impl Cursor {
    /// Construct a cursor with explicit anchor and head positions.
    pub fn new(anchor: usize, head: usize) -> Self {
        Self { anchor, head }
    }

    /// A zero-width cursor at `head`.
    pub fn point(head: usize) -> Self {
        Self::new(head, head)
    }

    /// The anchor position.
    pub fn anchor(&self) -> usize {
        self.anchor
    }

    /// The head position. This is what most callers think of as "the cursor".
    pub fn head(&self) -> usize {
        self.head
    }

    /// Start of the range.
    pub fn start(&self) -> usize {
        self.anchor.min(self.head)
    }

    /// End of the range.
    pub fn end(&self) -> usize {
        self.anchor.max(self.head)
    }

    /// `true` when anchor and head are at the same position.
    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

/// Range algebra that selection-aware motions need but the current point-cursor
/// callers don't yet exercise. Lands incrementally with its callers (Vi visual /
/// Helix selection); kept here, fully tested, so the primitive is complete.
#[allow(dead_code)]
impl Cursor {
    /// `(start, end)` as a `Range<usize>`, for consumers that want it.
    pub fn to_range(self) -> std::ops::Range<usize> {
        self.start()..self.end()
    }

    /// Total byte length of the range.
    pub fn len(&self) -> usize {
        self.end() - self.start()
    }

    /// `Forward` when `head >= anchor`, `Backward` otherwise.
    pub fn direction(&self) -> Direction {
        if self.head < self.anchor {
            Direction::Backward
        } else {
            Direction::Forward
        }
    }

    /// Swap anchor and head.
    pub fn flip(self) -> Self {
        Self {
            anchor: self.head,
            head: self.anchor,
        }
    }

    /// Return the cursor if it already points in `direction`, otherwise flip it.
    pub fn with_direction(self, direction: Direction) -> Self {
        if self.direction() == direction {
            self
        } else {
            self.flip()
        }
    }

    /// Move head to `new_head`, preserving anchor. Used for selecting motions.
    pub fn move_head(self, new_head: usize) -> Self {
        Self {
            anchor: self.anchor,
            head: new_head,
        }
    }

    /// Collapse to a point at `head`. Used for non-selecting motions.
    pub fn collapse_to(self, head: usize) -> Self {
        Self::point(head)
    }

    /// Collapse to a point at the current head position.
    pub fn collapse(self) -> Self {
        Self::point(self.head)
    }

    /// Grow the range to cover at least `[from, to]`, preserving anchor/head
    /// ordering.
    ///
    /// If the cursor is currently `Forward`, the anchor can only move left and
    /// the head can only move right. If `Backward`, the roles are inverted.
    pub fn extend(self, from: usize, to: usize) -> Self {
        debug_assert!(from <= to);
        if self.anchor <= self.head {
            Self {
                anchor: self.anchor.min(from),
                head: self.head.max(to),
            }
        } else {
            Self {
                anchor: self.anchor.max(to),
                head: self.head.min(from),
            }
        }
    }

    /// `true` if `pos` lies inside the range (left-inclusive, right-exclusive).
    pub fn contains(&self, pos: usize) -> bool {
        self.start() <= pos && pos < self.end()
    }

    /// The visible caret position
    pub fn caret(&self, buf: &str) -> usize {
        if self.head > self.anchor {
            prev_grapheme_boundary(buf, self.head)
        } else {
            self.head
        }
    }

    /// Move the caret to land on the grapheme at byte `target`, returning the new
    /// cursor. Mirrors Helix's `Range::put_cursor`.
    ///
    /// [`Movement::Move`] collapses to a point at `target` (the commit boundary's
    /// `Block` policy then widens it to a 1-grapheme block). [`Movement::Extend`]
    /// keeps the anchor side — but **flips the anchor onto the other edge of its
    /// grapheme when the selection changes direction**, so the grapheme the
    /// selection started on stays covered (see the type docs / convergence note).
    ///
    /// `geometry` is the per-mode selection geometry: when extending *forward*, a
    /// `Block` (vi-normal) cursor places the head one grapheme past `target` so
    /// `target`'s grapheme is covered; a `Bar` (emacs / Between) cursor places the
    /// head exactly on `target`. It only affects the forward head — a backward
    /// range already covers `target` at its low end, and a collapse ignores it.
    pub fn put_cursor(
        self,
        buf: &str,
        target: usize,
        movement: Movement,
        geometry: CaretGeometry,
    ) -> Self {
        if movement == Movement::Move {
            return Self::point(target);
        }
        let inclusive = geometry.is_inclusive();

        // Flip the anchor onto the far edge of its grapheme when direction
        // changes, so the grapheme the selection started on stays covered.
        let anchor = flip_anchor(buf, self.anchor, self.head, target, inclusive);

        // Place the head so `caret()` lands back on `target`'s grapheme: forward
        // *and inclusive* → head on the far edge; otherwise → head *is* `target`.
        // This forward widening is the `CoverLanding` selection model.
        let head = if anchor <= target && inclusive {
            next_grapheme_boundary(buf, target)
        } else {
            target
        };

        Self::new(anchor, head)
    }

    /// Grow a selection under the [`SelectionExtent::Span`] model: place the head
    /// at the motion's gap-indexed end (`op_end`, with per-motion inclusivity
    /// already baked in by [`resolve_motion`](super::resolve_motion)), with *no*
    /// vi-visual widening, while keeping the anchor grapheme covered through a
    /// block reversal via the same [`flip_anchor`] as [`Cursor::put_cursor`].
    pub(crate) fn extend_span(self, buf: &str, op_end: usize, geometry: CaretGeometry) -> Self {
        let anchor = flip_anchor(buf, self.anchor, self.head, op_end, geometry.is_inclusive());
        Self::new(anchor, op_end)
    }
}

impl Default for Cursor {
    fn default() -> Self {
        Self::point(0)
    }
}

