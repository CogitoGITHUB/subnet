//! `random_quote` — a bundled daily quote. Selection is deterministic per calendar day
//! (epoch days modulo the built-in quote count), so every `splashboard` invocation on
//! the same day sees the same line and the quote advances automatically at midnight.
//! Indexing by epoch days (not day-of-year) means the cycle drifts year over year, so
//! the same date doesn't always return the same quote. Zero-config, zero I/O, infallible.

use std::sync::LazyLock;

use chrono::{DateTime, Datelike, FixedOffset, Local};

use crate::fetcher::{FetchContext, RealtimeFetcher, Safety};
use crate::payload::{Body, Payload, TextBlockData, TextData};
use crate::render::Shape;
use crate::samples;

const SHAPES: &[Shape] = &[Shape::TextBlock, Shape::Text];

/// Quote / author pairs parsed from `data.txt`. Format: `%`-separated entries; each entry
/// is the quote text optionally followed by a final line starting with `— ` for the author.
/// Entries with no `—` line are treated as anonymous (author = "Anonymous").
static QUOTES: LazyLock<Vec<(String, String)>> = LazyLock::new(|| parse(include_str!("data.txt")));

pub struct RandomQuoteFetcher;

impl RealtimeFetcher for RandomQuoteFetcher {
    fn name(&self) -> &str {
        "random_quote"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Picks one quote-and-author pair from a bundled collection, advancing once per calendar day. Distinct from `random_fortune`, which serves anonymous Unix-style fortune cookies; this one always carries an attribution line."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        let (q, a) = first_quote()?;
        Some(match shape {
            Shape::Text => samples::text(&single_line(q, a)),
            Shape::TextBlock => samples::text_block(&[q, &format!("— {a}")]),
            _ => return None,
        })
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let now = now();
        let (quote, author) = pick(&now);
        let body = match ctx.shape.unwrap_or(Shape::TextBlock) {
            Shape::Text => Body::Text(TextData {
                value: single_line(quote, author),
            }),
            _ => Body::TextBlock(TextBlockData {
                lines: vec![quote.to_string(), format!("— {author}")],
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

/// Epoch-days modulo quote count — deterministic, rolls over at midnight, drifts through
/// the calendar year over year (unlike day-of-year, which would lock the same quote to
/// the same date every year).
fn pick(now: &DateTime<FixedOffset>) -> (&'static str, &'static str) {
    let entries: &'static [(String, String)] = &QUOTES;
    let idx = (now.num_days_from_ce() as usize) % entries.len();
    let (q, a) = &entries[idx];
    (q.as_str(), a.as_str())
}

fn first_quote() -> Option<(&'static str, &'static str)> {
    let entries: &'static [(String, String)] = &QUOTES;
    entries.first().map(|(q, a)| (q.as_str(), a.as_str()))
}

fn single_line(quote: &str, author: &str) -> String {
    format!("{quote} — {author}")
}

/// Parse `%`-separated entries. Each entry's last line starting with `— ` is the author;
/// everything before it (joined by spaces, trimmed) is the quote. Empty entries dropped.
/// Normalises CRLF → LF first so Windows checkouts (autocrlf) parse identically.
fn parse(raw: &str) -> Vec<(String, String)> {
    raw.replace("\r\n", "\n")
        .split("\n%\n")
        .filter_map(parse_entry)
        .collect()
}

fn parse_entry(chunk: &str) -> Option<(String, String)> {
    let trimmed = chunk.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lines: Vec<&str> = trimmed.lines().collect();
    let (author, quote_lines) = match lines.split_last() {
        Some((last, rest)) if last.trim_start().starts_with("— ") => (
            last.trim_start()
                .trim_start_matches("— ")
                .trim()
                .to_string(),
            rest,
        ),
        _ => ("Anonymous".to_string(), lines.as_slice()),
    };
    let quote = quote_lines
        .iter()
        .map(|s| s.trim())
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();
    if quote.is_empty() {
        None
    } else {
        Some((quote, author))
    }
}

