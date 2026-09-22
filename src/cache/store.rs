//! The content-addressed store: entries, backends and eviction.
//!
//! [`Cache`] is the logic; [`Storage`] is the hole a backend plugs into.
//! The library ships [`MemoryStorage`] only, because nothing under `src/`
//! except `src/bin/` touches the filesystem; the `reticle` binary supplies
//! a directory-backed one (`src/bin/reticle/cache_store.rs`), and
//! `docs/cache.md` shows how short that is for an embedder who wants a
//! different one.
//!
//! # What an entry holds
//!
//! An entry is a short text header followed by the artefact's bytes, so a
//! store can be read with `cat` and a broken entry can be diagnosed by
//! eye:
//!
//! ```text
//! reticle-cache 2
//! key 4f1c...c3
//! producer synth
//! created 1758499200
//! used 7
//! size 412
//! content 9a02...1b
//! input 5e0d...77 prog.hex
//! input missing boot.hex
//!
//! module counter
//!   ...
//! ```
//!
//! The header records what produced the artefact (`producer`), when
//! (`created`, a Unix timestamp the *caller* supplies, `0` when unknown —
//! the library has no clock of its own), its place in the least-recently-
//! used order (`used`) and its size. `key` and `content` are what
//! [`Cache::verify`] re-checks.
//!
//! The `input` lines, zero or more and sorted by path, are the files the
//! work read while it ran ([`super::inputs`]): the digest of what it found,
//! or `missing`, then the path. A key cannot cover them, because they are
//! only known once the work has run, so [`Cache::get_valid`] checks them
//! on every lookup instead.
//!
//! # `used` is a logical clock, not a time
//!
//! Least-recently-used ordering needs an order, not a date. A wall clock
//! would make the store depend on the host's time, would go backwards when
//! the clock is corrected, and would have to be read from inside a library
//! that is meant to be sans-I/O. So [`Cache`] keeps a counter: every `put`
//! and every hit takes the next number.
//!
//! # Eviction
//!
//! Give the cache a capacity with [`Cache::with_capacity`] and every `put`
//! that pushes the total encoded size past it drops entries in `used`
//! order, oldest first, until it fits. Ties break on the key, so eviction
//! is deterministic.
//!
//! Only [`Cache::with_capacity`] reads the store when it opens it, to
//! recover the counter as one past the largest `used` it finds. A cache
//! opened with [`Cache::new`] never evicts, so it starts counting from
//! one again and keeps the numbers in memory rather than writing them
//! back: a build that only wants hits should not pay to read a store it is
//! not going to prune. Mixing the two is safe — the numbers an uncapped
//! run leaves behind are older than anything a capped run then writes, so
//! they are evicted first.

use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Write as _;

use super::hash::{Hash128, hash128};
use super::inputs::FileInput;
use super::key::{CacheKey, FORMAT};

/// A place a [`Cache`] keeps bytes.
///
/// The contract is a map from [`CacheKey`] to bytes, nothing more: the
/// cache does the framing, the metadata and the eviction, so a backend is
/// as small as a `BTreeMap` or a directory of files. A backend may lose
/// entries at any time (a cache is allowed to forget), but it must never
/// return bytes for a key that were stored under another one.
pub trait Storage {
    /// The bytes stored under `key`, if any.
    fn get(&self, key: CacheKey) -> Option<Vec<u8>>;
    /// Stores `value` under `key`, replacing anything already there.
    fn put(&mut self, key: CacheKey, value: Vec<u8>);
    /// Drops whatever is stored under `key`; not an error if nothing is.
    fn remove(&mut self, key: CacheKey);
    /// Every key the backend currently holds, in ascending key order.
    fn keys(&self) -> Vec<CacheKey>;

    /// How many bytes are stored under `key`.
    ///
    /// The default reads the entry, which is correct but not cheap. A
    /// backend that can answer without reading — a filesystem one, from
    /// the directory entry — should override it: [`Cache::new`] asks for
    /// every key's size when it opens a store, and reading a whole store
    /// to add up its sizes would make opening it cost as much as the
    /// build it is meant to avoid.
    fn size(&self, key: CacheKey) -> Option<u64> {
        self.get(key)
            .map(|value| u64::try_from(value.len()).unwrap_or(u64::MAX))
    }
}

/// A [`Storage`] that keeps everything in memory.
///
/// Useful in tests, in a build that runs once, and as the reference a
/// filesystem backend is checked against.
#[derive(Clone, Debug, Default)]
pub struct MemoryStorage {
    entries: BTreeMap<CacheKey, Vec<u8>>,
}

impl MemoryStorage {
    /// An empty store.
    pub fn new() -> MemoryStorage {
        MemoryStorage::default()
    }

    /// How many entries it holds.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when it holds nothing.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Storage for MemoryStorage {
    fn get(&self, key: CacheKey) -> Option<Vec<u8>> {
        self.entries.get(&key).cloned()
    }

    fn put(&mut self, key: CacheKey, value: Vec<u8>) {
        self.entries.insert(key, value);
    }

    fn remove(&mut self, key: CacheKey) {
        self.entries.remove(&key);
    }

    fn keys(&self) -> Vec<CacheKey> {
        self.entries.keys().copied().collect()
    }

    fn size(&self, key: CacheKey) -> Option<u64> {
        self.entries
            .get(&key)
            .map(|value| u64::try_from(value.len()).unwrap_or(u64::MAX))
    }
}

/// One stored artefact and the metadata around it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The key this entry claims to be filed under.
    pub key: CacheKey,
    /// What produced the artefact, such as `elab:verilog` or `synth`.
    pub producer: String,
    /// A Unix timestamp supplied by the caller, or `0` when unknown.
    pub created: u64,
    /// Position in the least-recently-used order; see the module docs.
    pub used: u64,
    /// The files the work read while producing the artefact, sorted by
    /// path; see [`super::inputs`]. Empty for work that reads no files.
    pub inputs: Vec<FileInput>,
    /// The artefact itself.
    pub content: Vec<u8>,
}

impl Entry {
    /// An entry for `content`, with `used` and `created` left at zero for
    /// [`Cache::put`] to fill in.
    ///
    /// A newline in `producer` would make the header ambiguous, so any
    /// whitespace in it is folded to a single space.
    pub fn new(key: CacheKey, producer: impl AsRef<str>, content: Vec<u8>) -> Entry {
        let producer = producer
            .as_ref()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        Entry {
            key,
            producer,
            created: 0,
            used: 0,
            inputs: Vec::new(),
            content,
        }
    }

    /// The same entry, recording that the work read `inputs`.
    ///
    /// They are sorted by path and repeated paths are dropped, so the
    /// header is the same however the work happened to order its reads.
    pub fn with_inputs(mut self, mut inputs: Vec<FileInput>) -> Entry {
        inputs.sort();
        inputs.dedup_by(|a, b| a.path == b.path);
        self.inputs = inputs;
        self
    }

    /// The artefact's size in bytes.
    pub fn size(&self) -> usize {
        self.content.len()
    }

    /// The artefact as text, when it is valid UTF-8.
    ///
    /// Everything the compiler caches today is `.rtl` text or a scan
    /// summary, so this is the usual way to read an entry back.
    pub fn text(&self) -> Option<&str> {
        std::str::from_utf8(&self.content).ok()
    }

    /// The digest of the content, as stored in the `content` header field.
    pub fn content_hash(&self) -> Hash128 {
        hash128(&self.content)
    }

    /// The entry as the bytes a [`Storage`] holds.
    pub fn encode(&self) -> Vec<u8> {
        let mut header = String::new();
        let _ = writeln!(header, "reticle-cache {FORMAT}");
        let _ = writeln!(header, "key {}", self.key);
        let _ = writeln!(header, "producer {}", self.producer);
        let _ = writeln!(header, "created {}", self.created);
        let _ = writeln!(header, "used {}", self.used);
        let _ = writeln!(header, "size {}", self.content.len());
        let _ = writeln!(header, "content {}", self.content_hash());
        for input in &self.inputs {
            let _ = writeln!(header, "input {input}");
        }
        header.push('\n');
        let mut out = header.into_bytes();
        out.extend_from_slice(&self.content);
        out
    }

    /// Reads back what [`Entry::encode`] wrote.
    ///
    /// # Errors
    ///
    /// Returns an [`EntryError`] for anything that is not exactly the
    /// header this version writes, for a truncated payload, and for a
    /// payload whose digest does not match the header.
    pub fn decode(bytes: &[u8]) -> Result<Entry, EntryError> {
        let split = find_blank_line(bytes).ok_or(EntryError::NoHeader)?;
        let header = std::str::from_utf8(&bytes[..split]).map_err(|_| EntryError::NoHeader)?;
        let content = bytes[split + 2..].to_vec();

        let mut lines = header.lines();
        let magic = lines.next().ok_or(EntryError::NoHeader)?;
        let version = magic
            .strip_prefix("reticle-cache ")
            .ok_or(EntryError::NoHeader)?;
        if version != FORMAT.to_string() {
            return Err(EntryError::Version(version.to_owned()));
        }

        let mut field = |name: &'static str| -> Result<String, EntryError> {
            let line = lines.next().ok_or(EntryError::MissingField(name))?;
            line.strip_prefix(name)
                .and_then(|rest| rest.strip_prefix(' '))
                .map(str::to_owned)
                .ok_or(EntryError::MissingField(name))
        };

        let key = field("key")?;
        let key = CacheKey::from_hex(&key).ok_or(EntryError::BadField("key"))?;
        let producer = field("producer")?;
        let created = field("created")?
            .parse::<u64>()
            .map_err(|_| EntryError::BadField("created"))?;
        let used = field("used")?
            .parse::<u64>()
            .map_err(|_| EntryError::BadField("used"))?;
        let size = field("size")?
            .parse::<usize>()
            .map_err(|_| EntryError::BadField("size"))?;
        let digest = field("content")?;
        let digest = Hash128::from_hex(&digest).ok_or(EntryError::BadField("content"))?;
        let mut inputs = Vec::new();
        for line in lines {
            let input = line.strip_prefix("input ").ok_or(EntryError::NoHeader)?;
            inputs.push(FileInput::decode(input).ok_or(EntryError::BadField("input"))?);
        }

        if size != content.len() {
            return Err(EntryError::SizeMismatch {
                header: size,
                actual: content.len(),
            });
        }
        let entry = Entry {
            key,
            producer,
            created,
            used,
            inputs,
            content,
        };
        if entry.content_hash() != digest {
            return Err(EntryError::ContentMismatch);
        }
        Ok(entry)
    }
}

/// The offset of the `\n\n` that ends the header.
fn find_blank_line(bytes: &[u8]) -> Option<usize> {
    bytes.windows(2).position(|w| w == b"\n\n")
}

/// Why an entry could not be read back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntryError {
    /// The header is missing, not UTF-8, or has a line too many.
    NoHeader,
    /// The header was written by another version of the format.
    Version(String),
    /// A header field is absent or out of order.
    MissingField(&'static str),
    /// A header field is present but unreadable.
    BadField(&'static str),
    /// The payload is not the length the header claims.
    SizeMismatch {
        /// The length the header claims.
        header: usize,
        /// The length the payload actually has.
        actual: usize,
    },
    /// The payload does not hash to the digest in the header.
    ContentMismatch,
}

impl fmt::Display for EntryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EntryError::NoHeader => write!(f, "the entry has no readable header"),
            EntryError::Version(v) => write!(f, "the entry is in format version {v}, not {FORMAT}"),
            EntryError::MissingField(n) => write!(f, "the entry has no `{n}` field"),
            EntryError::BadField(n) => write!(f, "the entry's `{n}` field is unreadable"),
            EntryError::SizeMismatch { header, actual } => {
                write!(f, "the entry claims {header} bytes but holds {actual}")
            }
            EntryError::ContentMismatch => {
                write!(f, "the entry's content does not match its digest")
            }
        }
    }
}

/// One thing [`Cache::verify`] found wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Corruption {
    /// The key the entry was stored under.
    pub key: CacheKey,
    /// What is wrong with it.
    pub problem: Problem,
}

impl fmt::Display for Corruption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.key, self.problem)
    }
}

/// What [`Cache::verify`] found wrong with one entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// The backend listed the key but has nothing under it.
    Missing,
    /// The bytes are not a readable entry.
    Unreadable(EntryError),
    /// The entry is readable but names a different key than the one it is
    /// stored under, which is how a mixed-up or copied store shows itself.
    Misfiled(CacheKey),
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Problem::Missing => write!(f, "the backend lists the key but holds nothing under it"),
            Problem::Unreadable(e) => write!(f, "{e}"),
            Problem::Misfiled(k) => write!(f, "the entry says its key is {k}"),
        }
    }
}

/// What the cache knows about an entry without reading it.
#[derive(Clone, Copy, Debug)]
struct Meta {
    used: u64,
    /// The encoded size, which is what the capacity is measured in.
    encoded: u64,
}

/// A content-addressed store of build artefacts over a [`Storage`].
///
/// The cache borrows its backend, so one backend can serve several caches
/// in turn and the caller keeps ownership of whatever the backend needs
/// (a directory handle, a database connection).
pub struct Cache<'a> {
    storage: &'a mut dyn Storage,
    index: BTreeMap<CacheKey, Meta>,
    clock: u64,
    total: u64,
    capacity: Option<u64>,
    hits: u64,
    misses: u64,
    evicted: u64,
}

impl fmt::Debug for Cache<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The backend is opaque, so the summary is what the cache knows.
        f.debug_struct("Cache")
            .field("entries", &self.index.len())
            .field("bytes", &self.total)
            .field("capacity", &self.capacity)
            .field("hits", &self.hits)
            .field("misses", &self.misses)
            .field("evicted", &self.evicted)
            .finish()
    }
}

impl<'a> Cache<'a> {
    /// Opens `storage` with no capacity limit.
    ///
    /// This lists the backend's keys and asks each one's size, which a
    /// backend can usually answer without reading anything. It does *not*
    /// read the entries: their least-recently-used numbers only matter
    /// when something is going to be evicted, and reading a whole store to
    /// recover them would make opening a cache cost more than the build it
    /// is there to save.
    pub fn new(storage: &'a mut dyn Storage) -> Cache<'a> {
        let mut index = BTreeMap::new();
        let mut total = 0u64;
        for key in storage.keys() {
            let encoded = storage.size(key).unwrap_or(0);
            total = total.saturating_add(encoded);
            index.insert(key, Meta { used: 0, encoded });
        }
        Cache {
            storage,
            index,
            clock: 1,
            total,
            capacity: None,
            hits: 0,
            misses: 0,
            evicted: 0,
        }
    }

    /// Opens `storage` and evicts past `capacity` bytes.
    ///
    /// The capacity counts encoded entries, header included, which is what
    /// a backend actually stores. Unlike [`Cache::new`] this does read
    /// every entry, because eviction needs the least-recently-used order
    /// that only the entries themselves record. Opening an over-full store
    /// does not evict; the next [`Cache::put`] does.
    pub fn with_capacity(storage: &'a mut dyn Storage, capacity: u64) -> Cache<'a> {
        let mut cache = Cache::new(storage);
        cache.capacity = Some(capacity);
        cache.recover_order();
        cache
    }

    /// Reads every entry to recover the least-recently-used order.
    ///
    /// An entry that cannot be read keeps `used = 0`, so it is the first
    /// thing evicted and [`Cache::verify`] still reports it.
    fn recover_order(&mut self) {
        let mut clock = 0;
        for (&key, meta) in &mut self.index {
            if let Some(bytes) = self.storage.get(key)
                && let Ok(entry) = Entry::decode(&bytes)
            {
                meta.used = entry.used;
                clock = clock.max(entry.used);
            }
        }
        self.clock = clock.saturating_add(1);
    }

    /// The capacity in bytes, when one is set.
    pub fn capacity(&self) -> Option<u64> {
        self.capacity
    }

    /// How many entries the cache holds.
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// True when it holds nothing.
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// The total encoded size of every entry.
    pub fn total_bytes(&self) -> u64 {
        self.total
    }

    /// Every key held, in ascending order.
    pub fn keys(&self) -> Vec<CacheKey> {
        self.index.keys().copied().collect()
    }

    /// How many lookups hit.
    pub fn hits(&self) -> u64 {
        self.hits
    }

    /// How many lookups missed.
    pub fn misses(&self) -> u64 {
        self.misses
    }

    /// How many entries have been evicted to stay inside the capacity.
    pub fn evicted(&self) -> u64 {
        self.evicted
    }

    /// Looks `key` up, counting the result as a hit or a miss.
    ///
    /// A hit takes the next number in the least-recently-used order. That
    /// is written back to the backend only when a capacity is set, since
    /// without one nothing reads it and rewriting every entry a build
    /// touches would be pure overhead.
    ///
    /// An entry that is present but unreadable counts as a miss and is
    /// dropped: a corrupt entry must never be served, and leaving it in
    /// place would make every build miss on it forever.
    pub fn get(&mut self, key: CacheKey) -> Option<Entry> {
        self.get_valid(key, |_| true)
    }

    /// Looks `key` up like [`Cache::get`], but counts it a hit only when
    /// `valid` accepts the entry.
    ///
    /// This is how the recorded inputs of an entry
    /// ([`Entry::inputs`], [`super::inputs::still_valid`]) are checked: an
    /// entry that `valid` rejects is counted as a miss and not served. It
    /// is left in the store, since the work that follows a miss writes a
    /// fresh entry under the same key.
    pub fn get_valid(
        &mut self,
        key: CacheKey,
        valid: impl FnOnce(&Entry) -> bool,
    ) -> Option<Entry> {
        let Some(bytes) = self.storage.get(key) else {
            self.misses += 1;
            self.index.remove(&key);
            return None;
        };
        let mut entry = match Entry::decode(&bytes) {
            Ok(entry) if entry.key == key => entry,
            _ => {
                self.misses += 1;
                self.drop_key(key);
                return None;
            }
        };
        if !valid(&entry) {
            self.misses += 1;
            return None;
        }
        self.hits += 1;
        entry.used = self.next_stamp();
        if let Some(meta) = self.index.get_mut(&key) {
            meta.used = entry.used;
        }
        if self.capacity.is_some() {
            let (used, encoded) = (entry.used, entry.encode());
            self.replace_bytes(key, used, encoded);
        }
        Some(entry)
    }

    /// Reads `key` without counting a hit or a miss and without touching
    /// the least-recently-used order.
    ///
    /// This is for inspection — a `--list` or a report — not for a build.
    pub fn peek(&self, key: CacheKey) -> Option<Entry> {
        let bytes = self.storage.get(key)?;
        Entry::decode(&bytes).ok().filter(|e| e.key == key)
    }

    /// Stores `entry` under its own key, evicting if a capacity is set.
    ///
    /// `entry.used` is overwritten with the next number in the
    /// least-recently-used order; `entry.created` is kept as the caller set
    /// it, because the library has no clock.
    pub fn put(&mut self, mut entry: Entry) {
        entry.used = self.next_stamp();
        let (key, used) = (entry.key, entry.used);
        self.replace_bytes(key, used, entry.encode());
        self.enforce_capacity();
    }

    /// Stores `content` under `key`, attributed to `producer`.
    ///
    /// `created` is the Unix timestamp to record, or `0` when the caller
    /// has no clock to read.
    pub fn insert(&mut self, key: CacheKey, producer: &str, created: u64, content: Vec<u8>) {
        let mut entry = Entry::new(key, producer, content);
        entry.created = created;
        self.put(entry);
    }

    /// Drops one entry.
    pub fn remove(&mut self, key: CacheKey) {
        self.drop_key(key);
    }

    /// Drops every entry.
    pub fn clear(&mut self) {
        for key in self.keys() {
            self.drop_key(key);
        }
    }

    /// Re-checks every entry against its own content.
    ///
    /// For each key the backend lists, this reads the bytes back and
    /// checks that the header parses, that the recorded size matches the
    /// payload, that the payload hashes to the recorded digest, and that
    /// the key in the header is the key the entry is filed under. A store
    /// that was truncated, half-written, copied from another machine or
    /// edited by hand fails one of those, so a corrupted store is detected
    /// rather than trusted.
    ///
    /// It cannot re-derive the key from the artefact: a key describes the
    /// *inputs* that produced the artefact, which the artefact does not
    /// contain. That is why the key is written into the entry.
    pub fn verify(&self) -> Vec<Corruption> {
        let mut out = Vec::new();
        for key in self.storage.keys() {
            let Some(bytes) = self.storage.get(key) else {
                out.push(Corruption {
                    key,
                    problem: Problem::Missing,
                });
                continue;
            };
            match Entry::decode(&bytes) {
                Ok(entry) if entry.key == key => {}
                Ok(entry) => out.push(Corruption {
                    key,
                    problem: Problem::Misfiled(entry.key),
                }),
                Err(error) => out.push(Corruption {
                    key,
                    problem: Problem::Unreadable(error),
                }),
            }
        }
        out
    }

    /// [`Cache::verify`], then drops everything it complained about.
    pub fn verify_and_purge(&mut self) -> Vec<Corruption> {
        let bad = self.verify();
        for corruption in &bad {
            self.drop_key(corruption.key);
        }
        bad
    }

    /// Evicts least-recently-used entries until the total is at most
    /// `limit` bytes, and returns how many were dropped.
    ///
    /// Ties in the `used` order break on the key, so the choice is the
    /// same on every run and every platform.
    pub fn evict_to(&mut self, limit: u64) -> usize {
        let mut dropped = 0;
        while self.total > limit {
            let Some((&key, _)) = self
                .index
                .iter()
                .min_by_key(|(key, meta)| (meta.used, **key))
            else {
                break;
            };
            self.drop_key(key);
            self.evicted += 1;
            dropped += 1;
        }
        dropped
    }

    /// The next number in the least-recently-used order.
    fn next_stamp(&mut self) -> u64 {
        let stamp = self.clock;
        self.clock = self.clock.saturating_add(1);
        stamp
    }

    /// Writes `bytes` under `key` and keeps the index and the total right.
    fn replace_bytes(&mut self, key: CacheKey, used: u64, bytes: Vec<u8>) {
        let encoded = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if let Some(old) = self.index.insert(key, Meta { used, encoded }) {
            self.total = self.total.saturating_sub(old.encoded);
        }
        self.total = self.total.saturating_add(encoded);
        self.storage.put(key, bytes);
    }

    /// Removes `key` from the backend and the index.
    fn drop_key(&mut self, key: CacheKey) {
        if let Some(meta) = self.index.remove(&key) {
            self.total = self.total.saturating_sub(meta.encoded);
        }
        self.storage.remove(key);
    }

    /// Evicts down to the configured capacity, if there is one.
    fn enforce_capacity(&mut self) {
        if let Some(limit) = self.capacity {
            self.evict_to(limit);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::hash::Hash128;

    fn key(n: u64) -> CacheKey {
        CacheKey::from_hash(Hash128 { hi: 0, lo: n })
    }

    #[test]
    fn an_entry_round_trips() {
        let mut entry = Entry::new(key(1), "elab:verilog", b"module m\nend\n".to_vec());
        entry.created = 1_758_499_200;
        entry.used = 7;
        let bytes = entry.encode();
        assert_eq!(Entry::decode(&bytes), Ok(entry.clone()));
        // The header is text, and the artefact follows it verbatim.
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.starts_with("reticle-cache 2\nkey "));
        assert!(text.ends_with("\n\nmodule m\nend\n"));
        assert_eq!(entry.text(), Some("module m\nend\n"));
    }

    #[test]
    fn recorded_inputs_round_trip_sorted_in_the_header() {
        use crate::cache::inputs::FileInput;
        let entry = Entry::new(key(1), "synth", b"module m\nend\n".to_vec()).with_inputs(vec![
            FileInput {
                path: "b.hex".to_owned(),
                digest: None,
            },
            FileInput {
                path: "a.hex".to_owned(),
                digest: Some(hash128(b"00\n")),
            },
            FileInput {
                path: "a.hex".to_owned(),
                digest: Some(hash128(b"00\n")),
            },
        ]);
        assert_eq!(entry.inputs.len(), 2, "a repeated path is kept once");
        let bytes = entry.encode();
        assert_eq!(Entry::decode(&bytes), Ok(entry.clone()));
        let text = String::from_utf8(bytes).unwrap();
        let header = text.split("\n\n").next().unwrap();
        let inputs: Vec<&str> = header.lines().filter(|l| l.starts_with("input ")).collect();
        assert_eq!(inputs.len(), 2);
        assert!(inputs[0].ends_with(" a.hex"), "{header}");
        assert_eq!(inputs[1], "input missing b.hex");

        // A damaged input line is an unreadable entry, not a lost input.
        let bad = text.replacen("input missing", "input nonsense", 1);
        assert_eq!(
            Entry::decode(bad.as_bytes()),
            Err(EntryError::BadField("input"))
        );
    }

    #[test]
    fn a_rejected_entry_is_a_miss_and_stays_in_the_store() {
        let mut storage = MemoryStorage::new();
        let mut cache = Cache::new(&mut storage);
        cache.insert(key(1), "synth", 0, b"x".to_vec());
        assert!(cache.get_valid(key(1), |_| false).is_none());
        assert_eq!((cache.hits(), cache.misses()), (0, 1));
        assert!(cache.peek(key(1)).is_some());
        assert!(cache.get_valid(key(1), |e| e.content == b"x").is_some());
        assert_eq!((cache.hits(), cache.misses()), (1, 1));
    }

    #[test]
    fn an_empty_artefact_round_trips() {
        let entry = Entry::new(key(1), "elab:verilog", Vec::new());
        assert_eq!(Entry::decode(&entry.encode()), Ok(entry));
    }

    #[test]
    fn a_producer_with_newlines_is_flattened() {
        let entry = Entry::new(key(1), "elab\nverilog", b"x".to_vec());
        assert_eq!(entry.producer, "elab verilog");
        assert_eq!(Entry::decode(&entry.encode()), Ok(entry));
    }

    #[test]
    fn decoding_rejects_damage() {
        let entry = Entry::new(key(1), "elab", b"module m\nend\n".to_vec());
        let good = entry.encode();

        assert_eq!(Entry::decode(b"nonsense"), Err(EntryError::NoHeader));
        assert_eq!(Entry::decode(b"\n\n"), Err(EntryError::NoHeader));

        // A flipped byte in the payload.
        let mut flipped = good.clone();
        let last = flipped.len() - 2;
        flipped[last] ^= 0x20;
        assert_eq!(Entry::decode(&flipped), Err(EntryError::ContentMismatch));

        // A truncated payload.
        let truncated = good[..good.len() - 3].to_vec();
        assert!(matches!(
            Entry::decode(&truncated),
            Err(EntryError::SizeMismatch { .. })
        ));

        // A header from another format version.
        let other = String::from_utf8(good.clone()).unwrap().replacen(
            "reticle-cache 2",
            "reticle-cache 99",
            1,
        );
        assert_eq!(
            Entry::decode(other.as_bytes()),
            Err(EntryError::Version("99".to_owned()))
        );

        // A field that is gone.
        let missing = String::from_utf8(good.clone())
            .unwrap()
            .lines()
            .filter(|l| !l.starts_with("used "))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n\n";
        assert!(Entry::decode(missing.as_bytes()).is_err());

        // A field that is there but unreadable.
        let bad = String::from_utf8(good)
            .unwrap()
            .replacen("used 0", "used x", 1);
        assert_eq!(
            Entry::decode(bad.as_bytes()),
            Err(EntryError::BadField("used"))
        );
    }

    #[test]
    fn put_then_get_round_trips_through_the_backend() {
        let mut storage = MemoryStorage::new();
        let mut cache = Cache::new(&mut storage);
        cache.insert(key(1), "elab", 42, b"hello".to_vec());
        let got = cache.get(key(1)).expect("a hit");
        assert_eq!(got.content, b"hello");
        assert_eq!(got.producer, "elab");
        assert_eq!(got.created, 42);
        assert_eq!(cache.hits(), 1);
        assert_eq!(cache.misses(), 0);
        assert!(cache.get(key(2)).is_none());
        assert_eq!(cache.misses(), 1);
    }

    #[test]
    fn the_logical_clock_survives_reopening() {
        let mut storage = MemoryStorage::new();
        {
            let mut cache = Cache::new(&mut storage);
            cache.insert(key(1), "elab", 0, b"a".to_vec());
            cache.insert(key(2), "elab", 0, b"b".to_vec());
        }
        // A capped cache reads the store, so it keeps counting where the
        // last run left off and the order survives the restart.
        let mut cache = Cache::with_capacity(&mut storage, u64::MAX);
        cache.insert(key(3), "elab", 0, b"c".to_vec());
        let third = cache.peek(key(3)).unwrap();
        let second = cache.peek(key(2)).unwrap();
        assert!(
            third.used > second.used,
            "a reopened cache must keep counting up, got {} after {}",
            third.used,
            second.used
        );
    }

    #[test]
    fn an_uncapped_cache_does_not_read_the_store_to_open_it() {
        // It still knows what is there and how big it is; it just has not
        // looked at the least-recently-used numbers.
        let mut storage = MemoryStorage::new();
        {
            let mut cache = Cache::new(&mut storage);
            for n in 1..=3 {
                cache.insert(key(n), "elab", 0, vec![b'x'; 40]);
            }
        }
        let cache = Cache::new(&mut storage);
        assert_eq!(cache.len(), 3);
        assert_eq!(cache.keys(), vec![key(1), key(2), key(3)]);
        let expected: u64 = storage
            .keys()
            .iter()
            .filter_map(|k| storage.get(*k))
            .map(|v| u64::try_from(v.len()).unwrap())
            .sum();
        let cache = Cache::new(&mut storage);
        assert_eq!(cache.total_bytes(), expected);
    }

    #[test]
    fn eviction_drops_the_least_recently_used_first() {
        let mut storage = MemoryStorage::new();
        // Each entry encodes to the same size, so the capacity is a count.
        let one = Entry::new(key(0), "elab", vec![b'x'; 64]).encode().len();
        let capacity = u64::try_from(one * 3).unwrap();
        let mut cache = Cache::with_capacity(&mut storage, capacity);

        for n in 1..=3 {
            cache.insert(key(n), "elab", 0, vec![b'x'; 64]);
        }
        assert_eq!(cache.len(), 3);

        // Touch 1, so 2 becomes the oldest.
        assert!(cache.get(key(1)).is_some());
        cache.insert(key(4), "elab", 0, vec![b'x'; 64]);

        assert_eq!(cache.len(), 3);
        assert_eq!(cache.evicted(), 1);
        assert!(
            cache.peek(key(2)).is_none(),
            "2 was the least recently used"
        );
        assert!(cache.peek(key(1)).is_some());
        assert!(cache.peek(key(3)).is_some());
        assert!(cache.peek(key(4)).is_some());
        assert!(cache.total_bytes() <= capacity);
    }

    #[test]
    fn eviction_order_is_deterministic_on_ties() {
        // Two entries share a `used` value only if a store was written by
        // something that does not bump the clock, but the order still has
        // to be the same on every run: the tie breaks on the key, so the
        // smaller key goes first.
        let mut storage = MemoryStorage::new();
        for k in [key(9), key(1)] {
            let mut entry = Entry::new(k, "elab", vec![b'x'; 16]);
            entry.used = 5;
            storage.put(k, entry.encode());
        }
        let one = storage.get(key(1)).unwrap().len();
        let mut cache = Cache::new(&mut storage);
        assert_eq!(cache.evict_to(u64::try_from(one).unwrap()), 1);
        assert!(cache.peek(key(1)).is_none());
        assert!(cache.peek(key(9)).is_some());
    }

    #[test]
    fn eviction_stops_at_the_limit_and_keeps_the_total_right() {
        let mut storage = MemoryStorage::new();
        let mut cache = Cache::new(&mut storage);
        for n in 1..=10 {
            cache.insert(key(n), "elab", 0, vec![b'x'; 32]);
        }
        let all = cache.total_bytes();
        let dropped = cache.evict_to(all / 2);
        assert!(dropped > 0 && dropped < 10);
        assert!(cache.total_bytes() <= all / 2);
        // The total the cache reports matches what the backend holds.
        let actual: u64 = storage
            .keys()
            .iter()
            .filter_map(|k| storage.get(*k))
            .map(|v| u64::try_from(v.len()).unwrap())
            .sum();
        let cache = Cache::new(&mut storage);
        assert_eq!(cache.total_bytes(), actual);
    }

    #[test]
    fn verify_is_quiet_on_a_healthy_store() {
        let mut storage = MemoryStorage::new();
        let mut cache = Cache::new(&mut storage);
        for n in 1..=5 {
            cache.insert(
                key(n),
                "elab",
                0,
                format!("module m{n}\nend\n").into_bytes(),
            );
        }
        assert_eq!(cache.verify(), Vec::new());
    }

    #[test]
    fn verify_catches_a_corrupted_payload() {
        let mut storage = MemoryStorage::new();
        {
            let mut cache = Cache::new(&mut storage);
            cache.insert(key(1), "elab", 0, b"module m\nend\n".to_vec());
        }
        // Corrupt the artefact behind the cache's back, the way a half
        // written file or a bad disk would.
        let mut bytes = storage.get(key(1)).unwrap();
        let last = bytes.len() - 2;
        bytes[last] = b'!';
        storage.put(key(1), bytes);

        let mut cache = Cache::new(&mut storage);
        let bad = cache.verify();
        assert_eq!(bad.len(), 1);
        assert_eq!(bad[0].key, key(1));
        assert!(matches!(
            bad[0].problem,
            Problem::Unreadable(EntryError::ContentMismatch)
        ));
        assert!(bad[0].to_string().contains("does not match its digest"));

        // A corrupt entry is never served, and the lookup drops it.
        assert!(cache.get(key(1)).is_none());
        assert_eq!(cache.misses(), 1);
        assert!(cache.verify().is_empty());
    }

    #[test]
    fn verify_catches_a_misfiled_entry() {
        let mut storage = MemoryStorage::new();
        {
            let mut cache = Cache::new(&mut storage);
            cache.insert(key(1), "elab", 0, b"module m\nend\n".to_vec());
        }
        // Copy the bytes to another key, as a careless `cp` would.
        let bytes = storage.get(key(1)).unwrap();
        storage.put(key(2), bytes);

        let mut cache = Cache::new(&mut storage);
        let bad = cache.verify();
        assert_eq!(bad.len(), 1);
        assert_eq!(bad[0].key, key(2));
        assert_eq!(bad[0].problem, Problem::Misfiled(key(1)));
        assert!(
            cache.get(key(2)).is_none(),
            "a misfiled entry is not served"
        );
        assert_eq!(cache.verify_and_purge(), Vec::new());
    }

    #[test]
    fn verify_and_purge_leaves_a_clean_store() {
        let mut storage = MemoryStorage::new();
        {
            let mut cache = Cache::new(&mut storage);
            cache.insert(key(1), "elab", 0, b"good\n".to_vec());
        }
        storage.put(key(7), b"not an entry at all".to_vec());
        let mut cache = Cache::new(&mut storage);
        assert_eq!(cache.verify_and_purge().len(), 1);
        assert_eq!(cache.verify(), Vec::new());
        assert_eq!(cache.len(), 1);
        assert!(cache.get(key(1)).is_some());
    }

    #[test]
    fn clearing_empties_the_backend() {
        let mut storage = MemoryStorage::new();
        let mut cache = Cache::new(&mut storage);
        for n in 1..=4 {
            cache.insert(key(n), "elab", 0, b"x".to_vec());
        }
        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.total_bytes(), 0);
        assert!(storage.is_empty());
    }

    #[test]
    fn memory_storage_lists_keys_in_order() {
        let mut storage = MemoryStorage::new();
        storage.put(key(3), b"c".to_vec());
        storage.put(key(1), b"a".to_vec());
        storage.put(key(2), b"b".to_vec());
        assert_eq!(storage.keys(), vec![key(1), key(2), key(3)]);
        storage.remove(key(2));
        assert_eq!(storage.keys(), vec![key(1), key(3)]);
        assert_eq!(storage.len(), 2);
    }
}
