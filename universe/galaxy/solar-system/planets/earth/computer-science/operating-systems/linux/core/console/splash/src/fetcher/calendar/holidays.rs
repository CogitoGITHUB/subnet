//! `calendar_holidays` — public-holiday data for a country, served as Calendar / Text /
//! TextBlock.
//!
//! Safety::Safe: host `date.nager.at` is hardcoded. The only config-driven input is the
//! ISO 3166-1 alpha-2 country code, validated locally — config can't redirect traffic.

use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{Datelike, NaiveDate, Utc};
use reqwest::Client;
use serde::Deserialize;

use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{Body, CalendarData, Payload, TextBlockData, TextData};
use crate::render::Shape;

const API_BASE: &str = "https://date.nager.at/api/v3/PublicHolidays";
const USER_AGENT: &str = concat!("splashboard/", env!("CARGO_PKG_VERSION"));
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_LIMIT: u32 = 5;
const MIN_LIMIT: u32 = 1;
const MAX_LIMIT: u32 = 30;

const SHAPES: &[Shape] = &[Shape::Calendar, Shape::Text, Shape::TextBlock];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "country",
        type_hint: "string (ISO 3166-1 alpha-2, e.g. \"JP\" / \"US\")",
        required: true,
        default: None,
        description: "Country whose public holidays to fetch.",
    },
    OptionSchema {
        name: "language",
        type_hint: "\"local\" | \"english\"",
        required: false,
        default: Some("\"local\""),
        description: "Display-name source. `local` uses the country's localized name; `english` uses the English label.",
    },
    OptionSchema {
        name: "limit",
        type_hint: "integer (1..=30)",
        required: false,
        default: Some("5"),
        description: "Maximum number of upcoming holidays to list (TextBlock shape only).",
    },
];

pub struct CalendarHolidaysFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    pub country: Option<String>,
    #[serde(default)]
    pub language: Option<Language>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    Local,
    English,
}

#[async_trait]
impl Fetcher for CalendarHolidaysFetcher {
    fn name(&self) -> &str {
        "calendar_holidays"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Public holidays for a country (ISO 3166-1 alpha-2) via date.nager.at. `Calendar` highlights this month's holiday dates, `Text` shows today's holiday name when one matches, and `TextBlock` lists the next few upcoming holidays."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 60 * 24
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
            Shape::Calendar => sample_calendar(),
            Shape::Text => Body::Text(TextData {
                value: "🎌 Showa Day".into(),
            }),
            Shape::TextBlock => Body::TextBlock(TextBlockData {
                lines: vec![
                    "04/29 — Showa Day".into(),
                    "05/03 — Constitution Day".into(),
                    "05/04 — Greenery Day".into(),
                    "05/05 — Children's Day".into(),
                ],
            }),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let country = require_country(opts.country.as_deref())?;
        let language = opts.language.unwrap_or_default();
        let limit = opts
            .limit
            .unwrap_or(DEFAULT_LIMIT)
            .clamp(MIN_LIMIT, MAX_LIMIT) as usize;
        let today = Utc::now().date_naive();
        let holidays = fetch_year(today.year(), &country).await?;
        let shape = ctx.shape.unwrap_or(Shape::Calendar);
        Ok(payload(render_body(
            &holidays, shape, today, language, limit,
        )))
    }
}

fn require_country(raw: Option<&str>) -> Result<String, FetchError> {
    let raw = raw.filter(|s| !s.is_empty()).ok_or_else(|| {
        FetchError::Failed("calendar_holidays requires `country` (ISO 3166-1 alpha-2)".into())
    })?;
    encode_country(raw).ok_or_else(|| {
        FetchError::Failed(format!(
            "invalid country code (need ISO 3166-1 alpha-2): {raw:?}"
        ))
    })
}

/// ISO 3166-1 alpha-2 sanitisation — strips non-letters, uppercases, requires exactly 2 chars.
/// Defense-in-depth even though the host is fixed: keeps the URL path segment trivial.
fn encode_country(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .collect::<String>()
        .to_ascii_uppercase();
    (cleaned.len() == 2).then_some(cleaned)
}

async fn fetch_year(year: i32, country: &str) -> Result<Vec<Holiday>, FetchError> {
    let url = format!("{API_BASE}/{year}/{country}");
    let res = http()
        .get(&url)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("calendar_holidays request failed: {e}")))?;
    let status = res.status();
    if !status.is_success() {
        return Err(FetchError::Failed(format!("calendar_holidays {status}")));
    }
    res.json()
        .await
        .map_err(|e| FetchError::Failed(format!("calendar_holidays json parse: {e}")))
}

#[derive(Debug, Clone, Deserialize)]
struct Holiday {
    date: String,
    #[serde(rename = "localName")]
    local_name: String,
    name: String,
}

impl Holiday {
    fn parse_date(&self) -> Option<NaiveDate> {
        NaiveDate::parse_from_str(&self.date, "%Y-%m-%d").ok()
    }
    fn label(&self, language: Language) -> &str {
        match language {
            Language::Local => &self.local_name,
            Language::English => &self.name,
        }
    }
}

fn render_body(
    holidays: &[Holiday],
    shape: Shape,
    today: NaiveDate,
    language: Language,
    limit: usize,
) -> Body {
    match shape {
        Shape::Text => Body::Text(TextData {
            value: today_label(holidays, today, language).unwrap_or_default(),
        }),
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: upcoming_lines(holidays, today, language, limit),
        }),
        _ => Body::Calendar(month_calendar(holidays, today)),
    }
}

fn today_label(holidays: &[Holiday], today: NaiveDate, language: Language) -> Option<String> {
    holidays
        .iter()
        .find(|h| h.parse_date() == Some(today))
        .map(|h| format!("🎌 {}", h.label(language)))
}

fn upcoming_lines(
    holidays: &[Holiday],
    today: NaiveDate,
    language: Language,
    limit: usize,
) -> Vec<String> {
    let mut dated: Vec<(NaiveDate, &Holiday)> = holidays
        .iter()
        .filter_map(|h| Some((h.parse_date()?, h)))
        .filter(|(d, _)| *d >= today)
        .collect();
    dated.sort_by_key(|(d, _)| *d);
    dated
        .into_iter()
        .take(limit)
        .map(|(d, h)| format!("{:02}/{:02} — {}", d.month(), d.day(), h.label(language)))
        .collect()
}

fn month_calendar(holidays: &[Holiday], today: NaiveDate) -> CalendarData {
    let events: Vec<u8> = holidays
        .iter()
        .filter_map(|h| h.parse_date())
        .filter(|d| d.year() == today.year() && d.month() == today.month())
        .map(|d| d.day() as u8)
        .collect();
    CalendarData {
        year: today.year(),
        month: today.month() as u8,
        day: Some(today.day() as u8),
        events,
    }
}

fn sample_calendar() -> Body {
    Body::Calendar(CalendarData {
        year: 2026,
        month: 4,
        day: Some(29),
        events: vec![29],
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

