//! GitLab MR / issue DTOs and adapters into [`forge_items::ForgeRow`]. Two row variants
//! (`include_repo = true` for cross-project user-scope feeds, `false` for repo-scope feeds)
//! drive whether `references.full` or `references.short` is used as the label — same toggle
//! convention as the `github_*` family.

use serde::Deserialize;

use crate::fetcher::forge_items::ForgeRow;
use crate::fetcher::gitlab::common::parse_timestamp;

#[derive(Debug, Deserialize, Default, Clone)]
pub struct GitlabIssueLike {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub web_url: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub user_notes_count: u64,
    #[serde(default)]
    pub references: GitlabReferences,
    #[serde(default)]
    pub author: Option<GitlabAuthor>,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct GitlabReferences {
    #[serde(default)]
    pub short: String,
    #[serde(default)]
    pub full: String,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct GitlabAuthor {
    #[serde(default)]
    pub avatar_url: Option<String>,
}

/// Convert a GitLab MR / issue payload into a renderer-ready row. The `references` block on the
/// API response already carries the pre-formatted `group/proj!42` (MR) and `group/proj#75`
/// (issue) labels, so we don't have to re-stitch project + iid ourselves.
pub fn to_forge_row(item: &GitlabIssueLike, include_repo: bool) -> ForgeRow {
    let label = if include_repo {
        item.references.full.clone()
    } else {
        item.references.short.clone()
    };
    ForgeRow {
        label: if label.is_empty() { "?".into() } else { label },
        title: item.title.clone(),
        url: if item.web_url.is_empty() {
            None
        } else {
            Some(item.web_url.clone())
        },
        avatar_url: item.author.as_ref().and_then(|a| a.avatar_url.clone()),
        avatar_path: None,
        updated_at_unix: parse_timestamp(&item.updated_at),
        activity_count: item.user_notes_count,
    }
}

pub fn to_forge_rows(items: &[GitlabIssueLike], include_repo: bool) -> Vec<ForgeRow> {
    items
        .iter()
        .map(|i| to_forge_row(i, include_repo))
        .collect()
}

/// Build a tiny [`ForgeRow`] set with stable values for `sample_body`. Sharing the helper keeps
/// every list-shape fetcher (`gitlab_my_mrs`, `gitlab_repo_issues`, …) printing the same
/// row-formatting conventions in the docs.
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

