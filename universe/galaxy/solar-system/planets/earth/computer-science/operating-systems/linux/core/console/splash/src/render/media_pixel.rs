//! `media_pixel` renderer — draws a [`PixelArtData`] grid as truecolor half-blocks (`▀`). One
//! terminal cell carries two stacked pixels: the upper pixel becomes the cell foreground, the
//! lower pixel becomes the cell background. Works on any terminal with 24-bit colour, with no
//! dependency on kitty / sixel / iTerm2 graphics protocols (that's `media_image`'s job).
//!
//! Pixels with `a = 0` resolve to the theme background, so transparent regions blend with
//! whatever the rest of the splash sits on.

use ratatui::{
    Frame,
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::Line,
    widgets::{Paragraph, Widget},
};

use crate::options::OptionSchema;
use crate::payload::{Body, PixelArtData, PixelColor};
use crate::theme::{self, ColorKey, Theme};

use super::{Registry, RenderOptions, Renderer, Shape};

const HALF_BLOCK: &str = "\u{2580}"; // ▀ upper half block.

const COLOR_KEYS: &[ColorKey] = &[theme::BG, theme::TEXT];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "align",
        type_hint: "\"left\" | \"center\" | \"right\"",
        required: false,
        default: Some("\"left\""),
        description: "Horizontal placement of the sprite within the available cell.",
    },
    OptionSchema {
        name: "max_width",
        type_hint: "cells (u16)",
        required: false,
        default: Some("area width"),
        description: "Upper bound on the rendered sprite width in cells. Overflow is clipped from the right.",
    },
    OptionSchema {
        name: "max_height",
        type_hint: "cells (u16)",
        required: false,
        default: Some("area height"),
        description: "Upper bound on the rendered sprite height in cells (each cell = 2 pixel rows). Overflow is clipped from the bottom.",
    },
];

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    pub max_width: Option<u16>,
    #[serde(default)]
    pub max_height: Option<u16>,
}

pub struct MediaPixelRenderer;

impl Renderer for MediaPixelRenderer {
    fn name(&self) -> &str {
        "media_pixel"
    }
    fn description(&self) -> &'static str {
        "Draws a small pixel-art sprite cell-by-cell with truecolor half-blocks. Works in any terminal with 24-bit colour. Two stacked pixels fit into one cell, so a 16x16 sprite renders as 16 columns × 8 rows. Transparent pixels (`a = 0`) blend with the theme background."
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::PixelArt]
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
        if let Body::PixelArt(d) = body {
            draw(frame, area, d, opts, theme);
        }
    }
    fn natural_height(
        &self,
        body: &Body,
        opts: &RenderOptions,
        _max_width: u16,
        _registry: &Registry,
    ) -> u16 {
        let Body::PixelArt(d) = body else {
            return 1;
        };
        let specific: Options = opts.parse_specific();
        let sprite_rows = pixel_grid_height(d);
        let capped = specific
            .max_height
            .map_or(sprite_rows, |m| m.min(sprite_rows));
        capped + label_height(d)
    }
}

fn draw(frame: &mut Frame, area: Rect, data: &PixelArtData, opts: &RenderOptions, theme: &Theme) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    if !rows_are_uniform(data) {
        frame.render_widget(
            Paragraph::new("[pixel_art: jagged rows]")
                .style(Style::default().fg(theme.status_error)),
            area,
        );
        return;
    }
    let target = compute_target(area, data, opts);
    let widget = SpriteWidget { data, theme };
    frame.render_widget(widget, target.sprite);
    if let (Some(label), Some(label_area)) = (data.label.as_deref(), target.label) {
        let p = Paragraph::new(Line::from(label)).style(Style::default().fg(theme.text));
        frame.render_widget(p, label_area);
    }
}

struct SpriteTarget {
    sprite: Rect,
    label: Option<Rect>,
}

fn compute_target(area: Rect, data: &PixelArtData, opts: &RenderOptions) -> SpriteTarget {
    let specific: Options = opts.parse_specific();
    let sprite_w_natural = pixel_grid_width(data);
    let sprite_h_natural = pixel_grid_height(data);
    let lbl_h = label_height(data).min(area.height);
    let available_h = area.height.saturating_sub(lbl_h);
    let capped_w = specific
        .max_width
        .unwrap_or(sprite_w_natural)
        .min(sprite_w_natural)
        .min(area.width);
    let capped_h = specific
        .max_height
        .unwrap_or(sprite_h_natural)
        .min(sprite_h_natural)
        .min(available_h);
    let x_offset = match opts.align.as_deref() {
        Some("center") => area.width.saturating_sub(capped_w) / 2,
        Some("right") => area.width.saturating_sub(capped_w),
        _ => 0,
    };
    let sprite = Rect {
        x: area.x + x_offset,
        y: area.y,
        width: capped_w,
        height: capped_h,
    };
    // Label spans the full row so multi-character captions don't truncate to the sprite's
    // single-cell width — callers passing a 1-cell sprite still want "hi" to fit.
    let label = (lbl_h > 0 && area.height >= sprite.height + lbl_h).then(|| Rect {
        x: area.x,
        y: area.y + sprite.height,
        width: area.width,
        height: lbl_h,
    });
    SpriteTarget { sprite, label }
}

fn pixel_grid_width(data: &PixelArtData) -> u16 {
    data.pixels.first().map(|row| row.len() as u16).unwrap_or(0)
}

fn pixel_grid_height(data: &PixelArtData) -> u16 {
    // Two pixel rows pack into one terminal cell (half-block), so divide by 2 rounding up.
    (data.pixels.len() as u16).div_ceil(2)
}

fn label_height(data: &PixelArtData) -> u16 {
    if data.label.as_deref().is_some_and(|s| !s.is_empty()) {
        1
    } else {
        0
    }
}

fn rows_are_uniform(data: &PixelArtData) -> bool {
    let Some(first) = data.pixels.first() else {
        return true;
    };
    let expected = first.len();
    data.pixels.iter().all(|row| row.len() == expected)
}

struct SpriteWidget<'a> {
    data: &'a PixelArtData,
    theme: &'a Theme,
}

impl<'a> Widget for SpriteWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        for cell_y in 0..area.height {
            let pixel_y_top = cell_y as usize * 2;
            let pixel_y_bot = pixel_y_top + 1;
            for cell_x in 0..area.width {
                let px = cell_x as usize;
                let top = pixel_at(self.data, pixel_y_top, px);
                let bot = pixel_at(self.data, pixel_y_bot, px);
                let fg = resolve(top, self.theme);
                let bg = resolve(bot, self.theme);
                let pos = (area.x + cell_x, area.y + cell_y);
                if let Some(c) = buf.cell_mut(pos) {
                    c.set_symbol(HALF_BLOCK)
                        .set_style(Style::default().fg(fg).bg(bg));
                }
            }
        }
    }
}

fn pixel_at(data: &PixelArtData, row: usize, col: usize) -> Option<PixelColor> {
    data.pixels.get(row).and_then(|r| r.get(col)).copied()
}

fn resolve(pixel: Option<PixelColor>, theme: &Theme) -> Color {
    match pixel {
        Some(p) if !p.is_transparent() => Color::Rgb(p.r, p.g, p.b),
        _ => theme.bg,
    }
}

