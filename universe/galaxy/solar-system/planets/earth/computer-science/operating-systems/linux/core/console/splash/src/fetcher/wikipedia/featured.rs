//! `wikipedia_featured` — today's Featured Article (TFA) via the Wikipedia REST API
//! `feed/featured/{YYYY}/{MM}/{DD}` endpoint.
//!
//! Safety::Safe: host is `*.wikipedia.org`. The TFA slot is best populated for `lang = "en"`;
//! other languages may return an empty `tfa` block, which surfaces as a fetch error so the
//! splash falls back to its `error_placeholder`.

use async_trait::async_trait;
use chrono::{Datelike, Utc};
use serde::Deserialize;

use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::thumbnails;
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{Body, Payload};
use crate::render::Shape;
use crate::samples;

use super::client::{
    DEFAULT_LANG, PageSummary, get, render_page_summary, render_page_summary_with_thumbnail,
    rest_api_base,
};

const SHAPES: &[Shape] = &[
    Shape::LinkedTextBlock,
    Shape::ImageLinkedList,
    Shape::TextBlock,
    Shape::Text,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "lang",
    type_hint: "string (Wikipedia language code)",
    required: false,
    default: Some("\"en\""),
    description: "Wikipedia language edition. The TFA endpoint is best populated for `\"en\"`.",
}];

pub struct WikipediaFeaturedFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub lang: Option<String>,
}

#[async_trait]
impl Fetcher for WikipediaFeaturedFetcher {
    fn name(&self) -> &str {
        "wikipedia_featured"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Today's English Wikipedia \"Today's Featured Article\" — the daily curated front-page pick, with title, summary, and link. Use `wikipedia_on_this_day` for historical events on this date or `wikipedia_random` for an arbitrary article."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 60
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
            Shape::LinkedTextBlock => samples::linked_text_block(&[(
                "Hyperion (poem)",
                Some("https://en.wikipedia.org/wiki/Hyperion_(poem)"),
            )]),
            Shape::ImageLinkedList => samples::image_linked_list(&[(
                "Hyperion (poem)",
                Some("https://en.wikipedia.org/wiki/Hyperion_(poem)"),
                None,
                Some("Hyperion is an unfinished epic poem by John Keats."),
            )]),
            Shape::TextBlock => samples::text_block(&[
                "Hyperion (poem)",
                "Hyperion is an unfinished epic poem by John Keats, recounting the despair of the Titans after their defeat by the Olympians.",
            ]),
            Shape::Text => {
                samples::text("Hyperion (poem): Hyperion is an unfinished epic poem by John Keats.")
            }
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let lang = opts.lang.as_deref().unwrap_or(DEFAULT_LANG);
        let summary = fetch_tfa(lang).await?;
        let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);
        let body = match shape {
            Shape::ImageLinkedList => {
                let path = match summary.thumbnail_url() {
                    Some(url) => thumbnails::download_to_cache(url).await.ok().flatten(),
                    None => None,
                };
                render_page_summary_with_thumbnail(&summary, shape, path)
            }
            _ => render_page_summary(&summary, shape),
        };
        Ok(payload(body))
    }
}

async fn fetch_tfa(lang: &str) -> Result<PageSummary, FetchError> {
    let now = Utc::now();
    let url = format!(
        "{}/feed/featured/{:04}/{:02}/{:02}",
        rest_api_base(lang),
        now.year(),
        now.month(),
        now.day()
    );
    let response: FeaturedResponse = get(&url).await?;
    response
        .tfa
        .ok_or_else(|| FetchError::Failed("wikipedia featured: no `tfa` in response".into()))
}

#[derive(Debug, Deserialize)]
struct FeaturedResponse {
    #[serde(default)]
    tfa: Option<PageSummary>,
}

