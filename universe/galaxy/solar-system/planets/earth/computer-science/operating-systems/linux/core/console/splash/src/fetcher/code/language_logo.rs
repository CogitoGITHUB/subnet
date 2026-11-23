use std::collections::HashMap;
use std::path::PathBuf;

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use crate::options::OptionSchema;
use crate::paths;
use crate::payload::{Body, ImageData, Payload};
use crate::render::Shape;

use super::super::git::{open_repo, payload, repo_cache_key};
use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::logo_assets;
use super::scan::classify_path;
use super::scan::for_each_tracked_path;

const SHAPES: &[Shape] = &[Shape::Image];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "language",
    type_hint: "string",
    required: false,
    default: Some("auto-detected from the dominant language in the repo"),
    description: "Override the detected language. Accepts the labels emitted by the shared `code_*` classifier (`Rust`, `Go`, `Python`, …); unknown values fall back to the generic `</>` glyph.",
}];

/// Picks the dominant programming language across the discovered git repo's tracked files and
/// renders a bundled Devicon PNG for it. Counts files (not lines) per language — cheap, no
/// blob reads, and "what kind of repo is this" is more honest by file mix than line totals.
/// Languages without a dedicated asset fall back to a generic `</>` glyph so the
/// `project_codebase` hero never renders empty.
pub struct CodeLanguageLogo;

#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    language: Option<String>,
}

#[async_trait]
impl Fetcher for CodeLanguageLogo {
    fn name(&self) -> &str {
        "code_language_logo"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Detects the dominant language across the repo's tracked files and emits a bundled Devicon PNG (covers ~35 languages; falls back to a generic `</>` glyph). Pair with the `media_image` renderer for a `project_codebase`-style hero that scales to whatever cell the layout assigns."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 60
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        // Mix the `language` option into the key so an explicit override doesn't collide with
        // the auto-detected entry for the same repo.
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
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref())?;
        let language = match opts.language {
            Some(label) => Some(label),
            None => detect_dominant_language()?.map(String::from),
        };
        let (slug, bytes) = logo_assets::asset_for(language.as_deref());
        let path = ensure_extracted(slug, bytes)?;
        Ok(payload(Body::Image(ImageData {
            path: path.to_string_lossy().into_owned(),
        })))
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

fn detect_dominant_language() -> Result<Option<&'static str>, FetchError> {
    let repo = open_repo()?;
    detect_dominant_in(&repo)
}

fn detect_dominant_in(repo: &gix::Repository) -> Result<Option<&'static str>, FetchError> {
    let mut counts: HashMap<&'static str, u32> = HashMap::new();
    for_each_tracked_path(repo, |path| {
        if let Some(lang) = classify_path(path) {
            *counts.entry(lang).or_insert(0) += 1;
        }
    })?;
    Ok(counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(a.0)))
        .map(|(lang, _)| lang))
}

/// Materialises the bundled bytes onto disk so `media_image` (which only takes a path) can
/// open them. Stable per-slug filename under the cache dir means repeat invocations reuse the
/// same file and ratatui-image's protocol can warm-cache decoded frames.
fn ensure_extracted(slug: &str, bytes: &[u8]) -> Result<PathBuf, FetchError> {
    let dir = logos_cache_dir()?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| FetchError::Failed(format!("create logos cache dir: {e}")))?;
    let path = dir.join(format!("{slug}.png"));
    if !path.exists() {
        std::fs::write(&path, bytes)
            .map_err(|e| FetchError::Failed(format!("write logo {slug}: {e}")))?;
    }
    Ok(path)
}

fn logos_cache_dir() -> Result<PathBuf, FetchError> {
    paths::cache_dir()
        .map(|d| d.join("logos"))
        .ok_or_else(|| FetchError::Failed("no SPLASH_HOME / HOME".into()))
}

