//! `youtube_channel` — recent uploads from one or more public YouTube channels.
//!
//! Reads `https://www.youtube.com/feeds/videos.xml` per channel. No API key, no OAuth — public
//! uploads only.
//!
//! Two endpoint variants, matching the glanceapp/glance pattern:
//! - default (`include_shorts = false`): rewrite `UC...` → `UULF...` and use
//!   `?playlist_id=UULF...`, the channel's auto-generated long-form-uploads playlist (excludes
//!   Shorts).
//! - `include_shorts = true`: keep `?channel_id=UC...`, which returns every upload.
//!
//! As of 2026 the YouTube RSS endpoint is intermittently broken (glanceapp/glance #910): the
//! channel_id and playlist_id forms occasionally return 404 for valid IDs. Per-channel
//! failures fall through silently as long as one channel succeeds; only all-fail surfaces an
//! error.
//!
//! Safety::Safe. The host is hardcoded; each `channel_id` is validated against
//! `^UC[A-Za-z0-9_-]{22}$` before it reaches the URL, so a config can't inject path / query
//! bytes that escape youtube.com. Multi-channel = parallel fetch + merge by `published` desc.

use std::sync::OnceLock;

use async_trait::async_trait;
use feed_rs::model::{Entry, Feed, FeedType};
use regex::Regex;
use serde::Deserialize;
use tokio::task::JoinSet;
use url::Url;

use crate::fetcher::feed;
use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{Body, Payload};
use crate::render::Shape;
use crate::samples;

const FEED_BASE: &str = "https://www.youtube.com/feeds/videos.xml";
const DEFAULT_COUNT: u32 = 5;
const MIN_COUNT: u32 = 1;
const MAX_COUNT: u32 = 20;

const SHAPES: &[Shape] = &[
    Shape::LinkedTextBlock,
    Shape::TextBlock,
    Shape::Text,
    Shape::MarkdownTextBlock,
    Shape::ImageLinkedList,
    Shape::Entries,
    Shape::Image,
    Shape::Timeline,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "channel_ids",
        type_hint: "array of strings (UC… channel IDs)",
        required: true,
        default: None,
        description: "One or more YouTube channel IDs. Each ID must match `UC` + 22 chars from `[A-Za-z0-9_-]` (visible in the channel URL `https://www.youtube.com/channel/UC...`).",
    },
    OptionSchema {
        name: "count",
        type_hint: "integer (1..=20)",
        required: false,
        default: Some("5"),
        description: "Number of merged uploads to display, sorted newest first across all channels.",
    },
    OptionSchema {
        name: "include_shorts",
        type_hint: "boolean",
        required: false,
        default: Some("false"),
        description: "When `false` (default), uploads are pulled from each channel's auto-generated `UULF` long-form-uploads playlist — Shorts are excluded. Set to `true` to read the channel feed directly and include Shorts.",
    },
];

pub struct YoutubeChannelFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub channel_ids: Option<Vec<String>>,
    #[serde(default)]
    pub count: Option<u32>,
    #[serde(default)]
    pub include_shorts: Option<bool>,
}

#[async_trait]
impl Fetcher for YoutubeChannelFetcher {
    fn name(&self) -> &str {
        "youtube_channel"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Recent uploads from one or more public YouTube channels, merged newest-first. Reads each channel's Atom feed at `youtube.com/feeds/videos.xml`; no API key or OAuth required."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 30
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
                (
                    "Apr 26  Channel A: new tutorial",
                    Some("https://www.youtube.com/watch?v=aaaaaaaaaaA"),
                ),
                (
                    "Apr 25  Channel B: release recap",
                    Some("https://www.youtube.com/watch?v=bbbbbbbbbbB"),
                ),
                (
                    "Apr 24  Channel A: behind the scenes",
                    Some("https://www.youtube.com/watch?v=ccccccccccC"),
                ),
            ]),
            Shape::TextBlock => samples::text_block(&[
                "Apr 26  Channel A: new tutorial",
                "Apr 25  Channel B: release recap",
                "Apr 24  Channel A: behind the scenes",
            ]),
            Shape::Text => samples::text("Channel A: new tutorial"),
            Shape::MarkdownTextBlock => samples::markdown(
                "- [Apr 26  Channel A: new tutorial](https://www.youtube.com/watch?v=aaaaaaaaaaA)\n- [Apr 25  Channel B: release recap](https://www.youtube.com/watch?v=bbbbbbbbbbB)\n- [Apr 24  Channel A: behind the scenes](https://www.youtube.com/watch?v=ccccccccccC)",
            ),
            Shape::ImageLinkedList => samples::image_linked_list(&[
                (
                    "Channel A: new tutorial",
                    Some("https://www.youtube.com/watch?v=aaaaaaaaaaA"),
                    None,
                    Some("Apr 26"),
                ),
                (
                    "Channel B: release recap",
                    Some("https://www.youtube.com/watch?v=bbbbbbbbbbB"),
                    None,
                    Some("Apr 25"),
                ),
                (
                    "Channel A: behind the scenes",
                    Some("https://www.youtube.com/watch?v=ccccccccccC"),
                    None,
                    Some("Apr 24"),
                ),
            ]),
            Shape::Entries => samples::entries(&[
                ("Channel A: new tutorial", "Apr 26"),
                ("Channel B: release recap", "Apr 25"),
                ("Channel A: behind the scenes", "Apr 24"),
            ]),
            Shape::Timeline => samples::timeline(&[
                (
                    1_745_625_600,
                    "Channel A: new tutorial",
                    Some("www.youtube.com"),
                ),
                (
                    1_745_539_200,
                    "Channel B: release recap",
                    Some("www.youtube.com"),
                ),
                (
                    1_745_452_800,
                    "Channel A: behind the scenes",
                    Some("www.youtube.com"),
                ),
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let channel_ids = validated_channel_ids(opts.channel_ids.as_deref())?;
        let count = opts
            .count
            .unwrap_or(DEFAULT_COUNT)
            .clamp(MIN_COUNT, MAX_COUNT) as usize;
        let include_shorts = opts.include_shorts.unwrap_or(false);
        let merged = fetch_and_merge(&channel_ids, include_shorts).await?;
        let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);
        let body = match shape {
            Shape::ImageLinkedList => feed::render_image_linked(&merged, count, ctx).await,
            Shape::Image => feed::render_image_body(&merged).await,
            other => feed::render_body(
                &merged,
                count,
                other,
                ctx.timezone.as_deref(),
                ctx.locale.as_deref(),
            ),
        };
        Ok(payload(body))
    }
}

fn channel_id_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^UC[A-Za-z0-9_-]{22}$").unwrap())
}

fn validated_channel_ids(raw: Option<&[String]>) -> Result<Vec<String>, FetchError> {
    let ids = raw.filter(|v| !v.is_empty()).ok_or_else(|| {
        FetchError::Failed(
            "youtube_channel: option `channel_ids` is required (at least one UC...)".into(),
        )
    })?;
    let re = channel_id_regex();
    for id in ids {
        if !re.is_match(id) {
            return Err(FetchError::Failed(format!(
                "youtube_channel: invalid channel_id `{id}` (must match `^UC[A-Za-z0-9_-]{{22}}$`)"
            )));
        }
    }
    Ok(ids.to_vec())
}

/// Build the Atom feed URL for a single channel.
///
/// `include_shorts = false` rewrites `UC<22>` → `UULF<22>` and queries the uploads-playlist
/// endpoint, which YouTube auto-generates for every channel and which excludes Shorts. This
/// mirrors the glanceapp/glance trick — Shorts otherwise dominate many channels' chronological
/// upload streams.
///
/// `include_shorts = true` falls back to `?channel_id=...`, the raw uploads feed.
fn channel_feed_url(channel_id: &str, include_shorts: bool) -> Url {
    let query = if include_shorts {
        format!("channel_id={channel_id}")
    } else {
        let playlist_id = format!("UULF{}", &channel_id[2..]);
        format!("playlist_id={playlist_id}")
    };
    Url::parse(&format!("{FEED_BASE}?{query}"))
        .expect("channel_id is regex-validated to URL-safe bytes before this is called")
}

/// Fetches each channel's Atom feed in parallel and merges entries newest-first. Per-channel
/// failures are tolerated when at least one channel succeeds — a 5-channel widget with one 404
/// still renders the surviving 4. When *every* channel fails, the first error is surfaced so the
/// user can see why (a silent empty-placeholder would hide a misspelled ID or a YouTube outage).
async fn fetch_and_merge(channel_ids: &[String], include_shorts: bool) -> Result<Feed, FetchError> {
    let mut set: JoinSet<Result<Vec<Entry>, FetchError>> = JoinSet::new();
    for id in channel_ids {
        let url = channel_feed_url(id, include_shorts);
        set.spawn(async move {
            let bytes = feed::fetch_bytes(&url, "youtube_channel").await?;
            let parsed = feed::parse_feed(&bytes, "youtube_channel")?;
            Ok(parsed.entries)
        });
    }
    let mut entries: Vec<Entry> = Vec::new();
    let mut first_error: Option<FetchError> = None;
    let mut any_success = false;
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok(Ok(chunk)) => {
                any_success = true;
                entries.extend(chunk);
            }
            Ok(Err(e)) => {
                first_error.get_or_insert(e);
            }
            Err(e) => {
                first_error.get_or_insert(FetchError::Failed(format!("youtube_channel join: {e}")));
            }
        }
    }
    if !any_success {
        return Err(first_error.unwrap_or_else(|| {
            FetchError::Failed("youtube_channel: no channels yielded entries".into())
        }));
    }
    entries.sort_by_key(|e| std::cmp::Reverse(e.published.or(e.updated)));
    Ok(empty_feed_with_entries(entries))
}

fn empty_feed_with_entries(entries: Vec<Entry>) -> Feed {
    Feed {
        feed_type: FeedType::Atom,
        id: String::new(),
        updated: None,
        authors: vec![],
        title: None,
        description: None,
        links: vec![],
        categories: vec![],
        contributors: vec![],
        generator: None,
        icon: None,
        language: None,
        logo: None,
        published: None,
        rating: None,
        rights: None,
        ttl: None,
        people: vec![],
        entries,
    }
}

