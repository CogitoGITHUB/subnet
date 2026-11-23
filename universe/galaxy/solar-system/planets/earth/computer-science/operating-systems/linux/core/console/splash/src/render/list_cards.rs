//! `list_cards` — `ImageLinkedList` rendered as card rows: a thumbnail cell on the left,
//! title (+ optional subtitle) on the right, with rows that carry a `url` wrapped in OSC 8
//! hyperlinks so modern terminals make them clickable. Rows without a thumbnail keep a blank
//! cell of the same width so the text column stays column-aligned across rows.
//!
//! Pairs with media-feed fetchers (`rss` with `media:thumbnail`, `reddit_subreddit_posts`,
//! `wikipedia_featured` / `wikipedia_random`) where the thumbnail carries as much glance
//! value as the title.

use ratatui::{
    Frame,
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
};
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{Body, ImageLinkedItem, ImageLinkedListData};
use crate::theme::{self, ColorKey, Theme};

use super::list_links::wrap_osc8;
use super::media_image::draw_thumbnail;
use super::{Registry, RenderOptions, Renderer, Shape};

const COLOR_KEYS: &[ColorKey] = &[theme::TEXT, theme::TEXT_DIM];

const DEFAULT_THUMBNAIL_WIDTH: u16 = 6;
const DEFAULT_ROW_HEIGHT: u16 = 3;
const DEFAULT_GAP: u16 = 1;

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "max_items",
        type_hint: "positive integer",
        required: false,
        default: Some("all rows"),
        description: "Cap on rendered cards. Truncates from the end when the input has more rows than the cap.",
    },
    OptionSchema {
        name: "thumbnail_width",
        type_hint: "cells (u16)",
        required: false,
        default: Some("6"),
        description: "Width in cells of the thumbnail column. Rows without a thumbnail still consume this width as blank space so the text column stays aligned.",
    },
    OptionSchema {
        name: "row_height",
        type_hint: "cells (u16)",
        required: false,
        default: Some("3"),
        description: "Height in cells of each card row. Row 0 carries the title (linked when `url` is set), row 1 the optional subtitle, row 2 is breathing room.",
    },
    OptionSchema {
        name: "gap",
        type_hint: "cells (u16)",
        required: false,
        default: Some("1"),
        description: "Horizontal gap in cells between the thumbnail column and the text column.",
    },
    OptionSchema {
        name: "fit",
        type_hint: "\"contain\" | \"cover\" | \"stretch\"",
        required: false,
        default: Some("\"contain\""),
        description: "How each thumbnail is sized into its cell. Matches `media_image`'s `fit`.",
    },
];

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    pub thumbnail_width: Option<u16>,
    #[serde(default)]
    pub row_height: Option<u16>,
    #[serde(default)]
    pub gap: Option<u16>,
    #[serde(default)]
    pub fit: Option<String>,
}

pub struct ListCardsRenderer;

impl Renderer for ListCardsRenderer {
    fn name(&self) -> &str {
        "list_cards"
    }
    fn description(&self) -> &'static str {
        "Card-shaped rows with a thumbnail on the left and title (+ optional subtitle) on the right. Rows that carry a URL get an OSC 8 hyperlink wrap on the title so terminals make it clickable. Use this for media-feed widgets (RSS with images, subreddit posts, Wikipedia featured) where the thumbnail is part of the headline."
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::ImageLinkedList]
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
        if let Body::ImageLinkedList(d) = body {
            render_cards(frame, area, d, opts, theme);
        }
    }
    fn natural_height(
        &self,
        body: &Body,
        opts: &RenderOptions,
        _max_width: u16,
        _registry: &Registry,
    ) -> u16 {
        if let Body::ImageLinkedList(d) = body {
            let cap = opts.max_items.unwrap_or(usize::MAX);
            let count = d.items.len().min(cap) as u16;
            let row_h = row_height(opts);
            return count.saturating_mul(row_h);
        }
        1
    }
}

fn render_cards(
    frame: &mut Frame,
    area: Rect,
    data: &ImageLinkedListData,
    opts: &RenderOptions,
    theme: &Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let cap = opts.max_items.unwrap_or(usize::MAX);
    let row_h = row_height(opts);
    let thumb_w = thumb_width(opts, area.width);
    let gap = gap(opts);
    let specific: Options = opts.parse_specific();
    let fit = specific.fit.as_deref();
    let max_rows = (area.height / row_h.max(1)) as usize;
    data.items
        .iter()
        .take(cap)
        .take(max_rows)
        .enumerate()
        .for_each(|(i, item)| {
            let y = area.y + i as u16 * row_h;
            let row = Rect {
                x: area.x,
                y,
                width: area.width,
                height: row_h.min(area.y + area.height - y),
            };
            draw_card(frame, row, item, thumb_w, gap, fit, theme);
        });
}

fn draw_card(
    frame: &mut Frame,
    row: Rect,
    item: &ImageLinkedItem,
    thumb_w: u16,
    gap: u16,
    fit: Option<&str>,
    theme: &Theme,
) {
    let thumb_rect = Rect {
        x: row.x,
        y: row.y,
        width: thumb_w.min(row.width),
        height: row.height,
    };
    if let Some(path) = item.thumbnail_path.as_deref().filter(|s| !s.is_empty()) {
        let _ = draw_thumbnail(frame, thumb_rect, path, fit, theme);
    }
    let text_x = row.x + thumb_rect.width + gap;
    if text_x >= row.x + row.width {
        return;
    }
    let text_w = row.x + row.width - text_x;
    draw_text(
        frame.buffer_mut(),
        text_x,
        row.y,
        text_w,
        row.height,
        item,
        theme,
    );
}

fn draw_text(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
    item: &ImageLinkedItem,
    theme: &Theme,
) {
    if width == 0 || height == 0 {
        return;
    }
    let title_style = Style::default().fg(theme.text).add_modifier(Modifier::BOLD);
    let (end_x, _) = buf.set_stringn(x, y, &item.title, width as usize, title_style);
    if let Some(url) = item.url.as_deref().filter(|u| !u.is_empty()) {
        wrap_osc8(buf, x, y, end_x, url, title_style);
    }
    if height >= 2
        && let Some(sub) = item.subtitle.as_deref().filter(|s| !s.is_empty())
    {
        let sub_style = Style::default().fg(theme.text_dim);
        buf.set_stringn(x, y + 1, sub, width as usize, sub_style);
    }
}

fn row_height(opts: &RenderOptions) -> u16 {
    let specific: Options = opts.parse_specific();
    specific.row_height.unwrap_or(DEFAULT_ROW_HEIGHT).max(1)
}

fn thumb_width(opts: &RenderOptions, area_width: u16) -> u16 {
    let specific: Options = opts.parse_specific();
    specific
        .thumbnail_width
        .unwrap_or(DEFAULT_THUMBNAIL_WIDTH)
        .min(area_width)
}

fn gap(opts: &RenderOptions) -> u16 {
    let specific: Options = opts.parse_specific();
    specific.gap.unwrap_or(DEFAULT_GAP)
}

