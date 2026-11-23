//! `rss` — generic feed fetcher (RSS 2.0 / RSS 1.0 / Atom / JSON Feed via `feed-rs`).
//!
//! Safety::Network — the URL is config-provided, so the destination is whatever the user (or a
//! local-config they trust) types. Local configs must be `splashboard trust`-ed before this
//! widget runs; global config is implicitly trusted.
//!
//! Output shapes: `LinkedTextBlock` (default — clickable rows for `list_links`),
//! `ImageLinkedList` (cards with thumbnails), and `TextBlock` (titles only). All wiring around
//! HTTP retrieval, feed-rs parsing, and entry-to-row formatting lives in `crate::fetcher::feed`
//! so the `news_*` family can reuse it.

use async_trait::async_trait;
use serde::Deserialize;
use url::Url;

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
        name: "url",
        type_hint: "string (http / https)",
        required: true,
        default: None,
        description: "Feed URL — RSS 2.0, RSS 1.0, Atom, or JSON Feed.",
    },
    OptionSchema {
        name: "count",
        type_hint: "integer (1..=20)",
        required: false,
        default: Some("5"),
        description: "Number of feed entries to display.",
    },
];

pub struct RssFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub count: Option<u32>,
}

#[async_trait]
impl Fetcher for RssFetcher {
    fn name(&self) -> &str {
        "rss"
    }
    fn safety(&self) -> Safety {
        Safety::Network
    }
    fn description(&self) -> &'static str {
        "Generic feed reader for any RSS 2.0 / RSS 1.0 / Atom / JSON Feed URL, parsed via `feed-rs`. Each row shows the entry's published date and title, optionally linked. Trust-gated because the URL is config-controlled."
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
        cache_key(self.name(), ctx, &extra)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::LinkedTextBlock => samples::linked_text_block(&[
                ("Apr 26  Why X over Y", Some("https://example.com/post-1")),
                (
                    "Apr 24  Release notes 0.42",
                    Some("https://example.com/post-2"),
                ),
                ("Apr 22  A short note", Some("https://example.com/post-3")),
            ]),
            Shape::ImageLinkedList => samples::image_linked_list(&[
                (
                    "Why X over Y",
                    Some("https://example.com/post-1"),
                    None,
                    Some("Apr 26"),
                ),
                (
                    "Release notes 0.42",
                    Some("https://example.com/post-2"),
                    None,
                    Some("Apr 24"),
                ),
                (
                    "A short note",
                    Some("https://example.com/post-3"),
                    None,
                    Some("Apr 22"),
                ),
            ]),
            Shape::TextBlock => samples::text_block(&[
                "Apr 26  Why X over Y",
                "Apr 24  Release notes 0.42",
                "Apr 22  A short note",
            ]),
            Shape::Text => samples::text("Why X over Y"),
            Shape::MarkdownTextBlock => samples::markdown(
                "- [Apr 26  Why X over Y](https://example.com/post-1)\n- [Apr 24  Release notes 0.42](https://example.com/post-2)\n- [Apr 22  A short note](https://example.com/post-3)",
            ),
            Shape::Entries => samples::entries(&[
                ("Why X over Y", "Apr 26"),
                ("Release notes 0.42", "Apr 24"),
                ("A short note", "Apr 22"),
            ]),
            Shape::Timeline => samples::timeline(&[
                (1_745_625_600, "Why X over Y", Some("example.com")),
                (1_745_452_800, "Release notes 0.42", Some("example.com")),
                (1_745_280_000, "A short note", Some("example.com")),
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let url = validated_url(opts.url.as_deref())?;
        let count = opts
            .count
            .unwrap_or(feed::DEFAULT_COUNT)
            .clamp(feed::MIN_COUNT, feed::MAX_COUNT) as usize;
        let bytes = feed::fetch_bytes(&url, "rss").await?;
        let parsed = feed::parse_feed(&bytes, "rss")?;
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

fn validated_url(raw: Option<&str>) -> Result<Url, FetchError> {
    let raw = raw
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| FetchError::Failed("rss: option `url` is required".into()))?;
    let parsed =
        Url::parse(raw).map_err(|e| FetchError::Failed(format!("rss: invalid url: {e}")))?;
    match parsed.scheme() {
        "http" | "https" => Ok(parsed),
        other => Err(FetchError::Failed(format!(
            "rss: unsupported url scheme `{other}` (only http/https allowed)"
        ))),
    }
}

