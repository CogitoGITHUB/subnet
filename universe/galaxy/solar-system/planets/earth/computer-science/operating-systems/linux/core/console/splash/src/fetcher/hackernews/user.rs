//! `hackernews_user` — profile rollup for an HN account via `/user/{login}.json`. Same read,
//! three shapes:
//!
//! - `Text` — `"@login · Npt karma"` for hero / subtitle bands.
//! - `Entries` — login / karma / created / submissions count, key/value rows.
//! - `TextBlock` — bio block (login / about / created / karma) for sidebar use.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{Body, EntriesData, Entry, LinkedLine, LinkedTextBlockData, Payload};
use crate::render::Shape;
use crate::samples;

use super::client::{API_BASE, HN_USER_URL, get};

#[cfg(test)]
use crate::payload::{TextBlockData, TextData};

const SHAPES: &[Shape] = &[
    Shape::Entries,
    Shape::Text,
    Shape::LinkedTextBlock,
    Shape::TextBlock,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "user",
    type_hint: "string (HN login)",
    required: true,
    default: None,
    description: "Hacker News login to fetch (e.g. `\"pg\"`).",
}];

/// Profile rollup for a single Hacker News account.
pub struct HackernewsUserFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub user: Option<String>,
}

#[async_trait]
impl Fetcher for HackernewsUserFetcher {
    fn name(&self) -> &str {
        "hackernews_user"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Profile rollup for one Hacker News account — login, karma, join date, submission count, and bio. Use `hackernews_user_submissions` for that user's recent stories or `hackernews_user_comments` for their recent comments."
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
        let extra = ctx
            .options
            .as_ref()
            .and_then(|v| toml::to_string(v).ok())
            .unwrap_or_default();
        cache_key(self.name(), ctx, &extra)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Text => samples::text("@pg · 156000pt karma"),
            Shape::Entries => samples::entries(&[
                ("login", "pg"),
                ("karma", "156000"),
                ("created", "2006-10-09"),
                ("submissions", "1234"),
            ]),
            Shape::TextBlock => samples::text_block(&[
                "@pg",
                "156000pt karma",
                "joined 2006-10-09",
                "Paul Graham, Y Combinator co-founder.",
            ]),
            Shape::LinkedTextBlock => samples::linked_text_block(&[(
                "@pg · 156000pt karma",
                Some("https://news.ycombinator.com/user?id=pg"),
            )]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let login = opts
            .user
            .ok_or_else(|| FetchError::Failed("hackernews_user requires `user` option".into()))?;
        let info: UserInfo = get(&format!("{API_BASE}/user/{login}.json")).await?;
        Ok(payload(render_body(
            &info,
            ctx.shape.unwrap_or(Shape::Entries),
        )))
    }
}

#[derive(Debug, Deserialize)]
struct UserInfo {
    id: String,
    #[serde(default)]
    karma: u64,
    /// Unix seconds.
    #[serde(default)]
    created: i64,
    #[serde(default)]
    about: Option<String>,
    #[serde(default)]
    submitted: Vec<u64>,
}

fn render_body(info: &UserInfo, shape: Shape) -> Body {
    match shape {
        Shape::Text => crate::payload::Body::Text(crate::payload::TextData {
            value: format!("@{} · {}pt karma", info.id, info.karma),
        }),
        Shape::TextBlock => crate::payload::Body::TextBlock(crate::payload::TextBlockData {
            lines: text_block_lines(info),
        }),
        Shape::LinkedTextBlock => Body::LinkedTextBlock(LinkedTextBlockData {
            items: vec![LinkedLine {
                text: format!("@{} · {}pt karma", info.id, info.karma),
                url: Some(format!("{HN_USER_URL}{}", info.id)),
            }],
        }),
        _ => Body::Entries(EntriesData {
            items: entries(info),
        }),
    }
}

fn entries(info: &UserInfo) -> Vec<Entry> {
    let mut items = vec![
        kv("login", info.id.clone()),
        kv("karma", info.karma.to_string()),
        kv("created", iso_date(info.created)),
        kv("submissions", info.submitted.len().to_string()),
    ];
    if let Some(about) = info.about.as_deref().filter(|s| !s.is_empty()) {
        items.push(kv("about", strip_tags(about)));
    }
    items
}

fn text_block_lines(info: &UserInfo) -> Vec<String> {
    let mut lines = vec![
        format!("@{}", info.id),
        format!("{}pt karma", info.karma),
        format!("joined {}", iso_date(info.created)),
    ];
    if let Some(about) = info.about.as_deref().filter(|s| !s.is_empty()) {
        lines.push(strip_tags(about));
    }
    lines
}

fn kv(key: &str, value: String) -> Entry {
    Entry {
        key: key.into(),
        value: Some(value),
        status: None,
    }
}

fn iso_date(unix_seconds: i64) -> String {
    DateTime::<Utc>::from_timestamp(unix_seconds, 0)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// HN's `about` field embeds raw HTML (`<p>`, `<a>`, `&#x2F;`). Strip the markup conservatively
/// so the rollup stays text-only — replacing `<br>` / `<p>` with spaces and dropping the rest.
/// This isn't a full HTML parser; it's a "best-effort make it readable on a splash" pass.
fn strip_tags(raw: &str) -> String {
    let with_breaks = raw.replace("<p>", " ").replace("<br>", " ");
    let mut out = String::with_capacity(with_breaks.len());
    let mut in_tag = false;
    for ch in with_breaks.chars() {
        match (ch, in_tag) {
            ('<', _) => in_tag = true,
            ('>', _) => in_tag = false,
            (c, false) => out.push(c),
            _ => {}
        }
    }
    decode_entities(&out)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#x2F;", "/")
        .replace("&#39;", "'")
}

