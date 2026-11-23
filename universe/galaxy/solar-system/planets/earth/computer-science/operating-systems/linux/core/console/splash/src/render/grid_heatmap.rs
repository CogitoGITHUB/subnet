use ratatui::{Frame, buffer::Buffer, layout::Rect};

use crate::options::OptionSchema;
use crate::payload::{Body, HeatmapData};
use crate::theme::{self, ColorKey, Theme};

use super::{Registry, RenderOptions, Renderer, Shape};

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    pub cell_char: Option<String>,
}

const COLOR_KEYS: &[ColorKey] = &[theme::PALETTE_HEATMAP, theme::TEXT];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "cell_char",
        type_hint: "single character (e.g. \"■\", \"●\", \"▮\")",
        required: false,
        default: Some("\"■\""),
        description: "Glyph painted at every cell. Strings longer than one visible character are truncated to the first glyph.",
    },
    OptionSchema {
        name: "align",
        type_hint: "\"left\" | \"center\" | \"right\"",
        required: false,
        default: Some("\"left\""),
        description: "Horizontal alignment of the heatmap inside the slot when the grid is narrower than the available width.",
    },
];

/// 2D intensity grid — GitHub-style contribution graph and any other daily/periodic heatmap.
/// Each cell takes two terminal columns and one row so the grid reads as "dots of equal width"
/// despite the 1:2 character aspect ratio. Buckets each cell's value into one of 5 intensity
/// levels; level 0 is dim background, level 4 is full-saturation theme color. Thresholds come
/// from the payload when provided, otherwise fall back to an auto-quartile split of the data.
pub struct GridHeatmapRenderer;

const LEVELS: usize = 5;
/// Each cell takes 2 terminal columns so it reads as square-ish despite the 1:2 character
/// aspect. The first column paints the filled glyph; the second is a blank spacer so adjacent
/// cells don't merge into a single band. 1 row per cell — rows are already tall.
const CELL_W: u16 = 2;
const CELL_H: u16 = 1;
const CELL_GLYPH: &str = "■";

impl Renderer for GridHeatmapRenderer {
    fn name(&self) -> &str {
        "grid_heatmap"
    }
    fn description(&self) -> &'static str {
        "GitHub-contribution-style 2D grid of square-ish cells bucketed into five intensity levels from the heatmap palette, with optional column labels along the top. Right-aligns when the grid is wider than the slot so the most recent columns stay visible."
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::Heatmap]
    }
    fn color_keys(&self) -> &[ColorKey] {
        COLOR_KEYS
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn render(
        &self,
        frame: &mut Frame,
        area: Rect,
        body: &Body,
        opts: &RenderOptions,
        theme: &Theme,
        _registry: &Registry,
    ) {
        if let Body::Heatmap(d) = body {
            render_heatmap(frame, area, d, opts, theme);
        }
    }
}

fn render_heatmap(
    frame: &mut Frame,
    area: Rect,
    data: &HeatmapData,
    opts: &RenderOptions,
    theme: &Theme,
) {
    if data.cells.is_empty() || data.cells[0].is_empty() || area.width == 0 || area.height == 0 {
        return;
    }
    let thresholds = resolve_thresholds(data);
    // Reserve a top row for column labels only when we have labels AND enough vertical room
    // to draw them without eating into the actual grid.
    let label_row = has_col_labels(data) && area.height as usize > data.cells.len();
    let grid_area = if label_row {
        Rect {
            x: area.x,
            y: area.y + 1,
            width: area.width,
            height: area.height.saturating_sub(1),
        }
    } else {
        area
    };
    let (visible_cols, col_offset) = visible_col_window(data, grid_area);
    // Horizontal alignment: the grid itself is narrower than the slot when the terminal is
    // wide. Without alignment the grid hugs the left edge, which looks lopsided next to
    // centered widgets above/below it. `align` (left / center / right) shifts the grid's
    // x-origin inside `area`.
    let grid_px_width = (visible_cols as u16) * CELL_W;
    let x_shift = horizontal_shift(opts.align.as_deref(), grid_area.width, grid_px_width);
    let shifted = Rect {
        x: grid_area.x + x_shift,
        ..grid_area
    };
    let label_area = Rect {
        x: area.x + x_shift,
        ..area
    };
    if label_row {
        paint_col_labels(
            frame.buffer_mut(),
            label_area,
            data,
            visible_cols,
            col_offset,
            theme,
        );
    }
    let specific: Options = opts.parse_specific();
    paint_cells(
        frame.buffer_mut(),
        shifted,
        data,
        &thresholds,
        visible_cols,
        col_offset,
        theme,
        &cell_glyph(specific.cell_char.as_deref()),
    );
}

/// First `char` of the user-supplied glyph — we paint a single-width symbol into cell
/// column 0 of every heatmap cell, and column 1 stays empty as the spacer between
/// cells. A multi-char input like `"ab"` would bleed into that spacer and collide with
/// the 2-col-per-cell geometry, so we truncate to the leading character.
fn cell_glyph(opt: Option<&str>) -> String {
    opt.and_then(|s| s.chars().next())
        .map(|c| c.to_string())
        .unwrap_or_else(|| CELL_GLYPH.to_string())
}

fn horizontal_shift(align: Option<&str>, area_width: u16, content_width: u16) -> u16 {
    if content_width >= area_width {
        return 0;
    }
    let slack = area_width - content_width;
    match align {
        Some("center") => slack / 2,
        Some("right") => slack,
        _ => 0,
    }
}

fn has_col_labels(data: &HeatmapData) -> bool {
    data.col_labels
        .as_ref()
        .is_some_and(|v| v.iter().any(|s| !s.is_empty()))
}

fn visible_col_window(data: &HeatmapData, grid_area: Rect) -> (usize, usize) {
    let cols = data.cells[0].len();
    let max_visible_cols = (grid_area.width / CELL_W) as usize;
    let visible_cols = cols.min(max_visible_cols);
    // Right-align columns so "the most recent N weeks" is what survives when the slot is
    // narrower than the full year. Row alignment is top-anchored (weekday order is fixed).
    let col_offset = cols.saturating_sub(visible_cols);
    (visible_cols, col_offset)
}

#[allow(clippy::too_many_arguments)]
fn paint_cells(
    buf: &mut Buffer,
    area: Rect,
    data: &HeatmapData,
    thresholds: &[u32],
    visible_cols: usize,
    col_offset: usize,
    theme: &Theme,
    glyph: &str,
) {
    let rows = data.cells.len();
    let max_visible_rows = (area.height / CELL_H) as usize;
    let visible_rows = rows.min(max_visible_rows);
    for r in 0..visible_rows {
        for c in 0..visible_cols {
            let value = data.cells[r][c + col_offset];
            let level = bucket(value, thresholds);
            let color = theme.heatmap_level(level);
            let x = area.x + (c as u16) * CELL_W;
            let y = area.y + (r as u16) * CELL_H;
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(glyph);
                cell.set_fg(color);
            }
            // Column 1 of each cell stays as the terminal default so the grid reads as
            // distinct cells rather than a single continuous band.
        }
    }
}

fn paint_col_labels(
    buf: &mut Buffer,
    area: Rect,
    data: &HeatmapData,
    visible_cols: usize,
    col_offset: usize,
    theme: &Theme,
) {
    // Labels live on the top row of `area` (which includes the label row; `grid_area` starts
    // one row below). Write each non-empty label at its column's x-origin, extending as far
    // as there's room before the next label starts.
    let Some(labels) = &data.col_labels else {
        return;
    };
    let mut next_label_start: usize = visible_cols;
    // Scan from right to left so we know where the next label starts when we paint the
    // current one — lets us stop writing before colliding with the neighbor.
    for c in (0..visible_cols).rev() {
        let Some(label) = labels.get(c + col_offset) else {
            continue;
        };
        if label.is_empty() {
            continue;
        }
        let x0 = area.x + (c as u16) * CELL_W;
        let max_chars = (next_label_start - c) * CELL_W as usize;
        for (i, ch) in label.chars().take(max_chars).enumerate() {
            if let Some(cell) = buf.cell_mut((x0 + i as u16, area.y)) {
                cell.set_symbol(&ch.to_string());
                cell.set_fg(theme.text);
            }
        }
        next_label_start = c;
    }
}

fn bucket(value: u32, thresholds: &[u32]) -> usize {
    // thresholds has exactly 4 boundaries: level 0 = value == 0; level 1..=4 = above each
    // boundary. Keeping level 0 reserved for "no activity" matches the GitHub convention and
    // reads as visually distinct even when everything else is faint.
    if value == 0 {
        return 0;
    }
    let mut level = 1;
    for (i, &boundary) in thresholds.iter().enumerate() {
        if value >= boundary {
            level = (i + 1).min(LEVELS - 1) + 1;
        }
    }
    level.min(LEVELS - 1)
}

fn resolve_thresholds(data: &HeatmapData) -> Vec<u32> {
    if let Some(t) = &data.thresholds
        && !t.is_empty()
    {
        // User-provided: take up to 4 ascending boundaries; pad/truncate for deterministic
        // bucket count.
        let mut t = t.clone();
        t.sort_unstable();
        t.truncate(LEVELS - 1);
        while t.len() < LEVELS - 1 {
            t.push(t.last().copied().unwrap_or(1).saturating_add(1));
        }
        return t;
    }
    auto_quartile_thresholds(data)
}

fn auto_quartile_thresholds(data: &HeatmapData) -> Vec<u32> {
    let mut nonzero: Vec<u32> = data
        .cells
        .iter()
        .flat_map(|r| r.iter().copied())
        .filter(|v| *v > 0)
        .collect();
    if nonzero.is_empty() {
        return vec![1, 2, 3, 4];
    }
    nonzero.sort_unstable();
    let n = nonzero.len();
    // Four evenly-spaced percentiles (roughly 25/50/75/90) for the 4 boundaries between
    // levels 1-4. Using >75% as the top boundary keeps rare spikes visually distinct
    // without letting outliers dominate.
    let pick = |pct: usize| nonzero[((n * pct) / 100).min(n - 1)];
    let mut out = vec![pick(25), pick(50), pick(75), pick(90)];
    // Ensure strictly increasing so buckets are well-defined even on low-variance data.
    for i in 1..out.len() {
        if out[i] <= out[i - 1] {
            out[i] = out[i - 1] + 1;
        }
    }
    out
}

