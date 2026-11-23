//! `deariary_today` — the most recent auto-generated diary entry.
//!
//! deariary.com aggregates external tools (GitHub, Calendar, Slack, Linear, ...) into a
//! written entry each morning, covering roughly the previous 24 hours. This fetcher targets
//! whichever entry is freshest right now: usually today's, falling back to yesterday's when
//! today's hasn't been generated yet (e.g. early morning splash). Requires the deariary
//! Advanced plan and a Bearer token via `options.token` or `DEARIARY_TOKEN` env.
//!
//! Safety::Safe: host `api.deariary.com` is hardcoded.

use async_trait::async_trait;
use chrono::{Local, NaiveDate};
use serde::Deserialize;

use super::client::{
    ApiEntry, cache_extra, cached_get_entries, cached_get_entry, entry_url, resolve_token,
};
use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Body, EntriesData, Entry, LinkedLine, LinkedTextBlockData, MarkdownTextBlockData,
    Payload, Status, TextBlockData, TextData,
};
use crate::render::Shape;
use crate::samples;

const SHAPES: &[Shape] = &[
    Shape::TextBlock,
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
    description: "Deariary API token (Bearer). Falls back to the `DEARIARY_TOKEN` env var.",
}];

pub struct DeariaryToday;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    token: Option<String>,
}

#[async_trait]
impl Fetcher for DeariaryToday {
    fn name(&self) -> &str {
        "deariary_today"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "The most recent auto-generated deariary.com entry — typically today's, falling back to yesterday's when today's hasn't generated yet. Aggregated from external tools (GitHub, Calendar, Slack, Linear, ...); requires the Advanced plan. `TextBlock` shows the body, `MarkdownTextBlock` preserves formatting for `text_markdown`, `LinkedTextBlock` is a one-line clickable headline that opens the entry on deariary.com, `Entries` summarises title / date / tags / sources, `Text` is a one-line headline, `Badge` is a status pill."
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
        let opts: Options = parse_options(ctx.options.as_ref()).unwrap_or_default();
        let extra = cache_extra(opts.token.as_deref(), ctx.options.as_ref());
        cache_key(self.name(), ctx, &extra)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Text => samples::text("📔 My adventurous day"),
            Shape::TextBlock => samples::text_block(&[
                "Caught the morning train to the office and shipped the heatmap renderer.",
                "Reviewed two pull requests with the team and wrote release notes.",
                "Wrapped up with a calm dinner and a long walk along the river.",
            ]),
            Shape::MarkdownTextBlock => samples::markdown(
                "## My adventurous day\n\nCaught the morning train to the office and shipped the heatmap renderer.\n\n- 🐱 GitHub: 3 commits, 2 PRs reviewed\n- 📅 Calendar: 1:1 with manager",
            ),
            Shape::LinkedTextBlock => samples::linked_text_block(&[(
                "📔 My adventurous day",
                Some("https://app.deariary.com/entries/2026/04/27"),
            )]),
            Shape::Entries => samples::entries(&[
                ("title", "My adventurous day"),
                ("date", "2026-04-27"),
                ("tags", "work, travel"),
                ("sources", "github, calendar"),
            ]),
            Shape::Badge => Body::Badge(BadgeData {
                status: Status::Ok,
                label: "📔 today".into(),
            }),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let token = resolve_token(opts.token.as_deref())?;
        let today = Local::now().date_naive();
        let entry = latest_entry(&token, today).await?;
        let shape = ctx.shape.unwrap_or(Shape::TextBlock);
        Ok(payload(render_body(entry.as_ref(), shape, today)))
    }
}

/// Pulls the freshest entry, but only when it's recent enough to be honest about. The
/// fetcher's description promises "typically today's, falling back to yesterday's when
/// today's hasn't generated yet" — surfacing a 5-day-old entry as "today" would be a
/// silent lie. If the newest available entry isn't from today or yesterday, return
/// `Ok(None)` and let the empty-state placeholder render instead.
///
/// The list call is cached and shared with `deariary_recent`; the per-date lookup is
/// cached and shared with `deariary_on_this_day` — `/entries` list responses may strip
/// content for size, so we re-fetch the full body via the date endpoint.
async fn latest_entry(token: &str, today: NaiveDate) -> Result<Option<ApiEntry>, FetchError> {
    let entries = cached_get_entries(token, None).await?;
    let Some(latest) = entries.first() else {
        return Ok(None);
    };
    if !is_today_or_yesterday(&latest.date, today) {
        return Ok(None);
    }
    cached_get_entry(token, &latest.date).await
}

fn is_today_or_yesterday(raw: &str, today: NaiveDate) -> bool {
    let Ok(date) = NaiveDate::parse_from_str(raw, "%Y-%m-%d") else {
        return false;
    };
    let days_back = (today - date).num_days();
    (0..=1).contains(&days_back)
}

fn render_body(entry: Option<&ApiEntry>, shape: Shape, today: NaiveDate) -> Body {
    match entry {
        None => empty_body(shape),
        Some(e) => match shape {
            Shape::Text => render_text(e),
            Shape::MarkdownTextBlock => render_markdown(e),
            Shape::LinkedTextBlock => render_linked(e),
            Shape::Entries => render_entries(e),
            Shape::Badge => render_badge(e, today),
            _ => render_text_block(e),
        },
    }
}

fn render_text(e: &ApiEntry) -> Body {
    Body::Text(TextData {
        value: format!("📔 {}", e.title),
    })
}

fn render_text_block(e: &ApiEntry) -> Body {
    Body::TextBlock(TextBlockData {
        lines: content_lines(e),
    })
}

fn render_markdown(e: &ApiEntry) -> Body {
    Body::MarkdownTextBlock(MarkdownTextBlockData {
        value: e.content.clone().unwrap_or_default(),
    })
}

/// Single-row clickable headline opening the entry on deariary.com. Multi-line content is
/// the job of `TextBlock` / `MarkdownTextBlock`; here the user just wants one line they can
/// click to keep reading.
fn render_linked(e: &ApiEntry) -> Body {
    Body::LinkedTextBlock(LinkedTextBlockData {
        items: vec![LinkedLine {
            text: format!("📔 {}", e.title),
            url: Some(entry_url(&e.date)),
        }],
    })
}

fn render_entries(e: &ApiEntry) -> Body {
    let items = [
        ("title", Some(e.title.clone())),
        ("date", Some(e.date.clone())),
        ("tags", (!e.tags.is_empty()).then(|| e.tags.join(", "))),
        (
            "sources",
            (!e.sources.is_empty()).then(|| e.sources.join(", ")),
        ),
    ]
    .into_iter()
    .filter_map(|(key, value)| {
        value.map(|v| Entry {
            key: key.into(),
            value: Some(v),
            status: None,
        })
    })
    .collect();
    Body::Entries(EntriesData { items })
}

fn render_badge(e: &ApiEntry, today: NaiveDate) -> Body {
    Body::Badge(BadgeData {
        status: Status::Ok,
        label: format!("📔 {}", relative_day(&e.date, today)),
    })
}

/// `today` / `yesterday` / falls back to the raw ISO date for older entries — keeps the badge
/// label honest when today's hasn't generated yet and the fetcher returns yesterday's instead.
fn relative_day(raw: &str, today: NaiveDate) -> String {
    let Ok(date) = NaiveDate::parse_from_str(raw, "%Y-%m-%d") else {
        return raw.to_string();
    };
    match (today - date).num_days() {
        0 => "today".into(),
        1 => "yesterday".into(),
        _ => raw.to_string(),
    }
}

fn content_lines(e: &ApiEntry) -> Vec<String> {
    e.content
        .as_deref()
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

fn empty_body(shape: Shape) -> Body {
    match shape {
        Shape::Text => Body::Text(TextData {
            value: String::new(),
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: String::new(),
        }),
        Shape::LinkedTextBlock => Body::LinkedTextBlock(LinkedTextBlockData { items: vec![] }),
        Shape::Entries => Body::Entries(EntriesData { items: vec![] }),
        Shape::Badge => Body::Badge(BadgeData {
            status: Status::Warn,
            label: "📔 no entry yet".into(),
        }),
        _ => Body::TextBlock(TextBlockData { lines: vec![] }),
    }
}

