//! `github_repo` — identity triple `{ slug, description, license }` sourced from the GitHub
//! REST API `/repos/{o}/{n}`. Entries shape feeds `grid_table` (inline or rows); Text joins
//! the non-empty fields with `·` for a compact subtitle.

use async_trait::async_trait;
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{Body, EntriesData, Entry, Payload, TextBlockData, TextData};
use crate::render::Shape;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::client::rest_get;
use super::common::{RepoSlug, cache_key, parse_options, resolve_repo};

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "repo",
    type_hint: "\"owner/name\"",
    required: false,
    default: Some("git remote of cwd"),
    description: "Repository to query. Falls back to the current directory's github remote.",
}];

pub struct GithubRepo;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub repo: Option<String>,
}

#[async_trait]
impl Fetcher for GithubRepo {
    fn name(&self) -> &str {
        "github_repo"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Repo identity for the current directory: slug, description, and SPDX license. Use `github_repo_stars` instead for the social counters (stars, forks, watchers)."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 60
    }
    fn shapes(&self) -> &[Shape] {
        &[Shape::Entries, Shape::TextBlock, Shape::Text]
    }
    fn default_shape(&self) -> Shape {
        Shape::Entries
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        let extra = repo_for_key(ctx);
        cache_key(self.name(), ctx, &extra)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Entries => Body::Entries(EntriesData {
                items: vec![
                    entry("slug", "unhappychoice/splashboard"),
                    entry("description", "terminal splash renderer"),
                    entry("license", "ISC"),
                ],
            }),
            Shape::TextBlock => Body::TextBlock(TextBlockData {
                lines: vec![
                    "unhappychoice/splashboard".into(),
                    "terminal splash renderer".into(),
                    "ISC".into(),
                ],
            }),
            Shape::Text => Body::Text(TextData {
                value: "unhappychoice/splashboard · terminal splash renderer · ISC".into(),
            }),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let slug = resolve_repo(opts.repo.as_deref())?;
        let info: RepoInfo = rest_get(&format!("/repos/{}/{}", slug.owner, slug.name)).await?;
        let meta = Metadata {
            slug: Some(format!("{}/{}", slug.owner, slug.name)),
            description: info.description.filter(|s| !s.is_empty()),
            license: info.license.and_then(|l| l.spdx_id),
        };
        let body = match ctx.shape.unwrap_or(Shape::Entries) {
            Shape::Text => Body::Text(TextData {
                value: meta.as_text(),
            }),
            Shape::TextBlock => Body::TextBlock(TextBlockData {
                lines: meta.as_lines(),
            }),
            _ => Body::Entries(EntriesData {
                items: meta.as_entries(),
            }),
        };
        Ok(payload(body))
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

#[derive(Debug, Deserialize)]
struct RepoInfo {
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    license: Option<LicenseInfo>,
}

#[derive(Debug, Deserialize)]
struct LicenseInfo {
    #[serde(default)]
    spdx_id: Option<String>,
}

#[derive(Default)]
struct Metadata {
    slug: Option<String>,
    description: Option<String>,
    license: Option<String>,
}

impl Metadata {
    fn as_entries(&self) -> Vec<Entry> {
        [
            ("slug", self.slug.as_deref()),
            ("description", self.description.as_deref()),
            ("license", self.license.as_deref()),
        ]
        .into_iter()
        .filter_map(|(k, v)| v.map(|val| entry(k, val)))
        .collect()
    }
    fn as_text(&self) -> String {
        [
            self.slug.as_deref(),
            self.description.as_deref(),
            self.license.as_deref(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ")
    }
    fn as_lines(&self) -> Vec<String> {
        [
            self.slug.as_deref(),
            self.description.as_deref(),
            self.license.as_deref(),
        ]
        .into_iter()
        .flatten()
        .map(str::to_owned)
        .collect()
    }
}

fn entry(key: &str, value: &str) -> Entry {
    Entry {
        key: key.into(),
        value: Some(value.into()),
        status: None,
    }
}

fn payload(body: Body) -> Payload {
    Payload {
        icon: None,
        status: None,
        format: None,
        body,
    }
}

