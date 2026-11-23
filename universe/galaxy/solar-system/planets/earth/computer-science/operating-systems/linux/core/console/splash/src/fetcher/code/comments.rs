//! `code_comments` — comment density per language across tracked source files. Pairs with
//! `code_loc` (which counts physical lines) to surface the documentation posture of the
//! codebase: "Rust 18% comments / TypeScript 4%" tells you which language family carries
//! the team's writing.

use std::collections::HashMap;

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Bar, BarsData, Body, EntriesData, Entry, MarkdownTextBlockData, Payload, RatioData,
    Status, TextBlockData, TextData,
};
use crate::render::Shape;
use crate::samples;

use super::super::git::{open_repo, payload, repo_cache_key};
use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::scan::for_each_tokei_stat;

const SHAPES: &[Shape] = &[
    Shape::Text,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::Entries,
    Shape::Bars,
    Shape::Ratio,
    Shape::Badge,
];
const DEFAULT_LIMIT: usize = 10;

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "limit",
        type_hint: "integer",
        required: false,
        default: Some("10"),
        description: "Cap on rendered languages (`TextBlock` / `MarkdownTextBlock` / `Entries` / `Bars`). The `Text` summary always reports the whole-repo ratio.",
    },
    OptionSchema {
        name: "unit",
        type_hint: "`percent` (alias `%`) | `loc` | `kloc`",
        required: false,
        default: Some("percent"),
        description: "Display format for per-language values: `percent` (`18.3%` of that language's `code+comments`), `loc` (raw comment lines, `1,234`), or `kloc` (`1.2k`). `Ratio` always emits the whole-repo comment share regardless of this option; `Badge` always reports tier + percent.",
    },
];

pub struct CodeComments;

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
    #[serde(alias = "%")]
    Percent,
    Loc,
    Kloc,
}

#[async_trait]
impl Fetcher for CodeComments {
    fn name(&self) -> &str {
        "code_comments"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Comment-density per language across tracked source files in the discovered git repo. tokei parses each file into `code` / `comments` / `blanks`; this fetcher surfaces the comment share. `Text` headlines the whole-repo ratio; `TextBlock` / `MarkdownTextBlock` / `Entries` / `Bars` rank per-language values (default `percent`, override with `unit = loc | kloc`); `Ratio` exposes the whole-repo share for gauges; `Badge` tiers documentation posture (`undocumented` / `light` / `balanced` / `documented` / `verbose`)."
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
            Shape::Text => samples::text("18.3% comments · 3,456 / 18,892 lines"),
            Shape::TextBlock => samples::text_block(&[
                "Rust         24.1%",
                "Markdown     58.0%",
                "TypeScript    9.4%",
                "TOML          4.2%",
            ]),
            Shape::MarkdownTextBlock => samples::markdown(
                "- **Rust** 24.1%\n- **Markdown** 58.0%\n- **TypeScript** 9.4%\n- **TOML** 4.2%",
            ),
            Shape::Entries => samples::entries(&[
                ("Rust", "24.1%"),
                ("Markdown", "58.0%"),
                ("TypeScript", "9.4%"),
                ("TOML", "4.2%"),
            ]),
            Shape::Bars => samples::bars(&[
                ("Rust", 1900),
                ("Markdown", 580),
                ("TypeScript", 230),
                ("TOML", 32),
            ]),
            Shape::Ratio => samples::ratio(0.183, "comments"),
            Shape::Badge => samples::badge(Status::Ok, "balanced · 18%"),
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

#[derive(Debug, Default, Clone)]
struct LangStat {
    code: u64,
    comments: u64,
}

#[derive(Debug, Default)]
struct Totals {
    /// Sorted by comment ratio desc (then by name asc), languages with zero `code+comments` dropped.
    by_language: Vec<(String, LangStat)>,
    total_code: u64,
    total_comments: u64,
}

fn scan_cwd() -> Result<Totals, FetchError> {
    let repo = open_repo()?;
    scan_repo(&repo)
}

fn scan_repo(repo: &gix::Repository) -> Result<Totals, FetchError> {
    let mut by: HashMap<&'static str, LangStat> = HashMap::new();
    let mut total_code = 0u64;
    let mut total_comments = 0u64;
    for_each_tokei_stat(repo, |_path, name, stats| {
        // Prose languages (Markdown / MDX / Plain Text / AsciiDoc) report every line as
        // `comments` and zero `code`, which makes their comment-density ratio collapse to
        // 100% and dominates the per-language ranking. They aren't documenting code, so
        // they don't belong in a code-comment-density metric. Filtering on `code > 0`
        // also drops empty / header-only files that would distort small-language ratios.
        if stats.code == 0 {
            return;
        }
        let entry = by.entry(name).or_default();
        entry.code += stats.code as u64;
        entry.comments += stats.comments as u64;
        total_code += stats.code as u64;
        total_comments += stats.comments as u64;
    })?;
    let mut by_language: Vec<(String, LangStat)> = by
        .into_iter()
        .filter(|(_, s)| s.code + s.comments > 0)
        .map(|(name, stat)| (name.to_string(), stat))
        .collect();
    by_language.sort_by(|a, b| {
        ratio_of(&b.1)
            .partial_cmp(&ratio_of(&a.1))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    Ok(Totals {
        by_language,
        total_code,
        total_comments,
    })
}

fn ratio_of(stat: &LangStat) -> f64 {
    let denom = stat.code + stat.comments;
    if denom == 0 {
        0.0
    } else {
        stat.comments as f64 / denom as f64
    }
}

fn render_body(totals: Totals, shape: Shape, limit: usize, unit: Unit) -> Body {
    match shape {
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: totals
                .by_language
                .iter()
                .take(limit)
                .map(|(name, stat)| format!("{:<12} {}", name, format_value(stat, unit)))
                .collect(),
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: totals
                .by_language
                .iter()
                .take(limit)
                .map(|(name, stat)| format!("- **{name}** {}", format_value(stat, unit)))
                .collect::<Vec<_>>()
                .join("\n"),
        }),
        Shape::Entries => Body::Entries(EntriesData {
            items: totals
                .by_language
                .iter()
                .take(limit)
                .map(|(name, stat)| Entry {
                    key: name.clone(),
                    value: Some(format_value(stat, unit)),
                    status: None,
                })
                .collect(),
        }),
        Shape::Bars => Body::Bars(BarsData {
            bars: totals
                .by_language
                .iter()
                .take(limit)
                .map(|(name, stat)| Bar {
                    label: name.clone(),
                    value: bar_value(stat, unit),
                    value_label: None,
                })
                .collect(),
        }),
        Shape::Ratio => render_ratio(&totals),
        Shape::Badge => render_badge(&totals),
        _ => render_text(&totals),
    }
}

fn render_text(totals: &Totals) -> Body {
    let denom = totals.total_code + totals.total_comments;
    if denom == 0 {
        return Body::Text(TextData {
            value: String::new(),
        });
    }
    let pct = (totals.total_comments as f64 / denom as f64) * 100.0;
    Body::Text(TextData {
        value: format!(
            "{:.1}% comments · {} / {} lines",
            pct,
            format_with_commas(totals.total_comments),
            format_with_commas(denom),
        ),
    })
}

fn render_ratio(totals: &Totals) -> Body {
    let denom = totals.total_code + totals.total_comments;
    let (value, denominator, label) = if denom == 0 {
        (0.0, None, None)
    } else {
        (
            (totals.total_comments as f64 / denom as f64).clamp(0.0, 1.0),
            Some(denom),
            Some("comments".into()),
        )
    };
    Body::Ratio(RatioData {
        value,
        label,
        denominator,
    })
}

fn render_badge(totals: &Totals) -> Body {
    let denom = totals.total_code + totals.total_comments;
    if denom == 0 {
        return Body::Badge(BadgeData {
            status: Status::Ok,
            label: "empty".into(),
        });
    }
    let pct = (totals.total_comments as f64 / denom as f64) * 100.0;
    let (tier, status) = tier_for(pct);
    Body::Badge(BadgeData {
        status,
        label: format!("{tier} · {pct:.0}%"),
    })
}

fn tier_for(pct: f64) -> (&'static str, Status) {
    if pct < 5.0 {
        ("undocumented", Status::Warn)
    } else if pct < 15.0 {
        ("light", Status::Ok)
    } else if pct < 30.0 {
        ("balanced", Status::Ok)
    } else if pct < 50.0 {
        ("documented", Status::Ok)
    } else {
        ("verbose", Status::Warn)
    }
}

fn format_value(stat: &LangStat, unit: Unit) -> String {
    match unit {
        Unit::Percent => format!("{:.1}%", ratio_of(stat) * 100.0),
        Unit::Loc => format_with_commas(stat.comments),
        Unit::Kloc => format_kloc(stat.comments),
    }
}

fn bar_value(stat: &LangStat, unit: Unit) -> u64 {
    match unit {
        // Percent shape on Bars: scale to basis points (×10) so renderers using integer
        // heights still differentiate "8.3%" from "8.7%".
        Unit::Percent => (ratio_of(stat) * 1000.0).round() as u64,
        Unit::Loc | Unit::Kloc => stat.comments,
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

