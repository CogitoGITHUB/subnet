//! `todoist_tasks` — Todoist task snapshot with structured filters.
//!
//! Safety::Safe: every request targets Todoist's fixed REST host (`api.todoist.com`), so
//! config cannot redirect tokens to arbitrary origins.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Datelike, FixedOffset, NaiveDate};
use reqwest::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Body, EntriesData, Entry, LinkedLine, LinkedTextBlockData, Payload, Status,
    TextBlockData, TextData, TimelineData, TimelineEvent,
};
use crate::render::Shape;
use crate::samples;
use crate::time as t;

use super::{FetchContext, FetchError, Fetcher, Safety};

const API_TASKS: &str = "https://api.todoist.com/api/v1/tasks";
const API_TASKS_FILTER: &str = "https://api.todoist.com/api/v1/tasks/filter";
const USER_AGENT: &str = concat!("splashboard/", env!("CARGO_PKG_VERSION"));
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_MAX_ITEMS: usize = 10;
const MAX_ITEMS_CAP: usize = 100;
const SHAPES: &[Shape] = &[
    Shape::LinkedTextBlock,
    Shape::Text,
    Shape::TextBlock,
    Shape::Entries,
    Shape::Timeline,
    Shape::Badge,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "token",
        type_hint: "string",
        required: false,
        default: None,
        description: "Todoist API token. If omitted, `TODOIST_TOKEN` env var is used.",
    },
    OptionSchema {
        name: "filter_due",
        type_hint: "\"today\" | \"overdue\" | \"upcoming\" | \"all\"",
        required: false,
        default: Some("\"all\""),
        description: "Due-date window.",
    },
    OptionSchema {
        name: "filter_include_overdue",
        type_hint: "boolean",
        required: false,
        default: Some("true"),
        description: "When `filter_due = \"today\"`, include overdue tasks too.",
    },
    OptionSchema {
        name: "filter_projects",
        type_hint: "array of strings",
        required: false,
        default: None,
        description: "Project names matched as an OR clause (`#ProjectA | #ProjectB`).",
    },
    OptionSchema {
        name: "filter_labels",
        type_hint: "array of strings",
        required: false,
        default: None,
        description: "Label names matched as an OR clause (`@urgent | @backend`).",
    },
    OptionSchema {
        name: "filter_priorities",
        type_hint: "array of integers (1..=4)",
        required: false,
        default: None,
        description: "Priority filter matched as an OR clause (`p1 | p2`).",
    },
    OptionSchema {
        name: "max_items",
        type_hint: "integer (1..=100)",
        required: false,
        default: Some("10"),
        description: "Maximum rendered tasks for list-like shapes (TextBlock/Entries/Timeline).",
    },
];

pub fn fetchers() -> Vec<Arc<dyn Fetcher>> {
    vec![Arc::new(TodoistTasks)]
}

pub struct TodoistTasks;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    #[serde(alias = "due")]
    filter_due: Option<DueWindow>,
    #[serde(default)]
    #[serde(alias = "include_overdue")]
    filter_include_overdue: Option<bool>,
    #[serde(default)]
    #[serde(alias = "projects")]
    filter_projects: Option<Vec<String>>,
    #[serde(default)]
    #[serde(alias = "labels")]
    filter_labels: Option<Vec<String>>,
    #[serde(default)]
    #[serde(alias = "priorities")]
    filter_priorities: Option<Vec<u8>>,
    #[serde(default)]
    max_items: Option<usize>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum DueWindow {
    Today,
    Overdue,
    Upcoming,
    #[default]
    All,
}

#[derive(Debug, Deserialize)]
struct ApiTask {
    id: String,
    content: String,
    priority: u8,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    due: Option<ApiDue>,
}

#[derive(Debug, Deserialize)]
struct ApiTasksResponse {
    results: Vec<ApiTask>,
}

#[derive(Debug, Deserialize)]
struct ApiDue {
    date: String,
    #[serde(default)]
    datetime: Option<String>,
}

#[derive(Debug)]
struct TaskView {
    content: String,
    priority: u8,
    url: Option<String>,
    due_label: Option<String>,
    due_sort_key: Option<i64>,
    timeline_ts: Option<i64>,
    is_overdue: bool,
}

#[async_trait]
impl Fetcher for TodoistTasks {
    fn name(&self) -> &str {
        "todoist_tasks"
    }

    fn safety(&self) -> Safety {
        Safety::Safe
    }

    fn description(&self) -> &'static str {
        "Todoist task snapshot, sorted by due date and priority, with structured filters for due window, projects, labels, and priority. `Badge` summarises overdue/total counts; list shapes show each task with its due label and priority and link back to the Todoist app."
    }

    fn refresh_interval(&self) -> u64 {
        60 * 5
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }

    fn default_shape(&self) -> Shape {
        Shape::LinkedTextBlock
    }

    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }

    fn cache_key(&self, ctx: &FetchContext) -> String {
        cache_key(self.name(), ctx)
    }

    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::LinkedTextBlock => samples::linked_text_block(&[
                (
                    "overdue · P4 Fix flaky CI on macOS runner",
                    Some("https://app.todoist.com/app/task/101"),
                ),
                (
                    "overdue · P3 Reply to design review feedback",
                    Some("https://app.todoist.com/app/task/102"),
                ),
                (
                    "today · P1 Ship v2.3 release notes",
                    Some("https://app.todoist.com/app/task/103"),
                ),
                (
                    "today · P2 Pair with @sam on auth refactor",
                    Some("https://app.todoist.com/app/task/104"),
                ),
                (
                    "today · P3 Review @lin's PR #482",
                    Some("https://app.todoist.com/app/task/105"),
                ),
                (
                    "today · P4 Update onboarding doc screenshots",
                    Some("https://app.todoist.com/app/task/106"),
                ),
                (
                    "tomorrow · P2 1:1 prep — quarterly goals",
                    Some("https://app.todoist.com/app/task/107"),
                ),
                (
                    "wed · P3 Draft proposal for caching layer",
                    Some("https://app.todoist.com/app/task/108"),
                ),
                (
                    "fri · P3 Migrate flaky integration tests",
                    Some("https://app.todoist.com/app/task/109"),
                ),
            ]),
            Shape::Text => samples::text("todo 9 tasks (2 overdue · 4 today)"),
            Shape::TextBlock => samples::text_block(&[
                "overdue · P4 Fix flaky CI on macOS runner",
                "overdue · P3 Reply to design review feedback",
                "today · P1 Ship v2.3 release notes",
                "today · P2 Pair with @sam on auth refactor",
                "today · P3 Review @lin's PR #482",
                "today · P4 Update onboarding doc screenshots",
                "tomorrow · P2 1:1 prep — quarterly goals",
                "wed · P3 Draft proposal for caching layer",
                "fri · P3 Migrate flaky integration tests",
            ]),
            Shape::Entries => samples::entries(&[
                ("Fix flaky CI on macOS runner", "overdue · P4"),
                ("Reply to design review feedback", "overdue · P3"),
                ("Ship v2.3 release notes", "today · P1"),
                ("Pair with @sam on auth refactor", "today · P2"),
                ("Review @lin's PR #482", "today · P3"),
                ("Update onboarding doc screenshots", "today · P4"),
                ("1:1 prep — quarterly goals", "tomorrow · P2"),
                ("Draft proposal for caching layer", "wed · P3"),
                ("Migrate flaky integration tests", "fri · P3"),
            ]),
            Shape::Timeline => samples::timeline(&[
                (
                    1_776_456_000,
                    "Fix flaky CI on macOS runner",
                    Some("overdue · P4"),
                ),
                (
                    1_776_542_400,
                    "Reply to design review feedback",
                    Some("overdue · P3"),
                ),
                (1_776_628_800, "Ship v2.3 release notes", Some("today · P1")),
                (
                    1_776_715_200,
                    "Pair with @sam on auth refactor",
                    Some("today · P2"),
                ),
                (1_776_801_600, "Review @lin's PR #482", Some("today · P3")),
                (
                    1_776_888_000,
                    "1:1 prep — quarterly goals",
                    Some("tomorrow · P2"),
                ),
                (
                    1_777_060_800,
                    "Draft proposal for caching layer",
                    Some("wed · P3"),
                ),
                (
                    1_777_233_600,
                    "Migrate flaky integration tests",
                    Some("fri · P3"),
                ),
            ]),
            Shape::Badge => Body::Badge(BadgeData {
                status: Status::Error,
                label: "todo 9 (2 overdue)".into(),
            }),
            _ => return None,
        })
    }

    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref())?;
        let token = resolve_token(opts.token.as_deref())?;
        let tasks = fetch_tasks(&token, build_filter(&opts)).await?;
        let now = t::now_in(ctx.timezone.as_deref());
        let mut views: Vec<TaskView> = tasks.into_iter().map(|t| to_task_view(t, &now)).collect();
        views.sort_by(cmp_views);
        let max_items = opts
            .max_items
            .unwrap_or(DEFAULT_MAX_ITEMS)
            .clamp(1, MAX_ITEMS_CAP);
        Ok(payload(render_body(
            &views,
            ctx.shape.unwrap_or(Shape::TextBlock),
            max_items,
        )))
    }
}

fn cache_key(name: &str, ctx: &FetchContext) -> String {
    let shape = ctx.shape.map(|s| s.as_str()).unwrap_or("default");
    let opts = ctx
        .options
        .as_ref()
        .map(toml::Value::to_string)
        .unwrap_or_default();
    let raw = format!(
        "{}|{}|{}|{}",
        name,
        shape,
        ctx.format.as_deref().unwrap_or(""),
        opts
    );
    let digest = Sha256::digest(raw.as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("{name}-{hex}")
}

async fn fetch_tasks(token: &str, filter: Option<String>) -> Result<Vec<ApiTask>, FetchError> {
    let (url, req) = match filter.as_deref() {
        Some(q) => (
            API_TASKS_FILTER,
            http()
                .get(API_TASKS_FILTER)
                .bearer_auth(token)
                .query(&[("query", q)]),
        ),
        None => (API_TASKS, http().get(API_TASKS).bearer_auth(token)),
    };
    let res = req
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("todoist request failed: {e}")))?;
    let status = res.status();
    let body = res
        .bytes()
        .await
        .map_err(|e| FetchError::Failed(format!("todoist response body: {e}")))?;
    if !status.is_success() {
        return Err(FetchError::Failed(todoist_error(status, &body)));
    }
    serde_json::from_slice::<ApiTasksResponse>(&body)
        .map(|v| v.results)
        .map_err(|e| FetchError::Failed(format!("todoist json parse ({url}): {e}")))
}

fn todoist_error(status: reqwest::StatusCode, body: &[u8]) -> String {
    #[derive(Deserialize)]
    struct ApiError {
        #[serde(default)]
        error: Option<String>,
        #[serde(default)]
        message: Option<String>,
    }
    let text = serde_json::from_slice::<ApiError>(body)
        .ok()
        .and_then(|e| e.error.or(e.message))
        .or_else(|| {
            std::str::from_utf8(body)
                .ok()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(ToString::to_string)
        });
    text.map(|t| format!("todoist {status}: {t}"))
        .unwrap_or_else(|| format!("todoist {status}"))
}

fn build_filter(opts: &Options) -> Option<String> {
    let due = due_clause(
        opts.filter_due.unwrap_or_default(),
        opts.filter_include_overdue.unwrap_or(true),
    );
    let projects = scoped_or_clause('#', opts.filter_projects.as_deref());
    let labels = scoped_or_clause('@', opts.filter_labels.as_deref());
    let priorities = priorities_clause(opts.filter_priorities.as_deref());
    let clauses: Vec<String> = [due, projects, labels, priorities]
        .into_iter()
        .flatten()
        .collect();
    (!clauses.is_empty()).then(|| clauses.join(" & "))
}

fn due_clause(due: DueWindow, include_overdue: bool) -> Option<String> {
    match due {
        DueWindow::Today => Some(if include_overdue {
            "(today | overdue)".into()
        } else {
            "today".into()
        }),
        DueWindow::Overdue => Some("overdue".into()),
        DueWindow::Upcoming => Some("7 days & !today".into()),
        DueWindow::All => None,
    }
}

fn scoped_or_clause(prefix: char, values: Option<&[String]>) -> Option<String> {
    let terms: Vec<String> = values
        .unwrap_or(&[])
        .iter()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(|v| format!("{prefix}{}", escape_filter_atom(v)))
        .collect();
    or_clause(&terms)
}

fn priorities_clause(values: Option<&[u8]>) -> Option<String> {
    let terms: Vec<String> = values
        .unwrap_or(&[])
        .iter()
        .filter(|v| (1u8..=4u8).contains(v))
        .map(|v| format!("p{v}"))
        .collect();
    or_clause(&terms)
}

fn or_clause(values: &[String]) -> Option<String> {
    match values.len() {
        0 => None,
        1 => Some(values[0].clone()),
        _ => Some(format!("({})", values.join(" | "))),
    }
}

fn escape_filter_atom(value: &str) -> String {
    let plain = value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if plain {
        value.to_string()
    } else {
        format!("\"{}\"", value.replace('"', "\\\""))
    }
}

fn to_task_view(task: ApiTask, now: &DateTime<FixedOffset>) -> TaskView {
    let created_ts = task
        .created_at
        .as_deref()
        .and_then(parse_rfc3339_timestamp)
        .unwrap_or_else(|| now.timestamp());
    let (due_label, due_sort_key, timeline_ts, is_overdue) = task
        .due
        .as_ref()
        .map(|d| parse_due(d, now))
        .unwrap_or((None, None, None, false));
    TaskView {
        content: task.content,
        priority: task.priority.clamp(1, 4),
        url: task_link(&task.id),
        due_label,
        due_sort_key,
        timeline_ts: timeline_ts.or(Some(created_ts)),
        is_overdue,
    }
}

fn parse_due(
    due: &ApiDue,
    now: &DateTime<FixedOffset>,
) -> (Option<String>, Option<i64>, Option<i64>, bool) {
    let tz = *now.offset();
    if let Some(dt) = due.datetime.as_deref().and_then(parse_rfc3339_timestamp) {
        let today = now.date_naive();
        let date_label = DateTime::from_timestamp(dt, 0)
            .map(|d| d.with_timezone(&tz).date_naive())
            .map(|d| due_label_from_date(d, today))
            .unwrap_or_else(|| "scheduled".into());
        return (Some(date_label), Some(dt), Some(dt), dt < now.timestamp());
    }
    NaiveDate::parse_from_str(&due.date, "%Y-%m-%d")
        .ok()
        .map(|date| {
            let ts = date
                .and_hms_opt(12, 0, 0)
                .map(|v| v.and_utc().timestamp())
                .unwrap_or(0);
            (
                Some(due_label_from_date(date, now.date_naive())),
                Some(ts),
                Some(ts),
                date < now.date_naive(),
            )
        })
        .unwrap_or((None, None, None, false))
}

fn due_label_from_date(date: NaiveDate, today: NaiveDate) -> String {
    if date < today {
        "overdue".into()
    } else if date == today {
        "today".into()
    } else if date == today.succ_opt().unwrap_or(today) {
        "tomorrow".into()
    } else {
        format!("{:02}-{:02}", date.month(), date.day())
    }
}

fn parse_rfc3339_timestamp(raw: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.timestamp())
}

fn cmp_views(a: &TaskView, b: &TaskView) -> std::cmp::Ordering {
    a.due_sort_key
        .unwrap_or(i64::MAX)
        .cmp(&b.due_sort_key.unwrap_or(i64::MAX))
        .then_with(|| b.priority.cmp(&a.priority))
        .then_with(|| a.content.cmp(&b.content))
}

fn render_body(tasks: &[TaskView], shape: Shape, max_items: usize) -> Body {
    let overdue_count = tasks.iter().filter(|t| t.is_overdue).count();
    match shape {
        Shape::Text => Body::Text(TextData {
            value: format!("todo {} tasks ({} overdue)", tasks.len(), overdue_count),
        }),
        Shape::LinkedTextBlock => Body::LinkedTextBlock(LinkedTextBlockData {
            items: tasks
                .iter()
                .take(max_items)
                .map(|t| LinkedLine {
                    text: task_line(t),
                    url: t.url.clone(),
                })
                .collect(),
        }),
        Shape::Entries => Body::Entries(EntriesData {
            items: tasks
                .iter()
                .take(max_items)
                .map(|t| Entry {
                    key: t.content.clone(),
                    value: Some(task_meta(t)),
                    status: None,
                })
                .collect(),
        }),
        Shape::Timeline => Body::Timeline(TimelineData {
            events: tasks
                .iter()
                .filter_map(|t| {
                    t.timeline_ts.map(|ts| TimelineEvent {
                        timestamp: ts,
                        title: t.content.clone(),
                        detail: Some(task_meta(t)),
                        status: t.is_overdue.then_some(Status::Error),
                    })
                })
                .take(max_items)
                .collect(),
        }),
        Shape::Badge => Body::Badge(BadgeData {
            status: badge_status(tasks.len(), overdue_count),
            label: format!("todo {} ({} overdue)", tasks.len(), overdue_count),
        }),
        _ => Body::TextBlock(TextBlockData {
            lines: tasks.iter().take(max_items).map(task_line).collect(),
        }),
    }
}

fn badge_status(total: usize, overdue: usize) -> Status {
    if overdue > 0 {
        Status::Error
    } else if total > 0 {
        Status::Warn
    } else {
        Status::Ok
    }
}

fn task_line(task: &TaskView) -> String {
    format!("{} {}", task_meta(task), task.content)
}

fn task_meta(task: &TaskView) -> String {
    let due = task.due_label.as_deref().unwrap_or("no due");
    format!("{due} · P{}", task.priority)
}

fn task_link(id: &str) -> Option<String> {
    (!id.is_empty()).then(|| format!("https://app.todoist.com/app/task/{id}"))
}

fn parse_options(raw: Option<&toml::Value>) -> Result<Options, FetchError> {
    raw.map(|v| {
        v.clone()
            .try_into::<Options>()
            .map_err(|e| FetchError::Failed(format!("invalid options: {e}")))
    })
    .unwrap_or_else(|| Ok(Options::default()))
}

fn resolve_token(config_token: Option<&str>) -> Result<String, FetchError> {
    config_token
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .or_else(|| std::env::var("TODOIST_TOKEN").ok())
        .ok_or_else(|| {
            FetchError::Failed("todoist token missing: set options.token or TODOIST_TOKEN".into())
        })
}

fn http() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .user_agent(USER_AGENT)
            .timeout(REQUEST_TIMEOUT)
            .gzip(true)
            .build()
            .expect("reqwest client should build with default config")
    })
}

fn payload(body: Body) -> Payload {
    Payload {
        icon: None,
        status: None,
        format: None,
        body,
    }
}

