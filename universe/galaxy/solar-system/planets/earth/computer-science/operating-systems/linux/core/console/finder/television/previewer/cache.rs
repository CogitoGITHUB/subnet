use rustc_hash::FxHashMap;

use crate::utils::cache::RingSet;
use ratatui::text::Text;
use tracing::debug;

/// Default size of the preview cache: 50 entries.
///
/// This does seem kind of arbitrary for now, will need to play around with it.
/// Assuming a worst case scenario where files are an average of 1 MB this means
/// the cache will never exceed 50 MB which sounds safe enough.
const DEFAULT_CACHE_SIZE: usize = 50;

/// A cache for previews.
/// The cache is implemented as an LRU cache with a fixed size.
#[derive(Debug)]
pub struct Cache {
    entries: FxHashMap<String, Text<'static>>,
    ring_set: RingSet<String>,
}

impl Cache {
    /// Create a new preview cache with the given capacity.
    pub fn new(capacity: usize) -> Self {
        Cache {
            entries: FxHashMap::default(),
            ring_set: RingSet::with_capacity(capacity),
        }
    }

    pub fn get(&self, key: &str) -> Option<Text<'static>> {
        self.entries.get(key).cloned()
    }

    /// Insert a new preview into the cache.
    /// If the cache is full, the oldest entry will be removed.
    /// If the key is already in the cache, the preview will be updated.
    pub fn insert(&mut self, key: &str, text: &Text<'static>) {
        debug!("Inserting preview into cache for key: {:?}", key);
        let key = key.to_string();
        self.entries.insert(key.clone(), text.clone());
        if let Some(oldest_key) = self.ring_set.push(key) {
            debug!("Cache full, removing oldest entry: {:?}", oldest_key);
            self.entries.remove(&oldest_key);
        }
    }

    /// In this context, the size represents the number of occupied slots within the cache.
    /// Not to be mistaken with the cache's capacity which is its max size.
    pub fn size(&self) -> usize {
        self.ring_set.size()
    }

    pub fn clear(&mut self) {
        debug!("Clearing preview cache");
        self.entries.clear();
        self.ring_set.clear();
    }
}

impl Default for Cache {
    fn default() -> Self {
        Cache::new(DEFAULT_CACHE_SIZE)
    }
}

