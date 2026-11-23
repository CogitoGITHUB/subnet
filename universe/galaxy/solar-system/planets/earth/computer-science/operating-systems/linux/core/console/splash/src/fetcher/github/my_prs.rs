//! `github_my_prs` — open pull requests authored by the authenticated user across every repo.
//! Uses `/search/issues?q=is:pr+is:open+author:@me`. The query is fixed (no config-controlled
//! path) so the fetcher is `Safe`.

use async_trait::async_trait;
use serde::Deserialize;

use crate::fetcher::forge_items::{self, LIST_SHAPES};
use crate::options::OptionSchema;
use crate::payload::{Body, Payload};
use crate::render::Shape;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::client::rest_get;
use super::common::{cache_key, parse_options, payload};
use super::items::{SearchResult, sample_rows, to_forge_rows};

const DEFAULT_LIMIT: u32 = 10;

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "limit",
    type_hint: "integer (1..=30)",
    required: false,
    default: Some("10"),
    description: "Maximum number of PRs to show.",
}];

pub struct GithubMyPrs;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub limit: Option<u32>,
}

#[async_trait]
impl Fetcher for GithubMyPrs {
    fn name(&self) -> &str {
        "github_my_prs"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Open pull requests authored by the authenticated user across every repo. Use `github_repo_prs` instead to list PRs against one specific repo regardless of who opened them."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 5
    }
    fn shapes(&self) -> &[Shape] {
        LIST_SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        cache_key(self.name(), ctx, "")
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        let rows = sample_rows(&[
            (
                "unhappychoice/splashboard#54",
                "feat(docs): generate widget catalogue",
                Some("https://github.com/unhappychoice/splashboard/pull/54"),
                7,
                1_774_000_000,
            ),
            (
                "unhappychoice/splashboard#51",
                "feat(fetcher): split clock options",
                Some("https://github.com/unhappychoice/splashboard/pull/51"),
                2,
                1_773_800_000,
            ),
        ]);
        forge_items::dispatch_sample(&rows, shape, "open PR", "open PRs")
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let limit = opts.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 30);
        let path = format!(
            "/search/issues?q=is%3Apr+is%3Aopen+author%3A%40me&per_page={limit}&sort=updated"
        );
        let res: SearchResult = rest_get(&path).await?;
        let rows = to_forge_rows(&res.items, true);
        let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);
        let body = forge_items::dispatch_rows_async(rows, shape, "open PR", "open PRs").await;
        Ok(payload(body))
    }
}

