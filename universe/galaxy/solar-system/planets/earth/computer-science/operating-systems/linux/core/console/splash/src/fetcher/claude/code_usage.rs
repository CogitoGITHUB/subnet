//! `claude_code_usage` — token + cost rollup from local Claude Code session JSONL files.
//!
//! Walks `<root>/projects/<encoded-cwd>/<session-uuid>.jsonl` under every root in
//! [`crate::fetcher::claude::common::discover_jsonl_dirs`], filters lines to assistant events
//! since the configured window, deduplicates by `(message.id, request_id)` keeping the row with
//! the higher token total, then groups by project / model / day depending on `group_by`.
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

use crate::fetcher::claude::common::{
    discover_jsonl_dirs, format_cost, format_tokens, project_name_from_cwd,
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

pub struct ClaudeCodeUsage;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    pub since: Option<String>,
    pub limit: Option<u32>,
    pub group_by: Option<String>,
}

#[async_trait]
impl Fetcher for ClaudeCodeUsage {
    fn name(&self) -> &str {
        "claude_code_usage"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Token and cost rollup from local Claude Code session JSONL files. Aggregates assistant turns under the chosen `since` window and groups them by project, model, or day. Pricing is bundled, so the splash works offline."
    }
    fn refresh_interval(&self) -> u64 {
        // The JSONL files only grow when the user is actively running Claude Code; a 5-minute
        // refresh is fast enough to mirror "what I just spent" without re-walking MBs every cd.
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
            // Calendar-day comparison so this matches `daily_series`'s bucket window exactly
            // (today .. today-(n-1)). A rolling `now - n days` cutoff would leak partial-day
            // events into the totals that never show up in the per-day series.
            Since::Days(n) => {
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
    message_id: String,
    request_id: String,
    input_tokens: u64,
    output_tokens: u64,
    cache_write_5m: u64,
    cache_write_1h: u64,
    cache_read: u64,
}

impl UsageEvent {
    fn token_total(&self) -> u64 {
        self.input_tokens
            + self.output_tokens
            + self.cache_write_5m
            + self.cache_write_1h
            + self.cache_read
    }
    fn cost(&self, prices: &PriceMap) -> f64 {
        llm_pricing::cost_usd(
            prices,
            &self.model,
            self.input_tokens,
            self.output_tokens,
            self.cache_read,
            self.cache_write_5m,
            self.cache_write_1h,
        )
    }
}

fn build_snapshot(since: &Since, group_by: GroupBy, limit: usize, prices: &PriceMap) -> Snapshot {
    let events = dedup(collect_events(&discover_jsonl_dirs(), since));
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

fn collect_events(roots: &[PathBuf], since: &Since) -> Vec<UsageEvent> {
    roots.iter().flat_map(|r| walk_jsonl(r, since)).collect()
}

fn walk_jsonl(root: &PathBuf, since: &Since) -> Vec<UsageEvent> {
    let Ok(projects) = fs::read_dir(root) else {
        return Vec::new();
    };
    projects
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .flat_map(|dir| read_sessions(&dir.path(), since))
        .collect()
}

fn read_sessions(dir: &PathBuf, since: &Since) -> Vec<UsageEvent> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    // Claude session files are `<uuid>.jsonl` — no date in the filename. mtime is the only
    // cheap signal we have for "was this session touched in the window?", so stat each file
    // (one syscall, ~10µs) and skip the open+line-parse path when the file is stale.
    let cutoff = mtime_cutoff(since);
    entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .filter(|e| mtime_within_cutoff(&e.path(), cutoff))
        .flat_map(|e| read_session_file(&e.path(), since))
        .collect()
}

fn mtime_cutoff(since: &Since) -> Option<NaiveDate> {
    let today = Utc::now().date_naive();
    match since {
        Since::All => None,
        // One-day buffer covers sessions Claude touched late yesterday whose mtime races our
        // midnight comparison.
        Since::Today => Some(today - Duration::days(1)),
        Since::Days(n) => Some(today - Duration::days(*n)),
    }
}

fn mtime_within_cutoff(path: &Path, cutoff: Option<NaiveDate>) -> bool {
    let Some(cutoff) = cutoff else {
        return true;
    };
    let Ok(meta) = fs::metadata(path) else {
        return true;
    };
    let Ok(mtime) = meta.modified() else {
        return true;
    };
    let mtime_date = DateTime::<Utc>::from(mtime).date_naive();
    mtime_date >= cutoff
}

fn read_session_file(path: &PathBuf, since: &Since) -> Vec<UsageEvent> {
    let Ok(file) = fs::File::open(path) else {
        return Vec::new();
    };
    // Let `parse_event` do the type-filter — its internal `kind != "assistant"` check is the
    // authoritative one. A substring pre-filter here would silently drop valid records if Claude
    // Code's JSONL formatter ever switches to `"type": "assistant"` with whitespace.
    BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| parse_event(&line))
        .filter(|e| since.includes(e.timestamp))
        .collect()
}

#[derive(Deserialize)]
struct RawEvent {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    timestamp: String,
    #[serde(default)]
    cwd: String,
    #[serde(rename = "requestId", default)]
    request_id: String,
    #[serde(default)]
    message: Option<RawMessage>,
}

#[derive(Deserialize)]
struct RawMessage {
    #[serde(default)]
    id: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    usage: Option<RawUsage>,
}

#[derive(Deserialize)]
struct RawUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
    #[serde(default)]
    cache_creation: Option<RawCacheCreation>,
}

#[derive(Deserialize)]
struct RawCacheCreation {
    #[serde(default)]
    ephemeral_5m_input_tokens: u64,
    #[serde(default)]
    ephemeral_1h_input_tokens: u64,
}

fn parse_event(line: &str) -> Option<UsageEvent> {
    let raw: RawEvent = serde_json::from_str(line).ok()?;
    if raw.kind != "assistant" {
        return None;
    }
    let message = raw.message?;
    let usage = message.usage?;
    let timestamp = DateTime::parse_from_rfc3339(&raw.timestamp)
        .ok()?
        .with_timezone(&Utc);
    let (cache_write_5m, cache_write_1h) = match usage.cache_creation {
        Some(c) => (c.ephemeral_5m_input_tokens, c.ephemeral_1h_input_tokens),
        // Older JSONL sessions only carry the rollup count; treat it all as 5m write tier.
        None => (usage.cache_creation_input_tokens, 0),
    };
    Some(UsageEvent {
        timestamp,
        model: message.model,
        project: project_name_from_cwd(&raw.cwd),
        message_id: message.id,
        request_id: raw.request_id,
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cache_write_5m,
        cache_write_1h,
        cache_read: usage.cache_read_input_tokens,
    })
}

fn dedup(events: Vec<UsageEvent>) -> Vec<UsageEvent> {
    let mut by_key: HashMap<(String, String), UsageEvent> = HashMap::new();
    let mut passthrough: Vec<UsageEvent> = Vec::new();
    for e in events {
        // serde `#[default]` fills missing IDs with `""`. Collapsing every empty-ID record into
        // one dedup bucket would silently drop unrelated assistant turns — fall through to the
        // passthrough list instead so usage stays accurate even if a future schema omits an ID.
        if e.message_id.is_empty() || e.request_id.is_empty() {
            passthrough.push(e);
            continue;
        }
        let key = (e.message_id.clone(), e.request_id.clone());
        // ccusage gotcha: a `(message.id, requestId)` can repeat across files when a session
        // resumes. Keep the row with the higher total — that's the canonical final usage.
        by_key
            .entry(key)
            .and_modify(|prev| {
                if e.token_total() > prev.token_total() {
                    *prev = e.clone();
                }
            })
            .or_insert(e);
    }
    let mut out: Vec<UsageEvent> = by_key.into_values().collect();
    out.extend(passthrough);
    out
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

/// `claude-opus-4-7-20250929` → `opus-4-7`. Drops the family prefix and any trailing date so the
/// `model` group key reads as a model family rather than a release id.
fn short_model_name(model: &str) -> String {
    let lower = model.to_lowercase();
    let stripped = lower.strip_prefix("claude-").unwrap_or(&lower);
    stripped
        .rsplit_once('-')
        .map(|(head, tail)| {
            if tail.chars().all(|c| c.is_ascii_digit()) && tail.len() >= 6 {
                head.to_string()
            } else {
                stripped.to_string()
            }
        })
        .unwrap_or_else(|| stripped.to_string())
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
    if snap.rows.is_empty() && snap.total_tokens > 0 {
        // Window covers activity but the grouping yielded no rows — likely an obscure axis on a
        // tiny session. Keep the headline; don't add a misleading "no data" line.
    } else if snap.rows.is_empty() {
        lines.push("no recent Claude Code activity".into());
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
        Shape::Text => samples::text("Today: 3.4M tokens / $12.34"),
        Shape::TextBlock => samples::text_block(&[
            "Today: 3.4M tokens / $12.34",
            "splashboard  2.1M / $7.20",
            "playground   1.3M / $5.14",
        ]),
        Shape::MarkdownTextBlock => samples::markdown(
            "**Today: 3.4M tokens / $12.34**\n- **splashboard** — 2.1M / $7.20\n- **playground** — 1.3M / $5.14",
        ),
        Shape::Entries => samples::entries(&[
            ("splashboard", "2.1M / $7.20"),
            ("playground", "1.3M / $5.14"),
        ]),
        Shape::Bars => Body::Bars(BarsData {
            bars: vec![
                Bar {
                    label: "splashboard".into(),
                    value: 2_100_000,
                    value_label: Some("$7.20".into()),
                },
                Bar {
                    label: "playground".into(),
                    value: 1_300_000,
                    value_label: Some("$5.14".into()),
                },
            ],
        }),
        Shape::NumberSeries => {
            samples::number_series(&[0, 100_000, 250_000, 800_000, 600_000, 2_400_000, 3_400_000])
        }
        Shape::Badge => samples::badge(Status::Warn, "Today $12.34"),
        _ => return None,
    })
}

