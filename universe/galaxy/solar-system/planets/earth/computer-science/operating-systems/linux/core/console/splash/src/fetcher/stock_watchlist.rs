//! `stock_watchlist` — Yahoo Finance v8 chart snapshot for a configurable set of tickers.
//!
//! Safety::Safe because the host (`query1.finance.yahoo.com`) is hardcoded: the user supplies
//! ticker symbols only, both as path segment and as the response keys we read. No API key, no
//! token leaves the machine, and the symbol allowlist (alphanumeric plus `. ^ - =`) plus
//! per-symbol path encoding closes the obvious injection routes.

use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use tokio::task::JoinSet;

use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Bar, BarsData, Body, EntriesData, Entry, LinkedLine, LinkedTextBlockData,
    MarkdownTextBlockData, NumberSeriesData, Payload, PointSeries, PointSeriesData, Status,
    TextBlockData, TextData,
};
use crate::render::Shape;

use super::github::common::cache_key;
use super::{FetchContext, FetchError, Fetcher, Safety};

const API_BASE: &str = "https://query1.finance.yahoo.com/v8/finance/chart";
const QUOTE_PAGE_BASE: &str = "https://finance.yahoo.com/quote";
const USER_AGENT: &str = concat!("splashboard/", env!("CARGO_PKG_VERSION"));
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_BYTES: usize = 5 * 1024 * 1024;
const SPARKLINE_RANGE: &str = "5d";
const SPARKLINE_INTERVAL: &str = "15m";

const DEFAULT_SYMBOLS: &[&str] = &["AAPL", "MSFT"];
/// Cap the symbol list so a misconfig can't fan out arbitrarily large requests.
const MAX_SYMBOLS: usize = 20;
/// Cap chart series so multi-ticker `PointSeries` stays readable in a typical widget slot.
const MAX_SERIES_STOCKS: usize = 5;
/// `Bars` carry abs(% change) × 100 (basis points) since `Bar.value` is `u64`.
const PCT_TO_BP: f64 = 100.0;
/// Volatility threshold (%) for badge / entry status flip from Ok to Warn. Direction-neutral
/// because a watchlist isn't sentiment-bearing — the user might be long or short any ticker.
const VOLATILITY_THRESHOLD: f64 = 5.0;

const SHAPES: &[Shape] = &[
    Shape::Entries,
    Shape::Text,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::LinkedTextBlock,
    Shape::NumberSeries,
    Shape::PointSeries,
    Shape::Bars,
    Shape::Badge,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "symbols",
    type_hint: "list of strings (Yahoo Finance ticker symbols)",
    required: false,
    default: Some("[\"AAPL\", \"MSFT\"]"),
    description: "Yahoo Finance ticker symbols, including suffixes for non-US listings (e.g. `[\"AAPL\", \"7203.T\", \"^GSPC\"]`). Capped at 20 entries.",
}];

pub struct StockWatchlistFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub symbols: Option<Vec<String>>,
}

#[async_trait]
impl Fetcher for StockWatchlistFetcher {
    fn name(&self) -> &str {
        "stock_watchlist"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Yahoo Finance price snapshot for a configurable list of tickers. `Entries` (default) / `Text` / `TextBlock` / `MarkdownTextBlock` / `LinkedTextBlock` summarise spot price + intraday change vs previous close; `NumberSeries` carries the first ticker's 5-day intraday price as cents above the period low (so high-magnitude tickers don't flatten the sparkline); `PointSeries` carries one series per ticker for line / scatter charts; `Bars` ranks tickers by |% change| (basis points); `Badge` flags the top mover. No API key required."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 15
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
        body_for_shape(&sample_snapshot(), shape)
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let symbols = resolve_symbols(opts.symbols.as_deref())?;
        let snapshot = fetch_snapshot(&symbols).await?;
        let shape = ctx.shape.unwrap_or(Shape::Entries);
        let body = body_for_shape(&snapshot, shape).unwrap_or_else(|| entries_body(&snapshot));
        Ok(payload(body))
    }
}

fn body_for_shape(snapshot: &Snapshot, shape: Shape) -> Option<Body> {
    Some(match shape {
        Shape::Entries => entries_body(snapshot),
        Shape::Text => Body::Text(TextData {
            value: text_value(snapshot),
        }),
        Shape::TextBlock => text_block_body(snapshot),
        Shape::MarkdownTextBlock => markdown_block_body(snapshot),
        Shape::LinkedTextBlock => linked_text_block_body(snapshot),
        Shape::NumberSeries => number_series_body(snapshot),
        Shape::PointSeries => point_series_body(snapshot),
        Shape::Bars => bars_body(snapshot),
        Shape::Badge => Body::Badge(badge_for(snapshot)),
        _ => return None,
    })
}

fn resolve_symbols(raw: Option<&[String]>) -> Result<Vec<String>, FetchError> {
    let supplied = match raw {
        Some(items) if !items.is_empty() => items.iter().map(String::as_str).collect::<Vec<_>>(),
        _ => DEFAULT_SYMBOLS.to_vec(),
    };
    let symbols: Vec<String> = supplied
        .iter()
        .filter_map(|s| sanitise_symbol(s))
        .take(MAX_SYMBOLS)
        .collect();
    if symbols.is_empty() {
        return Err(FetchError::Failed(
            "stock_watchlist: `symbols` must contain at least one Yahoo Finance ticker".into(),
        ));
    }
    Ok(symbols)
}

/// Allowlist: ASCII alphanumerics plus `.` (suffix separator like `7203.T`), `^` (indices
/// like `^GSPC`), `-` (`BTC-USD`), `=` (futures / forex like `EURUSD=X`). Locked down to
/// avoid sneaking extra path or query characters past the URL builder.
fn sanitise_symbol(raw: &str) -> Option<String> {
    let symbol = raw.trim().to_uppercase();
    let valid_len = (1..=16).contains(&symbol.len());
    let valid_chars = symbol
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '^' | '-' | '='));
    (valid_len && valid_chars).then_some(symbol)
}

async fn fetch_snapshot(symbols: &[String]) -> Result<Snapshot, FetchError> {
    let mut set: JoinSet<(usize, Result<Option<StockPoint>, FetchError>)> = JoinSet::new();
    for (idx, symbol) in symbols.iter().enumerate() {
        let symbol = symbol.clone();
        set.spawn(async move { (idx, fetch_one(&symbol).await) });
    }
    let mut indexed: Vec<(usize, StockPoint)> = Vec::new();
    let mut last_error: Option<FetchError> = None;
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok((idx, Ok(Some(point)))) => indexed.push((idx, point)),
            Ok((_, Ok(None))) => {}
            Ok((_, Err(err))) => last_error = Some(err),
            Err(join_err) => {
                last_error = Some(FetchError::Failed(format!(
                    "stock_watchlist task failed: {join_err}"
                )))
            }
        }
    }
    if indexed.is_empty() {
        return Err(last_error.unwrap_or_else(|| {
            FetchError::Failed("stock_watchlist: no symbols returned data".into())
        }));
    }
    indexed.sort_by_key(|(idx, _)| *idx);
    Ok(Snapshot {
        stocks: indexed.into_iter().map(|(_, p)| p).collect(),
    })
}

async fn fetch_one(symbol: &str) -> Result<Option<StockPoint>, FetchError> {
    let url = build_url(symbol);
    let res = http()
        .get(&url)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("stock_watchlist request failed: {e}")))?;
    let status = res.status();
    let bytes = res
        .bytes()
        .await
        .map_err(|e| FetchError::Failed(format!("stock_watchlist read body: {e}")))?;
    if bytes.len() > MAX_BYTES {
        return Err(FetchError::Failed(format!(
            "stock_watchlist response too large ({} bytes, cap {MAX_BYTES})",
            bytes.len()
        )));
    }
    // Yahoo serves 404s for delisted / unknown tickers with a JSON error body. Treat that as
    // "skip this row" rather than failing the whole watchlist — typo in one symbol shouldn't
    // sink the rest. Anything else (5xx, 429) propagates up so the runtime surfaces it.
    if status == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !status.is_success() {
        return Err(FetchError::Failed(format!("stock_watchlist {status}")));
    }
    let parsed: ApiResponse = serde_json::from_slice(&bytes)
        .map_err(|e| FetchError::Failed(format!("stock_watchlist json parse: {e}")))?;
    Ok(parsed
        .chart
        .result
        .and_then(|r| r.into_iter().next())
        .and_then(StockPoint::from_api))
}

fn build_url(symbol: &str) -> String {
    format!(
        "{API_BASE}/{encoded}?range={SPARKLINE_RANGE}&interval={SPARKLINE_INTERVAL}",
        encoded = encode_symbol_segment(symbol),
    )
}

/// Allowlist already restricts to `[A-Z0-9.^\-=]`. Of those, `^` and `=` need percent-encoding
/// in a path segment; the rest are safe as-is.
fn encode_symbol_segment(symbol: &str) -> String {
    let mut out = String::with_capacity(symbol.len());
    for c in symbol.chars() {
        match c {
            '^' => out.push_str("%5E"),
            '=' => out.push_str("%3D"),
            c => out.push(c),
        }
    }
    out
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

#[derive(Debug, Deserialize)]
struct ApiResponse {
    chart: ApiChart,
}

#[derive(Debug, Deserialize)]
struct ApiChart {
    #[serde(default)]
    result: Option<Vec<ApiResult>>,
}

#[derive(Debug, Deserialize)]
struct ApiResult {
    meta: ApiMeta,
    #[serde(default)]
    indicators: Option<ApiIndicators>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiMeta {
    symbol: String,
    #[serde(default)]
    currency: Option<String>,
    #[serde(default)]
    regular_market_price: Option<f64>,
    #[serde(default)]
    chart_previous_close: Option<f64>,
    #[serde(default)]
    previous_close: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct ApiIndicators {
    #[serde(default)]
    quote: Vec<ApiQuoteSeries>,
}

#[derive(Debug, Deserialize)]
struct ApiQuoteSeries {
    #[serde(default)]
    close: Vec<Option<f64>>,
}

#[derive(Debug, Clone)]
struct Snapshot {
    stocks: Vec<StockPoint>,
}

#[derive(Debug, Clone)]
struct StockPoint {
    symbol: String,
    currency: String,
    price: f64,
    change_pct: f64,
    sparkline: Vec<f64>,
}

impl StockPoint {
    fn from_api(result: ApiResult) -> Option<Self> {
        let price = result.meta.regular_market_price?;
        let symbol = result.meta.symbol.to_uppercase();
        let prev_close = result
            .meta
            .chart_previous_close
            .or(result.meta.previous_close)
            .filter(|v| v.is_finite() && v.abs() > f64::EPSILON)
            .unwrap_or(price);
        let change_pct = if prev_close.abs() > f64::EPSILON {
            (price / prev_close - 1.0) * 100.0
        } else {
            0.0
        };
        let currency = result
            .meta
            .currency
            .unwrap_or_else(|| "USD".into())
            .to_lowercase();
        let sparkline = result
            .indicators
            .and_then(|i| i.quote.into_iter().next())
            .map(|q| {
                q.close
                    .into_iter()
                    .flatten()
                    .filter(|v| v.is_finite())
                    .collect()
            })
            .unwrap_or_default();
        Some(Self {
            symbol,
            currency,
            price,
            change_pct,
            sparkline,
        })
    }

    fn arrow(&self) -> char {
        if self.change_pct > 0.0 {
            '▲'
        } else if self.change_pct < 0.0 {
            '▼'
        } else {
            '·'
        }
    }
}

fn entries_body(snapshot: &Snapshot) -> Body {
    Body::Entries(EntriesData {
        items: snapshot
            .stocks
            .iter()
            .map(|s| Entry {
                key: s.symbol.clone(),
                value: Some(format!(
                    "{} ({})",
                    format_price(s.price, &s.currency),
                    format_change(s.change_pct),
                )),
                status: Some(status_for_change(s.change_pct)),
            })
            .collect(),
    })
}

fn text_value(snapshot: &Snapshot) -> String {
    match top_mover(snapshot) {
        Some(s) => format!(
            "{} {} {} ({})",
            s.arrow(),
            s.symbol,
            format_price(s.price, &s.currency),
            format_change(s.change_pct),
        ),
        None => "no symbols".into(),
    }
}

fn text_block_body(snapshot: &Snapshot) -> Body {
    Body::TextBlock(TextBlockData {
        lines: snapshot.stocks.iter().map(line_for).collect(),
    })
}

fn markdown_block_body(snapshot: &Snapshot) -> Body {
    let value = snapshot
        .stocks
        .iter()
        .map(|s| {
            format!(
                "- **{}** {} ({})",
                s.symbol,
                format_price(s.price, &s.currency),
                format_change(s.change_pct),
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    Body::MarkdownTextBlock(MarkdownTextBlockData { value })
}

fn linked_text_block_body(snapshot: &Snapshot) -> Body {
    Body::LinkedTextBlock(LinkedTextBlockData {
        items: snapshot
            .stocks
            .iter()
            .map(|s| LinkedLine {
                text: line_for(s),
                url: Some(format!(
                    "{QUOTE_PAGE_BASE}/{}",
                    encode_symbol_segment(&s.symbol)
                )),
            })
            .collect(),
    })
}

/// First ticker's intraday price as cents *above the period minimum*. Same baseline trick as
/// `crypto_watchlist`: `chart_sparkline` normalises against series max with an implicit zero
/// floor, so emitting raw cents flattens the trace for any high-magnitude ticker — BRK-A at
/// \$600k with a 1 % swing collapses into a band glued to the top of the slot. Subtract the
/// period minimum so the variation occupies the full bar height.
fn number_series_body(snapshot: &Snapshot) -> Body {
    let values: Vec<u64> = snapshot
        .stocks
        .first()
        .map(|s| {
            let baseline = s
                .sparkline
                .iter()
                .copied()
                .filter(|p| p.is_finite())
                .fold(f64::INFINITY, f64::min);
            let baseline = if baseline.is_finite() { baseline } else { 0.0 };
            s.sparkline
                .iter()
                .map(|p| ((*p - baseline).max(0.0) * 100.0).round() as u64)
                .collect()
        })
        .unwrap_or_default();
    Body::NumberSeries(NumberSeriesData { values })
}

fn point_series_body(snapshot: &Snapshot) -> Body {
    Body::PointSeries(PointSeriesData {
        series: snapshot
            .stocks
            .iter()
            .take(MAX_SERIES_STOCKS)
            .map(|s| PointSeries {
                name: s.symbol.clone(),
                points: s
                    .sparkline
                    .iter()
                    .enumerate()
                    .map(|(i, p)| (i as f64, *p))
                    .collect(),
            })
            .collect(),
    })
}

fn bars_body(snapshot: &Snapshot) -> Body {
    Body::Bars(BarsData {
        bars: snapshot
            .stocks
            .iter()
            .map(|s| Bar {
                label: format!("{} {}", s.arrow(), s.symbol),
                value: (s.change_pct.abs() * PCT_TO_BP).round() as u64,
                value_label: None,
            })
            .collect(),
    })
}

fn badge_for(snapshot: &Snapshot) -> BadgeData {
    match top_mover(snapshot) {
        Some(s) => BadgeData {
            status: status_for_change(s.change_pct),
            label: format!("{} {} {}", s.arrow(), s.symbol, format_change(s.change_pct)),
        },
        None => BadgeData {
            status: Status::Warn,
            label: "no symbols".into(),
        },
    }
}

fn line_for(s: &StockPoint) -> String {
    format!(
        "{} {}  {}  {}",
        s.arrow(),
        s.symbol,
        format_price(s.price, &s.currency),
        format_change(s.change_pct),
    )
}

fn top_mover(snapshot: &Snapshot) -> Option<&StockPoint> {
    snapshot.stocks.iter().max_by(|a, b| {
        a.change_pct
            .abs()
            .partial_cmp(&b.change_pct.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    })
}

/// Volatility-coded status — large absolute moves flip Ok → Warn so the badge stands out.
/// Direction itself is encoded in the arrow / sign in the label, leaving the status agnostic
/// to whether the user is long or short.
fn status_for_change(change_pct: f64) -> Status {
    if change_pct.abs() >= VOLATILITY_THRESHOLD {
        Status::Warn
    } else {
        Status::Ok
    }
}

fn format_price(value: f64, currency: &str) -> String {
    let amount = format_amount(value);
    match currency_symbol(currency) {
        Some(sym) => format!("{sym}{amount}"),
        None => format!("{amount} {}", currency.to_uppercase()),
    }
}

fn currency_symbol(code: &str) -> Option<&'static str> {
    match code {
        "usd" => Some("$"),
        "eur" => Some("€"),
        "jpy" => Some("¥"),
        "gbp" => Some("£"),
        "krw" => Some("₩"),
        _ => None,
    }
}

/// Locale-agnostic "1,234.56" with adaptive precision: ≥ 1 → 2 decimals, ≥ 0.01 → 4 decimals,
/// otherwise → 6 decimals. Matches `crypto_watchlist::format_amount` so the widgets stack
/// visually consistent in a mixed dashboard.
fn format_amount(value: f64) -> String {
    let abs = value.abs();
    let formatted = if abs == 0.0 {
        "0.00".to_string()
    } else if abs < 0.01 {
        format!("{value:.6}")
    } else if abs < 1.0 {
        format!("{value:.4}")
    } else {
        format!("{value:.2}")
    };
    add_thousands_separators(&formatted)
}

fn add_thousands_separators(formatted: &str) -> String {
    let (int_part, fractional) = formatted.split_once('.').unwrap_or((formatted, ""));
    let (sign, digits) = match int_part.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", int_part),
    };
    let with_sep: String = digits
        .as_bytes()
        .rchunks(3)
        .rev()
        .map(|chunk| std::str::from_utf8(chunk).unwrap_or(""))
        .collect::<Vec<_>>()
        .join(",");
    if fractional.is_empty() {
        format!("{sign}{with_sep}")
    } else {
        format!("{sign}{with_sep}.{fractional}")
    }
}

fn format_change(pct: f64) -> String {
    let sign = if pct >= 0.0 { "+" } else { "" };
    format!("{sign}{pct:.2}%")
}

fn parse_options<T: serde::de::DeserializeOwned + Default>(
    raw: Option<&toml::Value>,
) -> Result<T, String> {
    match raw {
        None => Ok(T::default()),
        Some(value) => value
            .clone()
            .try_into::<T>()
            .map_err(|e| format!("invalid options: {e}")),
    }
}

fn payload(body: Body) -> Payload {
    Payload {
        icon: None,
        status: None,
        format: None,
        body,
    }
}

fn sample_snapshot() -> Snapshot {
    let make_sparkline = |start: f64, drift: f64| -> Vec<f64> {
        (0..130u32)
            .map(|i| start + drift * (i as f64 / 26.0) + ((i as f64 / 8.0).sin() * start * 0.005))
            .collect()
    };
    Snapshot {
        stocks: vec![
            StockPoint {
                symbol: "AAPL".into(),
                currency: "usd".into(),
                price: 192.45,
                change_pct: 1.34,
                sparkline: make_sparkline(190.0, 0.6),
            },
            StockPoint {
                symbol: "MSFT".into(),
                currency: "usd".into(),
                price: 412.18,
                change_pct: -0.62,
                sparkline: make_sparkline(415.0, -0.4),
            },
        ],
    }
}

