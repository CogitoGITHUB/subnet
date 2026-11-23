//! `random_fortune` — bundled Unix-style fortune cookies. One entry per day, deterministic
//! by epoch days so the cookie advances at midnight and drifts through the calendar year
//! over year. Distinct from `random_quote`: cookies are anonymous, irreverent, and may
//! span multiple lines (haikus, short verses, multi-line aphorisms). Zero-config, zero
//! I/O, infallible.

use std::sync::LazyLock;

use chrono::{DateTime, Datelike, FixedOffset, Local};

use crate::fetcher::{FetchContext, RealtimeFetcher, Safety};
use crate::payload::{Body, Payload, TextBlockData, TextData};
use crate::render::Shape;
use crate::samples;

const SHAPES: &[Shape] = &[Shape::TextBlock, Shape::Text];

/// Cookie strings parsed from `data.txt`. Format: `%`-separated entries; entries may be
/// single-line one-liners or multi-line short verses. Newlines inside an entry are
/// preserved verbatim for `TextBlock` (one row per line); `Text` collapses them with
/// ` · ` separators to honour the single-string contract.
static COOKIES: LazyLock<Vec<String>> = LazyLock::new(|| parse(include_str!("data.txt")));

pub struct RandomFortuneFetcher;

impl RealtimeFetcher for RandomFortuneFetcher {
    fn name(&self) -> &str {
        "random_fortune"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Picks one bundled Unix-style fortune cookie, advancing once per calendar day. Distinct from `random_quote`: cookies are anonymous and irreverent and may span multiple lines (haikus, short verses), with no attribution line."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        let cookie = first_cookie()?;
        Some(match shape {
            Shape::Text => samples::text(&single_line(cookie)),
            Shape::TextBlock => samples::text_block(&cookie.lines().collect::<Vec<_>>()),
            _ => return None,
        })
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let cookie = pick(&now());
        let body = match ctx.shape.unwrap_or(Shape::TextBlock) {
            Shape::Text => Body::Text(TextData {
                value: single_line(cookie),
            }),
            _ => Body::TextBlock(TextBlockData {
                lines: cookie.lines().map(str::to_string).collect(),
            }),
        };
        Payload {
            icon: None,
            status: None,
            format: None,
            body,
        }
    }
}

fn now() -> DateTime<FixedOffset> {
    Local::now().fixed_offset()
}

/// Epoch-days modulo cookie count. Deterministic per calendar day; drifts year over year.
fn pick(now: &DateTime<FixedOffset>) -> &'static str {
    let entries: &'static [String] = &COOKIES;
    let idx = (now.num_days_from_ce() as usize) % entries.len();
    entries[idx].as_str()
}

fn first_cookie() -> Option<&'static str> {
    let entries: &'static [String] = &COOKIES;
    entries.first().map(String::as_str)
}

fn single_line(cookie: &str) -> String {
    cookie.lines().collect::<Vec<_>>().join(" · ")
}

/// Parse `%`-separated entries. Trims surrounding whitespace per entry, drops empties.
/// Internal newlines are preserved so multi-line cookies render as multiple lines.
/// Normalises CRLF → LF first so Windows checkouts (autocrlf) parse identically.
fn parse(raw: &str) -> Vec<String> {
    raw.replace("\r\n", "\n")
        .split("\n%\n")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

