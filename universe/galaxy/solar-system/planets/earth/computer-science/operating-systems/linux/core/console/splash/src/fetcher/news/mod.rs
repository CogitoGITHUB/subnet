//! `news_*` — curated single-source feed family.
//!
//! Each `NewsSource` in [`sources::SOURCES`] is exposed as a separate `news_<slug>` fetcher with
//! a hardcoded feed URL set. Compared to the generic [`crate::fetcher::rss::RssFetcher`]:
//!
//! - URLs are baked at compile time (no `url` option), so the family is `Safety::Safe` —
//!   trust-gate friction drops, and a per-dir `.splash/dashboard.toml` shipped with a repo
//!   renders for cloners without `splashboard trust`.
//! - Each source can carry multiple sub-feeds (`feed = "tech"`); config picks among them via a
//!   closed enum, but cannot inject new destinations.
//!
//! Parsing / fetching / row-formatting live in [`crate::fetcher::feed`], shared with `rss`.

mod sources;

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use url::Url;

pub use sources::{NewsCategory, NewsFeed, NewsSource, SOURCES};

use crate::fetcher::feed;
use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{Body, Payload};
use crate::render::Shape;
use crate::samples;

const SHAPES: &[Shape] = &[
    Shape::LinkedTextBlock,
    Shape::ImageLinkedList,
    Shape::TextBlock,
    Shape::Text,
    Shape::MarkdownTextBlock,
    Shape::Entries,
    Shape::Image,
    Shape::Timeline,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "feed",
        type_hint: "string (source-specific key)",
        required: false,
        default: Some("first sub-feed of the source"),
        description: "Sub-feed key to select (e.g. `\"world\"` / `\"tech\"`). Run `splashboard catalog fetcher news_<source>` to list available keys.",
    },
    OptionSchema {
        name: "count",
        type_hint: "integer (1..=20)",
        required: false,
        default: Some("5"),
        description: "Number of feed entries to display.",
    },
];

pub fn fetchers() -> Vec<Arc<dyn Fetcher>> {
    SOURCES
        .iter()
        .map(|source| -> Arc<dyn Fetcher> { Arc::new(NewsFeedFetcher { source }) })
        .collect()
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    feed: Option<String>,
    #[serde(default)]
    count: Option<u32>,
}

pub struct NewsFeedFetcher {
    source: &'static NewsSource,
}

#[async_trait]
impl Fetcher for NewsFeedFetcher {
    fn name(&self) -> &str {
        self.source.name
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        self.source.description
    }
    fn refresh_interval(&self) -> u64 {
        60 * 15
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
        cache_key(self.source.name, ctx, &extra)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        let display = self.source.display;
        Some(match shape {
            Shape::LinkedTextBlock => samples::linked_text_block(&[
                (
                    &format!("Apr 26  {display}: top story"),
                    Some("https://example.com/article-1"),
                ),
                (
                    &format!("Apr 25  {display}: feature piece"),
                    Some("https://example.com/article-2"),
                ),
                (
                    &format!("Apr 24  {display}: shorter update"),
                    Some("https://example.com/article-3"),
                ),
            ]),
            Shape::ImageLinkedList => samples::image_linked_list(&[
                (
                    "Top story",
                    Some("https://example.com/article-1"),
                    None,
                    Some("Apr 26"),
                ),
                (
                    "Feature piece",
                    Some("https://example.com/article-2"),
                    None,
                    Some("Apr 25"),
                ),
                (
                    "Shorter update",
                    Some("https://example.com/article-3"),
                    None,
                    Some("Apr 24"),
                ),
            ]),
            Shape::TextBlock => samples::text_block(&[
                &format!("Apr 26  {display}: top story"),
                &format!("Apr 25  {display}: feature piece"),
                &format!("Apr 24  {display}: shorter update"),
            ]),
            Shape::Text => samples::text(&format!("{display}: top story")),
            Shape::MarkdownTextBlock => samples::markdown(&format!(
                "- [Apr 26  {display}: top story](https://example.com/article-1)\n- [Apr 25  {display}: feature piece](https://example.com/article-2)\n- [Apr 24  {display}: shorter update](https://example.com/article-3)",
            )),
            Shape::Entries => samples::entries(&[
                ("top story", "Apr 26"),
                ("feature piece", "Apr 25"),
                ("shorter update", "Apr 24"),
            ]),
            Shape::Timeline => samples::timeline(&[
                (1_745_625_600, "top story", Some("example.com")),
                (1_745_539_200, "feature piece", Some("example.com")),
                (1_745_452_800, "shorter update", Some("example.com")),
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let selected = resolve_feed(self.source, opts.feed.as_deref())?;
        let url = Url::parse(selected.url).map_err(|e| {
            FetchError::Failed(format!(
                "{}: bundled feed url for `{}` is malformed: {e}",
                self.source.name, selected.key
            ))
        })?;
        let count = opts
            .count
            .unwrap_or(feed::DEFAULT_COUNT)
            .clamp(feed::MIN_COUNT, feed::MAX_COUNT) as usize;
        let bytes = feed::fetch_bytes(&url, self.source.name).await?;
        let parsed = feed::parse_feed(&bytes, self.source.name)?;
        let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);
        let body = match shape {
            Shape::ImageLinkedList => feed::render_image_linked(&parsed, count, ctx).await,
            Shape::Image => feed::render_image_body(&parsed).await,
            other => feed::render_body(
                &parsed,
                count,
                other,
                ctx.timezone.as_deref(),
                ctx.locale.as_deref(),
            ),
        };
        Ok(payload(body))
    }
}

fn resolve_feed(
    source: &'static NewsSource,
    requested: Option<&str>,
) -> Result<&'static NewsFeed, FetchError> {
    let Some(key) = requested.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(source.default_feed());
    };
    source.find_feed(key).ok_or_else(|| {
        let available = source
            .feeds
            .iter()
            .map(|f| f.key)
            .collect::<Vec<_>>()
            .join(", ");
        FetchError::Failed(format!(
            "{}: unknown feed key `{key}` (available: {available})",
            source.name
        ))
    })
}

