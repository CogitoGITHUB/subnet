// Here we are using sqlite as a pretty dumb store, and will not be running any complex queries.
// Multiple stores of multiple types are all stored in one chonky table (for now), and we just index
// by tag/host

use std::str::FromStr;
use std::{path::Path, time::Duration};

use async_trait::async_trait;
use eyre::{Result, eyre};
use fs_err as fs;

use sqlx::{
    Row,
    sqlite::{
        SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteRow,
        SqliteSynchronous,
    },
};

use shell_history_common::record::{
    EncryptedData, Host, HostId, Record, RecordId, RecordIdx, RecordStatus,
};
use shell_history_common::utils;
use uuid::Uuid;

use super::encryption::PASETO_V4;
use super::store::Store;

#[derive(Debug, Clone)]
pub struct SqliteStore {
    pool: SqlitePool,
}

impl SqliteStore {
    pub async fn new(path: impl AsRef<Path>, timeout: f64) -> Result<Self> {
        let path = path.as_ref();

        debug!("opening sqlite database at {path:?}");

        if utils::broken_symlink(path) {
            eprintln!(
                "console history: Sqlite db path ({path:?}) is a broken symlink. Unable to read or create replacement."
            );
            std::process::exit(1);
        }

        if !path.exists()
            && let Some(dir) = path.parent()
        {
            fs::create_dir_all(dir)?;
        }

        let opts = SqliteConnectOptions::from_str(path.as_os_str().to_str().unwrap())?
            .journal_mode(SqliteJournalMode::Wal)
            .optimize_on_close(true, None)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true)
            .create_if_missing(true);

        let pool = SqlitePoolOptions::new()
            .acquire_timeout(Duration::from_secs_f64(timeout))
            .connect_with(opts)
            .await?;

        Self::setup_db(&pool).await?;

        Ok(Self { pool })
    }

    async fn setup_db(pool: &SqlitePool) -> Result<()> {
        debug!("running sqlite database setup");

        sqlx::migrate!("./record-migrations").run(pool).await?;

        Ok(())
    }

    async fn save_raw(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        r: &Record<EncryptedData>,
    ) -> Result<()> {
        // In sqlite, we are "limited" to i64. But that is still fine, until 2262.
        sqlx::query(
            "insert or ignore into store(id, idx, host, tag, timestamp, version, data, cek)
                values(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .bind(r.id.0.as_hyphenated().to_string())
        .bind(r.idx as i64)
        .bind(r.host.id.0.as_hyphenated().to_string())
        .bind(r.tag.as_str())
        .bind(r.timestamp as i64)
        .bind(r.version.as_str())
        .bind(r.data.data.as_str())
        .bind(r.data.content_encryption_key.as_str())
        .execute(&mut **tx)
        .await?;

        Ok(())
    }

    fn query_row(row: SqliteRow) -> Record<EncryptedData> {
        let idx: i64 = row.get("idx");
        let timestamp: i64 = row.get("timestamp");

        // tbh at this point things are pretty fucked so just panic
        let id = Uuid::from_str(row.get("id")).expect("invalid id UUID format in sqlite DB");
        let host = Uuid::from_str(row.get("host")).expect("invalid host UUID format in sqlite DB");

        Record {
            id: RecordId(id),
            idx: idx as u64,
            host: Host::new(HostId(host)),
            timestamp: timestamp as u64,
            tag: row.get("tag"),
            version: row.get("version"),
            data: EncryptedData {
                data: row.get("data"),
                content_encryption_key: row.get("cek"),
            },
        }
    }

    async fn load_all(&self) -> Result<Vec<Record<EncryptedData>>> {
        let res = sqlx::query("select * from store ")
            .map(Self::query_row)
            .fetch_all(&self.pool)
            .await?;

        Ok(res)
    }
}

#[async_trait]
impl Store for SqliteStore {
    async fn push_batch(
        &self,
        records: impl Iterator<Item = &Record<EncryptedData>> + Send + Sync,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        for record in records {
            Self::save_raw(&mut tx, record).await?;
        }

        tx.commit().await?;

        Ok(())
    }

    async fn get(&self, id: RecordId) -> Result<Record<EncryptedData>> {
        let res = sqlx::query("select * from store where store.id = ?1")
            .bind(id.0.as_hyphenated().to_string())
            .map(Self::query_row)
            .fetch_one(&self.pool)
            .await?;

        Ok(res)
    }

    async fn delete(&self, id: RecordId) -> Result<()> {
        sqlx::query("delete from store where id = ?1")
            .bind(id.0.as_hyphenated().to_string())
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn delete_all(&self) -> Result<()> {
        sqlx::query("delete from store").execute(&self.pool).await?;

        Ok(())
    }

    async fn last(&self, host: HostId, tag: &str) -> Result<Option<Record<EncryptedData>>> {
        let res =
            sqlx::query("select * from store where host=?1 and tag=?2 order by idx desc limit 1")
                .bind(host.0.as_hyphenated().to_string())
                .bind(tag)
                .map(Self::query_row)
                .fetch_one(&self.pool)
                .await;

        match res {
            Err(sqlx::Error::RowNotFound) => Ok(None),
            Err(e) => Err(eyre!("an error occurred: {}", e)),
            Ok(record) => Ok(Some(record)),
        }
    }

    async fn first(&self, host: HostId, tag: &str) -> Result<Option<Record<EncryptedData>>> {
        self.idx(host, tag, 0).await
    }

    async fn len_all(&self) -> Result<u64> {
        let res: Result<(i64,), sqlx::Error> = sqlx::query_as("select count(*) from store")
            .fetch_one(&self.pool)
            .await;
        match res {
            Err(e) => Err(eyre!("failed to fetch local store len: {}", e)),
            Ok(v) => Ok(v.0 as u64),
        }
    }

    async fn len_tag(&self, tag: &str) -> Result<u64> {
        let res: Result<(i64,), sqlx::Error> =
            sqlx::query_as("select count(*) from store where tag=?1")
                .bind(tag)
                .fetch_one(&self.pool)
                .await;
        match res {
            Err(e) => Err(eyre!("failed to fetch local store len: {}", e)),
            Ok(v) => Ok(v.0 as u64),
        }
    }

    async fn len(&self, host: HostId, tag: &str) -> Result<u64> {
        let last = self.last(host, tag).await?;

        if let Some(last) = last {
            return Ok(last.idx + 1);
        }

        return Ok(0);
    }

    async fn next(
        &self,
        host: HostId,
        tag: &str,
        idx: RecordIdx,
        limit: u64,
    ) -> Result<Vec<Record<EncryptedData>>> {
        let res = sqlx::query(
            "select * from store where idx >= ?1 and host = ?2 and tag = ?3 order by idx asc limit ?4",
        )
        .bind(idx as i64)
        .bind(host.0.as_hyphenated().to_string())
        .bind(tag)
        .bind(limit as i64)
        .map(Self::query_row)
        .fetch_all(&self.pool)
        .await?;

        Ok(res)
    }

    async fn idx(
        &self,
        host: HostId,
        tag: &str,
        idx: RecordIdx,
    ) -> Result<Option<Record<EncryptedData>>> {
        let res = sqlx::query("select * from store where idx = ?1 and host = ?2 and tag = ?3")
            .bind(idx as i64)
            .bind(host.0.as_hyphenated().to_string())
            .bind(tag)
            .map(Self::query_row)
            .fetch_one(&self.pool)
            .await;

        match res {
            Err(sqlx::Error::RowNotFound) => Ok(None),
            Err(e) => Err(eyre!("an error occurred: {}", e)),
            Ok(v) => Ok(Some(v)),
        }
    }

    async fn status(&self) -> Result<RecordStatus> {
        let mut status = RecordStatus::new();

        let res: Result<Vec<(String, String, i64)>, sqlx::Error> =
            sqlx::query_as("select host, tag, max(idx) from store group by host, tag")
                .fetch_all(&self.pool)
                .await;

        let res = match res {
            Err(e) => return Err(eyre!("failed to fetch local store status: {}", e)),
            Ok(v) => v,
        };

        for i in res {
            let host = HostId(
                Uuid::from_str(i.0.as_str()).expect("failed to parse uuid for local store status"),
            );

            status.set_raw(host, i.1, i.2 as u64);
        }

        Ok(status)
    }

    async fn all_tagged(&self, tag: &str) -> Result<Vec<Record<EncryptedData>>> {
        let res = sqlx::query("select * from store where tag = ?1 order by timestamp asc")
            .bind(tag)
            .map(Self::query_row)
            .fetch_all(&self.pool)
            .await?;

        Ok(res)
    }

    /// Reencrypt every single item in this store with a new key
    /// Be careful - this may mess with sync.
    async fn re_encrypt(&self, old_key: &[u8; 32], new_key: &[u8; 32]) -> Result<()> {
        // Load all the records
        // In memory like some of the other code here
        // This will never be called in a hot loop, and only under the following circumstances
        // 1. The user has logged into a new account, with a new key. They are unlikely to have a
        //    lot of data
        // 2. The user has encountered some sort of issue, and runs a maintenance command that
        //    invokes this
        let all = self.load_all().await?;

        let re_encrypted = all
            .into_iter()
            .map(|record| record.re_encrypt::<PASETO_V4>(old_key, new_key))
            .collect::<Result<Vec<_>>>()?;

        // next up, we delete all the old data and reinsert the new stuff
        // do it in one transaction, so if anything fails we rollback OK

        let mut tx = self.pool.begin().await?;

        let res = sqlx::query("delete from store").execute(&mut *tx).await?;

        let rows = res.rows_affected();
        debug!("deleted {rows} rows");

        // don't call push_batch, as it will start its own transaction
        // call the underlying save_raw

        for record in re_encrypted {
            Self::save_raw(&mut tx, &record).await?;
        }

        tx.commit().await?;

        Ok(())
    }

    /// Verify that every record in this store can be decrypted with the current key
    /// Someday maybe also check each tag/record can be deserialized, but not for now.
    async fn verify(&self, key: &[u8; 32]) -> Result<()> {
        let all = self.load_all().await?;

        all.into_iter()
            .map(|record| record.decrypt::<PASETO_V4>(key))
            .collect::<Result<Vec<_>>>()?;

        Ok(())
    }

    /// Verify that every record in this store can be decrypted with the current key
    /// Someday maybe also check each tag/record can be deserialized, but not for now.
    async fn purge(&self, key: &[u8; 32]) -> Result<()> {
        let all = self.load_all().await?;

        for record in all.iter() {
            match record.clone().decrypt::<PASETO_V4>(key) {
                Ok(_) => continue,
                Err(_) => {
                    println!(
                        "Failed to decrypt {}, deleting",
                        record.id.0.as_hyphenated()
                    );

                    self.delete(record.id).await?;
                }
            }
        }

        Ok(())
    }
}

