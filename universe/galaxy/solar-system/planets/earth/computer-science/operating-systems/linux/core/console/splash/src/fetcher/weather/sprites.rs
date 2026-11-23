//! Bundled 16x16 weather sprites for the `weather` fetcher's `PixelArt` shape. Sprites are
//! authored as ASCII grids — one character per pixel — paired with a palette mapping each
//! character to an RGBA colour. Decoding happens once on first use and the resulting
//! [`PixelArtData`] values are cached in process for the lifetime of the binary.
//!
//! Why ASCII-in-source vs. bundled PNG: the diff stays text-only, sprite edits show up cleanly
//! in PR review, and there is no PNG decode at runtime beyond the constant-time palette lookup
//! used here.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::payload::{PixelArtData, PixelColor};

use super::common::weather_description;

pub const SPRITE_SIZE: usize = 16;

pub struct SpriteCatalog {
    pub clear: PixelArtData,
    pub mostly_clear: PixelArtData,
    pub partly_cloudy: PixelArtData,
    pub overcast: PixelArtData,
    pub fog: PixelArtData,
    pub rain: PixelArtData,
    pub snow: PixelArtData,
    pub thunderstorm: PixelArtData,
}

pub fn catalog() -> &'static SpriteCatalog {
    static CATALOG: OnceLock<SpriteCatalog> = OnceLock::new();
    CATALOG.get_or_init(build_catalog)
}

/// Pick the sprite matching a WMO code (Open-Meteo's standard table). Falls back to `overcast`
/// for codes outside the bundled set so the renderer always has something to draw.
pub fn sprite_for_code(code: u16) -> PixelArtData {
    let c = catalog();
    let key = match code {
        0 => &c.clear,
        1 => &c.mostly_clear,
        2 => &c.partly_cloudy,
        3 => &c.overcast,
        45 | 48 => &c.fog,
        51..=57 | 61..=67 | 80..=82 => &c.rain,
        71..=77 | 85 | 86 => &c.snow,
        95..=99 => &c.thunderstorm,
        _ => &c.overcast,
    };
    let mut sprite = key.clone();
    let (_, label) = weather_description(code);
    sprite.label = Some(label.into());
    sprite
}

fn build_catalog() -> SpriteCatalog {
    SpriteCatalog {
        clear: decode(SPRITE_CLEAR, palette_clear()),
        mostly_clear: decode(SPRITE_MOSTLY_CLEAR, palette_mostly_clear()),
        partly_cloudy: decode(SPRITE_PARTLY_CLOUDY, palette_partly_cloudy()),
        overcast: decode(SPRITE_OVERCAST, palette_overcast()),
        fog: decode(SPRITE_FOG, palette_fog()),
        rain: decode(SPRITE_RAIN, palette_rain()),
        snow: decode(SPRITE_SNOW, palette_snow()),
        thunderstorm: decode(SPRITE_THUNDERSTORM, palette_thunderstorm()),
    }
}

fn decode(grid: &[&str], palette: HashMap<char, PixelColor>) -> PixelArtData {
    let pixels = grid
        .iter()
        .map(|row| {
            row.chars()
                .map(|c| palette.get(&c).copied().unwrap_or(PixelColor::TRANSPARENT).roled())
                .collect()
        })
        .collect();
    PixelArtData {
        pixels,
        label: None,
    }
}

// ---- Palette helpers ----

const SUN_RIM: PixelColor = PixelColor::opaque(255, 213, 79);
const SUN_CORE: PixelColor = PixelColor::opaque(255, 167, 38);
const CLOUD_EDGE: PixelColor = PixelColor::opaque(189, 195, 199);
const CLOUD_FILL: PixelColor = PixelColor::opaque(236, 240, 241);
const CLOUD_DARK: PixelColor = PixelColor::opaque(127, 140, 141);
const RAINDROP: PixelColor = PixelColor::opaque(52, 152, 219);
const SNOW: PixelColor = PixelColor::opaque(245, 245, 245);
const BOLT: PixelColor = PixelColor::opaque(255, 235, 59);
const FOG: PixelColor = PixelColor::opaque(176, 190, 197);

fn palette_clear() -> HashMap<char, PixelColor> {
    HashMap::from([
        ('.', PixelColor::TRANSPARENT),
        ('Y', SUN_RIM),
        ('O', SUN_CORE),
    ])
}

fn palette_mostly_clear() -> HashMap<char, PixelColor> {
    HashMap::from([
        ('.', PixelColor::TRANSPARENT),
        ('Y', SUN_RIM),
        ('O', SUN_CORE),
        ('c', CLOUD_EDGE),
        ('w', CLOUD_FILL),
    ])
}

fn palette_partly_cloudy() -> HashMap<char, PixelColor> {
    HashMap::from([
        ('.', PixelColor::TRANSPARENT),
        ('Y', SUN_RIM),
        ('O', SUN_CORE),
        ('c', CLOUD_EDGE),
        ('w', CLOUD_FILL),
    ])
}

fn palette_overcast() -> HashMap<char, PixelColor> {
    HashMap::from([
        ('.', PixelColor::TRANSPARENT),
        ('c', CLOUD_EDGE),
        ('w', CLOUD_FILL),
        ('d', CLOUD_DARK),
    ])
}

fn palette_fog() -> HashMap<char, PixelColor> {
    HashMap::from([
        ('.', PixelColor::TRANSPARENT),
        ('f', FOG),
        ('w', CLOUD_FILL),
    ])
}

fn palette_rain() -> HashMap<char, PixelColor> {
    HashMap::from([
        ('.', PixelColor::TRANSPARENT),
        ('c', CLOUD_EDGE),
        ('w', CLOUD_FILL),
        ('d', CLOUD_DARK),
        ('r', RAINDROP),
    ])
}

fn palette_snow() -> HashMap<char, PixelColor> {
    HashMap::from([
        ('.', PixelColor::TRANSPARENT),
        ('c', CLOUD_EDGE),
        ('w', CLOUD_FILL),
        ('d', CLOUD_DARK),
        ('s', SNOW),
    ])
}

fn palette_thunderstorm() -> HashMap<char, PixelColor> {
    HashMap::from([
        ('.', PixelColor::TRANSPARENT),
        ('c', CLOUD_EDGE),
        ('w', CLOUD_FILL),
        ('d', CLOUD_DARK),
        ('b', BOLT),
    ])
}

// ---- Sprite grids (16x16) ----

const SPRITE_CLEAR: &[&str] = &[
    "................",
    "................",
    ".....YYYYYY.....",
    "...YYYOOOOYYY...",
    "..YYOOOOOOOOYY..",
    "..YOOOOOOOOOOY..",
    ".YOOOOOOOOOOOOY.",
    ".YOOOOOOOOOOOOY.",
    ".YOOOOOOOOOOOOY.",
    ".YOOOOOOOOOOOOY.",
    "..YOOOOOOOOOOY..",
    "..YYOOOOOOOOYY..",
    "...YYYOOOOYYY...",
    ".....YYYYYY.....",
    "................",
    "................",
];

const SPRITE_MOSTLY_CLEAR: &[&str] = &[
    "................",
    "....YYYY........",
    "...YYOOYY.......",
    "..YYOOOOYY......",
    "..YOOOOOOY......",
    "..YOOOOOOY......",
    "...YOOOOY..ccc..",
    "....YYYY..cwwwc.",
    "........ccwwwwc.",
    "........cwwwwwwc",
    ".........ccwwwc.",
    "..........ccc...",
    "................",
    "................",
    "................",
    "................",
];

const SPRITE_PARTLY_CLOUDY: &[&str] = &[
    "................",
    "................",
    "....cccc........",
    "...cwwwwc.......",
    "..cwwwwwwc......",
    ".cwwwwwwwwc.....",
    ".cwwwwwwwwc.....",
    ".cwwwwwwwwc.....",
    "..cwwwwwwwccc...",
    "...cccccccwwwc..",
    "........cwwwwwc.",
    "........cwwwwwc.",
    ".........cwwwc..",
    "..........ccc...",
    "................",
    "................",
];

const SPRITE_OVERCAST: &[&str] = &[
    "................",
    "................",
    "....cccccc......",
    "...cwwwwwwc.....",
    "..cwwwwwwwwc....",
    ".cwwwwwwwwwwc...",
    ".cwwwwwwwwwwc...",
    ".cwwwwwwwwwwccc.",
    "..cccccccccwwwc.",
    "...ddddddcwwwwc.",
    "..ddwwwwwwwwwwc.",
    ".ddwwwwwwwwwwc..",
    ".dwwwwwwwwwwc...",
    ".ddwwwwwwwwc....",
    "..dddddddddd....",
    "................",
];

// Bands are two pixel rows thick so each one packs into a single half-block cell as solid
// fog rather than a fg/bg pinstripe. Horizontal indent varies per band for a drifting feel.
const SPRITE_FOG: &[&str] = &[
    "................",
    "................",
    "..fffffffffff...",
    "..fffffffffff...",
    "................",
    "................",
    ".fffffffffffff..",
    ".fffffffffffff..",
    "................",
    "................",
    "...fffffffffff..",
    "...fffffffffff..",
    "................",
    "................",
    ".fffffffffffff..",
    ".fffffffffffff..",
];

const SPRITE_RAIN: &[&str] = &[
    "................",
    "....cccccc......",
    "...cwwwwwwc.....",
    "..cwwwwwwwwcc...",
    ".cwwwwwwwwwwc...",
    ".cwwwwwwwwwwc...",
    ".dddddddddddd...",
    "................",
    "..r..r..r..r....",
    ".r..r..r..r..r..",
    "................",
    "..r..r..r..r....",
    ".r..r..r..r..r..",
    "................",
    "..r..r..r..r....",
    "................",
];

const SPRITE_SNOW: &[&str] = &[
    "................",
    "....cccccc......",
    "...cwwwwwwc.....",
    "..cwwwwwwwwcc...",
    ".cwwwwwwwwwwc...",
    ".cwwwwwwwwwwc...",
    ".dddddddddddd...",
    "................",
    "..s..s..s..s....",
    ".s..s..s..s..s..",
    "................",
    "..s..s..s..s....",
    ".s..s..s..s..s..",
    "................",
    "..s..s..s..s....",
    "................",
];

const SPRITE_THUNDERSTORM: &[&str] = &[
    "................",
    "....cccccc......",
    "...cwwwwwwc.....",
    "..cwwwwwwwwcc...",
    ".cwwwwwwwwwwc...",
    ".cwwwwwwwwwwc...",
    ".dddddddddddd...",
    "................",
    "......bbbb......",
    ".....bbbb.......",
    "....bbbb........",
    "...bbbbbbb......",
    "......bbb.......",
    ".....bb.........",
    "....bb..........",
    "................",
];

