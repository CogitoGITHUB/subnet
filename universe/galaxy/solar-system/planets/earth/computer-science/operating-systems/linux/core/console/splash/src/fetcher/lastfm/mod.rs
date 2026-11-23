//! Last.fm fetchers — user listening statistics (recent scrobbles, top artists / tracks /
//! albums) plus global charts.
//!
//! Every request targets the fixed host `ws.audioscrobbler.com`. Config-provided fields
//! (`user`, `period`, …) only change which resource on that host is queried; the API key
//! never leaves to an attacker-controlled origin. Classified as `Safety::Safe`, same model
//! as the `github_*` family.
//!
//! Auth: `LASTFM_API_KEY` — create one at https://www.last.fm/api/account/create and put it
//! in `$HOME/.splash/secrets.toml`.

mod charts;
mod client;
mod common;
mod scrobbles_today;
mod top;
mod top_albums;
mod top_artists;
mod top_tracks;

use std::sync::Arc;

use crate::fetcher::Fetcher;

pub fn fetchers() -> Vec<Arc<dyn Fetcher>> {
    vec![
        Arc::new(scrobbles_today::LastfmScrobblesToday),
        Arc::new(top_artists::LastfmTopArtists),
        Arc::new(top_tracks::LastfmTopTracks),
        Arc::new(top_albums::LastfmTopAlbums),
        Arc::new(charts::LastfmCharts),
    ]
}

