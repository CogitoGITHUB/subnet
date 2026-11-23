//! `clock_state` — temporal boolean states (business-hours / weekend / night) as `Shape::Badge`.
//! `Ok` when active, `Warn` when not.

use chrono::{DateTime, Datelike, FixedOffset, Timelike, Weekday};
use serde::Deserialize;

use crate::fetcher::{FetchContext, RealtimeFetcher, Safety};
use crate::payload::{BadgeData, Body, Payload, Status};
use crate::render::Shape;
use crate::samples;

use super::common;

const SHAPES: &[Shape] = &[Shape::Badge];

pub struct ClockStateFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub kind: Option<Kind>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    #[default]
    BusinessHours,
    Weekend,
    Night,
}

impl RealtimeFetcher for ClockStateFetcher {
    fn name(&self) -> &str {
        "clock_state"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Whether the current moment falls inside a named temporal window (business hours, weekend, night), rendered as a status badge that flips between `Ok` and `Warn`. Pick `kind` to choose which window."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        match shape {
            Shape::Badge => Some(samples::badge(Status::Ok, "open")),
            _ => None,
        }
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let opts: Options = match common::parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return common::placeholder(&msg),
        };
        let now = common::now_in(opts.timezone.as_deref());
        let (active, label) = match opts.kind.unwrap_or(Kind::BusinessHours) {
            Kind::BusinessHours => business_hours(&now),
            Kind::Weekend => weekend(&now),
            Kind::Night => night(&now),
        };
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Badge(BadgeData {
                status: if active { Status::Ok } else { Status::Warn },
                label: label.into(),
            }),
        }
    }
}

fn business_hours(now: &DateTime<FixedOffset>) -> (bool, &'static str) {
    let weekday = !matches!(now.weekday(), Weekday::Sat | Weekday::Sun);
    let in_hours = (9..17).contains(&now.hour());
    let active = weekday && in_hours;
    (active, if active { "open" } else { "closed" })
}

fn weekend(now: &DateTime<FixedOffset>) -> (bool, &'static str) {
    let active = matches!(now.weekday(), Weekday::Sat | Weekday::Sun);
    (active, if active { "weekend" } else { "weekday" })
}

fn night(now: &DateTime<FixedOffset>) -> (bool, &'static str) {
    let active = !(6..22).contains(&now.hour());
    (active, if active { "night" } else { "day" })
}

