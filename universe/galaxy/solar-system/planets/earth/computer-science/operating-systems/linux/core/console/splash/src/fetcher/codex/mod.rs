//! Codex (OpenAI Codex CLI) fetchers.
//!
//! - [`usage::CodexUsage`] aggregates the local `rollout-*.jsonl` files Codex CLI writes for
//!   every session, then HTTP-GETs the shared LLM pricing snapshot from splashboard's own
//!   GitHub repo (see [`crate::fetcher::llm_pricing`]) to convert tokens into a USD cost.
//! - [`subscription::CodexSubscription`] extracts rate-limit utilisation (5h / 7d windows +
//!   plan type) from the most recent session's last `token_count` event. No network — the
//!   payload Codex CLI already wrote is what we read.
//!
//! Both are `Safety::Safe`. `codex_subscription` is local-only. `codex_usage` makes one
//! host-fixed HTTP GET (the URL never accepts user input) — same model as `nasa_apod` or any
//! other Safe + outbound fetcher: config can't redirect the traffic, no token leaves the
//! host.

mod common;
mod subscription;
mod usage;

use std::sync::Arc;

use crate::fetcher::Fetcher;

pub fn fetchers() -> Vec<Arc<dyn Fetcher>> {
    vec![
        Arc::new(usage::CodexUsage),
        Arc::new(subscription::CodexSubscription),
    ]
}

