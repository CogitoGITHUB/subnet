//! `claude_subscription` — Claude (Max plan) subscription utilisation as Claude Code sees it.
//!
//! Reads the OAuth token at `~/.claude/.credentials.json` and queries the **undocumented**
//! endpoint `GET https://api.anthropic.com/api/oauth/usage` with the `oauth-2025-04-20` beta
//! header (see the catalog body and the [`ohugonnot/claude-code-statusline`] reference).
//! The response carries one entry per usage window — `five_hour`, `seven_day`,
//! `seven_day_sonnet`, `seven_day_opus`, `seven_day_omelette` (the "Claude Design" pool in the
//! claude.ai UI), and a handful of internal codenames — most of which are `null` for any one
//! account. We walk the response generically, keep every non-null window in a canonical order,
//! and label known keys with the UI string the user sees on claude.ai/account.
//!
//! Undocumented = fragile: every failure path short-circuits to [`FetchError`] so the splash
//! still renders a placeholder, never panics.
//!
//! `Safety::Safe` — host is hardcoded, the OAuth token only travels to `api.anthropic.com`.
//!
//! [`ohugonnot/claude-code-statusline`]: https://github.com/ohugonnot/claude-code-statusline/blob/main/statusline.sh

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;

use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Bar, BarsData, Body, EntriesData, Entry, MarkdownTextBlockData, Payload, RatioData,
    Status, TextBlockData, TextData, TimelineData, TimelineEvent,
};
use crate::render::Shape;

const SHAPES: &[Shape] = &[
    Shape::Ratio,
    Shape::Text,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::Entries,
    Shape::Bars,
    Shape::Badge,
    Shape::Timeline,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "window",
    type_hint: "string (response key — e.g. \"five_hour\" / \"seven_day\" / \"seven_day_sonnet\" / \"seven_day_omelette\")",
    required: false,
    default: Some("\"five_hour\""),
    description: "Which window the single-value shapes (`Ratio`, `Badge`) report. Multi-row shapes always list every non-null window the response carries.",
}];

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const USER_AGENT: &str = concat!("splashboard/", env!("CARGO_PKG_VERSION"));

pub struct ClaudeSubscription;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    pub window: Option<String>,
}

#[async_trait]
impl Fetcher for ClaudeSubscription {
    fn name(&self) -> &str {
        "claude_subscription"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Claude (Max plan) subscription utilisation as seen by Claude Code's OAuth credentials. Reads `~/.claude/.credentials.json` and queries the undocumented `oauth/usage` endpoint, exposing every window the response carries (5-hour, 7-day, per-model and pool-specific siblings)."
    }
    fn refresh_interval(&self) -> u64 {
        15 * 60
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
        let target = normalise_window_key(opts.window.as_deref());
        let shape = ctx.shape.unwrap_or(Shape::Ratio);
        let token = load_oauth_token()?;
        let snapshot = fetch_snapshot(&token).await?;
        Ok(payload(render_body(&snapshot, &target, shape)))
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

/// User-facing strings for the response's window keys, in claude.ai/account language. Unknown
/// keys fall through [`humanize_key`].
fn known_label(key: &str) -> Option<&'static str> {
    match key {
        "five_hour" => Some("5h"),
        "seven_day" => Some("7d"),
        "seven_day_opus" => Some("7d Opus"),
        "seven_day_sonnet" => Some("7d Sonnet"),
        "seven_day_oauth_apps" => Some("7d OAuth apps"),
        "seven_day_cowork" => Some("7d Cowork"),
        "seven_day_omelette" => Some("Claude Design"),
        "seven_day_omelette_promotional" => Some("Claude Design (promo)"),
        "omelette_promotional" => Some("Claude Design (promo)"),
        "tangelo" => Some("Tangelo"),
        "iguana_necktie" => Some("Iguana Necktie"),
        _ => None,
    }
}

fn label_for(key: &str) -> String {
    known_label(key)
        .map(str::to_string)
        .unwrap_or_else(|| humanize_key(key))
}

/// `seven_day_some_thing` → `7d some thing`, `tangelo` → `Tangelo`. Capitalises the first letter
/// of the first word but otherwise keeps lowercase so multi-word codenames stay scannable.
fn humanize_key(key: &str) -> String {
    if key.is_empty() {
        return String::new();
    }
    let normalised = key.replace("seven_day", "7d").replace('_', " ");
    let mut chars = normalised.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Canonical sort key. Lower wins. Keeps `5h` first, then `7d (all)`, then `7d Sonnet` /
/// `7d Opus`, other `seven_day_*` next, codenames last (alphabetical).
fn sort_priority(key: &str) -> (u8, &str) {
    match key {
        "five_hour" => (0, key),
        "seven_day" => (1, key),
        "seven_day_sonnet" => (2, key),
        "seven_day_opus" => (3, key),
        "seven_day_omelette" => (4, key),
        "seven_day_omelette_promotional" => (5, key),
        "omelette_promotional" => (5, key),
        _ if key.starts_with("seven_day_") => (6, key),
        _ => (7, key),
    }
}

fn normalise_window_key(raw: Option<&str>) -> String {
    let raw = raw.unwrap_or("five_hour").trim();
    // Accept a few friendly aliases for the keys users are most likely to type by hand.
    match raw {
        "" | "5h" => "five_hour".into(),
        "7d" => "seven_day".into(),
        "7d_sonnet" | "7d-sonnet" => "seven_day_sonnet".into(),
        "7d_opus" | "7d-opus" => "seven_day_opus".into(),
        "claude_design" | "design" => "seven_day_omelette".into(),
        other => other.into(),
    }
}

fn credentials_path() -> Result<PathBuf, FetchError> {
    if let Ok(p) = std::env::var("SPLASH_CLAUDE_CREDENTIALS") {
        return Ok(PathBuf::from(p));
    }
    dirs::home_dir()
        .map(|h| h.join(".claude").join(".credentials.json"))
        .ok_or_else(|| FetchError::Failed("could not resolve $HOME for Claude credentials".into()))
}

#[derive(Deserialize)]
struct CredentialsFile {
    #[serde(rename = "claudeAiOauth", default)]
    oauth: Option<OauthEntry>,
}

#[derive(Deserialize)]
struct OauthEntry {
    #[serde(rename = "accessToken", default)]
    access_token: String,
}

fn load_oauth_token() -> Result<String, FetchError> {
    let path = credentials_path()?;
    let bytes = std::fs::read(&path)
        .map_err(|e| FetchError::Failed(format!("read {}: {e}", path.display())))?;
    let creds: CredentialsFile = serde_json::from_slice(&bytes)
        .map_err(|e| FetchError::Failed(format!("parse {}: {e}", path.display())))?;
    let token = creds
        .oauth
        .map(|o| o.access_token)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| {
            FetchError::Failed("no `claudeAiOauth.accessToken` in credentials".into())
        })?;
    Ok(token)
}

#[derive(Debug, Clone, Default)]
struct Snapshot {
    windows: Vec<NamedWindow>,
}

#[derive(Debug, Clone)]
struct NamedWindow {
    key: String,
    label: String,
    state: WindowState,
}

#[derive(Debug, Clone)]
struct WindowState {
    utilization: f64,
    resets_at: Option<DateTime<Utc>>,
}

impl Snapshot {
    fn find(&self, key: &str) -> Option<&NamedWindow> {
        self.windows.iter().find(|w| w.key == key)
    }
}

async fn fetch_snapshot(token: &str) -> Result<Snapshot, FetchError> {
    // The endpoint is undocumented; upstream tooling (claude-code-statusline) calls it as GET
    // with the OAuth bearer + beta header. POSTing here returns 405 Method Not Allowed.
    let res = http()
        .get(USAGE_URL)
        .bearer_auth(token)
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("content-type", "application/json")
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("claude_subscription request: {e}")))?;
    let status = res.status();
    let bytes = res
        .bytes()
        .await
        .map_err(|e| FetchError::Failed(format!("read body: {e}")))?;
    if !status.is_success() {
        return Err(FetchError::Failed(format!(
            "claude_subscription HTTP {status}"
        )));
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| FetchError::Failed(format!("parse oauth/usage: {e}")))?;
    Ok(parse_snapshot(&value))
}

fn parse_snapshot(value: &serde_json::Value) -> Snapshot {
    let Some(obj) = value.as_object() else {
        return Snapshot::default();
    };
    let mut windows: Vec<NamedWindow> = obj
        .iter()
        .filter_map(|(key, val)| parse_window_entry(key, val))
        .collect();
    windows.sort_by(|a, b| sort_priority(&a.key).cmp(&sort_priority(&b.key)));
    Snapshot { windows }
}

fn parse_window_entry(key: &str, val: &serde_json::Value) -> Option<NamedWindow> {
    // `extra_usage` carries a different schema (is_enabled / monthly_limit / used_credits …);
    // not part of the window family — skip it. Future enhancement can surface it as a separate
    // shape if anyone uses the paid extra-credit pool.
    if key == "extra_usage" {
        return None;
    }
    let obj = val.as_object()?;
    let util = obj.get("utilization").and_then(|v| v.as_f64())?;
    let resets_at = obj
        .get("resets_at")
        .and_then(|v| v.as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc));
    Some(NamedWindow {
        key: key.to_string(),
        label: label_for(key),
        state: WindowState {
            // Endpoint returns 0..100 percent — normalise to 0..1, allow 1.5 for over-quota so a
            // blown window still draws distinctly rather than clipping to "full".
            utilization: (util / 100.0).clamp(0.0, 1.5),
            resets_at,
        },
    })
}

fn render_body(snap: &Snapshot, target: &str, shape: Shape) -> Body {
    match shape {
        Shape::Ratio => ratio_body(snap, target),
        Shape::Text => text_body(snap),
        Shape::TextBlock => text_block_body(snap),
        Shape::MarkdownTextBlock => markdown_body(snap),
        Shape::Entries => entries_body(snap),
        Shape::Bars => bars_body(snap),
        Shape::Badge => badge_body(snap, target),
        Shape::Timeline => timeline_body(snap),
        _ => text_body(snap),
    }
}

fn ratio_body(snap: &Snapshot, target: &str) -> Body {
    let label = known_label(target).map(str::to_string).unwrap_or_else(|| {
        snap.find(target)
            .map(|w| w.label.clone())
            .unwrap_or_else(|| humanize_key(target))
    });
    let Some(window) = snap.find(target) else {
        return Body::Ratio(RatioData {
            value: 0.0,
            label: Some(format!("{label} · n/a")),
            denominator: Some(100),
        });
    };
    let reset = window
        .state
        .resets_at
        .map(format_reset)
        .unwrap_or_else(|| "n/a".into());
    Body::Ratio(RatioData {
        value: window.state.utilization.clamp(0.0, 1.0),
        label: Some(format!("{label} · resets {reset}")),
        denominator: Some(100),
    })
}

fn text_body(snap: &Snapshot) -> Body {
    let parts: Vec<String> = snap
        .windows
        .iter()
        .map(|w| format!("{} {}", w.label, percent(w.state.utilization)))
        .collect();
    Body::Text(TextData {
        value: if parts.is_empty() {
            "claude subscription: n/a".into()
        } else {
            parts.join(" · ")
        },
    })
}

fn text_block_body(snap: &Snapshot) -> Body {
    let lines: Vec<String> = snap.windows.iter().map(format_window_line).collect();
    Body::TextBlock(TextBlockData {
        lines: if lines.is_empty() {
            vec!["claude subscription: n/a".into()]
        } else {
            lines
        },
    })
}

fn markdown_body(snap: &Snapshot) -> Body {
    let lines: Vec<String> = snap
        .windows
        .iter()
        .map(|w| {
            let reset = w
                .state
                .resets_at
                .map(format_reset)
                .unwrap_or_else(|| "n/a".into());
            format!(
                "- **{}** — {} (resets {reset})",
                w.label,
                percent(w.state.utilization),
            )
        })
        .collect();
    Body::MarkdownTextBlock(MarkdownTextBlockData {
        value: if lines.is_empty() {
            "_claude subscription unavailable_".into()
        } else {
            lines.join("\n")
        },
    })
}

fn entries_body(snap: &Snapshot) -> Body {
    let items: Vec<Entry> = snap
        .windows
        .iter()
        .map(|w| Entry {
            key: w.label.clone(),
            value: Some(format!(
                "{} (resets {})",
                percent(w.state.utilization),
                w.state
                    .resets_at
                    .map(format_reset)
                    .unwrap_or_else(|| "n/a".into()),
            )),
            status: Some(status_for(w.state.utilization)),
        })
        .collect();
    Body::Entries(EntriesData {
        items: if items.is_empty() {
            vec![Entry {
                key: "claude".into(),
                value: Some("subscription unavailable".into()),
                status: Some(Status::Warn),
            }]
        } else {
            items
        },
    })
}

fn bars_body(snap: &Snapshot) -> Body {
    let bars: Vec<Bar> = snap
        .windows
        .iter()
        .map(|w| Bar {
            label: w.label.clone(),
            // Bars take u64 — store utilisation as basis points (0..=10000+) so a fully spent
            // window outranks a quarter-spent one in chart_bar.
            value: ((w.state.utilization * 10_000.0).round() as i64).max(0) as u64,
            value_label: Some(percent(w.state.utilization)),
        })
        .collect();
    Body::Bars(BarsData { bars })
}

fn badge_body(snap: &Snapshot, target: &str) -> Body {
    let label = known_label(target).map(str::to_string).unwrap_or_else(|| {
        snap.find(target)
            .map(|w| w.label.clone())
            .unwrap_or_else(|| humanize_key(target))
    });
    let Some(window) = snap.find(target) else {
        return Body::Badge(BadgeData {
            status: Status::Warn,
            label: format!("{label}: n/a"),
        });
    };
    Body::Badge(BadgeData {
        status: status_for(window.state.utilization),
        label: format!("{label} {}", percent(window.state.utilization)),
    })
}

fn timeline_body(snap: &Snapshot) -> Body {
    let mut events: Vec<TimelineEvent> = snap
        .windows
        .iter()
        .filter_map(|w| {
            w.state.resets_at.map(|ts| TimelineEvent {
                timestamp: ts.timestamp(),
                title: format!("{} resets", w.label),
                detail: Some(format!("at {}", percent(w.state.utilization))),
                status: Some(status_for(w.state.utilization)),
            })
        })
        .collect();
    events.sort_by_key(|e| e.timestamp);
    Body::Timeline(TimelineData { events })
}

fn format_window_line(window: &NamedWindow) -> String {
    let reset = window
        .state
        .resets_at
        .map(format_reset)
        .unwrap_or_else(|| "n/a".into());
    format!(
        "{}  {} · resets {reset}",
        window.label,
        percent(window.state.utilization),
    )
}

fn percent(util: f64) -> String {
    format!("{:.0}%", (util * 100.0).clamp(0.0, 150.0))
}

fn format_reset(ts: DateTime<Utc>) -> String {
    let delta = ts.signed_duration_since(Utc::now());
    if delta <= chrono::Duration::zero() {
        return "now".into();
    }
    let mins = delta.num_minutes();
    if mins < 60 {
        format!("{mins}m")
    } else if mins < 60 * 48 {
        format!("{}h", delta.num_hours())
    } else {
        format!("{}d", delta.num_days())
    }
}

fn status_for(util: f64) -> Status {
    if util >= 0.85 {
        Status::Error
    } else if util >= 0.60 {
        Status::Warn
    } else {
        Status::Ok
    }
}

fn sample_body(shape: Shape) -> Option<Body> {
    use crate::samples;
    Some(match shape {
        Shape::Ratio => samples::ratio(0.07, "5h · resets 2h"),
        Shape::Text => samples::text("5h 7% · 7d 2% · 7d Sonnet 0% · Claude Design 0%"),
        Shape::TextBlock => samples::text_block(&[
            "5h             7% · resets 2h",
            "7d             2% · resets 6d",
            "7d Sonnet      0% · resets 6d",
            "Claude Design  0% · resets n/a",
        ]),
        Shape::MarkdownTextBlock => samples::markdown(
            "- **5h** — 7% (resets 2h)\n- **7d** — 2% (resets 6d)\n- **7d Sonnet** — 0% (resets 6d)\n- **Claude Design** — 0% (resets n/a)",
        ),
        Shape::Entries => samples::entries(&[
            ("5h", "7% (resets 2h)"),
            ("7d", "2% (resets 6d)"),
            ("7d Sonnet", "0% (resets 6d)"),
            ("Claude Design", "0% (resets n/a)"),
        ]),
        Shape::Bars => Body::Bars(BarsData {
            bars: vec![
                Bar {
                    label: "5h".into(),
                    value: 700,
                    value_label: Some("7%".into()),
                },
                Bar {
                    label: "7d".into(),
                    value: 200,
                    value_label: Some("2%".into()),
                },
                Bar {
                    label: "7d Sonnet".into(),
                    value: 0,
                    value_label: Some("0%".into()),
                },
                Bar {
                    label: "Claude Design".into(),
                    value: 0,
                    value_label: Some("0%".into()),
                },
            ],
        }),
        Shape::Badge => samples::badge(Status::Ok, "5h 7%"),
        Shape::Timeline => samples::timeline(&[
            (Utc::now().timestamp() + 7_200, "5h resets", Some("at 7%")),
            (
                Utc::now().timestamp() + 6 * 86_400,
                "7d resets",
                Some("at 2%"),
            ),
        ]),
        _ => return None,
    })
}

