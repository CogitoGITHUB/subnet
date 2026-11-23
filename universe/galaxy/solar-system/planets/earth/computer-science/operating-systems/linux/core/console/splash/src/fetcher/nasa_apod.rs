//! `nasa_apod` — NASA's Astronomy Picture of the Day, exposed as an image plus its metadata.
//!
//! Safety::Safe: the API host (`api.nasa.gov`) is hardcoded and the optional `api_key` only
//! ever leaves to that known host — the `github_*` "fixed-host authenticated fetcher stays
//! Safe" rule. The JSON response carries an arbitrary image URL string; like `random_dog` we
//! re-validate its host against an allowlist so a future API change can't redirect the image
//! download off-host.
//!
//! Multi-shape: `Image` (default) downloads the picture; `Text` / `TextBlock` /
//! `MarkdownTextBlock` / `LinkedTextBlock` / `Entries` reshape the same single API read into
//! the title / explanation / date metadata. On video days (APOD posts a video roughly weekly)
//! the `Image` shape errors out while the text shapes still render.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;

use crate::fetcher::github::common::{cache_key, parse_options, payload, text_block_body};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::paths;
use crate::payload::{
    Body, EntriesData, Entry, ImageData, LinkedLine, LinkedTextBlockData, MarkdownTextBlockData,
    Payload, TextData,
};
use crate::render::Shape;
use crate::samples;

const API_BASE: &str = "https://api.nasa.gov/planetary/apod";
const ALLOWED_IMAGE_HOSTS: &[&str] = &["apod.nasa.gov", "www.nasa.gov", "science.nasa.gov"];
const DEFAULT_API_KEY: &str = "DEMO_KEY";
const API_KEY_ENV: &str = "NASA_API_KEY";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const USER_AGENT: &str = concat!("splashboard/", env!("CARGO_PKG_VERSION"));
/// Cap response bytes so a hostile / runaway response can't OOM the daemon. APOD's standard-res
/// `url` images run a few hundred KB to a couple MB; 10 MB is generous headroom while bounded.
const MAX_BYTES: usize = 10 * 1024 * 1024;

const SHAPES: &[Shape] = &[
    Shape::Image,
    Shape::Text,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::LinkedTextBlock,
    Shape::Entries,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "api_key",
    type_hint: "string",
    required: false,
    default: Some("\"DEMO_KEY\""),
    description: "NASA API key. Falls back to the `NASA_API_KEY` env var, then to the shared `DEMO_KEY` (rate-limited but fine for a once-daily splash). Get a free key at api.nasa.gov.",
}];

pub struct NasaApodFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub api_key: Option<String>,
}

#[async_trait]
impl Fetcher for NasaApodFetcher {
    fn name(&self) -> &str {
        "nasa_apod"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "NASA's Astronomy Picture of the Day — a fresh curated space photograph each day. The default `Image` shape renders the picture; text shapes surface the title, explanation, and date. On the occasional video day the image shape errors while text shapes still render."
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
            Shape::Image => return None,
            Shape::Text => samples::text("Pillars of Creation"),
            Shape::TextBlock => samples::text_block(&[
                "Pillars of Creation",
                "These columns of cool interstellar gas and dust in the Eagle Nebula are incubators for new stars.",
            ]),
            Shape::MarkdownTextBlock => samples::markdown(
                "# Pillars of Creation\n\nThese columns of cool interstellar gas and dust in the Eagle Nebula are incubators for new stars.",
            ),
            Shape::LinkedTextBlock => samples::linked_text_block(&[(
                "Pillars of Creation",
                Some("https://apod.nasa.gov/apod/ap240115.html"),
            )]),
            Shape::Entries => samples::entries(&[
                ("date", "2024-01-15"),
                ("title", "Pillars of Creation"),
                ("media", "image"),
                ("copyright", "NASA, ESA, CSA, STScI"),
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let api_key = resolve_api_key(opts.api_key.as_deref());
        let apod = fetch_apod(&build_api_url(&api_key)).await?;
        let shape = ctx.shape.unwrap_or(Shape::Image);
        let body = match shape {
            Shape::Image => image_body(&apod, &self.cache_key(ctx)).await?,
            other => text_body(&apod, other).ok_or_else(|| {
                FetchError::Failed(format!("nasa_apod can't emit {}", other.as_str()))
            })?,
        };
        Ok(payload(body))
    }
}

/// Reshape a fetched APOD into one of the non-image shapes. `None` for `Image` (handled
/// separately, since it needs an async download) or any shape outside `SHAPES`. Pure and sync
/// so every text-shape branch is testable without a network round-trip.
fn text_body(apod: &ApiResponse, shape: Shape) -> Option<Body> {
    Some(match shape {
        Shape::Text => Body::Text(TextData {
            value: apod.title.clone(),
        }),
        Shape::TextBlock => text_block_body(vec![apod.title.clone(), apod.explanation.clone()]),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: format!("# {}\n\n{}", apod.title, apod.explanation),
        }),
        Shape::LinkedTextBlock => Body::LinkedTextBlock(LinkedTextBlockData {
            items: vec![LinkedLine {
                text: apod.title.clone(),
                url: apod_page_url(&apod.date),
            }],
        }),
        Shape::Entries => entries_body(apod),
        _ => return None,
    })
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    date: String,
    title: String,
    explanation: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    media_type: String,
    #[serde(default)]
    copyright: Option<String>,
}

fn resolve_api_key(explicit: Option<&str>) -> String {
    let from_opt = explicit.map(str::trim).filter(|s| !s.is_empty());
    let from_env = || {
        std::env::var(API_KEY_ENV)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    from_opt
        .map(String::from)
        .or_else(from_env)
        .unwrap_or_else(|| DEFAULT_API_KEY.to_string())
}

fn build_api_url(api_key: &str) -> String {
    format!("{API_BASE}?api_key={}", encode_segment(api_key))
}

fn encode_segment(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// GET + parse the APOD JSON. Takes the fully-built URL (rather than the api key) so tests can
/// point it at a local server — mirrors `random_dog`'s `fetch_image_url`.
async fn fetch_apod(url: &str) -> Result<ApiResponse, FetchError> {
    let res = http()
        .get(url)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("apod API request failed: {e}")))?;
    let status = res.status();
    if !status.is_success() {
        return Err(FetchError::Failed(format!("apod API {status}")));
    }
    res.json::<ApiResponse>()
        .await
        .map_err(|e| FetchError::Failed(format!("apod API body: {e}")))
}

async fn image_body(apod: &ApiResponse, key: &str) -> Result<Body, FetchError> {
    if apod.media_type != "image" {
        return Err(FetchError::Failed(format!(
            "APOD is a {} today: {}",
            apod.media_type, apod.title
        )));
    }
    let path = download_image(&apod.url, key).await?;
    Ok(Body::Image(ImageData {
        path: path.to_string_lossy().into_owned(),
    }))
}

async fn download_image(image_url: &str, key: &str) -> Result<PathBuf, FetchError> {
    let out_dir = apod_dir()
        .ok_or_else(|| FetchError::Failed("$HOME not available for apod cache".into()))?;
    std::fs::create_dir_all(&out_dir)
        .map_err(|e| FetchError::Failed(format!("create apod cache dir: {e}")))?;
    enforce_allowed_host(image_url)?;
    let bytes = fetch_bytes(image_url).await?;
    let ext = image_extension(&bytes)
        .ok_or_else(|| FetchError::Failed("unrecognized image format from apod".into()))?;
    remove_stale(&out_dir, key);
    let path = out_dir.join(format!("{key}.{ext}"));
    std::fs::write(&path, &bytes).map_err(|e| FetchError::Failed(format!("write apod: {e}")))?;
    Ok(path)
}

/// Defense in depth: the API hands us an arbitrary URL string in `url`. If `api.nasa.gov` ever
/// changes infrastructure or is compromised, we still refuse to follow off-host links rather
/// than handing the API a free SSRF primitive into our image downloader.
fn enforce_allowed_host(url: &str) -> Result<(), FetchError> {
    let host = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .and_then(|rest| rest.split('/').next())
        .unwrap_or("");
    if ALLOWED_IMAGE_HOSTS.contains(&host) {
        Ok(())
    } else {
        Err(FetchError::Failed(format!(
            "apod API returned off-host image URL: {url}"
        )))
    }
}

async fn fetch_bytes(url: &str) -> Result<Vec<u8>, FetchError> {
    let res = http()
        .get(url)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("apod image request failed: {e}")))?;
    let status = res.status();
    if !status.is_success() {
        return Err(FetchError::Failed(format!("apod image {status}")));
    }
    let bytes = res
        .bytes()
        .await
        .map_err(|e| FetchError::Failed(format!("apod image body: {e}")))?;
    if bytes.len() > MAX_BYTES {
        return Err(FetchError::Failed(format!(
            "apod image too large: {} bytes",
            bytes.len()
        )));
    }
    Ok(bytes.to_vec())
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

/// APOD archive page for a `YYYY-MM-DD` date: `https://apod.nasa.gov/apod/apYYMMDD.html`.
/// Returns `None` for any date that isn't the expected all-digit `YYYY-MM-DD` shape.
fn apod_page_url(date: &str) -> Option<String> {
    let parts: Vec<&str> = date.split('-').collect();
    let [year, month, day] = parts.as_slice() else {
        return None;
    };
    let well_formed = year.len() == 4
        && month.len() == 2
        && day.len() == 2
        && [year, month, day]
            .iter()
            .all(|p| p.bytes().all(|b| b.is_ascii_digit()));
    if !well_formed {
        return None;
    }
    Some(format!(
        "https://apod.nasa.gov/apod/ap{}{month}{day}.html",
        &year[2..]
    ))
}

fn entries_body(apod: &ApiResponse) -> Body {
    let mut items = vec![
        entry("date", &apod.date),
        entry("title", &apod.title),
        entry("media", &apod.media_type),
    ];
    if let Some(copyright) = apod
        .copyright
        .as_deref()
        .map(clean_copyright)
        .filter(|s| !s.is_empty())
    {
        items.push(entry("copyright", &copyright));
    }
    Body::Entries(EntriesData { items })
}

fn entry(key: &str, value: &str) -> Entry {
    Entry {
        key: key.into(),
        value: Some(value.into()),
        status: None,
    }
}

/// APOD's `copyright` field is notoriously full of stray newlines and runs of spaces. Collapse
/// any whitespace run to a single space so it renders as one tidy line.
fn clean_copyright(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn image_extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("jpg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

fn apod_dir() -> Option<PathBuf> {
    paths::cache_dir().map(|d| d.join("apod"))
}

/// Remove any prior `<key>.<ext>` for this cache key so a format-changing refresh
/// (JPG → PNG) doesn't leave a misleading stale file with a different extension.
fn remove_stale(dir: &std::path::Path, key: &str) {
    for ext in ["png", "jpg", "gif", "webp"] {
        let _ = std::fs::remove_file(dir.join(format!("{key}.{ext}")));
    }
}

