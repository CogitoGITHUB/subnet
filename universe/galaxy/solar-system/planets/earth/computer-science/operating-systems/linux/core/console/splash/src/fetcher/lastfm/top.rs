//! Shared types and rendering helpers for the `lastfm_top_*` family
//! (`top_artists` / `top_tracks` / `top_albums`).
//!
//! All three endpoints return a structurally identical "ranked entity with a playcount" list,
//! differing only in (a) which API method is hit, (b) whether the row has a secondary artist
//! label (artists: none; tracks/albums: yes), and (c) the URL convention. Normalising those
//! differences into [`TopRow`] lets each sibling stay around ~200 lines.

use serde::Deserialize;

use crate::fetcher::lastfm::common::{format_count, plays_word};
use crate::fetcher::thumbnails;
use crate::payload::{
    BadgeData, Bar, BarsData, Body, EntriesData, Entry, ImageLinkedItem, ImageLinkedListData,
    LinkedLine, LinkedTextBlockData, MarkdownTextBlockData, NumberSeriesData, RatioData, Status,
    TextBlockData, TextData,
};

/// Rolling window for the `user.getTop*` endpoints. Default `SevenDay` matches Last.fm's own
/// "Last 7 days" tab on the user profile, and is the most actionable value (long enough to
/// have meaningful data, short enough to reflect recent taste shifts).
#[derive(Debug, Default, Clone, Copy, Deserialize, PartialEq, Eq)]
pub enum Period {
    #[serde(rename = "7day")]
    #[default]
    SevenDay,
    #[serde(rename = "1month")]
    OneMonth,
    #[serde(rename = "3month")]
    ThreeMonth,
    #[serde(rename = "6month")]
    SixMonth,
    #[serde(rename = "12month")]
    TwelveMonth,
    #[serde(rename = "overall")]
    Overall,
}

impl Period {
    /// Value of the `period` query parameter. Last.fm's accepted values are exactly the
    /// serde-renamed variants below; we keep the as_param mapping explicit so a future rename
    /// can't silently drift one side.
    pub fn as_param(self) -> &'static str {
        match self {
            Self::SevenDay => "7day",
            Self::OneMonth => "1month",
            Self::ThreeMonth => "3month",
            Self::SixMonth => "6month",
            Self::TwelveMonth => "12month",
            Self::Overall => "overall",
        }
    }

    /// Short human-readable label used inside `Text` / `Badge` / `Entries` rollups so the user
    /// can tell at a glance which window is in view.
    pub fn label(self) -> &'static str {
        match self {
            Self::SevenDay => "last 7d",
            Self::OneMonth => "last 1m",
            Self::ThreeMonth => "last 3m",
            Self::SixMonth => "last 6m",
            Self::TwelveMonth => "last 12m",
            Self::Overall => "overall",
        }
    }
}

/// One ranked entity normalised across all three sibling endpoints.
#[derive(Debug, Clone)]
pub struct TopRow {
    /// 1-indexed display rank.
    pub rank: usize,
    /// Row title: artist name, track name, or album name depending on the sibling.
    pub primary: String,
    /// For `top_tracks` / `top_albums` this is the artist. For `top_artists` it's `None`.
    pub secondary: Option<String>,
    pub playcount: u64,
    pub url: String,
    pub image_url: Option<String>,
}

impl TopRow {
    /// `"Artist"` or `"Track — Artist"` form, used in row titles.
    pub fn display_title(&self) -> String {
        match &self.secondary {
            Some(s) if !s.is_empty() => format!("{} — {}", self.primary, s),
            _ => self.primary.clone(),
        }
    }

    /// Right-side count column: `"42 plays"` (singular when applicable).
    pub fn count_label(&self) -> String {
        format!(
            "{} {}",
            format_count(self.playcount),
            plays_word(self.playcount)
        )
    }
}

/// Headline for `Text` shape: `"#1 Title (42 plays)"`. Falls back when the list is empty.
pub fn headline(rows: &[TopRow], empty_label: &str) -> String {
    match rows.first() {
        Some(top) => format!(
            "#{} {}  ({})",
            top.rank,
            top.display_title(),
            top.count_label()
        ),
        None => empty_label.into(),
    }
}

pub fn text_body(rows: &[TopRow], empty_label: &str) -> Body {
    Body::Text(TextData {
        value: headline(rows, empty_label),
    })
}

pub fn text_block_body(rows: &[TopRow], empty_label: &str) -> Body {
    Body::TextBlock(TextBlockData {
        lines: lines(rows, empty_label),
    })
}

fn lines(rows: &[TopRow], empty_label: &str) -> Vec<String> {
    if rows.is_empty() {
        return vec![empty_label.into()];
    }
    rows.iter()
        .map(|r| format!("#{} {}  {}", r.rank, r.display_title(), r.count_label()))
        .collect()
}

pub fn markdown_body(rows: &[TopRow], empty_label: &str) -> Body {
    let value = if rows.is_empty() {
        format!("_{empty_label}_")
    } else {
        rows.iter()
            .map(|r| {
                format!(
                    "- **#{} {}** — {}",
                    r.rank,
                    r.display_title(),
                    r.count_label()
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    Body::MarkdownTextBlock(MarkdownTextBlockData { value })
}

pub fn linked_text_body(rows: &[TopRow], empty_label: &str) -> Body {
    let items: Vec<LinkedLine> = if rows.is_empty() {
        vec![LinkedLine {
            text: empty_label.into(),
            url: None,
        }]
    } else {
        rows.iter()
            .map(|r| LinkedLine {
                text: format!("#{} {}  {}", r.rank, r.display_title(), r.count_label()),
                url: Some(r.url.clone()),
            })
            .collect()
    };
    Body::LinkedTextBlock(LinkedTextBlockData { items })
}

pub async fn image_linked_body(rows: &[TopRow]) -> Body {
    let urls: Vec<Option<String>> = rows.iter().map(|r| r.image_url.clone()).collect();
    let paths = thumbnails::download_many(&urls).await;
    let items = rows
        .iter()
        .zip(paths)
        .map(|(r, path)| ImageLinkedItem {
            title: format!("#{} {}", r.rank, r.primary),
            url: Some(r.url.clone()),
            thumbnail_path: path.map(|p| p.to_string_lossy().into_owned()),
            subtitle: subtitle(r),
        })
        .collect();
    Body::ImageLinkedList(ImageLinkedListData { items })
}

fn subtitle(row: &TopRow) -> Option<String> {
    let count = row.count_label();
    match &row.secondary {
        Some(s) if !s.is_empty() => Some(format!("{s}  ·  {count}")),
        _ => Some(count),
    }
}

pub fn entries_body(rows: &[TopRow]) -> Body {
    let items: Vec<Entry> = if rows.is_empty() {
        vec![Entry {
            key: "—".into(),
            value: Some("no scrobbles yet".into()),
            status: None,
        }]
    } else {
        rows.iter()
            .map(|r| Entry {
                key: format!("#{} {}", r.rank, r.display_title()),
                value: Some(r.count_label()),
                status: None,
            })
            .collect()
    };
    Body::Entries(EntriesData { items })
}

/// Top entity's share of the total playcount across the surfaced window.
/// Renderers receive `value` (0..=1) + `label` (the entity name) + `denominator` (the
/// summed playcount, so the gauge can spell "23 of 142" if it wants).
pub fn ratio_body(rows: &[TopRow]) -> Body {
    let total: u64 = rows.iter().map(|r| r.playcount).sum();
    let (value, label, denominator) = match rows.first() {
        Some(top) if total > 0 => (
            top.playcount as f64 / total as f64,
            Some(top.display_title()),
            Some(total),
        ),
        _ => (0.0, None, None),
    };
    Body::Ratio(RatioData {
        value,
        label,
        denominator,
    })
}

pub fn number_series_body(rows: &[TopRow]) -> Body {
    Body::NumberSeries(NumberSeriesData {
        values: rows.iter().map(|r| r.playcount).collect(),
    })
}

pub fn bars_body(rows: &[TopRow]) -> Body {
    Body::Bars(BarsData {
        bars: rows
            .iter()
            .map(|r| Bar {
                label: r.display_title(),
                value: r.playcount,
                // Carry the row's pluralised play-count ("1.5k plays") so the value column
                // reads with units in `list_ranking` instead of the bare `u64`.
                value_label: Some(r.count_label()),
            })
            .collect(),
    })
}

/// Promotes the top entity to the badge label; falls back to a "quiet window" pill so the
/// widget stays visible even when the user has no scrobbles in the chosen window.
pub fn badge_body(rows: &[TopRow], period: Period) -> Body {
    let (status, label) = match rows.first() {
        Some(top) => (Status::Ok, format!("#1 {}", top.primary)),
        None => (Status::Warn, format!("quiet ({})", period.label())),
    };
    Body::Badge(BadgeData { status, label })
}

