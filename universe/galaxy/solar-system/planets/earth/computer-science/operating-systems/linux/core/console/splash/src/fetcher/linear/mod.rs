//! `linear_*` fetcher family — Linear issues, notifications, current-cycle progress.
//!
//! Every fetcher in this family targets the fixed host `api.linear.app` (see `client::API_URL`),
//! so the personal API key (`options.token` or `LINEAR_TOKEN` env) can never be redirected to
//! an attacker-controlled origin — Safety::Safe.
//!
//! OAuth flow is intentionally not supported: splashboard's startup window has no place to host
//! a callback. Personal API keys (`lin_api_*`) are sent as-is in the `Authorization` header by
//! [`client::graphql_query`].

mod client;
pub mod cycle;
pub mod issues;
pub mod notifications;

use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::fetcher::{FetchContext, Fetcher};

pub fn fetchers() -> Vec<Arc<dyn Fetcher>> {
    vec![
        issues::fetcher(),
        notifications::fetcher(),
        cycle::fetcher(),
    ]
}

/// Cache key for any `linear_*` fetcher. Mixes shape, format, options blob, and a token-scope
/// prefix so two accounts sharing a `$HOME/.splash/cache` directory can't observe each
/// other's payloads.
pub(crate) fn cache_key(name: &str, ctx: &FetchContext) -> String {
    let shape = ctx.shape.map(|s| s.as_str()).unwrap_or("");
    let format = ctx.format.as_deref().unwrap_or("");
    let opts_token = ctx
        .options
        .as_ref()
        .and_then(|v| v.get("token"))
        .and_then(|v| v.as_str());
    let extra = client::cache_extra(opts_token, ctx.options.as_ref());
    let raw = format!("{name}|{shape}|{format}|{extra}");
    let digest = Sha256::digest(raw.as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("{name}-{hex}")
}

