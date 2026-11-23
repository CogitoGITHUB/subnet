#![allow(dead_code)]

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::cache::tmp_path_for;
use crate::config::{DashboardConfig, WidgetConfig};
use crate::fetcher::{Registry, Safety};
use crate::payload::Payload;

const TRUST_ALL_ENV: &str = "SPLASH_TRUST_ALL";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustDecision {
    /// Global config, baked-in default, or `SPLASH_TRUST_ALL` set. No store entry needed.
    ImplicitlyTrusted,
    /// Local config whose hash matches an entry in the trust store.
    Trusted,
    /// Local config with no entry, or an entry whose hash no longer matches (config edited).
    Untrusted,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrustStore {
    #[serde(default, rename = "entry")]
    entries: Vec<TrustEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrustEntry {
    pub path: PathBuf,
    pub sha256: String,
}

impl TrustStore {
    /// Loads the on-disk store, or returns an empty store if it's missing or corrupt. A broken
    /// trust store must never cause the splash to misbehave; the worst case is that the user
    /// needs to re-run `splash trust`.
    pub fn load() -> Self {
        store_path()
            .and_then(|p| std::fs::read_to_string(&p).ok())
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Atomic write so a concurrent invocation or crashed write never leaves the store empty
    /// (which would silently untrust every entry). Same tmp+rename idiom as the payload cache.
    pub fn save(&self) -> io::Result<()> {
        let Some(path) = store_path() else {
            return Err(io::Error::other("could not resolve trust store path"));
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let body = toml::to_string(self).map_err(io::Error::other)?;
        let tmp = tmp_path_for(&path);
        std::fs::write(&tmp, body)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }

    /// Caller is expected to have already read the dashboard bytes and hashed them (via
    /// [`load_dashboard_and_hash`]). Re-hashing here would open a TOCTOU window: a trusted file
    /// could be swapped for an attacker-crafted one between the caller's load and our hash.
    ///
    /// `ident = None` signals a HOME-backed source (settings, `home.dashboard.toml`, or
    /// `project.dashboard.toml`) — always implicitly trusted because the user owns HOME.
    pub fn decide(&self, ident: Option<(&Path, &str)>) -> TrustDecision {
        if trust_all_override() {
            return TrustDecision::ImplicitlyTrusted;
        }
        let Some((path, hash)) = ident else {
            return TrustDecision::ImplicitlyTrusted;
        };
        let canon = canonicalize_or(path);
        for entry in &self.entries {
            if canonicalize_or(&entry.path) == canon && entry.sha256 == hash {
                return TrustDecision::Trusted;
            }
        }
        TrustDecision::Untrusted
    }

    pub fn trust(&mut self, path: &Path, hash: String) -> io::Result<()> {
        let canon = path.canonicalize()?;
        self.entries.retain(|e| canonicalize_or(&e.path) != canon);
        self.entries.push(TrustEntry {
            path: canon,
            sha256: hash,
        });
        self.save()
    }

    pub fn revoke(&mut self, path: &Path) -> io::Result<bool> {
        let canon = canonicalize_or(path);
        let before = self.entries.len();
        self.entries.retain(|e| canonicalize_or(&e.path) != canon);
        let removed = self.entries.len() < before;
        if removed {
            self.save()?;
        }
        Ok(removed)
    }

    pub fn list(&self) -> &[TrustEntry] {
        &self.entries
    }
}

/// Reads the dashboard bytes once and returns both the parsed `DashboardConfig` and its
/// sha256. All trust-sensitive callers MUST use this instead of `DashboardConfig::parse` +
/// `hash_file` — two separate reads would let an attacker swap the file between the load and
/// the hash.
pub fn load_dashboard_and_hash(path: &Path) -> io::Result<(DashboardConfig, String)> {
    let bytes = std::fs::read(path)?;
    let hash = hash_bytes(&bytes);
    let text = std::str::from_utf8(&bytes).map_err(io::Error::other)?;
    let dashboard = DashboardConfig::parse(text).map_err(io::Error::other)?;
    Ok((dashboard, hash))
}

pub fn hash_file(path: &Path) -> io::Result<String> {
    Ok(hash_bytes(&std::fs::read(path)?))
}

pub fn hash_bytes(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Partitions widgets into (fetchable, gated). Widgets whose fetcher requires trust
/// (Network or Exec) are moved to `gated` when the config is untrusted; callers should render
/// `requires_trust_placeholder()` for those slots instead of fetching.
pub fn partition_by_trust(
    widgets: &[WidgetConfig],
    registry: &Registry,
    decision: TrustDecision,
) -> (Vec<WidgetConfig>, Vec<WidgetConfig>) {
    if !matches!(decision, TrustDecision::Untrusted) {
        return (widgets.to_vec(), Vec::new());
    }
    widgets
        .iter()
        .cloned()
        .partition(|w| match registry.get(&w.fetcher) {
            Some(f) => matches!(f.safety(), Safety::Safe),
            None => true,
        })
}

pub fn requires_trust_placeholder() -> Payload {
    Payload {
        icon: None,
        status: Some(crate::payload::Status::Error),
        format: None,
        body: crate::payload::Body::Error(crate::payload::ErrorData {
            kind: crate::payload::ErrorKind::RequiresTrust,
            message: "🔒 requires trust".into(),
            detail: Some("run `splash trust`".into()),
        }),
    }
}

fn trust_all_override() -> bool {
    matches!(
        std::env::var(TRUST_ALL_ENV).ok().as_deref(),
        Some("1") | Some("true")
    )
}

fn canonicalize_or(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn store_path() -> Option<PathBuf> {
    crate::paths::trust_store_path()
}

