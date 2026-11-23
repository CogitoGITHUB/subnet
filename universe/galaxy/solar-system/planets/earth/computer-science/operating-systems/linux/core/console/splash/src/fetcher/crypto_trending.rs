//! `crypto_trending` — top 15 trending coins on CoinGecko, by user search activity.
//!
//! Safety::Safe: both the API host (`api.coingecko.com`) and the coin-page host
//! (`www.coingecko.com`) are hardcoded; the response carries thumbnail URLs on
//! `assets.coingecko.com`, which `thumbnails::download_to_cache` accepts as-is once
//! sniffed for image magic bytes. No API key required.
//!
//! Distinct from the shipped `crypto_watchlist` (a user-curated price snapshot for chosen
//! coins): trending is the inverse — CoinGecko picks the list based on what people are
//! searching, the user supplies only `count` and an optional quote currency.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{
    Bar, BarsData, Body, EntriesData, Entry, ImageLinkedItem, ImageLinkedListData, LinkedLine,
    LinkedTextBlockData, MarkdownTextBlockData, Payload, TextBlockData, TextData,
};
use crate::render::Shape;

use super::github::common::{cache_key, parse_options, payload};
use super::thumbnails;
use super::{FetchContext, FetchError, Fetcher, Safety};

const API_URL: &str = "https://api.coingecko.com/api/v3/search/trending";
const COIN_PAGE_BASE: &str = "https://www.coingecko.com/en/coins";
const USER_AGENT: &str = concat!("splashboard/", env!("CARGO_PKG_VERSION"));
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_BYTES: usize = 5 * 1024 * 1024;

const DEFAULT_COUNT: u32 = 10;
const MIN_COUNT: u32 = 1;
/// CoinGecko `/search/trending` returns at most 15 coins; clamping at the source.
const MAX_COUNT: u32 = 15;
const DEFAULT_VS_CURRENCY: &str = "usd";

const SHAPES: &[Shape] = &[
    Shape::LinkedTextBlock,
    Shape::ImageLinkedList,
    Shape::TextBlock,
    Shape::Text,
    Shape::MarkdownTextBlock,
    Shape::Entries,
    Shape::Bars,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "count",
        type_hint: "integer (1..=15)",
        required: false,
        default: Some("10"),
        description: "Number of trending coins to display.",
    },
    OptionSchema {
        name: "vs_currency",
        type_hint: "string (lowercase 2-5 letter code)",
        required: false,
        default: Some("\"usd\""),
        description: "Quote currency for the 24h-change column (e.g. `usd`, `jpy`, `eur`, `btc`). Letters only; matches `crypto_watchlist`.",
    },
];

pub struct CryptoTrendingFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub count: Option<u32>,
    #[serde(default)]
    pub vs_currency: Option<String>,
}

#[async_trait]
impl Fetcher for CryptoTrendingFetcher {
    fn name(&self) -> &str {
        "crypto_trending"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Top trending coins on CoinGecko (ranked by user search activity over the last 24h). No API key required. Companion to the shipped `crypto_watchlist` — same source, but CoinGecko picks the list instead of the user."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 60
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
        sample_trending_body(shape)
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let count = opts
            .count
            .unwrap_or(DEFAULT_COUNT)
            .clamp(MIN_COUNT, MAX_COUNT) as usize;
        let vs_currency = resolve_vs_currency(opts.vs_currency.as_deref())?;
        let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);
        let coins = fetch_trending(&vs_currency, count).await?;
        let body = render_for_shape(&coins, shape).await;
        Ok(payload(body))
    }
}

fn resolve_vs_currency(raw: Option<&str>) -> Result<String, FetchError> {
    let value = raw
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase)
        .unwrap_or_else(|| DEFAULT_VS_CURRENCY.to_string());
    let valid_len = (2..=5).contains(&value.len());
    if !valid_len || !value.chars().all(|c| c.is_ascii_lowercase()) {
        return Err(FetchError::Failed(format!(
            "crypto_trending: invalid vs_currency `{value}` (lowercase 2-5 letter code)"
        )));
    }
    Ok(value)
}

/// Trending coin as the renderer wants it. `position` is the 0-indexed slot in CoinGecko's
/// response (= trending rank). `price_change_24h` is the 24h % move in the requested quote
/// currency, falling through `None` when CoinGecko hasn't populated that currency yet for a
/// freshly-trending obscure coin.
#[derive(Debug, Clone)]
struct Coin {
    name: String,
    symbol: String,
    slug: String,
    market_cap_rank: Option<i32>,
    price_change_24h: Option<f64>,
    thumb_url: Option<String>,
    position: usize,
}

impl Coin {
    fn url(&self) -> String {
        format!("{COIN_PAGE_BASE}/{}", self.slug)
    }

    fn display_label(&self) -> String {
        format!("{}  {}", self.symbol, self.name)
    }

    /// Single-line score used by Text / LinkedTextBlock / TextBlock / Entries rows: `+5.2%`,
    /// `-2.1%`, or `· trending` when the 24h change isn't available for this quote currency.
    fn score_line(&self) -> String {
        match self.price_change_24h {
            Some(pct) => format!("{pct:+.1}% 24h"),
            None => "· trending".into(),
        }
    }
}

async fn fetch_trending(vs_currency: &str, limit: usize) -> Result<Vec<Coin>, FetchError> {
    let response = fetch_response().await?;
    Ok(response
        .coins
        .into_iter()
        .enumerate()
        .take(limit)
        .map(|(i, entry)| coin_from(i, entry.item, vs_currency))
        .collect())
}

async fn fetch_response() -> Result<TrendingResponse, FetchError> {
    let res = http()
        .get(API_URL)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("crypto_trending request failed: {e}")))?;
    let status = res.status();
    if !status.is_success() {
        return Err(FetchError::Failed(format!("crypto_trending {status}")));
    }
    let bytes = res
        .bytes()
        .await
        .map_err(|e| FetchError::Failed(format!("crypto_trending read body: {e}")))?;
    if bytes.len() > MAX_BYTES {
        return Err(FetchError::Failed(format!(
            "crypto_trending response too large ({} bytes, cap {MAX_BYTES})",
            bytes.len()
        )));
    }
    serde_json::from_slice(&bytes)
        .map_err(|e| FetchError::Failed(format!("crypto_trending json parse: {e}")))
}

fn coin_from(position: usize, item: ApiItem, vs_currency: &str) -> Coin {
    let price_change_24h = item
        .data
        .as_ref()
        .and_then(|d| d.price_change_percentage_24h.as_ref())
        .and_then(|map| map.get(vs_currency).copied());
    Coin {
        name: item.name,
        symbol: item.symbol.to_uppercase(),
        slug: item.slug,
        market_cap_rank: item.market_cap_rank,
        price_change_24h,
        thumb_url: item.thumb.filter(|u| !u.is_empty()),
        position,
    }
}

fn http() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .user_agent(USER_AGENT)
            .timeout(REQUEST_TIMEOUT)
            .gzip(true)
            .build()
            .expect("reqwest client should build with default config")
    })
}

async fn render_for_shape(coins: &[Coin], shape: Shape) -> Body {
    if matches!(shape, Shape::ImageLinkedList) {
        let paths = resolve_thumbnails(coins).await;
        return image_linked_body(coins, &paths);
    }
    render_sync(coins, shape)
}

async fn resolve_thumbnails(coins: &[Coin]) -> Vec<Option<PathBuf>> {
    let urls: Vec<Option<String>> = coins.iter().map(|c| c.thumb_url.clone()).collect();
    thumbnails::download_many(&urls).await
}

fn render_sync(coins: &[Coin], shape: Shape) -> Body {
    match shape {
        Shape::Text => Body::Text(TextData {
            value: headline(coins),
        }),
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: coins.iter().map(row_line).collect(),
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: markdown_body(coins),
        }),
        Shape::Entries => Body::Entries(EntriesData {
            items: coins.iter().map(entry_row).collect(),
        }),
        Shape::Bars => Body::Bars(BarsData {
            bars: coins.iter().map(rank_bar).collect(),
        }),
        _ => Body::LinkedTextBlock(LinkedTextBlockData {
            items: coins
                .iter()
                .map(|c| LinkedLine {
                    text: row_line(c),
                    url: Some(c.url()),
                })
                .collect(),
        }),
    }
}

fn image_linked_body(coins: &[Coin], paths: &[Option<PathBuf>]) -> Body {
    Body::ImageLinkedList(ImageLinkedListData {
        items: coins
            .iter()
            .enumerate()
            .map(|(i, c)| ImageLinkedItem {
                title: c.display_label(),
                url: Some(c.url()),
                thumbnail_path: paths
                    .get(i)
                    .and_then(|p| p.as_ref())
                    .map(|p| p.to_string_lossy().into_owned()),
                subtitle: Some(subtitle_line(c)),
            })
            .collect(),
    })
}

fn headline(coins: &[Coin]) -> String {
    coins
        .first()
        .map(|c| format!("#1 {}  {}", c.display_label(), c.score_line()))
        .unwrap_or_else(|| "(no trending coins)".into())
}

fn row_line(coin: &Coin) -> String {
    format!(
        "#{} {}  {}",
        coin.position + 1,
        coin.display_label(),
        coin.score_line()
    )
}

fn subtitle_line(coin: &Coin) -> String {
    let mut s = format!("#{} trending · {}", coin.position + 1, coin.score_line());
    if let Some(rank) = coin.market_cap_rank {
        s.push_str(&format!("  · mcap #{rank}"));
    }
    s
}

fn markdown_body(coins: &[Coin]) -> String {
    coins
        .iter()
        .map(|c| {
            format!(
                "- **#{} {}** — {}  ([page]({}))",
                c.position + 1,
                c.display_label(),
                c.score_line(),
                c.url()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn entry_row(coin: &Coin) -> Entry {
    Entry {
        key: format!("#{} {}", coin.position + 1, coin.display_label()),
        value: Some(coin.score_line()),
        status: None,
    }
}

/// Trending rank as `(count - position)` so the top coin gets the tallest bar. Coingecko's
/// own `score` field is just `0..N` ordering and decays to zero for the lower half — using
/// `count - position` keeps the chart legible regardless of slot width.
fn rank_bar(coin: &Coin) -> Bar {
    Bar {
        label: coin.display_label(),
        value: (MAX_COUNT as usize - coin.position).max(1) as u64,
        value_label: None,
    }
}

fn sample_trending_body(shape: Shape) -> Option<Body> {
    Some(match shape {
        Shape::LinkedTextBlock => crate::samples::linked_text_block(&[
            (
                "#1 BTC  Bitcoin  +5.2% 24h",
                Some("https://www.coingecko.com/en/coins/bitcoin"),
            ),
            (
                "#2 ETH  Ethereum  +3.1% 24h",
                Some("https://www.coingecko.com/en/coins/ethereum"),
            ),
        ]),
        Shape::ImageLinkedList => crate::samples::image_linked_list(&[
            (
                "BTC  Bitcoin",
                Some("https://www.coingecko.com/en/coins/bitcoin"),
                None,
                Some("#1 trending · +5.2% 24h  · mcap #1"),
            ),
            (
                "ETH  Ethereum",
                Some("https://www.coingecko.com/en/coins/ethereum"),
                None,
                Some("#2 trending · +3.1% 24h  · mcap #2"),
            ),
        ]),
        Shape::TextBlock => crate::samples::text_block(&[
            "#1 BTC  Bitcoin  +5.2% 24h",
            "#2 ETH  Ethereum  +3.1% 24h",
        ]),
        Shape::Text => crate::samples::text("#1 BTC  Bitcoin  +5.2% 24h"),
        Shape::MarkdownTextBlock => crate::samples::markdown(
            "- **#1 BTC  Bitcoin** — +5.2% 24h  ([page](https://www.coingecko.com/en/coins/bitcoin))\n- **#2 ETH  Ethereum** — +3.1% 24h  ([page](https://www.coingecko.com/en/coins/ethereum))",
        ),
        Shape::Entries => crate::samples::entries(&[
            ("#1 BTC  Bitcoin", "+5.2% 24h"),
            ("#2 ETH  Ethereum", "+3.1% 24h"),
        ]),
        Shape::Bars => crate::samples::bars(&[("BTC  Bitcoin", 15), ("ETH  Ethereum", 14)]),
        _ => return None,
    })
}

#[derive(Debug, Deserialize)]
struct TrendingResponse {
    coins: Vec<TrendingEntry>,
}

#[derive(Debug, Deserialize)]
struct TrendingEntry {
    item: ApiItem,
}

#[derive(Debug, Deserialize)]
struct ApiItem {
    name: String,
    symbol: String,
    slug: String,
    #[serde(default)]
    market_cap_rank: Option<i32>,
    #[serde(default)]
    thumb: Option<String>,
    #[serde(default)]
    data: Option<ApiData>,
}

#[derive(Debug, Deserialize)]
struct ApiData {
    #[serde(default)]
    price_change_percentage_24h: Option<std::collections::HashMap<String, f64>>,
}

