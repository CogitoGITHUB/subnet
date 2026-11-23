//! `weather` fetcher — Open-Meteo "current conditions" snapshot for a fixed
//! (latitude, longitude). Multi-shape: Entries (default) / Text / PointSeries / NumberSeries /
//! Bars / Badge.
//!
//! Safety::Safe because the host is hardcoded: the user supplies coordinates, not a URL, so
//! config can't redirect traffic to an attacker-controlled origin. No API key is required and
//! no token leaves the machine.

use async_trait::async_trait;
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{
    Bar, BarsData, Body, EntriesData, Entry, NumberSeriesData, Payload, PointSeries,
    PointSeriesData, TextData,
};
use crate::render::Shape;

use super::super::github::common::cache_key;
use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::common::{
    API_BASE, Units, http, parse_options, payload, weather_badge, weather_description,
};
use super::sprites::sprite_for_code;

const HOURLY_HOURS: usize = 24;

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
        description: "Temperature and wind unit system. Metric renders °C / m/s; imperial renders °F / mph.",
    },
];

/// Open-Meteo "now" widget. Entries shape (primary): condition / temp / wind / humidity.
pub struct WeatherFetcher;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeatherOptions {
    pub latitude: f64,
    pub longitude: f64,
    #[serde(default)]
    pub units: Option<Units>,
}

#[async_trait]
impl Fetcher for WeatherFetcher {
    fn name(&self) -> &str {
        "weather"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Forecast for a fixed (latitude, longitude) via Open-Meteo. `Entries` / `Text` summarise current conditions; `PointSeries` carries the next 24h of hourly temperature (signed °C / °F); `NumberSeries` / `Bars` carry hourly precipitation in tenths of mm/inch; `Badge` flags severe weather codes (thunderstorm = error, freezing / snow / heavy rain = warn); `PixelArt` paints a 16×16 truecolor sprite of the current sky. Metric or imperial units, no API key required."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 30
    }
    fn shapes(&self) -> &[Shape] {
        &[
            Shape::Entries,
            Shape::Text,
            Shape::PointSeries,
            Shape::NumberSeries,
            Shape::Bars,
            Shape::Badge,
            Shape::PixelArt,
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
        let sample = Sample::default();
        Some(match shape {
            Shape::Entries => entries(&sample),
            Shape::Text => Body::Text(TextData {
                value: sample.as_text(),
            }),
            Shape::PointSeries => point_series(&sample.hourly, sample.units),
            Shape::NumberSeries => number_series(&sample.hourly),
            Shape::Bars => precipitation_bars(&sample.hourly),
            Shape::Badge => Body::Badge(weather_badge(sample.code)),
            Shape::PixelArt => Body::PixelArt(sprite_for_code(sample.code)),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: WeatherOptions =
            parse_options(ctx.options.as_ref(), self.name()).map_err(FetchError::Failed)?;
        let units = opts.units.unwrap_or_default();
        let report = fetch_report(opts.latitude, opts.longitude, units).await?;
        let body = match ctx.shape.unwrap_or(Shape::Entries) {
            Shape::Text => Body::Text(TextData {
                value: report.as_text(),
            }),
            Shape::PointSeries => point_series(&report.hourly, report.units),
            Shape::NumberSeries => number_series(&report.hourly),
            Shape::Bars => precipitation_bars(&report.hourly),
            Shape::Badge => Body::Badge(weather_badge(report.code)),
            Shape::PixelArt => Body::PixelArt(sprite_for_code(report.code)),
            _ => entries(&report),
        };
        Ok(payload(body))
    }
}

async fn fetch_report(lat: f64, lon: f64, units: Units) -> Result<Sample, FetchError> {
    let url = build_url(lat, lon, units);
    let res = http()
        .get(&url)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("weather request failed: {e}")))?;
    let status = res.status();
    if !status.is_success() {
        return Err(FetchError::Failed(format!("weather {status}")));
    }
    let raw: ApiResponse = res
        .json()
        .await
        .map_err(|e| FetchError::Failed(format!("weather json parse: {e}")))?;
    Ok(Sample {
        code: raw.current.weather_code,
        temperature: raw.current.temperature_2m,
        wind_speed: raw.current.wind_speed_10m,
        humidity: raw.current.relative_humidity_2m,
        hourly: hourly_from(&raw.hourly),
        units,
    })
}

fn hourly_from(raw: &Hourly) -> Vec<HourPoint> {
    let temps = raw.temperature_2m.iter().copied();
    let precs = raw
        .precipitation
        .iter()
        .copied()
        .chain(std::iter::repeat(0.0));
    temps
        .zip(precs)
        .take(HOURLY_HOURS)
        .enumerate()
        .map(|(i, (temp, precip))| HourPoint {
            offset_hours: i as u32,
            temperature: temp,
            precipitation: precip,
        })
        .collect()
}

fn build_url(lat: f64, lon: f64, units: Units) -> String {
    let base = format!(
        "{API_BASE}?latitude={lat}&longitude={lon}\
         &current=temperature_2m,weather_code,wind_speed_10m,relative_humidity_2m\
         &hourly=temperature_2m,precipitation\
         &forecast_days=2"
    );
    match units {
        Units::Metric => format!("{base}&wind_speed_unit=ms"),
        Units::Imperial => format!("{base}&temperature_unit=fahrenheit&wind_speed_unit=mph"),
    }
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    current: Current,
    #[serde(default)]
    hourly: Hourly,
}

#[derive(Debug, Deserialize)]
struct Current {
    temperature_2m: f64,
    weather_code: u16,
    wind_speed_10m: f64,
    relative_humidity_2m: u8,
}

#[derive(Debug, Default, Deserialize)]
struct Hourly {
    #[serde(default)]
    temperature_2m: Vec<f64>,
    #[serde(default)]
    precipitation: Vec<f64>,
}

struct Sample {
    code: u16,
    temperature: f64,
    wind_speed: f64,
    humidity: u8,
    hourly: Vec<HourPoint>,
    units: Units,
}

#[derive(Debug, Clone, Copy)]
struct HourPoint {
    offset_hours: u32,
    temperature: f64,
    /// Precipitation in mm for metric, inches for imperial.
    precipitation: f64,
}

impl Default for Sample {
    fn default() -> Self {
        let hourly = (0..HOURLY_HOURS as u32)
            .map(|i| HourPoint {
                offset_hours: i,
                temperature: 16.0 + ((i as f64) * 0.4 - (i as f64 / 8.0).sin() * 4.0),
                precipitation: if (10..=14).contains(&i) { 0.6 } else { 0.0 },
            })
            .collect();
        Self {
            code: 3,
            temperature: 18.0,
            wind_speed: 4.0,
            humidity: 67,
            hourly,
            units: Units::Metric,
        }
    }
}

impl Sample {
    fn temperature_label(&self) -> String {
        format!("{:.0}{}", self.temperature, self.units.temperature_label())
    }
    fn wind_label(&self) -> String {
        format!("{:.0} {}", self.wind_speed, self.units.wind_label())
    }
    fn humidity_label(&self) -> String {
        format!("{}%", self.humidity)
    }
    fn as_text(&self) -> String {
        let (emoji, condition) = weather_description(self.code);
        format!(
            "{emoji} {condition} · {} · 💨 {} · 💧 {}",
            self.temperature_label(),
            self.wind_label(),
            self.humidity_label(),
        )
    }
}

fn point_series(hourly: &[HourPoint], units: Units) -> Body {
    Body::PointSeries(PointSeriesData {
        series: vec![PointSeries {
            name: format!("temperature ({})", units.temperature_label()),
            points: hourly
                .iter()
                .map(|h| (f64::from(h.offset_hours), h.temperature))
                .collect(),
        }],
    })
}

/// Hourly precipitation as `NumberSeries`. Temperature can go negative and would silently
/// clamp to 0 in `u64`, so the curve lives on `PointSeries` instead. Precipitation is
/// non-negative by definition; values are tenths of a millimetre (or an inch on imperial)
/// so one decimal of precision survives the integer round-trip.
fn number_series(hourly: &[HourPoint]) -> Body {
    Body::NumberSeries(NumberSeriesData {
        values: hourly
            .iter()
            .map(|h| (h.precipitation.max(0.0) * 10.0).round() as u64)
            .collect(),
    })
}

fn precipitation_bars(hourly: &[HourPoint]) -> Body {
    Body::Bars(BarsData {
        bars: hourly
            .iter()
            .map(|h| Bar {
                label: format!("+{}h", h.offset_hours),
                // Bars are integer-valued; multiply by 10 to keep one decimal of precision
                // (i.e. units = "tenths of mm" / "tenths of an inch").
                value: (h.precipitation.max(0.0) * 10.0).round() as u64,
                value_label: None,
            })
            .collect(),
    })
}

fn entries(s: &Sample) -> Body {
    let (emoji, condition) = weather_description(s.code);
    let rows = [
        (format!("{emoji} {condition}"), s.temperature_label()),
        ("💨 wind".into(), s.wind_label()),
        ("💧 humidity".into(), s.humidity_label()),
    ];
    Body::Entries(EntriesData {
        items: rows
            .into_iter()
            .map(|(k, v)| Entry {
                key: k,
                value: Some(v),
                status: None,
            })
            .collect(),
    })
}

