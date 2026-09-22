//! Identifier interning.
//!
//! HDL designs repeat the same names tens of thousands of times: every
//! reference to a signal, module, port or parameter is spelled out in full,
//! and elaboration multiplies that by the instance count. An [`Interner`]
//! maps each distinct string to a small [`Symbol`] once, so AST and IR
//! objects carry a `u32` per name and compare names by integer equality.
//!
//! # Storage
//!
//! Every interned string is appended to one arena `String`; a symbol is an
//! index into a table of `(start, end)` byte ranges into that arena, so each
//! string is stored exactly once. Lookup goes through a small open-addressing
//! hash table whose slots hold symbol indices; the 64-bit hash of every
//! string is kept alongside its range so a probe compares text only after a
//! hash match. No `HashMap<String, _>` is involved, which keeps the memory
//! per identifier at its byte length plus 20 bytes of bookkeeping.
//!
//! # Case
//!
//! Verilog identifiers are case-sensitive; VHDL basic identifiers are not.
//! Both frontends share one interner: Verilog calls [`Interner::intern`] with
//! the spelling as written, VHDL calls [`Interner::intern_ci`], which folds
//! to lowercase first, so `Clk`, `CLK` and `clk` in VHDL resolve to the same
//! symbol while the same three spellings in Verilog stay distinct. VHDL
//! extended identifiers (`\Clk\`) are case-sensitive and go through
//! [`Interner::intern`] with their backslashes kept, so they can never
//! collide with a basic identifier. A VHDL entity name therefore matches a
//! Verilog module name only when the Verilog spelling is lowercase, which is
//! the rule mixed-language tools generally apply.
//!
//! # Determinism
//!
//! Symbols are dense and assigned in insertion order, so a `Vec` indexed by
//! [`Symbol::index`] is a cheap side table, and a given sequence of `intern`
//! calls always yields the same numbering. Nothing here iterates the hash
//! table in table order.

use std::fmt;
use std::hash::{DefaultHasher, Hasher};
use std::ops::Index;

/// An interned string: a dense `u32` handle that is cheap to copy, compare
/// and hash, and resolves back to text through the [`Interner`] it came from.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Symbol(u32);

impl Symbol {
    /// The raw handle.
    pub fn as_u32(self) -> u32 {
        self.0
    }

    /// Rebuilds a symbol from a handle obtained with [`Symbol::as_u32`].
    ///
    /// The handle is only meaningful for the interner that produced it; the
    /// caller is responsible for keeping the two together.
    pub fn from_u32(raw: u32) -> Self {
        Symbol(raw)
    }

    /// The handle as a `usize`, for indexing side tables.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Debug for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Symbol({})", self.0)
    }
}

/// Marker for an unused hash table slot. Symbol handles never reach this
/// value: [`Interner::intern`] refuses to allocate it.
const EMPTY: u32 = u32::MAX;

/// Hashes a string for table lookup.
///
/// `DefaultHasher::new()` uses fixed keys, so the hash of a given string is
/// stable within a build; nothing depends on it being stable across builds.
fn hash_str(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    h.write(s.as_bytes());
    h.finish()
}

/// Reduces a hash to a slot index in a table of `len` slots, where `len` is
/// a power of two.
fn slot_of(hash: u64, len: usize) -> usize {
    debug_assert!(len.is_power_of_two());
    let mask = u64::try_from(len - 1).expect("table length fits u64");
    usize::try_from(hash & mask).expect("masked hash is below the table length")
}

/// Narrows an arena offset to the `u32` stored in a range.
fn offset(n: usize) -> u32 {
    u32::try_from(n).expect("interner arena exceeds 4 GiB")
}

/// Places a symbol index in the first free slot of its probe sequence.
fn place(slots: &mut [u32], hash: u64, idx: u32) {
    let len = slots.len();
    let mut i = slot_of(hash, len);
    while slots[i] != EMPTY {
        i = (i + 1) & (len - 1);
    }
    slots[i] = idx;
}

/// Maps strings to [`Symbol`]s and back. See the module docs for the
/// storage layout and the case-folding contract.
#[derive(Clone, Debug, Default)]
pub struct Interner {
    /// All interned text, back to back.
    arena: String,
    /// `(start, end)` byte range in `arena` for each symbol, by handle.
    ranges: Vec<(u32, u32)>,
    /// Hash of each symbol's text, by handle.
    hashes: Vec<u64>,
    /// Open-addressing table of symbol handles, `EMPTY` for free slots.
    /// Its length is zero or a power of two, and the load factor is kept
    /// at or below one half.
    slots: Vec<u32>,
}

impl Interner {
    /// Creates an empty interner.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an interner with room for `symbols` strings totalling about
    /// `bytes` bytes before it needs to reallocate.
    pub fn with_capacity(symbols: usize, bytes: usize) -> Self {
        let slots = (symbols * 2).next_power_of_two().max(16);
        Interner {
            arena: String::with_capacity(bytes),
            ranges: Vec::with_capacity(symbols),
            hashes: Vec::with_capacity(symbols),
            slots: vec![EMPTY; slots],
        }
    }

    /// Number of distinct strings interned so far.
    pub fn len(&self) -> usize {
        self.ranges.len()
    }

    /// True when nothing has been interned.
    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    /// The text of the symbol with handle `idx`.
    fn text_at(&self, idx: usize) -> &str {
        let (start, end) = self.ranges[idx];
        &self.arena[start as usize..end as usize]
    }

    /// Looks a string up without inserting it.
    pub fn get(&self, s: &str) -> Option<Symbol> {
        if self.slots.is_empty() {
            return None;
        }
        let hash = hash_str(s);
        self.lookup(s, hash)
    }

    /// Looks a string up by its lowercase folding, the way [`intern_ci`]
    /// would have interned it.
    ///
    /// [`intern_ci`]: Interner::intern_ci
    pub fn get_ci(&self, s: &str) -> Option<Symbol> {
        if is_folded(s) {
            self.get(s)
        } else {
            self.get(&s.to_lowercase())
        }
    }

    /// Probes the table for `s`, whose hash is `hash`.
    fn lookup(&self, s: &str, hash: u64) -> Option<Symbol> {
        let len = self.slots.len();
        let mut i = slot_of(hash, len);
        loop {
            let idx = self.slots[i];
            if idx == EMPTY {
                return None;
            }
            let at = idx as usize;
            if self.hashes[at] == hash && self.text_at(at) == s {
                return Some(Symbol(idx));
            }
            i = (i + 1) & (len - 1);
        }
    }

    /// Interns a string exactly as spelled and returns its symbol.
    ///
    /// Interning the same text twice returns the same symbol. Use this for
    /// Verilog identifiers and VHDL extended identifiers; VHDL basic
    /// identifiers go through [`intern_ci`].
    ///
    /// # Panics
    ///
    /// Panics if the arena would exceed 4 GiB or the symbol count would
    /// exceed `u32::MAX - 1`, which no real design approaches.
    ///
    /// [`intern_ci`]: Interner::intern_ci
    pub fn intern(&mut self, s: &str) -> Symbol {
        let hash = hash_str(s);
        if !self.slots.is_empty()
            && let Some(sym) = self.lookup(s, hash)
        {
            return sym;
        }
        if (self.ranges.len() + 1) * 2 > self.slots.len() {
            self.grow();
        }
        let idx = u32::try_from(self.ranges.len())
            .ok()
            .filter(|&i| i != EMPTY)
            .expect("interner holds too many symbols");
        let start = offset(self.arena.len());
        self.arena.push_str(s);
        let end = offset(self.arena.len());
        self.ranges.push((start, end));
        self.hashes.push(hash);
        place(&mut self.slots, hash, idx);
        Symbol(idx)
    }

    /// Interns a string case-insensitively: the text is folded to lowercase
    /// first, so `Clk` and `clk` yield the same symbol, and
    /// [`resolve`] returns the lowercase spelling.
    ///
    /// This is the entry point for VHDL basic identifiers. Folding uses
    /// Unicode lowercasing so the VHDL-2008 Latin-1 letters fold as the
    /// standard prescribes; pure-ASCII lowercase input takes a fast path
    /// that allocates nothing.
    ///
    /// [`resolve`]: Interner::resolve
    pub fn intern_ci(&mut self, s: &str) -> Symbol {
        if is_folded(s) {
            self.intern(s)
        } else {
            self.intern(&s.to_lowercase())
        }
    }

    /// The text of a symbol.
    ///
    /// # Panics
    ///
    /// Panics if `sym` did not come from this interner.
    pub fn resolve(&self, sym: Symbol) -> &str {
        self.text_at(sym.index())
    }

    /// Iterates over every symbol and its text in handle order.
    pub fn iter(&self) -> impl Iterator<Item = (Symbol, &str)> {
        (0..self.ranges.len()).map(|i| (Symbol(offset(i)), self.text_at(i)))
    }

    /// Doubles the hash table (or creates it) and re-places every symbol.
    fn grow(&mut self) {
        let len = (self.slots.len() * 2).max(16);
        let mut slots = vec![EMPTY; len];
        for (i, &hash) in self.hashes.iter().enumerate() {
            place(&mut slots, hash, offset(i));
        }
        self.slots = slots;
    }
}

impl Index<Symbol> for Interner {
    type Output = str;

    fn index(&self, sym: Symbol) -> &str {
        self.resolve(sym)
    }
}

/// True when lowercasing `s` would return it unchanged and it is ASCII, so
/// it can be interned without allocating a folded copy.
fn is_folded(s: &str) -> bool {
    s.bytes().all(|b| b.is_ascii() && !b.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interns_and_resolves() {
        let mut i = Interner::new();
        assert!(i.is_empty());
        let clk = i.intern("clk");
        let rst = i.intern("rst_n");
        assert_ne!(clk, rst);
        assert_eq!(i.intern("clk"), clk);
        assert_eq!(i.len(), 2);
        assert_eq!(i.resolve(clk), "clk");
        assert_eq!(&i[rst], "rst_n");
        assert_eq!(i.get("clk"), Some(clk));
        assert_eq!(i.get("CLK"), None);
        assert_eq!(i.get("data"), None);
        assert_eq!(clk.as_u32(), 0);
        assert_eq!(rst.index(), 1);
        assert_eq!(Symbol::from_u32(1), rst);
        assert_eq!(format!("{clk:?}"), "Symbol(0)");
    }

    #[test]
    fn empty_interner_lookups_miss() {
        let i = Interner::new();
        assert_eq!(i.get(""), None);
        assert_eq!(i.get_ci("X"), None);
    }

    #[test]
    fn case_insensitive_shares_symbols() {
        let mut i = Interner::new();
        let a = i.intern_ci("Clk");
        let b = i.intern_ci("CLK");
        let c = i.intern_ci("clk");
        assert_eq!(a, b);
        assert_eq!(b, c);
        assert_eq!(i.resolve(a), "clk");
        assert_eq!(i.get_ci("cLk"), Some(a));
        // The case-sensitive spelling is a different symbol.
        let d = i.intern("Clk");
        assert_ne!(a, d);
        assert_eq!(i.resolve(d), "Clk");
        assert_eq!(i.len(), 2);
        // Verilog `clk` (lowercase) meets VHDL `CLK`.
        assert_eq!(i.intern("clk"), a);
    }

    #[test]
    fn folds_non_ascii() {
        let mut i = Interner::new();
        let a = i.intern_ci("Éve");
        let b = i.intern_ci("éve");
        assert_eq!(a, b);
        assert_eq!(i.resolve(a), "éve");
    }

    #[test]
    fn empty_and_similar_strings_are_distinct() {
        let mut i = Interner::new();
        let e = i.intern("");
        let a = i.intern("a");
        let aa = i.intern("aa");
        assert_eq!(i.resolve(e), "");
        assert_ne!(e, a);
        assert_ne!(a, aa);
        assert_eq!(i.intern(""), e);
        assert_eq!(i.intern("a"), a);
    }

    #[test]
    fn survives_growth() {
        let mut i = Interner::with_capacity(4, 16);
        let names: Vec<String> = (0..10_000).map(|n| format!("net_{n}")).collect();
        let syms: Vec<Symbol> = names.iter().map(|n| i.intern(n)).collect();
        assert_eq!(i.len(), names.len());
        for (n, (name, sym)) in names.iter().zip(&syms).enumerate() {
            assert_eq!(sym.index(), n);
            assert_eq!(i.resolve(*sym), name);
            assert_eq!(i.get(name), Some(*sym));
            assert_eq!(i.intern(name), *sym);
        }
        let collected: Vec<(Symbol, &str)> = i.iter().collect();
        assert_eq!(collected.len(), names.len());
        assert_eq!(collected[42], (syms[42], names[42].as_str()));
    }

    #[test]
    fn numbering_is_deterministic() {
        let run = || {
            let mut i = Interner::new();
            ["b", "a", "c", "a", "b"]
                .iter()
                .map(|s| i.intern(s).as_u32())
                .collect::<Vec<_>>()
        };
        assert_eq!(run(), vec![0, 1, 2, 1, 0]);
        assert_eq!(run(), run());
    }
}
