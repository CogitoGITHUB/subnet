//! Pixel-art sprites for `clock_derived`'s `PixelArt` shape. Same authoring pattern as
//! `weather/sprites.rs`: ASCII grids paired with a palette, decoded once into
//! [`PixelArtData`] and stashed in a `OnceLock` for the lifetime of the process.
//!
//! Only kinds with a natural visual translation are supported here. Numeric / textual kinds
//! (`iso_week`, `day_of_year`, `julian_day`, `unix_epoch`, `time_of_day`, `rokuyou`,
//! `jp_season`) fall through to a placeholder so a misconfigured widget surfaces a clear
//! "no sprite for kind" hint rather than a blank frame.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::payload::{PixelArtData, PixelColor};

pub const SPRITE_SIZE: usize = 16;

// ── Public catalog API ─────────────────────────────────────────────

pub fn moon_sprite(phase_idx: usize, label: &str) -> PixelArtData {
    let catalog = moon_catalog();
    let mut sprite = catalog[phase_idx % 8].clone();
    sprite.label = Some(label.into());
    sprite
}

pub fn season_sprite(name: &str) -> PixelArtData {
    let c = season_catalog();
    let mut sprite = match name {
        "Spring" => c.spring.clone(),
        "Summer" => c.summer.clone(),
        "Autumn" => c.autumn.clone(),
        _ => c.winter.clone(),
    };
    sprite.label = Some(name.into());
    sprite
}

pub fn placeholder(kind_label: &str) -> PixelArtData {
    PixelArtData {
        pixels: vec![vec![PixelColor::TRANSPARENT; 1]],
        label: Some(format!("no sprite: {kind_label}")),
    }
}

// ── Catalogs ───────────────────────────────────────────────────────

fn moon_catalog() -> &'static [PixelArtData; 8] {
    static CATALOG: OnceLock<[PixelArtData; 8]> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let pal = palette_moon();
        [
            decode(MOON_NEW, &pal),
            decode(MOON_WAXING_CRESCENT, &pal),
            decode(MOON_FIRST_QUARTER, &pal),
            decode(MOON_WAXING_GIBBOUS, &pal),
            decode(MOON_FULL, &pal),
            decode(MOON_WANING_GIBBOUS, &pal),
            decode(MOON_LAST_QUARTER, &pal),
            decode(MOON_WANING_CRESCENT, &pal),
        ]
    })
}

struct SeasonCatalog {
    spring: PixelArtData,
    summer: PixelArtData,
    autumn: PixelArtData,
    winter: PixelArtData,
}

fn season_catalog() -> &'static SeasonCatalog {
    static CATALOG: OnceLock<SeasonCatalog> = OnceLock::new();
    CATALOG.get_or_init(|| SeasonCatalog {
        spring: decode(SPRING, &palette_spring()),
        summer: decode(SUMMER, &palette_summer()),
        autumn: decode(AUTUMN, &palette_autumn()),
        winter: decode(WINTER, &palette_winter()),
    })
}

fn decode(grid: &[&str], palette: &HashMap<char, PixelColor>) -> PixelArtData {
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

// ── Palettes ───────────────────────────────────────────────────────

const MOON_DARK: PixelColor = PixelColor::opaque(57, 62, 70);
const MOON_LIT: PixelColor = PixelColor::opaque(245, 232, 199);
const CHERRY_PETAL: PixelColor = PixelColor::opaque(255, 182, 193);
const CHERRY_CORE: PixelColor = PixelColor::opaque(255, 105, 180);
const CHERRY_POLLEN: PixelColor = PixelColor::opaque(255, 235, 59);
const LEAF_GREEN: PixelColor = PixelColor::opaque(76, 175, 80);
const STEM_BROWN: PixelColor = PixelColor::opaque(121, 85, 72);
const SUN_RIM: PixelColor = PixelColor::opaque(255, 213, 79);
const SUN_CORE: PixelColor = PixelColor::opaque(255, 152, 0);
const MAPLE_RED: PixelColor = PixelColor::opaque(229, 57, 53);
const MAPLE_ORANGE: PixelColor = PixelColor::opaque(255, 152, 0);
const SNOW_WHITE: PixelColor = PixelColor::opaque(245, 245, 245);
const SKY_BLUE: PixelColor = PixelColor::opaque(176, 224, 230);

fn palette_moon() -> HashMap<char, PixelColor> {
    HashMap::from([
        ('.', PixelColor::TRANSPARENT),
        ('D', MOON_DARK),
        ('L', MOON_LIT),
    ])
}

fn palette_spring() -> HashMap<char, PixelColor> {
    HashMap::from([
        ('.', PixelColor::TRANSPARENT),
        ('P', CHERRY_PETAL),
        ('p', CHERRY_CORE),
        ('Y', CHERRY_POLLEN),
        ('G', LEAF_GREEN),
        ('T', STEM_BROWN),
    ])
}

fn palette_summer() -> HashMap<char, PixelColor> {
    HashMap::from([
        ('.', PixelColor::TRANSPARENT),
        ('Y', SUN_RIM),
        ('O', SUN_CORE),
    ])
}

fn palette_autumn() -> HashMap<char, PixelColor> {
    HashMap::from([
        ('.', PixelColor::TRANSPARENT),
        ('R', MAPLE_RED),
        ('O', MAPLE_ORANGE),
        ('T', STEM_BROWN),
    ])
}

fn palette_winter() -> HashMap<char, PixelColor> {
    HashMap::from([
        ('.', PixelColor::TRANSPARENT),
        ('S', SNOW_WHITE),
        ('B', SKY_BLUE),
    ])
}

// ── Moon-phase sprites (idx 0..=7 in standard astronomical order) ──

const MOON_NEW: &[&str] = &[
    "......DDDD......",
    "....DDDDDDDD....",
    "...DDDDDDDDDD...",
    "..DDDDDDDDDDDD..",
    "..DDDDDDDDDDDD..",
    ".DDDDDDDDDDDDDD.",
    ".DDDDDDDDDDDDDD.",
    ".DDDDDDDDDDDDDD.",
    ".DDDDDDDDDDDDDD.",
    ".DDDDDDDDDDDDDD.",
    ".DDDDDDDDDDDDDD.",
    "..DDDDDDDDDDDD..",
    "..DDDDDDDDDDDD..",
    "...DDDDDDDDDD...",
    "....DDDDDDDD....",
    "......DDDD......",
];

const MOON_WAXING_CRESCENT: &[&str] = &[
    "......DDDD......",
    "....DDDDDDDD....",
    "...DDDDDDDDDL...",
    "..DDDDDDDDDDLL..",
    "..DDDDDDDDDDLL..",
    ".DDDDDDDDDDDLLL.",
    ".DDDDDDDDDDDLLL.",
    ".DDDDDDDDDDDLLL.",
    ".DDDDDDDDDDDLLL.",
    ".DDDDDDDDDDDLLL.",
    ".DDDDDDDDDDDLLL.",
    "..DDDDDDDDDDLL..",
    "..DDDDDDDDDDLL..",
    "...DDDDDDDDDL...",
    "....DDDDDDDD....",
    "......DDDD......",
];

const MOON_FIRST_QUARTER: &[&str] = &[
    "......DDLL......",
    "....DDDDLLLL....",
    "...DDDDDLLLLL...",
    "..DDDDDDLLLLLL..",
    "..DDDDDDLLLLLL..",
    ".DDDDDDDLLLLLLL.",
    ".DDDDDDDLLLLLLL.",
    ".DDDDDDDLLLLLLL.",
    ".DDDDDDDLLLLLLL.",
    ".DDDDDDDLLLLLLL.",
    ".DDDDDDDLLLLLLL.",
    "..DDDDDDLLLLLL..",
    "..DDDDDDLLLLLL..",
    "...DDDDDLLLLL...",
    "....DDDDLLLL....",
    "......DDLL......",
];

const MOON_WAXING_GIBBOUS: &[&str] = &[
    "......LLLL......",
    "....LLLLLLLL....",
    "...DLLLLLLLLL...",
    "..DDLLLLLLLLLL..",
    "..DDLLLLLLLLLL..",
    ".DDDLLLLLLLLLLL.",
    ".DDDLLLLLLLLLLL.",
    ".DDDLLLLLLLLLLL.",
    ".DDDLLLLLLLLLLL.",
    ".DDDLLLLLLLLLLL.",
    ".DDDLLLLLLLLLLL.",
    "..DDLLLLLLLLLL..",
    "..DDLLLLLLLLLL..",
    "...DLLLLLLLLL...",
    "....LLLLLLLL....",
    "......LLLL......",
];

const MOON_FULL: &[&str] = &[
    "......LLLL......",
    "....LLLLLLLL....",
    "...LLLLLLLLLL...",
    "..LLLLLLLLLLLL..",
    "..LLLLLLLLLLLL..",
    ".LLLLLLLLLLLLLL.",
    ".LLLLLLLLLLLLLL.",
    ".LLLLLLLLLLLLLL.",
    ".LLLLLLLLLLLLLL.",
    ".LLLLLLLLLLLLLL.",
    ".LLLLLLLLLLLLLL.",
    "..LLLLLLLLLLLL..",
    "..LLLLLLLLLLLL..",
    "...LLLLLLLLLL...",
    "....LLLLLLLL....",
    "......LLLL......",
];

const MOON_WANING_GIBBOUS: &[&str] = &[
    "......LLLL......",
    "....LLLLLLLL....",
    "...LLLLLLLLLD...",
    "..LLLLLLLLLLDD..",
    "..LLLLLLLLLLDD..",
    ".LLLLLLLLLLLDDD.",
    ".LLLLLLLLLLLDDD.",
    ".LLLLLLLLLLLDDD.",
    ".LLLLLLLLLLLDDD.",
    ".LLLLLLLLLLLDDD.",
    ".LLLLLLLLLLLDDD.",
    "..LLLLLLLLLLDD..",
    "..LLLLLLLLLLDD..",
    "...LLLLLLLLLD...",
    "....LLLLLLLL....",
    "......LLLL......",
];

const MOON_LAST_QUARTER: &[&str] = &[
    "......LLDD......",
    "....LLLLDDDD....",
    "...LLLLLDDDDD...",
    "..LLLLLLDDDDDD..",
    "..LLLLLLDDDDDD..",
    ".LLLLLLLDDDDDDD.",
    ".LLLLLLLDDDDDDD.",
    ".LLLLLLLDDDDDDD.",
    ".LLLLLLLDDDDDDD.",
    ".LLLLLLLDDDDDDD.",
    ".LLLLLLLDDDDDDD.",
    "..LLLLLLDDDDDD..",
    "..LLLLLLDDDDDD..",
    "...LLLLLDDDDD...",
    "....LLLLDDDD....",
    "......LLDD......",
];

const MOON_WANING_CRESCENT: &[&str] = &[
    "......DDDD......",
    "....DDDDDDDD....",
    "...LDDDDDDDDD...",
    "..LLDDDDDDDDDD..",
    "..LLDDDDDDDDDD..",
    ".LLLDDDDDDDDDDD.",
    ".LLLDDDDDDDDDDD.",
    ".LLLDDDDDDDDDDD.",
    ".LLLDDDDDDDDDDD.",
    ".LLLDDDDDDDDDDD.",
    ".LLLDDDDDDDDDDD.",
    "..LLDDDDDDDDDD..",
    "..LLDDDDDDDDDD..",
    "...LDDDDDDDDD...",
    "....DDDDDDDD....",
    "......DDDD......",
];

// ── Season sprites ─────────────────────────────────────────────────

const SPRING: &[&str] = &[
    "................",
    "................",
    "....PP....PP....",
    "...PpPP..PPpP...",
    "...PPPP..PPPP...",
    "....PPPYPPP.....",
    "....PPYYYPP.....",
    "....PPPYPPP.....",
    "...PPPP..PPPP...",
    "...PpPP..PPpP...",
    "....PP....PP....",
    ".......TT.......",
    ".......TT.......",
    "......GTTG......",
    ".....GGTTGG.....",
    "................",
];

const SUMMER: &[&str] = &[
    "................",
    "......YYYY......",
    ".....YYYYYY.....",
    "....YYOOOOYY....",
    "...YYOOOOOOYY...",
    "..YYOOOOOOOOYY..",
    ".YYOOOOOOOOOOYY.",
    ".YOOOOOOOOOOOOY.",
    ".YOOOOOOOOOOOOY.",
    ".YYOOOOOOOOOOYY.",
    "..YYOOOOOOOOYY..",
    "...YYOOOOOOYY...",
    "....YYOOOOYY....",
    ".....YYYYYY.....",
    "......YYYY......",
    "................",
];

const AUTUMN: &[&str] = &[
    "................",
    "......RRR.......",
    ".....RRRRR......",
    "...RRORRRORRR...",
    "..RROOOORRROORR.",
    "..ROOOROOORROOR.",
    "...ROORRRROORR..",
    "....RROORROOR...",
    "....RROORROOR...",
    "...RROORRRROORR.",
    "..ROOOROOORROOR.",
    "..RROOOORRROORR.",
    "...RRORRRORRR...",
    ".....RRRRR......",
    "......RTR.......",
    ".......T........",
];

const WINTER: &[&str] = &[
    "................",
    ".......S........",
    "...S...S...S....",
    "...SS.SBS.SS....",
    "....SSSBSSSS....",
    ".....SSBSSS.....",
    ".SSSSSSBSSSSSS..",
    ".....SSBSSS.....",
    ".SSSSSSBSSSSSS..",
    "....SSSBSSSS....",
    "...SS.SBS.SS....",
    "....SS.S.SS.....",
    "...S...S...S....",
    ".......S........",
    "................",
    "................",
];

