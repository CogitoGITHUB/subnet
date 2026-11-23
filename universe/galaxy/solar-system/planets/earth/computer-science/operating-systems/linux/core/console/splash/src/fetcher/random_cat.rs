//! `random_cat` — download a random cat image from cataas.com and expose it as `Body::Image`.
//!
//! Safety::Safe: the host is hardcoded (`cataas.com`), so config can't redirect traffic to a
//! different origin. No auth is needed, so no token can leak. Mirrors `github_avatar`'s
//! "fixed-host fetcher stays Safe" stance — see `project_trust_model.md`.
//!
//! Caching: bytes are written to `$SPLASH_HOME/cache/cats/<cache_key>.<ext>`. Each
//! `refresh_interval` tick produces a new cat (the API returns a different image per request).
//! The framework gates re-downloads via the cache TTL.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;

use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::paths;
use crate::payload::{Body, ImageData, Payload};
use crate::render::Shape;

const CATAAS_BASE: &str = "https://cataas.com";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const USER_AGENT: &str = concat!("splashboard/", env!("CARGO_PKG_VERSION"));
/// Cap response bytes so a hostile / runaway response can't OOM the daemon. Cat photos
/// are typically 50–500 KB — 5 MB is plenty of headroom while still bounded.
const MAX_BYTES: usize = 5 * 1024 * 1024;

const SHAPES: &[Shape] = &[Shape::Image];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "tag",
    type_hint: "string",
    required: false,
    default: None,
    description: "cataas.com tag filter (e.g. `cute`, `orange`, `kitten`). Omit for any cat.",
}];

pub struct RandomCatFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub tag: Option<String>,
}

#[async_trait]
impl Fetcher for RandomCatFetcher {
    fn name(&self) -> &str {
        "random_cat"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "A random cat picture from cataas.com, downloaded each refresh cycle and rendered as an image. Optional `tag` filters by mood (`cute` / `orange` / `kitten` / …)."
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
    fn sample_body(&self, _shape: Shape) -> Option<Body> {
        None
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let key = self.cache_key(ctx);
        let path = download_cat(opts.tag.as_deref(), &key).await?;
        Ok(payload(Body::Image(ImageData {
            path: path.to_string_lossy().into_owned(),
        })))
    }
}

async fn download_cat(tag: Option<&str>, key: &str) -> Result<PathBuf, FetchError> {
    let out_dir =
        cat_dir().ok_or_else(|| FetchError::Failed("$HOME not available for cat cache".into()))?;
    std::fs::create_dir_all(&out_dir)
        .map_err(|e| FetchError::Failed(format!("create cat cache dir: {e}")))?;
    let url = build_url(tag);
    let bytes = fetch_bytes(&url).await?;
    let ext = image_extension(&bytes)
        .ok_or_else(|| FetchError::Failed("unrecognized image format from cataas".into()))?;
    remove_stale(&out_dir, key);
    let path = out_dir.join(format!("{key}.{ext}"));
    std::fs::write(&path, &bytes).map_err(|e| FetchError::Failed(format!("write cat: {e}")))?;
    Ok(path)
}

fn build_url(tag: Option<&str>) -> String {
    match tag.map(str::trim).filter(|s| !s.is_empty()) {
        Some(t) => format!("{CATAAS_BASE}/cat/{}", encode_tag(t)),
        None => format!("{CATAAS_BASE}/cat"),
    }
}

/// Percent-encode the tag so spaces / commas / other URL-significant chars don't break the path.
/// cataas.com accepts comma-joined tags (`cute,fat`) but we treat the option as a single token —
/// the user can pass `"cute,fat"` and it'll be encoded transparently.
fn encode_tag(tag: &str) -> String {
    tag.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b',' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

async fn fetch_bytes(url: &str) -> Result<Vec<u8>, FetchError> {
    let res = http()
        .get(url)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("cat request failed: {e}")))?;
    let status = res.status();
    if !status.is_success() {
        return Err(FetchError::Failed(format!("cat {status}")));
    }
    let bytes = res
        .bytes()
        .await
        .map_err(|e| FetchError::Failed(format!("cat body: {e}")))?;
    if bytes.len() > MAX_BYTES {
        return Err(FetchError::Failed(format!(
            "cat body too large: {} bytes",
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

fn cat_dir() -> Option<PathBuf> {
    paths::cache_dir().map(|d| d.join("cats"))
}

/// Remove any prior `<key>.<ext>` for this cache key so format-changing refreshes
/// (PNG → JPG) don't leave a misleading stale file with a different extension.
fn remove_stale(dir: &std::path::Path, key: &str) {
    for ext in ["png", "jpg", "gif", "webp"] {
        let _ = std::fs::remove_file(dir.join(format!("{key}.{ext}")));
    }
}

