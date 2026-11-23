use eyre::{Context, Result};

use shell_history_client::{
    database::Database, history::store::HistoryStore, record::sqlite_store::SqliteStore,
    settings::Settings,
};
use shell_history_common::record::RecordId;

/// Rebuild the history store after a sync (incremental).
pub async fn build(
    settings: &Settings,
    store: &SqliteStore,
    db: &dyn Database,
    downloaded: Option<&[RecordId]>,
) -> Result<()> {
    let encryption_key: [u8; 32] = shell_history_client::encryption::load_key(settings)
        .context("could not load encryption key")?
        .into();

    let host_id = Settings::host_id().await?;
    let downloaded = downloaded.unwrap_or(&[]);

    let history_store = HistoryStore::new(store.clone(), host_id, encryption_key);

    if let Err(e) = history_store.incremental_build(db, downloaded).await {
        eprintln!("Warning: failed to build history: {e}");
    }

    Ok(())
}
