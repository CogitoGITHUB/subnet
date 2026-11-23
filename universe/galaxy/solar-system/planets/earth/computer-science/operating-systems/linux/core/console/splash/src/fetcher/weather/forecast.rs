//! `weather_forecast` fetcher — Open-Meteo multi-day daily forecast for a fixed
//! (latitude, longitude). Sibling of `weather` which covers current conditions only.
//!
//! Safety::Safe — host is hardcoded; config supplies coordinates / units / day count, never
//! a URL. No API key.

use async_trait::async_trait;
use chrono::{Datelike, NaiveDate, Weekday};
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Bar, BarsData, Body, EntriesData, Entry, NumberSeriesData, Payload, PointSeries,
    PointSeriesData, RatioData, Status, TextBlockData, TextData, TimelineData, TimelineEvent,
};
use crate::render::Shape;

use super::super::github::common::cache_key;
use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::common::{
    API_BASE, Units, http, parse_options, payload, severity_rank, weather_badge,
    weather_description,
};

const DEFAULT_DAYS: u8 = 3;
const MIN_DAYS: u8 = 1;
const MAX_DAYS: u8 = 7;

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "latitude",
        type_hint: "float (degrees)",
        required: true,
        default: None,
        description: "Latitude of the location to query (e.g., `35.68` for Tokyo).",
    },
    OptionSchema {
        name: "longitude",
        type_hint: "float (degrees)",
        required: true,
        default: None,
        description: "Longitude of the location to query (e.g., `139.76` for Tokyo).",
    },
    OptionSchema {
        name: "units",
        type_hint: "\"metric\" | \"imperial\"",
        required: false,
        default: Some("\"metric\""),
        description: "Temperature unit system. Metric renders °C; imperial renders °F and reports precipitation in inches.",
    },
    OptionSchema {
        name: "days",
        type_hint: "int (1..=7)",
        required: false,
        default: Some("3"),
        description: "How many days of forecast to fetch. Clamped to 1..=7 — Open-Meteo serves longer windows, but accuracy drops past a week.",
    },
];

pub struct WeatherForecastFetcher;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeatherForecastOptions {
    pub latitude: f64,
    pub longitude: f64,
    #[serde(default)]
    pub units: Option<Units>,
    #[serde(default)]
    pub days: Option<u8>,
}

#[async_trait]
impl Fetcher for WeatherForecastFetcher {
    fn name(&self) -> &str {
        "weather_forecast"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Daily multi-day forecast for a fixed (latitude, longitude) via Open-Meteo. `TextBlock` / `Entries` / `Text` summarise highs / lows / precipitation per day; `Ratio` reports the worst precipitation probability in the window; `NumberSeries` carries per-day rainfall totals (tenths of mm or inch) for sparkline / histogram consumers; `Bars` carries per-day precipitation **probability** (%) so the bar chart stays informative even in dry forecasts; `PointSeries` carries high+low temperature curves across days; `Badge` flags the worst weather code; `Timeline` lays the days out chronologically. `days` defaults to 3 (range 1..=7), metric units by default, no API key required."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 60
    }
    fn shapes(&self) -> &[Shape] {
        &[
            Shape::TextBlock,
            Shape::Text,
            Shape::Entries,
            Shape::Ratio,
            Shape::NumberSeries,
            Shape::PointSeries,
            Shape::Bars,
            Shape::Badge,
            Shape::Timeline,
        ]
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
        Some(body_for_shape(&Forecast::sample(), shape))
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: WeatherForecastOptions =
            parse_options(ctx.options.as_ref(), self.name()).map_err(FetchError::Failed)?;
        let days = resolve_days(opts.days).map_err(FetchError::Failed)?;
        let units = opts.units.unwrap_or_default();
        let forecast = fetch_forecast(opts.latitude, opts.longitude, units, days).await?;
        let shape = ctx.shape.unwrap_or(Shape::TextBlock);
        Ok(payload(body_for_shape(&forecast, shape)))
    }
}

fn body_for_shape(forecast: &Forecast, shape: Shape) -> Body {
    match shape {
        Shape::Text => text(forecast),
        Shape::Entries => entries(forecast),
        Shape::Ratio => ratio(forecast),
        Shape::NumberSeries => number_series(forecast),
        Shape::PointSeries => point_series(forecast),
        Shape::Bars => bars(forecast),
        Shape::Badge => badge(forecast),
        Shape::Timeline => timeline(forecast),
        _ => text_block(forecast),
    }
}

fn resolve_days(raw: Option<u8>) -> Result<u8, String> {
    let value = raw.unwrap_or(DEFAULT_DAYS);
    if !(MIN_DAYS..=MAX_DAYS).contains(&value) {
        return Err(format!(
            "weather_forecast `days` must be between {MIN_DAYS} and {MAX_DAYS}, got {value}"
        ));
    }
    Ok(value)
}

async fn fetch_forecast(
    lat: f64,
    lon: f64,
    units: Units,
    days: u8,
) -> Result<Forecast, FetchError> {
    let url = build_url(lat, lon, units, days);
    let res = http()
        .get(&url)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("weather_forecast request failed: {e}")))?;
    let status = res.status();
    if !status.is_success() {
        return Err(FetchError::Failed(format!("weather_forecast {status}")));
    }
    let raw: ApiResponse = res
        .json()
        .await
        .map_err(|e| FetchError::Failed(format!("weather_forecast json parse: {e}")))?;
    Ok(Forecast {
        days: days_from(&raw.daily),
        units,
    })
}

fn build_url(lat: f64, lon: f64, units: Units, days: u8) -> String {
    let base = format!(
        "{API_BASE}?latitude={lat}&longitude={lon}\
         &daily=temperature_2m_max,temperature_2m_min,weather_code,precipitation_sum,precipitation_probability_max\
         &forecast_days={days}\
         &timezone=auto"
    );
    match units {
        Units::Metric => base,
        Units::Imperial => format!("{base}&temperature_unit=fahrenheit&precipitation_unit=inch"),
    }
}

fn days_from(raw: &Daily) -> Vec<DayPoint> {
    let len = raw
        .time
        .len()
        .min(raw.temperature_2m_max.len())
        .min(raw.temperature_2m_min.len())
        .min(raw.weather_code.len());
    (0..len)
        .filter_map(|i| {
            let date = NaiveDate::parse_from_str(&raw.time[i], "%Y-%m-%d").ok()?;
            Some(DayPoint {
                date,
                weather_code: raw.weather_code[i],
                high: raw.temperature_2m_max[i],
                low: raw.temperature_2m_min[i],
                precipitation: raw.precipitation_sum.get(i).copied().unwrap_or(0.0),
                precip_probability: raw
                    .precipitation_probability_max
                    .get(i)
                    .copied()
                    .unwrap_or(0),
            })
        })
        .collect()
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    #[serde(default)]
    daily: Daily,
}

#[derive(Debug, Default, Deserialize)]
struct Daily {
    #[serde(default)]
    time: Vec<String>,
    #[serde(default)]
    temperature_2m_max: Vec<f64>,
    #[serde(default)]
    temperature_2m_min: Vec<f64>,
    #[serde(default)]
    weather_code: Vec<u16>,
    #[serde(default)]
    precipitation_sum: Vec<f64>,
    #[serde(default)]
    precipitation_probability_max: Vec<u8>,
}

struct Forecast {
    days: Vec<DayPoint>,
    units: Units,
}

#[derive(Debug, Clone, Copy)]
struct DayPoint {
    date: NaiveDate,
    weather_code: u16,
    high: f64,
    low: f64,
    precipitation: f64,
    precip_probability: u8,
}

impl Forecast {
    fn sample() -> Self {
        let base = NaiveDate::from_ymd_opt(2026, 5, 11).expect("valid date");
        let days = [
            (2u16, 22.4, 15.1, 0.0, 10u8),
            (63, 18.2, 12.8, 8.4, 80),
            (1, 20.5, 14.7, 0.0, 30),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (code, high, low, precip, prob))| DayPoint {
            date: base + chrono::Duration::days(i as i64),
            weather_code: code,
            high,
            low,
            precipitation: precip,
            precip_probability: prob,
        })
        .collect();
        Self {
            days,
            units: Units::Metric,
        }
    }
}

fn weekday_short(weekday: Weekday) -> &'static str {
    match weekday {
        Weekday::Mon => "Mon",
        Weekday::Tue => "Tue",
        Weekday::Wed => "Wed",
        Weekday::Thu => "Thu",
        Weekday::Fri => "Fri",
        Weekday::Sat => "Sat",
        Weekday::Sun => "Sun",
    }
}

fn temp_range_label(day: &DayPoint, units: Units) -> String {
    format!(
        "{:.0}°/{:.0}°{}",
        day.high,
        day.low,
        units.temperature_label().trim_start_matches('°')
    )
}

fn precip_label(day: &DayPoint, units: Units) -> Option<String> {
    if day.precipitation <= 0.0 {
        return None;
    }
    let precision = match units {
        Units::Metric => 1,
        Units::Imperial => 2,
    };
    Some(format!(
        "{:.*}{}",
        precision,
        day.precipitation,
        units.precipitation_label()
    ))
}

fn precip_summary(day: &DayPoint, units: Units) -> String {
    match precip_label(day, units) {
        Some(amount) => format!("💧{}% {amount}", day.precip_probability),
        None => format!("💧{}%", day.precip_probability),
    }
}

fn day_line(day: &DayPoint, units: Units) -> String {
    let (emoji, _) = weather_description(day.weather_code);
    format!(
        "{}  {emoji} {}  {}",
        weekday_short(day.date.weekday()),
        temp_range_label(day, units),
        precip_summary(day, units)
    )
}

fn text_block(forecast: &Forecast) -> Body {
    Body::TextBlock(TextBlockData {
        lines: forecast
            .days
            .iter()
            .map(|d| day_line(d, forecast.units))
            .collect(),
    })
}

fn text(forecast: &Forecast) -> Body {
    let summary = forecast
        .days
        .iter()
        .map(|d| {
            let (emoji, _) = weather_description(d.weather_code);
            format!("{emoji} {}", temp_range_label(d, forecast.units))
        })
        .collect::<Vec<_>>()
        .join(" → ");
    Body::Text(TextData { value: summary })
}

fn entries(forecast: &Forecast) -> Body {
    Body::Entries(EntriesData {
        items: forecast
            .days
            .iter()
            .map(|d| {
                let (emoji, _) = weather_description(d.weather_code);
                Entry {
                    key: format!("{} {emoji}", weekday_short(d.date.weekday())),
                    value: Some(format!(
                        "{}  {}",
                        temp_range_label(d, forecast.units),
                        precip_summary(d, forecast.units)
                    )),
                    status: None,
                }
            })
            .collect(),
    })
}

/// `Ratio` summarises the worst precipitation chance across the window. Pairs with
/// `gauge_circle` / `gauge_battery` for a "umbrella?" glance — the single highest probability
/// in the next few days, labelled with the day it falls on.
fn ratio(forecast: &Forecast) -> Body {
    let worst = forecast
        .days
        .iter()
        .max_by_key(|d| d.precip_probability)
        .copied();
    let (value, label) = match worst {
        Some(day) => (
            f64::from(day.precip_probability) / 100.0,
            Some(format!("{} rain risk", weekday_short(day.date.weekday()))),
        ),
        None => (0.0, None),
    };
    Body::Ratio(RatioData {
        value,
        label,
        denominator: Some(100),
    })
}

/// Per-day precipitation totals in tenths of mm (or inch for imperial). Same precision
/// strategy as `weather`'s hourly precipitation — survives the `u64` round-trip without
/// losing the sub-millimetre signal that distinguishes drizzle from "actually wet".
fn number_series(forecast: &Forecast) -> Body {
    Body::NumberSeries(NumberSeriesData {
        values: forecast
            .days
            .iter()
            .map(|d| (d.precipitation.max(0.0) * 10.0).round() as u64)
            .collect(),
    })
}

/// Two series — `high` and `low` temperatures — across days. `PointSeries` because
/// temperature can go negative; `NumberSeries` would silently clamp.
fn point_series(forecast: &Forecast) -> Body {
    let unit = forecast.units.temperature_label();
    let to_points = |pick: fn(&DayPoint) -> f64| -> Vec<(f64, f64)> {
        forecast
            .days
            .iter()
            .enumerate()
            .map(|(i, d)| (i as f64, pick(d)))
            .collect()
    };
    Body::PointSeries(PointSeriesData {
        series: vec![
            PointSeries {
                name: format!("high ({unit})"),
                points: to_points(|d| d.high),
            },
            PointSeries {
                name: format!("low ({unit})"),
                points: to_points(|d| d.low),
            },
        ],
    })
}

/// Per-day precipitation **probability** (0..=100) with weekday labels. Probability rather
/// than rainfall amount because most forecasts have wide stretches of `precipitation_sum =
/// 0mm` even when chance-of-rain varies meaningfully (15% Tue, 35% Wed, 10% Thu). The
/// amount sequence stays available on `NumberSeries` for sparkline / histogram consumers.
fn bars(forecast: &Forecast) -> Body {
    Body::Bars(BarsData {
        bars: forecast
            .days
            .iter()
            .map(|d| Bar {
                label: weekday_short(d.date.weekday()).into(),
                value: u64::from(d.precip_probability),
                value_label: None,
            })
            .collect(),
    })
}

fn badge(forecast: &Forecast) -> Body {
    let worst_code = forecast
        .days
        .iter()
        .max_by_key(|d| severity_rank(d.weather_code))
        .map(|d| d.weather_code)
        .unwrap_or(0);
    let mut data = weather_badge(worst_code);
    if data.status == Status::Ok {
        data = BadgeData {
            status: Status::Ok,
            label: "clear ahead".into(),
        };
    }
    Body::Badge(data)
}

fn timeline(forecast: &Forecast) -> Body {
    Body::Timeline(TimelineData {
        events: forecast
            .days
            .iter()
            .map(|d| {
                let (emoji, condition) = weather_description(d.weather_code);
                TimelineEvent {
                    timestamp: d
                        .date
                        .and_hms_opt(0, 0, 0)
                        .expect("midnight is always valid")
                        .and_utc()
                        .timestamp(),
                    title: format!("{emoji} {condition}"),
                    detail: Some(format!(
                        "{}  {}",
                        temp_range_label(d, forecast.units),
                        precip_summary(d, forecast.units)
                    )),
                    status: Some(weather_badge(d.weather_code).status),
                }
            })
            .collect(),
    })
}

