use nu_ansi_term::Style;

use crate::core_editor::{ensure_grapheme_boundary_next, ensure_grapheme_boundary_prev};
use crate::terminal_extensions::semantic_prompt::{PromptKind, SemanticPromptMarkers};
use crate::Prompt;

use super::utils::{escape_control, strip_ansi};

/// A representation of a buffer with styling, used for doing syntax highlighting
#[derive(Clone)]
pub struct StyledText {
    /// The component, styled parts of the text
    pub buffer: Vec<(Style, String)>,
}

impl Default for StyledText {
    fn default() -> Self {
        Self::new()
    }
}

impl StyledText {
    /// Construct a new `StyledText`
    pub const fn new() -> Self {
        Self { buffer: vec![] }
    }

    /// Add a new styled string to the buffer
    pub fn push(&mut self, styled_string: (Style, String)) {
        self.buffer.push(styled_string);
    }

    /// Restyle the byte range `from..to` of the whole buffer.
    ///
    /// The bounds are snapped outward to grapheme boundaries, so a range that
    /// lands inside a multi-byte character or a combining sequence styles the
    /// whole grapheme instead of splitting it. Pairs the range does not touch
    /// are kept whole; no zero-length pair is ever produced.
    pub fn style_range(&mut self, from: usize, to: usize, new_style: Style) {
        let (from, to) = (from.min(to), from.max(to));
        let mut rebuilt = Vec::with_capacity(self.buffer.len() + 2);
        let mut start = 0;
        for (style, text) in std::mem::take(&mut self.buffer) {
            let end = start + text.len();
            // Where the range meets this pair, in the pair's own byte offsets.
            let lo = ensure_grapheme_boundary_prev(&text, from.clamp(start, end) - start);
            let hi = ensure_grapheme_boundary_next(&text, to.clamp(start, end) - start);

            match (text.get(..lo), text.get(lo..hi), text.get(hi..)) {
                (Some(before), Some(styled), Some(after)) if lo < hi => {
                    let pieces = [(style, before), (new_style, styled), (style, after)];
                    rebuilt.extend(
                        pieces
                            .into_iter()
                            .filter(|(_, piece)| !piece.is_empty())
                            .map(|(style, piece)| (style, piece.to_string())),
                    );
                }
                // The range does not touch this pair, or (after snapping, which
                // makes this unreachable) a bound is not a char boundary: keep
                // the pair whole either way.
                _ => rebuilt.push((style, text)),
            }
            start = end;
        }
        self.buffer = rebuilt;
    }

    /// Render the styled string. We use the insertion point to render around so that
    /// we can properly write out the styled string to the screen and find the correct
    /// place to put the cursor. This assumes a logic that prints the first part of the
    /// string, saves the cursor position, prints the second half, and then restores
    /// the cursor position
    ///
    /// Also inserts the multiline continuation prompt with optional semantic markers
    pub fn render_around_insertion_point(
        &self,
        insertion_point: usize,
        prompt: &dyn Prompt,
        use_ansi_coloring: bool,
        semantic_markers: Option<&dyn SemanticPromptMarkers>,
    ) -> (String, String) {
        let mut current_idx = 0;
        let mut left_string = String::new();
        let mut right_string = String::new();

        let multiline_prompt = prompt.render_prompt_multiline_indicator();
        let prompt_style = Style::new().fg(prompt.get_prompt_multiline_color());

        for pair in &self.buffer {
            if current_idx >= insertion_point {
                right_string.push_str(&render_as_string(
                    pair,
                    &prompt_style,
                    &multiline_prompt,
                    semantic_markers,
                ));
            } else if pair.1.len() + current_idx <= insertion_point {
                left_string.push_str(&render_as_string(
                    pair,
                    &prompt_style,
                    &multiline_prompt,
                    semantic_markers,
                ));
            } else if pair.1.len() + current_idx > insertion_point {
                let offset = insertion_point - current_idx;

                let left_side = pair.1[..offset].to_string();
                let right_side = pair.1[offset..].to_string();

                left_string.push_str(&render_as_string(
                    &(pair.0, left_side),
                    &prompt_style,
                    &multiline_prompt,
                    semantic_markers,
                ));
                right_string.push_str(&render_as_string(
                    &(pair.0, right_side),
                    &prompt_style,
                    &multiline_prompt,
                    semantic_markers,
                ));
            }
            current_idx += pair.1.len();
        }

        if use_ansi_coloring {
            (left_string, right_string)
        } else {
            (strip_ansi(&left_string), strip_ansi(&right_string))
        }
    }

    /// Apply the ANSI style formatting to the full string.
    pub fn render_simple(&self) -> String {
        self.buffer
            .iter()
            .map(|(style, text)| style.paint(escape_control(text)).to_string())
            .collect()
    }

    /// Get the unformatted text as a single continuous string.
    pub fn raw_string(&self) -> String {
        self.buffer.iter().map(|(_, str)| str.as_str()).collect()
    }
}

/// Whether `style` would make a blank cell look different from unstyled text.
///
/// A space has no glyph, so a foreground colour and every weight-ish attribute
/// (bold, dim, italic, blink) have nothing to act on. A background paints the
/// cell, reversed colours paint the foreground *as* the background, and a rule
/// through or under the cell is drawn regardless of what is in it.
///
/// `Style::reverse()` is the one that matters in practice: it is the natural way
/// to write a selection style and it leaves `background` at `None`, so a check
/// for a background alone silently excludes it.
fn shows_on_a_blank_cell(style: &Style) -> bool {
    style.background.is_some() || style.is_reverse || style.is_underline || style.is_strikethrough
}

/// What follows each newline of the buffer: the multiline indicator, wrapped in
/// secondary prompt markers when the terminal gets them
/// (`\n` + `A;k=s` + indicator + `B`).
fn format_continuation(
    multiline_prompt: &str,
    semantic_markers: Option<&dyn SemanticPromptMarkers>,
) -> String {
    match semantic_markers {
        Some(markers) => format!(
            "\n{}{}{}",
            markers.prompt_start(PromptKind::Secondary),
            multiline_prompt,
            markers.command_input_start()
        ),
        None => format!("\n{multiline_prompt}"),
    }
}

/// Gives each line a hint continues onto the multiline indicator the buffer's
/// lines get (#530), so a multi-line hint lines up with the text it completes.
///
/// The indicator goes in unstyled, inside whatever style the hinter has open
/// at the newline, so it is drawn like the hint and marks those rows as not
/// typed yet, rather than in the prompt's multiline color.
pub(crate) fn continue_hint_lines(
    hint: String,
    prompt: &dyn Prompt,
    use_ansi_coloring: bool,
    semantic_markers: Option<&dyn SemanticPromptMarkers>,
) -> String {
    if !hint.contains('\n') {
        return hint;
    }

    // Stripped, since a reset in a styled indicator would end the hint's style
    // for the rest of the row.
    let continuation = format_continuation(
        &strip_ansi(&prompt.render_prompt_multiline_indicator()),
        semantic_markers,
    );
    // Without coloring the buffer drops its markers too, see
    // `render_around_insertion_point`.
    let continuation = if use_ansi_coloring {
        continuation
    } else {
        strip_ansi(&continuation)
    };
    hint.replace('\n', &continuation)
}

fn render_as_string(
    renderable: &(Style, String),
    prompt_style: &Style,
    multiline_prompt: &str,
    semantic_markers: Option<&dyn SemanticPromptMarkers>,
) -> String {
    let mut rendered = String::new();
    let formatted_multiline_prompt = format_continuation(multiline_prompt, semantic_markers);

    // `split` consumes the `\n`, so a terminator has no glyph left to carry the
    // style and a selection covering it paints nothing. Every piece but the last
    // had one consumed after it: give those a cell, and the `\n` itself comes
    // back with the multiline prompt.
    //
    // Only when the style would actually show on a blank cell, or a highlighter
    // that emits the buffer as one plain chunk would gain a stray space per line
    // in every mode. The `\r` of a CRLF is the terminator's own, thus the cell
    // replaces it; a trailing `\r` on the *last* piece is buffer content, and
    // shows as `^M` with every other control character.
    //
    // The cell is a real column, so `required_lines` and `cursor_pos` count it.
    // That shows up only on a selected line which exactly fills the terminal.
    let mut lines = renderable.1.split('\n').peekable();
    while let Some(line) = lines.next() {
        let terminated = lines.peek().is_some();
        let line = if terminated {
            line.strip_suffix('\r').unwrap_or(line)
        } else {
            line
        };
        let line = escape_control(line);

        // One `paint` for line and cell together: two would wrap each in its own
        // escape pair, and the rendered string is asserted verbatim.
        if terminated && shows_on_a_blank_cell(&renderable.0) {
            rendered.push_str(&renderable.0.paint(format!("{line} ")).to_string());
        } else {
            rendered.push_str(&renderable.0.paint(line).to_string());
        }

        if terminated {
            rendered.push_str(&prompt_style.paint(&formatted_multiline_prompt).to_string());
        }
    }
    rendered
}

