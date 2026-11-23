//! `code_largest_files` — ranks the top tracked files by line count or byte size. Headlines the
//! single biggest file in `Text`; multi-row shapes (`TextBlock` / `Entries` / `Bars` /
//! `MarkdownTextBlock` / `NumberSeries`) carry the per-file ranking. `Badge` flags whether the
//! repo is harbouring a refactor-candidate-sized file (separate health signal from `code_loc`'s
//! repo-total tier).

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Bar, BarsData, Body, EntriesData, Entry, MarkdownTextBlockData, NumberSeriesData,
    Payload, Status, TextBlockData, TextData,
};
use crate::render::Shape;
use crate::samples;

use super::super::git::{open_repo, payload, repo_cache_key};
use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::scan::for_each_tracked_file;

const SHAPES: &[Shape] = &[
    Shape::Text,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::Entries,
    Shape::Bars,
    Shape::NumberSeries,
    Shape::Badge,
];
const DEFAULT_LIMIT: usize = 10;

// Refactor-candidate tiers for `Badge`. Boundaries chosen for the LOC metric (single-file
// review effort): under ~200 lines is comfortable, ~500 starts to feel heavy, ~1000+ is the
// "split this up" zone. Bytes metric reuses the same tier names with KiB-scaled thresholds —
// the qualitative read ("is anything in this repo too big?") stays consistent.
const TIER_LOC_TIDY: u64 = 200;
const TIER_LOC_BIG: u64 = 500;
const TIER_LOC_BLOATED: u64 = 1_000;
const TIER_BYTES_TIDY: u64 = 16 * 1024;
const TIER_BYTES_BIG: u64 = 64 * 1024;
const TIER_BYTES_BLOATED: u64 = 256 * 1024;

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "metric",
        type_hint: "`loc` | `bytes`",
        required: false,
        default: Some("loc"),
        description: "What to rank files by: `loc` (text line count, binaries skipped) or `bytes` (raw file size). LOC suits refactor-candidate hunting; bytes suits asset-weight audits.",
    },
    OptionSchema {
        name: "limit",
        type_hint: "integer",
        required: false,
        default: Some("10"),
        description: "Cap on ranked rows (`TextBlock` / `MarkdownTextBlock` / `Entries` / `Bars` / `NumberSeries`). The `Text` headline always names just the single largest file.",
    },
];

/// Ranks the largest tracked source files in the discovered git repo. Walks the `gix` index
/// via the shared family helper, so untracked / `.gitignore`-d files plus committed-vendored
/// trees, lockfiles, and binaries are skipped automatically.
pub struct CodeLargestFiles;

#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    metric: Option<Metric>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Clone, Copy, Default, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Metric {
    #[default]
    Loc,
    Bytes,
}

#[async_trait]
impl Fetcher for CodeLargestFiles {
    fn name(&self) -> &str {
        "code_largest_files"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Ranks the largest tracked source files in the discovered git repo. `metric = \"loc\"` (default) counts text lines per file; `metric = \"bytes\"` measures raw size. `Text` headlines the single biggest file; `TextBlock` / `MarkdownTextBlock` / `Entries` / `Bars` carry the per-file ranking; `NumberSeries` sketches the size distribution; `Badge` flags whether the repo holds a refactor-candidate-sized file (`tidy` / `big` / `bloated` / `monster`). Vendored / generated dirs, lockfiles, and binaries are skipped via the shared scan helper."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 10
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn default_shape(&self) -> Shape {
        Shape::Entries
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
            Shape::Text => samples::text("src/render/mod.rs (1,234 LOC)"),
            Shape::TextBlock => samples::text_block(&[
                "src/render/mod.rs (1,234)",
                "src/fetcher/git/mod.rs (812)",
                "src/payload.rs (640)",
            ]),
            Shape::MarkdownTextBlock => samples::markdown(
                "- **src/render/mod.rs** — 1,234 LOC\n- **src/fetcher/git/mod.rs** — 812 LOC\n- **src/payload.rs** — 640 LOC",
            ),
            Shape::Entries => samples::entries(&[
                ("src/render/mod.rs", "1,234"),
                ("src/fetcher/git/mod.rs", "812"),
                ("src/payload.rs", "640"),
            ]),
            Shape::Bars => samples::bars(&[
                ("src/render/mod.rs", 1234),
                ("src/fetcher/git/mod.rs", 812),
                ("src/payload.rs", 640),
            ]),
            Shape::NumberSeries => samples::number_series(&[1234, 812, 640, 420, 280]),
            Shape::Badge => samples::badge(Status::Warn, "bloated · src/render/mod.rs (1,234)"),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref())?;
        let metric = opts.metric.unwrap_or_default();
        let limit = opts.limit.filter(|n| *n > 0).unwrap_or(DEFAULT_LIMIT);
        let scan = scan_cwd(metric)?;
        Ok(payload(render_body(
            scan,
            ctx.shape.unwrap_or(Shape::Entries),
            metric,
            limit,
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
struct Scan {
    /// `(path, measure)` sorted by measure desc, then path asc.
    by_file: Vec<(String, u64)>,
}

fn scan_cwd(metric: Metric) -> Result<Scan, FetchError> {
    let repo = open_repo()?;
    scan_repo(&repo, metric)
}

fn scan_repo(repo: &gix::Repository, metric: Metric) -> Result<Scan, FetchError> {
    let mut by_file: Vec<(String, u64)> = Vec::new();
    for_each_tracked_file(repo, |path, bytes| {
        let measure = measure_file(metric, bytes);
        if measure == 0 {
            return;
        }
        by_file.push((path.to_string(), measure));
    })?;
    by_file.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Ok(Scan { by_file })
}

fn measure_file(metric: Metric, bytes: &[u8]) -> u64 {
    match metric {
        Metric::Bytes => bytes.len() as u64,
        Metric::Loc => match std::str::from_utf8(bytes) {
            Ok(text) => text.lines().count() as u64,
            Err(_) => 0,
        },
    }
}

fn render_body(scan: Scan, shape: Shape, metric: Metric, limit: usize) -> Body {
    if scan.by_file.is_empty() {
        return empty_body(shape);
    }
    match shape {
        Shape::Text => text_body(&scan, metric),
        Shape::TextBlock => text_block_body(&scan, limit),
        Shape::MarkdownTextBlock => markdown_body(&scan, metric, limit),
        Shape::Entries => entries_body(&scan, limit),
        Shape::Bars => bars_body(scan, limit),
        Shape::NumberSeries => number_series_body(&scan, limit),
        Shape::Badge => badge_body(&scan, metric),
        _ => text_body(&scan, metric),
    }
}

fn empty_body(shape: Shape) -> Body {
    match shape {
        Shape::TextBlock => Body::TextBlock(TextBlockData { lines: vec![] }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: String::new(),
        }),
        Shape::Entries => Body::Entries(EntriesData { items: vec![] }),
        Shape::Bars => Body::Bars(BarsData { bars: vec![] }),
        Shape::NumberSeries => Body::NumberSeries(NumberSeriesData { values: vec![] }),
        Shape::Badge => Body::Badge(BadgeData {
            status: Status::Ok,
            label: "empty".into(),
        }),
        _ => Body::Text(TextData {
            value: String::new(),
        }),
    }
}

fn text_body(scan: &Scan, metric: Metric) -> Body {
    let (path, measure) = scan.by_file.first().expect("non-empty scan");
    Body::Text(TextData {
        value: format!("{path} ({})", format_measure(*measure, metric)),
    })
}

fn text_block_body(scan: &Scan, limit: usize) -> Body {
    Body::TextBlock(TextBlockData {
        lines: scan
            .by_file
            .iter()
            .take(limit)
            .map(|(path, n)| format!("{path} ({})", format_with_commas(*n)))
            .collect(),
    })
}

fn markdown_body(scan: &Scan, metric: Metric, limit: usize) -> Body {
    Body::MarkdownTextBlock(MarkdownTextBlockData {
        value: scan
            .by_file
            .iter()
            .take(limit)
            .map(|(path, n)| format!("- **{path}** — {}", format_measure(*n, metric)))
            .collect::<Vec<_>>()
            .join("\n"),
    })
}

fn entries_body(scan: &Scan, limit: usize) -> Body {
    Body::Entries(EntriesData {
        items: scan
            .by_file
            .iter()
            .take(limit)
            .map(|(path, n)| Entry {
                key: path.clone(),
                value: Some(format_with_commas(*n)),
                status: None,
            })
            .collect(),
    })
}

fn bars_body(scan: Scan, limit: usize) -> Body {
    Body::Bars(BarsData {
        bars: scan
            .by_file
            .into_iter()
            .take(limit)
            .map(|(path, n)| Bar {
                label: path,
                value: n,
                value_label: None,
            })
            .collect(),
    })
}

fn number_series_body(scan: &Scan, limit: usize) -> Body {
    Body::NumberSeries(NumberSeriesData {
        values: scan.by_file.iter().take(limit).map(|(_, n)| *n).collect(),
    })
}

fn badge_body(scan: &Scan, metric: Metric) -> Body {
    let (path, measure) = scan.by_file.first().expect("non-empty scan");
    let (tier, status) = tier_for(metric, *measure);
    Body::Badge(BadgeData {
        status,
        label: format!("{tier} · {path} ({})", format_with_commas(*measure)),
    })
}

fn tier_for(metric: Metric, measure: u64) -> (&'static str, Status) {
    let (tidy, big, bloated) = match metric {
        Metric::Loc => (TIER_LOC_TIDY, TIER_LOC_BIG, TIER_LOC_BLOATED),
        Metric::Bytes => (TIER_BYTES_TIDY, TIER_BYTES_BIG, TIER_BYTES_BLOATED),
    };
    if measure < tidy {
        ("tidy", Status::Ok)
    } else if measure < big {
        ("big", Status::Ok)
    } else if measure < bloated {
        ("bloated", Status::Warn)
    } else {
        ("monster", Status::Error)
    }
}

fn format_measure(n: u64, metric: Metric) -> String {
    let suffix = match metric {
        Metric::Loc => "LOC",
        Metric::Bytes => "bytes",
    };
    format!("{} {suffix}", format_with_commas(n))
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

