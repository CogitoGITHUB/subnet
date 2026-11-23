//! `net_proxy` — the `$*_PROXY` environment, i.e. whether this shell routes through a proxy.
//!
//! The one realtime fetcher in the family: pure `std::env` reads, infallible, no I/O.

use crate::payload::{
    BadgeData, Body, EntriesData, MarkdownTextBlockData, Payload, Status, TextBlockData, TextData,
};
use crate::render::Shape;

use super::super::{FetchContext, RealtimeFetcher, Safety};
use super::{entry, payload};

const SHAPES: &[Shape] = &[
    Shape::Entries,
    Shape::Text,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::Badge,
];

const DIRECT: &str = "(direct)";
const UNSET: &str = "(unset)";

pub struct NetProxy;

/// Resolved proxy environment. Each field prefers the lowercase variable name (`http_proxy`)
/// over the uppercase one (`HTTP_PROXY`), matching the de-facto precedence most CLIs use.
struct ProxyState {
    http: Option<String>,
    https: Option<String>,
    no_proxy: Option<String>,
}

impl ProxyState {
    fn is_proxied(&self) -> bool {
        self.http.is_some() || self.https.is_some()
    }

    /// The proxy that actually carries traffic — HTTPS first since nearly everything is TLS now.
    fn active(&self) -> Option<&str> {
        self.https.as_deref().or(self.http.as_deref())
    }
}

impl RealtimeFetcher for NetProxy {
    fn name(&self) -> &str {
        "net_proxy"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "The shell's `$*_PROXY` environment. `Entries` (default) / `TextBlock` / `MarkdownTextBlock` roll up `http_proxy` / `https_proxy` / `no_proxy`; `Text` shows the proxy that actually carries traffic (HTTPS first), or `(direct)`; `Badge` is a proxied / direct pill."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        body_for_shape(&sample_state(), shape)
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let state = read_proxy_state();
        let shape = ctx.shape.unwrap_or(Shape::Entries);
        payload(body_for_shape(&state, shape).unwrap_or_else(|| entries_body(&state)))
    }
}

fn read_proxy_state() -> ProxyState {
    ProxyState {
        http: env_either("http_proxy", "HTTP_PROXY"),
        https: env_either("https_proxy", "HTTPS_PROXY"),
        no_proxy: env_either("no_proxy", "NO_PROXY"),
    }
}

fn env_either(lower: &str, upper: &str) -> Option<String> {
    std::env::var(lower)
        .ok()
        .or_else(|| std::env::var(upper).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn body_for_shape(state: &ProxyState, shape: Shape) -> Option<Body> {
    Some(match shape {
        Shape::Entries => entries_body(state),
        Shape::Text => Body::Text(TextData {
            value: state.active().unwrap_or(DIRECT).to_string(),
        }),
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: rows(state).map(|(k, v)| format!("{k}  {v}")).collect(),
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: rows(state)
                .map(|(k, v)| format!("- **{k}** {v}"))
                .collect::<Vec<_>>()
                .join("\n"),
        }),
        Shape::Badge => Body::Badge(proxy_badge(state)),
        _ => return None,
    })
}

fn entries_body(state: &ProxyState) -> Body {
    Body::Entries(EntriesData {
        items: rows(state).map(|(k, v)| entry(k, &v)).collect(),
    })
}

fn rows(state: &ProxyState) -> impl Iterator<Item = (&'static str, String)> {
    [
        ("http_proxy", state.http.clone()),
        ("https_proxy", state.https.clone()),
        ("no_proxy", state.no_proxy.clone()),
    ]
    .into_iter()
    .map(|(k, v)| (k, v.unwrap_or_else(|| UNSET.to_string())))
}

/// `Warn`, not `Error` — being proxied isn't broken, but it's worth a glance-level flag since it
/// silently changes where every request goes.
fn proxy_badge(state: &ProxyState) -> BadgeData {
    if state.is_proxied() {
        BadgeData {
            status: Status::Warn,
            label: "proxied".into(),
        }
    } else {
        BadgeData {
            status: Status::Ok,
            label: "direct".into(),
        }
    }
}

fn sample_state() -> ProxyState {
    ProxyState {
        http: Some("http://proxy.corp:8080".into()),
        https: Some("http://proxy.corp:8080".into()),
        no_proxy: Some("localhost,127.0.0.1,.internal".into()),
    }
}

