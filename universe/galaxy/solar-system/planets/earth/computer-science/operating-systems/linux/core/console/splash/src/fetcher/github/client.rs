//! Shared HTTP client + auth for the `github_*` fetcher family. Reads `GH_TOKEN` or
//! `GITHUB_TOKEN` (in that order) once per process. The host defaults to `github.com` but
//! can be redirected to a GitHub Enterprise Server via `GH_HOST` / `GITHUB_HOST` (and
//! optionally `GH_API_URL` / `GH_GRAPHQL_URL` for older `<host>/api/v3` deployments). Host
//! selection is env-var-only — a `.splash.toml` cannot redirect traffic to an
//! attacker-chosen origin, so the token cannot leak by design.
//!
//! The rest helpers accept a path like `"/user"` or `"/repos/foo/bar"`; the base URL is joined
//! here so fetchers stay free of URL plumbing. GraphQL goes through a single POST helper.

use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::fetcher::FetchError;

const DEFAULT_HOST: &str = "github.com";
const USER_AGENT: &str = concat!("splashboard/", env!("CARGO_PKG_VERSION"));

fn read_env(var: &str) -> Option<String> {
    std::env::var(var).ok().and_then(|raw| normalize_env(&raw))
}

/// Trim whitespace and a trailing `/` so callers can rely on host / URL values being free of
/// surface-level user typos like `GH_API_URL=https://ghe.example.com/api/v3/`. Returns `None`
/// when the value collapses to empty after normalization so the caller can fall back to the
/// default.
fn normalize_env(raw: &str) -> Option<String> {
    let v = raw.trim().trim_end_matches('/');
    (!v.is_empty()).then(|| v.to_owned())
}

pub(crate) fn host() -> &'static str {
    static HOST: OnceLock<String> = OnceLock::new();
    HOST.get_or_init(|| {
        read_env("GH_HOST")
            .or_else(|| read_env("GITHUB_HOST"))
            .unwrap_or_else(|| DEFAULT_HOST.into())
            // Normalize to lowercase so SCP-style remote parsing (`git@<host>:`) and
            // `eq_ignore_ascii_case` checks against `"github.com"` stay consistent when
            // the user types `GH_HOST=GitHub.com`. DNS is case-insensitive so this
            // doesn't affect URL composition.
            .to_ascii_lowercase()
    })
    .as_str()
}

/// REST API base URL. Defaults to `https://api.<host>` (github.com and modern GHE).
/// Override via `GH_API_URL` / `GITHUB_API_URL` for older `<host>/api/v3` GHE deployments
/// (the env var name matches GitHub Actions' standard `GITHUB_API_URL`).
pub(crate) fn api_base() -> &'static str {
    static API_BASE: OnceLock<String> = OnceLock::new();
    API_BASE.get_or_init(|| {
        read_env("GH_API_URL")
            .or_else(|| read_env("GITHUB_API_URL"))
            .unwrap_or_else(|| format!("https://api.{}", host()))
    })
}

/// GraphQL endpoint. Defaults to `https://api.<host>/graphql`. Override via
/// `GH_GRAPHQL_URL` / `GITHUB_GRAPHQL_URL` for older `<host>/api/graphql` GHE deployments
/// (matches GitHub Actions' standard `GITHUB_GRAPHQL_URL` env var).
pub(crate) fn graphql_url() -> &'static str {
    static GRAPHQL_URL: OnceLock<String> = OnceLock::new();
    GRAPHQL_URL.get_or_init(|| {
        read_env("GH_GRAPHQL_URL")
            .or_else(|| read_env("GITHUB_GRAPHQL_URL"))
            .unwrap_or_else(|| format!("https://api.{}/graphql", host()))
    })
}

pub(crate) fn web_base() -> &'static str {
    static WEB_BASE: OnceLock<String> = OnceLock::new();
    WEB_BASE.get_or_init(|| format!("https://{}", host()))
}

pub(crate) fn api_repos_prefix() -> &'static str {
    static API_REPOS_PREFIX: OnceLock<String> = OnceLock::new();
    API_REPOS_PREFIX.get_or_init(|| format!("{}/repos/", api_base()))
}

const ACCEPT: &str = "application/vnd.github+json";
const API_VERSION: &str = "2022-11-28";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
static AUTHENTICATED_USER_CACHE: OnceLock<Mutex<Option<String>>> = OnceLock::new();

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
    std::env::var("GH_TOKEN")
        .or_else(|_| std::env::var("GITHUB_TOKEN"))
        .map_err(|_| FetchError::Failed("GH_TOKEN / GITHUB_TOKEN not set".into()))
}

/// Login of the token-authenticated user. Resolved lazily via `GET /user` and memoised for the
/// rest of the process so we never spend more than one roundtrip on a value that never changes
/// within a session. Lets `github_avatar` / `github_user` work with zero config when a token is
/// already set.
pub async fn resolve_authenticated_user() -> Result<String, FetchError> {
    let slot = AUTHENTICATED_USER_CACHE.get_or_init(|| Mutex::new(None));
    if let Some(cached) = slot.lock().ok().and_then(|g| g.clone()) {
        return Ok(cached);
    }
    #[derive(Deserialize)]
    struct Me {
        login: String,
    }
    let me: Me = rest_get("/user").await?;
    if let Ok(mut g) = slot.lock() {
        *g = Some(me.login.clone());
    }
    Ok(me.login)
}

#[cfg(test)]
pub(crate) fn clear_authenticated_user_cache() {
    if let Ok(mut guard) = AUTHENTICATED_USER_CACHE
        .get_or_init(|| Mutex::new(None))
        .lock()
    {
        *guard = None;
    }
}

/// REST GET → deserialize JSON. Non-2xx responses surface the GitHub-reported `message` when
/// present (`{"message":"Not Found"}`) so the runtime's error placeholder and the log line
/// are both actionable.
pub async fn rest_get<T: DeserializeOwned>(path: &str) -> Result<T, FetchError> {
    let token = resolve_token()?;
    let url = format!("{}{path}", api_base());
    let res = http()
        .get(&url)
        .bearer_auth(&token)
        .header("Accept", ACCEPT)
        .header("X-GitHub-Api-Version", API_VERSION)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("github request failed: {e}")))?;
    parse_json(res).await
}

/// GraphQL POST. GitHub returns 200 even on query errors, so we also look at the `errors` array.
pub async fn graphql<T: DeserializeOwned>(
    query: &str,
    variables: serde_json::Value,
) -> Result<T, FetchError> {
    let token = resolve_token()?;
    let body = serde_json::json!({ "query": query, "variables": variables });
    let res = http()
        .post(graphql_url())
        .bearer_auth(&token)
        .header("Accept", ACCEPT)
        .header("X-GitHub-Api-Version", API_VERSION)
        .json(&body)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("github graphql request failed: {e}")))?;
    let wrapper: GqlResponse<T> = parse_json(res).await?;
    if let Some(errs) = wrapper.errors.filter(|e| !e.is_empty()) {
        let joined = errs
            .iter()
            .map(|e| e.message.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(FetchError::Failed(format!("github graphql: {joined}")));
    }
    wrapper
        .data
        .ok_or_else(|| FetchError::Failed("github graphql: empty data".into()))
}

async fn parse_json<T: DeserializeOwned>(res: reqwest::Response) -> Result<T, FetchError> {
    let status = res.status();
    let bytes = res
        .bytes()
        .await
        .map_err(|e| FetchError::Failed(format!("github response body: {e}")))?;
    if !status.is_success() {
        return Err(FetchError::Failed(error_message(status, &bytes)));
    }
    serde_json::from_slice(&bytes)
        .map_err(|e| FetchError::Failed(format!("github json parse: {e}")))
}

fn error_message(status: StatusCode, body: &[u8]) -> String {
    #[derive(Deserialize)]
    struct ApiError {
        message: Option<String>,
    }
    let reported = serde_json::from_slice::<ApiError>(body)
        .ok()
        .and_then(|e| e.message);
    match reported {
        Some(m) => format!("github {status}: {m}"),
        None => format!("github {status}"),
    }
}

#[derive(Debug, Deserialize)]
struct GqlResponse<T> {
    data: Option<T>,
    #[serde(default)]
    errors: Option<Vec<GqlError>>,
}

#[derive(Debug, Deserialize)]
struct GqlError {
    message: String,
}

