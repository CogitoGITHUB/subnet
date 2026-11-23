//! `deal_*` family — gaming and (future) retail deal feeds.
//!
//! Each sibling normalises its upstream into the shared [`common::DealRow`] vocabulary so the
//! same renderer chain (`list_links`, `list_cards`, `chart_bar`, `status_badge`, …) handles
//! every variant. Batch 1 ships three gaming-focused sources; the `deal_*` prefix leaves room
//! for non-gaming retail siblings without further renaming.

mod common;
mod free_games;
mod games;
mod steam_daily;

use std::sync::Arc;

use crate::fetcher::Fetcher;

pub fn fetchers() -> Vec<Arc<dyn Fetcher>> {
    vec![
        Arc::new(free_games::FreeGamesFetcher),
        Arc::new(steam_daily::SteamDailyFetcher),
        Arc::new(games::GamesFetcher),
    ]
}

