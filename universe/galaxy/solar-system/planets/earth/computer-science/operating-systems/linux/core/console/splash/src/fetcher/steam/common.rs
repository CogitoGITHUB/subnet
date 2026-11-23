//! Cross-cutting helpers for the `steam_*` family: appid-derived URLs, playtime formatting,
//! count units.

/// Steam store landing page for an app: `https://store.steampowered.com/app/<appid>/`.
pub fn store_url(appid: u32) -> String {
    format!("https://store.steampowered.com/app/{appid}/")
}

/// CDN header image for an app — the wide "capsule" graphic Valve serves on every store page.
/// Path is stable for every released app; missing images 404 silently, which the thumbnail
/// fetcher already handles.
pub fn header_image_url(appid: u32) -> String {
    format!("https://cdn.akamai.steamstatic.com/steam/apps/{appid}/header.jpg")
}

/// `4830` minutes -> `"80h"`. `45` minutes -> `"45m"`. Anything ≥ 1 hour rounds down to whole
/// hours so the row stays narrow; the catalog's recently-played / owned-games widgets read as
/// "hours played", not "minutes played".
pub fn format_minutes(minutes: u32) -> String {
    if minutes >= 60 {
        format!("{}h", minutes / 60)
    } else {
        format!("{minutes}m")
    }
}

/// Human-readable concurrent-player count: `1234` -> `"1.2k"`, `2_100_000` -> `"2.1M"`.
/// Mirrors `lastfm::common::format_count` so the catalog reads consistently across families.
pub fn format_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// `"player"` vs `"players"` — kept inline-able but spelled out so call sites read naturally.
pub fn players_word(n: u64) -> &'static str {
    if n == 1 { "player" } else { "players" }
}

