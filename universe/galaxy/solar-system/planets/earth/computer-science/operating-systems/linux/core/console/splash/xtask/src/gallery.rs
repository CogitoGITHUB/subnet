//! Generic dashboard-gallery pipeline. Discovers `*.toml` files under a source directory,
//! parses each file's `[showcase]` metadata table, renders the dashboard to HTML via
//! `dashboard_snapshot`, and emits a `_index.json` so the docs MDX page can lay out the
//! gallery without a static import list.
//!
//! Used by both `examples/usecases/` (maintainer-curated, opinionated environment-specific
//! samples) and `examples/community/` (user-submitted PRs). Same input shape, different
//! curation tier — keeping the pipeline single-purpose-but-reusable means a community
//! submission isn't a separate code path that can drift.
//!
//! Each TOML has an extra top-level table that DashboardConfig::parse harmlessly ignores
//! (no `deny_unknown_fields`):
//!
//! ```toml
//! [showcase]
//! title = "..."
//! description = "..."
//! context = "home"  # or "project"
//! requires = ["weather (network)"]  # optional
//! author = "Name (@handle)"          # optional, for community submissions
//! source = "https://example.com"     # optional, link to dotfiles / discussion
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::dashboard_snapshot;

#[derive(Debug, Deserialize)]
struct ShowcaseFile {
    showcase: ShowcaseMeta,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ShowcaseMeta {
    pub title: String,
    pub description: String,
    pub context: ShowcaseContext,
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ShowcaseContext {
    Home,
    Project,
}

#[derive(Debug, Serialize)]
struct IndexEntry {
    slug: String,
    title: String,
    description: String,
    context: ShowcaseContext,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    requires: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
    /// The TOML body the user would actually paste into their config — same as the source
    /// file but with the `[showcase]` metadata block removed (that table is xtask-only and
    /// would do nothing in a user dashboard). Embedded into the index so the gallery's
    /// "Show config" `<details>` has a copy-friendly source without an extra fetch.
    code: String,
}

pub fn run(source_dir: &Path, out_dir: &Path, width: u16, height: u16) -> Result<()> {
    let entries = discover(source_dir)?;
    fs::create_dir_all(out_dir).with_context(|| format!("create {}", out_dir.display()))?;

    let index = entries
        .iter()
        .map(|e| render_one(e, out_dir, width, height))
        .collect::<Result<Vec<_>>>()?;

    let index_path = out_dir.join("_index.json");
    let index_json = serde_json::to_string_pretty(&index).context("serialize gallery index")?;
    fs::write(&index_path, index_json)
        .with_context(|| format!("write {}", index_path.display()))?;
    println!("wrote {}", index_path.display());
    Ok(())
}

#[derive(Debug)]
struct DiscoveredShowcase {
    slug: String,
    path: PathBuf,
    meta: ShowcaseMeta,
}

fn discover(dir: &Path) -> Result<Vec<DiscoveredShowcase>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut entries: Vec<DiscoveredShowcase> = fs::read_dir(dir)
        .with_context(|| format!("read {}", dir.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "toml"))
        .map(|p| {
            read_meta(&p).map(|meta| DiscoveredShowcase {
                slug: slug_for(&p),
                path: p,
                meta,
            })
        })
        .collect::<Result<_>>()?;
    entries.sort_by(|a, b| a.slug.cmp(&b.slug));
    Ok(entries)
}

fn slug_for(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("showcase")
        .to_string()
}

fn read_meta(path: &Path) -> Result<ShowcaseMeta> {
    let body = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let parsed: ShowcaseFile =
        toml::from_str(&body).with_context(|| format!("parse [showcase] in {}", path.display()))?;
    Ok(parsed.showcase)
}

fn render_one(
    entry: &DiscoveredShowcase,
    out_dir: &Path,
    width: u16,
    height: u16,
) -> Result<IndexEntry> {
    let html = dashboard_snapshot::render_config_html(&entry.path, width, height)?;
    let dest = out_dir.join(format!("{}.html", entry.slug));
    fs::write(&dest, html).with_context(|| format!("write {}", dest.display()))?;
    println!("wrote {}", dest.display());
    let body = fs::read_to_string(&entry.path)
        .with_context(|| format!("re-read {}", entry.path.display()))?;
    Ok(IndexEntry {
        slug: entry.slug.clone(),
        title: entry.meta.title.clone(),
        description: entry.meta.description.clone(),
        context: entry.meta.context,
        requires: entry.meta.requires.clone(),
        author: entry.meta.author.clone(),
        source: entry.meta.source.clone(),
        code: strip_showcase_table(&body),
    })
}

/// Drop the `[showcase]` metadata table (and the blank line that follows it) from the TOML
/// so the "Show config" panel renders only the part a user would actually paste into their
/// `dashboard.toml`. Walks lines instead of round-tripping through `toml::Value` so the
/// comments and formatting that make the example readable survive intact.
///
/// The `[showcase]` block follows the convention `[showcase]` → `key = value` lines → blank
/// line → next section. The first blank line after the header closes the block, which means
/// comments belonging to the *next* section (between the closing blank line and `[[widget]]`)
/// stay with the section they introduce.
fn strip_showcase_table(body: &str) -> String {
    let mut out = String::new();
    let mut in_showcase = false;
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed == "[showcase]" {
            in_showcase = true;
            continue;
        }
        if in_showcase {
            if trimmed.is_empty() {
                in_showcase = false;
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim_start_matches('\n').to_string()
}

