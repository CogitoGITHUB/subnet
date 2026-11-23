pub mod env;
pub mod serde;
pub mod statusline;

use process_control::{ChildExt, Control};
use std::ffi::OsStr;
use std::fmt::Debug;
use std::fs;
use std::io::{Error, ErrorKind, Read, Result, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::context::Context;
use crate::context::Shell;

/// Default timeout for command execution in milliseconds
pub const DEFAULT_COMMAND_TIMEOUT_MS: u64 = 500;

/// Create a `PathBuf` from an absolute path, where the root directory will be mocked in test
#[cfg(not(test))]
#[inline]
#[allow(dead_code)]
pub fn context_path<S: AsRef<OsStr> + ?Sized>(_context: &Context, s: &S) -> PathBuf {
    PathBuf::from(s)
}

/// Create a `PathBuf` from an absolute path, where the root directory will be mocked in test

/// The largest file `read_file` is willing to read.
const MAX_FILE_SIZE: u64 = 100 * 1024 * 1024;

/// Return the string contents of a file
pub fn read_file<P: AsRef<Path> + Debug>(file_name: P) -> Result<String> {
    log::trace!("Trying to read from {file_name:?}");

    // `O_NONBLOCK` so that the open itself cannot hang
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(nix::fcntl::OFlag::O_NONBLOCK.bits());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS.0);
    }
    let file = options.open(file_name.as_ref())?;

    let stat = file.metadata()?;
    if !stat.is_file() {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("{file_name:?} is not a file"),
        ));
    }
    if stat.len() > MAX_FILE_SIZE {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("{file_name:?} is too large"),
        ));
    }

    // Bound the read as well, so a file that grows once it is open, or one whose
    // reported size is wrong, still cannot be read without limit.
    let mut out = String::new();
    #[allow(clippy::disallowed_methods)]
    let result = file
        .take(MAX_FILE_SIZE + 1)
        .read_to_string(&mut out)
        .and_then(|read| {
            if read as u64 > MAX_FILE_SIZE {
                Err(Error::new(
                    ErrorKind::InvalidData,
                    format!("{file_name:?} is too large"),
                ))
            } else {
                Ok(())
            }
        });

    if result.is_err() {
        log::debug!("Error reading file: {result:?}");
    } else {
        log::trace!("File read successfully");
    }

    result.map(|()| out)
}

/// Write a string to a file

/// Write contents to a file by first writing to a temporary file
/// and then move it to the target location in place
/// Only overwrites existing files if `force` is true
pub fn write_file_atomic<P: AsRef<Path>, S: AsRef<str>>(
    target_path: P,
    text: S,
    force: bool,
) -> std::result::Result<(), String> {
    let target_path = target_path.as_ref();
    let text = text.as_ref();

    log::trace!("Trying to write {text:?} to {target_path:?}");

    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut builder = tempfile::Builder::new();

    // On Unix, the default permissions are too restrictive, so we need to relax them
    // This should be safe because we're creating a temporary file in the same directory
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let permissions = target_path.metadata().as_ref().map_or_else(
            |_| {
                let all_read_write = 0o666;
                std::fs::Permissions::from_mode(all_read_write)
            },
            fs::Metadata::permissions,
        );

        builder.permissions(permissions);
    }

    let Some(parent_dir) = target_path.parent() else {
        return Err(format!(
            "Unable to determine parent directory of {target_path:?}"
        ));
    };

    let mut temp_file = builder
        .tempfile_in(parent_dir)
        .map_err(|e| format!("Error creating temporary file: {e}"))?;

    if let Err(err) = temp_file.write_all(text.as_bytes()) {
        return Err(format!("Error writing to temporary file: {err}"));
    }

    let result = if force {
        temp_file.persist(target_path)
    } else {
        temp_file.persist_noclobber(target_path)
    };

    result.map_err(|e| {
        if !force && e.error.kind() == ErrorKind::AlreadyExists {
            "Error saving file, use --force to overwrite existing configuration file".to_string()
        } else {
            format!("Error moving temporary file to target location: {e}")
        }
    })?;

    log::trace!("File {target_path:?} written successfully");

    Ok(())
}

/// Reads command output from stderr or stdout depending on to which stream program streamed it's output
pub fn get_command_string_output(command: CommandOutput) -> String {
    if command.stdout.is_empty() {
        command.stderr
    } else {
        command.stdout
    }
}

/// Attempt to resolve `binary_name` from and creates a new `Command` pointing at it
/// This allows executing cmd files on Windows and prevents running executable from cwd on Windows
/// This function also initializes std{err,out,in} to protect against processes changing the console mode
pub fn create_command<T: AsRef<OsStr>>(binary_name: T) -> Result<Command> {
    let binary_name = binary_name.as_ref();
    log::trace!("Creating Command for binary {binary_name:?}");

    let full_path = match which::which(binary_name) {
        Ok(full_path) => {
            log::trace!("Using {full_path:?} as {binary_name:?}");
            full_path
        }
        Err(error) => {
            log::trace!("Unable to find {binary_name:?} in PATH, {error:?}");
            return Err(Error::new(ErrorKind::NotFound, error));
        }
    };

    #[allow(clippy::disallowed_methods)]
    let mut cmd = Command::new(full_path);
    cmd.stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .stdin(Stdio::null());

    Ok(cmd)
}

#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
}

impl PartialEq for CommandOutput {
    fn eq(&self, other: &Self) -> bool {
        self.stdout == other.stdout && self.stderr == other.stderr
    }
}


/// Execute a command and return the output on stdout and stderr if successful
pub fn exec_cmd<T: AsRef<OsStr> + Debug, U: AsRef<OsStr> + Debug>(
    cmd: T,
    args: &[U],
    time_limit: Duration,
) -> Option<CommandOutput> {
    log::trace!("Executing command {cmd:?} with args {args:?}");
    internal_exec_cmd(cmd, args, time_limit)
}


/// Wraps ANSI color escape sequences in the shell-appropriate wrappers.
pub fn wrap_colorseq_for_shell(ansi: String, shell: Shell) -> String {
    const ESCAPE_BEGIN: char = '\u{1b}';
    const ESCAPE_END: char = 'm';
    wrap_seq_for_shell(ansi, shell, ESCAPE_BEGIN, ESCAPE_END)
}

/// Many shells cannot deal with raw unprintable characters and miscompute the cursor position,
/// leading to strange visual bugs like duplicated/missing chars. This function wraps a specified
/// sequence in shell-specific escapes to avoid these problems.
pub fn wrap_seq_for_shell(
    ansi: String,
    shell: Shell,
    escape_begin: char,
    escape_end: char,
) -> String {
    let (beg, end) = match shell {
        // \[ and \]
        Shell::Bash => ("\u{5c}\u{5b}", "\u{5c}\u{5d}"),
        // %{ and %}
        Shell::Tcsh | Shell::Zsh => ("\u{25}\u{7b}", "\u{25}\u{7d}"),
        _ => return ansi,
    };

    // ANSI escape codes cannot be nested, so we can keep track of whether we're
    // in an escape or not with a single boolean variable
    let mut escaped = false;
    let final_string: String = ansi
        .chars()
        .map(|x| {
            if x == escape_begin && !escaped {
                escaped = true;
                format!("{beg}{escape_begin}")
            } else if x == escape_end && escaped {
                escaped = false;
                format!("{escape_end}{end}")
            } else {
                x.to_string()
            }
        })
        .collect();
    final_string
}

fn internal_exec_cmd<T: AsRef<OsStr> + Debug, U: AsRef<OsStr> + Debug>(
    cmd: T,
    args: &[U],
    time_limit: Duration,
) -> Option<CommandOutput> {
    let mut cmd = create_command(cmd).ok()?;
    cmd.args(args);
    exec_timeout(&mut cmd, time_limit)
}

pub fn exec_timeout(cmd: &mut Command, time_limit: Duration) -> Option<CommandOutput> {
    let start = Instant::now();
    let process = match cmd.spawn() {
        Ok(process) => process,
        Err(error) => {
            log::info!("Unable to run {:?}, {:?}", cmd.get_program(), error);
            return None;
        }
    };
    match process
        .controlled_with_output()
        .time_limit(time_limit)
        .terminate_for_timeout()
        .wait()
    {
        Ok(Some(output)) => {
            let stdout_string = match String::from_utf8(output.stdout) {
                Ok(stdout) => stdout,
                Err(error) => {
                    log::warn!("Unable to decode stdout: {error:?}");
                    return None;
                }
            };
            let stderr_string = match String::from_utf8(output.stderr) {
                Ok(stderr) => stderr,
                Err(error) => {
                    log::warn!("Unable to decode stderr: {error:?}");
                    return None;
                }
            };

            log::trace!(
                "stdout: {:?}, stderr: {:?}, exit code: \"{:?}\", took {:?}",
                stdout_string,
                stderr_string,
                output.status.code(),
                start.elapsed()
            );

            if !output.status.success() {
                return None;
            }

            Some(CommandOutput {
                stdout: stdout_string,
                stderr: stderr_string,
            })
        }
        Ok(None) => {
            log::warn!("Executing command {:?} timed out.", cmd.get_program());
            log::warn!(
                "You can set command_timeout in your config to a higher value to allow longer-running commands to keep executing."
            );
            None
        }
        Err(error) => {
            log::info!(
                "Executing command {:?} failed by: {:?}",
                cmd.get_program(),
                error
            );
            None
        }
    }
}

// Render the time into a nice human-readable string
pub fn render_time(raw_millis: u128, show_millis: bool) -> String {
    // Fast returns for zero cases to render something
    match (raw_millis, show_millis) {
        (0, true) => return "0ms".into(),
        (0..=999, false) => return "0s".into(),
        _ => (),
    }

    // Calculate a simple breakdown into days/hours/minutes/seconds/milliseconds
    let (millis, raw_seconds) = (raw_millis % 1000, raw_millis / 1000);
    let (seconds, raw_minutes) = (raw_seconds % 60, raw_seconds / 60);
    let (minutes, raw_hours) = (raw_minutes % 60, raw_minutes / 60);
    let (hours, days) = (raw_hours % 24, raw_hours / 24);

    // Calculate how long the string will be to allocate once in most cases
    let result_capacity = match raw_millis {
        1..=59 => 3,
        60..=3599 => 6,
        3600..=86399 => 9,
        _ => 12,
    } + if show_millis { 5 } else { 0 };

    let components = [(days, "d"), (hours, "h"), (minutes, "m"), (seconds, "s")];

    // Concat components ito result starting from the first non-zero one
    let result = components.iter().fold(
        String::with_capacity(result_capacity),
        |acc, (component, suffix)| match component {
            0 if acc.is_empty() => acc,
            n => acc + &n.to_string() + suffix,
        },
    );

    if show_millis {
        result + &millis.to_string() + "ms"
    } else {
        result
    }
}

/// Formats an integer into a human-readable string using SI prefixes (k, M, G, T)
pub fn humanize_int(n: u64) -> String {
    if n < 1000 {
        return n.to_string();
    }

    let n = n as f64;
    let units = ["k", "M", "G", "T", "P", "E"];
    let mut unit_idx = 0;
    let mut val = n / 1000.0;

    // 999.5 and above would round to "1000" in the current unit, so promote it.
    while val >= 999.5 && unit_idx < units.len() - 1 {
        val /= 1000.0;
        unit_idx += 1;
    }

    if val < 10.0 {
        let s = format!("{:.1}{}", val, units[unit_idx]);
        if s.contains(".0") {
            s.replace(".0", "")
        } else {
            s
        }
    } else {
        format!("{:.0}{}", val, units[unit_idx])
    }
}

pub fn home_dir() -> Option<PathBuf> {
    dirs::home_dir()
}

const HEXTABLE: &[char] = &[
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f',
];

/// Encode a u8 slice into a hexadecimal string.
pub fn encode_to_hex(slice: &[u8]) -> String {
    // let mut j = 0;
    let mut dst = Vec::with_capacity(slice.len() * 2);
    for &v in slice {
        dst.push(HEXTABLE[(v >> 4) as usize] as u8);
        dst.push(HEXTABLE[(v & 0x0f) as usize] as u8);
    }
    String::from_utf8(dst).unwrap()
}

pub trait PathExt {
    /// Get device / volume info
    fn device_id(&self) -> Option<u64>;
}

#[cfg(windows)]
impl PathExt for Path {
    fn device_id(&self) -> Option<u64> {
        // Maybe it should use unimplemented!
        Some(42u64)
    }
}

#[cfg(not(windows))]
impl PathExt for Path {
    #[cfg(target_os = "linux")]
    fn device_id(&self) -> Option<u64> {
        use std::os::linux::fs::MetadataExt;
        Some(self.metadata().ok()?.st_dev())
    }

    #[cfg(all(unix, not(target_os = "linux")))]
    fn device_id(&self) -> Option<u64> {
        use std::os::unix::fs::MetadataExt;
        Some(self.metadata().ok()?.dev())
    }
}

