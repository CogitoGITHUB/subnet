//! Cross-cutting helpers for the `github_*` fetchers: option parsing, repo resolution,
//! cache-key construction, timestamp parsing, shared payload/body constructors.

use std::path::PathBuf;

use chrono::DateTime;
use gix::bstr::ByteSlice;
use sha2::{Digest, Sha256};

use crate::fetcher::{FetchContext, FetchError};
use crate::payload::{Body, Payload, TextBlockData};

pub fn payload(body: Body) -> Payload {
    Payload {
        icon: None,
        status: None,
        format: None,
        body,
    }
}

pub fn text_block_body(values: Vec<String>) -> Body {
    Body::TextBlock(TextBlockData { lines: values })
}

pub fn parse_options<T: serde::de::DeserializeOwned + Default>(
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

/// Unix timestamp from an RFC3339 string (`"2026-04-22T10:15:30Z"`). Timeline events store
/// absolute seconds so the renderer re-computes the relative label each draw.
pub fn parse_timestamp(raw: &str) -> i64 {
    DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.timestamp())
        .unwrap_or(0)
}

/// Cache key that mixes fetcher name + shape + format + auxiliary fields (repo spec, etc). The
/// output is stable for identical invocations so the daemon reuses the same on-disk file.
pub fn cache_key(name: &str, ctx: &FetchContext, extra: &str) -> String {
    let shape = ctx.shape.map(|s| s.as_str()).unwrap_or("default");
    let raw = format!(
        "{}|{}|{}|{}",
        name,
        shape,
        ctx.format.as_deref().unwrap_or(""),
        extra
    );
    let digest = Sha256::digest(raw.as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("{name}-{hex}")
}

/// `owner/name` as split by a single `/`. Anything else returns `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoSlug {
    pub owner: String,
    pub name: String,
}

impl RepoSlug {
    pub fn parse(raw: &str) -> Option<Self> {
        let (owner, name) = raw.split_once('/')?;
        if owner.is_empty() || name.is_empty() || name.contains('/') {
            return None;
        }
        Some(Self {
            owner: owner.into(),
            name: name.into(),
        })
    }

    pub fn as_path(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

/// Resolves the repo for a repo-scope fetcher: explicit `options.repo` wins, otherwise the git
/// remote named `origin` (or the first remote) is parsed. Returns `FetchError::Failed` with a
/// human-readable reason when neither yields a valid slug.
pub fn resolve_repo(explicit: Option<&str>) -> Result<RepoSlug, FetchError> {
    if let Some(raw) = explicit {
        return RepoSlug::parse(raw)
            .ok_or_else(|| FetchError::Failed(format!("invalid repo option: {raw:?}")));
    }
    match remote_slug_from_cwd() {
        Some(slug) => Ok(slug),
        None => Err(FetchError::Failed(
            "no repo: set `repo = \"owner/name\"` or run inside a git repo with a github remote"
                .into(),
        )),
    }
}

fn remote_slug_from_cwd() -> Option<RepoSlug> {
    let cwd = std::env::current_dir().ok()?;
    let repo = gix::discover(&cwd).ok()?;
    remote_slug(&repo)
}

fn remote_slug(repo: &gix::Repository) -> Option<RepoSlug> {
    let names: Vec<_> = repo.remote_names().into_iter().collect();
    let preferred = names
        .iter()
        .find(|n| n.as_bstr() == "origin")
        .or_else(|| names.first());
    let name = preferred?;
    let remote = repo.find_remote(name).ok()?;
    let url = remote.url(gix::remote::Direction::Fetch)?;
    slug_from_url(&url.to_bstring().to_string())
}

/// Parses the github URL shapes splashboard supports against the configured host:
/// - `<user>@<host>:owner/name(.git)?` (SCP-style; `<user>` is conventionally `git` on
///   github.com but can be anything — Microsoft GHE uses `microsoft@...`, for instance)
/// - `https://<host>/owner/name(.git)?`
/// - `https://<user>(:<token>)?@<host>/owner/name(.git)?`
/// - `ssh://<user>@<host>/owner/name(.git)?`
///
/// When `GH_HOST` is set, the configured host wins; we also try github.com as a fallback so
/// a user with a GHE shell can still parse github.com remotes.
pub fn slug_from_url(url: &str) -> Option<RepoSlug> {
    let host = super::client::host();
    parse_with_host(host, url).or_else(|| {
        if host.eq_ignore_ascii_case("github.com") {
            None
        } else {
            parse_with_host("github.com", url)
        }
    })
}

fn parse_with_host(host: &str, url: &str) -> Option<RepoSlug> {
    let rest = strip_scp_prefix(host, url)
        .or_else(|| strip_https_prefix(host, url))
        .or_else(|| strip_ssh_prefix(host, url))?;
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    RepoSlug::parse(rest)
}

fn strip_scp_prefix<'a>(host: &str, url: &'a str) -> Option<&'a str> {
    let (userinfo, rest) = url.split_once('@')?;
    if userinfo.is_empty() || userinfo.contains([':', '/']) {
        return None;
    }
    rest.strip_prefix(&format!("{host}:")[..])
}

fn strip_https_prefix<'a>(host: &str, url: &'a str) -> Option<&'a str> {
    let after_scheme = url.strip_prefix("https://")?;
    let after_userinfo = match after_scheme.split_once('@') {
        Some((userinfo, rest)) if !userinfo.contains('/') => rest,
        _ => after_scheme,
    };
    after_userinfo.strip_prefix(&format!("{host}/")[..])
}

fn strip_ssh_prefix<'a>(host: &str, url: &'a str) -> Option<&'a str> {
    let after_scheme = url.strip_prefix("ssh://")?;
    let (userinfo, rest) = after_scheme.split_once('@')?;
    if userinfo.is_empty() || userinfo.contains([':', '/']) {
        return None;
    }
    rest.strip_prefix(&format!("{host}/")[..])
}

#[allow(dead_code)]
pub fn cwd_path() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from(""))
}

