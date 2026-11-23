//! `lastfm_scrobbles_today` — today's scrobbles for a Last.fm user, plus the currently-playing
//! track when one is in flight.
//!
//! Reads `user.getRecentTracks` with `from = midnight UTC today`. The currently-playing track
//! (signalled by `@attr.nowplaying = "true"` on the most recent entry) is split out from the
//! today-count rollup so the headline can choose between "♪ now playing" and a count summary
//! depending on whether the user is actively listening when the splash renders.
//!
//! Refresh interval is short (5 minutes) so the splash reflects "what I'm playing now" within
//! a coffee break, but not so short that we hammer the API on a fresh `cd`.

use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;

use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::lastfm::client;
use crate::fetcher::lastfm::common::{ImageEntry, best_image};
use crate::fetcher::thumbnails;
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Body, EntriesData, Entry, ImageLinkedItem, ImageLinkedListData, LinkedLine,
    LinkedTextBlockData, MarkdownTextBlockData, Payload, Status, TextBlockData, TextData,
    TimelineData, TimelineEvent,
};
use crate::render::Shape;

const SHAPES: &[Shape] = &[
    Shape::LinkedTextBlock,
    Shape::ImageLinkedList,
    Shape::TextBlock,
    Shape::Text,
    Shape::MarkdownTextBlock,
    Shape::Entries,
    Shape::Badge,
    Shape::Timeline,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "user",
        type_hint: "string",
        required: true,
        default: None,
        description: "Last.fm username whose scrobbles to read.",
    },
    OptionSchema {
        name: "limit",
        type_hint: "integer (1..=50)",
        required: false,
        default: Some("10"),
        description: "Maximum number of recently scrobbled tracks to show (excluding the currently-playing slot).",
    },
];

const DEFAULT_LIMIT: u32 = 10;
const MIN_LIMIT: u32 = 1;
const MAX_LIMIT: u32 = 50;

pub struct LastfmScrobblesToday;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    pub user: Option<String>,
    pub limit: Option<u32>,
}

#[async_trait]
impl Fetcher for LastfmScrobblesToday {
    fn name(&self) -> &str {
        "lastfm_scrobbles_today"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Today's scrobbles for a Last.fm user, with the currently-playing track surfaced when one is in flight. Reads `user.getRecentTracks` since UTC midnight; intentionally bounded to today so the cached splash doesn't drift across day boundaries."
    }
    fn refresh_interval(&self) -> u64 {
        5 * 60
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        let extra = ctx
            .options
            .as_ref()
            .and_then(|v| toml::to_string(v).ok())
            .unwrap_or_default();
        cache_key(self.name(), ctx, &extra)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        sample_body(shape)
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let user = opts
            .user
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                FetchError::Failed(
                    "lastfm_scrobbles_today: `user = \"<lastfm name>\"` is required".into(),
                )
            })?;
        let limit = opts
            .limit
            .unwrap_or(DEFAULT_LIMIT)
            .clamp(MIN_LIMIT, MAX_LIMIT) as usize;
        let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);

        let snapshot = fetch_snapshot(user, limit).await?;
        let body = render_body(&snapshot, shape).await;
        Ok(payload(body))
    }
}

/// Parsed view of the response the rest of the rendering code reads from.
#[derive(Debug, Clone, Default)]
struct Snapshot {
    /// Tracks scrobbled since midnight UTC today, newest first. Excludes the nowplaying entry.
    tracks: Vec<TrackRow>,
    now_playing: Option<TrackRow>,
}

#[derive(Debug, Clone)]
struct TrackRow {
    name: String,
    artist: String,
    album: Option<String>,
    url: String,
    image_url: Option<String>,
    timestamp: Option<i64>,
}

impl Snapshot {
    fn today_count(&self) -> u64 {
        self.tracks.len() as u64
    }
}

async fn fetch_snapshot(user: &str, limit: usize) -> Result<Snapshot, FetchError> {
    let from = start_of_today_utc().to_string();
    // The nowplaying slot, when present, doesn't count toward `limit` from the API's perspective
    // — request `limit + 1` so a continuously listening user doesn't lose a row to it.
    let cap = (limit + 1).min((MAX_LIMIT + 1) as usize).to_string();
    let raw: RecentTracksResponse = client::get_json(
        "user.getRecentTracks",
        &[("user", user), ("from", &from), ("limit", &cap)],
    )
    .await?;
    let mut snap = parse_recent(raw);
    // Defensive cap — Last.fm sometimes returns one extra row past `limit` regardless of the
    // nowplaying split.
    snap.tracks.truncate(limit);
    Ok(snap)
}

fn start_of_today_utc() -> i64 {
    let today = Utc::now().date_naive().and_hms_opt(0, 0, 0).unwrap();
    chrono::DateTime::<Utc>::from_naive_utc_and_offset(today, Utc).timestamp()
}

fn parse_recent(raw: RecentTracksResponse) -> Snapshot {
    let mut snap = Snapshot::default();
    for raw_track in raw.recenttracks.track {
        let is_now = raw_track
            .attr
            .as_ref()
            .and_then(|a| a.nowplaying.as_deref())
            == Some("true");
        let row = TrackRow {
            name: raw_track.name,
            artist: raw_track.artist.text,
            album: raw_track.album.map(|a| a.text).filter(|s| !s.is_empty()),
            url: raw_track.url,
            image_url: best_image(&raw_track.image),
            timestamp: raw_track.date.and_then(|d| d.uts.parse().ok()),
        };
        if is_now {
            snap.now_playing = Some(row);
        } else {
            snap.tracks.push(row);
        }
    }
    snap
}

async fn render_body(snap: &Snapshot, shape: Shape) -> Body {
    match shape {
        Shape::ImageLinkedList => image_linked_body(snap).await,
        Shape::LinkedTextBlock => linked_text_body(snap),
        Shape::TextBlock => text_block_body(snap),
        Shape::MarkdownTextBlock => markdown_body(snap),
        Shape::Entries => entries_body(snap),
        Shape::Badge => badge_body(snap),
        Shape::Timeline => timeline_body(snap),
        _ => text_body(snap),
    }
}

fn text_body(snap: &Snapshot) -> Body {
    Body::Text(TextData {
        value: headline(snap),
    })
}

fn headline(snap: &Snapshot) -> String {
    if let Some(nowp) = &snap.now_playing {
        format!("♪ {} — {}", nowp.name, nowp.artist)
    } else if snap.today_count() == 1 {
        "1 scrobble today".into()
    } else if snap.today_count() > 0 {
        format!("{} scrobbles today", snap.today_count())
    } else {
        "quiet day".into()
    }
}

fn text_block_body(snap: &Snapshot) -> Body {
    Body::TextBlock(TextBlockData {
        lines: text_lines(snap),
    })
}

fn text_lines(snap: &Snapshot) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(nowp) = &snap.now_playing {
        lines.push(format!("♪ {} — {}", nowp.name, nowp.artist));
    }
    lines.extend(snap.tracks.iter().map(format_track_row));
    if lines.is_empty() {
        lines.push("quiet day".into());
    }
    lines
}

fn format_track_row(t: &TrackRow) -> String {
    let time = t
        .timestamp
        .map(format_time_label)
        .unwrap_or_else(|| "--:--".into());
    format!("{time}  {} — {}", t.name, t.artist)
}

fn format_time_label(ts: i64) -> String {
    chrono::DateTime::<Utc>::from_timestamp(ts, 0)
        .map(|dt| dt.format("%H:%M").to_string())
        .unwrap_or_else(|| "--:--".into())
}

fn markdown_body(snap: &Snapshot) -> Body {
    let mut lines = Vec::new();
    if let Some(nowp) = &snap.now_playing {
        lines.push(format!("- ▶ **{}** — {}", nowp.name, nowp.artist));
    }
    lines.extend(
        snap.tracks
            .iter()
            .map(|t| format!("- **{}** — {}", t.name, t.artist)),
    );
    if lines.is_empty() {
        lines.push("_quiet day_".into());
    }
    Body::MarkdownTextBlock(MarkdownTextBlockData {
        value: lines.join("\n"),
    })
}

fn linked_text_body(snap: &Snapshot) -> Body {
    let mut items = Vec::new();
    if let Some(nowp) = &snap.now_playing {
        items.push(LinkedLine {
            text: format!("♪ {} — {}", nowp.name, nowp.artist),
            url: Some(nowp.url.clone()),
        });
    }
    items.extend(snap.tracks.iter().map(|t| LinkedLine {
        text: format_track_row(t),
        url: Some(t.url.clone()),
    }));
    if items.is_empty() {
        items.push(LinkedLine {
            text: "quiet day".into(),
            url: None,
        });
    }
    Body::LinkedTextBlock(LinkedTextBlockData { items })
}

async fn image_linked_body(snap: &Snapshot) -> Body {
    let rows = collect_rows_for_image(snap);
    let urls: Vec<Option<String>> = rows.iter().map(|(t, _, _)| t.image_url.clone()).collect();
    let paths = thumbnails::download_many(&urls).await;
    let items = rows
        .into_iter()
        .zip(paths)
        .map(|((t, title, subtitle), path)| ImageLinkedItem {
            title,
            url: Some(t.url.clone()),
            thumbnail_path: path.map(|p| p.to_string_lossy().into_owned()),
            subtitle,
        })
        .collect();
    Body::ImageLinkedList(ImageLinkedListData { items })
}

/// Produce `(track, title, subtitle)` rows in display order; nowplaying first.
fn collect_rows_for_image(snap: &Snapshot) -> Vec<(&TrackRow, String, Option<String>)> {
    let mut rows = Vec::new();
    if let Some(nowp) = &snap.now_playing {
        rows.push((
            nowp,
            format!("♪ {} — {}", nowp.name, nowp.artist),
            Some("now playing".into()),
        ));
    }
    for t in &snap.tracks {
        let subtitle = match t.timestamp {
            Some(ts) => Some(format!("{} · {}", format_time_label(ts), t.artist)),
            None => Some(t.artist.clone()),
        };
        rows.push((t, t.name.clone(), subtitle));
    }
    rows
}

fn entries_body(snap: &Snapshot) -> Body {
    let mut items = Vec::new();
    if let Some(nowp) = &snap.now_playing {
        items.push(Entry {
            key: nowp.name.clone(),
            value: Some(format!("▶ {}", nowp.artist)),
            status: Some(Status::Ok),
        });
    }
    items.extend(snap.tracks.iter().map(|t| Entry {
        key: t.name.clone(),
        value: Some(t.artist.clone()),
        status: None,
    }));
    if items.is_empty() {
        items.push(Entry {
            key: "today".into(),
            value: Some("no scrobbles yet".into()),
            status: None,
        });
    }
    Body::Entries(EntriesData { items })
}

fn badge_body(snap: &Snapshot) -> Body {
    let (status, label) = if let Some(nowp) = &snap.now_playing {
        (Status::Ok, format!("♪ {}", nowp.name))
    } else if snap.today_count() == 1 {
        (Status::Ok, "1 today".into())
    } else if snap.today_count() > 0 {
        (Status::Ok, format!("{} today", snap.today_count()))
    } else {
        (Status::Warn, "quiet day".into())
    };
    Body::Badge(BadgeData { status, label })
}

fn timeline_body(snap: &Snapshot) -> Body {
    let mut events = Vec::new();
    if let Some(nowp) = &snap.now_playing {
        events.push(TimelineEvent {
            timestamp: Utc::now().timestamp(),
            title: format!("♪ {} — {}", nowp.name, nowp.artist),
            detail: nowp.album.clone(),
            status: Some(Status::Ok),
        });
    }
    events.extend(snap.tracks.iter().map(|t| TimelineEvent {
        timestamp: t.timestamp.unwrap_or(0),
        title: format!("{} — {}", t.name, t.artist),
        detail: t.album.clone(),
        status: None,
    }));
    Body::Timeline(TimelineData { events })
}

fn sample_body(shape: Shape) -> Option<Body> {
    use crate::samples;
    Some(match shape {
        Shape::Text => samples::text("♪ Strawberry Swing — Coldplay"),
        Shape::TextBlock => samples::text_block(&[
            "♪ Strawberry Swing — Coldplay",
            "14:32  Lost Stars — Adam Levine",
            "14:28  Bloom — Beach House",
        ]),
        Shape::MarkdownTextBlock => samples::markdown(
            "- ▶ **Strawberry Swing** — Coldplay\n- **Lost Stars** — Adam Levine\n- **Bloom** — Beach House",
        ),
        Shape::LinkedTextBlock => samples::linked_text_block(&[
            (
                "♪ Strawberry Swing — Coldplay",
                Some("https://www.last.fm/music/Coldplay/_/Strawberry+Swing"),
            ),
            (
                "14:32  Lost Stars — Adam Levine",
                Some("https://www.last.fm/music/Adam+Levine/_/Lost+Stars"),
            ),
        ]),
        Shape::ImageLinkedList => samples::image_linked_list(&[
            (
                "♪ Strawberry Swing — Coldplay",
                Some("https://www.last.fm/music/Coldplay/_/Strawberry+Swing"),
                None,
                Some("now playing"),
            ),
            (
                "Lost Stars",
                Some("https://www.last.fm/music/Adam+Levine/_/Lost+Stars"),
                None,
                Some("14:32 · Adam Levine"),
            ),
        ]),
        Shape::Entries => samples::entries(&[
            ("Strawberry Swing", "▶ Coldplay"),
            ("Lost Stars", "Adam Levine"),
        ]),
        Shape::Badge => samples::badge(Status::Ok, "♪ Strawberry Swing"),
        Shape::Timeline => samples::timeline(&[
            (
                1_700_000_000,
                "♪ Strawberry Swing — Coldplay",
                Some("Viva la Vida"),
            ),
            (1_699_999_500, "Lost Stars — Adam Levine", None),
        ]),
        _ => return None,
    })
}

#[derive(Deserialize)]
struct RecentTracksResponse {
    recenttracks: RecentTracksInner,
}

#[derive(Deserialize)]
struct RecentTracksInner {
    #[serde(default)]
    track: Vec<RawTrack>,
}

#[derive(Deserialize)]
struct RawTrack {
    name: String,
    artist: ArtistObj,
    #[serde(default)]
    album: Option<AlbumObj>,
    url: String,
    #[serde(default)]
    image: Vec<ImageEntry>,
    #[serde(default)]
    date: Option<DateObj>,
    #[serde(rename = "@attr", default)]
    attr: Option<TrackAttr>,
}

#[derive(Deserialize)]
struct ArtistObj {
    #[serde(rename = "#text", default)]
    text: String,
}

#[derive(Deserialize)]
struct AlbumObj {
    #[serde(rename = "#text", default)]
    text: String,
}

#[derive(Deserialize)]
struct DateObj {
    uts: String,
}

#[derive(Deserialize)]
struct TrackAttr {
    #[serde(default)]
    nowplaying: Option<String>,
}

