//! `codex_usage` — token + cost rollup from local Codex CLI session JSONL files.
//!
//! Walks `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` (or `$CODEX_HOME/sessions/...`),
//! filters lines to `event_msg` of type `token_count` carrying per-turn `last_token_usage`,
//! then groups by project / model / day depending on `group_by`. The `turn_context` event
//! that precedes each turn carries the model id and `cwd`; we thread that state forward as
//! we parse so token_count events get attributed to the right turn.
//!
//! `Safety::Safe` — every session read is rooted at a `$HOME`-relative directory the user
//! owns. The fetcher *does* make a single HTTP GET for the LLM pricing snapshot (see
//! [`crate::fetcher::llm_pricing`]) — the URL is hardcoded to splashboard's own GitHub repo,
//! which keeps it under the host-fixed-URL Safe rule. On HTTP failure we fall back to the
//! embedded floor so cost columns stay populated for the headline models.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::Deserialize;

use crate::fetcher::codex::common::{
    discover_session_dirs, format_cost, format_tokens, list_session_files, project_name_from_cwd,
    short_model_name,
};
use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::llm_pricing::{self, PriceMap};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Bar, BarsData, Body, EntriesData, Entry, MarkdownTextBlockData, NumberSeriesData,
    Payload, Status, TextBlockData, TextData,
};
use crate::render::Shape;

const SHAPES: &[Shape] = &[
    Shape::Text,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::Entries,
    Shape::Bars,
    Shape::NumberSeries,
    Shape::Badge,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "since",
        type_hint: "\"today\" | \"7d\" | \"30d\" | \"all\"",
        required: false,
        default: Some("\"today\""),
        description: "Window the rollup covers, anchored on the current UTC day.",
    },
    OptionSchema {
        name: "limit",
        type_hint: "integer (1..=50)",
        required: false,
        default: Some("10"),
        description: "Maximum rows surfaced by multi-row shapes (`Bars`, `Entries`, …).",
    },
    OptionSchema {
        name: "group_by",
        type_hint: "\"project\" | \"model\" | \"day\"",
        required: false,
        default: Some("\"project\""),
        description: "Axis used by `Bars` / `Entries` / `TextBlock` row shapes.",
    },
];

const DEFAULT_LIMIT: u32 = 10;
const MAX_LIMIT: u32 = 50;

pub struct CodexUsage;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    pub since: Option<String>,
    pub limit: Option<u32>,
    pub group_by: Option<String>,
}

#[async_trait]
impl Fetcher for CodexUsage {
    fn name(&self) -> &str {
        "codex_usage"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Token and cost rollup from local Codex CLI session JSONL files. Aggregates per-turn usage under the chosen `since` window and groups it by project, model, or day. Pricing is bundled, so the splash works offline."
    }
    fn refresh_interval(&self) -> u64 {
        // Mirrors claude_code_usage: JSONL only grows when Codex is actively running, so a
        // 5-minute refresh tracks "what I just spent" without re-walking MBs every cd.
        5 * 60
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        let extra = ctx
            .options
            .as_ref()
            .and_then(|v| toml::to_string(v).ok())
            .unwrap_or_default();
        cache_key(self.name(), ctx, &extra)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        sample_body(shape)
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let since = parse_since(opts.since.as_deref())?;
        let limit = opts.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT) as usize;
        let group_by = parse_group_by(opts.group_by.as_deref())?;
        let shape = ctx.shape.unwrap_or(Shape::Text);

        let prices = llm_pricing::fetch_pricing(http()).await;
        let snapshot = build_snapshot(&since, group_by, limit, &prices);
        Ok(payload(render_body(&snapshot, shape)))
    }
}

fn http() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(concat!("splashboard/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("reqwest client should build with default config")
    })
}

#[derive(Debug, Clone, Copy)]
enum Since {
    Today,
    Days(i64),
    All,
}

impl Since {
    fn label(&self) -> &'static str {
        match self {
            Since::Today => "Today",
            Since::Days(7) => "7d",
            Since::Days(30) => "30d",
            Since::Days(_) => "window",
            Since::All => "All-time",
        }
    }
    fn includes(&self, ts: DateTime<Utc>) -> bool {
        match self {
            Since::All => true,
            Since::Today => ts.date_naive() == Utc::now().date_naive(),
            Since::Days(n) => {
                // Calendar-day comparison so this matches `daily_series`'s bucket window exactly
                // (today .. today-(n-1)). A rolling `now - n days` cutoff would leak partial-day
                // events into the totals that never show up in the per-day series.
                let today = Utc::now().date_naive();
                let start = today - Duration::days((*n - 1).max(0));
                let day = ts.date_naive();
                day >= start && day <= today
            }
        }
    }
}

fn parse_since(raw: Option<&str>) -> Result<Since, FetchError> {
    match raw.unwrap_or("today").trim().to_lowercase().as_str() {
        "today" => Ok(Since::Today),
        "7d" => Ok(Since::Days(7)),
        "30d" => Ok(Since::Days(30)),
        "all" => Ok(Since::All),
        other => Err(FetchError::Failed(format!(
            "invalid since: {other:?} (expected today | 7d | 30d | all)"
        ))),
    }
}

#[derive(Debug, Clone, Copy)]
enum GroupBy {
    Project,
    Model,
    Day,
}

fn parse_group_by(raw: Option<&str>) -> Result<GroupBy, FetchError> {
    match raw.unwrap_or("project").trim().to_lowercase().as_str() {
        "project" => Ok(GroupBy::Project),
        "model" => Ok(GroupBy::Model),
        "day" => Ok(GroupBy::Day),
        other => Err(FetchError::Failed(format!(
            "invalid group_by: {other:?} (expected project | model | day)"
        ))),
    }
}

#[derive(Debug, Clone, Default)]
struct Snapshot {
    rows: Vec<UsageRow>,
    total_cost: f64,
    total_tokens: u64,
    daily_tokens: Vec<u64>,
    since_label: &'static str,
}

#[derive(Debug, Clone)]
struct UsageRow {
    label: String,
    tokens: u64,
    cost: f64,
}

#[derive(Debug, Clone)]
struct UsageEvent {
    timestamp: DateTime<Utc>,
    model: String,
    project: String,
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
}

impl UsageEvent {
    fn token_total(&self) -> u64 {
        // `cached_input_tokens` is a subset of `input_tokens` in OpenAI's schema; don't double-
        // count it. Reasoning tokens already roll into `output_tokens`.
        self.input_tokens + self.output_tokens
    }
    fn cost(&self, prices: &PriceMap) -> f64 {
        // OpenAI reports `cached_input_tokens` as a subset of `input_tokens`; pass the
        // non-cached chunk at the full input rate and the cached chunk at `cache_read`.
        let billable_input = self.input_tokens.saturating_sub(self.cached_input_tokens);
        llm_pricing::cost_usd(
            prices,
            &self.model,
            billable_input,
            self.output_tokens,
            self.cached_input_tokens,
            0,
            0,
        )
    }
}

fn build_snapshot(since: &Since, group_by: GroupBy, limit: usize, prices: &PriceMap) -> Snapshot {
    let events = collect_events(&list_session_files(&discover_session_dirs()), since);
    let total_tokens = events.iter().map(UsageEvent::token_total).sum();
    let total_cost: f64 = events.iter().map(|e| e.cost(prices)).sum();
    let rows = group_rows(&events, group_by, limit, prices);
    let daily_tokens = daily_series(&events, since);
    Snapshot {
        rows,
        total_cost,
        total_tokens,
        daily_tokens,
        since_label: since.label(),
    }
}

fn collect_events(files: &[PathBuf], since: &Since) -> Vec<UsageEvent> {
    // Codex session filenames are `rollout-YYYY-MM-DDTHH-MM-SS-uuid.jsonl`; skip the whole
    // file when its date prefix is older than the `since` cutoff. Saves opening hundreds of
    // MB of JSONL on cold cache when the user just wants Today / 7d.
    let cutoff = cutoff_date(since);
    files
        .iter()
        .filter(|p| filename_within_cutoff(p, cutoff))
        .flat_map(|f| read_session_file(f, since))
        .collect()
}

fn cutoff_date(since: &Since) -> Option<NaiveDate> {
    let today = Utc::now().date_naive();
    match since {
        Since::All => None,
        // One-day buffer covers sessions that started before midnight and spilled past it.
        Since::Today => Some(today - Duration::days(1)),
        Since::Days(n) => Some(today - Duration::days(*n)),
    }
}

fn filename_within_cutoff(path: &Path, cutoff: Option<NaiveDate>) -> bool {
    let Some(cutoff) = cutoff else {
        return true;
    };
    parse_filename_date(path).is_none_or(|date| date >= cutoff)
}

fn parse_filename_date(path: &Path) -> Option<NaiveDate> {
    let name = path.file_name()?.to_str()?;
    let rest = name.strip_prefix("rollout-")?;
    if rest.len() < 10 {
        return None;
    }
    NaiveDate::parse_from_str(&rest[..10], "%Y-%m-%d").ok()
}

fn read_session_file(path: &PathBuf, since: &Since) -> Vec<UsageEvent> {
    let Ok(file) = fs::File::open(path) else {
        return Vec::new();
    };
    let mut model = String::new();
    let mut cwd = String::new();
    let mut out: Vec<UsageEvent> = Vec::new();
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        // `session_meta` and `turn_context` set the current model/cwd; `token_count` events
        // consume that context. Order is sequential, so a running mutable state is enough.
        if let Some(raw) = parse_line(&line) {
            apply_event(raw, &mut model, &mut cwd, since, &mut out);
        }
    }
    out
}

fn apply_event(
    raw: RawLine,
    model: &mut String,
    cwd: &mut String,
    since: &Since,
    out: &mut Vec<UsageEvent>,
) {
    let RawLine {
        timestamp,
        kind,
        payload,
    } = raw;
    match kind.as_str() {
        "session_meta" => {
            if let Some(c) = payload.cwd {
                *cwd = c;
            }
        }
        "turn_context" => {
            if let Some(m) = payload.model {
                *model = m;
            }
            if let Some(c) = payload.cwd {
                *cwd = c;
            }
        }
        "event_msg" => {
            if let Some(usage) = token_usage_from_event(&payload)
                && let Some(ts) = parse_ts(&timestamp)
                && since.includes(ts)
            {
                out.push(UsageEvent {
                    timestamp: ts,
                    model: model.clone(),
                    project: project_name_from_cwd(cwd),
                    input_tokens: usage.input_tokens,
                    cached_input_tokens: usage.cached_input_tokens,
                    output_tokens: usage.output_tokens,
                });
            }
        }
        _ => {}
    }
}

fn parse_ts(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

#[derive(Deserialize)]
struct RawLine {
    #[serde(default)]
    timestamp: String,
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    payload: RawPayload,
}

#[derive(Default, Deserialize)]
struct RawPayload {
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    model: Option<String>,
    // `event_msg` payloads carry their own `type` discriminator (`token_count`, `task_started`,
    // …); we flatten just the fields we read so unknown subtypes don't trip the deserializer.
    #[serde(default, rename = "type")]
    msg_type: Option<String>,
    #[serde(default)]
    info: Option<RawTokenInfo>,
}

#[derive(Deserialize)]
struct RawTokenInfo {
    #[serde(default)]
    last_token_usage: Option<RawTokenUsage>,
}

#[derive(Deserialize)]
struct RawTokenUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    cached_input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
}

fn parse_line(line: &str) -> Option<RawLine> {
    serde_json::from_str(line).ok()
}

fn token_usage_from_event(payload: &RawPayload) -> Option<RawTokenUsage> {
    if payload.msg_type.as_deref()? != "token_count" {
        return None;
    }
    let info = payload.info.as_ref()?;
    info.last_token_usage.as_ref().map(|u| RawTokenUsage {
        input_tokens: u.input_tokens,
        cached_input_tokens: u.cached_input_tokens,
        output_tokens: u.output_tokens,
    })
}

fn group_rows(
    events: &[UsageEvent],
    group_by: GroupBy,
    limit: usize,
    prices: &PriceMap,
) -> Vec<UsageRow> {
    let mut by_key: HashMap<String, (u64, f64)> = HashMap::new();
    for e in events {
        let key = match group_by {
            GroupBy::Project => e.project.clone(),
            GroupBy::Model => short_model_name(&e.model),
            GroupBy::Day => e.timestamp.date_naive().to_string(),
        };
        let entry = by_key.entry(key).or_insert((0, 0.0));
        entry.0 += e.token_total();
        entry.1 += e.cost(prices);
    }
    let mut rows: Vec<UsageRow> = by_key
        .into_iter()
        .map(|(label, (tokens, cost))| UsageRow {
            label,
            tokens,
            cost,
        })
        .collect();
    rows.sort_by(|a, b| {
        b.cost
            .partial_cmp(&a.cost)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.tokens.cmp(&a.tokens))
    });
    rows.truncate(limit);
    rows
}

fn daily_series(events: &[UsageEvent], since: &Since) -> Vec<u64> {
    let span_days = match since {
        Since::Today => 1,
        Since::Days(n) => *n as usize,
        Since::All => 30,
    };
    let mut buckets: HashMap<NaiveDate, u64> = HashMap::new();
    for e in events {
        *buckets.entry(e.timestamp.date_naive()).or_default() += e.token_total();
    }
    let today = Utc::now().date_naive();
    (0..span_days)
        .rev()
        .map(|i| {
            let day = today - Duration::days(i as i64);
            buckets.get(&day).copied().unwrap_or(0)
        })
        .collect()
}

fn render_body(snap: &Snapshot, shape: Shape) -> Body {
    match shape {
        Shape::Text => text_body(snap),
        Shape::TextBlock => text_block_body(snap),
        Shape::MarkdownTextBlock => markdown_body(snap),
        Shape::Entries => entries_body(snap),
        Shape::Bars => bars_body(snap),
        Shape::NumberSeries => number_series_body(snap),
        Shape::Badge => badge_body(snap),
        _ => text_body(snap),
    }
}

fn text_body(snap: &Snapshot) -> Body {
    Body::Text(TextData {
        value: headline(snap),
    })
}

fn headline(snap: &Snapshot) -> String {
    if snap.total_tokens == 0 {
        format!("{}: no sessions", snap.since_label)
    } else {
        format!(
            "{}: {} tokens / {}",
            snap.since_label,
            format_tokens(snap.total_tokens),
            format_cost(snap.total_cost),
        )
    }
}

fn text_block_body(snap: &Snapshot) -> Body {
    let mut lines = vec![headline(snap)];
    lines.extend(snap.rows.iter().map(format_row));
    if snap.rows.is_empty() && snap.total_tokens == 0 {
        lines.push("no recent Codex CLI activity".into());
    }
    Body::TextBlock(TextBlockData { lines })
}

fn format_row(row: &UsageRow) -> String {
    format!(
        "{}  {} / {}",
        row.label,
        format_tokens(row.tokens),
        format_cost(row.cost),
    )
}

fn markdown_body(snap: &Snapshot) -> Body {
    let mut lines = vec![format!("**{}**", headline(snap))];
    lines.extend(snap.rows.iter().map(|r| {
        format!(
            "- **{}** — {} / {}",
            r.label,
            format_tokens(r.tokens),
            format_cost(r.cost),
        )
    }));
    Body::MarkdownTextBlock(MarkdownTextBlockData {
        value: lines.join("\n"),
    })
}

fn entries_body(snap: &Snapshot) -> Body {
    let items = if snap.rows.is_empty() {
        vec![Entry {
            key: snap.since_label.into(),
            value: Some(headline(snap)),
            status: None,
        }]
    } else {
        snap.rows
            .iter()
            .map(|r| Entry {
                key: r.label.clone(),
                value: Some(format!(
                    "{} / {}",
                    format_tokens(r.tokens),
                    format_cost(r.cost)
                )),
                status: None,
            })
            .collect()
    };
    Body::Entries(EntriesData { items })
}

fn bars_body(snap: &Snapshot) -> Body {
    // Bars use raw token counts so chart_bar still ranks correctly; the value_label carries the
    // formatted cost so list_ranking presents money instead of seven-digit token counts.
    let bars = snap
        .rows
        .iter()
        .map(|r| Bar {
            label: r.label.clone(),
            value: r.tokens,
            value_label: Some(format_cost(r.cost)),
        })
        .collect();
    Body::Bars(BarsData { bars })
}

fn number_series_body(snap: &Snapshot) -> Body {
    Body::NumberSeries(NumberSeriesData {
        values: snap.daily_tokens.clone(),
    })
}

fn badge_body(snap: &Snapshot) -> Body {
    let (status, label) = if snap.total_tokens == 0 {
        (Status::Warn, format!("{}: quiet", snap.since_label))
    } else {
        let tone = if snap.total_cost >= 50.0 {
            Status::Error
        } else if snap.total_cost >= 10.0 {
            Status::Warn
        } else {
            Status::Ok
        };
        (
            tone,
            format!("{} {}", snap.since_label, format_cost(snap.total_cost)),
        )
    };
    Body::Badge(BadgeData { status, label })
}

fn sample_body(shape: Shape) -> Option<Body> {
    use crate::samples;
    Some(match shape {
        Shape::Text => samples::text("Today: 1.8M tokens / $7.42"),
        Shape::TextBlock => samples::text_block(&[
            "Today: 1.8M tokens / $7.42",
            "splashboard  1.2M / $4.80",
            "image_gen    600k / $2.62",
        ]),
        Shape::MarkdownTextBlock => samples::markdown(
            "**Today: 1.8M tokens / $7.42**\n- **splashboard** — 1.2M / $4.80\n- **image_gen** — 600k / $2.62",
        ),
        Shape::Entries => samples::entries(&[
            ("splashboard", "1.2M / $4.80"),
            ("image_gen", "600k / $2.62"),
        ]),
        Shape::Bars => Body::Bars(BarsData {
            bars: vec![
                Bar {
                    label: "splashboard".into(),
                    value: 1_200_000,
                    value_label: Some("$4.80".into()),
                },
                Bar {
                    label: "image_gen".into(),
                    value: 600_000,
                    value_label: Some("$2.62".into()),
                },
            ],
        }),
        Shape::NumberSeries => {
            samples::number_series(&[0, 80_000, 200_000, 500_000, 700_000, 1_400_000, 1_800_000])
        }
        Shape::Badge => samples::badge(Status::Warn, "Today $7.42"),
        _ => return None,
    })
}

