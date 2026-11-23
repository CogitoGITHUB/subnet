use std::collections::HashMap;

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Bar, BarsData, Body, EntriesData, Entry, MarkdownTextBlockData, NumberSeriesData,
    Payload, RatioData, Status, TextBlockData, TextData,
};
use crate::render::Shape;
use crate::samples;

use super::super::git::{open_repo, payload, repo_cache_key};
use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::scan::{classify_with_lang, for_each_tracked_file};

const SHAPES: &[Shape] = &[
    Shape::Text,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::Entries,
    Shape::Bars,
    Shape::Ratio,
    Shape::NumberSeries,
    Shape::Badge,
];
const DEFAULT_LIMIT: usize = 10;
const OTHER_LABEL: &str = "Other";

// Order-of-magnitude tiers for the `Badge` shape. Boundaries are powers of 10 so the labels
// match common engineering shorthand ("10k LOC repo" / "100k LOC monolith"). Status mapping
// stays neutral up through `medium` because most healthy app repos sit there; `large` /
// `huge` shade toward warn / error to flag "this is a heavy codebase, scope changes
// accordingly". Users wanting different semantics can render via `status_badge` with their
// own tone overrides.
const TIER_SMALL: u64 = 1_000;
const TIER_MEDIUM: u64 = 10_000;
const TIER_LARGE: u64 = 100_000;
const TIER_HUGE: u64 = 1_000_000;

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "limit",
        type_hint: "integer",
        required: false,
        default: Some("10"),
        description: "Cap on rendered languages (`TextBlock` / `MarkdownTextBlock` / `Entries` / `Bars` / `NumberSeries`). The `Text` summary always reports the full count.",
    },
    OptionSchema {
        name: "unit",
        type_hint: "`loc` | `kloc` | `percent` (alias `%`)",
        required: false,
        default: Some("loc"),
        description: "Display format for counts: `loc` (`1,234`), `kloc` (`1.2k`), or `percent` (`67.8%`). Percent falls back to `loc` for the `Text` summary and `kloc` for `Badge` (where a percentage of self is meaningless). `Ratio` and `NumberSeries` ignore this option (Ratio is always 0..=1; NumberSeries renderers normalise visually).",
    },
];

/// Counts lines per language across tracked source files in the discovered git repo.
/// Languages are inferred from extension / bare-filename map; unknown files bucket into
/// `"Other"`. Walks the `gix` index so untracked / `.gitignore`-d files plus committed-vendored
/// trees and lockfiles are skipped automatically.
///
/// Eight shapes from one read:
/// - `Text`: `"12,345 lines across 6 languages"` summary.
/// - `TextBlock`: aligned `"Language       count"` rows.
/// - `MarkdownTextBlock`: `"- **Rust** 8,234"` markdown list, for `text_markdown`.
/// - `Entries` / `Bars`: rank languages by count for tabular / chart renderers.
/// - `Ratio`: primary language's share of the total (`0..=1`), for gauges.
/// - `NumberSeries`: sorted-desc sequence of per-language counts, for sparklines / heatmaps.
/// - `Badge`: order-of-magnitude tier (`toy` / `small` / `medium` / `large` / `huge`).
pub struct CodeLoc;

#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    unit: Option<Unit>,
}

#[derive(Debug, Clone, Copy, Default, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Unit {
    #[default]
    Loc,
    Kloc,
    #[serde(alias = "%")]
    Percent,
}

#[async_trait]
impl Fetcher for CodeLoc {
    fn name(&self) -> &str {
        "code_loc"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Counts lines per language across tracked source files in the discovered git repo (extension-based classification, vendored / generated dirs and lockfiles skipped). `Text` summarises totals; `TextBlock` / `MarkdownTextBlock` / `Entries` / `Bars` rank languages; `Ratio` exposes the primary language's share; `NumberSeries` sketches the distribution; `Badge` tiers the codebase by size. The `unit` option toggles raw / kloc / percent display."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 10
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn default_shape(&self) -> Shape {
        Shape::Text
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        // Mix `[widget.options]` into the key — both `limit` and `unit` change the rendered
        // output, so two widgets with different options must not share a cache slot.
        let base = repo_cache_key(self.name(), ctx);
        let opts = ctx
            .options
            .as_ref()
            .map(toml::Value::to_string)
            .unwrap_or_default();
        if opts.is_empty() {
            return base;
        }
        let digest = Sha256::digest(opts.as_bytes());
        let hex: String = digest.iter().take(4).map(|b| format!("{b:02x}")).collect();
        format!("{base}-{hex}")
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Text => samples::text("12,345 lines across 6 languages"),
            Shape::TextBlock => samples::text_block(&[
                "Rust         8,234",
                "TypeScript   2,111",
                "Markdown       820",
                "TOML           560",
            ]),
            Shape::MarkdownTextBlock => samples::markdown(
                "- **Rust** 8,234\n- **TypeScript** 2,111\n- **Markdown** 820\n- **TOML** 560",
            ),
            Shape::Entries => samples::entries(&[
                ("Rust", "8,234"),
                ("TypeScript", "2,111"),
                ("Markdown", "820"),
                ("TOML", "560"),
            ]),
            Shape::Bars => samples::bars(&[
                ("Rust", 8234),
                ("TypeScript", 2111),
                ("Markdown", 820),
                ("TOML", 560),
            ]),
            Shape::Ratio => samples::ratio(0.70, "Rust"),
            Shape::NumberSeries => samples::number_series(&[8234, 2111, 820, 560, 234, 56]),
            Shape::Badge => samples::badge(Status::Ok, "medium · 12k"),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref())?;
        let limit = opts.limit.filter(|n| *n > 0).unwrap_or(DEFAULT_LIMIT);
        let unit = opts.unit.unwrap_or_default();
        let totals = scan_cwd()?;
        Ok(payload(render_body(
            totals,
            ctx.shape.unwrap_or(Shape::Text),
            limit,
            unit,
        )))
    }
}

fn parse_options(raw: Option<&toml::Value>) -> Result<Options, FetchError> {
    match raw {
        None => Ok(Options::default()),
        Some(value) => value
            .clone()
            .try_into::<Options>()
            .map_err(|e| FetchError::Failed(format!("invalid options: {e}"))),
    }
}

#[derive(Debug, Default)]
struct Totals {
    /// Sorted by line count desc, then language name asc.
    by_language: Vec<(String, u64)>,
    total_lines: u64,
}

fn scan_cwd() -> Result<Totals, FetchError> {
    let repo = open_repo()?;
    scan_repo(&repo)
}

fn scan_repo(repo: &gix::Repository) -> Result<Totals, FetchError> {
    let cfg = tokei::Config::default();
    let mut counts: HashMap<&'static str, u64> = HashMap::new();
    let mut other_lines: u64 = 0;
    let mut total: u64 = 0;
    for_each_tracked_file(repo, |path, bytes| match classify_with_lang(path, &cfg) {
        Some((name, lang)) => {
            let stats = lang.parse_from_slice(bytes, &cfg);
            let lines = (stats.code + stats.comments + stats.blanks) as u64;
            if lines == 0 {
                return;
            }
            total += lines;
            *counts.entry(name).or_insert(0) += lines;
        }
        None => {
            let Ok(text) = std::str::from_utf8(bytes) else {
                return;
            };
            let lines = text.lines().count() as u64;
            if lines == 0 {
                return;
            }
            total += lines;
            other_lines += lines;
        }
    })?;
    let mut by_language: Vec<(String, u64)> = counts
        .into_iter()
        .map(|(lang, n)| (lang.to_string(), n))
        .collect();
    if other_lines > 0 {
        by_language.push((OTHER_LABEL.to_string(), other_lines));
    }
    by_language.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Ok(Totals {
        by_language,
        total_lines: total,
    })
}

fn render_body(totals: Totals, shape: Shape, limit: usize, unit: Unit) -> Body {
    let total = totals.total_lines;
    match shape {
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: totals
                .by_language
                .into_iter()
                .take(limit)
                .map(|(lang, n)| format!("{:<14} {}", lang, format_count(n, total, unit)))
                .collect(),
        }),
        Shape::MarkdownTextBlock => render_markdown(totals, limit, unit),
        Shape::Entries => Body::Entries(EntriesData {
            items: totals
                .by_language
                .into_iter()
                .take(limit)
                .map(|(lang, n)| Entry {
                    key: lang,
                    value: Some(format_count(n, total, unit)),
                    status: None,
                })
                .collect(),
        }),
        Shape::Bars => Body::Bars(BarsData {
            bars: totals
                .by_language
                .into_iter()
                .take(limit)
                .map(|(lang, n)| Bar {
                    label: lang,
                    value: n,
                    value_label: None,
                })
                .collect(),
        }),
        Shape::Ratio => render_ratio(&totals),
        Shape::NumberSeries => render_number_series(&totals, limit),
        Shape::Badge => render_badge(total, unit),
        _ => text_summary(total, totals.by_language.len(), unit),
    }
}

fn render_markdown(totals: Totals, limit: usize, unit: Unit) -> Body {
    let total = totals.total_lines;
    let value = totals
        .by_language
        .into_iter()
        .take(limit)
        .map(|(lang, n)| format!("- **{lang}** {}", format_count(n, total, unit)))
        .collect::<Vec<_>>()
        .join("\n");
    Body::MarkdownTextBlock(MarkdownTextBlockData { value })
}

fn render_ratio(totals: &Totals) -> Body {
    let (label, value, denom) = match totals.by_language.first() {
        Some((lang, n)) if totals.total_lines > 0 => {
            let v = (*n as f64 / totals.total_lines as f64).clamp(0.0, 1.0);
            (Some(lang.clone()), v, Some(totals.total_lines))
        }
        _ => (None, 0.0, None),
    };
    Body::Ratio(RatioData {
        value,
        label,
        denominator: denom,
    })
}

fn render_number_series(totals: &Totals, limit: usize) -> Body {
    Body::NumberSeries(NumberSeriesData {
        values: totals
            .by_language
            .iter()
            .take(limit)
            .map(|(_, n)| *n)
            .collect(),
    })
}

fn render_badge(total: u64, unit: Unit) -> Body {
    if total == 0 {
        return Body::Badge(BadgeData {
            status: Status::Ok,
            label: "empty".into(),
        });
    }
    let (tier, status) = tier_for(total);
    // Percent of self is always 100% — meaningless on Badge. Fall back to kloc so the
    // count still reads compactly inside the pill.
    let display_unit = if unit == Unit::Percent {
        Unit::Kloc
    } else {
        unit
    };
    Body::Badge(BadgeData {
        status,
        label: format!("{tier} · {}", format_count(total, total, display_unit)),
    })
}

fn tier_for(total: u64) -> (&'static str, Status) {
    if total < TIER_SMALL {
        ("toy", Status::Ok)
    } else if total < TIER_MEDIUM {
        ("small", Status::Ok)
    } else if total < TIER_LARGE {
        ("medium", Status::Ok)
    } else if total < TIER_HUGE {
        ("large", Status::Warn)
    } else {
        ("huge", Status::Error)
    }
}

fn text_summary(total: u64, langs: usize, unit: Unit) -> Body {
    let value = if total == 0 {
        String::new()
    } else {
        // "X% lines across N languages" doesn't parse — fall back to raw count.
        let display_unit = if unit == Unit::Percent {
            Unit::Loc
        } else {
            unit
        };
        let count_str = format_count(total, total, display_unit);
        format!(
            "{count_str} line{} across {langs} language{}",
            if total == 1 { "" } else { "s" },
            if langs == 1 { "" } else { "s" },
        )
    };
    Body::Text(TextData { value })
}

fn format_count(n: u64, total: u64, unit: Unit) -> String {
    match unit {
        Unit::Loc => format_with_commas(n),
        Unit::Kloc => format_kloc(n),
        Unit::Percent => format_percent(n, total),
    }
}

fn format_with_commas(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

fn format_kloc(n: u64) -> String {
    if n < 1_000 {
        n.to_string()
    } else if n < 1_000_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    }
}

fn format_percent(n: u64, total: u64) -> String {
    if total == 0 {
        "0.0%".into()
    } else {
        format!("{:.1}%", (n as f64 / total as f64) * 100.0)
    }
}

