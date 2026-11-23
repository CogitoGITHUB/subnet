use async_trait::async_trait;
use gix::prelude::ObjectIdExt;

use crate::payload::{Body, Payload, TimelineData, TimelineEvent};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::{fail, open_repo, payload, repo_cache_key, text_block_body};

const SHAPES: &[Shape] = &[Shape::Timeline, Shape::TextBlock];
const DEFAULT_LIMIT: usize = 10;

/// Newest commits on HEAD, walked via gix rev-walk. The `format` field is parsed as a commit
/// count (`"5"`, `"20"`) — absent or unparseable falls back to 10. `TextBlock` renders
/// `<short> <summary>` one per line so users who want a flat list instead of the timeline
/// renderer's relative-ago labels can still wire it up.
pub struct GitRecentCommits;

#[async_trait]
impl Fetcher for GitRecentCommits {
    fn name(&self) -> &str {
        "git_recent_commits"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Newest commits on HEAD, defaulting to ten (override with `format = \"N\"`). `Timeline` shows relative-ago labels; `TextBlock` is a flat `<short> <summary>` list."
    }
    fn refresh_interval(&self) -> u64 {
        60
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn default_shape(&self) -> Shape {
        Shape::Timeline
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        repo_cache_key(self.name(), ctx)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Timeline => samples::timeline(&[
                (1_700_000_000, "feat(render): add heatmap", Some("a1b2c3d")),
                (1_699_990_000, "fix(fetcher): tz fallback", Some("d4e5f6a")),
                (1_699_900_000, "chore: bump ratatui", Some("e7f8a9b")),
            ]),
            Shape::TextBlock => samples::text_block(&[
                "a1b2c3d feat(render): add heatmap",
                "d4e5f6a fix(fetcher): tz fallback",
                "e7f8a9b chore: bump ratatui",
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let repo = open_repo()?;
        let limit = parse_limit(ctx.format.as_deref());
        let commits = recent_commits(&repo, limit)?;
        Ok(payload(render_body(
            commits,
            ctx.shape.unwrap_or(Shape::Timeline),
        )))
    }
}

struct CommitInfo {
    short_hash: String,
    summary: String,
    timestamp: i64,
}

fn recent_commits(repo: &gix::Repository, limit: usize) -> Result<Vec<CommitInfo>, FetchError> {
    let Ok(head_id) = repo.head_id() else {
        return Ok(Vec::new());
    };
    let walker = repo.rev_walk([head_id.detach()]).all().map_err(fail)?;
    let mut out = Vec::with_capacity(limit);
    for info in walker.take(limit) {
        let Ok(info) = info else { continue };
        let Ok(commit) = repo.find_commit(info.id) else {
            continue;
        };
        let summary = commit
            .message()
            .ok()
            .map(|m| m.summary().to_string())
            .unwrap_or_default();
        let timestamp = commit.time().map(|t| t.seconds).unwrap_or(0);
        let short_hash = info
            .id
            .attach(repo)
            .shorten()
            .map(|p| p.to_string())
            .unwrap_or_default();
        out.push(CommitInfo {
            short_hash,
            summary,
            timestamp,
        });
    }
    Ok(out)
}

fn parse_limit(format: Option<&str>) -> usize {
    format
        .and_then(|s| s.trim().parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_LIMIT)
}

fn render_body(commits: Vec<CommitInfo>, shape: Shape) -> Body {
    match shape {
        Shape::TextBlock => text_block_body(
            commits
                .into_iter()
                .map(|c| format!("{} {}", c.short_hash, c.summary))
                .collect(),
        ),
        _ => Body::Timeline(TimelineData {
            events: commits
                .into_iter()
                .map(|c| TimelineEvent {
                    timestamp: c.timestamp,
                    title: c.summary,
                    detail: Some(c.short_hash),
                    status: None,
                })
                .collect(),
        }),
    }
}

