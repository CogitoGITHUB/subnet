//! Shared HTTP client + base URL for the `lobsters_*` family. Targets the public `lobste.rs`
//! JSON endpoints — fixed host, no auth, so every fetcher in the family stays Safety::Safe.

use std::sync::OnceLock;
use std::time::Duration;

use reqwest::Client;
use serde::de::DeserializeOwned;

use crate::fetcher::FetchError;

pub const SITE_BASE: &str = "https://lobste.rs";
const USER_AGENT: &str = concat!("splashboard/", env!("CARGO_PKG_VERSION"));
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

pub fn http() -> &'static Client {
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

pub async fn get<T: DeserializeOwned>(url: &str) -> Result<T, FetchError> {
    let res = http()
        .get(url)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("lobsters request failed: {e}")))?;
    let status = res.status();
    if !status.is_success() {
        return Err(FetchError::Failed(format!("lobsters {status}")));
    }
    res.json()
        .await
        .map_err(|e| FetchError::Failed(format!("lobsters json parse: {e}")))
}

