use crate::{
    core_editor::{
        graphemes::{next_grapheme_boundary, prev_grapheme_boundary},
        line, recohere,
        word::{self, categorize_char, CharClass},
        CaretGeometry, Cursor,
    },
    enums::{Direction, MotionTarget, WordEdge},
    FindStop, WordKind,
};

/// A resolved motion, as two byte positions:
/// - `head` — where the cursor lands (used by `Move`/`Extend`).
/// - `op_end` — the far edge an operator consumes (used by `Cut`/`Copy`/`Erase`).
///
/// They differ only for *inclusive* motions: a forward word-end (`e`) or find
/// (`f`/`t`) lands the cursor *on* a grapheme, but an operator eats it — so
/// `op_end` is one grapheme past `head`. For exclusive motions `op_end == head`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedMotion {
    pub(crate) head: usize,
    pub(crate) op_end: usize,
}

/// The span an operator (`Cut`/`Copy`/`Erase`) acts over: a [`Cursor`] from
/// `origin` to the motion's `op_end`. `start()..end()` is the byte range to
/// consume — inclusivity and direction are already baked into `op_end`, so the
/// operator never has to reconsider them.
///
/// Run through [`recohere`] before returning, so that range is always safe to
/// slice with. An operator consumes it *before* the commit boundary would
/// normalize anything, and a [`Position`](MotionTarget::Position) can name a
/// byte inside a grapheme; expanding outward consumes that grapheme whole
/// rather than truncating it.
pub(crate) fn operator_span(
    buf: &str,
    origin: usize,
    target: MotionTarget,
    geometry: CaretGeometry,
) -> Cursor {
    recohere(
        buf,
        Cursor::new(origin, resolve_motion(buf, origin, target, geometry).op_end),
    )
}

/// Resolve a public [`MotionTarget`] against `buf`, relative to `origin`.
///
/// Total over every variant — a target that cannot land anywhere (a `Find` that
/// misses, a `Line` past the first/last line) stays at `origin` (a no-op) rather
/// than panicking, so a target constructed from config or another mode can never
/// crash the editor. Context-aware (takes `buf`), so line/buffer edges resolve
/// correctly where a context-free conversion couldn't.
///
/// `geometry` is the caret geometry of the active mode ([`CaretGeometry::Block`]
/// for vi normal, [`CaretGeometry::Bar`] for emacs / vi insert). It selects the
/// forward word-end landing (see [`word::locate_word`]) and, for inclusive
/// motions, whether the operator eats the grapheme the caret lands on: a block
/// caret sits *on* the last grapheme so the operator reaches one past it, while a
/// bar caret already rests on the trailing boundary, so `op_end` is the head itself.
pub(crate) fn resolve_motion(
    buf: &str,
    origin: usize,
    target: MotionTarget,
    geometry: CaretGeometry,
) -> ResolvedMotion {
    let block = geometry.is_inclusive();
    let span = |head: usize, inclusive: bool| ResolvedMotion {
        head,
        op_end: if inclusive {
            next_grapheme_boundary(buf, head)
        } else {
            head
        },
    };
    match target {
        MotionTarget::Grapheme(Direction::Forward) => {
            span(next_grapheme_boundary(buf, origin), false)
        }
        MotionTarget::Grapheme(Direction::Backward) => {
            span(prev_grapheme_boundary(buf, origin), false)
        }
        MotionTarget::Word {
            kind,
            edge,
            direction,
        } => {
            let forward = direction == Direction::Forward;
            // `locate_word` gives the bar boundary. A block caret's forward
            // word-end (vi `e`) instead rests *on* the last grapheme — the same
            // boundary probed one cell ahead and rendered one cell back, so a
            // caret already on a word-end advances. This is where the caret
            // geometry axis lives; the word resolver stays purely structural.
            let on_grapheme = block && forward && edge == WordEdge::End;
            let head = if on_grapheme {
                let probe = next_grapheme_boundary(buf, origin);
                prev_grapheme_boundary(buf, word::locate_word(buf, probe, kind, edge, direction))
            } else {
                word::locate_word(buf, origin, kind, edge, direction)
            };
            // A forward word-end is operator-inclusive only in block mode: the
            // head sits on the last grapheme, so the operator reaches one past
            // it. In bar mode the head is already the trailing boundary.
            span(head, on_grapheme)
        }
        MotionTarget::Position(n) => span(n.min(buf.len()), false),
        MotionTarget::BufferEdge(Direction::Backward) => span(0, false),
        MotionTarget::BufferEdge(Direction::Forward) => span(buf.len(), false),
        // Exclusive, because covering the landing grapheme depends on which side
        // of the *anchor* it falls, which only `Cursor::put_cursor` can see (the
        // `Extend` dispatch in `Editor` routes this target there). Staying put
        // on a blank line is `resolve_motion` being total, not a special case.
        MotionTarget::LineStartNonBlank => {
            span(line::first_non_blank(buf, origin).unwrap_or(origin), false)
        }
        MotionTarget::LineEdge(Direction::Backward) => {
            span(line::start_of_line(buf, origin), false)
        }
        // CRLF-aware via `end_of_line`: `$` stops before the `\r` of a `\r\n`
        // terminator, matching `LineBuffer::find_current_line_end`.
        MotionTarget::LineEdge(Direction::Forward) => span(line::end_of_line(buf, origin), false),
        // The adjacent line (`j`/`k`). Lands on the *start* of the line below /
        // above; on the first/last line it stays put (so `dj`/`dk` there only
        // affect the current line). Operators snap the span to whole lines.
        MotionTarget::Line(Direction::Forward) => {
            let head = line::start_of_next_line(buf, origin).unwrap_or(origin);
            span(head, false)
        }
        MotionTarget::Line(Direction::Backward) => {
            let line_start = line::start_of_line(buf, origin);
            let head = if line_start == 0 {
                origin
            } else {
                line::start_of_line(buf, line_start - 1)
            };
            span(head, false)
        }
        // Character search (vi `f`/`t`/`F`/`T`). A miss stays at `origin` (a
        // no-op) rather than panicking. Forward find is inclusive (`df` eats the
        // target char); backward is exclusive.
        MotionTarget::Find {
            ch,
            direction,
            stop,
        } => {
            let hit = find_char(buf, origin, ch, direction, stop);
            let inclusive = hit.is_some() && direction == Direction::Forward;
            span(hit.unwrap_or(origin), inclusive)
        }
    }
}

pub(crate) fn resolve_selection(
    buf: &str,
    origin: usize,
    target: MotionTarget,
    geometry: CaretGeometry,
) -> Cursor {
    let m = resolve_motion(buf, origin, target, geometry);
    if m.head == origin {
        return Cursor::point(origin);
    }

    if m.head < origin {
        let anchor = if word_boundary_at_origin(buf, origin, target, Direction::Backward) {
            origin
        } else {
            next_grapheme_boundary(buf, origin)
        };
        return Cursor::new(anchor, m.head);
    }

    let next = next_grapheme_boundary(buf, origin);
    let on_boundary = word_boundary_at_origin(buf, origin, target, Direction::Forward);
    let (anchor, movement) = match target {
        MotionTarget::Word {
            edge: WordEdge::Start,
            ..
        } if on_boundary => (next, resolve_motion(buf, next, target, geometry)),
        MotionTarget::Word {
            edge: WordEdge::End,
            ..
        } if on_boundary => (next, m),
        _ => (origin, m),
    };

    Cursor::new(anchor, movement.op_end.max(anchor))
}

/// Helix's `reached_target` evaluated at the cursor itself.
/// Checks if `origin` already landed at a word boundary and thus
/// must hop over it to select the next word target.
/// Only meaningful for word targets; not valid for the emacs
/// `WordKind::Unicode`
fn word_boundary_at_origin(
    buf: &str,
    origin: usize,
    target: MotionTarget,
    direction: Direction,
) -> bool {
    let MotionTarget::Word { kind, edge, .. } = target else {
        return false;
    };

    // `a` is the grapheme under the cursor, `b` its neighbor in travel direction.
    let Some(a) = buf[origin..].chars().next() else {
        return false;
    };
    let b = match direction {
        Direction::Forward => buf[next_grapheme_boundary(buf, origin)..].chars().next(),
        Direction::Backward => buf[prev_grapheme_boundary(buf, origin)..].chars().next(),
    };

    let Some(b) = b else {
        return false;
    };

    let is_boundary = match kind {
        WordKind::Word => word::is_word_boundary(a, b),
        WordKind::LongWord => word::is_long_word_boundary(a, b),
        // The hop mirrors helix's `reached_target`, which is defined
        // for the char-class kinds; `Unicode` takes the default
        // anchor rule and is never produced by the helix machine.
        WordKind::Unicode => return false,
    };

    // Mirrors Helix's `reached_target`: a *start* must land on the word grapheme;
    // an *end* must leave one.
    let class_ok = match (edge, direction) {
        (WordEdge::Start, Direction::Forward) => {
            matches!(categorize_char(b), CharClass::Word | CharClass::Punctuation)
        }
        (WordEdge::End, Direction::Forward) | (WordEdge::Start, Direction::Backward) => {
            matches!(categorize_char(a), CharClass::Word | CharClass::Punctuation)
        }
        (WordEdge::End, Direction::Backward) => {
            matches!(categorize_char(b), CharClass::Word | CharClass::Punctuation)
        }
    };
    is_boundary && class_ok
}

/// Byte offset for a `f`/`t`/`F`/`T` search, or `None` if `ch` isn't found in
/// the search direction. `FindStop::Before` (`t`/`T`) backs off one grapheme.
fn find_char(
    buf: &str,
    origin: usize,
    ch: char,
    direction: Direction,
    stop: FindStop,
) -> Option<usize> {
    let hit = match direction {
        Direction::Forward => {
            let start = next_grapheme_boundary(buf, origin);
            buf[start..].find(ch).map(|rel| start + rel)
        }
        Direction::Backward => buf[..origin].rfind(ch),
    }?;

    Some(match (direction, stop) {
        (_, FindStop::On) => hit,
        (Direction::Forward, FindStop::Before) => prev_grapheme_boundary(buf, hit),
        (Direction::Backward, FindStop::Before) => next_grapheme_boundary(buf, hit),
    })
}

