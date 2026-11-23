#![allow(dead_code)]

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::options::OptionSchema;
use crate::payload::{Body, ErrorData, ErrorKind, Payload, Status};
use crate::render::Shape;
use crate::samples;

pub mod basic;
pub mod calendar;
pub mod claude;
pub mod clock;
pub mod code;
pub mod codex;
pub mod crypto_trending;
pub mod crypto_watchlist;
pub mod deal;
pub mod deariary;
pub mod feed;
pub mod forge_items;
pub mod git;
pub mod github;
pub mod gitlab;
pub mod hackernews;
pub mod huggingface_trending;
pub mod lastfm;
pub mod linear;
pub mod llm_pricing;
pub mod lobsters;
pub mod nasa_apod;
pub mod net;
pub mod news;
pub mod project;
pub mod random_cat;
pub mod random_dog;
pub mod random_fortune;
pub mod random_quote;
pub mod read_store;
pub mod reddit;
pub mod rss;
pub mod static_text;
pub mod steam;
pub mod stock_watchlist;
pub mod system;
pub mod thumbnails;
pub mod todoist;
pub mod weather;
pub mod wikipedia;
pub mod youtube;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Safety {
    Safe,
    Network,
    Exec,
}

#[derive(Debug, Clone)]
pub enum FetchError {
    Unknown(String),
    Failed(String),
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(name) => write!(f, "unknown fetcher: {name}"),
            Self::Failed(msg) => write!(f, "fetch failed: {msg}"),
        }
    }
}

impl std::error::Error for FetchError {}

#[derive(Debug, Clone, Default)]
pub struct FetchContext {
    pub widget_id: String,
    pub format: Option<String>,
    pub timeout: Duration,
    /// read_store file encoding — "json" / "toml" / "text". Set by config; ignored by
    /// fetchers that don't read files.
    pub file_format: Option<String>,
    /// Target shape the fetcher should emit. Derived at the runtime boundary from the
    /// widget's renderer (renderers are single-shape today) or the fetcher's
    /// [`Fetcher::default_shape`] when no renderer is specified. Multi-shape fetchers branch
    /// on this; single-shape fetchers ignore it.
    pub shape: Option<Shape>,
    /// Fetcher-specific options passed through from `[widget.options]`. Each fetcher
    /// deserializes its own typed view; unknown keys are ignored.
    pub options: Option<toml::Value>,
    /// `[general].timezone` resolved by the runtime. `None` means "use host `Local`" — every
    /// helper in [`crate::time`] honours that fallback. Per-widget overrides can still be
    /// pulled off `options` (for example `clock_*` honour their own `timezone` field), but
    /// fetchers that don't take their own override read this.
    pub timezone: Option<String>,
    /// `[general].locale` resolved by the runtime — chrono locale code such as `"en_US"`. `None`
    /// resolves to `Locale::POSIX`, matching chrono's unlocalised `format()` output.
    pub locale: Option<String>,
}

#[async_trait]
pub trait Fetcher: Send + Sync {
    fn name(&self) -> &str;
    fn safety(&self) -> Safety;
    /// One- or two-sentence summary surfaced in the generated reference docs. Should explain
    /// what the fetcher shows and, where multiple siblings exist within the same family,
    /// what makes this one different (`clock_almanac` vs `clock_sunrise`, `git_status` vs
    /// `git_stash_count`). No leading verb tense convention; just write so a config author
    /// scanning the catalog can decide whether this is the one they want.
    fn description(&self) -> &'static str;
    /// How long a successful payload stays fresh in the disk cache, in seconds. Required, with no
    /// default impl: the fetcher knows how volatile its data is, so every author must make a
    /// deliberate choice. A per-widget `refresh_interval` in the dashboard config overrides this;
    /// the runtime resolves the config override first and this value otherwise, with no global
    /// fallback.
    fn refresh_interval(&self) -> u64;
    /// Shapes this fetcher can emit. Single-shape fetchers (static_text, disk) return one
    /// variant; fetchers whose one physical read can be reshaped for different widgets (clock,
    /// read_store) list every variant they accept. Validated against the renderer-derived
    /// shape at the runtime boundary so an unsupported pairing renders a placeholder rather
    /// than silently returning the wrong Body.
    fn shapes(&self) -> &[Shape];
    /// Shape used when config doesn't specify a renderer. Must be an element of `shapes()`.
    /// Defaults to the first entry so simple single-shape fetchers don't need to override.
    fn default_shape(&self) -> Shape {
        self.shapes()[0]
    }
    /// Identity of this fetcher-invocation for caching. Two calls that share a key see the same
    /// payload. Default covers the common case (`name + format`); fetchers whose output depends on
    /// more (cwd, repo, URL) override this to mix those in.
    fn cache_key(&self, ctx: &FetchContext) -> String {
        default_cache_key(self.name(), ctx)
    }
    /// Options this fetcher accepts from `[widget.options]`. Default is an empty slice —
    /// fetchers with no tunables don't need to override. Consumed at docs-generation time by the
    /// `xtask` crate; parsing at runtime is still serde-driven (the schema and the `Options`
    /// struct live side-by-side in the same file).
    fn option_schemas(&self) -> &[OptionSchema] {
        &[]
    }
    /// A representative payload body for the given shape, used to preview what this fetcher
    /// emits without executing it. Default falls back to [`crate::samples::canonical_sample`];
    /// fetchers override to surface something closer to real output (`"main +2 ◆3"` for
    /// `git_status`, `"14:35"` for `clock`). Must be a member of `shapes()`.
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        samples::canonical_sample(shape)
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError>;
}

/// Fetchers whose value is intrinsically "right now" — wall clock, instantaneous system counters,
/// countdowns. Contract: computed in <1ms, infallible, no I/O. Bypasses the disk cache entirely
/// and is recomputed by the runtime on every frame. If a prospective fetcher can't meet that
/// contract, it belongs on [`Fetcher`] instead.
pub trait RealtimeFetcher: Send + Sync {
    fn name(&self) -> &str;
    fn safety(&self) -> Safety;
    /// See [`Fetcher::description`] — same purpose for the realtime side of the registry.
    fn description(&self) -> &'static str;
    fn shapes(&self) -> &[Shape];
    fn default_shape(&self) -> Shape {
        self.shapes()[0]
    }
    /// See [`Fetcher::option_schemas`]. Default is an empty slice.
    fn option_schemas(&self) -> &[OptionSchema] {
        &[]
    }
    /// See [`Fetcher::sample_body`]. Default falls through to the shape-level canonical sample.
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        samples::canonical_sample(shape)
    }
    fn compute(&self, ctx: &FetchContext) -> Payload;
}

/// Fetcher can't emit the shape the renderer expects. Surfaced as a placeholder so the splash
/// keeps rendering even when the config pairs, say, `fetcher = "system_monitor_disk"` with `render = "grid_calendar"`.
#[derive(Debug, Clone)]
pub struct ShapeMismatch {
    pub fetcher: String,
    pub requested: Shape,
}

/// Compose an error placeholder payload. Used by every constructor in this module that
/// produces a placeholder when a real fetch can't run (`error_placeholder` /
/// `timeout_placeholder` / `unknown_fetcher_placeholder` / `shape_mismatch_placeholder`),
/// plus [`crate::trust::requires_trust_placeholder`]. `Body::Error` carries the structured
/// kind / message / detail straight through to `render_payload`, which renders it inline
/// regardless of the configured renderer — bypassing the shape × renderer dispatch so the
/// message reaches the user even for shape-only chart / grid renderers.
fn error_payload(kind: ErrorKind, message: String, detail: Option<&str>) -> Payload {
    Payload {
        icon: None,
        status: Some(Status::Error),
        format: None,
        body: Body::Error(ErrorData {
            kind,
            message,
            detail: detail.map(str::to_string),
        }),
    }
}

/// Two-line placeholder shown when the fetcher/renderer pair can't produce data the renderer
/// will accept. First line names the offending fetcher and the shape it was asked to emit;
/// the second line tells the user where to fix it.
pub fn shape_mismatch_placeholder(err: &ShapeMismatch) -> Payload {
    error_payload(
        ErrorKind::ShapeMismatch,
        format!("⚠ {} can't emit {}", err.fetcher, err.requested.as_str()),
        Some("check `render` in config"),
    )
}

/// Payload rendered in place of a widget whose configured fetcher name isn't registered. Mirrors
/// the renderer side of the contract (`render_payload` draws `unknown renderer: <name>` for the
/// same situation): the typical trigger is an installed binary older than the config it's reading,
/// so the second line points at upgrading rather than the log file.
pub fn unknown_fetcher_placeholder(name: &str) -> Payload {
    error_payload(
        ErrorKind::UnknownFetcher,
        format!("⚠ unknown fetcher: {name}"),
        Some("upgrade splashboard?"),
    )
}

/// Payload rendered in place of a widget whose fetch returned `Err`. One-line `⚠ <msg>`, with a
/// follow-up pointer to the log file so the user can track structured details (error chain,
/// cache key, fetcher name) without forcing a second line into the splash for every error.
pub fn error_placeholder(msg: &str) -> Payload {
    error_payload(
        ErrorKind::Fetch,
        format!("⚠ {msg}"),
        Some("see $HOME/.splash/logs"),
    )
}

/// Payload rendered in place of a widget whose fetch exceeded the runtime deadline. Kept
/// separate from [`error_placeholder`] so users can tell "slow" from "broken" at a glance.
pub fn timeout_placeholder() -> Payload {
    error_payload(
        ErrorKind::Timeout,
        "⏱ timed out".into(),
        Some("see $HOME/.splash/logs"),
    )
}

pub fn default_cache_key(name: &str, ctx: &FetchContext) -> String {
    use sha2::{Digest, Sha256};
    // Shape is part of the key because multi-shape fetchers branch their `fetch` body on
    // `ctx.shape` — the cached payload's body variant must round-trip with the next read,
    // or the renderer will reject the wrong shape. Single-shape fetchers also include it
    // (constant), so the digest stays the same shape across them. Without this, adding a
    // new shape variant to a fetcher silently poisons every existing cache entry until it
    // expires (last seen with `basic_static` + `text_markdown`).
    let shape = ctx.shape.map(|s| s.as_str()).unwrap_or("");
    let raw = format!("{}|{}|{}", name, shape, ctx.format.as_deref().unwrap_or(""));
    let digest = Sha256::digest(raw.as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("{name}-{hex}")
}

/// Registry entry — either a cache-backed async fetcher or a recompute-each-frame realtime one.
/// The trust gate and name lookup work against this unified type; runtime unwraps to the specific
/// kind only when it needs the kind-specific behavior (cache I/O vs. per-frame compute).
#[derive(Clone)]
pub enum RegisteredFetcher {
    Cached(Arc<dyn Fetcher>),
    Realtime(Arc<dyn RealtimeFetcher>),
}

impl RegisteredFetcher {
    pub fn name(&self) -> &str {
        match self {
            Self::Cached(f) => f.name(),
            Self::Realtime(f) => f.name(),
        }
    }
    pub fn safety(&self) -> Safety {
        match self {
            Self::Cached(f) => f.safety(),
            Self::Realtime(f) => f.safety(),
        }
    }
    pub fn description(&self) -> &'static str {
        match self {
            Self::Cached(f) => f.description(),
            Self::Realtime(f) => f.description(),
        }
    }
    pub fn shapes(&self) -> Vec<Shape> {
        match self {
            Self::Cached(f) => f.shapes().to_vec(),
            Self::Realtime(f) => f.shapes().to_vec(),
        }
    }
    pub fn default_shape(&self) -> Shape {
        match self {
            Self::Cached(f) => f.default_shape(),
            Self::Realtime(f) => f.default_shape(),
        }
    }
    pub fn option_schemas(&self) -> &[OptionSchema] {
        match self {
            Self::Cached(f) => f.option_schemas(),
            Self::Realtime(f) => f.option_schemas(),
        }
    }
    pub fn sample_body(&self, shape: Shape) -> Option<Body> {
        match self {
            Self::Cached(f) => f.sample_body(shape),
            Self::Realtime(f) => f.sample_body(shape),
        }
    }
    pub fn as_cached(&self) -> Option<Arc<dyn Fetcher>> {
        match self {
            Self::Cached(f) => Some(f.clone()),
            Self::Realtime(_) => None,
        }
    }
    pub fn as_realtime(&self) -> Option<Arc<dyn RealtimeFetcher>> {
        match self {
            Self::Realtime(f) => Some(f.clone()),
            Self::Cached(_) => None,
        }
    }
}

#[derive(Default, Clone)]
pub struct Registry {
    fetchers: HashMap<String, RegisteredFetcher>,
}

impl Registry {
    pub fn with_builtins() -> Self {
        let mut r = Self::default();
        r.register(Arc::new(static_text::StaticText));
        r.register(Arc::new(read_store::ReadStoreFetcher));
        r.register(Arc::new(rss::RssFetcher));
        r.register(Arc::new(crypto_watchlist::CryptoWatchlistFetcher));
        r.register(Arc::new(crypto_trending::CryptoTrendingFetcher));
        r.register(Arc::new(stock_watchlist::StockWatchlistFetcher));
        r.register(Arc::new(random_cat::RandomCatFetcher));
        r.register(Arc::new(random_dog::RandomDogFetcher));
        r.register(Arc::new(nasa_apod::NasaApodFetcher));
        r.register(Arc::new(youtube::YoutubeChannelFetcher));
        r.register(Arc::new(huggingface_trending::HuggingfaceTrendingFetcher));
        for f in calendar::fetchers() {
            r.register(f);
        }
        for f in claude::fetchers() {
            r.register(f);
        }
        for f in codex::fetchers() {
            r.register(f);
        }
        for f in deal::fetchers() {
            r.register(f);
        }
        for f in deariary::fetchers() {
            r.register(f);
        }
        for f in hackernews::fetchers() {
            r.register(f);
        }
        for f in lastfm::fetchers() {
            r.register(f);
        }
        for f in linear::fetchers() {
            r.register(f);
        }
        for f in lobsters::fetchers() {
            r.register(f);
        }
        for f in news::fetchers() {
            r.register(f);
        }
        for f in todoist::fetchers() {
            r.register(f);
        }
        for f in reddit::fetchers() {
            r.register(f);
        }
        for f in steam::fetchers() {
            r.register(f);
        }
        for f in weather::fetchers() {
            r.register(f);
        }
        for f in wikipedia::fetchers() {
            r.register(f);
        }
        for f in clock::realtime_fetchers() {
            r.register_realtime(f);
        }
        r.register_realtime(Arc::new(random_quote::RandomQuoteFetcher));
        r.register_realtime(Arc::new(random_fortune::RandomFortuneFetcher));
        for f in system::realtime_fetchers() {
            r.register_realtime(f);
        }
        for f in system::cached_fetchers() {
            r.register(f);
        }
        for f in net::cached_fetchers() {
            r.register(f);
        }
        for f in net::realtime_fetchers() {
            r.register_realtime(f);
        }
        for f in git::fetchers() {
            r.register(f);
        }
        for f in github::fetchers() {
            r.register(f);
        }
        for f in gitlab::fetchers() {
            r.register(f);
        }
        for f in code::fetchers() {
            r.register(f);
        }
        for f in project::fetchers() {
            r.register(f);
        }
        for f in basic::realtime_fetchers() {
            r.register_realtime(f);
        }
        r
    }

    pub fn register(&mut self, f: Arc<dyn Fetcher>) {
        self.fetchers
            .insert(f.name().to_string(), RegisteredFetcher::Cached(f));
    }

    pub fn register_realtime(&mut self, f: Arc<dyn RealtimeFetcher>) {
        self.fetchers
            .insert(f.name().to_string(), RegisteredFetcher::Realtime(f));
    }

    pub fn get(&self, name: &str) -> Option<RegisteredFetcher> {
        self.fetchers.get(name).cloned()
    }

    pub fn get_cached(&self, name: &str) -> Option<Arc<dyn Fetcher>> {
        self.get(name).and_then(|f| f.as_cached())
    }

    pub fn get_realtime(&self, name: &str) -> Option<Arc<dyn RealtimeFetcher>> {
        self.get(name).and_then(|f| f.as_realtime())
    }

    pub fn names(&self) -> Vec<&str> {
        self.fetchers.keys().map(String::as_str).collect()
    }

    /// All registered fetchers, sorted by name. Useful for deterministic docs generation.
    pub fn sorted(&self) -> Vec<RegisteredFetcher> {
        let mut entries: Vec<_> = self.fetchers.values().cloned().collect();
        entries.sort_by(|a, b| a.name().cmp(b.name()));
        entries
    }
}

