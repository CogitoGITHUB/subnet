use std::collections::HashMap;

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use crate::options::OptionSchema;
use crate::payload::{
    Bar, BarsData, Body, EntriesData, Entry, MarkdownTextBlockData, Payload, TextBlockData,
    TextData,
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
];
const DEFAULT_MARKERS: &[&str] = &["TODO", "FIXME"];
const DEFAULT_LIMIT: usize = 20;
const INTERNAL_HIT_CAP: usize = 500;
const SNIPPET_CHARS: usize = 100;

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "markers",
        type_hint: "array of strings",
        required: false,
        default: Some("[\"TODO\", \"FIXME\"]"),
        description: "Markers grepped for. Each must appear as a whole-word ASCII token followed by `:` (or `(name):`) — `// TODO: refactor` matches, `## TODO` heading and `\"TODO\"` literals don't.",
    },
    OptionSchema {
        name: "limit",
        type_hint: "integer",
        required: false,
        default: Some("20"),
        description: "Cap on rendered hits (TextBlock) or rows (Entries / Bars). Total count surfaced by `Text` is unaffected.",
    },
];

/// Greps tracked source files for `TODO:` / `FIXME:`-style comment markers. The trailing
/// colon (also accepting `TODO(name):`) is required so README headings, prose, and string
/// literals don't inflate the count. `Text` summarises `"N TODOs in M files"`; `TextBlock`
/// lists `path:line: snippet`; `Entries` / `Bars` rank files by hit count. Walks the index
/// of the repo discovered from CWD so untracked and `.gitignore`-d files are skipped
/// automatically; vendored / generated trees that happen to be committed (`node_modules/`,
/// `vendor/`, `dist/`, `target/`, …) are skipped via the shared scan helper so library
/// TODOs don't bleed into the count.
pub struct CodeTodos;

#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    markers: Option<Vec<String>>,
    #[serde(default)]
    limit: Option<usize>,
}

#[async_trait]
impl Fetcher for CodeTodos {
    fn name(&self) -> &str {
        "code_todos"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Greps tracked source files in the discovered git repo for `TODO:` / `FIXME:` style markers (trailing colon required, vendored / generated dirs skipped). `Text` summarises `\"N TODOs in M files\"`, `TextBlock` and `MarkdownTextBlock` list `path:line: snippet` (markdown variant uses inline-code formatting), and `Entries` / `Bars` rank files by hit count."
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
        // Mix `[widget.options]` into the key — markers / limit change what gets read, so two
        // widgets pointing at this fetcher with different options must not share a cache slot.
        // `toml = "0.8"` keeps tables in sorted order, so `Value::to_string()` is stable across runs.
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
            Shape::Text => samples::text("12 TODOs in 5 files"),
            Shape::TextBlock => samples::text_block(&[
                "src/render/mod.rs:120: TODO: handle empty body",
                "src/fetcher/git.rs:88: FIXME: don't panic on detached head",
                "src/main.rs:14: TODO: load config from $HOME",
            ]),
            Shape::MarkdownTextBlock => samples::markdown(
                "- `src/render/mod.rs:120` — TODO: handle empty body\n- `src/fetcher/git.rs:88` — FIXME: don't panic on detached head\n- `src/main.rs:14` — TODO: load config from $HOME",
            ),
            Shape::Entries => samples::entries(&[
                ("src/render/mod.rs", "5"),
                ("src/fetcher/git.rs", "3"),
                ("src/main.rs", "2"),
            ]),
            Shape::Bars => samples::bars(&[
                ("src/render/mod.rs", 5),
                ("src/fetcher/git.rs", 3),
                ("src/main.rs", 2),
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref())?;
        let markers = resolve_markers(opts.markers.as_deref());
        let limit = opts.limit.filter(|n| *n > 0).unwrap_or(DEFAULT_LIMIT);
        let scan = scan_cwd(&markers)?;
        Ok(payload(render_body(
            scan,
            ctx.shape.unwrap_or(Shape::Text),
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

fn resolve_markers(opt: Option<&[String]>) -> Vec<String> {
    let custom: Vec<String> = opt
        .map(|v| {
            v.iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    if custom.is_empty() {
        DEFAULT_MARKERS.iter().map(|s| (*s).into()).collect()
    } else {
        custom
    }
}

#[derive(Debug, Default)]
struct ScanResult {
    hits: Vec<Hit>,
    total: usize,
    files_with_hits: usize,
    file_counts: Vec<(String, u64)>,
}

#[derive(Debug)]
struct Hit {
    path: String,
    line: usize,
    text: String,
}

fn scan_cwd(markers: &[String]) -> Result<ScanResult, FetchError> {
    let repo = open_repo()?;
    scan_repo(&repo, markers)
}

fn scan_repo(repo: &gix::Repository, markers: &[String]) -> Result<ScanResult, FetchError> {
    let needles: Vec<&str> = markers.iter().map(String::as_str).collect();
    let mut hits = Vec::new();
    let mut counts: HashMap<String, u64> = HashMap::new();
    let mut total = 0usize;
    for_each_tracked_file(repo, |path_str, bytes| {
        let Ok(text) = std::str::from_utf8(bytes) else {
            return;
        };
        for (idx, line) in text.lines().enumerate() {
            if let Some(snippet) = match_marker(line, &needles) {
                total += 1;
                *counts.entry(path_str.to_string()).or_insert(0) += 1;
                if hits.len() < INTERNAL_HIT_CAP {
                    hits.push(Hit {
                        path: path_str.to_string(),
                        line: idx + 1,
                        text: snippet,
                    });
                }
            }
        }
    })?;
    let files_with_hits = counts.len();
    let mut file_counts: Vec<_> = counts.into_iter().collect();
    file_counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Ok(ScanResult {
        hits,
        total,
        files_with_hits,
        file_counts,
    })
}

fn match_marker(line: &str, markers: &[&str]) -> Option<String> {
    let bytes = line.as_bytes();
    let mut best: Option<usize> = None;
    for needle in markers {
        if let Some(pos) = find_marker(bytes, needle.as_bytes())
            && best.map(|b| pos < b).unwrap_or(true)
        {
            best = Some(pos);
        }
    }
    best.map(|pos| truncate(line[pos..].trim_end(), SNIPPET_CHARS))
}

/// Find a marker followed by a `TODO:` / `TODO(name):` style colon. Word-boundary on the
/// leading side so `TODOLIST` doesn't match; trailing `:` excludes README headings, prose,
/// and string-literal hits like `"TODO"`. The optional `(...)` arm handles the `TODO(yuji):`
/// author-tag convention. `::` (C++ namespace) is rejected explicitly.
fn find_marker(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).find(|&i| {
        let end = i + needle.len();
        haystack[i..end] == *needle
            && (i == 0 || !is_word_byte(haystack[i - 1]))
            && (end < haystack.len() && !is_word_byte(haystack[end]))
            && trailing_colon(haystack, end)
    })
}

fn trailing_colon(bytes: &[u8], mut i: usize) -> bool {
    if bytes.get(i) == Some(&b'(') {
        let mut depth = 1;
        i += 1;
        while i < bytes.len() && depth > 0 {
            match bytes[i] {
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ => {}
            }
            i += 1;
        }
        if depth != 0 {
            return false;
        }
    }
    bytes.get(i) == Some(&b':') && bytes.get(i + 1) != Some(&b':')
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let head: String = s.chars().take(max_chars).collect();
        format!("{head}…")
    }
}

fn render_body(scan: ScanResult, shape: Shape, limit: usize) -> Body {
    match shape {
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: scan
                .hits
                .into_iter()
                .take(limit)
                .map(|h| format!("{}:{}: {}", h.path, h.line, h.text))
                .collect(),
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: scan
                .hits
                .into_iter()
                .take(limit)
                .map(|h| format!("- `{}:{}` — {}", h.path, h.line, h.text))
                .collect::<Vec<_>>()
                .join("\n"),
        }),
        Shape::Entries => Body::Entries(EntriesData {
            items: scan
                .file_counts
                .into_iter()
                .take(limit)
                .map(|(p, c)| Entry {
                    key: p,
                    value: Some(c.to_string()),
                    status: None,
                })
                .collect(),
        }),
        Shape::Bars => Body::Bars(BarsData {
            bars: scan
                .file_counts
                .into_iter()
                .take(limit)
                .map(|(p, c)| Bar {
                    label: p,
                    value: c,
                    value_label: None,
                })
                .collect(),
        }),
        _ => text_summary(scan.total, scan.files_with_hits),
    }
}

fn text_summary(total: usize, files: usize) -> Body {
    let value = if total == 0 {
        String::new()
    } else {
        format!(
            "{total} TODO{} in {files} file{}",
            if total == 1 { "" } else { "s" },
            if files == 1 { "" } else { "s" },
        )
    };
    Body::Text(TextData { value })
}

