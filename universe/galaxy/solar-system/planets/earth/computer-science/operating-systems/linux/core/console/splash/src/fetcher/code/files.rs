//! `code_files` — counts tracked files in the discovered git repo's index and ranks the
//! top-level directories that hold them. `Text` is the headline file count;
//! `TextBlock` / `Entries` / `Bars` / `MarkdownTextBlock` all carry the per-top-level-dir
//! breakdown. Vendored / generated trees are skipped via the family-wide exclusion list.

use std::collections::HashMap;

use async_trait::async_trait;

use crate::payload::{
    Bar, BarsData, Body, EntriesData, Entry, MarkdownTextBlockData, Payload, TextBlockData,
    TextData,
};
use crate::render::Shape;
use crate::samples;

use super::super::git::{open_repo, payload, repo_cache_key};
use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::scan::for_each_tracked_path;

const SHAPES: &[Shape] = &[
    Shape::Text,
    Shape::TextBlock,
    Shape::Entries,
    Shape::Bars,
    Shape::MarkdownTextBlock,
];
const TOP_LEVEL_LIMIT: usize = 10;
const ROOT_LABEL: &str = "(root)";

pub struct CodeFiles;

#[async_trait]
impl Fetcher for CodeFiles {
    fn name(&self) -> &str {
        "code_files"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Counts tracked files in the discovered git repo's index and ranks top-level directories by file count. `Text` is the headline file count (`\"342 files\"`); `TextBlock` lists `\"<dir> (<count>)\"` rows; `Entries` exposes the same as key/value rows; `Bars` ranks them as bar values; `MarkdownTextBlock` formats them as a Markdown bullet list with emphasis. Files at the repo root are grouped under `\"(root)\"`. Vendored / generated dirs (`node_modules` / `vendor` / `target` / `dist` / `build`) are filtered out even when committed."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 5
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn default_shape(&self) -> Shape {
        Shape::Text
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        repo_cache_key(self.name(), ctx)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Text => samples::text("342 files"),
            Shape::TextBlock => {
                samples::text_block(&["src (180)", "tests (42)", "docs (18)", "(root) (5)"])
            }
            Shape::Entries => samples::entries(&[
                ("src", "180"),
                ("tests", "42"),
                ("docs", "18"),
                ("(root)", "5"),
            ]),
            Shape::Bars => samples::bars(&[("src", 180), ("tests", 42), ("docs", 18)]),
            Shape::MarkdownTextBlock => samples::markdown(
                "- **src** — 180 files\n- **tests** — 42 files\n- **docs** — 18 files",
            ),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let scan = scan_cwd()?;
        Ok(payload(render_body(scan, ctx.shape.unwrap_or(Shape::Text))))
    }
}

#[derive(Debug, Default)]
struct Scan {
    file_count: usize,
    top_level: Vec<(String, u64)>,
}

fn scan_cwd() -> Result<Scan, FetchError> {
    let repo = open_repo()?;
    scan_repo(&repo)
}

fn scan_repo(repo: &gix::Repository) -> Result<Scan, FetchError> {
    let mut file_count = 0usize;
    let mut top_counts: HashMap<String, u64> = HashMap::new();
    for_each_tracked_path(repo, |path| {
        file_count += 1;
        *top_counts.entry(top_level_key(path)).or_insert(0) += 1;
    })?;
    let mut top_level: Vec<_> = top_counts.into_iter().collect();
    top_level.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Ok(Scan {
        file_count,
        top_level,
    })
}

fn top_level_key(path: &str) -> String {
    match path.split_once('/') {
        Some((head, _)) => head.to_string(),
        None => ROOT_LABEL.to_string(),
    }
}

fn render_body(scan: Scan, shape: Shape) -> Body {
    if scan.file_count == 0 {
        return empty_body(shape);
    }
    match shape {
        Shape::TextBlock => text_block_body(&scan),
        Shape::Entries => entries_body(&scan),
        Shape::Bars => bars_body(scan),
        Shape::MarkdownTextBlock => markdown_body(&scan),
        _ => text_body(&scan),
    }
}

fn empty_body(shape: Shape) -> Body {
    match shape {
        Shape::TextBlock => Body::TextBlock(TextBlockData { lines: vec![] }),
        Shape::Entries => Body::Entries(EntriesData { items: vec![] }),
        Shape::Bars => Body::Bars(BarsData { bars: vec![] }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: String::new(),
        }),
        _ => Body::Text(TextData {
            value: String::new(),
        }),
    }
}

fn text_body(scan: &Scan) -> Body {
    Body::Text(TextData {
        value: format!("{} {}", scan.file_count, files_word(scan.file_count)),
    })
}

fn text_block_body(scan: &Scan) -> Body {
    Body::TextBlock(TextBlockData {
        lines: top_level_iter(&scan.top_level)
            .map(|(label, count)| format!("{label} ({count})"))
            .collect(),
    })
}

fn entries_body(scan: &Scan) -> Body {
    Body::Entries(EntriesData {
        items: top_level_iter(&scan.top_level)
            .map(|(label, count)| Entry {
                key: label.to_string(),
                value: Some(count.to_string()),
                status: None,
            })
            .collect(),
    })
}

fn bars_body(scan: Scan) -> Body {
    Body::Bars(BarsData {
        bars: scan
            .top_level
            .into_iter()
            .take(TOP_LEVEL_LIMIT)
            .map(|(label, value)| Bar {
                label,
                value,
                value_label: None,
            })
            .collect(),
    })
}

fn markdown_body(scan: &Scan) -> Body {
    Body::MarkdownTextBlock(MarkdownTextBlockData {
        value: top_level_iter(&scan.top_level)
            .map(|(label, count)| format!("- **{label}** — {count} {}", files_word(count as usize)))
            .collect::<Vec<_>>()
            .join("\n"),
    })
}

fn top_level_iter(top_level: &[(String, u64)]) -> impl Iterator<Item = (&str, u64)> {
    top_level
        .iter()
        .take(TOP_LEVEL_LIMIT)
        .map(|(k, v)| (k.as_str(), *v))
}

fn files_word(n: usize) -> &'static str {
    if n == 1 { "file" } else { "files" }
}

