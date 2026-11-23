#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::payload::Payload;

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

const STALE_LOCK_THRESHOLD: Duration = Duration::from_secs(30);

/// Outcome of the fetch that produced this cache entry. Surfaced so rendering and diagnostics
/// can tell a successful fetch from one that failed or timed out without peeking at the body.
/// Serialized in lowercase (`"ok"`, `"err"`, `"timeout"`); missing from an older cache file
/// deserializes back to `Ok` via `#[serde(default)]` on the [`CacheEntry::kind`] field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CacheEntryKind {
    #[default]
    Ok,
    Err,
    Timeout,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CacheEntry {
    pub refreshed_at: u64,
    pub ttl_seconds: u64,
    #[serde(default)]
    pub kind: CacheEntryKind,
    pub payload: Payload,
}

impl CacheEntry {
    pub fn new(payload: Payload, ttl_seconds: u64) -> Self {
        Self::new_with_kind(payload, ttl_seconds, CacheEntryKind::Ok)
    }

    pub fn new_with_kind(payload: Payload, ttl_seconds: u64, kind: CacheEntryKind) -> Self {
        Self {
            refreshed_at: now_secs(),
            ttl_seconds,
            kind,
            payload,
        }
    }

    pub fn is_fresh(&self) -> bool {
        age_of(self.refreshed_at) < self.ttl_seconds
    }
}

pub struct Cache {
    dir: PathBuf,
}

impl Cache {
    pub fn open_default() -> Option<Self> {
        Self::open(crate::paths::cache_dir()?)
    }

    pub fn open(dir: PathBuf) -> Option<Self> {
        std::fs::create_dir_all(&dir).ok()?;
        Some(Self { dir })
    }

    pub fn load(&self, widget_id: &str) -> Option<CacheEntry> {
        let data = std::fs::read_to_string(self.path_for(widget_id)).ok()?;
        serde_json::from_str(&data).ok()
    }

    pub fn store(&self, widget_id: &str, entry: &CacheEntry) -> std::io::Result<()> {
        let path = self.path_for(widget_id);
        let tmp = tmp_path_for(&path);
        let json = serde_json::to_string(entry).map_err(io_err)?;
        ensure_parent(&path)?;
        std::fs::write(&tmp, json)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }

    pub fn path_for(&self, widget_id: &str) -> PathBuf {
        self.dir.join(format!("{}.json", sanitize(widget_id)))
    }
}

/// Abstraction over the storage backend used by `fetch_all` / `load_entries`. The one-shot
/// splash uses the disk-backed `Cache`; `watch` runs against `MemCache` because the disk is
/// only justified as IPC between the splash parent and the detached fetch daemon — `watch`
/// has no child process, so a write-through to disk on every refresh is pure waste.
pub trait CacheBackend: Send + Sync {
    fn load(&self, key: &str) -> Option<CacheEntry>;
    fn store(&self, key: &str, entry: &CacheEntry) -> std::io::Result<()>;
    /// Non-blocking. `None` if a parallel fetch already holds the slot. Held across the fetch
    /// await so the same widget can't be refetched twice in one pass.
    fn try_lock(&self, key: &str) -> Option<CacheLockGuard>;
}

/// Token returned by [`CacheBackend::try_lock`]. Dropping it releases the lock — removes the
/// `.lock` file for the disk path, takes the key out of the in-flight set for the in-memory one.
pub enum CacheLockGuard {
    File(Lock),
    Memory(MemLockGuard),
}

impl CacheBackend for Cache {
    fn load(&self, key: &str) -> Option<CacheEntry> {
        Cache::load(self, key)
    }
    fn store(&self, key: &str, entry: &CacheEntry) -> std::io::Result<()> {
        Cache::store(self, key, entry)
    }
    fn try_lock(&self, key: &str) -> Option<CacheLockGuard> {
        Lock::try_acquire(&self.dir, key).map(CacheLockGuard::File)
    }
}

/// In-memory cache backend for `watch`. Same TTL semantics as the disk `Cache` (entries carry
/// `refreshed_at` + `ttl_seconds`), but all reads and writes are HashMap operations and the
/// per-key locks are a HashSet of in-flight keys. The disk `Cache` is only touched for
/// warm-start at the top of `watch` and best-effort flush at the bottom.
#[derive(Clone, Default)]
pub struct MemCache {
    entries: Arc<RwLock<HashMap<String, CacheEntry>>>,
    in_flight: Arc<Mutex<HashSet<String>>>,
    /// Monotonic counter bumped on every successful `store`. Watch's foreground reads it to
    /// decide whether to re-walk the entries map at all — when it hasn't advanced since last
    /// check, nothing has been written, and the per-frame cache read can be skipped.
    generation: Arc<AtomicU64>,
}

impl MemCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Current write generation. Use this to short-circuit redundant `load` walks when the
    /// memory map hasn't been mutated since last check.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Copy entries matching `keys` from `disk` into a fresh `MemCache`. Missing keys are
    /// skipped — the watch fetch loop will populate them on its first pass.
    pub fn warm_from<'a>(disk: Option<&Cache>, keys: impl IntoIterator<Item = &'a str>) -> Self {
        let mem = Self::new();
        if let (Some(d), Ok(mut entries)) = (disk, mem.entries.write()) {
            entries.extend(
                keys.into_iter()
                    .filter_map(|k| d.load(k).map(|e| (k.to_string(), e))),
            );
        }
        mem
    }

    /// Write every in-memory entry back to disk. Errors per-key are swallowed — losing one
    /// write on shutdown only forces a refetch next session.
    pub fn flush_to(&self, disk: &Cache) {
        let Ok(entries) = self.entries.read() else {
            return;
        };
        for (key, entry) in entries.iter() {
            let _ = disk.store(key, entry);
        }
    }
}

impl CacheBackend for MemCache {
    fn load(&self, key: &str) -> Option<CacheEntry> {
        self.entries.read().ok()?.get(key).cloned()
    }
    fn store(&self, key: &str, entry: &CacheEntry) -> std::io::Result<()> {
        let Ok(mut e) = self.entries.write() else {
            return Ok(());
        };
        e.insert(key.to_string(), entry.clone());
        drop(e);
        self.generation.fetch_add(1, Ordering::Release);
        Ok(())
    }
    fn try_lock(&self, key: &str) -> Option<CacheLockGuard> {
        let mut set = self.in_flight.lock().ok()?;
        if set.contains(key) {
            return None;
        }
        set.insert(key.to_string());
        Some(CacheLockGuard::Memory(MemLockGuard {
            key: key.to_string(),
            in_flight: Arc::clone(&self.in_flight),
        }))
    }
}

pub struct MemLockGuard {
    key: String,
    in_flight: Arc<Mutex<HashSet<String>>>,
}

impl Drop for MemLockGuard {
    fn drop(&mut self) {
        if let Ok(mut set) = self.in_flight.lock() {
            set.remove(&self.key);
        }
    }
}

pub struct Lock {
    path: PathBuf,
}

impl Lock {
    pub fn try_acquire(dir: &Path, key: &str) -> Option<Self> {
        Self::try_acquire_with(dir, key, STALE_LOCK_THRESHOLD)
    }

    fn try_acquire_with(dir: &Path, key: &str, stale_after: Duration) -> Option<Self> {
        let path = lock_path_for(dir, key);
        match create_exclusive(&path) {
            Ok(()) => Some(Self { path }),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if is_lock_stale(&path, stale_after) {
                    let _ = std::fs::remove_file(&path);
                    create_exclusive(&path).ok().map(|_| Self { path })
                } else {
                    None
                }
            }
            Err(_) => None,
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn lock_path_for(dir: &Path, key: &str) -> PathBuf {
    dir.join(format!("{}.lock", sanitize(key)))
}

fn create_exclusive(path: &Path) -> std::io::Result<()> {
    ensure_parent(path)?;
    let mut f = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)?;
    writeln!(f, "{}", std::process::id())?;
    Ok(())
}

fn is_lock_stale(path: &Path, threshold: Duration) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    let Ok(mtime) = meta.modified() else {
        return false;
    };
    mtime.elapsed().map(|d| d > threshold).unwrap_or(false)
}

/// Builds a per-invocation unique tmp path next to `final_path`. Pid keeps it distinct across
/// processes; the atomic counter keeps it distinct across concurrent calls within one process.
/// Exported so other modules (e.g. trust store) can share the write-tmp-then-rename idiom.
pub(crate) fn tmp_path_for(final_path: &Path) -> PathBuf {
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let file_name = final_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    final_path.with_file_name(format!("{file_name}.{pid}.{seq}.tmp"))
}

/// Maps an arbitrary cache key to an on-disk filename stem by keeping ASCII alnum / `-` / `_`
/// and replacing everything else with `_`. Exposed because the `splashboard cache clear <id>`
/// command (in the binary crate) needs to compute the same stem from the same key — keeping a
/// private copy in `main.rs` would silently desync if this rule ever tightens. Marked
/// `#[doc(hidden)]` because external library consumers should not rely on it.
#[doc(hidden)]
pub fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn ensure_parent(path: &Path) -> std::io::Result<()> {
    match path.parent() {
        Some(p) => std::fs::create_dir_all(p),
        None => Ok(()),
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn age_of(past: u64) -> u64 {
    now_secs().saturating_sub(past)
}

fn io_err<E: std::fmt::Display>(e: E) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

