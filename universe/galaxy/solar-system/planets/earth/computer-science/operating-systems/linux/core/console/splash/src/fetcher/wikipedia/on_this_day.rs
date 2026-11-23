//! `wikipedia_on_this_day` — historical events that occurred on today's calendar date,
//! linked to their Wikipedia articles.
//!
//! Safety::Safe: host is `*.wikipedia.org`. The `lang` option picks a language subdomain;
//! it can't redirect traffic outside Wikipedia.

use async_trait::async_trait;
use chrono::{Datelike, NaiveDate, Utc};
use serde::Deserialize;

use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{
    Body, LinkedLine, LinkedTextBlockData, Payload, TextBlockData, TextData, TimelineData,
    TimelineEvent,
};
use crate::render::Shape;
use crate::samples;

use super::client::{DEFAULT_LANG, PageSummary, get, rest_api_base};

const DEFAULT_LIMIT: u32 = 5;
const MIN_LIMIT: u32 = 1;
const MAX_LIMIT: u32 = 30;

const SHAPES: &[Shape] = &[
    Shape::LinkedTextBlock,
    Shape::TextBlock,
    Shape::Text,
    Shape::Timeline,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "kind",
        type_hint: "\"selected\" | \"events\" | \"births\" | \"deaths\" | \"holidays\" | \"all\"",
        required: false,
        default: Some("\"selected\""),
        description: "Which slice of the on-this-day feed to fetch. `selected` is the curated highlights.",
    },
    OptionSchema {
        name: "limit",
        type_hint: "integer (1..=30)",
        required: false,
        default: Some("5"),
        description: "Maximum number of events to display.",
    },
    OptionSchema {
        name: "lang",
        type_hint: "string (Wikipedia language code, e.g. `\"en\"` / `\"ja\"`)",
        required: false,
        default: Some("\"en\""),
        description: "Wikipedia language edition to query.",
    },
];

pub struct WikipediaOnThisDayFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub kind: Option<Kind>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub lang: Option<String>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Selected,
    Events,
    Births,
    Deaths,
    Holidays,
    All,
}

impl Kind {
    fn endpoint(self) -> &'static str {
        match self {
            Self::Selected => "selected",
            Self::Events => "events",
            Self::Births => "births",
            Self::Deaths => "deaths",
            Self::Holidays => "holidays",
            Self::All => "all",
        }
    }
}

#[async_trait]
impl Fetcher for WikipediaOnThisDayFetcher {
    fn name(&self) -> &str {
        "wikipedia_on_this_day"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Events that happened on today's calendar date in history, sourced from Wikipedia's on-this-day feed (selected highlights, events, births, deaths, holidays, or all). Use `wikipedia_featured` for today's featured article or `wikipedia_random` for an arbitrary page."
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
            Shape::LinkedTextBlock => samples::linked_text_block(&[
                (
                    "1969 — Apollo 11 lands on the Moon",
                    Some("https://en.wikipedia.org/wiki/Apollo_11"),
                ),
                (
                    "1492 — Columbus departs Palos de la Frontera",
                    Some("https://en.wikipedia.org/wiki/Christopher_Columbus"),
                ),
                (
                    "1989 — Tim Berners-Lee proposes the World Wide Web",
                    Some("https://en.wikipedia.org/wiki/World_Wide_Web"),
                ),
            ]),
            Shape::TextBlock => samples::text_block(&[
                "1969 — Apollo 11 lands on the Moon",
                "1492 — Columbus departs Palos de la Frontera",
                "1989 — Tim Berners-Lee proposes the World Wide Web",
            ]),
            Shape::Text => samples::text("1969 — Apollo 11 lands on the Moon"),
            Shape::Timeline => samples::timeline(&[
                (
                    -86_400 * 365 * 2, // doesn't matter much; demo only
                    "1969 — Apollo 11 lands on the Moon",
                    None,
                ),
                (
                    -86_400 * 365 * 4,
                    "1492 — Columbus departs Palos de la Frontera",
                    None,
                ),
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let kind = opts.kind.unwrap_or_default();
        let lang = opts.lang.as_deref().unwrap_or(DEFAULT_LANG);
        let limit = opts
            .limit
            .unwrap_or(DEFAULT_LIMIT)
            .clamp(MIN_LIMIT, MAX_LIMIT) as usize;
        let events = fetch_events(lang, kind).await?;
        let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);
        Ok(payload(render_body(&events, shape, limit)))
    }
}

async fn fetch_events(lang: &str, kind: Kind) -> Result<Vec<EventEntry>, FetchError> {
    let now = Utc::now();
    let (month, day) = (now.month(), now.day());
    let url = format!(
        "{}/feed/onthisday/{}/{:02}/{:02}",
        rest_api_base(lang),
        kind.endpoint(),
        month,
        day
    );
    let response: OnThisDayResponse = get(&url).await?;
    Ok(response.into_events(kind, now.year(), month, day))
}

#[derive(Debug, Default, Deserialize)]
struct OnThisDayResponse {
    #[serde(default)]
    selected: Vec<RawEvent>,
    #[serde(default)]
    events: Vec<RawEvent>,
    #[serde(default)]
    births: Vec<RawEvent>,
    #[serde(default)]
    deaths: Vec<RawEvent>,
    #[serde(default)]
    holidays: Vec<RawEvent>,
}

impl OnThisDayResponse {
    fn into_events(self, kind: Kind, today_year: i32, month: u32, day: u32) -> Vec<EventEntry> {
        let raw = match kind {
            Kind::Selected => self.selected,
            Kind::Events => self.events,
            Kind::Births => self.births,
            Kind::Deaths => self.deaths,
            Kind::Holidays => self.holidays,
            Kind::All => {
                let mut all = self.selected;
                all.extend(self.events);
                all.extend(self.births);
                all.extend(self.deaths);
                all.extend(self.holidays);
                all
            }
        };
        raw.into_iter()
            .map(|r| EventEntry::from_raw(r, today_year, month, day))
            .collect()
    }
}

#[derive(Debug, Deserialize)]
struct RawEvent {
    #[serde(default)]
    text: String,
    #[serde(default)]
    year: Option<i32>,
    #[serde(default)]
    pages: Vec<PageSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EventEntry {
    label: String,
    url: Option<String>,
    timestamp: i64,
}

impl EventEntry {
    fn from_raw(raw: RawEvent, today_year: i32, month: u32, day: u32) -> Self {
        let label = match raw.year {
            Some(year) => format!("{year} — {}", raw.text),
            None => raw.text,
        };
        let url = raw.pages.first().and_then(|p| p.url());
        let year = raw.year.unwrap_or(today_year);
        Self {
            label,
            url,
            timestamp: anniversary_timestamp(year, month, day),
        }
    }
}

fn anniversary_timestamp(year: i32, month: u32, day: u32) -> i64 {
    let date = NaiveDate::from_ymd_opt(year, month, day)
        .or_else(|| NaiveDate::from_ymd_opt(year, month, 1))
        .or_else(|| NaiveDate::from_ymd_opt(year, 1, 1));
    date.and_then(|d| d.and_hms_opt(12, 0, 0))
        .map(|dt| dt.and_utc().timestamp())
        .unwrap_or(0)
}

fn render_body(events: &[EventEntry], shape: Shape, limit: usize) -> Body {
    match shape {
        Shape::Text => Body::Text(TextData {
            value: events.first().map(|e| e.label.clone()).unwrap_or_default(),
        }),
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: events.iter().take(limit).map(|e| e.label.clone()).collect(),
        }),
        Shape::Timeline => Body::Timeline(TimelineData {
            events: events
                .iter()
                .take(limit)
                .map(|e| TimelineEvent {
                    timestamp: e.timestamp,
                    title: e.label.clone(),
                    detail: None,
                    status: None,
                })
                .collect(),
        }),
        _ => Body::LinkedTextBlock(LinkedTextBlockData {
            items: events
                .iter()
                .take(limit)
                .map(|e| LinkedLine {
                    text: e.label.clone(),
                    url: e.url.clone(),
                })
                .collect(),
        }),
    }
}

