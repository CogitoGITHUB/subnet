//! Shared HTTP client + auth for the `gitlab_*` family. GitLab uses `PRIVATE-TOKEN` (Personal
//! Access Token) rather than bearer auth as its canonical PAT header; we honour that so users
//! can paste a token from the GitLab UI directly into `secrets.toml`.
//!
//! Every request flows through [`rest_get`], which composes `https://{host}/api/v4{path}` —
//! `host` is the caller-supplied (and pre-validated) GitLab instance, defaulting to
//! `gitlab.com`. The fetcher resolves and validates the host before reaching this module, so we
//! treat it as already-safe here.

use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::fetcher::FetchError;

const USER_AGENT: &str = concat!("splashboard/", env!("CARGO_PKG_VERSION"));
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_HOST: &str = "gitlab.com";

static AUTHENTICATED_USERNAME_CACHE: OnceLock<Mutex<Option<(String, String)>>> = OnceLock::new();

pub fn default_host() -> &'static str {
    DEFAULT_HOST
}

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

pub fn resolve_token() -> Result<String, FetchError> {
    std::env::var("GITLAB_TOKEN").map_err(|_| FetchError::Failed("GITLAB_TOKEN not set".into()))
}

/// Username of the token-authenticated user. Resolved lazily via `/api/v4/user` and memoised
/// per-host so the `gitlab_review_requests` widget doesn't pay a roundtrip on every refresh.
/// Keyed on `host` because two configs pointing at different GitLab instances must not bleed
/// usernames into each other.
pub async fn resolve_authenticated_username(host: &str) -> Result<String, FetchError> {
    let slot = AUTHENTICATED_USERNAME_CACHE.get_or_init(|| Mutex::new(None));
    if let Some(cached) = slot
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .filter(|(h, _)| h == host)
    {
        return Ok(cached.1);
    }
    #[derive(Deserialize)]
    struct Me {
        username: String,
    }
    let me: Me = rest_get(host, "/user").await?;
    if let Ok(mut g) = slot.lock() {
        *g = Some((host.into(), me.username.clone()));
    }
    Ok(me.username)
}

#[cfg(test)]
pub(crate) fn clear_authenticated_username_cache() {
    if let Ok(mut g) = AUTHENTICATED_USERNAME_CACHE
        .get_or_init(|| Mutex::new(None))
        .lock()
    {
        *g = None;
    }
}

/// REST GET → deserialize JSON. `path` is the part after `/api/v4` (e.g. `"/user"`,
/// `"/projects/group%2Fproj"`). Non-2xx responses surface the GitLab `message` field when
/// present so the error placeholder and the log line are both actionable.
pub async fn rest_get<T: DeserializeOwned>(host: &str, path: &str) -> Result<T, FetchError> {
    let token = resolve_token()?;
    let url = format!("https://{host}/api/v4{path}");
    let res = http()
        .get(&url)
        .header("PRIVATE-TOKEN", &token)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("gitlab request failed: {e}")))?;
    parse_json(res).await
}

async fn parse_json<T: DeserializeOwned>(res: reqwest::Response) -> Result<T, FetchError> {
    let status = res.status();
    let bytes = res
        .bytes()
        .await
        .map_err(|e| FetchError::Failed(format!("gitlab response body: {e}")))?;
    if !status.is_success() {
        return Err(FetchError::Failed(error_message(status, &bytes)));
    }
    serde_json::from_slice(&bytes)
        .map_err(|e| FetchError::Failed(format!("gitlab json parse: {e}")))
}

fn error_message(status: StatusCode, body: &[u8]) -> String {
    #[derive(Deserialize)]
    struct ApiError {
        message: Option<serde_json::Value>,
        error: Option<String>,
    }
    let reported = serde_json::from_slice::<ApiError>(body).ok().and_then(|e| {
        e.message
            .as_ref()
            .map(|m| match m {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .or(e.error)
    });
    match reported {
        Some(m) => format!("gitlab {status}: {m}"),
        None => format!("gitlab {status}"),
    }
}

