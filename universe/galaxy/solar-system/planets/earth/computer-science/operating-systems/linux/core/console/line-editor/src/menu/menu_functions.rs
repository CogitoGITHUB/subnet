//! Collection of common functions that can be used to create menus
use std::borrow::Cow;
use std::ops::Range;

use nu_ansi_term::{ansi::RESET, Style};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::{
    menu::{InputMode, MenuSettings, OutputMode},
    painting::{escape_control, escape_control_keep_sgr, sgr_len, Painter},
    CompletionOrigin, CompletionResult, Editor, Partial, Span, Suggestion, Suggestions,
    UndoBehavior,
};

/// Appended to whatever `truncate_with_ansi` keeps, so callers must allow at
/// least its width.
const TRUNCATION_SUFFIX: &str = "...";

/// Index result obtained from parsing a string with an index marker
/// For example, the next string:
///     "this is an example :10"
///
/// Contains an index marker :10. This marker indicates that the user
/// may want to select the 10th element from a list
#[derive(Debug, PartialEq, Eq)]
pub struct ParseResult<'buffer> {
    /// Text before the marker
    pub remainder: &'buffer str,
    /// Parsed value from the marker
    pub index: Option<usize>,
    /// Marker representation as string
    pub marker: Option<&'buffer str>,
    /// Direction of the search based on the marker
    pub action: ParseAction,
    /// Prefix to search for
    pub prefix: Option<&'buffer str>,
}

/// Direction of the index found in the string
#[derive(Debug, PartialEq, Eq)]
pub enum ParseAction {
    /// Forward index search
    ForwardSearch,
    /// Backward index search
    BackwardSearch,
    /// Last token
    LastToken,
    /// Last executed command.
    LastCommand,
    /// Backward search for a prefix
    BackwardPrefixSearch,
}

/// Splits a string that contains a marker character
///
/// ## Example usage
/// ```
/// use reedline::menu_functions::{parse_selection_char, ParseAction, ParseResult};
///
/// let parsed = parse_selection_char("this is an example!10", '!');
///
/// assert_eq!(
///     parsed,
///     ParseResult {
///         remainder: "this is an example",
///         index: Some(10),
///         marker: Some("!10"),
///         action: ParseAction::ForwardSearch,
///         prefix: None,
///     }
/// )
///
/// ```
pub fn parse_selection_char(buffer: &str, marker: char) -> ParseResult<'_> {
    if buffer.is_empty() {
        return ParseResult {
            remainder: buffer,
            index: None,
            marker: None,
            action: ParseAction::ForwardSearch,
            prefix: None,
        };
    }

    let mut input = buffer.chars().peekable();

    let mut index = 0;
    while let Some(char) = input.next() {
        if char == marker {
            match input.peek() {
                #[cfg(feature = "bashisms")]
                Some(&x) if x == marker => {
                    return ParseResult {
                        remainder: &buffer[0..index],
                        index: Some(0),
                        marker: Some(&buffer[index..index + 2 * marker.len_utf8()]),
                        action: ParseAction::LastCommand,
                        prefix: None,
                    };
                }
                #[cfg(feature = "bashisms")]
                Some(&x) if x == '$' => {
                    return ParseResult {
                        remainder: &buffer[0..index],
                        index: Some(0),
                        marker: Some(&buffer[index..index + 2]),
                        action: ParseAction::LastToken,
                        prefix: None,
                    };
                }
                Some(&x) if x.is_ascii_digit() || x == '-' => {
                    let mut count: usize = 0;
                    let mut size: usize = marker.len_utf8();
                    let action = if x == '-' {
                        size += 1;
                        let _ = input.next();
                        ParseAction::BackwardSearch
                    } else {
                        ParseAction::ForwardSearch
                    };
                    while let Some(&c) = input.peek() {
                        if let Some(c) = c.to_digit(10) {
                            let _ = input.next();
                            count *= 10;
                            count += c as usize;
                            size += 1;
                        } else {
                            return ParseResult {
                                remainder: &buffer[0..index],
                                index: Some(count),
                                marker: Some(&buffer[index..index + size]),
                                action,
                                prefix: None,
                            };
                        }
                    }
                    return ParseResult {
                        remainder: &buffer[0..index],
                        index: Some(count),
                        marker: Some(&buffer[index..index + size]),
                        action,
                        prefix: None,
                    };
                }
                #[cfg(feature = "bashisms")]
                Some(&x) if x.is_ascii_alphabetic() => {
                    return ParseResult {
                        remainder: &buffer[0..index],
                        index: Some(0),
                        marker: Some(&buffer[index..index + marker.len_utf8()]),
                        action: ParseAction::BackwardPrefixSearch,
                        prefix: Some(&buffer[index + marker.len_utf8()..buffer.len()]),
                    };
                }
                None => {
                    return ParseResult {
                        remainder: &buffer[0..index],
                        index: Some(0),
                        marker: Some(&buffer[index..buffer.len()]),
                        action: ParseAction::ForwardSearch,
                        prefix: Some(&buffer[index..buffer.len()]),
                    };
                }
                _ => {}
            }
        }
        index += char.len_utf8();
    }

    ParseResult {
        remainder: buffer,
        index: None,
        marker: None,
        action: ParseAction::ForwardSearch,
        prefix: None,
    }
}

/// Find differing substring between two strings.
///
/// Skips the common prefix; if the rest of `old_string` is a suffix of what
/// remains, the difference is the text in between, otherwise everything after
/// the prefix. Returns the byte offset of the difference and the difference.
pub fn string_difference<'a>(new_string: &'a str, old_string: &str) -> (usize, &'a str) {
    let prefix = new_string
        .char_indices()
        .zip(old_string.chars())
        .take_while(|((_, n), o)| n == o)
        .map(|((i, c), _)| i + c.len_utf8())
        .last()
        .unwrap_or(0);

    // `prefix` is a char boundary of both strings by construction, so the
    // fallbacks are unreachable; they only keep the slicing panic-free.
    let (_, old_rest) = old_string.split_at_checked(prefix).unwrap_or_default();
    let (head, new_rest) = new_string.split_at_checked(prefix).unwrap_or_default();
    let diff = new_rest.strip_suffix(old_rest).unwrap_or(new_rest);

    (head.len(), diff)
}

/// Get the part of the line that should be given as input to the completer, as well
/// as the index of the end of that piece of text
///
/// `prev_input` is the text in the buffer when the menu was activated. Needed for `InputMode::Diff`.
pub fn completer_input(
    buffer: &str,
    insertion_point: usize,
    prev_input: Option<&str>,
    input_mode: InputMode,
) -> (String, usize) {
    match input_mode {
        InputMode::FullBuffer => (buffer.to_owned(), insertion_point),
        InputMode::CursorPrefix => {
            // TODO previously, all but the list menu replaced newlines with spaces here
            // The completers should be adapted to account for this, and tests need to be added
            (buffer[..insertion_point].to_owned(), insertion_point)
        }
        InputMode::Diff => {
            if let Some(old_string) = prev_input {
                let (start, input) = string_difference(buffer, old_string);
                if !input.is_empty() {
                    (input.to_owned(), start + input.len())
                } else {
                    (String::new(), insertion_point)
                }
            } else {
                (String::new(), insertion_point)
            }
        }
    }
}

/// Stashes the buffer on first call when in `InputMode::Diff` (so later calls can diff
/// against the original), then resolves the completer input via [`completer_input`].
///
/// Centralises the input-resolution boilerplate shared by all menu `update_values` impls.
pub fn resolve_completer_input(
    editor: &Editor,
    saved_input: &mut Option<String>,
    settings: &MenuSettings,
) -> (String, usize) {
    let mode = settings.effective_input_mode();
    if mode == InputMode::Diff && saved_input.is_none() {
        *saved_input = Some(editor.get_buffer().to_string());
    }
    completer_input(
        editor.get_buffer(),
        editor.completion_point(),
        saved_input.as_deref(),
        mode,
    )
}

/// Find the closest index less than or equal to the current index that's a
/// character boundary
///
/// This is already a method on `str`, but it's nightly-only. Once that becomes
/// stable, this function will be removed.
pub fn floor_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        s.len()
    } else {
        (1..=index)
            .rev()
            .find(|i| s.is_char_boundary(*i))
            .unwrap_or(0)
    }
}

/// Number of lines available for the menu body once the painter has scrolled
/// to make room for it
pub(crate) fn available_lines(painter: &Painter, min_rows: u16, max_lines: u16) -> u16 {
    let lines = painter.remaining_lines_after_scroll().min(max_lines);
    if lines == 0 {
        // Handle the case where a prompt uses the entire screen.
        // Drawing the menu has priority over the drawing the prompt.
        painter.remaining_lines().min(min_rows)
    } else {
        lines
    }
}

/// Scroll a fixed window so the selected row stays inside
pub(crate) fn scroll_offset(selected: u16, current: u16, window: u16) -> u16 {
    scroll_offset_usize(selected.into(), current.into(), window.into()).min(u16::MAX.into()) as u16
}

/// The same viewport calculation for menus whose choices outnumber terminal coordinates.
pub(crate) fn scroll_offset_usize(selected: usize, current: usize, window: usize) -> usize {
    if selected <= current {
        // Selection is above the visible area, scroll up
        selected
    } else if selected >= current.saturating_add(window) {
        // Selection is below the visible area, scroll down
        selected.saturating_sub(window) + 1
    } else {
        // Selection is within the visible area
        current
    }
}

/// Where a menu stands between its activation and a final answer about the
/// line on screen.
///
/// One value instead of three flags (`awaiting_results`, `provisional_results`,
/// `opening`), whose invariants — pending implies provisional, and only a final
/// answer may end the opening phase — lived in the update order rather than in
/// a type that cannot express their violation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum CompletionPhase {
    /// Activated with nothing asked about the line yet: no answer is
    /// outstanding and none is provisional.
    #[default]
    Unasked,
    /// Only provisional answers so far. The menu stays off screen, so one
    /// about to be closed by a lone suggestion never appears.
    Opening(AnswerKind),
    /// A final answer landed once. Later provisional answers keep the menu on
    /// screen: stale values beat blanking it on every keystroke.
    Open(AnswerKind),
}

/// How final the last answer was, as [`CompletionPhase`] remembers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AnswerKind {
    /// Computed for the line on screen.
    Fresh,
    /// Cached values for another line: shown, never decided on.
    Stale,
    /// Nothing yet; the background work is still running.
    Pending,
}

impl CompletionPhase {
    /// A new activation forgets the previous line's answer entirely.
    pub(crate) fn on_activate(&mut self) {
        *self = CompletionPhase::Unasked;
    }

    /// Fold the answer to an update into the phase.
    pub(crate) fn note(&mut self, result: &CompletionResult) {
        let kind = if result.is_pending() {
            AnswerKind::Pending
        } else if result.is_provisional() {
            AnswerKind::Stale
        } else {
            AnswerKind::Fresh
        };
        *self = match (*self, kind) {
            // Once open, no later answer can put the menu back off screen.
            (CompletionPhase::Open(_), kind) => CompletionPhase::Open(kind),
            // Only a final answer ends the opening phase.
            (_, AnswerKind::Fresh) => CompletionPhase::Open(AnswerKind::Fresh),
            (_, kind) => CompletionPhase::Opening(kind),
        };
    }

    /// Whether the values on display may still be superseded.
    pub(crate) fn provisional(self) -> bool {
        match self {
            CompletionPhase::Unasked => false,
            CompletionPhase::Opening(kind) | CompletionPhase::Open(kind) => {
                kind != AnswerKind::Fresh
            }
        }
    }

    /// Whether a background answer is still outstanding, so an empty menu
    /// means "still computing" rather than "no records".
    pub(crate) fn awaiting_results(self) -> bool {
        matches!(
            self,
            CompletionPhase::Opening(AnswerKind::Pending)
                | CompletionPhase::Open(AnswerKind::Pending)
        )
    }

    /// Whether no final answer about the line on screen has landed yet.
    pub(crate) fn awaiting_first_answer(self) -> bool {
        !matches!(self, CompletionPhase::Open(_))
    }
}

/// A menu's suggestions, tied to the buffer they were computed against.
/// Spans are validated before reaching the buffer.
#[derive(Default)]
pub struct CompletionDisplay {
    /// Menu suggestions (private; spans via accept/common_prefix)
    values: Suggestions,
    /// Display widths
    pub display_widths: Vec<usize>,
    /// Shortest of the strings the suggestions are based on.
    pub shortest_base_string: String,
    /// Width of the longest suggestion in `values`.
    pub longest_suggestion: usize,
    /// Completer-supplied partial (None = use derivation)
    partial: Option<Partial>,
    /// Buffer/cursor for span validation
    computed_for: CompletionOrigin,
}

impl CompletionDisplay {
    /// Build the display for a completion `result`, or `None` when there is
    /// nothing to adopt yet.
    pub fn from_result(
        result: CompletionResult,
        base_ranges: &[Range<usize>],
        editor: &Editor,
    ) -> Option<Self> {
        match result {
            CompletionResult::Pending => None,
            CompletionResult::Fresh {
                suggestions,
                partial,
            } => {
                // Stamped from live editor (Fresh result)
                let origin = CompletionOrigin::new(editor.get_buffer(), editor.insertion_point());
                Some(Self::new(suggestions, partial, base_ranges, origin))
            }
            CompletionResult::Stale {
                suggestions,
                origin,
                partial,
            } => Some(Self::new(suggestions, partial, base_ranges, origin)),
        }
    }

    /// Adopt suggestions and measure display metrics
    fn new(
        values: Suggestions,
        partial: Option<Partial>,
        base_ranges: &[Range<usize>],
        computed_for: CompletionOrigin,
    ) -> Self {
        let values = escape_suggestions(values);
        let display_widths: Vec<usize> = values
            .iter()
            .map(|suggestion| strip_ansi_escapes::strip_str(suggestion.display_value()).width())
            .collect();

        // Find the maximum width
        let longest_suggestion = display_widths.iter().copied().max().unwrap_or(0);

        // Shortest slice from stored buffer (may differ from editor if stale)
        let buffer = computed_for.buffer.as_str();
        let shortest_base_string = base_ranges
            .iter()
            .map(|range| {
                let end_index = floor_char_boundary(buffer, range.end);
                let start_index = floor_char_boundary(buffer, range.start).min(end_index);
                &buffer[start_index..end_index]
            })
            .min_by_key(|buffer_slice| buffer_slice.width())
            .map(String::from)
            .unwrap_or_default();

        Self {
            values,
            display_widths,
            shortest_base_string,
            longest_suggestion,
            partial,
            computed_for,
        }
    }

    /// Read-only: values, descriptions, widths (no spans)
    pub fn suggestions(&self) -> &[Suggestion] {
        &self.values
    }

    /// Whether spans match the current editor buffer
    fn is_current(&self, editor: &Editor) -> bool {
        self.computed_for
            .matches(editor.get_buffer(), editor.insertion_point())
    }

    /// Accept suggestion at index (no-op if stale or out of range)
    pub fn accept(&self, index: usize, editor: &mut Editor, output_mode: Option<OutputMode>) {
        if self.is_current(editor) {
            replace_in_buffer(self.values.get(index).cloned(), editor, output_mode);
        }
    }

    /// Apply partial completion (completer-supplied or derived). No-op and returns false when stale or unchanged.
    pub fn common_prefix(&self, editor: &mut Editor) -> bool {
        if !self.is_current(editor) {
            return false;
        }
        match &self.partial {
            Some(partial) => apply_partial(partial, editor),
            None => match derive_common_prefix(editor.get_buffer(), &self.values) {
                Some(partial) => apply_partial(&partial, editor),
                None => false,
            },
        }
    }
}

/// Escapes the control characters in what a menu shows of each suggestion,
/// keeping the SGR styling of a `display_override`, description and extra
/// (see [`escape_control_keep_sgr`]).
///
/// `value` is never touched: it is what accepting inserts, and the buffer
/// renders it escaped. Escaping shifts the characters `match_indices` points
/// at, so a rewritten entry drops them and its highlighting falls back to
/// finding the typed text. A clean list keeps its `Arc`.
pub(crate) fn escape_suggestions(values: Suggestions) -> Suggestions {
    let mut escaped: Option<Vec<Suggestion>> = None;
    for (i, suggestion) in values.iter().enumerate() {
        if let Some(shown) = escape_suggestion(suggestion) {
            escaped.get_or_insert_with(|| values.to_vec())[i] = shown;
        }
    }
    escaped.map_or(values, Into::into)
}

/// `suggestion` with its shown fields escaped, if any of them needed it.
fn escape_suggestion(suggestion: &Suggestion) -> Option<Suggestion> {
    fn changed(text: Cow<str>) -> Option<String> {
        match text {
            Cow::Owned(text) => Some(text),
            Cow::Borrowed(_) => None,
        }
    }
    fn escaped(text: &str) -> Option<String> {
        changed(escape_control_keep_sgr(text))
    }

    // Only a `display_override` is styled by the completer. Shown bare, `value`
    // is data, so its SGR is escaped too.
    let shown = match &suggestion.display_override {
        Some(display) => escaped(display),
        None => changed(escape_control(&suggestion.value)),
    };

    let mut out: Option<Suggestion> = None;
    if let Some(shown) = shown {
        let out = out.get_or_insert_with(|| suggestion.clone());
        out.display_override = Some(shown);
        out.match_indices = None;
    }
    if let Some(description) = suggestion.description.as_deref().and_then(escaped) {
        out.get_or_insert_with(|| suggestion.clone()).description = Some(description);
    }
    if let Some(extra) = &suggestion.extra {
        let shown: Vec<Cow<str>> = extra.iter().map(|e| escape_control_keep_sgr(e)).collect();
        if shown.iter().any(|e| matches!(e, Cow::Owned(_))) {
            out.get_or_insert_with(|| suggestion.clone()).extra =
                Some(shown.into_iter().map(Cow::into_owned).collect());
        }
    }
    out
}

/// Longest common prefix of what the suggestions would leave on the line.
///
/// Suggestions can replace different spans, such as `README.md` for `re` and
/// `foo rebase` for `foo re`. Each one is read as the text it leaves between the
/// earliest span start and the latest span end, so the prefix keeps only what every
/// suggestion agrees on: here `foo `, which doesn't extend `foo re`.
fn derive_common_prefix(line: &str, suggestions: &[Suggestion]) -> Option<Partial> {
    // Snapped to char boundaries, so slicing `line` with them can't panic
    let bounds = |span: Span| {
        let end = floor_char_boundary(line, span.end);
        (floor_char_boundary(line, span.start).min(end), end)
    };
    let (start, end) = suggestions
        .iter()
        .map(|s| bounds(s.span))
        .reduce(|(start, end), (s_start, s_end)| (start.min(s_start), end.max(s_end)))?;
    // Lazy `char` iterators, so only the first outcome is allocated
    let mut outcomes = suggestions.iter().map(|s| {
        let (s_start, s_end) = bounds(s.span);
        line[start..s_start]
            .chars()
            .chain(s.value.chars())
            .chain(line[s_end..end].chars())
    });

    let mut insert: String = outcomes.next()?.collect();
    for mut outcome in outcomes {
        let shared = insert
            .char_indices()
            .find_map(|(i, a)| (outcome.next() != Some(a)).then_some(i))
            .unwrap_or(insert.len());
        insert.truncate(shared);
    }

    let entered = &line[start..end];

    // Ensure prefix extends (not overwrites) user input
    let extends = !insert.is_empty()
        && insert != entered
        && insert
            .as_str()
            .to_lowercase()
            .contains(&entered.to_lowercase());
    extends.then_some(Partial {
        span: Span::new(start, end),
        insert,
    })
}

/// Apply partial to buffer. No-op if buffer already matches
fn apply_partial(partial: &Partial, editor: &mut Editor) -> bool {
    let buffer = editor.get_buffer();
    let end = floor_char_boundary(buffer, partial.span.end);
    let start = floor_char_boundary(buffer, partial.span.start).min(end);
    if buffer[start..end] == partial.insert {
        return false;
    }
    commit_buffer_replacement(editor, start, end, &partial.insert);
    true
}

/// Apply string replacement to line buffer
fn commit_buffer_replacement(editor: &mut Editor, start: usize, end: usize, replacement: &str) {
    // Use cursor head, clear selection
    let mut line_buffer = std::mem::take(editor.line_buffer_mut());
    let head = line_buffer.cursor().head();

    line_buffer.replace_range(start..end, replacement);
    line_buffer.clear_selection();
    line_buffer.set_insertion_point(
        head.saturating_add(replacement.len())
            .saturating_sub(end - start),
    );

    editor.set_line_buffer(line_buffer, UndoBehavior::CreateUndoPoint);
}

/// Helper to accept a completion suggestion and edit the buffer
pub fn replace_in_buffer(
    value: Option<Suggestion>,
    editor: &mut Editor,
    output_mode: Option<OutputMode>,
) {
    let Some(Suggestion {
        mut value,
        span,
        append_whitespace,
        ..
    }) = value
    else {
        return;
    };

    let buffer = editor.get_buffer();
    let (raw_start, raw_end) = match output_mode {
        Some(OutputMode::FullBuffer) => (0, buffer.len()),
        Some(OutputMode::ExtendToEnd) => (span.start, buffer.len()),
        Some(OutputMode::SuggestedSpan) | None => (span.start, span.end),
    };

    let end = floor_char_boundary(buffer, raw_end);
    let start = floor_char_boundary(buffer, raw_start).min(end);

    if append_whitespace {
        value.push(' ');
    }

    commit_buffer_replacement(editor, start, end, &value);
}

#[derive(Debug, PartialEq)]
struct AnsiSegment<'a> {
    /// One or more Select Graphic Rendition control sequences.
    /// Note: does NOT include the Control Sequence Introducer ('ESC [') at the beginning.
    escape: Option<&'a str>,
    text: &'a str,
}

struct AnsiEscape {
    /// Index where Control Sequence Introducer ('ESC [') starts
    csi_start: usize,
    /// Index where SGR arguments start. `None` if it ends in the reset attribute
    escape_start: Option<usize>,
    escape_end: usize,
    /// Whether the original sequence contained the reset attribute
    had_reset: bool,
}

const ANSI_SGR_START: &str = "\x1b[";

/// Parse ANSI sequences for setting display attributes in the given string.
///
/// Notes:
/// * The resulting `AnsiSegment`s don't include resets. A reset is implied before every segment.
/// * A single `AnsiSegment` can contain multiple consecutive control sequences.
///
/// Only parses Select Graphic Rendition control sequences, ignoring other ANSI sequencse.
/// Essentially just looks for 'ESC [' followed by /[0-9;]*m/.
fn parse_ansi<'a>(s: &'a str) -> Vec<AnsiSegment<'a>> {
    let mut segments = Vec::new();

    let find_escape_end = |sgr_args_start: usize| {
        let csi_start = sgr_args_start - ANSI_SGR_START.len();
        // `sgr_len` decides what counts as SGR, so the escaping that keeps
        // these sequences and the parsing here cannot disagree.
        let escape_end = csi_start + sgr_len(&s[csi_start..])?;
        let mut escape_start = sgr_args_start;
        let mut contains_reset = false;
        // Whether all digits of the current argument have been 0 so far (this
        // is true for empty arguments too). A 0 (or empty argument) represents
        // the reset attribute.
        let mut all_zeroes = true;
        for (i, c) in s[sgr_args_start..escape_end - 1].char_indices() {
            match c {
                '0' => {}
                ';' => {
                    if all_zeroes {
                        contains_reset = true;
                        escape_start = sgr_args_start + i + 1;
                    }
                    all_zeroes = true;
                }
                // `1` to `9`: `sgr_len` let nothing else through.
                _ => all_zeroes = false,
            }
        }
        Some(if all_zeroes {
            AnsiEscape {
                csi_start,
                escape_start: None,
                escape_end,
                had_reset: true,
            }
        } else {
            AnsiEscape {
                csi_start,
                escape_start: Some(escape_start),
                escape_end,
                had_reset: contains_reset,
            }
        })
    };

    let find_escape = |mut search_start: usize| {
        while let Some(i) = s[search_start..].find(ANSI_SGR_START) {
            if let Some(res) = find_escape_end(search_start + i + ANSI_SGR_START.len()) {
                return Some(res);
            } else {
                search_start = search_start + i + ANSI_SGR_START.len();
            }
        }
        None
    };

    let Some(AnsiEscape {
        csi_start,
        mut escape_start,
        mut escape_end,
        had_reset: _,
    }) = find_escape(0)
    else {
        return vec![AnsiSegment {
            escape: None,
            text: s,
        }];
    };
    // The unformatted text at the start, without any ANSI escapes before it
    segments.push(AnsiSegment {
        escape: None,
        text: &s[..csi_start],
    });

    loop {
        while s[escape_end..].starts_with(ANSI_SGR_START) {
            if let Some(AnsiEscape {
                csi_start: _,
                escape_start: next_start,
                escape_end: next_end,
                had_reset,
            }) = find_escape_end(escape_end + ANSI_SGR_START.len())
            {
                if had_reset || escape_start.is_none() {
                    escape_start = next_start;
                }
                escape_end = next_end;
            } else {
                break;
            }
        }

        let escape = escape_start.map(|start| &s[start..escape_end]);
        if let Some(AnsiEscape {
            csi_start,
            escape_start: new_start,
            escape_end: new_end,
            had_reset: _,
        }) = find_escape(escape_end)
        {
            segments.push(AnsiSegment {
                escape,
                text: &s[escape_end..csi_start],
            });
            escape_start = new_start;
            escape_end = new_end;
        } else {
            segments.push(AnsiSegment {
                escape,
                text: &s[escape_end..s.len()],
            });
            break;
        }
    }

    segments
}

/// Style a suggestion to be shown in a completer menu
///
/// * `match_indices` - Indices of graphemes (NOT bytes or chars) that matched the typed text
/// * `match_style` - Style to use for matched characters
pub fn style_suggestion(
    suggestion: &str,
    match_indices: &[usize],
    text_style: &Style,
    match_style: &Style,
    selected_style: Option<&Style>,
) -> String {
    let text_style_prefix = text_style.prefix().to_string();
    let match_style_prefix = match_style.prefix().to_string();
    let selected_prefix = selected_style
        .map(|s| s.prefix().to_string())
        .unwrap_or_default();
    let mut res = String::new();
    let mut offset = 0;
    let ansi_segments = parse_ansi(suggestion);
    for AnsiSegment { escape, text } in ansi_segments {
        if text.is_empty() {
            continue;
        }

        let graphemes = text.graphemes(true).collect::<Vec<_>>();
        let mut prev_matched = false;

        res.push_str(RESET);
        res.push_str(&text_style_prefix);
        res.push_str(&selected_prefix);
        if let Some(escape) = escape {
            res.push_str(ANSI_SGR_START);
            res.push_str(escape);
        }
        for (i, grapheme) in graphemes.iter().enumerate() {
            let is_match = match_indices.contains(&(i + offset));

            if is_match && !prev_matched {
                res.push_str(RESET);
                res.push_str(&text_style_prefix);
                res.push_str(&match_style_prefix);
                if let Some(escape) = escape {
                    res.push_str(ANSI_SGR_START);
                    res.push_str(escape);
                }
            } else if !is_match && prev_matched && i != 0 {
                res.push_str(RESET);
                res.push_str(&text_style_prefix);
                res.push_str(&selected_prefix);
                if let Some(escape) = escape {
                    res.push_str(ANSI_SGR_START);
                    res.push_str(escape);
                }
            }
            res.push_str(grapheme);
            prev_matched = is_match;
        }

        if prev_matched {
            res.push_str(RESET);
        }

        offset += graphemes.len();
    }

    res
}

/// If `match_indices` is given, then returns that. Otherwise, tries to find `typed_text`
/// inside `value`, then returns the indices for that substring.
pub fn get_match_indices<'a>(
    value: &str,
    match_indices: &'a Option<Vec<usize>>,
    typed_text: &str,
) -> Cow<'a, Vec<usize>> {
    if let Some(inds) = match_indices {
        Cow::Borrowed(inds)
    } else {
        let Some(match_pos) = value.to_lowercase().find(&typed_text.to_lowercase()) else {
            // Don't highlight anything if no match
            return Cow::Owned(vec![]);
        };
        let match_len = typed_text.graphemes(true).count();
        Cow::Owned((match_pos..match_pos + match_len).collect())
    }
}

/// Where `truncate_with_ansi` cuts: segments before `segment` are kept whole,
/// then the first `byte` bytes of `segment`, then the suffix.
struct Cut {
    segment: usize,
    /// Always a grapheme boundary within that segment's text.
    byte: usize,
}

/// How one segment sits against the width budget once everything before it
/// has been laid out.
enum Fit {
    /// The segment fits, and the suffix would still fit after it.
    WithDots,
    /// The segment fits, but the suffix after it would not: if anything later
    /// overflows, the cut has to land inside this segment.
    WithoutDots,
    /// The segment itself does not fit.
    Overflows,
}

fn fit(current_width: usize, segment_width: usize, suffix_width: usize, max_width: usize) -> Fit {
    let end = current_width + segment_width;
    if end > max_width {
        Fit::Overflows
    } else if end + suffix_width > max_width {
        Fit::WithoutDots
    } else {
        Fit::WithDots
    }
}

/// Byte length of the longest prefix of `text` whose display width is at most `budget`.
///
/// Walks graphemes so a wide or combining character is never split; a
/// grapheme that would overshoot the budget stops the walk.
fn prefix_len_within_width(text: &str, budget: usize) -> usize {
    let mut remaining = budget;
    let mut end = 0;
    for (start, grapheme) in text.grapheme_indices(true) {
        let width = grapheme.width();
        if width > remaining {
            break;
        }
        end = start + grapheme.len();
        remaining -= width;
    }
    end
}

/// Truncate a string with ANSI escapes to the given max width, which must be >=3.
///
/// If `s` is longer than `max_width`, the resulting string will end in "..."
/// and have width at most `max_width`.
pub(crate) fn truncate_with_ansi(s: &str, max_width: usize) -> Cow<'_, str> {
    let suffix_width = TRUNCATION_SUFFIX.width();
    let segments = parse_ansi(s);

    // The cut lands at the first place the suffix stops fitting: inside the
    // first `WithoutDots` segment after a clean boundary, or failing that
    // inside the segment that overflows. `pending` holds the former until we
    // know whether anything overflows at all; a `WithDots` segment resets it
    // since the suffix fits cleanly after that one again.
    let mut current_width = 0;
    let mut pending: Option<Cut> = None;
    let mut cut: Option<Cut> = None;
    for (i, segment) in segments.iter().enumerate() {
        let segment_width = segment.text.width();
        let budget = max_width
            .saturating_sub(current_width)
            .saturating_sub(suffix_width);
        match fit(current_width, segment_width, suffix_width, max_width) {
            Fit::WithDots => pending = None,
            Fit::WithoutDots => {
                if pending.is_none() {
                    pending = Some(Cut {
                        segment: i,
                        byte: prefix_len_within_width(segment.text, budget),
                    })
                }
            }
            Fit::Overflows => {
                cut = Some(pending.take().unwrap_or_else(|| Cut {
                    segment: i,
                    byte: prefix_len_within_width(segment.text, budget),
                }));
                break;
            }
        }
        current_width += segment_width;
    }

    let Some(Cut { segment, byte }) = cut else {
        return Cow::Borrowed(s);
    };
    // `segment` is an index the loop above produced, so both lookups hold; if
    // they ever did not, an untruncated string beats a panic while painting.
    let Some((keep, rest)) = segments.split_at_checked(segment) else {
        return Cow::Borrowed(s);
    };
    let Some(last) = rest.first() else {
        return Cow::Borrowed(s);
    };

    let mut res = String::new();
    for (i, segment) in keep.iter().enumerate() {
        if let Some(escape) = segment.escape {
            res.push_str(ANSI_SGR_START);
            res.push_str(escape);
        } else if i > 0 {
            // No need to put a RESET at the beginning of the string
            res.push_str(RESET);
        }
        res.push_str(segment.text);
    }
    // The cut segment's escape is emitted even when none of its text survives,
    // so the suffix carries its style.
    if let Some(escape) = last.escape {
        res.push_str(ANSI_SGR_START);
        res.push_str(escape);
    } else if segment > 0 {
        res.push_str(RESET);
    }
    res.push_str(last.text.get(..byte).unwrap_or(""));
    res.push_str(TRUNCATION_SUFFIX);
    Cow::Owned(res)
}

