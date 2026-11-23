//! Shared response types for the issue / PR family. The REST search endpoints and the
//! per-repo PR/issue endpoints return almost the same item shape; they share this struct so
//! each fetcher can focus on its own URL composition and rendering.
//!
//! Rendering goes through [`crate::fetcher::forge_items::render_forge_rows`] so the github and
//! gitlab families produce structurally identical output for every shape — adding a new shape
//! variant lights up on both families simultaneously.

use serde::Deserialize;

use crate::fetcher::forge_items::{self, ForgeRow};
use crate::payload::Body;
use crate::render::Shape;

use super::client::api_repos_prefix;
use super::common::{RepoSlug, parse_timestamp};

#[derive(Debug, Deserialize, Default)]
pub struct IssueItem {
    pub title: String,
    pub number: u64,
    /// Only present on the `/search/issues` response. Per-repo endpoints omit this.
    #[serde(default)]
    pub repository_url: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub state: String,
    /// Comments count, fed into the `Bars` shape as a per-row activity weight.
    #[serde(default)]
    pub comments: u64,
    /// Author block — only `avatar_url` is used today (for the `ImageLinkedList` shape).
    #[serde(default)]
    pub user: Option<GithubUserRef>,
}

#[derive(Debug, Deserialize, Default)]
pub struct GithubUserRef {
    #[serde(default)]
    pub avatar_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SearchResult {
    #[serde(default)]
    pub items: Vec<IssueItem>,
}

/// Extracts `owner/name` from a GitHub API repository URL. The prefix tracks the configured
/// host via `client::api_repos_prefix()`.
pub fn repo_from_url(url: &str) -> Option<RepoSlug> {
    let rest = url.strip_prefix(api_repos_prefix())?;
    RepoSlug::parse(rest)
}

/// Sync-only path for fetchers whose `shapes()` set doesn't include `Badge` /
/// `ImageLinkedList` (currently `good_first_issues`). Both of those variants need
/// preprocessing that this signature can't reach: `Badge` needs the row count + a noun pair
/// for `forge_items::render_count_badge`, and `ImageLinkedList` needs avatar URLs resolved
/// through `thumbnails::download_many` before render. Fetchers that accept those shapes go
/// through `forge_items::dispatch_rows_async` instead.
///
/// When `include_repo` is true, each label is prefixed with `owner/name#42` (used by
/// user-scope fetchers whose results span many repos). Per-repo fetchers pass `false` so the
/// row stays at `#42`.
pub fn render_items(items: &[IssueItem], shape: Shape, include_repo: bool) -> Body {
    debug_assert!(
        !matches!(shape, Shape::Badge | Shape::ImageLinkedList),
        "render_items is sync-only; route {shape:?} through forge_items::dispatch_rows_async"
    );
    let rows = to_forge_rows(items, include_repo);
    forge_items::render_forge_rows(&rows, shape)
}

/// Lower-level converter so fetchers can pre-process rows (e.g. resolving thumbnails for
/// `ImageLinkedList`) before handing them to the shared renderer.
pub fn to_forge_rows(items: &[IssueItem], include_repo: bool) -> Vec<ForgeRow> {
    items
        .iter()
        .map(|i| to_forge_row(i, include_repo))
        .collect()
}

/// Build a tiny [`ForgeRow`] set with stable values for `sample_body`. Mirrors the gitlab
/// helper so both families share the same row-formatting conventions in docs.
pub fn sample_rows(rows: &[(&str, &str, Option<&str>, u64, i64)]) -> Vec<ForgeRow> {
    rows.iter()
        .map(|(label, title, url, activity, ts)| ForgeRow {
            label: (*label).into(),
            title: (*title).into(),
            url: url.map(String::from),
            avatar_url: None,
            avatar_path: None,
            updated_at_unix: *ts,
            activity_count: *activity,
        })
        .collect()
}

fn to_forge_row(i: &IssueItem, include_repo: bool) -> ForgeRow {
    let label = if include_repo {
        let repo = repo_from_url(&i.repository_url)
            .map(|s| s.as_path())
            .unwrap_or_else(|| "?".into());
        format!("{repo}#{}", i.number)
    } else {
        format!("#{}", i.number)
    };
    ForgeRow {
        label,
        title: i.title.clone(),
        url: if i.html_url.is_empty() {
            None
        } else {
            Some(i.html_url.clone())
        },
        avatar_url: i.user.as_ref().and_then(|u| u.avatar_url.clone()),
        avatar_path: None,
        updated_at_unix: parse_timestamp(&i.updated_at),
        activity_count: i.comments,
    }
}

