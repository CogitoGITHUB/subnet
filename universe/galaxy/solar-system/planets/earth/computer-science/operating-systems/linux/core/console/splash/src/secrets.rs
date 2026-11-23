//! Loader for `$HOME/.splash/secrets.toml` — a flat top-level TOML map of
//! `KEY = "value"` pairs that get exported as process env at startup. Lives in its own
//! file (not `settings.toml`) so dotfiles-in-git users can git-ignore exactly the file
//! that holds tokens without losing their settings.
//!
//! Shell env wins: a key already present in the environment is left alone, so a
//! `GH_TOKEN=... splash ...` style override still works.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

/// Keys we refuse to import from `secrets.toml`. Two reasons:
///
/// - `SPLASH_*` controls splash's own behavior (HOME, log filter, trust escape
///   hatch). Letting a config file flip those creates a confusing layered setup.
/// - `PATH` / `HOME` / `SHELL` etc. are ambient process state owned by the shell. A typo
///   here could redirect `git2`/`gix` to a different binary or break HOME discovery.
const DENYLIST: &[&str] = &[
    "PATH",
    "HOME",
    "SHELL",
    "USER",
    "LOGNAME",
    "PWD",
    "OLDPWD",
    "IFS",
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "DYLD_INSERT_LIBRARIES",
    "DYLD_LIBRARY_PATH",
];

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(transparent)]
pub struct SecretsConfig {
    entries: BTreeMap<String, String>,
}

impl SecretsConfig {
    pub fn parse(toml_str: &str) -> Result<Self, String> {
        toml::from_str(toml_str).map_err(|e| e.to_string())
    }

    /// Loads the file if present, returns an empty config if missing, errors on parse
    /// failure. Same shape as `SettingsConfig::load_or_default`.
    pub fn load_or_default(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(s) => Self::parse(&s).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// Iterate the entries that survived denylist + `SPLASH_*` filtering. Skips
    /// silently rather than erroring so a stray entry doesn't tank the whole splash.
    pub fn importable(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries
            .iter()
            .filter(|(k, _)| is_importable(k))
            .map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Apply each entry to the process env, but only if the env doesn't already have a
    /// value for that key. Shell env always wins. Returns the keys actually set.
    pub fn apply_to_env<F, G>(&self, get: F, mut set: G) -> Vec<String>
    where
        F: Fn(&str) -> Option<String>,
        G: FnMut(&str, &str),
    {
        self.importable()
            .filter(|(k, _)| get(k).is_none())
            .map(|(k, v)| {
                set(k, v);
                k.to_string()
            })
            .collect()
    }
}

fn is_importable(key: &str) -> bool {
    !key.starts_with("SPLASH_") && !DENYLIST.contains(&key)
}

