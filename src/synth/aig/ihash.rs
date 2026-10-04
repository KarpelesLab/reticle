//! A fast hasher for the small integer keys the AIG passes hash.
//!
//! Every inner loop here keys a map or a set by something tiny and dense: a
//! node index, a 16-bit truth table, a pair of edges. The standard library's
//! default hasher is SipHash-1-3 with a per-map random key, which is the
//! right default for a map whose keys might come from outside the program
//! and the wrong one for these: it hashes a four-byte key through several
//! rounds of a block cipher and builds a fresh keyed hasher for every
//! lookup.
//!
//! On the technology mapper that cost **half of the running time**. Sampling
//! `reticle synth --lut 4` over an 11-file design (3168 `LUT4`) put 50.7% of
//! the samples inside hashing, spread over
//! [`cone_nodes`](super::cut::cone_nodes) (13.3%),
//! [`cone_truth`](super::cut::cone_truth) (10.7%), the rewriting library's
//! memo (8.4%), the structural hash table (4.9%), the MFFC counter's memo
//! (6.2%) and cut enumeration (4.4%). None of those keys is adversarial and
//! none of those maps is ever iterated, so the collision resistance buys
//! nothing.
//!
//! [`IntHasher`] replaces it with one multiply-rotate step per written word
//! and a final avalanche, in the manner of Firefox's `FxHash`. The final mix
//! matters: `hashbrown` takes the bucket index from the *low* bits of the
//! hash and its control byte from the *high* seven, and a bare multiplicative
//! hash leaves the high bits of a small key almost constant, which would make
//! every probe collide on the control byte.
//!
//! # What this does not change
//!
//! Nothing about what the passes compute. A hasher changes which bucket a key
//! lands in, so it changes the order a map *iterates* — and that is why the
//! aliases below are used only where the map is a lookup table and never a
//! sequence. Every site that iterates a map keeps the standard hasher.

use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};

/// The odd multiplier of the mixing step, from the 64-bit constant
/// `FxHash` uses (itself the fractional part of the golden ratio scaled to
/// 64 bits).
const MULTIPLY: u64 = 0x517c_c1b7_2722_0a95;

/// A hasher for dense integer keys: one rotate-xor-multiply per word.
///
/// It is not collision resistant and must not be used on anything a
/// caller outside the compiler chooses. See the module documentation.
#[derive(Clone, Copy, Debug, Default)]
pub struct IntHasher {
    state: u64,
}

impl IntHasher {
    /// Folds one 64-bit word into the state.
    #[inline]
    fn add(&mut self, word: u64) {
        self.state = (self.state.rotate_left(5) ^ word).wrapping_mul(MULTIPLY);
    }
}

impl Hasher for IntHasher {
    #[inline]
    fn finish(&self) -> u64 {
        // `hashbrown` reads the bucket from the low bits and the control
        // byte from the top seven, so the state has to avalanche into both
        // halves before it is handed over.
        let mut h = self.state;
        h ^= h >> 32;
        h = h.wrapping_mul(MULTIPLY);
        h ^= h >> 29;
        h
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let (chunks, rest) = bytes.as_chunks::<8>();
        for chunk in chunks {
            self.add(u64::from_le_bytes(*chunk));
        }
        if !rest.is_empty() {
            let mut buf = [0u8; 8];
            buf[..rest.len()].copy_from_slice(rest);
            self.add(u64::from_le_bytes(buf));
        }
    }

    #[inline]
    fn write_u8(&mut self, value: u8) {
        self.add(u64::from(value));
    }

    #[inline]
    fn write_u16(&mut self, value: u16) {
        self.add(u64::from(value));
    }

    #[inline]
    fn write_u32(&mut self, value: u32) {
        self.add(u64::from(value));
    }

    #[inline]
    fn write_u64(&mut self, value: u64) {
        self.add(value);
    }

    #[inline]
    fn write_usize(&mut self, value: usize) {
        self.add(value as u64);
    }
}

/// The [`std::hash::BuildHasher`] of [`IntHasher`].
pub type BuildIntHasher = BuildHasherDefault<IntHasher>;

/// A [`HashMap`] over integer keys, hashed by [`IntHasher`].
pub type IntMap<K, V> = HashMap<K, V, BuildIntHasher>;

/// A [`HashSet`] over integer keys, hashed by [`IntHasher`].
pub type IntSet<K> = HashSet<K, BuildIntHasher>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::hash::{BuildHasher, Hash};

    /// Hashing the same key twice gives the same answer, and two keys that
    /// differ in one bit do not land on the same bucket *or* the same
    /// control byte.
    ///
    /// The second half is the property a bare multiplicative hash lacks and
    /// the reason [`IntHasher::finish`] avalanches: `hashbrown` reads the
    /// bucket from the low bits and the control byte from the high seven, so
    /// a hash whose high bits barely move turns every table into one long
    /// probe. This does not test the quality of the hash, which is not the
    /// kind of claim a test makes; it tests that both halves of the word
    /// respond to the key at all.
    #[test]
    fn small_keys_reach_both_halves_of_the_hash() {
        let build = BuildIntHasher::default();
        let of = |v: u32| build.hash_one(v);
        assert_eq!(of(1234), of(1234));
        let mut low = std::collections::HashSet::new();
        let mut high = std::collections::HashSet::new();
        for i in 0..64u32 {
            let h = of(i);
            low.insert(h & 0x7f);
            high.insert(h >> 57);
        }
        // 64 consecutive keys over 128 buckets: an ideal hash would leave
        // about 50 distinct values at each end (128 * (1 - (127/128)^64)),
        // so the bar is well under that and only fails for a hash whose
        // high or low bits barely move. A multiplicative hash with no final
        // mix scores 1 on the high byte.
        assert!(low.len() > 40, "only {} distinct low bytes", low.len());
        assert!(high.len() > 40, "only {} distinct high bytes", high.len());
    }

    /// The aliases work as maps and sets, including for the tuple and
    /// `u16` keys the passes use.
    #[test]
    fn the_aliases_behave_like_the_standard_containers() {
        let mut m: IntMap<u32, &str> = IntMap::default();
        for i in 0..1000u32 {
            m.insert(i, "x");
        }
        assert_eq!(m.len(), 1000);
        assert_eq!(m.get(&999), Some(&"x"));
        assert_eq!(m.get(&1000), None);

        let mut s: IntSet<(u16, u16)> = IntSet::default();
        assert!(s.insert((1, 2)));
        assert!(!s.insert((1, 2)));
        assert!(s.contains(&(1, 2)));
        assert!(!s.contains(&(2, 1)));
    }

    /// A byte string reaches the hasher through `write`, which the derived
    /// `Hash` of a composite key also uses; it must not panic on a length
    /// that is not a multiple of eight.
    #[test]
    fn write_handles_a_partial_word() {
        let build = BuildIntHasher::default();
        let mut seen = std::collections::HashSet::new();
        for n in 0..24usize {
            let bytes: Vec<u8> = (0..n).map(|i| u8::try_from(i).expect("small")).collect();
            let mut h = IntHasher::default();
            bytes.as_slice().hash(&mut h);
            seen.insert(h.finish());
        }
        assert_eq!(seen.len(), 24, "two lengths collided");
        let _ = build.hash_one([1u8, 2, 3].as_slice());
    }
}
