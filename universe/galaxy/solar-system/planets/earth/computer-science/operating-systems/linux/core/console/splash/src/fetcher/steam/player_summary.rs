//! `steam_player_summary` — the configured Steam user's profile snapshot: persona name,
//! Steam level, online status, currently-playing game, and last-logoff timestamp.
//!
//! Reads `ISteamUser/GetPlayerSummaries/v2` and `IPlayerService/GetSteamLevel/v1` in parallel.
//! Same single-read-many-shapes pattern as `lastfm` family's user-scoped fetchers: every shape
//! reformats the same snapshot.

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use serde::Deserialize;

use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::steam::client;
use crate::fetcher::thumbnails;
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Body, EntriesData, Entry, ImageData, ImageLinkedItem, ImageLinkedListData,
    LinkedLine, LinkedTextBlockData, MarkdownTextBlockData, Payload, Status, TextBlockData,
    TextData,
};
use crate::render::Shape;

const SHAPES: &[Shape] = &[
    Shape::TextBlock,
    Shape::Text,
    Shape::MarkdownTextBlock,
    Shape::LinkedTextBlock,
    Shape::ImageLinkedList,
    Shape::Entries,
    Shape::Badge,
    Shape::Image,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "steam_id",
    type_hint: "string (Steam64 id)",
    required: false,
    default: None,
    description: "Steam64 id of the profile to read. Falls back to the `STEAM_ID` env var when omitted.",
}];

pub struct SteamPlayerSummary;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    pub steam_id: Option<String>,
}

#[async_trait]
impl Fetcher for SteamPlayerSummary {
    fn name(&self) -> &str {
        "steam_player_summary"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "The configured Steam user's profile snapshot: persona name, level, online status, currently-playing game, and last-logoff timestamp. Reads `GetPlayerSummaries` and `GetSteamLevel` in parallel."
    }
    fn refresh_interval(&self) -> u64 {
        30 * 60
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
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let steam_id = client::resolve_steam_id(opts.steam_id.as_deref())?;
        let shape = ctx.shape.unwrap_or(Shape::TextBlock);

        let snapshot = fetch_snapshot(&steam_id).await?;
        Ok(payload(render_body(&snapshot, shape).await))
    }
}

#[derive(Debug, Clone)]
struct Snapshot {
    steam_id: String,
    persona: String,
    profile_url: String,
    avatar_url: Option<String>,
    state: PersonaState,
    current_game: Option<String>,
    last_logoff: Option<i64>,
    level: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PersonaState {
    Offline,
    Online,
    Busy,
    Away,
    Snooze,
    LookingToTrade,
    LookingToPlay,
    InGame,
}

impl PersonaState {
    fn from_code(code: u8, in_game: bool) -> Self {
        if in_game {
            return Self::InGame;
        }
        match code {
            1 => Self::Online,
            2 => Self::Busy,
            3 => Self::Away,
            4 => Self::Snooze,
            5 => Self::LookingToTrade,
            6 => Self::LookingToPlay,
            _ => Self::Offline,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Offline => "offline",
            Self::Online => "online",
            Self::Busy => "busy",
            Self::Away => "away",
            Self::Snooze => "snooze",
            Self::LookingToTrade => "trading",
            Self::LookingToPlay => "looking to play",
            Self::InGame => "in game",
        }
    }

    fn badge_status(self) -> Status {
        match self {
            Self::Online | Self::InGame | Self::LookingToTrade | Self::LookingToPlay => Status::Ok,
            Self::Busy | Self::Away | Self::Snooze | Self::Offline => Status::Warn,
        }
    }

    /// Per-row `Entries` status pip. Offline returns `None` so a fresh-spawned offline profile
    /// doesn't decorate the status row with a yellow pip — only "actively in something other
    /// than online" surfaces a warning.
    fn entry_status(self) -> Option<Status> {
        match self {
            Self::Offline => None,
            other => Some(other.badge_status()),
        }
    }
}

async fn fetch_snapshot(steam_id: &str) -> Result<Snapshot, FetchError> {
    let (player_res, level_res) = tokio::join!(fetch_player(steam_id), fetch_level(steam_id));
    let player = player_res?;
    let level = level_res.ok().flatten();
    Ok(Snapshot {
        steam_id: steam_id.to_string(),
        persona: player.personaname,
        profile_url: player.profileurl,
        avatar_url: Some(player.avatarfull).filter(|s| !s.is_empty()),
        state: PersonaState::from_code(player.personastate, player.gameid.is_some()),
        current_game: player
            .gameextrainfo
            .filter(|s| !s.is_empty())
            .or_else(|| player.gameid.map(|id| format!("app/{id}"))),
        last_logoff: player.lastlogoff,
        level,
    })
}

async fn fetch_player(steam_id: &str) -> Result<RawPlayer, FetchError> {
    let raw: PlayerSummariesResponse = client::get_json(
        "ISteamUser/GetPlayerSummaries/v2/",
        &[("steamids", steam_id)],
    )
    .await?;
    raw.response
        .players
        .into_iter()
        .next()
        .ok_or_else(|| FetchError::Failed(format!("steam: no player for id {steam_id}")))
}

async fn fetch_level(steam_id: &str) -> Result<Option<u32>, FetchError> {
    let raw: SteamLevelResponse =
        client::get_json("IPlayerService/GetSteamLevel/v1/", &[("steamid", steam_id)]).await?;
    Ok(raw.response.player_level)
}

async fn render_body(snap: &Snapshot, shape: Shape) -> Body {
    match shape {
        Shape::Text => text_body(snap),
        Shape::TextBlock => text_block_body(snap),
        Shape::MarkdownTextBlock => markdown_body(snap),
        Shape::LinkedTextBlock => linked_text_body(snap),
        Shape::ImageLinkedList => image_linked_body(snap).await,
        Shape::Entries => entries_body(snap),
        Shape::Badge => badge_body(snap),
        Shape::Image => image_body(snap).await,
        _ => text_body(snap),
    }
}

fn headline(snap: &Snapshot) -> String {
    let mut parts = vec![snap.persona.clone()];
    if let Some(lv) = snap.level {
        parts.push(format!("Lv {lv}"));
    }
    if let Some(game) = &snap.current_game {
        parts.push(format!("in {game}"));
    } else {
        parts.push(snap.state.label().into());
    }
    parts.join(" · ")
}

fn text_body(snap: &Snapshot) -> Body {
    Body::Text(TextData {
        value: headline(snap),
    })
}

fn text_block_body(snap: &Snapshot) -> Body {
    Body::TextBlock(TextBlockData { lines: lines(snap) })
}

fn markdown_body(snap: &Snapshot) -> Body {
    let value = lines(snap)
        .into_iter()
        .enumerate()
        .map(|(i, line)| if i == 0 { format!("**{line}**") } else { line })
        .collect::<Vec<_>>()
        .join("\n");
    Body::MarkdownTextBlock(MarkdownTextBlockData { value })
}

fn lines(snap: &Snapshot) -> Vec<String> {
    let mut lines = vec![snap.persona.clone()];
    if let Some(lv) = snap.level {
        lines.push(format!("Level {lv}"));
    }
    lines.push(format!("Status: {}", snap.state.label()));
    if let Some(game) = &snap.current_game {
        lines.push(format!("Playing: {game}"));
    } else if let Some(ts) = snap.last_logoff {
        lines.push(format!("Last seen: {}", format_last_logoff(ts)));
    }
    lines
}

fn linked_text_body(snap: &Snapshot) -> Body {
    Body::LinkedTextBlock(LinkedTextBlockData {
        items: vec![LinkedLine {
            text: headline(snap),
            url: Some(snap.profile_url.clone()),
        }],
    })
}

async fn image_linked_body(snap: &Snapshot) -> Body {
    let path = match snap.avatar_url.as_deref() {
        Some(url) => thumbnails::download_to_cache(url)
            .await
            .ok()
            .flatten()
            .map(|p| p.to_string_lossy().into_owned()),
        None => None,
    };
    Body::ImageLinkedList(ImageLinkedListData {
        items: vec![ImageLinkedItem {
            title: snap.persona.clone(),
            url: Some(snap.profile_url.clone()),
            thumbnail_path: path,
            subtitle: Some(subtitle(snap)),
        }],
    })
}

fn subtitle(snap: &Snapshot) -> String {
    match (&snap.level, &snap.current_game) {
        (Some(lv), Some(game)) => format!("Lv {lv} · in {game}"),
        (Some(lv), None) => format!("Lv {lv} · {}", snap.state.label()),
        (None, Some(game)) => format!("in {game}"),
        (None, None) => snap.state.label().into(),
    }
}

fn entries_body(snap: &Snapshot) -> Body {
    let mut items = vec![Entry {
        key: "persona".into(),
        value: Some(snap.persona.clone()),
        status: None,
    }];
    if let Some(lv) = snap.level {
        items.push(Entry {
            key: "level".into(),
            value: Some(lv.to_string()),
            status: None,
        });
    }
    items.push(Entry {
        key: "status".into(),
        value: Some(snap.state.label().into()),
        status: snap.state.entry_status(),
    });
    if let Some(game) = &snap.current_game {
        items.push(Entry {
            key: "playing".into(),
            value: Some(game.clone()),
            status: None,
        });
    }
    if let Some(ts) = snap.last_logoff {
        items.push(Entry {
            key: "last seen".into(),
            value: Some(format_last_logoff(ts)),
            status: None,
        });
    }
    items.push(Entry {
        key: "steam id".into(),
        value: Some(snap.steam_id.clone()),
        status: None,
    });
    Body::Entries(EntriesData { items })
}

fn badge_body(snap: &Snapshot) -> Body {
    let label = match &snap.current_game {
        Some(game) => format!("in {game}"),
        None => snap.state.label().into(),
    };
    Body::Badge(BadgeData {
        status: snap.state.badge_status(),
        label,
    })
}

async fn image_body(snap: &Snapshot) -> Body {
    let path = match snap.avatar_url.as_deref() {
        Some(url) => thumbnails::download_to_cache(url)
            .await
            .ok()
            .flatten()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
        None => String::new(),
    };
    Body::Image(ImageData { path })
}

fn format_last_logoff(ts: i64) -> String {
    // The `lastlogoff` field is unix seconds UTC; format it as an ISO date so cached payloads
    // don't bake in a stale relative-time string. Renderers that want "3h ago" can re-derive
    // it from a Timeline shape — this fetcher only emits absolute snapshots.
    match Utc.timestamp_opt(ts, 0).single() {
        Some(dt) => dt.format("%Y-%m-%d").to_string(),
        None => format!("ts:{ts}"),
    }
}

#[derive(Debug, Deserialize)]
struct PlayerSummariesResponse {
    response: PlayerSummariesBody,
}

#[derive(Debug, Default, Deserialize)]
struct PlayerSummariesBody {
    #[serde(default)]
    players: Vec<RawPlayer>,
}

#[derive(Debug, Deserialize)]
struct RawPlayer {
    #[serde(default)]
    personaname: String,
    #[serde(default)]
    profileurl: String,
    #[serde(default)]
    avatarfull: String,
    #[serde(default)]
    personastate: u8,
    #[serde(default)]
    lastlogoff: Option<i64>,
    #[serde(default)]
    gameid: Option<String>,
    #[serde(default)]
    gameextrainfo: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SteamLevelResponse {
    response: SteamLevelBody,
}

#[derive(Debug, Default, Deserialize)]
struct SteamLevelBody {
    #[serde(default)]
    player_level: Option<u32>,
}

