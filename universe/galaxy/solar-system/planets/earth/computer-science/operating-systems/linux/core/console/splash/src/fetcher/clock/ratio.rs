//! `clock_ratio` — fraction of a named period (day / year / month / week / quarter / hour)
//! elapsed so far. Emits `Shape::Ratio` for gauge-like renderers.

use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, Timelike};
use serde::Deserialize;

use crate::fetcher::{FetchContext, RealtimeFetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{Body, Payload, RatioData};
use crate::render::Shape;
use crate::samples;

use super::common;

const SHAPES: &[Shape] = &[Shape::Ratio];
const SECS_PER_DAY: f64 = 86_400.0;

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "timezone",
        type_hint: "IANA timezone (e.g. \"Asia/Tokyo\")",
        required: false,
        default: Some("system local"),
        description: "Timezone the ratio is computed in. Omit to follow the system clock.",
    },
    OptionSchema {
        name: "period",
        type_hint: "\"day\" | \"year\" | \"month\" | \"week\" | \"quarter\" | \"hour\"",
        required: false,
        default: Some("\"day\""),
        description: "Named period whose elapsed fraction becomes the ratio.",
    },
];

pub struct ClockRatioFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub period: Option<Period>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Period {
    #[default]
    Day,
    Year,
    Month,
    Week,
    Quarter,
    Hour,
}

impl RealtimeFetcher for ClockRatioFetcher {
    fn name(&self) -> &str {
        "clock_ratio"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Fraction of the current period elapsed so far, as a `0..=1` value for gauge and progress-bar renderers. `period` selects which period (day, hour, week, month, quarter, year)."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        match shape {
            Shape::Ratio => Some(samples::ratio(0.45, "day")),
            _ => None,
        }
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let opts: Options = match common::parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return common::placeholder(&msg),
        };
        let now = common::now_in(opts.timezone.as_deref());
        let period = opts.period.unwrap_or(Period::Day);
        let (value, label) = fraction(&now, period);
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Ratio(RatioData {
                value: value.clamp(0.0, 1.0),
                label: Some(label),
                denominator: Some(total_units(&now, period)),
            }),
        }
    }
}

/// Total "slots" in the current period at a human-readable granularity: days for
/// year/month/quarter/week, hours for day, minutes for hour. Seconds-level granularity
/// was rejected — `43200 of 86400` reads as noise; `12 of 24` / `30 of 60` is what
/// people actually want to see beside a progress bar.
fn total_units(now: &DateTime<FixedOffset>, period: Period) -> u64 {
    match period {
        Period::Day => 24,
        Period::Hour => 60,
        Period::Week => 7,
        Period::Month => {
            let start = NaiveDate::from_ymd_opt(now.year(), now.month(), 1).unwrap();
            let end = next_month_start(now.year(), now.month());
            (end - start).num_days() as u64
        }
        Period::Quarter => {
            let first_month = ((now.month() - 1) / 3) * 3 + 1;
            let start = NaiveDate::from_ymd_opt(now.year(), first_month, 1).unwrap();
            let end = next_month_start(now.year(), first_month + 2);
            (end - start).num_days() as u64
        }
        Period::Year => {
            let start = NaiveDate::from_ymd_opt(now.year(), 1, 1).unwrap();
            let end = NaiveDate::from_ymd_opt(now.year() + 1, 1, 1).unwrap();
            (end - start).num_days() as u64
        }
    }
}

fn fraction(now: &DateTime<FixedOffset>, period: Period) -> (f64, String) {
    match period {
        Period::Day => (day(now), "day".into()),
        Period::Hour => (hour(now), "hour".into()),
        Period::Week => (week(now), "week".into()),
        Period::Month => (month(now), "month".into()),
        Period::Quarter => (quarter(now), "quarter".into()),
        Period::Year => (year(now), "year".into()),
    }
}

fn day(now: &DateTime<FixedOffset>) -> f64 {
    f64::from(now.num_seconds_from_midnight()) / SECS_PER_DAY
}

fn hour(now: &DateTime<FixedOffset>) -> f64 {
    f64::from(now.minute() * 60 + now.second()) / 3600.0
}

fn week(now: &DateTime<FixedOffset>) -> f64 {
    let wd = now.weekday().num_days_from_monday();
    (f64::from(wd) * SECS_PER_DAY + f64::from(now.num_seconds_from_midnight()))
        / (7.0 * SECS_PER_DAY)
}

fn month(now: &DateTime<FixedOffset>) -> f64 {
    let start = NaiveDate::from_ymd_opt(now.year(), now.month(), 1).unwrap();
    let end = next_month_start(now.year(), now.month());
    let total_days = (end - start).num_days() as f64;
    let elapsed_days = (now.day() - 1) as f64;
    (elapsed_days * SECS_PER_DAY + f64::from(now.num_seconds_from_midnight()))
        / (total_days * SECS_PER_DAY)
}

fn quarter(now: &DateTime<FixedOffset>) -> f64 {
    let first_month = ((now.month() - 1) / 3) * 3 + 1;
    let start = NaiveDate::from_ymd_opt(now.year(), first_month, 1).unwrap();
    let end = next_month_start(now.year(), first_month + 2);
    let total_days = (end - start).num_days() as f64;
    let elapsed_days = (now.date_naive() - start).num_days() as f64;
    (elapsed_days * SECS_PER_DAY + f64::from(now.num_seconds_from_midnight()))
        / (total_days * SECS_PER_DAY)
}

fn year(now: &DateTime<FixedOffset>) -> f64 {
    let start = NaiveDate::from_ymd_opt(now.year(), 1, 1).unwrap();
    let end = NaiveDate::from_ymd_opt(now.year() + 1, 1, 1).unwrap();
    let total_days = (end - start).num_days() as f64;
    let elapsed_days = (now.ordinal() - 1) as f64;
    (elapsed_days * SECS_PER_DAY + f64::from(now.num_seconds_from_midnight()))
        / (total_days * SECS_PER_DAY)
}

fn next_month_start(year: i32, month: u32) -> NaiveDate {
    let (y, m) = if month >= 12 {
        (
            year + 1 + ((month - 12) / 12) as i32,
            ((month - 12) % 12) + 1,
        )
    } else {
        (year, month + 1)
    };
    NaiveDate::from_ymd_opt(y, m, 1).unwrap_or_else(|| NaiveDate::from_ymd_opt(y, 12, 1).unwrap())
}

