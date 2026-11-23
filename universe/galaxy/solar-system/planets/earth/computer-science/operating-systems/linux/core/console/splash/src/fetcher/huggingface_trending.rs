//! `huggingface_trending` — trending models / datasets / spaces on Hugging Face Hub.
//!
//! Safety::Safe: API host (`huggingface.co`) is hardcoded; the user supplies only `kind`
//! (closed enum) and `count`. Thumbnails are fetched as a best-effort per-author lookup
//! against the same fixed host; failed lookups silently leave a row's thumbnail empty
//! rather than poisoning the whole feed.
//!
//! HF's `?sort=trendingScore&direction=-1` is the documented "Trending" sort on the public
//! Hub API (no auth required), so this fetcher mirrors what huggingface.co/{models,datasets,
//! spaces} shows under the "Trending" tab.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{
    Bar, BarsData, Body, EntriesData, Entry, ImageLinkedItem, ImageLinkedListData, LinkedLine,
    LinkedTextBlockData, MarkdownTextBlockData, Payload, TextBlockData, TextData,
};
use crate::render::Shape;

use super::github::common::{cache_key, parse_options, payload};
use super::thumbnails;
use super::{FetchContext, FetchError, Fetcher, Safety};

const API_BASE: &str = "https://huggingface.co/api";
const SITE_BASE: &str = "https://huggingface.co";
const USER_AGENT: &str = concat!("splashboard/", env!("CARGO_PKG_VERSION"));
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_BYTES: usize = 5 * 1024 * 1024;

const DEFAULT_COUNT: u32 = 10;
const MIN_COUNT: u32 = 1;
const MAX_COUNT: u32 = 30;

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
        name: "kind",
        type_hint: "\"models\" | \"datasets\" | \"spaces\"",
        required: false,
        default: Some("\"models\""),
        description: "Which Hub catalog to rank.",
    },
    OptionSchema {
        name: "count",
        type_hint: "integer (1..=30)",
        required: false,
        default: Some("10"),
        description: "Number of trending entries to display.",
    },
];

pub struct HuggingfaceTrendingFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub kind: Option<Kind>,
    #[serde(default)]
    pub count: Option<u32>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Models,
    Datasets,
    Spaces,
}

impl Kind {
    fn path(self) -> &'static str {
        match self {
            Self::Models => "models",
            Self::Datasets => "datasets",
            Self::Spaces => "spaces",
        }
    }

    /// Path used to build a clickable Hub page URL — same as the API path for all three.
    fn site_path(self) -> &'static str {
        self.path()
    }
}

#[async_trait]
impl Fetcher for HuggingfaceTrendingFetcher {
    fn name(&self) -> &str {
        "huggingface_trending"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Trending models, datasets, or spaces on Hugging Face Hub via the public `?sort=trendingScore` API (no auth required). Companion to the planned `github_trending` / `wikipedia_trending` family — each surface picks its own list, this one is HF's."
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
        let kind = opts.kind.unwrap_or_default();
        let count = opts
            .count
            .unwrap_or(DEFAULT_COUNT)
            .clamp(MIN_COUNT, MAX_COUNT) as usize;
        let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);
        let entries = fetch_trending(kind, count).await?;
        let body = render_for_shape(&entries, kind, shape).await;
        Ok(payload(body))
    }
}

/// Trending Hub entry as the renderer wants it. `position` is the 0-indexed slot in HF's
/// response (= trending rank); HF's own `trendingScore` is a float we display rather than
/// re-rank on. `id` is the canonical `author/repo` slug — we split off `author` for the
/// per-author avatar lookup that powers `ImageLinkedList`.
#[derive(Debug, Clone)]
struct Entrt {
    id: String,
    likes: u64,
    downloads: Option<u64>,
    pipeline_tag: Option<String>,
    position: usize,
}

impl Entrt {
    fn author(&self) -> Option<&str> {
        self.id.split_once('/').map(|(a, _)| a)
    }

    fn url(&self, kind: Kind) -> String {
        match kind {
            // Models live at the bare `/{id}` slug; datasets / spaces are namespaced.
            Kind::Models => format!("{SITE_BASE}/{}", self.id),
            other => format!("{SITE_BASE}/{}/{}", other.site_path(), self.id),
        }
    }

    fn score_line(&self) -> String {
        let likes = format_count(self.likes);
        match self.downloads {
            Some(d) if d > 0 => format!("♥ {likes}  ↓ {}", format_count(d)),
            _ => format!("♥ {likes}"),
        }
    }
}

async fn fetch_trending(kind: Kind, limit: usize) -> Result<Vec<Entrt>, FetchError> {
    let url = format!(
        "{API_BASE}/{}?sort=trendingScore&direction=-1&limit={limit}",
        kind.path()
    );
    let raw: Vec<ApiEntry> = get_json(&url).await?;
    Ok(raw
        .into_iter()
        .enumerate()
        .map(|(i, e)| Entrt {
            id: e.id,
            likes: e.likes.unwrap_or(0).max(0) as u64,
            downloads: e.downloads.map(|d| d.max(0) as u64),
            pipeline_tag: e.pipeline_tag,
            position: i,
        })
        .collect())
}

async fn get_json<T: serde::de::DeserializeOwned>(url: &str) -> Result<T, FetchError> {
    let res = http()
        .get(url)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("huggingface request failed: {e}")))?;
    let status = res.status();
    if !status.is_success() {
        return Err(FetchError::Failed(format!("huggingface {status}")));
    }
    let bytes = res
        .bytes()
        .await
        .map_err(|e| FetchError::Failed(format!("huggingface read body: {e}")))?;
    if bytes.len() > MAX_BYTES {
        return Err(FetchError::Failed(format!(
            "huggingface response too large ({} bytes, cap {MAX_BYTES})",
            bytes.len()
        )));
    }
    serde_json::from_slice(&bytes)
        .map_err(|e| FetchError::Failed(format!("huggingface json parse: {e}")))
}

fn http() -> &'static Client {
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

async fn render_for_shape(entries: &[Entrt], kind: Kind, shape: Shape) -> Body {
    if matches!(shape, Shape::ImageLinkedList) {
        let paths = resolve_thumbnails(entries).await;
        return image_linked_body(entries, kind, &paths);
    }
    render_sync(entries, kind, shape)
}

/// Avatar URLs aren't carried in the trending response, so resolve them per-author via the
/// `overview` endpoint. Authors repeat across rows (e.g. `meta-llama` ships several trending
/// models at once) — in-loop dedup so a 20-row feed doesn't fan out 20 lookups when 4
/// distinct authors are involved.
async fn resolve_thumbnails(entries: &[Entrt]) -> Vec<Option<PathBuf>> {
    let mut cache: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::new();
    let mut urls: Vec<Option<String>> = Vec::with_capacity(entries.len());
    for entry in entries {
        let avatar = match entry.author() {
            Some(author) => match cache.get(author) {
                Some(cached) => cached.clone(),
                None => {
                    let resolved = fetch_avatar_url(author).await;
                    cache.insert(author.to_string(), resolved.clone());
                    resolved
                }
            },
            None => None,
        };
        urls.push(avatar);
    }
    thumbnails::download_many(&urls).await
}

/// HF puts users and organizations in the same namespace but exposes them via separate API
/// paths. Try the user endpoint first; on any error fall through to the org endpoint. Either
/// way the failure is silent — a missing avatar shouldn't break the row.
async fn fetch_avatar_url(author: &str) -> Option<String> {
    for kind in ["users", "organizations"] {
        let url = format!("{API_BASE}/{kind}/{author}/overview");
        if let Ok(profile) = get_json::<HfProfile>(&url).await
            && let Some(avatar) = profile.avatar_url.filter(|s| !s.is_empty())
        {
            return Some(avatar);
        }
    }
    None
}

fn render_sync(entries: &[Entrt], kind: Kind, shape: Shape) -> Body {
    match shape {
        Shape::Text => Body::Text(TextData {
            value: headline(entries),
        }),
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: entries.iter().map(row_line).collect(),
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: markdown_body(entries, kind),
        }),
        Shape::Entries => Body::Entries(EntriesData {
            items: entries.iter().map(entry_row).collect(),
        }),
        Shape::Bars => Body::Bars(BarsData {
            bars: entries.iter().map(likes_bar).collect(),
        }),
        _ => Body::LinkedTextBlock(LinkedTextBlockData {
            items: entries
                .iter()
                .map(|e| LinkedLine {
                    text: row_line(e),
                    url: Some(e.url(kind)),
                })
                .collect(),
        }),
    }
}

fn image_linked_body(entries: &[Entrt], kind: Kind, paths: &[Option<PathBuf>]) -> Body {
    Body::ImageLinkedList(ImageLinkedListData {
        items: entries
            .iter()
            .enumerate()
            .map(|(i, e)| ImageLinkedItem {
                title: format!("#{} {}", e.position + 1, e.id),
                url: Some(e.url(kind)),
                thumbnail_path: paths
                    .get(i)
                    .and_then(|p| p.as_ref())
                    .map(|p| p.to_string_lossy().into_owned()),
                subtitle: Some(subtitle_line(e)),
            })
            .collect(),
    })
}

fn headline(entries: &[Entrt]) -> String {
    entries
        .first()
        .map(|e| format!("#1 {}  {}", e.id, e.score_line()))
        .unwrap_or_else(|| "(no trending entries)".into())
}

fn row_line(entry: &Entrt) -> String {
    format!(
        "#{} {}  {}",
        entry.position + 1,
        entry.id,
        entry.score_line()
    )
}

fn subtitle_line(entry: &Entrt) -> String {
    let mut s = entry.score_line();
    if let Some(tag) = &entry.pipeline_tag {
        s.push_str(&format!("  · {tag}"));
    }
    s
}

fn markdown_body(entries: &[Entrt], kind: Kind) -> String {
    entries
        .iter()
        .map(|e| {
            format!(
                "- **#{} {}** — {}  ([page]({}))",
                e.position + 1,
                e.id,
                e.score_line(),
                e.url(kind),
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn entry_row(entry: &Entrt) -> Entry {
    Entry {
        key: format!("#{} {}", entry.position + 1, entry.id),
        value: Some(entry.score_line()),
        status: None,
    }
}

fn likes_bar(entry: &Entrt) -> Bar {
    Bar {
        label: entry.id.clone(),
        value: entry.likes,
        value_label: None,
    }
}

fn format_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn sample_trending_body(shape: Shape) -> Option<Body> {
    Some(match shape {
        Shape::LinkedTextBlock => crate::samples::linked_text_block(&[
            (
                "#1 meta-llama/Llama-3.1-8B-Instruct  ♥ 3.2k  ↓ 2.1M",
                Some("https://huggingface.co/meta-llama/Llama-3.1-8B-Instruct"),
            ),
            (
                "#2 Qwen/Qwen2.5-72B-Instruct  ♥ 1.8k  ↓ 850.0k",
                Some("https://huggingface.co/Qwen/Qwen2.5-72B-Instruct"),
            ),
        ]),
        Shape::ImageLinkedList => crate::samples::image_linked_list(&[
            (
                "#1 meta-llama/Llama-3.1-8B-Instruct",
                Some("https://huggingface.co/meta-llama/Llama-3.1-8B-Instruct"),
                None,
                Some("♥ 3.2k  ↓ 2.1M  · text-generation"),
            ),
            (
                "#2 Qwen/Qwen2.5-72B-Instruct",
                Some("https://huggingface.co/Qwen/Qwen2.5-72B-Instruct"),
                None,
                Some("♥ 1.8k  ↓ 850.0k  · text-generation"),
            ),
        ]),
        Shape::TextBlock => crate::samples::text_block(&[
            "#1 meta-llama/Llama-3.1-8B-Instruct  ♥ 3.2k  ↓ 2.1M",
            "#2 Qwen/Qwen2.5-72B-Instruct  ♥ 1.8k  ↓ 850.0k",
        ]),
        Shape::Text => crate::samples::text("#1 meta-llama/Llama-3.1-8B-Instruct  ♥ 3.2k  ↓ 2.1M"),
        Shape::MarkdownTextBlock => crate::samples::markdown(
            "- **#1 meta-llama/Llama-3.1-8B-Instruct** — ♥ 3.2k  ↓ 2.1M  ([page](https://huggingface.co/meta-llama/Llama-3.1-8B-Instruct))\n- **#2 Qwen/Qwen2.5-72B-Instruct** — ♥ 1.8k  ↓ 850.0k  ([page](https://huggingface.co/Qwen/Qwen2.5-72B-Instruct))",
        ),
        Shape::Entries => crate::samples::entries(&[
            ("#1 meta-llama/Llama-3.1-8B-Instruct", "♥ 3.2k  ↓ 2.1M"),
            ("#2 Qwen/Qwen2.5-72B-Instruct", "♥ 1.8k  ↓ 850.0k"),
        ]),
        Shape::Bars => crate::samples::bars(&[
            ("meta-llama/Llama-3.1-8B-Instruct", 3_200),
            ("Qwen/Qwen2.5-72B-Instruct", 1_800),
        ]),
        _ => return None,
    })
}

#[derive(Debug, Deserialize)]
struct ApiEntry {
    id: String,
    #[serde(default)]
    likes: Option<i64>,
    #[serde(default)]
    downloads: Option<i64>,
    #[serde(default)]
    pipeline_tag: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HfProfile {
    #[serde(rename = "avatarUrl", default)]
    avatar_url: Option<String>,
}

