//! `gitlab_my_mrs` — open merge requests authored by the authenticated GitLab user across
//! every project. Uses `/api/v4/merge_requests?scope=created_by_me&state=opened`.

use async_trait::async_trait;
use serde::Deserialize;

use crate::fetcher::forge_items::{self, LIST_SHAPES};
use crate::options::OptionSchema;
use crate::payload::{Body, Payload};
use crate::render::Shape;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::client::{default_host, rest_get};
use super::common::{cache_key, parse_options, payload, validate_host};
use super::items::{GitlabIssueLike, sample_rows, to_forge_rows};

const DEFAULT_LIMIT: u32 = 10;

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "host",
        type_hint: "hostname",
        required: false,
        default: Some("gitlab.com"),
        description: "GitLab instance host. Use this for self-hosted GitLab.",
    },
    OptionSchema {
        name: "limit",
        type_hint: "integer (1..=30)",
        required: false,
        default: Some("10"),
        description: "Maximum number of MRs to show.",
    },
];

pub struct GitlabMyMrs;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[async_trait]
impl Fetcher for GitlabMyMrs {
    fn name(&self) -> &str {
        "gitlab_my_mrs"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Open merge requests authored by the authenticated GitLab user across every project. Use `gitlab_repo_mrs` instead to list MRs against one specific project regardless of who opened them."
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
        let host = host_for_key(ctx);
        cache_key(self.name(), ctx, &host)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        let rows = sample_rows(&[
            (
                "g/splashboard!54",
                "feat(docs): generate widget catalogue",
                Some("https://gitlab.com/g/splashboard/-/merge_requests/54"),
                7,
                1_774_000_000,
            ),
            (
                "g/splashboard!51",
                "feat(fetcher): split clock options",
                Some("https://gitlab.com/g/splashboard/-/merge_requests/51"),
                2,
                1_773_800_000,
            ),
        ]);
        forge_items::dispatch_sample(&rows, shape, "open MR", "open MRs")
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let host = resolve_host(opts.host.as_deref())?;
        let limit = opts.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 30);
        let path = format!(
            "/merge_requests?scope=created_by_me&state=opened&per_page={limit}&order_by=updated_at"
        );
        let items: Vec<GitlabIssueLike> = rest_get(&host, &path).await?;
        let rows = to_forge_rows(&items, true);
        let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);
        let body = forge_items::dispatch_rows_async(rows, shape, "open MR", "open MRs").await;
        Ok(payload(body))
    }
}

pub(super) fn resolve_host(explicit: Option<&str>) -> Result<String, FetchError> {
    let host = explicit.unwrap_or(default_host());
    validate_host(host)
        .map(String::from)
        .map_err(FetchError::Failed)
}

fn host_for_key(ctx: &FetchContext) -> String {
    ctx.options
        .as_ref()
        .and_then(|v| v.get("host"))
        .and_then(|v| v.as_str())
        .unwrap_or(default_host())
        .to_string()
}

