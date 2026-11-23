// import old shell history!
// automatically hoover up all that we can find

use std::borrow::Cow;
use std::path::PathBuf;

use async_trait::async_trait;
use directories::UserDirs;
use eyre::{Result, eyre};
use time::{Duration, OffsetDateTime};

use super::{Importer, Loader, get_histfile_path, unix_byte_lines};
use crate::history::History;
use crate::import::read_to_end;

#[derive(Debug)]
pub struct Zsh {
    bytes: Vec<u8>,
}

impl Zsh {
    fn num_entries(&self) -> usize {
        super::count_lines(&self.bytes)
    }
}

fn default_histpath() -> Result<PathBuf> {
    // oh-my-zsh sets HISTFILE=~/.zhistory
    // zsh has no default value for this var, but uses ~/.zhistory.
    // zsh-newuser-install propose as default .histfile https://github.com/zsh-users/zsh/blob/master/Functions/Newuser/zsh-newuser-install#L794
    // we could maybe be smarter about this in the future :)
    let user_dirs = UserDirs::new().ok_or_else(|| eyre!("could not find user directories"))?;
    let home_dir = user_dirs.home_dir();

    let mut candidates = [".zhistory", ".zsh_history", ".histfile"].iter();
    loop {
        match candidates.next() {
            Some(candidate) => {
                let histpath = home_dir.join(candidate);
                if histpath.exists() {
                    break Ok(histpath);
                }
            }
            None => {
                break Err(eyre!(
                    "Could not find history file. Try setting and exporting $HISTFILE"
                ));
            }
        }
    }
}

/// Represents a line of zsh history.
struct Entry {
    pub command: String,
    pub timestamp: Option<OffsetDateTime>,
    /// Nanoseconds
    pub duration: Option<i64>,
}

impl Entry {
    pub fn parse(line: &str) -> Self {
        if let Some(rest) = line.strip_prefix(": ") {
            let (time, rest) = rest.split_once(':').unwrap();
            let (duration, command) = rest.split_once(';').unwrap();
            let time = time
                .parse::<i64>()
                .ok()
                .and_then(|t| OffsetDateTime::from_unix_timestamp(t).ok());

            // use nanos, because why the hell not? we won't display them.
            let duration = duration.parse::<i64>().map_or(-1, |t| t * 1_000_000_000);
            Self {
                command: command.trim_end().to_owned(),
                timestamp: time,
                duration: Some(duration),
            }
        } else {
            Self {
                command: line.trim_end().to_owned(),
                timestamp: None,
                duration: None,
            }
        }
    }
}

#[async_trait]
impl Importer for Zsh {
    const NAME: &'static str = "zsh";

    async fn new() -> Result<Self> {
        let bytes = read_to_end(get_histfile_path(default_histpath)?)?;
        Ok(Self { bytes })
    }

    async fn entries(&mut self) -> Result<usize> {
        Ok(self.num_entries())
    }

    async fn load(self, h: &mut impl Loader) -> Result<()> {
        let mut line = String::new();
        let mut entries = Vec::with_capacity(self.num_entries());

        for b in unix_byte_lines(&self.bytes) {
            let s = match unmetafy(b) {
                Some(s) => s,
                _ => continue, // we can skip past things like invalid utf8
            };

            if let Some(s) = s.strip_suffix('\\') {
                line.push_str(s);
                line.push('\n');
            } else {
                line.push_str(&s);
                entries.push(Entry::parse(&line));
                line.clear();
            }
        }

        // Similar approach to preserving order as the Bash importer.
        let (commands_until_timestamp, first_timestamp) = entries
            .iter()
            .enumerate()
            .find_map(|(i, entry)| entry.timestamp.map(|t| (i + 1, t)))
            .unwrap_or_else(|| (entries.len(), OffsetDateTime::now_utc()));

        let timestamp_increment = Duration::milliseconds(1);
        let mut timestamp = first_timestamp
            - u32::try_from(commands_until_timestamp).unwrap_or(u32::MAX) * timestamp_increment;

        for entry in entries {
            if let Some(time) = entry.timestamp {
                timestamp = time;
            } else {
                timestamp += timestamp_increment;
            }

            let builder = History::import()
                .timestamp(timestamp)
                .command(entry.command);

            let imported = if let Some(duration) = entry.duration {
                builder.duration(duration).build()
            } else {
                builder.build()
            };
            h.push(imported.into()).await?;
        }
        Ok(())
    }
}

fn unmetafy(line: &[u8]) -> Option<Cow<'_, str>> {
    if line.contains(&0x83) {
        let mut s = Vec::with_capacity(line.len());
        let mut is_meta = false;
        for ch in line {
            if *ch == 0x83 {
                is_meta = true;
            } else if is_meta {
                is_meta = false;
                s.push(*ch ^ 32);
            } else {
                s.push(*ch)
            }
        }
        String::from_utf8(s).ok().map(Cow::Owned)
    } else {
        std::str::from_utf8(line).ok().map(Cow::Borrowed)
    }
}

