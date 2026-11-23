//! Lexing each file's bracket structure once.
//!
//! The parser lexes the inside of every list, record, closure, block and subexpression again when
//! it parses that construct, and every one of those lexes scans the bytes of all the groups nested
//! inside it to find where they close. Without help, a byte would be scanned once per nesting
//! level. Instead, the first lex of a whole file records where each of its groups closes (a
//! [`BracketTable`]), and later lexes of parts of the file jump over nested groups instead of
//! scanning them again (see [`Brackets`](crate::lex::Brackets)).
//!
//! That is exact because, once the lexer is inside a group (not in a string or comment), where the
//! group closes depends only on the bytes from its opening bracket on, never on the lexer options a
//! caller passes (`additional_whitespace`, `special_tokens`, `skip_comment`): those only apply at
//! the top level of a token. So a close recorded by one lex holds for every other lex that reaches
//! the same opening bracket inside a group. The exception is signature lexing, where `<` and `>`
//! nest too, so it neither records nor jumps.

use crate::lex::{
    LexMode, LexState, NoBrackets, RecordBrackets, Token, lex_in_mode, lex_n_tokens_with, lex_with,
};
use console_protocol::{
    ParseError, Span,
    engine::{BracketTable, StateWorkingSet},
};
use std::cell::Cell;

/// The sizes of files that get a bracket table; other files are lexed by scanning every group.
/// Below the minimum, recording and looking up a table costs more than the
/// little nesting a small file can have saves (measured on the default config files, 400-500
/// bytes); above the maximum, the table (four bytes per source byte) gets too large.
const BRACKET_TABLE_LEN: std::ops::RangeInclusive<usize> = 1024..=16 * 1024 * 1024;

/// Lex a whole file (or part of one) as [`lex`](crate::lex::lex) does. When
/// [`StateWorkingSet::lex_once`] is on, this records the file's [`BracketTable`] while lexing it,
/// or jumps over nested groups if the table is already there.
///
/// A table serves only the parse of its file: the caller (`parse` or `parse_module_block`)
/// truncates [`StateWorkingSet::bracket_tables`] back to its length before this call when it is
/// done with the file. So the tables are those of the files being parsed, innermost last, and a
/// working set that parses many files one after another (console-lsp's workspace search) does not keep
/// a table for each of them.
///
/// A UTF-8 byte-order mark at the start of a whole file is skipped. Editors on Windows can save a
/// script with one, and it would otherwise be lexed as part of the first word. The file's contents
/// keep it, so spans into the file don't move.
pub(crate) fn lex_file(
    working_set: &mut StateWorkingSet,
    span: Span,
    additional_whitespace: &[u8],
    special_tokens: &[u8],
    skip_comment: bool,
) -> (Vec<Token>, Option<ParseError>) {
    const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";
    let is_whole_file = |working_set: &StateWorkingSet, span: Span| {
        working_set
            .find_file_by_span(span)
            .is_some_and(|file| file.covered_span == span)
    };

    let requested = span;
    let span = if working_set.get_span_contents(span).starts_with(UTF8_BOM)
        && is_whole_file(working_set, span)
    {
        Span::new(span.start + UTF8_BOM.len(), span.end)
    } else {
        span
    };

    let is_new_file = working_set.lex_once
        && find_bracket_table(working_set, span).is_none()
        && BRACKET_TABLE_LEN.contains(&span.end.saturating_sub(span.start))
        && is_whole_file(working_set, requested);
    if !is_new_file {
        return lex_span(
            working_set,
            span,
            additional_whitespace,
            special_tokens,
            skip_comment,
        );
    }

    let (lexed, table) = record_bracket_table(
        working_set.get_span_contents(span),
        span.start,
        additional_whitespace,
        special_tokens,
        skip_comment,
    );
    working_set.bracket_tables.push(table);
    lexed
}

/// Lex `input`, the contents of a file whose covered span starts at `start`, as
/// [`lex`](crate::lex::lex) does, and record where each of its groups closes.
fn record_bracket_table(
    input: &[u8],
    start: usize,
    additional_whitespace: &[u8],
    special_tokens: &[u8],
    skip_comment: bool,
) -> ((Vec<Token>, Option<ParseError>), BracketTable) {
    let mut close = vec![0; input.len()].into_boxed_slice();
    let lexed = lex_with(
        input,
        start,
        additional_whitespace,
        special_tokens,
        skip_comment,
        RecordBrackets {
            start,
            close: Cell::from_mut(&mut close[..]).as_slice_of_cells(),
        },
    );
    let table = BracketTable {
        covered_span: Span::new(start, start + input.len()),
        close,
    };
    (lexed, table)
}

/// Lex part of a file as [`lex`](crate::lex::lex) does, jumping over nested groups with the
/// file's [`BracketTable`] if [`lex_file`] recorded one.
pub(crate) fn lex_span(
    working_set: &StateWorkingSet,
    span: Span,
    additional_whitespace: &[u8],
    special_tokens: &[u8],
    skip_comment: bool,
) -> (Vec<Token>, Option<ParseError>) {
    lex_span_in_mode(
        working_set,
        span,
        additional_whitespace,
        special_tokens,
        skip_comment,
        LexMode::Code,
    )
}

/// Lex the inside of a list literal (`span`, without its brackets) as [`lex_span`] does, with the
/// items separated by whitespace, newlines or commas, and a `]` that closes no `[` an error (see
/// [`LexMode::ListItems`]).
pub(crate) fn lex_list_items(
    working_set: &StateWorkingSet,
    span: Span,
) -> (Vec<Token>, Option<ParseError>) {
    lex_span_in_mode(working_set, span, b"\n\r,", &[], true, LexMode::ListItems)
}

/// [`lex_span`], reading the span as `mode` says.
fn lex_span_in_mode(
    working_set: &StateWorkingSet,
    span: Span,
    additional_whitespace: &[u8],
    special_tokens: &[u8],
    skip_comment: bool,
    mode: LexMode,
) -> (Vec<Token>, Option<ParseError>) {
    let input = working_set.get_span_contents(span);
    match find_bracket_table(working_set, span) {
        Some(table) => lex_in_mode(
            input,
            span.start,
            additional_whitespace,
            special_tokens,
            skip_comment,
            mode,
            table,
        ),
        None => lex_in_mode(
            input,
            span.start,
            additional_whitespace,
            special_tokens,
            skip_comment,
            mode,
            NoBrackets,
        ),
    }
}

/// [`lex_n_tokens`](crate::lex::lex_n_tokens), jumping over nested groups with `table` if there is
/// one (from [`find_bracket_table`] for the span `state` lexes).
pub(crate) fn lex_n_tokens_in(
    state: &mut LexState,
    additional_whitespace: &[u8],
    special_tokens: &[u8],
    skip_comment: bool,
    max_tokens: usize,
    table: Option<&BracketTable>,
) -> isize {
    match table {
        Some(table) => lex_n_tokens_with(
            state,
            additional_whitespace,
            special_tokens,
            skip_comment,
            max_tokens,
            table,
        ),
        None => lex_n_tokens_with(
            state,
            additional_whitespace,
            special_tokens,
            skip_comment,
            max_tokens,
            NoBrackets,
        ),
    }
}

/// The recorded bracket table of the file containing `span`, if [`StateWorkingSet::lex_once`] is
/// on. The tables are those of the files being parsed, innermost last (see [`lex_file`]), and the
/// innermost file is the one being lexed, so the search starts there.
pub(crate) fn find_bracket_table<'a>(
    working_set: &'a StateWorkingSet,
    span: Span,
) -> Option<&'a BracketTable> {
    if !working_set.lex_once {
        return None;
    }
    working_set
        .bracket_tables
        .iter()
        .rev()
        .find(|table| table.covered_span.contains_span(span))
}

