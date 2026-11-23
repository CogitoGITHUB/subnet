//! Claude (and Claude Code) fetchers.
//!
//! - [`code_usage::ClaudeCodeUsage`] aggregates the local JSONL files Claude Code writes for
//!   every session, with zero configuration and no network.
//! - [`subscription::ClaudeSubscription`] reads the OAuth token Claude Code stores in
//!   `~/.claude/.credentials.json` and reports Max-plan utilisation against an undocumented
//!   `api.anthropic.com` endpoint.
//!
//! Both are `Safety::Safe` — `code_usage` is local-only, `subscription` only ever talks to the
//! hardcoded `api.anthropic.com` host (same model as the `github_*` / `lastfm_*` families).

mod code_usage;
mod common;
mod subscription;

use std::sync::Arc;

use crate::fetcher::Fetcher;

pub fn fetchers() -> Vec<Arc<dyn Fetcher>> {
    vec![
        Arc::new(code_usage::ClaudeCodeUsage),
        Arc::new(subscription::ClaudeSubscription),
    ]
}

