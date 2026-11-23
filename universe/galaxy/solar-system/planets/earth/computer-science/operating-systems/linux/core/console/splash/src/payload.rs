#![allow(dead_code)]

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Payload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(flatten)]
    pub body: Body,
}

/// Data-shape of a fetched payload. Variants describe the **shape of the data**, not how it's
/// rendered — the same shape can feed multiple renderers (e.g. `Text` feeds both the plain
/// `text_plain` renderer and the big-text `text_ascii` renderer). Config's `render = "…"` picks the
/// renderer, compat-checked against the shape at dispatch time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "shape", content = "data", rename_all = "snake_case")]
pub enum Body {
    /// A single string. Used by anything that emits one logical row: clock time, greeting,
    /// branch name, countdown label.
    Text(TextData),
    /// Zero or more lines of text. Used by anything intrinsically multi-line: recent commits,
    /// worktrees, welcome notes, todo items.
    TextBlock(TextBlockData),
    /// A blob of Markdown source. Distinct shape (not a flavour of `Text` / `TextBlock`)
    /// because the contract is "this string MUST be parsed as Markdown" — a fetcher emitting
    /// it is committing to that semantic, and the matching renderer (`text_markdown`)
    /// refuses anything else. Keeps random `Text` payloads from being rendered as Markdown
    /// by mistake, and keeps `text_plain` from being asked to render Markdown source verbatim.
    MarkdownTextBlock(MarkdownTextBlockData),
    /// Zero or more lines of text where each line carries an optional URL. Renderers that
    /// understand hyperlinks (`list_links`) wrap rows whose `url` is `Some(_)` in OSC 8 escape
    /// sequences so modern terminals surface them as clickable. Renderers that don't honour
    /// links can ignore the urls and render the text only. Right for feeds — HN top, GitHub
    /// PRs/issues/releases — where each row has a canonical "open this" target.
    LinkedTextBlock(LinkedTextBlockData),
    /// `LinkedTextBlock` with a thumbnail column. Each item is a `(thumbnail?, title, url?,
    /// subtitle?)` tuple. Right for media-feed widgets (RSS with images, YouTube channels,
    /// Plex / Jellyfin recently-added, Reddit-with-thumbnails) where the thumbnail carries as
    /// much glance value as the title. Items without `thumbnail_path` get a blank cell of the
    /// same width so the text column stays aligned across rows.
    ImageLinkedList(ImageLinkedListData),
    /// Key/value rows. Used by system info, env dumps, anything label:value shaped.
    Entries(EntriesData),
    /// A single 0..=1 value with an optional display label. Gauges, progress bars, donuts.
    Ratio(RatioData),
    /// Sequence of unsigned integers. Sparklines, histograms, bar-height sequences.
    NumberSeries(NumberSeriesData),
    /// Sequence of `(x, y)` points, one or more series. Line charts, scatter.
    PointSeries(PointSeriesData),
    /// Labeled bars. BarChart, horizontal ranking.
    Bars(BarsData),
    /// Path to an image on disk.
    Image(ImageData),
    /// Small fixed-grid pixel-art sprite drawn cell-by-cell with truecolor half-blocks. Distinct
    /// from `Image`: `Image` is a path the user (or fetcher) supplies, rendered through whatever
    /// terminal-graphics protocol is available; `PixelArt` is inline RGBA data that always
    /// renders with `▀` half-blocks and works in any terminal that supports truecolor.
    PixelArt(PixelArtData),
    /// A month view anchored at a specific date, optionally with highlighted events. Calendar
    /// widget (and anything future that shows month-scale state) consumes this.
    Calendar(CalendarData),
    /// 2D grid of intensities. Used by the GitHub-style contribution graph, habit trackers, any
    /// daily-metric heatmap. Renderer maps each cell into a bucketed color.
    Heatmap(HeatmapData),
    /// Single traffic-light status + short label. Used by CI, deploy, SLO, oncall — one
    /// indicator per fetcher. Rows of badges are a composition concern handled by the nested
    /// layout (`combined_status_row`), not by a multi-entry payload.
    Badge(BadgeData),
    /// Time-stamped events, newest first. Used by git_recent_commits, deploy history, ci
    /// history. Timestamps are raw unix seconds so the renderer computes the `"3h ago"` label
    /// at draw time — keeping cached payloads from going stale.
    Timeline(TimelineData),
    /// Structured error payload — what the user sees instead of a real fetch result when the
    /// widget can't produce one (fetch failed / timed out, fetcher unknown, fetcher can't emit
    /// the renderer's required shape, network widget under untrusted config). Bypasses the
    /// shape × renderer dispatch entirely: `render_payload` short-circuits on `Body::Error` and
    /// draws the message inline regardless of the configured renderer, since asking
    /// `chart_bar` to render an error message via `Bars` would either silently drop it or
    /// trigger a misleading "cannot display" warning. Real fetchers never emit this — only the
    /// placeholder constructors in `crate::fetcher` and `crate::trust`.
    Error(ErrorData),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ErrorData {
    pub kind: ErrorKind,
    /// First line of the error pill. Convention: lead with the affected source ("⚠ deariary",
    /// "🔒 requires trust") so users can scan a multi-widget splash for which one needs
    /// attention without reading the full sentence.
    pub message: String,
    /// Optional second line. Typically a follow-up hint ("see logs", "check `render` in
    /// config"). `None` if the kind is self-explanatory or the area is too narrow for two
    /// lines anyway.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// Fetcher returned `Err` (HTTP failure, parse error, missing token, …).
    Fetch,
    /// Fetcher exceeded the runtime deadline.
    Timeout,
    /// Configured fetcher name is not in the registry — usually a config newer than the
    /// installed binary.
    UnknownFetcher,
    /// Fetcher can't emit the shape its renderer requires; user needs to change `render` or
    /// `fetcher` in config.
    ShapeMismatch,
    /// `Network`-class widget in an untrusted local config; user runs `splashboard trust` to
    /// unlock.
    RequiresTrust,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextData {
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextBlockData {
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MarkdownTextBlockData {
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LinkedTextBlockData {
    pub items: Vec<LinkedLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LinkedLine {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageLinkedListData {
    pub items: Vec<ImageLinkedItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageLinkedItem {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EntriesData {
    pub items: Vec<Entry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RatioData {
    pub value: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Optional total the ratio is a fraction of (e.g. 365 when `value` is "day of year /
    /// 365"). Populated by fetchers that know the underlying count so renderers can express
    /// the fraction directly ("118 of 365"). Omitted when no sensible denominator exists —
    /// renderers fall back to percent-only output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub denominator: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NumberSeriesData {
    pub values: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PointSeriesData {
    pub series: Vec<PointSeries>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PointSeries {
    pub name: String,
    pub points: Vec<(f64, f64)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BarsData {
    pub bars: Vec<Bar>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Bar {
    pub label: String,
    pub value: u64,
    /// Optional pre-formatted right-column text shown by text-first renderers (`list_ranking`).
    /// Visual renderers (`chart_bar`, `chart_pie`) ignore this — they only consume `value`. Let
    /// fetchers carry a unit (`"80h"`, `"1.5M peak"`, `"2024-07-03"`) so the value column reads
    /// as the fetcher intended rather than a raw `u64`. Omitted = renderers fall back to the
    /// numeric `value` printed as-is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageData {
    pub path: String,
}

/// One pixel in a [`PixelArtData`] grid. `a = 0` means transparent — the renderer fills the
/// terminal background through. Partial alpha is currently treated as opaque; the contract is
/// "opaque or transparent" so the artist palette stays simple.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct PixelColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    #[serde(default = "PixelColor::opaque_alpha")]
    pub a: u8,
}

impl PixelColor {
    pub const TRANSPARENT: Self = Self {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    };

    pub const fn opaque(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub fn is_transparent(self) -> bool {
        self.a == 0
    }

    fn opaque_alpha() -> u8 {
        255
    }

    /// Map this pixel to the text or dim style role.
    pub fn roled(self) -> Self {
        if self.a == 0 {
            return self;
        }
        static T: std::sync::OnceLock<crate::theme::Theme> =
            std::sync::OnceLock::new();
        let t = T.get_or_init(crate::theme::Theme::from_style);
        let lum = 0.299 * f32::from(self.r)
            + 0.587 * f32::from(self.g)
            + 0.114 * f32::from(self.b);
        let c = if lum > 128.0 { t.text } else { t.text_dim };
        match c {
            ratatui::style::Color::Rgb(r, g, b) => Self::opaque(r, g, b),
            _ => Self::opaque(255, 255, 255),
        }
    }
}

/// A small fixed-grid pixel-art sprite, rendered cell-by-cell with truecolor half-blocks. Right
/// for sprite-scale ambient widgets (weather icon, system mood face) that need full colour but
/// don't want a terminal-graphics-protocol dependency. Distinct from [`ImageData`], which is a
/// path to a real raster image rendered via ratatui-image. Rows must be the same length; the
/// renderer treats jagged rows as an error payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PixelArtData {
    /// Row-major grid; `pixels[0]` is the top row.
    pub pixels: Vec<Vec<PixelColor>>,
    /// Optional caption rendered under the sprite.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// 2D grid of intensities, row-major (`cells[row][col]`). The renderer picks a bucket per cell
/// via `thresholds` (explicit bucket boundaries) or an auto-quartile fallback when absent.
/// Labels are optional and may be rendered along the edges if space allows.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HeatmapData {
    pub cells: Vec<Vec<u32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thresholds: Option<Vec<u32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_labels: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub col_labels: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BadgeData {
    pub status: Status,
    pub label: String,
}

/// Time-stamped events for the timeline renderer. Timestamps are unix seconds UTC; the renderer
/// formats a relative label (`"3h ago"`, `"yesterday"`, `"Apr 5"`) at draw time so cached
/// payloads don't freeze stale relative strings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TimelineData {
    pub events: Vec<TimelineEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TimelineEvent {
    /// Unix seconds UTC.
    pub timestamp: i64,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CalendarData {
    /// ISO year (e.g. 2026).
    pub year: i32,
    /// 1..=12.
    pub month: u8,
    /// Optional focus / "today" day, 1..=31. If set, the calendar renderer highlights it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub day: Option<u8>,
    /// Extra days to mark (e.g. event days). Empty by default.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<u8>,
}

