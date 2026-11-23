//! `gitlab_pipeline_status` — current pipeline state for a GitLab project as a single badge,
//! optional text line, or `Entries` rollup (status / branch / sha / duration). Uses
//! `/api/v4/projects/{id}/pipelines?per_page=1[&ref=<branch>]`.

use async_trait::async_trait;
use serde::Deserialize;
use url::form_urlencoded::byte_serialize;

use crate::options::OptionSchema;
use crate::payload::{BadgeData, Body, EntriesData, Entry, Payload, Status, TextData};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::client::{default_host, rest_get};
use super::common::{ProjectPath, cache_key, parse_options, payload, resolve_project};
use super::my_mrs::resolve_host;

const SHAPES: &[Shape] = &[Shape::Badge, Shape::Text, Shape::Entries];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "host",
        type_hint: "hostname",
        required: false,
        default: Some("gitlab.com"),
        description: "GitLab instance host. Use this for self-hosted GitLab.",
    },
    OptionSchema {
        name: "project",
        type_hint: "\"group/name\" or \"group/sub/name\"",
        required: false,
        default: Some("git remote of cwd"),
        description: "Project to query. Falls back to the current directory's GitLab remote.",
    },
    OptionSchema {
        name: "branch",
        type_hint: "string",
        required: false,
        default: None,
        description: "Branch (ref) to filter the latest pipeline by. Omit for the most recent pipeline on any branch.",
    },
];

pub struct GitlabPipelineStatus;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
}

#[async_trait]
impl Fetcher for GitlabPipelineStatus {
    fn name(&self) -> &str {
        "gitlab_pipeline_status"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Latest CI pipeline run for a GitLab project as a pass/fail badge, short text line, or status rollup."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 5
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn default_shape(&self) -> Shape {
        Shape::Badge
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        cache_key(self.name(), ctx, &cache_extra(ctx))
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Badge => samples::badge(Status::Ok, "main · passing"),
            Shape::Text => samples::text("main · passing"),
            Shape::Entries => Body::Entries(EntriesData {
                items: vec![
                    entry("status", Some(Status::Ok), "passing"),
                    entry("branch", None, "main"),
                    entry("sha", None, "abc123de"),
                    entry("duration", None, "3m 12s"),
                ],
            }),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let host = resolve_host(opts.host.as_deref())?;
        let project = resolve_project(opts.project.as_deref(), &host)?;
        let mut path = format!("/projects/{}/pipelines?per_page=1", project.encoded());
        if let Some(branch) = opts.branch.as_deref() {
            // Branch names can carry `&`, `=`, `#`, `+`, `?`, and slashes (release/v2,
            // feature/foo&bar). Naive interpolation rewrote the URL semantics and silently
            // returned the wrong pipeline; percent-encode so the value stays a single
            // `ref=<branch>` pair.
            let encoded: String = byte_serialize(branch.as_bytes()).collect();
            path.push_str(&format!("&ref={encoded}"));
        }
        let pipelines: Vec<Pipeline> = rest_get(&host, &path).await?;
        let shape = ctx.shape.unwrap_or(Shape::Badge);
        let Some(latest) = pipelines.into_iter().next() else {
            return Ok(payload(no_runs_body(shape)));
        };
        Ok(payload(render_body(&latest, shape)))
    }
}

fn no_runs_body(shape: Shape) -> Body {
    match shape {
        Shape::Text => Body::Text(TextData {
            value: "no runs".into(),
        }),
        Shape::Entries => Body::Entries(EntriesData {
            items: vec![
                entry("status", Some(Status::Warn), "no runs"),
                entry("branch", None, "?"),
                entry("sha", None, "?"),
                entry("duration", None, "?"),
            ],
        }),
        _ => Body::Badge(BadgeData {
            status: Status::Warn,
            label: "no runs".into(),
        }),
    }
}

#[derive(Debug, Deserialize)]
struct Pipeline {
    #[serde(default)]
    status: String,
    #[serde(default, rename = "ref")]
    branch: Option<String>,
    #[serde(default)]
    sha: Option<String>,
    #[serde(default)]
    duration: Option<u64>,
}

fn render_body(p: &Pipeline, shape: Shape) -> Body {
    let (status, label_word) = classify(&p.status);
    let branch = p.branch.as_deref().unwrap_or("?");
    match shape {
        Shape::Text => Body::Text(TextData {
            value: format!("{branch} · {label_word}"),
        }),
        Shape::Entries => Body::Entries(EntriesData {
            items: vec![
                entry("status", Some(status), label_word),
                entry("branch", None, branch),
                entry("sha", None, &short_sha(p.sha.as_deref())),
                entry("duration", None, &format_duration(p.duration)),
            ],
        }),
        _ => Body::Badge(BadgeData {
            status,
            label: format!("{branch} · {label_word}"),
        }),
    }
}

fn classify(raw: &str) -> (Status, &'static str) {
    match raw {
        "success" => (Status::Ok, "passing"),
        "failed" => (Status::Error, "failing"),
        "canceled" => (Status::Warn, "cancelled"),
        "skipped" => (Status::Warn, "skipped"),
        "manual" => (Status::Warn, "manual"),
        "running" | "pending" | "created" | "waiting_for_resource" | "preparing" => {
            (Status::Warn, "running")
        }
        _ => (Status::Warn, "unknown"),
    }
}

fn entry(key: &str, status: Option<Status>, value: &str) -> Entry {
    Entry {
        key: key.into(),
        value: Some(value.into()),
        status,
    }
}

fn short_sha(sha: Option<&str>) -> String {
    sha.unwrap_or("?").chars().take(8).collect::<String>()
}

fn format_duration(seconds: Option<u64>) -> String {
    let Some(s) = seconds else {
        return "?".into();
    };
    let m = s / 60;
    let r = s % 60;
    if m == 0 {
        format!("{r}s")
    } else {
        format!("{m}m {r:02}s")
    }
}

fn cache_extra(ctx: &FetchContext) -> String {
    let host = ctx
        .options
        .as_ref()
        .and_then(|v| v.get("host"))
        .and_then(|v| v.as_str())
        .unwrap_or(default_host())
        .to_string();
    let project = ctx
        .options
        .as_ref()
        .and_then(|v| v.get("project"))
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| {
            resolve_project(None, &host)
                .ok()
                .map(|p: ProjectPath| p.as_str().to_string())
        })
        .unwrap_or_default();
    let branch = ctx
        .options
        .as_ref()
        .and_then(|v| v.get("branch"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    format!("{host}|{project}|{branch}")
}

