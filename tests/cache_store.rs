//! The cache store against a real filesystem backend.
//!
//! The library is sans-I/O, so [`reticle::cache::Storage`] is the seam a
//! persistent store plugs into. This file implements one — a directory of
//! one file per entry, the same shape the `reticle` binary uses — and
//! checks that the cache behaves over it exactly as it does over
//! [`reticle::cache::MemoryStorage`]: entries survive a restart, eviction
//! keeps the least-recently-used ones, and a file damaged behind the
//! cache's back is caught rather than served.
#![cfg(feature = "cache")]

use std::fs;
use std::path::{Path, PathBuf};

use reticle::cache::{Cache, CacheKey, Entry, MemoryStorage, Problem, Storage};

/// A `Storage` over one directory, one file per entry.
///
/// This is the whole of what a backend has to do; see
/// `src/bin/reticle/cache_store.rs` for the binary's copy, which adds
/// atomic replacement.
struct FileStorage {
    dir: PathBuf,
}

impl FileStorage {
    fn new(dir: impl Into<PathBuf>) -> FileStorage {
        let dir = dir.into();
        fs::create_dir_all(&dir).expect("a writable scratch directory");
        FileStorage { dir }
    }

    fn path(&self, key: CacheKey) -> PathBuf {
        self.dir.join(format!("{}.entry", key.to_hex()))
    }
}

impl Storage for FileStorage {
    fn get(&self, key: CacheKey) -> Option<Vec<u8>> {
        fs::read(self.path(key)).ok()
    }

    fn put(&mut self, key: CacheKey, value: Vec<u8>) {
        let _ = fs::write(self.path(key), value);
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
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "entry"))
            .filter_map(|p| CacheKey::from_hex(p.file_stem()?.to_str()?))
            .collect();
        keys.sort_unstable();
        keys
    }
}

/// A scratch directory unique to one test.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    dir
}

/// A key that is easy to write down; real ones come from a `KeyBuilder`.
fn key(n: u64) -> CacheKey {
    CacheKey::from_hash(reticle::cache::Hash128 { hi: 0, lo: n })
}

#[test]
fn entries_survive_the_process_that_wrote_them() {
    let dir = scratch("cache-store-persist");
    {
        let mut storage = FileStorage::new(&dir);
        let mut cache = Cache::new(&mut storage);
        cache.insert(
            key(1),
            "elab:verilog",
            1_758_499_200,
            b"module m\nend\n".to_vec(),
        );
        assert_eq!(cache.len(), 1);
    }

    // A fresh store over the same directory finds it, with its metadata.
    let mut storage = FileStorage::new(&dir);
    let mut cache = Cache::new(&mut storage);
    assert_eq!(cache.len(), 1);
    let entry = cache.get(key(1)).expect("a hit after a restart");
    assert_eq!(entry.text(), Some("module m\nend\n"));
    assert_eq!(entry.producer, "elab:verilog");
    assert_eq!(entry.created, 1_758_499_200);
    assert_eq!(cache.hits(), 1);

    // And the file on disk is readable text, header first.
    let raw = fs::read_to_string(dir.join(format!("{}.entry", key(1).to_hex()))).unwrap();
    assert!(raw.starts_with("reticle-cache 1\n"), "{raw}");
    assert!(raw.contains("producer elab:verilog\n"), "{raw}");
    assert!(raw.ends_with("\n\nmodule m\nend\n"), "{raw}");
}

#[test]
fn the_least_recently_used_entry_is_the_one_evicted() {
    let dir = scratch("cache-store-evict");
    let mut storage = FileStorage::new(&dir);
    let one = Entry::new(key(0), "elab", vec![b'x'; 128]).encode().len();
    let capacity = u64::try_from(one * 3).unwrap();

    let mut cache = Cache::with_capacity(&mut storage, capacity);
    for n in 1..=3 {
        cache.insert(key(n), "elab", 0, vec![b'x'; 128]);
    }
    assert_eq!(cache.len(), 3);
    assert!(cache.get(key(1)).is_some(), "touch 1, so 2 is the oldest");
    cache.insert(key(4), "elab", 0, vec![b'x'; 128]);

    assert_eq!(cache.evicted(), 1);
    assert!(cache.total_bytes() <= capacity);
    assert!(cache.peek(key(2)).is_none());
    for n in [1, 3, 4] {
        assert!(cache.peek(key(n)).is_some(), "{n} should still be there");
    }
    // The eviction reached the filesystem, not just the index.
    assert!(!dir.join(format!("{}.entry", key(2).to_hex())).exists());
}

#[test]
fn a_file_damaged_behind_the_cache_is_caught() {
    let dir = scratch("cache-store-corrupt");
    let mut storage = FileStorage::new(&dir);
    {
        let mut cache = Cache::new(&mut storage);
        cache.insert(key(1), "elab", 0, b"module m\nend\n".to_vec());
        cache.insert(key(2), "elab", 0, b"module n\nend\n".to_vec());
    }

    // Edit one entry's artefact, as a bad disk or a careless hand would.
    let path = dir.join(format!("{}.entry", key(1).to_hex()));
    let text = fs::read_to_string(&path)
        .unwrap()
        .replace("module m", "module X");
    fs::write(&path, text).unwrap();
    // And copy a good entry to a key it does not belong to.
    let good = fs::read(dir.join(format!("{}.entry", key(2).to_hex()))).unwrap();
    fs::write(dir.join(format!("{}.entry", key(3).to_hex())), good).unwrap();

    let mut cache = Cache::new(&mut storage);
    let bad = cache.verify();
    assert_eq!(bad.len(), 2, "{bad:?}");
    assert!(
        bad.iter()
            .any(|c| c.key == key(1) && matches!(c.problem, Problem::Unreadable(_)))
    );
    assert!(
        bad.iter()
            .any(|c| c.key == key(3) && c.problem == Problem::Misfiled(key(2)))
    );

    // Neither damaged entry is ever served.
    assert!(cache.get(key(1)).is_none());
    assert!(cache.get(key(3)).is_none());
    assert!(cache.get(key(2)).is_some());

    // Purging leaves a store that verifies clean.
    let left = cache.verify_and_purge();
    assert!(left.is_empty(), "the bad entries were dropped on lookup");
    assert!(cache.verify().is_empty());
}

#[test]
fn a_filesystem_store_matches_the_in_memory_one() {
    // The trait is the whole contract: the same calls must give the same
    // answers whichever backend is behind it.
    let dir = scratch("cache-store-parity");
    let mut disk = FileStorage::new(&dir);
    let mut memory = MemoryStorage::new();

    let mut on_disk = Cache::new(&mut disk);
    let mut in_memory = Cache::new(&mut memory);
    for n in 1..=5 {
        let content = format!("module m{n}\nend\n").into_bytes();
        on_disk.insert(key(n), "elab", 7, content.clone());
        in_memory.insert(key(n), "elab", 7, content);
    }
    assert_eq!(on_disk.keys(), in_memory.keys());
    assert_eq!(on_disk.total_bytes(), in_memory.total_bytes());
    assert_eq!(on_disk.verify(), in_memory.verify());
    for n in 1..=6 {
        assert_eq!(
            on_disk.get(key(n)).map(|e| e.content),
            in_memory.get(key(n)).map(|e| e.content)
        );
    }
    assert_eq!(on_disk.hits(), in_memory.hits());
    assert_eq!(on_disk.misses(), in_memory.misses());
}

/// The `reticle cache` command over the same store, end to end.
#[cfg(feature = "cli")]
mod cli {
    use super::*;
    use std::process::Command;

    fn reticle(dir: &Path, args: &[&str]) -> (i32, String, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_reticle"))
            .current_dir(dir)
            .args(args)
            .output()
            .expect("failed to run the reticle binary");
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    /// Copies the fixture design into a scratch directory.
    fn fixture(name: &str) -> PathBuf {
        let dir = scratch(name);
        fs::create_dir_all(&dir).unwrap();
        let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/cache");
        for file in ["leaf.v", "mid.v", "other.v", "top.v"] {
            fs::copy(from.join(file), dir.join(file)).unwrap();
        }
        dir
    }

    const FILES: &[&str] = &["leaf.v", "mid.v", "other.v", "top.v"];

    fn build(dir: &Path, extra: &[&str]) -> (i32, String, String) {
        let mut args = vec!["cache", "--top", "top", "--stats"];
        args.extend_from_slice(extra);
        args.extend_from_slice(FILES);
        reticle(dir, &args)
    }

    #[test]
    fn a_second_run_hits_and_writes_the_same_design() {
        let dir = fixture("cache-cli");

        let (code, stdout, stderr) = build(&dir, &["--output", "first.rtl"]);
        assert_eq!(code, 0, "{stderr}");
        assert!(stdout.contains("leaf: elaborate, miss"), "{stdout}");
        assert!(stderr.contains("0 cache hits"), "{stderr}");

        let (code, stdout, stderr) = build(&dir, &["--output", "second.rtl"]);
        assert_eq!(code, 0, "{stderr}");
        assert!(stdout.contains("leaf: elaborate, hit"), "{stdout}");
        assert!(stdout.contains("misses: 0"), "{stdout}");
        assert_eq!(
            fs::read_to_string(dir.join("first.rtl")).unwrap(),
            fs::read_to_string(dir.join("second.rtl")).unwrap(),
            "a cached build must give the same bytes as a cold one"
        );

        // The store is a directory of readable entries.
        let (code, stdout, _) = reticle(&dir, &["cache", "--list"]);
        assert_eq!(code, 0);
        assert!(stdout.contains("elab:verilog"), "{stdout}");

        let (code, stdout, _) = reticle(&dir, &["cache", "--verify"]);
        assert_eq!(code, 0);
        assert!(stdout.contains("0 damaged"), "{stdout}");

        // Editing a leaf invalidates it and everything above it.
        let leaf = dir.join("leaf.v");
        let text = fs::read_to_string(&leaf).unwrap().replace("~a", "a");
        fs::write(&leaf, text).unwrap();
        let (code, stdout, _) = build(&dir, &[]);
        assert_eq!(code, 0);
        assert!(stdout.contains("leaf: elaborate, miss"), "{stdout}");
        assert!(stdout.contains("mid: elaborate, miss"), "{stdout}");
        assert!(stdout.contains("top: elaborate, miss"), "{stdout}");
        assert!(stdout.contains("other: elaborate, hit"), "{stdout}");
    }

    #[test]
    fn a_damaged_store_fails_verification_and_clears() {
        let dir = fixture("cache-cli-damage");
        assert_eq!(build(&dir, &[]).0, 0);

        let store = dir.join(".reticle-cache");
        let victim = fs::read_dir(&store)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "entry"))
            .expect("the store has entries");
        fs::write(&victim, b"this is not a cache entry").unwrap();

        let (code, _, stderr) = reticle(&dir, &["cache", "--verify"]);
        assert_eq!(code, 1, "a damaged store must not pass verification");
        assert!(stderr.contains("error:"), "{stderr}");

        let (code, stdout, _) = reticle(&dir, &["cache", "--clear"]);
        assert_eq!(code, 0);
        assert!(stdout.contains("removed"), "{stdout}");
        let (_, stdout, _) = reticle(&dir, &["cache", "--verify"]);
        assert!(stdout.starts_with("0 entries"), "{stdout}");
    }
}
