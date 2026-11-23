use rusqlite::{Connection, OpenFlags};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub struct HistoryRow {
    pub command: String,
    pub exit: Option<i64>,
    pub when_ms: Option<i64>,
    pub dir: String,
}

pub fn last_component(path: &str) -> String {
    if path == "/" {
        return String::from("/");
    }
    path.rsplit("/")
        .find(|part| !part.is_empty())
        .unwrap_or("")
        .to_string()
}

pub fn format_age(now_ms: i64, then_ms: i64) -> String {
    let seconds = ((now_ms - then_ms) / 1000).max(0);
    if seconds < 60 {
        format!("{}s ago", seconds)
    } else if seconds < 3600 {
        format!("{}m ago", seconds / 60)
    } else if seconds < 86400 {
        format!("{}h ago", seconds / 3600)
    } else {
        format!("{}d ago", seconds / 86400)
    }
}

pub fn match_positions(query: &str, text: &str) -> Option<(Vec<usize>, bool)> {
    let wanted: String = query.trim().to_lowercase();
    if wanted.is_empty() {
        return Some((Vec::new(), true));
    }
    let lower = text.to_lowercase();
    if let Some(byte) = lower.find(&wanted) {
        let start = lower[..byte].chars().count();
        let length = wanted.chars().count();
        return Some(((start..start + length).collect(), true));
    }
    let letters: Vec<char> = wanted.chars().filter(|c| !c.is_whitespace()).collect();
    let mut positions: Vec<usize> = Vec::new();
    let mut next = 0;
    for (index, c) in lower.chars().enumerate() {
        if next < letters.len() && c == letters[next] {
            positions.push(index);
            next += 1;
        }
    }
    if next == letters.len() {
        Some((positions, false))
    } else {
        None
    }
}

pub fn filter_rows(rows: &[HistoryRow], query: &str) -> Vec<(usize, Vec<usize>)> {
    let mut found: Vec<(bool, usize, Vec<usize>)> = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        if let Some((positions, contiguous)) = match_positions(query, &row.command) {
            found.push((!contiguous, index, positions));
        }
    }
    found.sort_by_key(|entry| (entry.0, entry.1));
    found
        .into_iter()
        .map(|(_, index, positions)| (index, positions))
        .collect()
}

pub fn unique_commands(rows: Vec<HistoryRow>) -> Vec<HistoryRow> {
    let mut seen = std::collections::HashSet::new();
    rows.into_iter()
        .filter(|row| !row.command.trim().is_empty() && seen.insert(row.command.clone()))
        .collect()
}

fn database_path() -> PathBuf {
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("share"))
        })
        .unwrap_or_else(|| PathBuf::from("."));
    data_home.join("console").join("history.db")
}

fn read_rows(connection: &Connection, sql: &str, limit: i64) -> rusqlite::Result<Vec<HistoryRow>> {
    let mut statement = connection.prepare(sql)?;
    let rows = statement.query_map(rusqlite::params![limit], |row| {
        let exit: i64 = row.get(1)?;
        let timestamp: i64 = row.get(2)?;
        let cwd: String = row.get(3)?;
        Ok(HistoryRow {
            command: row.get(0)?,
            exit: if exit == -1 { None } else { Some(exit) },
            when_ms: Some(timestamp / 1_000_000),
            dir: last_component(&cwd),
        })
    })?;
    let collected: rusqlite::Result<Vec<HistoryRow>> = rows.collect();
    collected
}

pub fn load_database_rows(limit: i64) -> Vec<HistoryRow> {
    let path = database_path();
    let Ok(connection) = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_WRITE)
    else {
        return Vec::new();
    };
    let with_deleted = "select command, exit, timestamp, cwd from history where deleted_at is null order by timestamp desc limit ?1";
    let plain = "select command, exit, timestamp, cwd from history order by timestamp desc limit ?1";
    match read_rows(&connection, with_deleted, limit) {
        Ok(rows) => rows,
        Err(_) => read_rows(&connection, plain, limit).unwrap_or_default(),
    }
}

