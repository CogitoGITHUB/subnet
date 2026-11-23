//! `github_action_history` — recent workflow runs mapped either to a pass/fail number series
//! (`NumberSeries`, 1 = success / 0 = anything else — oldest first, feeds `sparkline`) or a
//! timeline of the most recent N runs.

use async_trait::async_trait;
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{
    Bar, BarsData, Body, NumberSeriesData, Payload, PointSeries, PointSeriesData, Status,
    TimelineData, TimelineEvent,
};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::client::rest_get;
use super::common::{RepoSlug, cache_key, parse_options, parse_timestamp, payload, resolve_repo};

const SHAPES: &[Shape] = &[
    Shape::NumberSeries,
    Shape::Timeline,
    Shape::PointSeries,
    Shape::Bars,
];
const DEFAULT_LIMIT: u32 = 30;

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "repo",
        type_hint: "\"owner/name\"",
        required: false,
        default: Some("git remote of cwd"),
        description: "Repository to query.",
    },
    OptionSchema {
        name: "limit",
        type_hint: "integer (1..=100)",
        required: false,
        default: Some("30"),
        description: "Number of recent runs to return.",
    },
    OptionSchema {
        name: "branch",
        type_hint: "string",
        required: false,
        default: None,
        description: "Branch filter. Omit for runs across all branches.",
    },
];

pub struct GithubActionHistory;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub branch: Option<String>,
}

#[async_trait]
impl Fetcher for GithubActionHistory {
    fn name(&self) -> &str {
        "github_action_history"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Recent CI workflow runs as a pass/fail sparkline (`NumberSeries`), a timeline of the last N runs, a duration scatter plot of `(run_number, seconds)` for spotting CI slowdowns (`PointSeries`), or a passed/failed count breakdown (`Bars`). Use `github_action_status` instead for just the current main-branch state."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 5
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
        Some(match shape {
            Shape::NumberSeries => samples::number_series(&[
                1, 1, 0, 1, 1, 1, 0, 1, 1, 1, 1, 0, 1, 1, 1, 0, 1, 1, 1, 1, 0, 0, 1, 1, 1, 1, 1, 0,
                1, 1,
            ]),
            Shape::Timeline => samples::timeline(&[
                (1_776_000_000, "#4235 main", Some("passing")),
                (1_775_800_000, "#4234 feat/auth-refactor", Some("failing")),
                (1_775_600_000, "#4233 main", Some("passing")),
                (1_775_400_000, "#4232 main", Some("passing")),
                (1_775_200_000, "#4231 hotfix/login", Some("passing")),
                (1_775_000_000, "#4230 main", Some("failing")),
                (1_774_800_000, "#4229 develop", Some("passing")),
                (1_774_600_000, "#4228 main", Some("passing")),
            ]),
            Shape::PointSeries => Body::PointSeries(PointSeriesData {
                series: vec![PointSeries {
                    name: "ci duration (s)".into(),
                    points: vec![
                        (4220.0, 138.0),
                        (4221.0, 145.0),
                        (4222.0, 152.0),
                        (4223.0, 148.0),
                        (4224.0, 162.0),
                        (4225.0, 158.0),
                        (4226.0, 175.0),
                        (4227.0, 168.0),
                        (4228.0, 142.0),
                        (4229.0, 156.0),
                        (4230.0, 220.0),
                        (4231.0, 195.0),
                        (4232.0, 148.0),
                        (4233.0, 142.0),
                        (4234.0, 168.0),
                        (4235.0, 138.0),
                        (4236.0, 220.0),
                        (4237.0, 145.0),
                    ],
                }],
            }),
            Shape::Bars => Body::Bars(BarsData {
                bars: vec![
                    Bar {
                        label: "passed".into(),
                        value: 25,
                        value_label: None,
                    },
                    Bar {
                        label: "failed".into(),
                        value: 5,
                        value_label: None,
                    },
                ],
            }),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let slug = resolve_repo(opts.repo.as_deref())?;
        let limit = opts.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 100);
        let mut path = format!(
            "/repos/{}/{}/actions/runs?per_page={limit}",
            slug.owner, slug.name
        );
        if let Some(branch) = opts.branch.as_deref() {
            path.push_str(&format!("&branch={branch}"));
        }
        let res: RunsResponse = rest_get(&path).await?;
        Ok(payload(render_body(
            &res.workflow_runs,
            ctx.shape.unwrap_or(Shape::NumberSeries),
        )))
    }
}

#[derive(Debug, Deserialize)]
struct RunsResponse {
    #[serde(default)]
    workflow_runs: Vec<WorkflowRun>,
}

#[derive(Debug, Deserialize)]
struct WorkflowRun {
    #[serde(default)]
    run_number: u64,
    #[serde(default)]
    status: String,
    #[serde(default)]
    conclusion: Option<String>,
    #[serde(default)]
    head_branch: Option<String>,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    updated_at: String,
}

fn render_body(runs: &[WorkflowRun], shape: Shape) -> Body {
    match shape {
        Shape::Timeline => Body::Timeline(TimelineData {
            events: runs
                .iter()
                .map(|r| TimelineEvent {
                    timestamp: parse_timestamp(&r.updated_at),
                    title: format!(
                        "#{} {}",
                        r.run_number,
                        r.head_branch.as_deref().unwrap_or("?")
                    ),
                    detail: Some(label(r)),
                    status: Some(status_of(r)),
                })
                .collect(),
        }),
        Shape::PointSeries => Body::PointSeries(PointSeriesData {
            series: vec![PointSeries {
                name: "ci duration (s)".into(),
                points: runs
                    .iter()
                    .rev()
                    .filter_map(|r| duration_seconds(r).map(|s| (r.run_number as f64, s)))
                    .collect(),
            }],
        }),
        Shape::Bars => {
            let passed = runs
                .iter()
                .filter(|r| matches!(r.conclusion.as_deref(), Some("success")))
                .count() as u64;
            let failed = (runs.len() as u64).saturating_sub(passed);
            Body::Bars(BarsData {
                bars: vec![
                    Bar {
                        label: "passed".into(),
                        value: passed,
                        value_label: None,
                    },
                    Bar {
                        label: "failed".into(),
                        value: failed,
                        value_label: None,
                    },
                ],
            })
        }
        _ => Body::NumberSeries(NumberSeriesData {
            values: runs
                .iter()
                .rev()
                .map(|r| {
                    if matches!(r.conclusion.as_deref(), Some("success")) {
                        1
                    } else {
                        0
                    }
                })
                .collect(),
        }),
    }
}

fn duration_seconds(r: &WorkflowRun) -> Option<f64> {
    let start = parse_timestamp(&r.created_at);
    let end = parse_timestamp(&r.updated_at);
    if start <= 0 || end <= 0 || end < start {
        return None;
    }
    Some((end - start) as f64)
}

fn status_of(r: &WorkflowRun) -> Status {
    if r.status == "completed" {
        match r.conclusion.as_deref() {
            Some("success") => Status::Ok,
            Some("failure") | Some("timed_out") | Some("startup_failure") => Status::Error,
            _ => Status::Warn,
        }
    } else {
        Status::Warn
    }
}

fn label(r: &WorkflowRun) -> String {
    if r.status == "completed" {
        r.conclusion.clone().unwrap_or_else(|| "completed".into())
    } else {
        r.status.clone()
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

