//! Shared HTTP client + URL helpers for the `wikipedia_*` family. Targets the public
//! Wikipedia REST API at `https://{lang}.wikipedia.org/api/rest_v1/...` — host stays inside
//! `*.wikipedia.org` regardless of which language code the config supplies, so every fetcher
//! in the family is Safety::Safe.

use std::sync::OnceLock;
use std::time::Duration;

use reqwest::Client;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::fetcher::FetchError;
use crate::payload::{Body, LinkedLine, LinkedTextBlockData, TextBlockData, TextData};
use crate::render::Shape;

pub const REST_API_PATH: &str = "/api/rest_v1";
pub const DEFAULT_LANG: &str = "en";
const USER_AGENT: &str = concat!(
    "splashboard/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/unhappychoice/splashboard-2)"
);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

pub fn http() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .user_agent(USER_AGENT)
            .timeout(REQUEST_TIMEOUT)
            .gzip(true)
            .build()
            .expect("reqwest client should build with default config")
    })
}

pub fn rest_api_base(lang: &str) -> String {
    format!("https://{lang}.wikipedia.org{REST_API_PATH}")
}

pub async fn get<T: DeserializeOwned>(url: &str) -> Result<T, FetchError> {
    let res = http()
        .get(url)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("wikipedia request failed: {e}")))?;
    let status = res.status();
    if !status.is_success() {
        return Err(FetchError::Failed(format!("wikipedia {status}")));
    }
    res.json()
        .await
        .map_err(|e| FetchError::Failed(format!("wikipedia json parse: {e}")))
}

/// Page summary returned by `/page/random/summary`, embedded inside `feed/featured`'s `tfa`
/// block, and inside each on-this-day event's `pages[]`. Featured / random share the rendering
/// helpers below; on-this-day uses only `url()` to wire up event links.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PageSummary {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub extract: Option<String>,
    #[serde(default)]
    pub content_urls: Option<ContentUrls>,
    /// REST API `thumbnail.source` — a remote `*.wikipedia.org` image URL. Optional because not
    /// every page has a thumbnail; the JSON omits the whole block in that case.
    #[serde(default)]
    pub thumbnail: Option<PageThumbnail>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PageThumbnail {
    #[serde(default)]
    pub source: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ContentUrls {
    #[serde(default)]
    pub desktop: Option<UrlsByPlatform>,
    #[serde(default)]
    pub mobile: Option<UrlsByPlatform>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UrlsByPlatform {
    pub page: String,
}

impl PageSummary {
    pub fn url(&self) -> Option<String> {
        self.content_urls
            .as_ref()
            .and_then(|c| c.desktop.as_ref().or(c.mobile.as_ref()))
            .map(|p| p.page.clone())
    }

    /// Remote URL of the page's thumbnail image (typically `upload.wikimedia.org/...`). Returns
    /// `None` when the REST API omits the thumbnail or returns an empty source. Callers feed
    /// this through [`crate::fetcher::thumbnails::download_to_cache`] before handing the local
    /// path to a `list_cards`-style renderer.
    pub fn thumbnail_url(&self) -> Option<&str> {
        self.thumbnail
            .as_ref()
            .map(|t| t.source.as_str())
            .filter(|s| !s.is_empty())
    }

    /// First sentence of the extract — split at the first `". "` so the Text shape can carry a
    /// preview without the whole multi-paragraph body. Returns `None` when the extract is
    /// missing or empty.
    pub fn first_sentence(&self) -> Option<String> {
        let extract = self.extract.as_deref()?.trim();
        if extract.is_empty() {
            return None;
        }
        let cut = extract.find(". ").map(|i| i + 1).unwrap_or(extract.len());
        Some(extract[..cut].trim().to_string())
    }
}

/// Shape-aware rendering for a single page summary — shared by `wikipedia_featured` and
/// `wikipedia_random` since both fetchers expose the same `{title, extract, url}` triplet.
pub fn render_page_summary(summary: &PageSummary, shape: Shape) -> Body {
    render_page_summary_with_thumbnail(summary, shape, None)
}

/// Render variant that pins a pre-resolved local thumbnail path onto the `ImageLinkedList`
/// shape's single card. Other shapes ignore the thumbnail (Text / TextBlock have no place to
/// surface it). The thumbnail download itself is async, so callers run
/// [`crate::fetcher::thumbnails::download_to_cache`] before calling this and pass the local
/// path in.
pub fn render_page_summary_with_thumbnail(
    summary: &PageSummary,
    shape: Shape,
    thumbnail_path: Option<std::path::PathBuf>,
) -> Body {
    match shape {
        Shape::Text => Body::Text(TextData {
            value: text_line(summary),
        }),
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: text_block_lines(summary),
        }),
        Shape::ImageLinkedList => Body::ImageLinkedList(crate::payload::ImageLinkedListData {
            items: vec![crate::payload::ImageLinkedItem {
                title: summary.title.clone(),
                url: summary.url(),
                thumbnail_path: thumbnail_path.map(|p| p.to_string_lossy().into_owned()),
                subtitle: summary.first_sentence(),
            }],
        }),
        _ => Body::LinkedTextBlock(LinkedTextBlockData {
            items: vec![LinkedLine {
                text: summary.title.clone(),
                url: summary.url(),
            }],
        }),
    }
}

fn text_line(summary: &PageSummary) -> String {
    match summary.first_sentence() {
        Some(sentence) => format!("{}: {sentence}", summary.title),
        None => summary.title.clone(),
    }
}

fn text_block_lines(summary: &PageSummary) -> Vec<String> {
    let mut lines = vec![summary.title.clone()];
    if let Some(extract) = summary.extract.as_deref().filter(|s| !s.is_empty()) {
        for line in extract.lines().filter(|l| !l.trim().is_empty()) {
            lines.push(line.to_string());
        }
    }
    lines
}

