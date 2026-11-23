use std::borrow::Cow;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Ensures input uses CRLF line endings.
///
/// Needed for correct output in raw mode.
/// Only replaces solitary LF with CRLF.
pub(crate) fn coerce_crlf(input: &str) -> Cow<'_, str> {
    let mut result = Cow::Borrowed(input);
    let mut cursor: usize = 0;
    for (idx, _) in input.match_indices('\n') {
        if !(idx > 0 && input.as_bytes().get(idx - 1) == Some(&b'\r')) {
            match &mut result {
                Cow::Borrowed(_) => {
                    // Best case 1 allocation, worst case 2 allocations.
                    // Avoid `AddAssign for Cow<str>` because its empty-LHS
                    // optimization may replace the preallocation.
                    let mut owned = String::with_capacity(input.len() + 1);
                    owned.push_str(&input[cursor..idx]);
                    owned.push_str("\r\n");
                    result = Cow::Owned(owned);
                }
                Cow::Owned(result) => {
                    result.push_str(&input[cursor..idx]);
                    result.push_str("\r\n");
                }
            }
            // Advance beyond the matched LF char (single byte)
            cursor = idx + 1;
        }
    }
    if let Cow::Owned(result) = &mut result {
        result.push_str(&input[cursor..input.len()]);
    }
    result
}

/// Returns string with the ANSI escape codes removed
///
/// If parsing fails silently returns the input string
pub(crate) fn strip_ansi(string: &str) -> String {
    String::from_utf8(strip_ansi_escapes::strip(string))
        .map_err(|_| ())
        .unwrap_or_else(|_| string.to_owned())
}

/// Shows control characters readline-style (`^[`, `^?`, `M-^[`) instead of
/// letting the terminal act on them, so untrusted text cannot erase or redraw
/// the line. `\n` and `\t` pass through.
pub(crate) fn escape_control(text: &str) -> Cow<'_, str> {
    escape_with(text, Escape::All)
}

/// [`escape_control`], but keeps well-formed SGR. SGR can still conceal text,
/// so this is for display-only text such as menu entries, never the buffer.
pub(crate) fn escape_control_keep_sgr(text: &str) -> Cow<'_, str> {
    escape_with(text, Escape::KeepSgr)
}

/// A menu as printed with coloring off. Suggestion text arrives escaped bar its
/// SGR (see `escape_suggestions`), so any `ESC` left is that SGR: whole,
/// uppercased into another command by a selected row (`ESC[5m` into `ESC[5M`,
/// delete lines), or cut short by truncation. The first two are dropped, a cut
/// one is shown. The menu's own `\r\n` passes.
pub(crate) fn plain_menu_text(text: &str) -> Cow<'_, str> {
    escape_with(text, Escape::DropSgr)
}

#[derive(Clone, Copy, PartialEq)]
enum Escape {
    /// Every control char but `\n` and `\t`.
    All,
    /// As `All`, keeping well-formed SGR.
    KeepSgr,
    /// Drops SGR in either case and shows only `ESC` and C1.
    DropSgr,
}

fn escape_with(text: &str, mode: Escape) -> Cow<'_, str> {
    // Allocated at the first char that is escaped or dropped.
    let mut result: Option<String> = None;
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        let sgr = match mode {
            Escape::All => None,
            Escape::KeepSgr => sgr_len(rest),
            Escape::DropSgr => sgr_len_ending(rest, b"mM"),
        };
        let len = sgr.unwrap_or(c.len_utf8());
        let escaped = sgr.is_none()
            && match mode {
                Escape::DropSgr => c == '\x1b' || ('\u{80}'..='\u{9f}').contains(&c),
                _ => c.is_control() && c != '\n' && c != '\t',
            };
        let dropped = sgr.is_some() && mode == Escape::DropSgr;

        if escaped || dropped {
            let out = result.get_or_insert_with(|| {
                let mut out = String::with_capacity(text.len() + 8);
                out.push_str(&text[..text.len() - rest.len()]);
                out
            });
            if escaped {
                push_caret(out, c);
            }
        } else if let Some(out) = &mut result {
            out.push_str(&rest[..len]);
        }
        rest = &rest[len..];
    }
    result.map_or(Cow::Borrowed(text), Cow::Owned)
}

/// Pushes control char `c` in caret runfile.
fn push_caret(out: &mut String, c: char) {
    // Every control char is below U+00A0, so it fits a byte.
    let byte = c as u32 as u8;
    match byte {
        0x7f => out.push_str("^?"),
        0x80.. => {
            out.push_str("M-^");
            out.push(char::from((byte - 0x80) ^ 0x40));
        }
        _ => {
            out.push('^');
            out.push(char::from(byte ^ 0x40));
        }
    }
}

/// Length of the SGR sequence (`ESC [`, digits and `;`, then `m`) that `text`
/// starts with, if it starts with one.
pub(crate) fn sgr_len(text: &str) -> Option<usize> {
    sgr_len_ending(text, b"m")
}

fn sgr_len_ending(text: &str, finals: &[u8]) -> Option<usize> {
    let args = text.strip_prefix("\x1b[")?;
    let end = args.find(|c: char| !(c.is_ascii_digit() || c == ';'))?;
    finals
        .contains(&args.as_bytes()[end])
        .then_some(2 + end + 1)
}

pub(crate) fn estimate_required_lines(input: &str, screen_width: u16) -> usize {
    input.lines().fold(0, |acc, line| {
        let wrap = estimate_single_line_wraps(line, screen_width);

        acc + 1 + wrap
    })
}

/// Reports the additional lines needed due to wrapping for the given line.
///
/// Callers pre-split, so `line` is not expected to contain a line break;
/// [`wrap_position`] would count one as a row.
///
/// If `line` fits in `terminal_columns` returns 0. A zero-width
/// `terminal_columns` can be reported by terminals mid-resize or when the
/// size is unknown; there is no layout to report, so [`wrap_position`]
/// answers `None` and this reports 0 (see #842).
///
/// Rows the line *wraps onto*, not rows the cursor reaches: a line filling
/// the width exactly wraps onto none and leaves the cursor pending on the
/// next. Only the callers placing a cursor count that row.
///
/// FIXME: the zero-column guard papers over a caller bug, it
/// doesn't solve it. `menu::list_menu::ListMenu::menu_required_lines`
/// passes `terminal_columns.saturating_sub(indicator_width + count_digits)`,
/// so on a terminal whose width is not greater than the indicator plus
/// the entry-index digits this function receives 0 and every entry is
/// reported as a single non-wrapping line. The real fix is to enforce a
/// minimum viable column budget in `menu_required_lines` (or to stop
/// subtracting the indicator width from the entry width). Tracked in
/// #842 / #428; remove this comment once the caller is fixed.
pub(crate) fn estimate_single_line_wraps(line: &str, terminal_columns: u16) -> usize {
    let Some((_, row)) = wrap_position([line], terminal_columns) else {
        return 0;
    };
    row as usize
}

/// Compute the line width for ANSI escaped text
pub(crate) fn line_width(line: &str) -> usize {
    strip_ansi(line).width()
}

/// Columns from one tab stop to the next, the terminal default.
const TAB_STOP: usize = 8;

/// `line` with each tab expanded to the spaces that reach the next tab stop,
/// counted from the start of the line. SGR takes no columns.
///
/// A terminal counts its stops from the left edge of each row, so a tab on a
/// wrapped row lands where no width estimate expects it, and the estimators
/// give it no width at all. Spaces wrap like any other text, so the rows
/// measured and the rows drawn stay the same.
pub(crate) fn expand_tabs(line: &str) -> Cow<'_, str> {
    if !line.contains('\t') {
        return Cow::Borrowed(line);
    }
    let mut expanded = String::with_capacity(line.len() + TAB_STOP);
    let mut column = 0;
    for (index, segment) in line.split('\t').enumerate() {
        if index > 0 {
            let spaces = TAB_STOP - column % TAB_STOP;
            expanded.extend(std::iter::repeat_n(' ', spaces));
            column += spaces;
        }
        expanded.push_str(segment);
        column += line_width(segment);
    }
    Cow::Owned(expanded)
}

/// Which row to move to when printing `pieces` lands the cursor on the
/// terminal's right margin, in the *deferred wrap* state.
///
/// A terminal does not move to the next row when a glyph lands in the final
/// column; it flags the cursor pending and only wraps once the next glyph
/// arrives. Terminals disagree about whether DECSC/DECRC carry that flag, so a
/// save taken there restores to either side of the margin and the caller has to
/// place the cursor absolutely instead. Returns how many rows past the start of
/// the run that row is, or `None` off the margin, where restoring is already
/// unambiguous. `pieces` are laid out end to end, since the walk is a fold and
/// never looks backwards.
pub(crate) fn deferred_wrap_row<'a>(
    pieces: impl IntoIterator<Item = &'a str>,
    terminal_columns: u16,
) -> Option<u16> {
    let end = wrap_position(pieces, terminal_columns)?;
    (end.0 >= terminal_columns).then(|| resolve_wrap(end, terminal_columns).1)
}

/// Where the cursor rests once a [`wrap_position`] landing is resolved: a run
/// ending on the margin left the wrap pending, and that belongs to the home
/// column of the next row (#1141), not to the column it filled.
///
/// The single owner of that rule, since every caller placing a cursor needs it
/// and one of them getting the saturation wrong is a panic, not a drawing bug.
pub(crate) fn resolve_wrap((col, row): (u16, u16), terminal_columns: u16) -> (u16, u16) {
    if col >= terminal_columns {
        (0, row.saturating_add(1))
    } else {
        (col, row)
    }
}

/// Where printing `pieces` leaves the cursor, walked a grapheme at a time
/// rather than divided out of the run's width. A double-width grapheme with
/// one column left cannot be split, so the terminal blanks that column and
/// wraps early: division reads a 42-column run on a 21-column terminal as two
/// exact rows ending on the margin, when the terminal needs three.
///
/// Returns `(column, rows past the start of the run)`. The column may equal
/// `terminal_columns`: the *deferred wrap* state, where the run has filled the
/// row but nothing has arrived to push it over. Resolving that is the caller's
/// job: [`resolve_wrap`] for the cursor-placing ones, and the estimators do
/// not count it at all.
pub(crate) fn wrap_position<'a>(
    pieces: impl IntoIterator<Item = &'a str>,
    terminal_columns: u16,
) -> Option<(u16, u16)> {
    if terminal_columns == 0 {
        return None;
    }
    let (mut row, mut col) = (0u16, 0u16);
    for piece in pieces {
        for grapheme in strip_ansi(piece).graphemes(true) {
            let (at, width) = advance_grapheme((row, col), grapheme, terminal_columns);
            (row, col) = at;
            col = col.saturating_add(width);
        }
    }
    Some((col, row))
}

/// One step of the [`wrap_position`] walk: where `grapheme` is drawn starting
/// from `(row, col)`, and how many columns it occupies.
///
/// The caller advances by adding the returned width, so the returned position
/// is where the grapheme *is*, not where the cursor ends up. Split out because
/// the reverse mapping in `Painter::screen_to_buffer_offset` has to walk the
/// same rule to turn a screen position back into a buffer offset, and the two
/// disagreeing is how a click lands on the wrong grapheme.
pub(crate) fn advance_grapheme(
    (row, col): (u16, u16),
    grapheme: &str,
    terminal_columns: u16,
) -> ((u16, u16), u16) {
    match grapheme {
        "\n" => ((row.saturating_add(1), 0), 0),
        "\r" => ((row, 0), 0),
        _ => {
            let width = grapheme.width() as u16;
            let (mut row, mut col) = (row, col);
            // The wrap this grapheme's arrival was deferred until.
            // Both checks can fire for one grapheme, so a single `||`
            // loses a row: a zero-width one trips only this.
            if col >= terminal_columns {
                (row, col) = (row.saturating_add(1), 0);
            }
            // No room for the whole grapheme: the trailing column stays
            // blank and the terminal wraps before drawing it. Only from a
            // column the run has already reached, since a grapheme wider
            // than the whole terminal has no row it would fit on and
            // wrapping it again would spend a row per glyph.
            if col > 0 && col.saturating_add(width) > terminal_columns {
                (row, col) = (row.saturating_add(1), 0);
            }
            ((row, col), width)
        }
    }
}

