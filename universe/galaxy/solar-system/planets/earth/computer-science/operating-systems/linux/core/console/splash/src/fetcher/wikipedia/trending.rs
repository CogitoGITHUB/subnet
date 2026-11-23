//! `wikipedia_trending` — most-viewed Wikipedia articles for the previous UTC day via the
//! Wikimedia Pageviews API.
//!
//! Safety::Safe: requests target two fixed hosts — `wikimedia.org` for the pageviews list and
//! `<lang>.wikipedia.org` for per-article summary lookups when the `ImageLinkedList` shape
//! needs thumbnails. The user-supplied `lang` only swaps the language subdomain; it can't
//! redirect traffic off the wiki ecosystem.
//!
//! The pageviews response is filtered to drop `Main_Page`, search pages, and meta-namespace
//! entries (`Wikipedia:`, `Portal:`, `File:`, `Category:`, …) so the trending list shows real
//! articles a reader would actually click. `date` defaults to "yesterday in UTC" because the
//! API serves complete days only and today's snapshot is not yet finalised.

use std::path::PathBuf;

use async_trait::async_trait;
use chrono::{Datelike, Duration, Utc};
use serde::Deserialize;
use url::Url;

use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::thumbnails;
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{
    Bar, BarsData, Body, EntriesData, Entry, ImageLinkedItem, ImageLinkedListData, LinkedLine,
    LinkedTextBlockData, MarkdownTextBlockData, Payload, TextBlockData, TextData,
};
use crate::render::Shape;
use crate::samples;

use super::client::{DEFAULT_LANG, PageSummary, get, rest_api_base};

const DEFAULT_COUNT: u32 = 10;
const MIN_COUNT: u32 = 1;
const MAX_COUNT: u32 = 20;

const SHAPES: &[Shape] = &[
    Shape::LinkedTextBlock,
    Shape::ImageLinkedList,
    Shape::TextBlock,
    Shape::Text,
    Shape::MarkdownTextBlock,
    Shape::Entries,
    Shape::Bars,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "lang",
        type_hint: "string (Wikipedia language code)",
        required: false,
        default: Some("\"en\""),
        description: "Wikipedia language edition.",
    },
    OptionSchema {
        name: "count",
        type_hint: "integer (1..=20)",
        required: false,
        default: Some("10"),
        description: "Number of trending articles to display.",
    },
    OptionSchema {
        name: "access",
        type_hint: "\"all-access\" | \"desktop\" | \"mobile-web\" | \"mobile-app\"",
        required: false,
        default: Some("\"all-access\""),
        description: "Which pageview channel to rank by.",
    },
];

pub struct WikipediaTrendingFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    lang: Option<String>,
    #[serde(default)]
    count: Option<u32>,
    #[serde(default)]
    access: Option<Access>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Access {
    /// Default; named `All` instead of `AllAccess` to satisfy clippy's
    /// `enum_variant_names` lint. Serde rename keeps the user-facing TOML value
    /// `"all-access"` matching the Wikimedia API path segment.
    #[default]
    #[serde(rename = "all-access")]
    All,
    Desktop,
    MobileWeb,
    MobileApp,
}

impl Access {
    fn as_path(self) -> &'static str {
        match self {
            Self::All => "all-access",
            Self::Desktop => "desktop",
            Self::MobileWeb => "mobile-web",
            Self::MobileApp => "mobile-app",
        }
    }
}

#[async_trait]
impl Fetcher for WikipediaTrendingFetcher {
    fn name(&self) -> &str {
        "wikipedia_trending"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Most-viewed Wikipedia articles for the previous UTC day via the public Wikimedia pageviews API. Filters out `Main_Page`, search, and meta-namespace pages so the list is real articles. Companion to the shipped `wikipedia_featured` (today's curated pick) and `wikipedia_random`."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 60 * 6
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
        sample_trending_body(shape)
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let lang = opts.lang.as_deref().unwrap_or(DEFAULT_LANG);
        let count = opts
            .count
            .unwrap_or(DEFAULT_COUNT)
            .clamp(MIN_COUNT, MAX_COUNT) as usize;
        let access = opts.access.unwrap_or_default();
        let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);

        let articles = fetch_trending_articles(lang, access, count).await?;
        let body = render_for_shape(&articles, shape, lang).await;
        Ok(payload(body))
    }
}

/// Trending article as the renderer wants it — title display form, source page URL, view
/// count. Thumbnail URL is resolved later (only when the shape needs it) via a per-article
/// `/page/summary` call.
#[derive(Debug, Clone)]
struct Article {
    title_display: String,
    title_raw: String,
    url: String,
    views: u64,
}

async fn fetch_trending_articles(
    lang: &str,
    access: Access,
    limit: usize,
) -> Result<Vec<Article>, FetchError> {
    let (year, month, day) = previous_utc_day();
    let url = format!(
        "https://wikimedia.org/api/rest_v1/metrics/pageviews/top/{lang}.wikipedia/{}/{year}/{month:02}/{day:02}",
        access.as_path(),
    );
    let response: TopResponse = get(&url).await?;
    Ok(response
        .items
        .into_iter()
        .flat_map(|item| item.articles)
        .filter(is_real_article)
        .take(limit)
        .map(|raw| article_from_raw(lang, raw))
        .collect())
}

fn previous_utc_day() -> (i32, u32, u32) {
    let yesterday = Utc::now() - Duration::days(1);
    (yesterday.year(), yesterday.month(), yesterday.day())
}

fn is_real_article(raw: &TopArticle) -> bool {
    let title = raw.article.as_str();
    if title == "Main_Page" || title == "-" {
        return false;
    }
    !title.starts_with("Special:")
        && !title.starts_with("Wikipedia:")
        && !title.starts_with("Portal:")
        && !title.starts_with("File:")
        && !title.starts_with("Category:")
        && !title.starts_with("Help:")
        && !title.starts_with("Template:")
        && !title.starts_with("Talk:")
}

fn article_from_raw(lang: &str, raw: TopArticle) -> Article {
    let title_display = raw.article.replace('_', " ");
    let url = build_article_url(lang, &raw.article);
    Article {
        title_display,
        title_raw: raw.article,
        url,
        views: raw.views,
    }
}

/// Build a `https://{lang}.wikipedia.org/wiki/{title}` URL. Goes through `url::Url` so reserved
/// path characters (`#`, `?`, …) and non-ASCII bytes are percent-encoded correctly; underscores
/// (Wikipedia's canonical title separator) pass through unchanged.
fn build_article_url(lang: &str, title_raw: &str) -> String {
    let mut url = Url::parse(&format!("https://{lang}.wikipedia.org/wiki/"))
        .expect("wiki article base parses");
    url.path_segments_mut()
        .expect("wikipedia URL is path-segments-capable")
        .pop_if_empty()
        .push(title_raw);
    url.into()
}

async fn render_for_shape(articles: &[Article], shape: Shape, lang: &str) -> Body {
    if matches!(shape, Shape::ImageLinkedList) {
        let paths = resolve_thumbnails(articles, lang).await;
        return image_linked_body(articles, &paths);
    }
    render_sync(articles, shape)
}

async fn resolve_thumbnails(articles: &[Article], lang: &str) -> Vec<Option<PathBuf>> {
    let urls = collect_thumbnail_urls(articles, lang).await;
    thumbnails::download_many(&urls).await
}

async fn collect_thumbnail_urls(articles: &[Article], lang: &str) -> Vec<Option<String>> {
    let base = rest_api_base(lang);
    let mut out = Vec::with_capacity(articles.len());
    for article in articles {
        out.push(fetch_thumbnail_url(&base, &article.title_raw).await);
    }
    out
}

async fn fetch_thumbnail_url(rest_base: &str, title_raw: &str) -> Option<String> {
    let url = build_summary_url(rest_base, title_raw);
    let summary: PageSummary = get(&url).await.ok()?;
    summary.thumbnail_url().map(str::to_string)
}

fn build_summary_url(rest_base: &str, title_raw: &str) -> String {
    let mut url =
        Url::parse(&format!("{rest_base}/page/summary/")).expect("wiki summary base parses");
    url.path_segments_mut()
        .expect("wikipedia URL is path-segments-capable")
        .pop_if_empty()
        .push(title_raw);
    url.into()
}

fn render_sync(articles: &[Article], shape: Shape) -> Body {
    match shape {
        Shape::Text => Body::Text(TextData {
            value: headline(articles),
        }),
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: articles.iter().map(article_line).collect(),
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: markdown_body(articles),
        }),
        Shape::Entries => Body::Entries(EntriesData {
            items: articles
                .iter()
                .map(|a| Entry {
                    key: a.title_display.clone(),
                    value: Some(format_views(a.views)),
                    status: None,
                })
                .collect(),
        }),
        Shape::Bars => Body::Bars(BarsData {
            bars: articles
                .iter()
                .map(|a| Bar {
                    label: a.title_display.clone(),
                    value: a.views,
                    value_label: None,
                })
                .collect(),
        }),
        _ => Body::LinkedTextBlock(LinkedTextBlockData {
            items: articles
                .iter()
                .map(|a| LinkedLine {
                    text: format!("{}  {}", format_views(a.views), a.title_display),
                    url: Some(a.url.clone()),
                })
                .collect(),
        }),
    }
}

fn image_linked_body(articles: &[Article], paths: &[Option<PathBuf>]) -> Body {
    Body::ImageLinkedList(ImageLinkedListData {
        items: articles
            .iter()
            .enumerate()
            .map(|(i, a)| ImageLinkedItem {
                title: a.title_display.clone(),
                url: Some(a.url.clone()),
                thumbnail_path: paths
                    .get(i)
                    .and_then(|p| p.as_ref())
                    .map(|p| p.to_string_lossy().into_owned()),
                subtitle: Some(format_views(a.views)),
            })
            .collect(),
    })
}

fn headline(articles: &[Article]) -> String {
    articles
        .first()
        .map(|a| format!("{}  {}", format_views(a.views), a.title_display))
        .unwrap_or_else(|| "(no trending articles)".into())
}

fn article_line(article: &Article) -> String {
    format!("{}  {}", format_views(article.views), article.title_display)
}

fn markdown_body(articles: &[Article]) -> String {
    articles
        .iter()
        .map(|a| format!("- **{}** — {}", a.title_display, format_views(a.views)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_views(views: u64) -> String {
    if views >= 1_000_000 {
        format!("{:.1}M views", views as f64 / 1_000_000.0)
    } else if views >= 1_000 {
        format!("{:.1}k views", views as f64 / 1_000.0)
    } else {
        format!("{views} views")
    }
}

fn sample_trending_body(shape: Shape) -> Option<Body> {
    Some(match shape {
        Shape::LinkedTextBlock => samples::linked_text_block(&[
            (
                "412.0k views  Quokka",
                Some("https://en.wikipedia.org/wiki/Quokka"),
            ),
            (
                "287.0k views  Apollo 11",
                Some("https://en.wikipedia.org/wiki/Apollo_11"),
            ),
        ]),
        Shape::ImageLinkedList => samples::image_linked_list(&[
            (
                "Quokka",
                Some("https://en.wikipedia.org/wiki/Quokka"),
                None,
                Some("412.0k views"),
            ),
            (
                "Apollo 11",
                Some("https://en.wikipedia.org/wiki/Apollo_11"),
                None,
                Some("287.0k views"),
            ),
        ]),
        Shape::TextBlock => {
            samples::text_block(&["412.0k views  Quokka", "287.0k views  Apollo 11"])
        }
        Shape::Text => samples::text("412.0k views  Quokka"),
        Shape::MarkdownTextBlock => {
            samples::markdown("- **Quokka** — 412.0k views\n- **Apollo 11** — 287.0k views")
        }
        Shape::Entries => {
            samples::entries(&[("Quokka", "412.0k views"), ("Apollo 11", "287.0k views")])
        }
        Shape::Bars => samples::bars(&[("Quokka", 412_000), ("Apollo 11", 287_000)]),
        _ => return None,
    })
}

#[derive(Debug, Deserialize)]
struct TopResponse {
    items: Vec<TopItem>,
}

#[derive(Debug, Deserialize)]
struct TopItem {
    articles: Vec<TopArticle>,
}

#[derive(Debug, Deserialize)]
struct TopArticle {
    article: String,
    views: u64,
}

