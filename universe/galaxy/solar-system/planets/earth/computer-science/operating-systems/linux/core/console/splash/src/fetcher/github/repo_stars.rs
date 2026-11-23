//! `github_repo_stars` — stargazer count (plus forks / watchers / open issues as `Entries`).
//! Uses `/repos/{o}/{r}`.

use async_trait::async_trait;
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Bar, BarsData, Body, EntriesData, Entry, MarkdownTextBlockData, Payload, Status,
    TextBlockData, TextData,
};
use crate::render::Shape;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::client::rest_get;
use super::common::{RepoSlug, cache_key, parse_options, payload, resolve_repo};

const SHAPES: &[Shape] = &[
    Shape::Text,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::Entries,
    Shape::Bars,
    Shape::Badge,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "repo",
    type_hint: "\"owner/name\"",
    required: false,
    default: Some("git remote of cwd"),
    description: "Repository to query. Falls back to the current directory's github remote.",
}];

pub struct GithubRepoStars;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub repo: Option<String>,
}

#[async_trait]
impl Fetcher for GithubRepoStars {
    fn name(&self) -> &str {
        "github_repo_stars"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Social counters for a repo: stargazers, forks, watchers, and open-issue count. Use `github_repo` instead for the identity fields (slug, description, license)."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 60
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        let extra = repo_for_key(ctx);
        cache_key(self.name(), ctx, &extra)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        if !SHAPES.contains(&shape) {
            return None;
        }
        let info = RepoInfo {
            stargazers_count: 142,
            forks_count: 9,
            subscribers_count: 12,
            open_issues_count: 7,
        };
        Some(render_body(&info, shape))
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let slug = resolve_repo(opts.repo.as_deref())?;
        let path = format!("/repos/{}/{}", slug.owner, slug.name);
        let repo: RepoInfo = rest_get(&path).await?;
        Ok(payload(render_body(
            &repo,
            ctx.shape.unwrap_or(Shape::Text),
        )))
    }
}

fn render_body(info: &RepoInfo, shape: Shape) -> Body {
    match shape {
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: vec![
                format!("★ {}", info.stargazers_count),
                format!("🍴 {}", info.forks_count),
                format!("👁 {}", info.subscribers_count),
                format!("🔓 {}", info.open_issues_count),
            ],
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: format!(
                "- **★ stars** {}\n- **🍴 forks** {}\n- **👁 watchers** {}\n- **🔓 open issues** {}",
                info.stargazers_count,
                info.forks_count,
                info.subscribers_count,
                info.open_issues_count
            ),
        }),
        Shape::Entries => Body::Entries(EntriesData {
            items: vec![
                entry("stars", &info.stargazers_count.to_string()),
                entry("forks", &info.forks_count.to_string()),
                entry("watchers", &info.subscribers_count.to_string()),
                entry("open_issues", &info.open_issues_count.to_string()),
            ],
        }),
        Shape::Bars => Body::Bars(BarsData {
            bars: vec![
                Bar {
                    label: "stars".into(),
                    value: info.stargazers_count,
                    value_label: None,
                },
                Bar {
                    label: "forks".into(),
                    value: info.forks_count,
                    value_label: None,
                },
                Bar {
                    label: "watchers".into(),
                    value: info.subscribers_count,
                    value_label: None,
                },
                Bar {
                    label: "open_issues".into(),
                    value: info.open_issues_count,
                    value_label: None,
                },
            ],
        }),
        Shape::Badge => Body::Badge(BadgeData {
            status: Status::Ok,
            label: format!("★ {}", info.stargazers_count),
        }),
        _ => Body::Text(TextData {
            value: format!("★ {}", info.stargazers_count),
        }),
    }
}

#[derive(Debug, Deserialize)]
struct RepoInfo {
    #[serde(default)]
    stargazers_count: u64,
    #[serde(default)]
    forks_count: u64,
    #[serde(default)]
    subscribers_count: u64,
    #[serde(default)]
    open_issues_count: u64,
}

fn entry(key: &str, value: &str) -> Entry {
    Entry {
        key: key.into(),
        value: Some(value.into()),
        status: None,
    }
}

fn repo_for_key(ctx: &FetchContext) -> String {
    ctx.options
        .as_ref()
        .and_then(|v| v.get("repo"))
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| resolve_repo(None).ok().map(|s: RepoSlug| s.as_path()))
        .unwrap_or_default()
}

