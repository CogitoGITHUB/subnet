#[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
use pwd::Passwd;
use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
const FALLBACK_USER_HOME_BASE_DIR: &str = "/Users";

#[cfg(target_os = "windows")]
const FALLBACK_USER_HOME_BASE_DIR: &str = "C:\\Users\\";

#[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
const FALLBACK_USER_HOME_BASE_DIR: &str = "/home";

#[cfg(all(unix, target_os = "android"))]
const FALLBACK_USER_HOME_BASE_DIR: &str = "/data";

#[cfg(target_os = "android")]
const TERMUX_HOME: &str = "/data/data/com.termux/files/home";

fn expand_tilde_with_home(path: impl AsRef<Path>, home: Option<PathBuf>) -> PathBuf {
    let path = path.as_ref();

    if !path.starts_with("~") {
        let string = path.to_string_lossy();
        let mut path_as_string = string.as_ref().bytes();
        return match path_as_string.next() {
            Some(b'~') => expand_tilde_with_another_user_home(path),
            _ => path.into(),
        };
    }

    let path_last_char = path.as_os_str().to_string_lossy().chars().next_back();
    let need_trailing_slash = path_last_char == Some('/') || path_last_char == Some('\\');

    match home {
        None => path.into(),
        Some(mut h) => {
            if h == Path::new("/") {
                // Corner case: `h` is a root directory;
                // don't prepend extra `/`, just drop the tilde.
                path.strip_prefix("~").unwrap_or(path).into()
            } else {
                if let Ok(p) = path.strip_prefix("~/") {
                    // Corner case: `p` is empty;
                    // Don't append extra '/', just keep `h` as is.
                    // This happens because PathBuf.push will always
                    // add a separator if the pushed path is relative,
                    // even if it's empty
                    if p != Path::new("") {
                        h.push(p)
                    }

                    if need_trailing_slash {
                        h.push("");
                    }
                }
                h
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn fallback_home_dir(username: &str) -> PathBuf {
    PathBuf::from_iter([FALLBACK_USER_HOME_BASE_DIR, username])
}

#[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
fn user_home_dir(username: &str) -> PathBuf {
    let passwd = Passwd::from_name(username);
    match &passwd.ok() {
        Some(Some(dir)) => PathBuf::from(&dir.dir),
        _ => fallback_home_dir(username),
    }
}

#[cfg(any(target_os = "android", target_os = "windows", target_os = "macos"))]
fn user_home_dir(username: &str) -> PathBuf {
    use std::path::Component;

    match dirs::home_dir() {
        None => {
            // Termux always has the same home directory
            #[cfg(target_os = "android")]
            if is_termux() {
                return PathBuf::from(TERMUX_HOME);
            }

            fallback_home_dir(username)
        }
        Some(user) => {
            let mut expected_path = user;

            if !cfg!(target_os = "android")
                && expected_path
                    .components()
                    .next_back()
                    .map(|last| last != Component::Normal(username.as_ref()))
                    .unwrap_or(false)
            {
                expected_path.pop();
                expected_path.push(Path::new(username));
            }

            if expected_path.is_dir() {
                expected_path
            } else {
                fallback_home_dir(username)
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn user_home_dir(_: &str) -> PathBuf {
    // if WASI is used, we try to get a home dir via HOME env, otherwise we don't have a home dir
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
    PathBuf::from(home)
}

/// Returns true if the shell is running inside the Termux terminal emulator
/// app.
#[cfg(target_os = "android")]
fn is_termux() -> bool {
    std::env::var("TERMUX_VERSION").is_ok()
}

fn expand_tilde_with_another_user_home(path: &Path) -> PathBuf {
    match path.to_str() {
        Some(file_path) => {
            let mut file = file_path.to_string();
            match file_path.find(['/', '\\']) {
                None => {
                    file.remove(0);
                    user_home_dir(&file)
                }
                Some(i) => {
                    let (pre_name, rest_of_path) = file.split_at(i);
                    let mut name = pre_name.to_string();
                    let mut rest_path = rest_of_path.to_string();
                    rest_path.remove(0);
                    name.remove(0);
                    let mut path = user_home_dir(&name);
                    path.push(Path::new(&rest_path));
                    path
                }
            }
        }
        None => path.to_path_buf(),
    }
}

/// Expand tilde ("~") into a home directory if it is the first path component
pub fn expand_tilde(path: impl AsRef<Path>) -> PathBuf {
    expand_tilde_with_home(path, dirs::home_dir())
}

