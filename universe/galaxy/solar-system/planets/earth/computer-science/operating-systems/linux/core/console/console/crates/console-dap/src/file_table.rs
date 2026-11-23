//! File identity: one [`FileId`] per file, however the path was spelled.
//!
//! The same file arrives spelled differently — absolute from the client's
//! `setBreakpoints`, bare from the engine, which records `source helper.nu`
//! under exactly that name. A breakpoint set through one spelling and hit
//! through the other must still match, so [`FileTable`] canonicalizes and
//! interns every spelling on the way in, and everything downstream
//! (breakpoints, valid lines, [`crate::source_map`]) is keyed by the id.
//! Canonicalization happens here and nowhere else, so nothing can drift.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// A file's identity for the length of a debug session. Comparing ids instead
/// of path strings is what makes lookups spelling-proof, and keeps the
/// per-instruction breakpoint probe off string hashing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct FileId(u32);

/// The session's path <-> [`FileId`] table.
///
/// Shared because the two sides of the join run on different threads: the
/// server thread interns the `setBreakpoints` path, the eval thread the names
/// in `EngineState::files()`. Owned by the `Session` and outliving `restart`,
/// so carried-over breakpoints keep pointing at the files they were set in.
///
/// Lock discipline: innermost and short-lived — never taken while holding
/// `SessionState`.
#[derive(Clone, Default)]
pub(crate) struct FileTable(Arc<Mutex<FileTableInner>>);

#[derive(Default)]
struct FileTableInner {
    ids: HashMap<String, FileId>,
    paths: Vec<String>,
}

impl FileTable {
    /// Intern a path: the one place a spelling becomes an identity.
    ///
    /// Relative spellings resolve against the *live* process cwd, which
    /// `engine::Target::enter_cwd` moves to the script's directory before a
    /// run. That is deliberate: the relative names nu records for `source`d
    /// files are relative to exactly that directory, so interning them there
    /// is what lets them meet the client's absolute path on one id. The
    /// consequence to keep in mind is that the same *relative* spelling
    /// interned before the run and during it can land on different ids;
    /// clients send absolute paths, so it does not bite in practice.
    pub(crate) fn intern(&self, path: impl AsRef<Path>) -> FileId {
        let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
        let key = console_path::canonicalize_with(path.as_ref(), cwd)
            .map(|c| c.to_string_lossy().into_owned())
            .unwrap_or_else(|_| path.as_ref().to_string_lossy().into_owned());

        let mut inner = self.0.lock();
        if let Some(&id) = inner.ids.get(&key) {
            return id;
        }
        let id = FileId(inner.paths.len() as u32);
        inner.paths.push(key.clone());
        inner.ids.insert(key, id);
        id
    }

    /// The canonical path `id` was interned from — what the client is handed
    /// as a DAP `Source` and must be able to open. Ids only come from
    /// [`Self::intern`], so the fallback is unreachable.
    pub(crate) fn path(&self, id: FileId) -> String {
        let inner = self.0.lock();
        inner.paths.get(id.0 as usize).cloned().unwrap_or_default()
    }
}

