//! `random_dog` — fetch a random dog photo from dog.ceo and expose it as `Body::Image`.
//!
//! Safety::Safe: the API host (`dog.ceo`) and the image CDN host (`images.dog.ceo`) are both
//! hardcoded. The two-step flow (JSON → image URL) deliberately re-validates the host of the
//! returned URL so a future API change can't redirect us to an attacker-controlled origin.
//!
//! Caching: bytes are written to `$SPLASH_HOME/cache/dogs/<cache_key>.<ext>`. Each
//! `refresh_interval` tick produces a new dog. Mirrors `random_cat`'s shape, options struct,
//! magic-byte detection, and stale-extension cleanup.

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

const API_BASE: &str = "https://dog.ceo/api";
const ALLOWED_IMAGE_HOSTS: &[&str] = &["images.dog.ceo", "dog.ceo"];
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const USER_AGENT: &str = concat!("splashboard/", env!("CARGO_PKG_VERSION"));
/// Cap response bytes so a hostile / runaway response can't OOM the daemon. Dog photos
/// from dog.ceo are typically 50–500 KB; 5 MB is plenty of headroom.
const MAX_BYTES: usize = 5 * 1024 * 1024;

const SHAPES: &[Shape] = &[Shape::Image];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "breed",
        type_hint: "string",
        required: false,
        default: None,
        description: "dog.ceo breed slug (e.g. `shiba`, `husky`, `golden`). Omit for any breed.",
    },
    OptionSchema {
        name: "sub_breed",
        type_hint: "string",
        required: false,
        default: None,
        description: "dog.ceo sub-breed slug (e.g. `cocker` under `breed = \"spaniel\"`). Requires `breed`.",
    },
];

pub struct RandomDogFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub breed: Option<String>,
    #[serde(default)]
    pub sub_breed: Option<String>,
}

#[async_trait]
impl Fetcher for RandomDogFetcher {
    fn name(&self) -> &str {
        "random_dog"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "A random dog photo from dog.ceo, downloaded each refresh cycle and rendered as an image. Optional `breed` (e.g. `shiba`, `husky`) and `sub_breed` (with `breed`) narrow the pool."
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
        validate_options(&opts).map_err(FetchError::Failed)?;
        let key = self.cache_key(ctx);
        let path = download_dog(opts.breed.as_deref(), opts.sub_breed.as_deref(), &key).await?;
        Ok(payload(Body::Image(ImageData {
            path: path.to_string_lossy().into_owned(),
        })))
    }
}

fn validate_options(opts: &Options) -> Result<(), String> {
    let has_breed = opts
        .breed
        .as_deref()
        .map(str::trim)
        .is_some_and(|s| !s.is_empty());
    let has_sub = opts
        .sub_breed
        .as_deref()
        .map(str::trim)
        .is_some_and(|s| !s.is_empty());
    if has_sub && !has_breed {
        return Err("sub_breed requires breed".into());
    }
    Ok(())
}

async fn download_dog(
    breed: Option<&str>,
    sub_breed: Option<&str>,
    key: &str,
) -> Result<PathBuf, FetchError> {
    let out_dir =
        dog_dir().ok_or_else(|| FetchError::Failed("$HOME not available for dog cache".into()))?;
    std::fs::create_dir_all(&out_dir)
        .map_err(|e| FetchError::Failed(format!("create dog cache dir: {e}")))?;
    let api_url = build_api_url(breed, sub_breed);
    let image_url = fetch_image_url(&api_url).await?;
    enforce_allowed_host(&image_url)?;
    let bytes = fetch_bytes(&image_url).await?;
    let ext = image_extension(&bytes)
        .ok_or_else(|| FetchError::Failed("unrecognized image format from dog.ceo".into()))?;
    remove_stale(&out_dir, key);
    let path = out_dir.join(format!("{key}.{ext}"));
    std::fs::write(&path, &bytes).map_err(|e| FetchError::Failed(format!("write dog: {e}")))?;
    Ok(path)
}

fn build_api_url(breed: Option<&str>, sub_breed: Option<&str>) -> String {
    let breed = breed.map(str::trim).filter(|s| !s.is_empty());
    let sub_breed = sub_breed.map(str::trim).filter(|s| !s.is_empty());
    match (breed, sub_breed) {
        (Some(b), Some(sb)) => format!(
            "{API_BASE}/breed/{}/{}/images/random",
            encode_segment(b),
            encode_segment(sb)
        ),
        (Some(b), None) => format!("{API_BASE}/breed/{}/images/random", encode_segment(b)),
        (None, _) => format!("{API_BASE}/breeds/image/random"),
    }
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

#[derive(Deserialize)]
struct ApiResponse {
    message: String,
    status: String,
}

async fn fetch_image_url(api_url: &str) -> Result<String, FetchError> {
    let res = http()
        .get(api_url)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("dog API request failed: {e}")))?;
    let status = res.status();
    if !status.is_success() {
        return Err(FetchError::Failed(format!("dog API {status}")));
    }
    let body: ApiResponse = res
        .json()
        .await
        .map_err(|e| FetchError::Failed(format!("dog API body: {e}")))?;
    if body.status != "success" {
        return Err(FetchError::Failed(format!(
            "dog API non-success status: {}",
            body.status
        )));
    }
    Ok(body.message)
}

/// Defense in depth: the API returns an arbitrary URL string in `message`. If dog.ceo ever
/// changes infrastructure or is compromised, we still refuse to follow off-host links rather
/// than giving the API a free SSRF primitive into our fetcher.
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
            "dog API returned off-host image URL: {url}"
        )))
    }
}

async fn fetch_bytes(url: &str) -> Result<Vec<u8>, FetchError> {
    let res = http()
        .get(url)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("dog image request failed: {e}")))?;
    let status = res.status();
    if !status.is_success() {
        return Err(FetchError::Failed(format!("dog image {status}")));
    }
    let bytes = res
        .bytes()
        .await
        .map_err(|e| FetchError::Failed(format!("dog image body: {e}")))?;
    if bytes.len() > MAX_BYTES {
        return Err(FetchError::Failed(format!(
            "dog image too large: {} bytes",
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

fn dog_dir() -> Option<PathBuf> {
    paths::cache_dir().map(|d| d.join("dogs"))
}

fn remove_stale(dir: &std::path::Path, key: &str) {
    for ext in ["png", "jpg", "gif", "webp"] {
        let _ = std::fs::remove_file(dir.join(format!("{key}.{ext}")));
    }
}

