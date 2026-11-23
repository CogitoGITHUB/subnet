//! `deariary_on_this_day` — past diary entries on this same calendar day.
//!
//! Fetches eight anchors in parallel (1m / 3m / 6m / 1y / 2y / 3y / 4y / 5y ago) and surfaces
//! whichever returned content. Splash-native take on deariary.com's opt-in Time Jump feature:
//! the app only shows past entries when the user opens that view; this widget makes it ambient
//! on every shell startup.
//!
//! Safety::Safe: host `api.deariary.com` is hardcoded.

use async_trait::async_trait;
use chrono::{Local, Months, NaiveDate};
use serde::Deserialize;
use tokio::task::JoinSet;

use super::client::{ApiEntry, cache_extra, cached_get_entry, entry_url, resolve_token};
use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Body, EntriesData, Entry, LinkedLine, LinkedTextBlockData, MarkdownTextBlockData,
    Payload, Status, TextBlockData, TextData, TimelineData, TimelineEvent,
};
use crate::render::Shape;
use crate::samples;

/// Anchors are stored newest-first; rendered output preserves that order so users scan from
/// the most recent past at the top down to the most distant (5 years) at the bottom.
const ANCHORS: &[(&str, u32)] = &[
    ("1 month ago", 1),
    ("3 months ago", 3),
    ("6 months ago", 6),
    ("1 year ago", 12),
    ("2 years ago", 24),
    ("3 years ago", 36),
    ("4 years ago", 48),
    ("5 years ago", 60),
];

const SHAPES: &[Shape] = &[
    Shape::TextBlock,
    Shape::Timeline,
    Shape::Text,
    Shape::MarkdownTextBlock,
    Shape::LinkedTextBlock,
    Shape::Entries,
    Shape::Badge,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "token",
    type_hint: "string",
    required: false,
    default: None,
    description: "Deariary API token. Falls back to the `DEARIARY_TOKEN` env var.",
}];

pub struct DeariaryOnThisDay;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    token: Option<String>,
}

#[async_trait]
impl Fetcher for DeariaryOnThisDay {
    fn name(&self) -> &str {
        "deariary_on_this_day"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Past auto-generated deariary.com entries from the same calendar day, fetched in parallel for 1m / 3m / 6m / 1y / 2y / 3y / 4y / 5y ago. Anchors with no entry are silently skipped. `TextBlock` is the default; `Timeline` plots the surviving anchors chronologically; `LinkedTextBlock` is a list of clickable rows; `Text` and `Badge` headline the most distant hit."
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
        let opts: Options = parse_options(ctx.options.as_ref()).unwrap_or_default();
        let extra = cache_extra(opts.token.as_deref(), ctx.options.as_ref());
        cache_key(self.name(), ctx, &extra)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::TextBlock => samples::text_block(&[
                "1 month ago — Reviewing PRs and shipping the heatmap renderer",
                "1 year ago — A quiet writing day",
                "3 years ago — First commit on splashboard",
            ]),
            Shape::Timeline => samples::timeline(&[
                (
                    1_775_000_000,
                    "Reviewing PRs and shipping the heatmap renderer",
                    Some("1 month ago"),
                ),
                (1_745_000_000, "A quiet writing day", Some("1 year ago")),
                (
                    1_682_000_000,
                    "First commit on splashboard",
                    Some("3 years ago"),
                ),
            ]),
            Shape::Text => samples::text("📔 3 years ago: First commit on splashboard"),
            Shape::MarkdownTextBlock => samples::markdown(
                "- **1 month ago** — Reviewing PRs and shipping the heatmap renderer\n- **1 year ago** — A quiet writing day\n- **3 years ago** — First commit on splashboard",
            ),
            Shape::LinkedTextBlock => samples::linked_text_block(&[
                (
                    "1 month ago — Reviewing PRs",
                    Some("https://app.deariary.com/entries/2026/03/27"),
                ),
                (
                    "1 year ago — A quiet writing day",
                    Some("https://app.deariary.com/entries/2025/04/27"),
                ),
                (
                    "3 years ago — First commit",
                    Some("https://app.deariary.com/entries/2023/04/27"),
                ),
            ]),
            Shape::Entries => samples::entries(&[
                ("1 month ago", "Reviewing PRs"),
                ("1 year ago", "A quiet writing day"),
                ("3 years ago", "First commit on splashboard"),
            ]),
            Shape::Badge => Body::Badge(BadgeData {
                status: Status::Ok,
                label: "📔 3 years ago".into(),
            }),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let token = resolve_token(opts.token.as_deref())?;
        let today = Local::now().date_naive();
        let hits = fetch_anchors(&token, today).await?;
        let shape = ctx.shape.unwrap_or(Shape::TextBlock);
        Ok(payload(render_body(&hits, shape, today)))
    }
}

/// Spawns one task per anchor and aggregates the results. Per-anchor 404 is "no entry that
/// day" and is silently skipped; 429 / 5xx propagate as errors only when *every* anchor failed
/// — otherwise a single rate-limited anchor would mask all the entries that did come back.
async fn fetch_anchors(
    token: &str,
    today: NaiveDate,
) -> Result<Vec<(usize, ApiEntry)>, FetchError> {
    let mut set: JoinSet<(usize, Result<Option<ApiEntry>, FetchError>)> = JoinSet::new();
    let mut spawned = 0usize;
    for (idx, (_, months)) in ANCHORS.iter().enumerate() {
        let Some(date) = anchor_date(today, *months) else {
            continue;
        };
        let token = token.to_string();
        spawned += 1;
        set.spawn(async move {
            let date_str = date.to_string();
            (idx, cached_get_entry(&token, &date_str).await)
        });
    }
    let mut hits: Vec<(usize, ApiEntry)> = Vec::new();
    let mut last_error: Option<FetchError> = None;
    let mut error_count = 0usize;
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok((idx, Ok(Some(entry)))) => hits.push((idx, entry)),
            Ok((_, Ok(None))) => {}
            Ok((_, Err(err))) => {
                error_count += 1;
                last_error = Some(err);
            }
            // Spawned task panicked or was cancelled. Count it like a fetch error so the
            // "all anchors failed" branch below can surface a real error rather than an
            // empty-but-Ok result that would silently render as "no entries".
            Err(join_err) => {
                error_count += 1;
                last_error = Some(FetchError::Failed(format!(
                    "deariary anchor task failed: {join_err}"
                )));
            }
        }
    }
    if hits.is_empty() && error_count == spawned && spawned > 0 {
        return Err(last_error.expect("at least one error when all anchors failed"));
    }
    hits.sort_by_key(|(idx, _)| *idx);
    Ok(hits)
}

fn anchor_date(today: NaiveDate, months: u32) -> Option<NaiveDate> {
    today.checked_sub_months(Months::new(months))
}

fn render_body(hits: &[(usize, ApiEntry)], shape: Shape, today: NaiveDate) -> Body {
    match shape {
        Shape::Text => render_text(hits),
        Shape::MarkdownTextBlock => render_markdown(hits),
        Shape::LinkedTextBlock => render_linked(hits),
        Shape::Entries => render_entries(hits),
        Shape::Badge => render_badge(hits),
        Shape::Timeline => render_timeline(hits, today),
        _ => render_text_block(hits),
    }
}

fn render_text(hits: &[(usize, ApiEntry)]) -> Body {
    let value = hits.last().map_or(String::new(), |(idx, e)| {
        format!("📔 {}: {}", ANCHORS[*idx].0, e.title)
    });
    Body::Text(TextData { value })
}

fn render_text_block(hits: &[(usize, ApiEntry)]) -> Body {
    Body::TextBlock(TextBlockData {
        lines: hits
            .iter()
            .map(|(idx, e)| format!("{} — {}", ANCHORS[*idx].0, e.title))
            .collect(),
    })
}

fn render_markdown(hits: &[(usize, ApiEntry)]) -> Body {
    let value = hits
        .iter()
        .map(|(idx, e)| format!("- **{}** — {}", ANCHORS[*idx].0, e.title))
        .collect::<Vec<_>>()
        .join("\n");
    Body::MarkdownTextBlock(MarkdownTextBlockData { value })
}

fn render_linked(hits: &[(usize, ApiEntry)]) -> Body {
    Body::LinkedTextBlock(LinkedTextBlockData {
        items: hits
            .iter()
            .map(|(idx, e)| LinkedLine {
                text: format!("{} — {}", ANCHORS[*idx].0, e.title),
                url: Some(entry_url(&e.date)),
            })
            .collect(),
    })
}

fn render_entries(hits: &[(usize, ApiEntry)]) -> Body {
    Body::Entries(EntriesData {
        items: hits
            .iter()
            .map(|(idx, e)| Entry {
                key: ANCHORS[*idx].0.into(),
                value: Some(e.title.clone()),
                status: None,
            })
            .collect(),
    })
}

fn render_badge(hits: &[(usize, ApiEntry)]) -> Body {
    let (status, label) = match hits.last() {
        Some((idx, _)) => (Status::Ok, format!("📔 {}", ANCHORS[*idx].0)),
        None => (Status::Warn, "📔 no past entries".into()),
    };
    Body::Badge(BadgeData { status, label })
}

fn render_timeline(hits: &[(usize, ApiEntry)], today: NaiveDate) -> Body {
    Body::Timeline(TimelineData {
        events: hits
            .iter()
            .filter_map(|(idx, e)| {
                let (label, months) = ANCHORS[*idx];
                let date = anchor_date(today, months)?;
                let timestamp = date.and_hms_opt(12, 0, 0)?.and_utc().timestamp();
                Some(TimelineEvent {
                    timestamp,
                    title: e.title.clone(),
                    detail: Some(label.into()),
                    status: None,
                })
            })
            .collect(),
    })
}

