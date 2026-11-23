use std::env;
use std::path::PathBuf;

use async_trait::async_trait;
use directories::BaseDirs;
use eyre::{Result, eyre};
use futures::TryStreamExt;
use sqlx::{FromRow, Row, sqlite::SqlitePool};
use time::OffsetDateTime;
use uuid::Uuid;
use uuid::timestamp::{Timestamp, context::NoContext};

use super::{Importer, Loader, get_histfile_path};
use crate::history::History;
use crate::utils::get_host_user;

#[derive(Debug, FromRow)]
struct HistDbEntry {
    inp: String,
    rtn: Option<i64>,
    tsb: f64,
    tse: f64,
    cwd: String,
    session_start: f64,
}

impl HistDbEntry {
    fn into_hist_with_hostname(self, hostname: String) -> History {
        let ts_nanos = (self.tsb * 1_000_000_000_f64) as i128;
        let timestamp = OffsetDateTime::from_unix_timestamp_nanos(ts_nanos).unwrap();

        let session_ts_seconds = self.session_start.trunc() as u64;
        let session_ts_nanos = (self.session_start.fract() * 1_000_000_000_f64) as u32;
        let session_ts = Timestamp::from_unix(NoContext, session_ts_seconds, session_ts_nanos);
        let session_id = Uuid::new_v7(session_ts).to_string();
        let duration = (self.tse - self.tsb) * 1_000_000_000_f64;

        if let Some(exit) = self.rtn {
            let imported = History::import()
                .timestamp(timestamp)
                .duration(duration.trunc() as i64)
                .exit(exit)
                .command(self.inp)
                .cwd(self.cwd)
                .session(session_id)
                .hostname(hostname);
            imported.build().into()
        } else {
            let imported = History::import()
                .timestamp(timestamp)
                .duration(duration.trunc() as i64)
                .command(self.inp)
                .cwd(self.cwd)
                .session(session_id)
                .hostname(hostname);
            imported.build().into()
        }
    }
}

fn xonsh_db_path(xonsh_data_dir: Option<String>) -> Result<PathBuf> {
    // if running within xonsh, this will be available
    if let Some(d) = xonsh_data_dir {
        let mut path = PathBuf::from(d);
        path.push("xonsh-history.sqlite");
        return Ok(path);
    }

    // otherwise, fall back to default
    let base = BaseDirs::new().ok_or_else(|| eyre!("Could not determine home directory"))?;

    let hist_file = base.data_dir().join("xonsh/xonsh-history.sqlite");
    if hist_file.exists() || cfg!(test) {
        Ok(hist_file)
    } else {
        Err(eyre!(
            "Could not find xonsh history db at: {}",
            hist_file.to_string_lossy()
        ))
    }
}

#[derive(Debug)]
pub struct XonshSqlite {
    pool: SqlitePool,
    hostname: String,
}

#[async_trait]
impl Importer for XonshSqlite {
    const NAME: &'static str = "xonsh_sqlite";

    async fn new() -> Result<Self> {
        // wrap xonsh-specific path resolver in general one so that it respects $HISTPATH
        let xonsh_data_dir = env::var("XONSH_DATA_DIR").ok();
        let db_path = get_histfile_path(|| xonsh_db_path(xonsh_data_dir))?;
        let connection_str = db_path.to_str().ok_or_else(|| {
            eyre!(
                "Invalid path for SQLite database: {}",
                db_path.to_string_lossy()
            )
        })?;

        let pool = SqlitePool::connect(connection_str).await?;
        let hostname = get_host_user();
        Ok(XonshSqlite { pool, hostname })
    }

    async fn entries(&mut self) -> Result<usize> {
        let query = "SELECT COUNT(*) FROM xonsh_history";
        let row = sqlx::query(query).fetch_one(&self.pool).await?;
        let count: u32 = row.get(0);
        Ok(count as usize)
    }

    async fn load(self, loader: &mut impl Loader) -> Result<()> {
        let query = r#"
            SELECT inp, rtn, tsb, tse, cwd,
            MIN(tsb) OVER (PARTITION BY sessionid) AS session_start
            FROM xonsh_history
            ORDER BY rowid
        "#;

        let mut entries = sqlx::query_as::<_, HistDbEntry>(query).fetch(&self.pool);

        let mut count = 0;
        while let Some(entry) = entries.try_next().await? {
            let hist = entry.into_hist_with_hostname(self.hostname.clone());
            loader.push(hist).await?;
            count += 1;
        }

        println!("Loaded: {count}");
        Ok(())
    }
}

