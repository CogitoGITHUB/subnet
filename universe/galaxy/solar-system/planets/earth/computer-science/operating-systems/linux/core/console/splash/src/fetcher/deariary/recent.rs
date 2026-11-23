//! `deariary_recent` — most recent auto-generated diary entries.
//!
//! Mirrors deariary.com's reverse-chronological list. The same data feeds Timeline (default),
//! a list-of-titles view (TextBlock / Entries / LinkedTextBlock / MarkdownTextBlock), a
//! coverage view (Calendar — which days this month received an entry), and Text / Badge
//! headlines.
//!
//! Safety::Safe: host `api.deariary.com` is hardcoded.

use async_trait::async_trait;
use chrono::{Datelike, Local, NaiveDate};
use serde::Deserialize;

use super::client::{
    ApiEntry, MAX_LIST_LIMIT, cache_extra, cached_get_entries, entry_url, resolve_token,
};
use crate::fetcher::github::common::{cache_key, parse_options, parse_timestamp, payload};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Body, CalendarData, EntriesData, Entry, LinkedLine, LinkedTextBlockData,
    MarkdownTextBlockData, Payload, Status, TextBlockData, TextData, TimelineData, TimelineEvent,
};
use crate::render::Shape;
use crate::samples;

const DEFAULT_LIMIT: u32 = 10;
const MIN_LIMIT: u32 = 1;
const SHAPES: &[Shape] = &[
    Shape::Timeline,
    Shape::Text,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::LinkedTextBlock,
    Shape::Entries,
    Shape::Badge,
    Shape::Calendar,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "token",
        type_hint: "string",
        required: false,
        default: None,
        description: "Deariary API token. Falls back to the `DEARIARY_TOKEN` env var.",
    },
    OptionSchema {
        name: "limit",
        type_hint: "integer (1..=100)",
        required: false,
        default: Some("10"),
        description: "Maximum number of entries to render in row-shaped views.",
    },
    OptionSchema {
        name: "tag",
        type_hint: "string",
        required: false,
        default: None,
        description: "Restrict results to a single tag slug.",
    },
];

pub struct DeariaryRecent;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    tag: Option<String>,
}

#[async_trait]
impl Fetcher for DeariaryRecent {
    fn name(&self) -> &str {
        "deariary_recent"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Recent auto-generated deariary.com entries, newest first. `Timeline` (default) carries the chronological ranking; `TextBlock` / `Entries` / `MarkdownTextBlock` / `LinkedTextBlock` carry the same ranking in their respective row formats; `Calendar` highlights this month's entry days; `Text` and `Badge` summarise the headline."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 10
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
            Shape::Timeline => samples::timeline(&[
                (1_777_478_400, "My adventurous day", Some("482w")),
                (1_777_392_000, "A quiet Sunday", Some("310w")),
                (1_777_305_600, "Shipped the heatmap renderer", Some("612w")),
            ]),
            Shape::Text => samples::text("📔 3 entries · latest: My adventurous day"),
            Shape::TextBlock => samples::text_block(&[
                "04/27 · My adventurous day",
                "04/26 · A quiet Sunday",
                "04/25 · Shipped the heatmap renderer",
            ]),
            Shape::MarkdownTextBlock => samples::markdown(
                "- **04/27** — My adventurous day\n- **04/26** — A quiet Sunday\n- **04/25** — Shipped the heatmap renderer",
            ),
            Shape::LinkedTextBlock => samples::linked_text_block(&[
                (
                    "04/27 · My adventurous day",
                    Some("https://app.deariary.com/entries/2026/04/27"),
                ),
                (
                    "04/26 · A quiet Sunday",
                    Some("https://app.deariary.com/entries/2026/04/26"),
                ),
                (
                    "04/25 · Shipped the heatmap renderer",
                    Some("https://app.deariary.com/entries/2026/04/25"),
                ),
            ]),
            Shape::Entries => samples::entries(&[
                ("2026-04-27", "My adventurous day"),
                ("2026-04-26", "A quiet Sunday"),
                ("2026-04-25", "Shipped the heatmap renderer"),
            ]),
            Shape::Badge => Body::Badge(BadgeData {
                status: Status::Ok,
                label: "📔 3 recent".into(),
            }),
            Shape::Calendar => samples::calendar(2026, 4, Some(27), &[25, 26, 27]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let token = resolve_token(opts.token.as_deref())?;
        let limit = opts
            .limit
            .unwrap_or(DEFAULT_LIMIT)
            .clamp(MIN_LIMIT, MAX_LIST_LIMIT);
        let entries = cached_get_entries(&token, opts.tag.as_deref()).await?;
        let shape = ctx.shape.unwrap_or(Shape::Timeline);
        let today = Local::now().date_naive();
        Ok(payload(render_body(&entries, shape, limit as usize, today)))
    }
}

fn render_body(entries: &[ApiEntry], shape: Shape, limit: usize, today: NaiveDate) -> Body {
    let limited: Vec<&ApiEntry> = entries.iter().take(limit).collect();
    match shape {
        // Headline shapes (Text/Badge) report the real total, not the truncated count —
        // a `limit = 5` widget with 23 cached entries should still say "📔 23 recent",
        // not "📔 5 recent". The user-set `limit` only governs how many ROWS render.
        Shape::Text => render_text(entries),
        Shape::Badge => render_badge(entries),
        Shape::TextBlock => render_text_block(&limited),
        Shape::MarkdownTextBlock => render_markdown(&limited),
        Shape::LinkedTextBlock => render_linked(&limited),
        Shape::Entries => render_entries(&limited),
        Shape::Calendar => render_calendar(entries, today),
        _ => render_timeline(&limited),
    }
}

fn render_text(entries: &[ApiEntry]) -> Body {
    let value = entries.first().map_or(String::new(), |e| {
        format!("📔 {} entries · latest: {}", entries.len(), e.title)
    });
    Body::Text(TextData { value })
}

fn render_text_block(entries: &[&ApiEntry]) -> Body {
    Body::TextBlock(TextBlockData {
        lines: entries.iter().map(|e| line_for(e)).collect(),
    })
}

fn render_markdown(entries: &[&ApiEntry]) -> Body {
    let value = entries
        .iter()
        .map(|e| format!("- **{}** — {}", short_date(&e.date), e.title))
        .collect::<Vec<_>>()
        .join("\n");
    Body::MarkdownTextBlock(MarkdownTextBlockData { value })
}

fn render_linked(entries: &[&ApiEntry]) -> Body {
    Body::LinkedTextBlock(LinkedTextBlockData {
        items: entries
            .iter()
            .map(|e| LinkedLine {
                text: line_for(e),
                url: Some(entry_url(&e.date)),
            })
            .collect(),
    })
}

fn render_entries(entries: &[&ApiEntry]) -> Body {
    Body::Entries(EntriesData {
        items: entries
            .iter()
            .map(|e| Entry {
                key: e.date.clone(),
                value: Some(e.title.clone()),
                status: None,
            })
            .collect(),
    })
}

fn render_badge(entries: &[ApiEntry]) -> Body {
    Body::Badge(BadgeData {
        status: if entries.is_empty() {
            Status::Warn
        } else {
            Status::Ok
        },
        label: format!("📔 {} recent", entries.len()),
    })
}

fn render_timeline(entries: &[&ApiEntry]) -> Body {
    Body::Timeline(TimelineData {
        events: entries
            .iter()
            .map(|e| TimelineEvent {
                timestamp: timestamp_for(e),
                title: e.title.clone(),
                detail: None,
                status: None,
            })
            .collect(),
    })
}

/// This month's grid with the days that received an entry marked as events. Useful for
/// "did I write today / how many days this month did I write" at a glance.
fn render_calendar(entries: &[ApiEntry], today: NaiveDate) -> Body {
    let events = entries
        .iter()
        .filter_map(|e| NaiveDate::parse_from_str(&e.date, "%Y-%m-%d").ok())
        .filter(|d| d.year() == today.year() && d.month() == today.month())
        .map(|d| d.day() as u8)
        .collect();
    Body::Calendar(CalendarData {
        year: today.year(),
        month: today.month() as u8,
        day: Some(today.day() as u8),
        events,
    })
}

fn line_for(e: &ApiEntry) -> String {
    format!("{} · {}", short_date(&e.date), e.title)
}

fn short_date(raw: &str) -> String {
    NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .map(|d| d.format("%m/%d").to_string())
        .unwrap_or_else(|_| raw.to_string())
}

fn timestamp_for(e: &ApiEntry) -> i64 {
    e.generated_at
        .as_deref()
        .map(parse_timestamp)
        .filter(|ts| *ts > 0)
        .or_else(|| {
            NaiveDate::parse_from_str(&e.date, "%Y-%m-%d")
                .ok()
                .and_then(|d| d.and_hms_opt(12, 0, 0))
                .map(|dt| dt.and_utc().timestamp())
        })
        .unwrap_or(0)
}

