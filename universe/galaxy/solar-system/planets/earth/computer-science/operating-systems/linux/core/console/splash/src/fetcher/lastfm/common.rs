//! Cross-cutting helpers for the `lastfm_*` family: image-URL picking, count parsing,
//! human-readable formatting.

use serde::Deserialize;

/// Per-row image descriptor returned by Last.fm. The JSON shape is `{"#text": "...",
/// "size": "..."}`; sizes are `small | medium | large | extralarge | mega`. Entries are
/// almost always present even when the underlying entity has no artwork — in that case
/// Last.fm serves a known placeholder hash that [`best_image`] filters out.
#[derive(Debug, Clone, Deserialize)]
pub struct ImageEntry {
    #[serde(rename = "#text", default)]
    pub url: String,
    #[serde(default)]
    pub size: String,
}

/// Largest non-placeholder image URL, or `None` when every entry is empty or the placeholder.
///
/// Last.fm tags artwork-less entities (track / artist / album) with a fixed gray-square hash
/// `2a96cbd8b46e442fc41c2b86b821562f`. Without filtering, an `ImageLinkedList` of those rows
/// would surface the same placeholder thumbnail on every row, which is worse than no
/// thumbnails at all.
pub fn best_image(images: &[ImageEntry]) -> Option<String> {
    const PLACEHOLDER: &str = "2a96cbd8b46e442fc41c2b86b821562f";
    const PRIORITY: &[&str] = &["mega", "extralarge", "large", "medium", "small"];
    PRIORITY
        .iter()
        .find_map(|size| {
            images.iter().find(|img| {
                img.size == *size && !img.url.is_empty() && !img.url.contains(PLACEHOLDER)
            })
        })
        .map(|img| img.url.clone())
}

/// Parse a Last.fm count field. Counts come back as JSON strings (`"42"`) for historical
/// reasons; anything that can't parse silently collapses to zero rather than failing the
/// entire fetch.
pub fn parse_count(raw: &str) -> u64 {
    raw.parse().unwrap_or(0)
}

/// Human-readable count: `1234` -> `"1.2k"`, `2_100_000` -> `"2.1M"`. Mirrors the
/// `huggingface_trending` convention so the catalog reads consistently.
pub fn format_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// `"play"` vs `"plays"` — small enough that inlining at every call site would obscure intent.
pub fn plays_word(n: u64) -> &'static str {
    if n == 1 { "play" } else { "plays" }
}

