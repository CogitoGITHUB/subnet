use clap::Args;
use eyre::{Result, bail};

use shell_history_client::{
    database::Database, encryption, history::store::HistoryStore,
    record::sqlite_store::SqliteStore, settings::Settings,
};

#[derive(Args, Debug)]
pub struct Rebuild {
    pub tag: String,
}

impl Rebuild {
    pub async fn run(
        &self,
        settings: &Settings,
        store: SqliteStore,
        database: &dyn Database,
    ) -> Result<()> {
        match self.tag.as_str() {
            "history" => {
                let encryption_key: [u8; 32] = encryption::load_key(settings)?.into();
                let host_id = Settings::host_id().await?;
                let history_store = HistoryStore::new(store, host_id, encryption_key);
                history_store.build(database).await?;
            }
            tag => bail!("unknown tag: {tag}"),
        }

        Ok(())
    }
}
