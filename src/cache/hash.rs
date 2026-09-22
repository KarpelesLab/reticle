//! The 128-bit hash the cache keys are built from.
//!
//! # The choice
//!
//! The crate ships no dependencies, so the hash is written here. It is
//! **xxHash64** (Yann Collet's algorithm, as published), run as two
//! independent lanes with different seeds and concatenated into 128 bits.
//! xxHash64 was chosen over FNV-1a because FNV's avalanche is poor on the
//! short, highly similar byte strings a build key is made of — two source
//! files that differ in one character — and because xxHash64 has published
//! test vectors, which the tests below check this implementation against.
//!
//! Two seeded lanes rather than one wider primitive keeps the code to one
//! well-specified algorithm. The lanes see the same bytes, so this is not a
//! proof of 128-bit strength in the cryptographic sense; it is a 64-bit
//! hash twice, which for a build cache is what matters: the chance that two
//! different unit-of-work descriptions collide in *both* lanes is
//! negligible for any realistic number of entries.
//!
//! # Not a security boundary
//!
//! This is a non-cryptographic hash. Anyone who can choose the input can
//! construct a collision. A cache key says "these are the same inputs", not
//! "this artefact is authentic"; a store an attacker can write to is a
//! compromised store no matter what hash guards it. Do not use
//! [`Hash128`] to authenticate anything.
//!
//! # Determinism
//!
//! The result depends only on the byte sequence written. Every integer is
//! folded in little-endian order and every length is widened through
//! [`u64::try_from`], so a 32-bit host, a 64-bit host and WebAssembly all
//! produce the same digest.

use std::fmt;

/// A 128-bit digest, printed as 32 lowercase hexadecimal digits.
///
/// The two halves are the two xxHash64 lanes; `hi` prints first so the
/// text form sorts the same way the pair compares.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hash128 {
    /// The high lane (seeded with [`SEED_HI`]).
    pub hi: u64,
    /// The low lane (seeded with [`SEED_LO`]).
    pub lo: u64,
}

/// Seed of the lane that becomes [`Hash128::hi`].
///
/// An arbitrary constant; it only has to differ from [`SEED_LO`].
pub const SEED_HI: u64 = 0x5265_7469_636c_6521; // "Reticle!"
/// Seed of the lane that becomes [`Hash128::lo`].
pub const SEED_LO: u64 = 0x9e37_79b9_7f4a_7c15;

impl Hash128 {
    /// The digest as 32 lowercase hexadecimal digits.
    pub fn to_hex(self) -> String {
        format!("{:016x}{:016x}", self.hi, self.lo)
    }

    /// Parses the form [`Hash128::to_hex`] writes.
    ///
    /// Returns `None` for anything that is not exactly 32 hexadecimal
    /// digits, uppercase included.
    pub fn from_hex(text: &str) -> Option<Hash128> {
        if text.len() != 32 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let hi = u64::from_str_radix(&text[..16], 16).ok()?;
        let lo = u64::from_str_radix(&text[16..], 16).ok()?;
        Some(Hash128 { hi, lo })
    }
}

impl fmt::Display for Hash128 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}{:016x}", self.hi, self.lo)
    }
}

/// Widens a length to `u64`.
///
/// Every value passed here is the length of a slice that already exists in
/// memory, so the conversion cannot fail on any supported target; the check
/// is kept rather than an `as` cast so a lossy conversion can never be
/// silent (see the `cast_possible_truncation` lint in `Cargo.toml`).
fn as_u64(n: usize) -> u64 {
    u64::try_from(n).expect("a length exceeds u64")
}

/// A streaming 128-bit hasher.
///
/// Bytes written with [`Hasher128::write`] are hashed exactly as if they
/// had all been handed over in one call, so a caller may fold in an input
/// piece by piece. The `write_*` helpers are *framed*: they prefix the
/// value with its length or write a fixed number of bytes, so that
/// `("ab", "c")` and `("a", "bc")` give different digests. Key building
/// must use the framed helpers; see [`super::key`].
#[derive(Clone, Debug)]
pub struct Hasher128 {
    hi: Xxh64,
    lo: Xxh64,
}

impl Default for Hasher128 {
    fn default() -> Self {
        Self::new()
    }
}

impl Hasher128 {
    /// A hasher with nothing written to it.
    pub fn new() -> Hasher128 {
        Hasher128 {
            hi: Xxh64::new(SEED_HI),
            lo: Xxh64::new(SEED_LO),
        }
    }

    /// Folds in raw bytes, unframed.
    pub fn write(&mut self, bytes: &[u8]) {
        self.hi.update(bytes);
        self.lo.update(bytes);
    }

    /// Folds in a byte string, prefixed with its length.
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        self.write_u64(as_u64(bytes.len()));
        self.write(bytes);
    }

    /// Folds in a string, prefixed with its length in bytes.
    pub fn write_str(&mut self, s: &str) {
        self.write_bytes(s.as_bytes());
    }

    /// Folds in an unsigned integer as 8 little-endian bytes.
    pub fn write_u64(&mut self, v: u64) {
        self.write(&v.to_le_bytes());
    }

    /// Folds in a length, widened to `u64`.
    pub fn write_len(&mut self, n: usize) {
        self.write_u64(as_u64(n));
    }

    /// Folds in a flag as one byte.
    pub fn write_bool(&mut self, v: bool) {
        self.write(&[u8::from(v)]);
    }

    /// Folds in another digest, as 16 little-endian bytes.
    pub fn write_hash(&mut self, h: Hash128) {
        self.write_u64(h.hi);
        self.write_u64(h.lo);
    }

    /// The digest of everything written so far.
    ///
    /// The hasher is left untouched, so more input may follow.
    pub fn finish(&self) -> Hash128 {
        Hash128 {
            hi: self.hi.digest(),
            lo: self.lo.digest(),
        }
    }
}

/// Hashes one byte string, for callers that have it all at once.
pub fn hash128(bytes: &[u8]) -> Hash128 {
    let mut h = Hasher128::new();
    h.write(bytes);
    h.finish()
}

// ---------------------------------------------------------------------------
// xxHash64
// ---------------------------------------------------------------------------

const PRIME1: u64 = 0x9E37_79B1_85EB_CA87;
const PRIME2: u64 = 0xC2B2_AE3D_27D4_EB4F;
const PRIME3: u64 = 0x1656_67B1_9E37_79F9;
const PRIME4: u64 = 0x85EB_CA77_C2B2_AE63;
const PRIME5: u64 = 0x27D4_EB2F_1656_67C5;

/// One xxHash64 lane, in its streaming form.
#[derive(Clone, Debug)]
struct Xxh64 {
    seed: u64,
    acc: [u64; 4],
    /// Bytes not yet consumed by a full 32-byte stripe.
    buf: [u8; 32],
    buf_len: usize,
    total: u64,
}

/// The `round` step of the algorithm.
fn round(acc: u64, input: u64) -> u64 {
    acc.wrapping_add(input.wrapping_mul(PRIME2))
        .rotate_left(31)
        .wrapping_mul(PRIME1)
}

/// The `mergeRound` step, folding one accumulator into the digest.
fn merge_round(acc: u64, val: u64) -> u64 {
    let val = round(0, val);
    (acc ^ val).wrapping_mul(PRIME1).wrapping_add(PRIME4)
}

/// Reads 8 bytes little-endian.
fn lane64(bytes: &[u8]) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&bytes[..8]);
    u64::from_le_bytes(b)
}

/// Reads 4 bytes little-endian, widened.
fn lane32(bytes: &[u8]) -> u64 {
    let mut b = [0u8; 4];
    b.copy_from_slice(&bytes[..4]);
    u64::from(u32::from_le_bytes(b))
}

impl Xxh64 {
    fn new(seed: u64) -> Xxh64 {
        Xxh64 {
            seed,
            acc: [
                seed.wrapping_add(PRIME1).wrapping_add(PRIME2),
                seed.wrapping_add(PRIME2),
                seed,
                seed.wrapping_sub(PRIME1),
            ],
            buf: [0; 32],
            buf_len: 0,
            total: 0,
        }
    }

    /// Consumes one 32-byte stripe.
    fn stripe(&mut self, block: &[u8]) {
        for (i, acc) in self.acc.iter_mut().enumerate() {
            *acc = round(*acc, lane64(&block[i * 8..]));
        }
    }

    fn update(&mut self, mut input: &[u8]) {
        self.total = self.total.wrapping_add(as_u64(input.len()));

        if self.buf_len > 0 {
            let want = 32 - self.buf_len;
            let take = want.min(input.len());
            self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&input[..take]);
            self.buf_len += take;
            input = &input[take..];
            if self.buf_len < 32 {
                return;
            }
            let block = self.buf;
            self.stripe(&block);
            self.buf_len = 0;
        }

        while input.len() >= 32 {
            let (block, rest) = input.split_at(32);
            self.stripe(block);
            input = rest;
        }

        if !input.is_empty() {
            self.buf[..input.len()].copy_from_slice(input);
            self.buf_len = input.len();
        }
    }

    fn digest(&self) -> u64 {
        let mut h = if self.total >= 32 {
            let [a, b, c, d] = self.acc;
            let mut h = a
                .rotate_left(1)
                .wrapping_add(b.rotate_left(7))
                .wrapping_add(c.rotate_left(12))
                .wrapping_add(d.rotate_left(18));
            h = merge_round(h, a);
            h = merge_round(h, b);
            h = merge_round(h, c);
            h = merge_round(h, d);
            h
        } else {
            self.seed.wrapping_add(PRIME5)
        };
        h = h.wrapping_add(self.total);

        let mut rest = &self.buf[..self.buf_len];
        while rest.len() >= 8 {
            h ^= round(0, lane64(rest));
            h = h.rotate_left(27).wrapping_mul(PRIME1).wrapping_add(PRIME4);
            rest = &rest[8..];
        }
        if rest.len() >= 4 {
            h ^= lane32(rest).wrapping_mul(PRIME1);
            h = h.rotate_left(23).wrapping_mul(PRIME2).wrapping_add(PRIME3);
            rest = &rest[4..];
        }
        for &b in rest {
            h ^= u64::from(b).wrapping_mul(PRIME5);
            h = h.rotate_left(11).wrapping_mul(PRIME1);
        }

        h ^= h >> 33;
        h = h.wrapping_mul(PRIME2);
        h ^= h >> 29;
        h = h.wrapping_mul(PRIME3);
        h ^= h >> 32;
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// The `hi` lane alone, for checking against published xxHash64 vectors.
    fn xxh64(bytes: &[u8], seed: u64) -> u64 {
        let mut s = Xxh64::new(seed);
        s.update(bytes);
        s.digest()
    }

    /// The bytes the 1000-byte vector below was taken over.
    fn long_input() -> Vec<u8> {
        (0..1000u32)
            .map(|i| u8::try_from((i * 7 + 3) % 251).expect("below 251"))
            .collect()
    }

    #[test]
    fn matches_published_xxhash64_vectors() {
        // Checked against the reference implementation (xxhsum 0.8.3), seed 0.
        assert_eq!(xxh64(b"", 0), 0xEF46_DB37_51D8_E999);
        assert_eq!(xxh64(b"a", 0), 0xD24E_C4F1_A98C_6E5B);
        assert_eq!(xxh64(b"abc", 0), 0x44BC_2CF5_AD77_0999);
        assert_eq!(xxh64(&long_input(), 0), 0x021F_7A74_2408_5EA4);
    }

    #[test]
    fn streaming_equals_one_shot() {
        let input = long_input();
        let once = hash128(&input);
        for chunk in [1usize, 3, 7, 8, 31, 32, 33, 256] {
            let mut h = Hasher128::new();
            for part in input.chunks(chunk) {
                h.write(part);
            }
            assert_eq!(h.finish(), once, "chunked by {chunk}");
        }
    }

    #[test]
    fn stable_across_runs() {
        // A digest pinned here: if the algorithm ever changes, every stored
        // cache entry silently becomes unreachable, so the change has to be
        // deliberate (and must bump `key::FORMAT`).
        assert_eq!(
            hash128(b"reticle").to_hex(),
            "fbba6de39856cdd15b398ec239a5ca96"
        );
        assert_eq!(hash128(b"").to_hex().len(), 32);
        assert_eq!(
            Hash128::from_hex(&hash128(b"reticle").to_hex()),
            Some(hash128(b"reticle"))
        );
    }

    #[test]
    fn hex_round_trips_and_rejects_junk() {
        let h = Hash128 {
            hi: 0x0123_4567_89ab_cdef,
            lo: 0xfedc_ba98_7654_3210,
        };
        assert_eq!(h.to_hex(), "0123456789abcdeffedcba9876543210");
        assert_eq!(Hash128::from_hex(&h.to_hex()), Some(h));
        assert_eq!(
            Hash128::from_hex("0123456789ABCDEFFEDCBA9876543210"),
            Some(h)
        );
        assert_eq!(Hash128::from_hex(""), None);
        assert_eq!(Hash128::from_hex("0123"), None);
        assert_eq!(Hash128::from_hex(&"z".repeat(32)), None);
    }

    #[test]
    fn one_bit_changes_both_lanes() {
        let base = hash128(b"module counter (input clk, output reg q);");
        let edit = hash128(b"module counter (input clk, output reg r);");
        assert_ne!(base.hi, edit.hi);
        assert_ne!(base.lo, edit.lo);
    }

    #[test]
    fn distribution_has_no_collisions_and_no_stuck_bits() {
        // 20k near-identical inputs, which is the shape a build cache sees:
        // the same text with one number moved.
        let mut seen = HashSet::new();
        let mut ones = [0u32; 128];
        let n = 20_000u32;
        for i in 0..n {
            let h = hash128(format!("module m{i}; endmodule").as_bytes());
            assert!(seen.insert(h), "collision at {i}");
            for bit in 0..64 {
                if h.hi & (1 << bit) != 0 {
                    ones[bit] += 1;
                }
                if h.lo & (1 << bit) != 0 {
                    ones[64 + bit] += 1;
                }
            }
        }
        // Every bit should be set about half the time. A stuck or strongly
        // biased bit is what a broken mixing step looks like; 40..60% is a
        // wide margin around the 50% a good hash gives.
        for (bit, &count) in ones.iter().enumerate() {
            assert!(
                count > n * 2 / 5 && count < n * 3 / 5,
                "bit {bit} set {count} times out of {n}"
            );
        }
    }

    #[test]
    fn framed_writes_are_unambiguous() {
        let mut a = Hasher128::new();
        a.write_str("ab");
        a.write_str("c");
        let mut b = Hasher128::new();
        b.write_str("a");
        b.write_str("bc");
        assert_ne!(a.finish(), b.finish());

        // Unframed writes deliberately are ambiguous; that is why key
        // building never uses them directly.
        let mut c = Hasher128::new();
        c.write(b"ab");
        c.write(b"c");
        assert_eq!(c.finish(), hash128(b"abc"));
    }

    #[test]
    fn integers_and_flags_are_framed() {
        let mut a = Hasher128::new();
        a.write_u64(1);
        let mut b = Hasher128::new();
        b.write_u64(2);
        assert_ne!(a.finish(), b.finish());

        let mut a = Hasher128::new();
        a.write_bool(true);
        let mut b = Hasher128::new();
        b.write_bool(false);
        assert_ne!(a.finish(), b.finish());
    }
}
