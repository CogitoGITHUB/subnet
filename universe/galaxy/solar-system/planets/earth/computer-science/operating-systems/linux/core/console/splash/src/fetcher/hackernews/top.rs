//! `hackernews_top` — Hacker News stories (top / new / best / ask / show / job).
//!
//! Safety::Safe because the host is hardcoded (HN's read-only Firebase backend). Config picks
//! the listing kind and how many stories to show, never the URL — there's no way for a config
//! to redirect traffic off-host. No auth, no token leaves the machine.

use async_trait::async_trait;
use serde::Deserialize;

use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{
    Body, EntriesData, Entry, LinkedLine, LinkedTextBlockData, Payload, TextBlockData,
};
use crate::render::Shape;
use crate::samples;

use super::client::{API_BASE, HN_ITEM_URL, get};

const DEFAULT_COUNT: u32 = 10;
const MIN_COUNT: u32 = 1;
const MAX_COUNT: u32 = 30;

const SHAPES: &[Shape] = &[Shape::LinkedTextBlock, Shape::TextBlock, Shape::Entries];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "count",
        type_hint: "integer (1..=30)",
        required: false,
        default: Some("10"),
        description: "Number of stories to display.",
    },
    OptionSchema {
        name: "kind",
        type_hint: "\"top\" | \"new\" | \"best\" | \"ask\" | \"show\" | \"job\"",
        required: false,
        default: Some("\"top\""),
        description: "Which Hacker News listing to query.",
    },
];

pub struct HackernewsTopFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub count: Option<u32>,
    #[serde(default)]
    pub kind: Option<Kind>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Top,
    New,
    Best,
    Ask,
    Show,
    Job,
}

impl Kind {
    fn endpoint(self) -> &'static str {
        match self {
            Self::Top => "topstories",
            Self::New => "newstories",
            Self::Best => "beststories",
            Self::Ask => "askstories",
            Self::Show => "showstories",
            Self::Job => "jobstories",
        }
    }
}

#[async_trait]
impl Fetcher for HackernewsTopFetcher {
    fn name(&self) -> &str {
        "hackernews_top"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Hacker News front-page listings — `top` / `new` / `best` / `ask` / `show` / `job`. Each row shows score, comment count, and title, linked to the story URL (or the HN comment page when there isn't one)."
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
            Shape::TextBlock => samples::text_block(&[
                "234pt 56c  Show HN: I built a thing",
                "187pt 41c  Why X over Y",
                "152pt 88c  Ask HN: how do you ...",
            ]),
            Shape::LinkedTextBlock => samples::linked_text_block(&[
                (
                    "234pt 56c  Show HN: I built a thing",
                    Some("https://example.com/show-hn"),
                ),
                (
                    "187pt 41c  Why X over Y",
                    Some("https://example.com/article"),
                ),
                (
                    "152pt 88c  Ask HN: how do you ...",
                    Some("https://news.ycombinator.com/item?id=1"),
                ),
            ]),
            Shape::Entries => samples::entries(&[
                ("Show HN: I built a thing", "234pt 56c"),
                ("Why X over Y", "187pt 41c"),
                ("Ask HN: how do you ...", "152pt 88c"),
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
        let items = fetch_stories(kind, count).await?;
        Ok(payload(render_body(
            &items,
            ctx.shape.unwrap_or(Shape::LinkedTextBlock),
        )))
    }
}

async fn fetch_stories(kind: Kind, count: usize) -> Result<Vec<Item>, FetchError> {
    let ids: Vec<u64> = get(&format!("{API_BASE}/{}.json", kind.endpoint())).await?;
    let handles: Vec<_> = ids
        .into_iter()
        .take(count)
        .map(|id| tokio::spawn(async move { fetch_item(id).await }))
        .collect();
    let mut items = Vec::with_capacity(handles.len());
    for h in handles {
        if let Ok(Ok(it)) = h.await {
            items.push(it);
        }
    }
    Ok(items)
}

async fn fetch_item(id: u64) -> Result<Item, FetchError> {
    get(&format!("{API_BASE}/item/{id}.json")).await
}

#[derive(Debug, Deserialize)]
struct Item {
    #[serde(default)]
    id: Option<u64>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    score: Option<u64>,
    #[serde(default)]
    descendants: Option<u64>,
    #[serde(default)]
    url: Option<String>,
}

fn link_for(it: &Item) -> Option<String> {
    it.url
        .clone()
        .or_else(|| it.id.map(|id| format!("{HN_ITEM_URL}{id}")))
}

fn render_body(items: &[Item], shape: Shape) -> Body {
    match shape {
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
        Shape::LinkedTextBlock => Body::LinkedTextBlock(LinkedTextBlockData {
            items: items
                .iter()
                .map(|it| LinkedLine {
                    text: format!("{}  {}", meta_label(it), title_or_placeholder(it)),
                    url: link_for(it),
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
    it.title.clone().unwrap_or_else(|| "(no title)".into())
}

fn meta_label(it: &Item) -> String {
    let score = it.score.unwrap_or(0);
    let comments = it.descendants.unwrap_or(0);
    format!("{score}pt {comments}c")
}

