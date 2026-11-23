//! `lobsters_top` — Lobsters story listings (hottest / newest / active), optionally filtered
//! by a single tag.
//!
//! Safety::Safe because the host is hardcoded (`lobste.rs`). Config picks the listing kind, the
//! row count, and an optional tag; the tag is normalized to a strict ASCII-alphanum-plus-`+_-`
//! subset before being interpolated into the path so no config can redirect traffic off-host.
//!
//! Lobsters' tag listing only exposes the hottest variant — `/t/<tag>/newest.json` and
//! `/t/<tag>/active.json` 404. When a `tag` is set, `kind` is silently forced to `hottest`.

use async_trait::async_trait;
use serde::Deserialize;

use crate::fetcher::github::common::{cache_key, parse_options, parse_timestamp, payload};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{
    Bar, BarsData, Body, EntriesData, Entry, LinkedLine, LinkedTextBlockData,
    MarkdownTextBlockData, Payload, TextBlockData, TextData, TimelineData, TimelineEvent,
};
use crate::render::Shape;
use crate::samples;

use super::client::{SITE_BASE, get};

const DEFAULT_COUNT: u32 = 10;
const MIN_COUNT: u32 = 1;
const MAX_COUNT: u32 = 25;
const TAG_MAX_LEN: usize = 32;

const SHAPES: &[Shape] = &[
    Shape::LinkedTextBlock,
    Shape::TextBlock,
    Shape::Text,
    Shape::MarkdownTextBlock,
    Shape::Entries,
    Shape::Bars,
    Shape::Timeline,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "count",
        type_hint: "integer (1..=25)",
        required: false,
        default: Some("10"),
        description: "Number of stories to display.",
    },
    OptionSchema {
        name: "kind",
        type_hint: "\"hottest\" | \"newest\" | \"active\"",
        required: false,
        default: Some("\"hottest\""),
        description: "Which Lobsters listing to query. Forced to `hottest` when `tag` is set (the tag listing only exposes the hottest variant).",
    },
    OptionSchema {
        name: "tag",
        type_hint: "string",
        required: false,
        default: None,
        description: "Filter to a single Lobsters tag (e.g. `\"rust\"`, `\"c++\"`). Allowed chars: `[a-z0-9_+-]`.",
    },
];

pub struct LobstersTopFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub count: Option<u32>,
    #[serde(default)]
    pub kind: Option<Kind>,
    #[serde(default)]
    pub tag: Option<String>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Hottest,
    Newest,
    Active,
}

impl Kind {
    fn endpoint(self) -> &'static str {
        match self {
            Self::Hottest => "hottest",
            Self::Newest => "newest",
            Self::Active => "active",
        }
    }
}

#[async_trait]
impl Fetcher for LobstersTopFetcher {
    fn name(&self) -> &str {
        "lobsters_top"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Lobsters front-page listings — `hottest` / `newest` / `active`, optionally filtered by a single tag. Each row shows score, comment count, and title, linked to the story URL (or the Lobsters comment page when there isn't one)."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 10
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        let extra = ctx
            .options
            .as_ref()
            .and_then(|v| toml::to_string(v).ok())
            .unwrap_or_default();
        cache_key(self.name(), ctx, &extra)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Text => samples::text("75pt 17c  Show: terminal dashboard with local trust"),
            Shape::TextBlock => samples::text_block(&[
                "75pt 17c  Show: terminal dashboard with local trust",
                "41pt 6c   Fake Notepad++ for Mac",
                "29pt 2c   This Wasm interpreter fits in a QR code",
            ]),
            Shape::LinkedTextBlock => samples::linked_text_block(&[
                (
                    "75pt 17c  Show: terminal dashboard with local trust",
                    Some("https://example.com/show"),
                ),
                (
                    "41pt 6c   Fake Notepad++ for Mac",
                    Some("https://example.com/np"),
                ),
                (
                    "29pt 2c   This Wasm interpreter fits in a QR code",
                    Some("https://lobste.rs/s/abc123"),
                ),
            ]),
            Shape::MarkdownTextBlock => samples::markdown(
                "- **75pt 17c**  [Show: terminal dashboard with local trust](https://example.com/show)\n\
                 - **41pt 6c**   [Fake Notepad++ for Mac](https://example.com/np)\n\
                 - **29pt 2c**   [This Wasm interpreter fits in a QR code](https://lobste.rs/s/abc123)",
            ),
            Shape::Entries => samples::entries(&[
                ("Show: terminal dashboard with local trust", "75pt 17c"),
                ("Fake Notepad++ for Mac", "41pt 6c"),
                ("This Wasm interpreter fits in a QR code", "29pt 2c"),
            ]),
            Shape::Bars => Body::Bars(BarsData {
                bars: vec![
                    Bar {
                        label: "Show: terminal dashboard …".into(),
                        value: 75,
                        value_label: None,
                    },
                    Bar {
                        label: "Fake Notepad++ for Mac".into(),
                        value: 41,
                        value_label: None,
                    },
                    Bar {
                        label: "This Wasm interpreter fits …".into(),
                        value: 29,
                        value_label: None,
                    },
                ],
            }),
            Shape::Timeline => samples::timeline(&[
                (
                    1_777_708_434,
                    "Show: terminal dashboard with local trust",
                    Some("75pt 17c"),
                ),
                (1_777_705_489, "Fake Notepad++ for Mac", Some("41pt 6c")),
                (
                    1_777_705_205,
                    "This Wasm interpreter fits in a QR code",
                    Some("29pt 2c"),
                ),
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let count = opts
            .count
            .unwrap_or(DEFAULT_COUNT)
            .clamp(MIN_COUNT, MAX_COUNT) as usize;
        let kind = opts.kind.unwrap_or_default();
        let tag = opts.tag.as_deref().map(normalize_tag).transpose()?;
        let path = listing_path(kind, tag.as_deref());
        let items = fetch_stories(&path, count).await?;
        Ok(payload(render_body(
            &items,
            ctx.shape.unwrap_or(Shape::LinkedTextBlock),
        )))
    }
}

fn listing_path(kind: Kind, tag: Option<&str>) -> String {
    match tag {
        // Tag listing only exposes hottest — silently force-route there.
        Some(t) => format!("/t/{t}.json"),
        None => format!("/{}.json", kind.endpoint()),
    }
}

fn normalize_tag(raw: &str) -> Result<String, FetchError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(FetchError::Failed("tag must not be empty".into()));
    }
    if trimmed.len() > TAG_MAX_LEN {
        return Err(FetchError::Failed("tag is too long".into()));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-'))
    {
        return Err(FetchError::Failed(
            "tag must contain only [a-z0-9_+-]".into(),
        ));
    }
    Ok(trimmed.to_ascii_lowercase())
}

async fn fetch_stories(path: &str, count: usize) -> Result<Vec<Item>, FetchError> {
    let items: Vec<Item> = get(&format!("{SITE_BASE}{path}")).await?;
    Ok(items.into_iter().take(count).collect())
}

#[derive(Debug, Deserialize)]
struct Item {
    #[serde(default)]
    short_id: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    score: Option<i64>,
    #[serde(default)]
    comment_count: Option<u64>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    comments_url: Option<String>,
    #[serde(default)]
    short_id_url: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
}

fn link_for(it: &Item) -> Option<String> {
    it.url
        .clone()
        .filter(|u| !u.is_empty())
        .or_else(|| it.comments_url.clone())
        .or_else(|| it.short_id_url.clone())
        .or_else(|| {
            it.short_id
                .as_deref()
                .map(|id| format!("{SITE_BASE}/s/{id}"))
        })
}

fn render_body(items: &[Item], shape: Shape) -> Body {
    match shape {
        Shape::Text => Body::Text(TextData {
            value: items
                .first()
                .map(|it| format!("{}  {}", meta_label(it), title_or_placeholder(it)))
                .unwrap_or_default(),
        }),
        Shape::Entries => Body::Entries(EntriesData {
            items: items
                .iter()
                .map(|it| Entry {
                    key: title_or_placeholder(it),
                    value: Some(meta_label(it)),
                    status: None,
                })
                .collect(),
        }),
        Shape::Bars => Body::Bars(BarsData {
            bars: items
                .iter()
                .map(|it| Bar {
                    label: title_or_placeholder(it),
                    value: it.score.unwrap_or(0).max(0) as u64,
                    value_label: None,
                })
                .collect(),
        }),
        Shape::LinkedTextBlock => Body::LinkedTextBlock(LinkedTextBlockData {
            items: items
                .iter()
                .map(|it| LinkedLine {
                    text: format!("{}  {}", meta_label(it), title_or_placeholder(it)),
                    url: link_for(it),
                })
                .collect(),
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: items
                .iter()
                .map(markdown_row)
                .collect::<Vec<_>>()
                .join("\n"),
        }),
        Shape::Timeline => Body::Timeline(TimelineData {
            events: items
                .iter()
                .map(|it| TimelineEvent {
                    timestamp: it.created_at.as_deref().map(parse_timestamp).unwrap_or(0),
                    title: title_or_placeholder(it),
                    detail: Some(meta_label(it)),
                    status: None,
                })
                .collect(),
        }),
        _ => Body::TextBlock(TextBlockData {
            lines: items
                .iter()
                .map(|it| format!("{}  {}", meta_label(it), title_or_placeholder(it)))
                .collect(),
        }),
    }
}

fn title_or_placeholder(it: &Item) -> String {
    it.title
        .as_deref()
        .filter(|t| !t.is_empty())
        .unwrap_or("(no title)")
        .to_string()
}

fn meta_label(it: &Item) -> String {
    let score = it.score.unwrap_or(0);
    let comments = it.comment_count.unwrap_or(0);
    format!("{score}pt {comments}c")
}

fn markdown_row(it: &Item) -> String {
    let title = title_or_placeholder(it);
    let meta = meta_label(it);
    match link_for(it) {
        Some(url) => format!("- **{meta}**  [{title}]({url})"),
        None => format!("- **{meta}**  {title}"),
    }
}

