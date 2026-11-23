// import old shell history from zsh-histdb!
// automatically hoover up all that we can find

// As far as i can tell there are no version numbers in the histdb sqlite DB, so we're going based
// on the schema from 2022-05-01
//
// I have run into some histories that will not import b/c of non UTF-8 characters.
//

//
// An Example sqlite query for hsitdb data:
//
//id|session|command_id|place_id|exit_status|start_time|duration|id|argv|id|host|dir
//
//
//  select
//    history.id,
//    history.start_time,
//    places.host,
//    places.dir,
//    commands.argv
//  from history
//    left join commands on history.command_id = commands.id
//    left join places on history.place_id = places.id ;
//
// CREATE TABLE history  (id integer primary key autoincrement,
//                       session int,
//                       command_id int references commands (id),
//                       place_id int references places (id),
//                       exit_status int,
//                       start_time int,
//                       duration int);
//

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use shell_history_common::utils::uuid_v7;
use directories::UserDirs;
use eyre::{Result, eyre};
use sqlx::{Pool, sqlite::SqlitePool};
use time::PrimitiveDateTime;

use super::Importer;
use crate::history::History;
use crate::import::Loader;
use crate::utils::{get_hostname, get_username};

#[derive(sqlx::FromRow, Debug)]
pub struct HistDbEntryCount {
    pub count: usize,
}

#[derive(sqlx::FromRow, Debug)]
pub struct HistDbEntry {
    pub id: i64,
    pub start_time: PrimitiveDateTime,
    pub host: Vec<u8>,
    pub dir: Vec<u8>,
    pub argv: Vec<u8>,
    pub duration: i64,
    pub exit_status: i64,
    pub session: i64,
}

#[derive(Debug)]
pub struct ZshHistDb {
    histdb: Vec<HistDbEntry>,
    username: String,
}

/// Read db at given file, return vector of entries.
async fn hist_from_db(dbpath: PathBuf) -> Result<Vec<HistDbEntry>> {
    let pool = SqlitePool::connect(dbpath.to_str().unwrap()).await?;
    hist_from_db_conn(pool).await
}

async fn hist_from_db_conn(pool: Pool<sqlx::Sqlite>) -> Result<Vec<HistDbEntry>> {
    let query = r#"
        SELECT
            history.id, history.start_time, history.duration, places.host, places.dir,
            commands.argv, history.exit_status, history.session
        FROM history
        LEFT JOIN commands ON history.command_id = commands.id
        LEFT JOIN places ON history.place_id = places.id
        ORDER BY history.start_time
    "#;
    let histdb_vec: Vec<HistDbEntry> = sqlx::query_as::<_, HistDbEntry>(query)
        .fetch_all(&pool)
        .await?;
    Ok(histdb_vec)
}

impl ZshHistDb {
    pub fn histpath_candidate() -> PathBuf {
        // By default histdb database is `${HOME}/.histdb/zsh-history.db`
        // This can be modified by ${HISTDB_FILE}
        //
        //  if [[ -z ${HISTDB_FILE} ]]; then
        //      typeset -g HISTDB_FILE="${HOME}/.histdb/zsh-history.db"
        let user_dirs = UserDirs::new().unwrap(); // should catch error here?
        let home_dir = user_dirs.home_dir();
        std::env::var("HISTDB_FILE")
            .as_ref()
            .map(|x| Path::new(x).to_path_buf())
            .unwrap_or_else(|_err| home_dir.join(".histdb/zsh-history.db"))
    }
    pub fn histpath() -> Result<PathBuf> {
        let histdb_path = ZshHistDb::histpath_candidate();
        if histdb_path.exists() {
            Ok(histdb_path)
        } else {
            Err(eyre!(
                "Could not find history file. Try setting $HISTDB_FILE"
            ))
        }
    }
}

#[async_trait]
impl Importer for ZshHistDb {
    // Not sure how this is used
    const NAME: &'static str = "zsh_histdb";

    /// Creates a new ZshHistDb and populates the history based on the pre-populated data
    /// structure.
    async fn new() -> Result<Self> {
        let dbpath = ZshHistDb::histpath()?;
        let histdb_entry_vec = hist_from_db(dbpath).await?;
        Ok(Self {
            histdb: histdb_entry_vec,
            username: get_username(),
        })
    }

    async fn entries(&mut self) -> Result<usize> {
        Ok(self.histdb.len())
    }

    async fn load(self, h: &mut impl Loader) -> Result<()> {
        let mut session_map = HashMap::new();
        for entry in self.histdb {
            let command = match std::str::from_utf8(&entry.argv) {
                Ok(s) => s.trim_end(),
                Err(_) => continue, // we can skip past things like invalid utf8
            };
            let cwd = match std::str::from_utf8(&entry.dir) {
                Ok(s) => s.trim_end(),
                Err(_) => continue, // we can skip past things like invalid utf8
            };
            let hostname = format!(
                "{}:{}",
                String::from_utf8(entry.host).unwrap_or_else(|_e| get_hostname()),
                self.username
            );
            let session = session_map.entry(entry.session).or_insert_with(uuid_v7);

            let imported = History::import()
                .timestamp(entry.start_time.assume_utc())
                .command(command)
                .cwd(cwd)
                .duration(entry.duration * 1_000_000_000)
                .exit(entry.exit_status)
                .session(session.as_simple().to_string())
                .hostname(hostname)
                .build();
            h.push(imported.into()).await?;
        }
        Ok(())
    }
}

