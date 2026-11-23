use async_trait::async_trait;

use crate::payload::{BadgeData, Body, EntriesData, Entry, Payload, Status};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::{fail, open_repo, payload, repo_cache_key, text_body};

const SHAPES: &[Shape] = &[Shape::Text, Shape::Entries, Shape::Badge];

/// Number of entries in the stash reflog. A missing `refs/stash` ref or an absent reflog returns
/// zero — both are common (no stashes yet) and shouldn't surface as an error.
pub struct GitStashCount;

#[async_trait]
impl Fetcher for GitStashCount {
    fn name(&self) -> &str {
        "git_stash_count"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Number of entries in the stash reflog, as a quiet reminder of work parked aside. `Text` collapses to empty when there are zero stashes; `Entries` always reports the count; `Badge` shows the count as a pill (Ok at zero, Warn otherwise)."
    }
    fn refresh_interval(&self) -> u64 {
        60
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        repo_cache_key(self.name(), ctx)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Text => samples::text("3 stashes"),
            Shape::Entries => samples::entries(&[("stashes", "3")]),
            Shape::Badge => samples::badge(Status::Warn, "3 stashes"),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let repo = open_repo()?;
        let count = stash_count(&repo)?;
        Ok(payload(render_body(
            count,
            ctx.shape.unwrap_or(Shape::Text),
        )))
    }
}

fn stash_count(repo: &gix::Repository) -> Result<usize, FetchError> {
    let Some(stash_ref) = repo.try_find_reference("refs/stash").map_err(fail)? else {
        return Ok(0);
    };
    let mut platform = stash_ref.log_iter();
    match platform.all().map_err(fail)? {
        Some(iter) => Ok(iter.filter_map(Result::ok).count()),
        None => Ok(0),
    }
}

fn render_body(count: usize, shape: Shape) -> Body {
    match shape {
        Shape::Entries => Body::Entries(EntriesData {
            items: vec![Entry {
                key: "stashes".into(),
                value: Some(count.to_string()),
                status: None,
            }],
        }),
        Shape::Badge => Body::Badge(BadgeData {
            status: if count == 0 { Status::Ok } else { Status::Warn },
            label: stash_label(count),
        }),
        _ => {
            if count == 0 {
                text_body("")
            } else {
                text_body(stash_label(count))
            }
        }
    }
}

fn stash_label(count: usize) -> String {
    match count {
        0 => "no stashes".into(),
        1 => "1 stash".into(),
        n => format!("{n} stashes"),
    }
}

