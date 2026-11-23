//! Convert a ratatui `Buffer` into an inline HTML fragment for embedding in Markdown pages.
//!
//! Output: a single `<pre class="splash-snapshot">` with one `<span>` per contiguous run of
//! cells that share fg/bg/modifier. Default-styled cells emit plain text (no wrapping span) so
//! the rendered markup stays readable and small.

use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::style::{Color, Modifier, Style};
use unicode_width::UnicodeWidthStr;

/// Cell rows of inner padding inserted above the content band. Every snapshot lands at
/// `buffer_height + PADDING_TOP_ROWS` cells tall — uniform across presets, content
/// top-aligned, with a consistent breathing-room band on top regardless of how densely
/// packed the layout is.
const PADDING_TOP_ROWS: u16 = 2;

/// Render a buffer to inline HTML. Use this for non-dashboard previews (the per-fetcher /
/// per-renderer reference snippets in `gen_matrix`) — no padding, no top-alignment, just
/// emit the buffer as-is.
pub fn buffer_to_html(buf: &Buffer) -> String {
    let mut out = String::new();
    out.push_str("<pre class=\"splash-snapshot\">");
    let area = buf.area();
    for y in 0..area.height {
        emit_row(&mut out, buf, y, area.width);
        out.push('\n');
    }
    out.push_str("</pre>");
    out
}

/// Render a dashboard buffer with `PADDING_TOP_ROWS` of theme-bg padding above the
/// content, with the buffer's own rows top-aligned (any internal `fill` centring or
/// `length = N` top spacing collapses by rotating the leading blanks to the bottom of the
/// canvas). Caller passes the dashboard's theme bg so the synthesised padding rows match
/// whatever palette the dashboard rendered under.
pub fn buffer_to_html_dashboard(buf: &Buffer, theme_bg: Color) -> String {
    let mut out = String::new();
    out.push_str("<pre class=\"splash-snapshot\">");
    let area = buf.area();
    for _ in 0..PADDING_TOP_ROWS {
        emit_padding_row(&mut out, theme_bg, area.width);
        out.push('\n');
    }
    let order = top_aligned_row_order(buf, theme_bg);
    for y in order {
        emit_row(&mut out, buf, y, area.width);
        out.push('\n');
    }
    out.push_str("</pre>");
    out
}

/// Emit a `width`-cell row of pure theme-bg padding. Same markup shape as the runs that
/// `flush_run` produces for blank-bg cells inside the buffer, so the synthesised rows
/// blend with the rest of the snapshot's CSS / styling.
fn emit_padding_row(out: &mut String, bg: Color, width: u16) {
    let style = Style::default().bg(bg);
    let cells: Vec<String> = (0..width).map(|_| " ".to_string()).collect();
    flush_run(out, Some(&style), &cells);
}

/// Build a row order that top-aligns the buffer's content while preserving its total row
/// count. Three classes of blank rows get pushed to the bottom of the canvas:
///
/// - **Leading blanks** — `fill = 1` top padding or `length = N` top spacing collapses to
///   nothing so the first content row hugs the top of the snapshot.
/// - **Long internal gaps** — `fill = 1` rows that ratatui inflated to absorb leftover
///   canvas space (`project_codebase` puts two of them between sections, ten rows each).
///   Anything beyond `KEEP_INTERNAL_BLANKS` rows of a contiguous gap moves to the bottom,
///   so the section boundary stays visible without the dashboard reading as half-empty.
/// - **Trailing blanks** — already at the bottom; left in place.
///
/// Top-of-snapshot padding is synthesised separately by `buffer_to_html_dashboard` — this
/// function only reorders rows that already exist in the buffer.
fn top_aligned_row_order(buf: &Buffer, theme_bg: Color) -> Vec<u16> {
    let area = buf.area();
    let total = area.height;
    let mut kept: Vec<u16> = Vec::new();
    let mut moved: Vec<u16> = Vec::new();
    let mut y = 0u16;
    while y < total {
        if !is_row_blank(buf, y, area.width, theme_bg) {
            kept.push(y);
            y += 1;
            continue;
        }
        let run_end = (y..total)
            .find(|&i| !is_row_blank(buf, i, area.width, theme_bg))
            .unwrap_or(total);
        let leading = kept.is_empty();
        let trailing = run_end == total;
        let keep_count = if leading || trailing {
            0
        } else {
            (run_end - y).min(KEEP_INTERNAL_BLANKS)
        };
        for i in y..y + keep_count {
            kept.push(i);
        }
        for i in y + keep_count..run_end {
            moved.push(i);
        }
        y = run_end;
    }
    let mut order = kept;
    order.extend(moved);
    order
}

/// Cap on how many rows of any contiguous internal blank run survive in place. The runs we
/// want to preserve are the 1–2 row gaps designs use to separate sections (home_splash,
/// home_minimal); anything longer is `fill = 1` slack we'd rather collapse so the snapshot
/// reads as one cohesive band.
const KEEP_INTERNAL_BLANKS: u16 = 2;

fn is_row_blank(buf: &Buffer, y: u16, width: u16, canvas_bg: Color) -> bool {
    (0..width).all(|x| {
        let cell = &buf[(x, y)];
        cell.symbol().chars().all(|c| c == ' ')
            && cell.fg == Color::Reset
            && cell.bg == canvas_bg
            && cell.modifier.is_empty()
    })
}

fn emit_row(out: &mut String, buf: &Buffer, y: u16, width: u16) {
    let mut run_style: Option<Style> = None;
    let mut run_cells: Vec<String> = Vec::new();
    let mut x = 0u16;
    while x < width {
        let cell = buf[(x, y)].clone();
        // Cells flagged as Skip are placeholders for the visible region of an OSC 8 hyperlink
        // — the link's first cell carries the whole sequence; the trailing cells exist only
        // so terminal cursor math lines up. Drop them in HTML to avoid emitting stray spaces.
        if matches!(cell.diff_option, CellDiffOption::Skip) {
            x += 1;
            continue;
        }
        let style = cell_style(&cell);
        if let Some((url, visible)) = parse_osc8(cell.symbol()) {
            flush_run(out, run_style.as_ref(), &run_cells);
            run_cells.clear();
            run_style = None;
            emit_link(out, &style, &url, &visible);
            // The TestBackend stores the OSC 8 sequence in the first cell only; the trailing
            // cells the renderer flagged with `skip` aren't preserved across the diff/flush
            // (the backend's per-cell store has no `skip` field), so they keep whatever the
            // prior frame left there. Step past them ourselves using the visible text's
            // display width so we don't double-emit the underlying spaces / glyphs.
            let cells_consumed = UnicodeWidthStr::width(visible.as_str()).max(1) as u16;
            x = x.saturating_add(cells_consumed);
            continue;
        }
        if Some(style) != run_style {
            flush_run(out, run_style.as_ref(), &run_cells);
            run_cells.clear();
            run_style = Some(style);
        }
        run_cells.push(cell.symbol().to_string());
        x += 1;
    }
    flush_run(out, run_style.as_ref(), &run_cells);
}

/// Parse an OSC 8 hyperlink wrapper of the form `ESC ] 8 ; ; <url> ESC \ <visible> ESC ] 8 ; ; ESC \`
/// into its URL and visible-text components. The terminal renderer (`list_links`) embeds the
/// whole sequence into a single cell symbol; in HTML we need the parts so we can emit a real
/// `<a>` tag instead of leaking the escape codes as text.
fn parse_osc8(symbol: &str) -> Option<(String, String)> {
    const PREFIX: &str = "\x1b]8;;";
    const ST: &str = "\x1b\\";
    const CLOSE: &str = "\x1b]8;;\x1b\\";
    let s = symbol.strip_prefix(PREFIX)?;
    let url_end = s.find(ST)?;
    let url = &s[..url_end];
    let after_url = &s[url_end + ST.len()..];
    let close_pos = after_url.rfind(CLOSE)?;
    let visible = &after_url[..close_pos];
    Some((url.to_string(), visible.to_string()))
}

fn emit_link(out: &mut String, style: &Style, url: &str, visible: &str) {
    let styled = has_visible_style(style);
    if styled {
        out.push_str("<span style=\"");
        out.push_str(&style_css(style));
        out.push_str("\">");
    }
    out.push_str("<a href=\"");
    out.push_str(&escape_html(url));
    out.push_str("\" class=\"splash-link\">");
    for ch in visible.chars() {
        out.push_str("<span class=\"c\">");
        out.push_str(&escape_html(&ch.to_string()));
        out.push_str("</span>");
    }
    out.push_str("</a>");
    if styled {
        out.push_str("</span>");
    }
}

/// Skip `Color::Reset` (ratatui's "no explicit color" sentinel) so default cells produce plain
/// text rather than gratuitous `<span>` tags covering the whole buffer.
fn cell_style(cell: &ratatui::buffer::Cell) -> Style {
    let mut s = Style::default().add_modifier(cell.modifier);
    if cell.fg != Color::Reset {
        s = s.fg(cell.fg);
    }
    if cell.bg != Color::Reset {
        s = s.bg(cell.bg);
    }
    s
}

/// Emit one cell per `<span class="c">` so the browser lays out every terminal cell into a
/// fixed 1ch-wide box regardless of how the font renders the individual glyph. Without this,
/// block characters (▟▜▛ etc.) get slightly different advance widths per-glyph and column
/// alignment drifts. Runs of same-styled cells share an outer styled span.
fn flush_run(out: &mut String, style: Option<&Style>, cells: &[String]) {
    if cells.is_empty() {
        return;
    }
    match style {
        Some(s) if has_visible_style(s) => {
            out.push_str("<span style=\"");
            out.push_str(&style_css(s));
            out.push_str("\">");
            emit_cells(out, cells);
            out.push_str("</span>");
        }
        _ => emit_cells(out, cells),
    }
}

fn emit_cells(out: &mut String, cells: &[String]) {
    for cell in cells {
        out.push_str("<span class=\"c\">");
        out.push_str(&escape_html(cell));
        out.push_str("</span>");
    }
}

fn has_visible_style(s: &Style) -> bool {
    s.fg.is_some() || s.bg.is_some() || !s.add_modifier.is_empty()
}

fn style_css(s: &Style) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(c) = s.fg {
        parts.push(format!("color:{}", color_hex(c)));
    }
    if let Some(c) = s.bg {
        parts.push(format!("background:{}", color_hex(c)));
    }
    if s.add_modifier.contains(Modifier::BOLD) {
        parts.push("font-weight:bold".into());
    }
    if s.add_modifier.contains(Modifier::ITALIC) {
        parts.push("font-style:italic".into());
    }
    if s.add_modifier.contains(Modifier::UNDERLINED) {
        parts.push("text-decoration:underline".into());
    }
    if s.add_modifier.contains(Modifier::DIM) {
        parts.push("opacity:0.6".into());
    }
    parts.join(";")
}

/// ANSI-16 baseline; truecolor passes through. `Reset`/`Indexed` fall back to `currentColor`
/// so dark-mode CSS can flip the page foreground without the snapshots going black-on-black.
fn color_hex(c: Color) -> String {
    match c {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Black => "#000000".into(),
        Color::Red => "#aa0000".into(),
        Color::Green => "#00aa00".into(),
        Color::Yellow => "#aa5500".into(),
        Color::Blue => "#0000aa".into(),
        Color::Magenta => "#aa00aa".into(),
        Color::Cyan => "#00aaaa".into(),
        Color::Gray => "#aaaaaa".into(),
        Color::DarkGray => "#555555".into(),
        Color::LightRed => "#ff5555".into(),
        Color::LightGreen => "#55ff55".into(),
        Color::LightYellow => "#ffff55".into(),
        Color::LightBlue => "#5555ff".into(),
        Color::LightMagenta => "#ff55ff".into(),
        Color::LightCyan => "#55ffff".into(),
        Color::White => "#ffffff".into(),
        Color::Reset | Color::Indexed(_) => "currentColor".into(),
    }
}

fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

