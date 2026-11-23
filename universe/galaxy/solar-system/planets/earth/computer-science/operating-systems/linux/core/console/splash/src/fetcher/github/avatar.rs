//! `github_avatar` — download an avatar PNG and expose it as `Body::Image`.
//!
//! - github.com: keeps the original public shortcut (`https://github.com/<user>.png`).
//! - GHE: the public shortcut redirects to `/login`, so we indirect through
//!   `/users/<login>` to get the server-signed `avatar_url`.
//!
//! Safety::Safe: the host is resolved via `client::host()` (env-var-only), so a malicious
//! `.splash.toml` cannot redirect traffic to an attacker-chosen origin.
//!
//! Caching: the PNG is written to `$SPLASH_HOME/cache/avatars/<user>-<size>.png`. The
//! fetcher framework already gates re-downloads via `refresh_interval`; we default to one day
//! since avatars rarely change.

use std::path::PathBuf;

use async_trait::async_trait;
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::paths;
use crate::payload::{Body, ImageData, Payload};
use crate::render::Shape;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::client::{host, http, resolve_authenticated_user, rest_get, web_base};
use super::common::cache_key;

const DEFAULT_SIZE: u32 = 200;

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "user",
        type_hint: "string (github login)",
        required: false,
        default: Some("authenticated token user"),
        description: "GitHub login to fetch the avatar for. Falls back to the `GITHUB_USER` env var, then to the login that owns `GH_TOKEN` / `GITHUB_TOKEN`.",
    },
    OptionSchema {
        name: "size",
        type_hint: "integer (px)",
        required: false,
        default: Some("200"),
        description: "Requested avatar edge size in pixels. GitHub serves a square image of this size.",
    },
];

/// Downloads the user's avatar PNG to the cache and emits an `Image` body pointing at it.
pub struct GithubAvatar;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub size: Option<u32>,
}

#[async_trait]
impl Fetcher for GithubAvatar {
    fn name(&self) -> &str {
        "github_avatar"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "A GitHub user's avatar PNG, downloaded once a day and rendered as an image. Useful as the visual hero next to a `github_user` text block."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 60 * 24
    }
    fn shapes(&self) -> &[Shape] {
        &[Shape::Image]
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
        let user = resolve_user(opts.user.as_deref())
            .await
            .map_err(FetchError::Failed)?;
        let size = opts.size.unwrap_or(DEFAULT_SIZE);
        let path = download_avatar(&user, size).await?;
        Ok(payload(Body::Image(ImageData {
            path: path.to_string_lossy().into_owned(),
        })))
    }
}

async fn download_avatar(user: &str, size: u32) -> Result<PathBuf, FetchError> {
    let out_dir = avatar_dir()
        .ok_or_else(|| FetchError::Failed("$HOME not available for avatar cache".into()))?;
    std::fs::create_dir_all(&out_dir)
        .map_err(|e| FetchError::Failed(format!("create avatar cache dir: {e}")))?;
    let url = if host().eq_ignore_ascii_case("github.com") {
        format!("{}/{user}.png?size={size}", web_base())
    } else {
        resolve_ghe_avatar_url(user, size).await?
    };
    let res = http()
        .get(&url)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("avatar request failed: {e}")))?;
    let status = res.status();
    if !status.is_success() {
        return Err(FetchError::Failed(format!("avatar {status}")));
    }
    let bytes = res
        .bytes()
        .await
        .map_err(|e| FetchError::Failed(format!("avatar body: {e}")))?;
    // GitHub serves whichever image format the user uploaded, even when the URL ends in `.png`.
    // The image crate decodes by extension hint, so we need to write the file with the right
    // suffix or decoding fails.
    let ext = image_extension(&bytes);
    let path = out_dir.join(format!("{user}-{size}.{ext}"));
    std::fs::write(&path, &bytes).map_err(|e| FetchError::Failed(format!("write avatar: {e}")))?;
    Ok(path)
}

#[derive(Debug, Deserialize)]
struct UserAvatar {
    avatar_url: String,
}

/// GHE-only: resolve the server-signed avatar URL via `/users/<login>` and append a size
/// query so the source is small enough for the media_image renderer.
async fn resolve_ghe_avatar_url(user: &str, size: u32) -> Result<String, FetchError> {
    let me: UserAvatar = rest_get(&format!("/users/{user}")).await?;
    let sep = if me.avatar_url.contains('?') {
        '&'
    } else {
        '?'
    };
    Ok(format!("{}{sep}s={size}", me.avatar_url))
}

fn image_extension(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "png"
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        "jpg"
    } else {
        "png"
    }
}

fn avatar_dir() -> Option<PathBuf> {
    paths::cache_dir().map(|d| d.join("avatars"))
}

async fn resolve_user(explicit: Option<&str>) -> Result<String, String> {
    if let Some(u) = explicit.filter(|s| !s.is_empty()) {
        return Ok(u.into());
    }
    if let Ok(u) = std::env::var("GITHUB_USER")
        && !u.is_empty()
    {
        return Ok(u);
    }
    resolve_authenticated_user()
        .await
        .map_err(|e| format!("resolve github user: {e}"))
}

fn parse_options<T: serde::de::DeserializeOwned + Default>(
    raw: Option<&toml::Value>,
) -> Result<T, String> {
    match raw {
        None => Ok(T::default()),
        Some(value) => value
            .clone()
            .try_into::<T>()
            .map_err(|e| format!("invalid options: {e}")),
    }
}

fn payload(body: Body) -> Payload {
    Payload {
        icon: None,
        status: None,
        format: None,
        body,
    }
}

