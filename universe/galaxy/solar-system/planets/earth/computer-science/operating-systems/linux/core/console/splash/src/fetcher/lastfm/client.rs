//! Shared HTTP client + auth for the `lastfm_*` fetcher family.
//!
//! Every request targets `ws.audioscrobbler.com` — the host is hardcoded, so the API key
//! never leaves to an attacker-controlled origin. Same `Safety::Safe` model as `github_*`:
//! config-provided fields (`user`, `period`, …) only change which resource within the fixed
//! host is queried.
//!
//! Auth: `LASTFM_API_KEY` (free, https://www.last.fm/api/account/create).
//! Last.fm signals errors with HTTP 200 + `{"error": <code>, "message": <msg>}` rather than a
//! non-2xx status, so [`get_json`] inspects the JSON envelope before deserializing into the
//! caller's target type.

use std::sync::OnceLock;
use std::time::Duration;

use reqwest::Client;
use serde::de::DeserializeOwned;

use crate::fetcher::FetchError;

const API_BASE: &str = "https://ws.audioscrobbler.com/2.0/";
const USER_AGENT: &str = concat!("splashboard/", env!("CARGO_PKG_VERSION"));
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_BYTES: usize = 5 * 1024 * 1024;

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

pub fn resolve_api_key() -> Result<String, FetchError> {
    std::env::var("LASTFM_API_KEY").map_err(|_| FetchError::Failed("LASTFM_API_KEY not set".into()))
}

/// `GET https://ws.audioscrobbler.com/2.0/?method=<method>&api_key=<key>&format=json&<params>`
/// then deserialize into `T`. Caller passes additional query pairs as `(key, value)` slices;
/// `method`, `api_key`, and `format` are appended unconditionally.
pub async fn get_json<T: DeserializeOwned>(
    method: &str,
    params: &[(&str, &str)],
) -> Result<T, FetchError> {
    let key = resolve_api_key()?;
    let url = build_url(method, &key, params);
    let res = http()
        .get(&url)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("lastfm request failed: {e}")))?;
    parse_json(res).await
}

fn build_url(method: &str, api_key: &str, params: &[(&str, &str)]) -> String {
    let mut url = url::Url::parse(API_BASE).expect("static API_BASE parses");
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("method", method);
        q.append_pair("api_key", api_key);
        q.append_pair("format", "json");
        for (k, v) in params {
            q.append_pair(k, v);
        }
    }
    url.into()
}

async fn parse_json<T: DeserializeOwned>(res: reqwest::Response) -> Result<T, FetchError> {
    let status = res.status();
    let bytes = res
        .bytes()
        .await
        .map_err(|e| FetchError::Failed(format!("lastfm read body: {e}")))?;
    if bytes.len() > MAX_BYTES {
        return Err(FetchError::Failed(format!(
            "lastfm response too large ({} bytes, cap {MAX_BYTES})",
            bytes.len()
        )));
    }
    if !status.is_success() {
        return Err(FetchError::Failed(format!("lastfm {status}")));
    }
    // Last.fm tunnels errors through a 200 + `{"error":N,"message":...}` envelope, so we
    // detect that before handing bytes to the caller's deserializer.
    if let Some(err) = parse_api_error(&bytes) {
        return Err(err);
    }
    serde_json::from_slice(&bytes)
        .map_err(|e| FetchError::Failed(format!("lastfm json parse: {e}")))
}

fn parse_api_error(bytes: &[u8]) -> Option<FetchError> {
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let code = value.get("error").and_then(|v| v.as_i64())?;
    let message = value
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("(no message)");
    Some(FetchError::Failed(format!(
        "lastfm error {code}: {message}"
    )))
}

