//! `clock_derived` — one-line values computed from today's date with no network or external
//! data. `kind` picks the computation: moon phase, zodiac, season, iso week, etc.

use chrono::{DateTime, Datelike, FixedOffset, Timelike};
use serde::Deserialize;

use crate::fetcher::{FetchContext, RealtimeFetcher, Safety};
use crate::payload::{Body, Payload, TextData};
use crate::render::Shape;
use crate::samples;

use super::common;
use super::sprites;

const SHAPES: &[Shape] = &[Shape::Text, Shape::PixelArt];

pub struct ClockDerivedFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub kind: Option<Kind>,
    /// Only meaningful when `kind = "season"`. `"north"` (default) applies Northern
    /// Hemisphere seasons (Mar–May = Spring, etc.); `"south"` flips them for users
    /// below the equator so April reads as Autumn instead of Spring.
    #[serde(default)]
    pub hemisphere: Option<Hemisphere>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Hemisphere {
    #[default]
    North,
    South,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    #[default]
    TimeOfDay,
    MoonPhase,
    Zodiac,
    ChineseZodiac,
    Season,
    JpSeason,
    Rokuyou,
    IsoWeek,
    DayOfYear,
    JulianDay,
    UnixEpoch,
}

impl RealtimeFetcher for ClockDerivedFetcher {
    fn name(&self) -> &str {
        "clock_derived"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "A single value derived from today's date, picked by `kind` (time of day, moon phase, zodiac, chinese zodiac, season, Japanese seasonal term, rokuyou, ISO week, day of year, Julian day, Unix epoch). Use `clock_almanac` to show several of these together as one block."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        match shape {
            Shape::Text => Some(samples::text("day 113 of 2026")),
            // Pick the most recognizable sprite (Full moon) so docs previews lead with a
            // visually striking sample rather than an empty New-moon disk.
            Shape::PixelArt => Some(Body::PixelArt(sprites::moon_sprite(4, "Full"))),
            _ => None,
        }
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let opts: Options = match common::parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return common::placeholder(&msg),
        };
        let now = common::now_in(opts.timezone.as_deref());
        let kind = opts.kind.unwrap_or(Kind::TimeOfDay);
        let hemisphere = opts.hemisphere.unwrap_or_default();
        if matches!(ctx.shape, Some(Shape::PixelArt)) {
            return Payload {
                icon: None,
                status: None,
                format: None,
                body: Body::PixelArt(sprite_for_kind(&now, kind, hemisphere)),
            };
        }
        let line = invoke(&now, kind, hemisphere);
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Text(TextData { value: line }),
        }
    }
}

/// Pick a sprite for the kind the user configured. Only moon-phase and season have visual
/// translations today; numeric / textual kinds (`iso_week`, `day_of_year`, `julian_day`,
/// `unix_epoch`, `time_of_day`, `rokuyou`, `jp_season`, `zodiac`, `chinese_zodiac`) fall
/// through to a labelled placeholder so the misconfiguration is visible rather than silent.
fn sprite_for_kind(
    now: &DateTime<FixedOffset>,
    kind: Kind,
    hemisphere: Hemisphere,
) -> crate::payload::PixelArtData {
    match kind {
        Kind::MoonPhase => {
            let idx = moon_phase_index(now);
            sprites::moon_sprite(idx, MOON_LABELS[idx])
        }
        Kind::Season => sprites::season_sprite(&season(now, hemisphere)),
        other => sprites::placeholder(kind_label(other)),
    }
}

fn kind_label(k: Kind) -> &'static str {
    match k {
        Kind::TimeOfDay => "time_of_day",
        Kind::MoonPhase => "moon_phase",
        Kind::Zodiac => "zodiac",
        Kind::ChineseZodiac => "chinese_zodiac",
        Kind::Season => "season",
        Kind::JpSeason => "jp_season",
        Kind::Rokuyou => "rokuyou",
        Kind::IsoWeek => "iso_week",
        Kind::DayOfYear => "day_of_year",
        Kind::JulianDay => "julian_day",
        Kind::UnixEpoch => "unix_epoch",
    }
}

const MOON_LABELS: [&str; 8] = [
    "New",
    "Waxing Crescent",
    "First Quarter",
    "Waxing Gibbous",
    "Full",
    "Waning Gibbous",
    "Last Quarter",
    "Waning Crescent",
];

/// Dispatch a single kind against an already-resolved `now`. Shared with the
/// `clock_almanac` rollup so both fetchers stay in sync on formatting.
pub(crate) fn invoke(now: &DateTime<FixedOffset>, kind: Kind, hemisphere: Hemisphere) -> String {
    match kind {
        Kind::TimeOfDay => time_of_day(now).into(),
        Kind::MoonPhase => moon_phase(now),
        Kind::Zodiac => zodiac(now),
        Kind::ChineseZodiac => chinese_zodiac(now),
        Kind::Season => season(now, hemisphere),
        Kind::JpSeason => jp_season(now),
        Kind::Rokuyou => rokuyou(now),
        Kind::IsoWeek => iso_week(now),
        Kind::DayOfYear => format!("day {}", now.ordinal()),
        Kind::JulianDay => format!("JD {}", common::julian_day(now.date_naive())),
        Kind::UnixEpoch => now.timestamp().to_string(),
    }
}

fn time_of_day(now: &DateTime<FixedOffset>) -> &'static str {
    match now.hour() {
        5..=11 => "morning",
        12..=16 => "afternoon",
        17..=20 => "evening",
        _ => "night",
    }
}

/// Conway's moon-phase approximation expressed as an index 0..=7. Shared with the
/// `PixelArt` shape so the sprite stays consistent with the text label. ±1 day accuracy,
/// adequate for a splash widget.
pub(crate) fn moon_phase_index(now: &DateTime<FixedOffset>) -> usize {
    let (mut year, month, day) = (now.year(), now.month() as i32, now.day() as i32);
    let m = if month < 3 {
        year -= 1;
        month + 12
    } else {
        month
    };
    let c = (year / 100) as f64;
    let y = (year % 100) as f64;
    let jdn = (365.25 * (y + 4712.0)).floor() + (30.6 * (m as f64 + 1.0)).floor() + day as f64
        - (c * 0.75).floor()
        + c
        - 37.5;
    let phase = ((jdn - 2_451_550.1) / 29.530_588_853).rem_euclid(1.0);
    (phase * 8.0).floor() as usize % 8
}

fn moon_phase(now: &DateTime<FixedOffset>) -> String {
    let idx = moon_phase_index(now);
    let glyph = ["🌑", "🌒", "🌓", "🌔", "🌕", "🌖", "🌗", "🌘"][idx];
    format!("{glyph} {}", MOON_LABELS[idx])
}

fn zodiac(now: &DateTime<FixedOffset>) -> String {
    let (m, d) = (now.month(), now.day());
    let (name, glyph) = match (m, d) {
        (3, 21..=31) | (4, 1..=19) => ("Aries", "♈"),
        (4, 20..=30) | (5, 1..=20) => ("Taurus", "♉"),
        (5, 21..=31) | (6, 1..=20) => ("Gemini", "♊"),
        (6, 21..=30) | (7, 1..=22) => ("Cancer", "♋"),
        (7, 23..=31) | (8, 1..=22) => ("Leo", "♌"),
        (8, 23..=31) | (9, 1..=22) => ("Virgo", "♍"),
        (9, 23..=30) | (10, 1..=22) => ("Libra", "♎"),
        (10, 23..=31) | (11, 1..=21) => ("Scorpio", "♏"),
        (11, 22..=30) | (12, 1..=21) => ("Sagittarius", "♐"),
        (12, 22..=31) | (1, 1..=19) => ("Capricorn", "♑"),
        (1, 20..=31) | (2, 1..=18) => ("Aquarius", "♒"),
        _ => ("Pisces", "♓"),
    };
    format!("{glyph} {name}")
}

fn chinese_zodiac(now: &DateTime<FixedOffset>) -> String {
    // 1900 = Rat; approximation ignores the lunar-new-year boundary.
    let animals = [
        ("Rat", "🐀"),
        ("Ox", "🐂"),
        ("Tiger", "🐅"),
        ("Rabbit", "🐇"),
        ("Dragon", "🐉"),
        ("Snake", "🐍"),
        ("Horse", "🐎"),
        ("Goat", "🐐"),
        ("Monkey", "🐒"),
        ("Rooster", "🐓"),
        ("Dog", "🐕"),
        ("Pig", "🐖"),
    ];
    let idx = ((now.year() - 1900).rem_euclid(12)) as usize;
    let (name, glyph) = animals[idx];
    format!("{glyph} {name}")
}

fn season(now: &DateTime<FixedOffset>, hemisphere: Hemisphere) -> String {
    let north = match now.month() {
        3..=5 => "Spring",
        6..=8 => "Summer",
        9..=11 => "Autumn",
        _ => "Winter",
    };
    match hemisphere {
        Hemisphere::North => north.into(),
        // Seasons are ~6 months shifted below the equator: April (Spring up north) is
        // Autumn in Sydney / Auckland / Buenos Aires.
        Hemisphere::South => match north {
            "Spring" => "Autumn",
            "Summer" => "Winter",
            "Autumn" => "Spring",
            _ => "Summer",
        }
        .into(),
    }
}

fn jp_season(now: &DateTime<FixedOffset>) -> String {
    // 二十四節気, fixed-date approximation (actual solar terms shift ±1 day year to year).
    const TERMS: &[(u32, u32, &str)] = &[
        (1, 6, "小寒"),
        (1, 20, "大寒"),
        (2, 4, "立春"),
        (2, 19, "雨水"),
        (3, 5, "啓蟄"),
        (3, 21, "春分"),
        (4, 5, "清明"),
        (4, 20, "穀雨"),
        (5, 5, "立夏"),
        (5, 21, "小満"),
        (6, 6, "芒種"),
        (6, 21, "夏至"),
        (7, 7, "小暑"),
        (7, 23, "大暑"),
        (8, 8, "立秋"),
        (8, 23, "処暑"),
        (9, 8, "白露"),
        (9, 23, "秋分"),
        (10, 8, "寒露"),
        (10, 24, "霜降"),
        (11, 7, "立冬"),
        (11, 22, "小雪"),
        (12, 7, "大雪"),
        (12, 22, "冬至"),
    ];
    let (m, d) = (now.month(), now.day());
    let mut current = "小寒";
    for &(tm, td, name) in TERMS {
        if (m, d) >= (tm, td) {
            current = name;
        }
    }
    current.into()
}

fn rokuyou(now: &DateTime<FixedOffset>) -> String {
    let jd = common::julian_day(now.date_naive());
    let idx = (jd.rem_euclid(6)) as usize;
    ["大安", "赤口", "先勝", "友引", "先負", "仏滅"][idx].into()
}

fn iso_week(now: &DateTime<FixedOffset>) -> String {
    let iso = now.iso_week();
    format!("{}-W{:02}", iso.year(), iso.week())
}

