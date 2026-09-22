//! A directory-backed [`Storage`] for the incremental build cache.
//!
//! The library is sans-I/O, so it ships an in-memory store and leaves the
//! persistent one to whoever drives it. This is that one: a flat directory
//! with one file per entry, named for its key plus `.entry`.
//!
//! ```text
//! .reticle-cache/
//!   0f3a....c1.entry
//!   4b12....9e.entry
//! ```
//!
//! An entry's bytes are exactly what
//! [`Entry::encode`](reticle::cache::Entry::encode) produced, so the store
//! can be inspected with `cat`, copied between machines and pruned with
//! `rm` without the cache losing track: a file that vanishes is a miss, and
//! one that is damaged is caught by
//! [`Cache::verify`](reticle::cache::Cache::verify).
//!
//! Failures are swallowed on purpose. A cache that cannot be written is a
//! slow build, not a broken one, so an unwritable directory or a full disk
//! must not fail a compile — the entry is simply not stored and the next
//! build misses again.

use std::fs;
use std::path::PathBuf;

use reticle::cache::{CacheKey, Storage};

/// The file extension every entry is written under.
const EXT: &str = "entry";

/// A [`Storage`] over one directory.
pub(crate) struct FileStorage {
    dir: PathBuf,
    /// True once the directory is known to exist, so a build does not call
    /// `create_dir_all` once per entry.
    ready: bool,
}

impl FileStorage {
    /// A store in `dir`, which is created on the first write.
    pub(crate) fn new(dir: impl Into<PathBuf>) -> FileStorage {
        FileStorage {
            dir: dir.into(),
            ready: false,
        }
    }

    /// The file one key is stored in.
    fn path(&self, key: CacheKey) -> PathBuf {
        self.dir.join(format!("{}.{EXT}", key.to_hex()))
    }
}

impl Storage for FileStorage {
    fn get(&self, key: CacheKey) -> Option<Vec<u8>> {
        fs::read(self.path(key)).ok()
    }

    fn put(&mut self, key: CacheKey, value: Vec<u8>) {
        if !self.ready {
            if fs::create_dir_all(&self.dir).is_err() {
                return;
            }
            self.ready = true;
        }
        // Write beside the target and rename, so a build that is killed
        // half way through leaves no truncated entry behind. A rename that
        // fails leaves the temporary file, which `keys` ignores because it
        // does not end in `.entry`.
        let temp = self.path(key).with_extension("tmp");
        if fs::write(&temp, value).is_ok() && fs::rename(&temp, self.path(key)).is_err() {
            let _ = fs::remove_file(&temp);
        }
    }

    fn remove(&mut self, key: CacheKey) {
        let _ = fs::remove_file(self.path(key));
    }

    fn keys(&self) -> Vec<CacheKey> {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut keys: Vec<CacheKey> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|e| e == EXT))
            .filter_map(|path| {
                let stem = path.file_stem()?.to_str()?;
                CacheKey::from_hex(stem)
            })
            .collect();
        // The directory order is whatever the filesystem gives; the trait
        // promises ascending keys so eviction is deterministic.
        keys.sort_unstable();
        keys
    }

    fn size(&self, key: CacheKey) -> Option<u64> {
        // From the directory entry, so opening a store costs one `stat`
        // per entry rather than a read of the whole thing.
        fs::metadata(self.path(key)).ok().map(|meta| meta.len())
    }
}
