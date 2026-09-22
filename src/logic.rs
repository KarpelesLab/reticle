//! Four-state logic values: single bits, VHDL `std_ulogic`, and
//! arbitrary-width bit vectors with Verilog operator semantics.
//!
//! Both frontends evaluate constant expressions on [`Logic`], the IR stores
//! literals as [`Logic`], and the simulator computes on it. [`Bit`] is one
//! four-state bit (`0 1 x z`); [`Std9`] is the nine-state IEEE 1164
//! `std_ulogic` (`U X 0 1 Z W L H -`) with the standard's resolution table,
//! which the VHDL frontend uses for resolved signals and which collapses to
//! [`Bit`] for everything after elaboration.
//!
//! # Encoding
//!
//! A [`Logic`] holds two bit planes of `u64` words, `value` and `unknown`,
//! with bit 0 of word 0 the least significant bit:
//!
//! | `unknown` | `value` | bit |
//! |-----------|---------|-----|
//! | 0         | 0       | `0` |
//! | 0         | 1       | `1` |
//! | 1         | 0       | `x` |
//! | 1         | 1       | `z` |
//!
//! So the `value` plane alone is the 2-state view of a fully known vector,
//! and `unknown == 0` is the test for "no x or z". Bits at or above `width`
//! in the last word are always zero, an invariant every constructor
//! establishes and debug builds assert, so the derived equality and hash
//! are exact and the top word never needs masking on read. A zero-width
//! vector has no words and is allowed (it is what an empty concatenation
//! produces); it prints as `0'h0`.
//!
//! # Semantics
//!
//! Operators follow IEEE 1364-2005 §5. Every operator takes operands that
//! the caller has already sized: binary operators require equal widths and
//! panic otherwise, because context-determined sizing (§5.4) is the
//! frontend's job, and doing it here would hide bugs there. Results keep
//! the operand width, except reductions, logical and relational operators,
//! which yield one unsigned bit. A result is signed only when both operands
//! are signed (§5.5.1); shifts and the power operator take the sign of the
//! left operand only. Arithmetic on any operand containing `x` or `z`
//! yields all `x` (§5.1.5), as does division by zero.
//!
//! `x` and `z` are distinguished only where the standard says so: the
//! case-equality, wildcard and `casez` / `casex` comparisons, structural
//! operations (slice, concatenation, shifts, resize) and literal parsing.
//! Bitwise, logical and arithmetic operators treat `z` as `x` in their
//! inputs and never produce `z`.
//!
//! # Performance
//!
//! This type favours correctness and uniformity: every value is heap-
//! allocated with two planes, and every operator handles four states. The
//! simulator will add fast paths for the common case of two-state values of
//! 64 bits or fewer (an inline `u64` with no `unknown` plane) once the
//! scheduler exists; those will convert to and from this type at the
//! boundaries, so the API here is meant to stay stable. The arbitrary-
//! precision helpers are schoolbook (`O(n²)` multiplication, shift-subtract
//! division), which is fine for constant evaluation and for the widths seen
//! in real designs.

use std::cmp::Ordering;
use std::fmt;
use std::ops::Not;

// ---------------------------------------------------------------------------
// Bit
// ---------------------------------------------------------------------------

/// One four-state bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Bit {
    /// Logic low.
    Zero,
    /// Logic high.
    One,
    /// Unknown.
    X,
    /// High impedance.
    Z,
}

impl Bit {
    /// `One` for `true`, `Zero` for `false`.
    pub fn from_bool(b: bool) -> Self {
        if b { Bit::One } else { Bit::Zero }
    }

    /// True for `Zero` and `One`.
    pub fn is_known(self) -> bool {
        matches!(self, Bit::Zero | Bit::One)
    }

    /// The boolean value of a known bit; `None` for `X` and `Z`.
    pub fn to_bool(self) -> Option<bool> {
        match self {
            Bit::Zero => Some(false),
            Bit::One => Some(true),
            Bit::X | Bit::Z => None,
        }
    }

    /// Parses one Verilog digit character: `0`, `1`, `x`/`X`, `z`/`Z`/`?`.
    pub fn from_char(c: char) -> Option<Self> {
        match c {
            '0' => Some(Bit::Zero),
            '1' => Some(Bit::One),
            'x' | 'X' => Some(Bit::X),
            'z' | 'Z' | '?' => Some(Bit::Z),
            _ => None,
        }
    }

    /// The Verilog character for this bit: `0`, `1`, `x` or `z`.
    pub fn to_char(self) -> char {
        match self {
            Bit::Zero => '0',
            Bit::One => '1',
            Bit::X => 'x',
            Bit::Z => 'z',
        }
    }

    /// The `(value, unknown)` plane bits for this state (see the module
    /// docs for the encoding).
    fn planes(self) -> (bool, bool) {
        match self {
            Bit::Zero => (false, false),
            Bit::One => (true, false),
            Bit::X => (false, true),
            Bit::Z => (true, true),
        }
    }

    /// Inverse of [`Bit::planes`].
    fn from_planes(value: bool, unknown: bool) -> Self {
        match (value, unknown) {
            (false, false) => Bit::Zero,
            (true, false) => Bit::One,
            (false, true) => Bit::X,
            (true, true) => Bit::Z,
        }
    }

    /// Four-state AND (IEEE 1364-2005 Table 5-16): a known `0` on either
    /// side wins; `z` acts as `x`.
    pub fn and(self, other: Self) -> Self {
        match (self.to_bool(), other.to_bool()) {
            (Some(false), _) | (_, Some(false)) => Bit::Zero,
            (Some(true), Some(true)) => Bit::One,
            _ => Bit::X,
        }
    }

    /// Four-state OR (Table 5-17): a known `1` on either side wins.
    pub fn or(self, other: Self) -> Self {
        match (self.to_bool(), other.to_bool()) {
            (Some(true), _) | (_, Some(true)) => Bit::One,
            (Some(false), Some(false)) => Bit::Zero,
            _ => Bit::X,
        }
    }

    /// Four-state XOR (Table 5-18): unknown if either input is unknown.
    pub fn xor(self, other: Self) -> Self {
        match (self.to_bool(), other.to_bool()) {
            (Some(a), Some(b)) => Bit::from_bool(a ^ b),
            _ => Bit::X,
        }
    }
}

/// Four-state NOT (IEEE 1364-2005 Table 5-9): `z` inverts to `x`.
impl Not for Bit {
    type Output = Bit;

    fn not(self) -> Bit {
        match self.to_bool() {
            Some(b) => Bit::from_bool(!b),
            None => Bit::X,
        }
    }
}

impl fmt::Display for Bit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.to_char().encode_utf8(&mut [0; 4]))
    }
}

impl From<bool> for Bit {
    fn from(b: bool) -> Self {
        Bit::from_bool(b)
    }
}

// ---------------------------------------------------------------------------
// Std9
// ---------------------------------------------------------------------------

/// The IEEE 1164 `std_ulogic` value set, in the standard's order.
///
/// The discriminants are the position in the resolution table, so
/// [`Std9::index`] can drive lookup tables in the VHDL runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Std9 {
    /// `'U'`: uninitialised.
    U,
    /// `'X'`: forcing unknown.
    X,
    /// `'0'`: forcing low.
    Zero,
    /// `'1'`: forcing high.
    One,
    /// `'Z'`: high impedance.
    Z,
    /// `'W'`: weak unknown.
    W,
    /// `'L'`: weak low.
    L,
    /// `'H'`: weak high.
    H,
    /// `'-'`: don't care.
    DontCare,
}

impl Std9 {
    /// Every value in table order (`U X 0 1 Z W L H -`).
    pub const ALL: [Std9; 9] = [
        Std9::U,
        Std9::X,
        Std9::Zero,
        Std9::One,
        Std9::Z,
        Std9::W,
        Std9::L,
        Std9::H,
        Std9::DontCare,
    ];

    /// Parses the character of a `std_ulogic` literal. Letters are accepted
    /// in either case, since VHDL-2008 bit-string values are written in
    /// either; `'-'` is the don't-care.
    pub fn from_char(c: char) -> Option<Self> {
        match c {
            'U' | 'u' => Some(Std9::U),
            'X' | 'x' => Some(Std9::X),
            '0' => Some(Std9::Zero),
            '1' => Some(Std9::One),
            'Z' | 'z' => Some(Std9::Z),
            'W' | 'w' => Some(Std9::W),
            'L' | 'l' => Some(Std9::L),
            'H' | 'h' => Some(Std9::H),
            '-' => Some(Std9::DontCare),
            _ => None,
        }
    }

    /// The character of the `std_ulogic` literal, as the standard spells it.
    pub fn to_char(self) -> char {
        match self {
            Std9::U => 'U',
            Std9::X => 'X',
            Std9::Zero => '0',
            Std9::One => '1',
            Std9::Z => 'Z',
            Std9::W => 'W',
            Std9::L => 'L',
            Std9::H => 'H',
            Std9::DontCare => '-',
        }
    }

    /// Position in the resolution table, `0..9`.
    pub fn index(self) -> usize {
        self as usize
    }

    /// The four-state view: `U`, `X`, `W` and `-` become `x`, the weak
    /// levels `L` and `H` become `0` and `1`.
    pub fn to_bit(self) -> Bit {
        match self {
            Std9::Zero | Std9::L => Bit::Zero,
            Std9::One | Std9::H => Bit::One,
            Std9::Z => Bit::Z,
            Std9::U | Std9::X | Std9::W | Std9::DontCare => Bit::X,
        }
    }

    /// The nine-state view of a four-state bit: `x` becomes the forcing
    /// unknown `X`.
    pub fn from_bit(b: Bit) -> Self {
        match b {
            Bit::Zero => Std9::Zero,
            Bit::One => Std9::One,
            Bit::X => Std9::X,
            Bit::Z => Std9::Z,
        }
    }

    /// The IEEE 1164 resolution function for two drivers.
    ///
    /// `U` dominates everything, then `X` (and `-`, which resolves as `X`);
    /// `Z` yields to any other driver; a conflict between forcing `0` and
    /// `1` is `X`; a forcing level beats a weak one; and two different weak
    /// levels are `W`. The full table is checked in the tests.
    pub fn resolve(a: Std9, b: Std9) -> Std9 {
        use Std9::*;
        match (a, b) {
            (U, _) | (_, U) => U,
            (X, _) | (_, X) | (DontCare, _) | (_, DontCare) => X,
            (Z, o) | (o, Z) => o,
            (a, b) if a == b => a,
            (Zero, One) | (One, Zero) => X,
            (Zero, _) | (_, Zero) => Zero,
            (One, _) | (_, One) => One,
            _ => W,
        }
    }

    /// Resolves any number of drivers; no driver at all resolves to `Z`, as
    /// for an undriven resolved signal.
    pub fn resolve_all(drivers: impl IntoIterator<Item = Std9>) -> Std9 {
        drivers.into_iter().fold(Std9::Z, Std9::resolve)
    }
}

impl fmt::Display for Std9 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.to_char().encode_utf8(&mut [0; 4]))
    }
}

impl From<Std9> for Bit {
    fn from(s: Std9) -> Self {
        s.to_bit()
    }
}

impl From<Bit> for Std9 {
    fn from(b: Bit) -> Self {
        Std9::from_bit(b)
    }
}

// ---------------------------------------------------------------------------
// Word-level helpers
// ---------------------------------------------------------------------------

/// Number of `u64` words needed for `width` bits.
fn word_count(width: u32) -> usize {
    (width as usize).div_ceil(64)
}

/// Mask of the valid bits in the last word of a `width`-bit vector.
fn top_mask(width: u32) -> u64 {
    match width % 64 {
        0 => u64::MAX,
        r => (1u64 << r) - 1,
    }
}

/// Clears the bits above `width` in the last word.
fn mask_top(words: &mut [u64], width: u32) {
    if let Some(last) = words.last_mut() {
        *last &= top_mask(width);
    }
}

/// Mask of the bits of word `i` that lie below `width`.
fn word_mask(i: usize, width: u32) -> u64 {
    if i + 1 == word_count(width) {
        top_mask(width)
    } else {
        u64::MAX
    }
}

/// Splits a bit index into `(word, bit within word)`.
fn locate(i: u32) -> (usize, u32) {
    ((i / 64) as usize, i % 64)
}

fn get_bit(words: &[u64], i: u32) -> bool {
    let (w, b) = locate(i);
    words.get(w).is_some_and(|&x| (x >> b) & 1 != 0)
}

fn set_bit(words: &mut [u64], i: u32, on: bool) {
    let (w, b) = locate(i);
    if on {
        words[w] |= 1u64 << b;
    } else {
        words[w] &= !(1u64 << b);
    }
}

/// Sets bits `from..to` to one.
fn fill_ones(words: &mut [u64], from: u32, to: u32) {
    if from >= to {
        return;
    }
    let (first, fb) = locate(from);
    let (last, lb) = locate(to - 1);
    for (i, w) in words.iter_mut().enumerate().take(last + 1).skip(first) {
        let lo = if i == first { fb } else { 0 };
        let hi = if i == last { lb } else { 63 };
        let span = if hi - lo == 63 {
            u64::MAX
        } else {
            ((1u64 << (hi - lo + 1)) - 1) << lo
        };
        *w |= span;
    }
}

fn is_zero(words: &[u64]) -> bool {
    words.iter().all(|&w| w == 0)
}

/// Index of the highest set bit, if any.
fn highest_bit(words: &[u64]) -> Option<u32> {
    words
        .iter()
        .enumerate()
        .rev()
        .find(|&(_, &w)| w != 0)
        .map(|(i, &w)| {
            let word = u32::try_from(i).expect("word index fits u32");
            word * 64 + (63 - w.leading_zeros())
        })
}

/// Unsigned comparison; the shorter operand is treated as zero-extended.
fn cmp_words(a: &[u64], b: &[u64]) -> Ordering {
    let n = a.len().max(b.len());
    for i in (0..n).rev() {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        match x.cmp(&y) {
            Ordering::Equal => continue,
            other => return other,
        }
    }
    Ordering::Equal
}

/// `acc += b` (with `b` zero-extended to `acc`'s length); returns the carry
/// out.
fn add_into(acc: &mut [u64], b: &[u64]) -> bool {
    let mut carry = false;
    for (i, a) in acc.iter_mut().enumerate() {
        let y = b.get(i).copied().unwrap_or(0);
        let (s1, c1) = a.overflowing_add(y);
        let (s2, c2) = s1.overflowing_add(u64::from(carry));
        *a = s2;
        carry = c1 || c2;
    }
    carry
}

/// `acc -= b` (with `b` zero-extended to `acc`'s length); returns the borrow
/// out.
fn sub_into(acc: &mut [u64], b: &[u64]) -> bool {
    let mut borrow = false;
    for (i, a) in acc.iter_mut().enumerate() {
        let y = b.get(i).copied().unwrap_or(0);
        let (d1, b1) = a.overflowing_sub(y);
        let (d2, b2) = d1.overflowing_sub(u64::from(borrow));
        *a = d2;
        borrow = b1 || b2;
    }
    borrow
}

/// Two's complement negation, truncated to `width` bits.
fn neg_words(words: &[u64], width: u32) -> Vec<u64> {
    let mut out: Vec<u64> = words.iter().map(|w| !w).collect();
    add_into(&mut out, &[1]);
    mask_top(&mut out, width);
    out
}

/// Low 64 bits of a double-word intermediate.
fn lo64(x: u128) -> u64 {
    u64::try_from(x & u128::from(u64::MAX)).expect("masked to 64 bits")
}

/// Schoolbook product, keeping only as many words as `a` has.
fn mul_low(a: &[u64], b: &[u64]) -> Vec<u64> {
    let n = a.len();
    let mut out = vec![0u64; n];
    for (i, &ai) in a.iter().enumerate() {
        if ai == 0 {
            continue;
        }
        let mut carry: u128 = 0;
        for j in 0..n - i {
            let t = u128::from(ai) * u128::from(b[j]) + u128::from(out[i + j]) + carry;
            out[i + j] = lo64(t);
            carry = t >> 64;
        }
    }
    out
}

/// Unsigned shift-subtract division; `d` must be non-zero. Both results have
/// `n`'s length.
fn divrem(n: &[u64], d: &[u64]) -> (Vec<u64>, Vec<u64>) {
    debug_assert!(!is_zero(d), "division by zero reaches divrem");
    let words = n.len();
    let mut q = vec![0u64; words];
    // One extra word so the shift never loses a bit when `n` is 64k wide.
    let mut r = vec![0u64; words + 1];
    let Some(top) = highest_bit(n) else {
        return (q, vec![0u64; words]);
    };
    for i in (0..=top).rev() {
        let mut carry = get_bit(n, i);
        for w in r.iter_mut() {
            let next = *w >> 63 != 0;
            *w = (*w << 1) | u64::from(carry);
            carry = next;
        }
        if cmp_words(&r, d) != Ordering::Less {
            sub_into(&mut r, d);
            set_bit(&mut q, i, true);
        }
    }
    r.truncate(words);
    (q, r)
}

/// `words = words * mul + add`, growing as needed (for parsing decimals).
fn mul_small_add(words: &mut Vec<u64>, mul: u64, add: u64) {
    let mut carry = u128::from(add);
    for w in words.iter_mut() {
        let t = u128::from(*w) * u128::from(mul) + carry;
        *w = lo64(t);
        carry = t >> 64;
    }
    if carry != 0 {
        words.push(lo64(carry));
    }
}

/// Logical shift left within a `width`-bit vector; vacated bits are zero.
fn shl_words(src: &[u64], n: u32, width: u32) -> Vec<u64> {
    let len = src.len();
    let mut out = vec![0u64; len];
    if n >= width {
        return out;
    }
    let (ws, bs) = locate(n);
    for (i, o) in out.iter_mut().enumerate().skip(ws) {
        let j = i - ws;
        let mut v = src[j] << bs;
        if bs != 0 && j > 0 {
            v |= src[j - 1] >> (64 - bs);
        }
        *o = v;
    }
    mask_top(&mut out, width);
    out
}

/// Shift right within a `width`-bit vector; vacated bits are `fill`.
fn shr_words(src: &[u64], n: u32, width: u32, fill: bool) -> Vec<u64> {
    let len = src.len();
    let mut out = vec![0u64; len];
    if n >= width {
        if fill {
            fill_ones(&mut out, 0, width);
        }
        return out;
    }
    let (ws, bs) = locate(n);
    for (i, o) in out.iter_mut().enumerate().take(len - ws) {
        let j = i + ws;
        let mut v = src[j] >> bs;
        if bs != 0 && j + 1 < len {
            v |= src[j + 1] << (64 - bs);
        }
        *o = v;
    }
    if fill {
        fill_ones(&mut out, width - n, width);
    }
    out
}

// ---------------------------------------------------------------------------
// Logic
// ---------------------------------------------------------------------------

/// A four-state bit vector of arbitrary width with a signedness flag.
///
/// See the module docs for the encoding and the operator semantics. The
/// derived `PartialEq`, `Eq` and `Hash` are structural: two values are equal
/// when their width, signedness and every bit agree, `x` and `z` included.
/// The Verilog `==` operator is [`Logic::eq`].
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Logic {
    width: u32,
    signed: bool,
    value: Vec<u64>,
    unknown: Vec<u64>,
}

impl Logic {
    // ---- construction ----

    /// Builds a value from its planes, normalising the top word.
    ///
    /// Both planes must have exactly `ceil(width / 64)` words; bits at
    /// or above `width` are cleared. This is the escape hatch for the
    /// simulator's fast paths; everything else goes through the named
    /// constructors.
    ///
    /// # Panics
    ///
    /// Panics if a plane has the wrong number of words.
    pub fn from_planes(
        width: u32,
        signed: bool,
        mut value: Vec<u64>,
        mut unknown: Vec<u64>,
    ) -> Self {
        let words = word_count(width);
        assert_eq!(value.len(), words, "value plane has the wrong word count");
        assert_eq!(
            unknown.len(),
            words,
            "unknown plane has the wrong word count"
        );
        mask_top(&mut value, width);
        mask_top(&mut unknown, width);
        Logic {
            width,
            signed,
            value,
            unknown,
        }
    }

    /// Debug check of the top-word invariant.
    fn check(&self) {
        debug_assert_eq!(self.value.len(), word_count(self.width));
        debug_assert_eq!(self.unknown.len(), word_count(self.width));
        if let (Some(&v), Some(&u)) = (self.value.last(), self.unknown.last()) {
            let m = top_mask(self.width);
            debug_assert_eq!(v & !m, 0, "value plane has bits above width");
            debug_assert_eq!(u & !m, 0, "unknown plane has bits above width");
        }
    }

    /// Builds an unsigned value from planes that are already normalised.
    fn raw(width: u32, signed: bool, value: Vec<u64>, unknown: Vec<u64>) -> Self {
        let l = Logic {
            width,
            signed,
            value,
            unknown,
        };
        l.check();
        l
    }

    /// Fills every bit of a `width`-wide unsigned vector with `bit`.
    pub fn filled(width: u32, bit: Bit) -> Self {
        let words = word_count(width);
        let (v, u) = bit.planes();
        let plane = |on: bool| {
            let mut w = vec![0u64; words];
            if on {
                fill_ones(&mut w, 0, width);
            }
            w
        };
        Logic::raw(width, false, plane(v), plane(u))
    }

    /// All zeros, unsigned.
    pub fn zero(width: u32) -> Self {
        Logic::filled(width, Bit::Zero)
    }

    /// All ones, unsigned.
    pub fn ones(width: u32) -> Self {
        Logic::filled(width, Bit::One)
    }

    /// All `x`, unsigned.
    pub fn x(width: u32) -> Self {
        Logic::filled(width, Bit::X)
    }

    /// All `z`, unsigned.
    pub fn z(width: u32) -> Self {
        Logic::filled(width, Bit::Z)
    }

    /// An unsigned vector holding the low `width` bits of `value`.
    ///
    /// Bits of `value` above `width` are dropped; widths above 64 are
    /// zero-extended.
    pub fn from_u64(value: u64, width: u32) -> Self {
        let words = word_count(width);
        let mut v = vec![0u64; words];
        if words > 0 {
            v[0] = value;
        }
        mask_top(&mut v, width);
        Logic::raw(width, false, v, vec![0u64; words])
    }

    /// A signed vector holding `value` sign-extended or truncated to
    /// `width` bits (two's complement).
    pub fn from_i64(value: i64, width: u32) -> Self {
        let words = word_count(width);
        let fill = if value < 0 { u64::MAX } else { 0 };
        let mut v = vec![fill; words];
        if words > 0 {
            v[0] = value.cast_unsigned();
        }
        mask_top(&mut v, width);
        Logic::raw(width, true, v, vec![0u64; words])
    }

    /// A one-bit unsigned vector.
    pub fn from_bool(b: bool) -> Self {
        Logic::from_u64(u64::from(b), 1)
    }

    /// A one-bit unsigned vector holding `bit`.
    pub fn from_bit(bit: Bit) -> Self {
        Logic::filled(1, bit)
    }

    /// An unsigned vector from individual bits, `bits[0]` being the LSB.
    ///
    /// # Panics
    ///
    /// Panics if there are more than `u32::MAX` bits.
    pub fn from_bits(bits: &[Bit]) -> Self {
        let width = u32::try_from(bits.len()).expect("bit count exceeds u32");
        let words = word_count(width);
        let mut value = vec![0u64; words];
        let mut unknown = vec![0u64; words];
        for (i, bit) in bits.iter().enumerate() {
            let (v, u) = bit.planes();
            let i = u32::try_from(i).expect("bit index fits u32");
            if v {
                set_bit(&mut value, i, true);
            }
            if u {
                set_bit(&mut unknown, i, true);
            }
        }
        Logic::raw(width, false, value, unknown)
    }

    /// An unsigned vector from `std_ulogic` values (folded through
    /// [`Std9::to_bit`]), `bits[0]` being the LSB.
    pub fn from_std9(bits: &[Std9]) -> Self {
        let bits: Vec<Bit> = bits.iter().map(|s| s.to_bit()).collect();
        Logic::from_bits(&bits)
    }

    // ---- inspection ----

    /// Width in bits.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// True when arithmetic and relational operators treat this as two's
    /// complement.
    pub fn is_signed(&self) -> bool {
        self.signed
    }

    /// The same bits, flagged signed.
    pub fn as_signed(mut self) -> Self {
        self.signed = true;
        self
    }

    /// The same bits, flagged unsigned.
    pub fn as_unsigned(mut self) -> Self {
        self.signed = false;
        self
    }

    /// The same bits with the given signedness.
    pub fn with_signed(mut self, signed: bool) -> Self {
        self.signed = signed;
        self
    }

    /// The `value` plane, LSB word first (see the module docs).
    pub fn value_words(&self) -> &[u64] {
        &self.value
    }

    /// The `unknown` plane, LSB word first (see the module docs).
    pub fn unknown_words(&self) -> &[u64] {
        &self.unknown
    }

    /// True when no bit is `x` or `z`.
    pub fn is_fully_known(&self) -> bool {
        is_zero(&self.unknown)
    }

    /// True when some bit is `x` or `z`.
    pub fn has_unknown(&self) -> bool {
        !self.is_fully_known()
    }

    /// True when every bit is a known `0`.
    pub fn is_zero(&self) -> bool {
        self.is_fully_known() && is_zero(&self.value)
    }

    /// True when the value is signed, fully known and its MSB is set.
    pub fn is_negative(&self) -> bool {
        self.signed && self.is_fully_known() && self.msb() == Bit::One
    }

    /// The bit at index `i` (0 is the LSB).
    ///
    /// # Panics
    ///
    /// Panics if `i >= width`. Out-of-range reads have language-specific
    /// meanings (Verilog yields `x`), which the caller applies through
    /// [`Logic::get`].
    pub fn bit(&self, i: u32) -> Bit {
        assert!(
            i < self.width,
            "bit {i} out of range for width {}",
            self.width
        );
        Bit::from_planes(get_bit(&self.value, i), get_bit(&self.unknown, i))
    }

    /// The bit at index `i`, or `None` when `i >= width`.
    pub fn get(&self, i: u32) -> Option<Bit> {
        (i < self.width).then(|| self.bit(i))
    }

    /// The most significant bit; `Zero` for a zero-width vector.
    pub fn msb(&self) -> Bit {
        self.get(self.width.wrapping_sub(1)).unwrap_or(Bit::Zero)
    }

    /// Sets the bit at index `i`.
    ///
    /// # Panics
    ///
    /// Panics if `i >= width`.
    pub fn set_bit(&mut self, i: u32, bit: Bit) {
        assert!(
            i < self.width,
            "bit {i} out of range for width {}",
            self.width
        );
        let (v, u) = bit.planes();
        set_bit(&mut self.value, i, v);
        set_bit(&mut self.unknown, i, u);
    }

    /// Every bit, LSB first.
    pub fn bits(&self) -> Vec<Bit> {
        (0..self.width).map(|i| self.bit(i)).collect()
    }

    /// The value as an unsigned integer: `None` if any bit is `x` or `z`,
    /// or if the value does not fit in 64 bits. Signedness is ignored (a
    /// negative signed value yields its raw two's complement bits when
    /// the width is at most 64).
    pub fn to_u64(&self) -> Option<u64> {
        if self.has_unknown() || !is_zero(self.value.get(1..).unwrap_or(&[])) {
            return None;
        }
        Some(self.value.first().copied().unwrap_or(0))
    }

    /// The value as a signed integer, interpreting the bits per
    /// [`Logic::is_signed`]: `None` if any bit is `x` or `z`, or if the
    /// value does not fit in an `i64`.
    pub fn to_i64(&self) -> Option<i64> {
        if self.has_unknown() {
            return None;
        }
        if self.width == 0 {
            return Some(0);
        }
        let low = self.value[0];
        if !self.signed {
            return self.to_u64().and_then(|v| i64::try_from(v).ok());
        }
        if self.width <= 64 {
            let shift = 64 - self.width;
            return Some((low << shift).cast_signed() >> shift);
        }
        // Wider than 64: every upper bit must repeat bit 63.
        let fill = if low >> 63 != 0 { u64::MAX } else { 0 };
        let upper_ok = self.value[1..]
            .iter()
            .enumerate()
            .all(|(i, &w)| w == fill & word_mask(i + 1, self.width));
        upper_ok.then(|| low.cast_signed())
    }

    /// The bits as a string of `0`, `1`, `x`, `z`, MSB first.
    pub fn to_binary_string(&self) -> String {
        (0..self.width)
            .rev()
            .map(|i| self.bit(i).to_char())
            .collect()
    }

    /// Hex digits, MSB first, when every nibble is either fully known, all
    /// `x` or all `z`; `None` when some nibble mixes states.
    fn hex_digits(&self) -> Option<String> {
        if self.width == 0 {
            return Some("0".to_owned());
        }
        let nibbles = self.width.div_ceil(4);
        let mut out = String::with_capacity(nibbles as usize);
        for n in (0..nibbles).rev() {
            let lo = n * 4;
            let count = (self.width - lo).min(4);
            let (w, b) = locate(lo);
            let full = (1u64 << count) - 1;
            let v = (self.value[w] >> b) & full;
            let u = (self.unknown[w] >> b) & full;
            let ch = if u == 0 {
                char::from_digit(u32::try_from(v).expect("nibble fits u32"), 16)?
            } else if u == full && v == 0 {
                'x'
            } else if u == full && v == full {
                'z'
            } else {
                return None;
            };
            out.push(ch);
        }
        Some(out)
    }

    /// The value as a sized Verilog literal such as `8'hff`, `4'b10x1` or
    /// `8'shff`.
    ///
    /// Hex is used when every nibble is representable, otherwise binary, so
    /// the text is lossless and [`Logic::parse_verilog`] reads it back to an
    /// identical value.
    pub fn to_verilog_literal(&self) -> String {
        let s = if self.signed { "s" } else { "" };
        match self.hex_digits() {
            Some(h) => format!("{}'{s}h{h}", self.width),
            None => format!("{}'{s}b{}", self.width, self.to_binary_string()),
        }
    }

    // ---- structural operations ----

    /// Bits `lo..=hi` as a new unsigned vector of width `hi - lo + 1`.
    ///
    /// # Panics
    ///
    /// Panics if `hi < lo` or `hi >= width`.
    pub fn slice(&self, hi: u32, lo: u32) -> Self {
        assert!(hi >= lo, "slice [{hi}:{lo}] is reversed");
        assert!(
            hi < self.width,
            "slice [{hi}:{lo}] exceeds width {}",
            self.width
        );
        self.extract(lo, hi - lo + 1)
    }

    /// `width` bits starting at `lo`, zero-filled past the end; unsigned.
    fn extract(&self, lo: u32, width: u32) -> Self {
        let words = word_count(width);
        let mut value = shr_words(&self.value, lo, self.width, false);
        let mut unknown = shr_words(&self.unknown, lo, self.width, false);
        value.resize(words, 0);
        unknown.resize(words, 0);
        mask_top(&mut value, width);
        mask_top(&mut unknown, width);
        Logic::raw(width, false, value, unknown)
    }

    /// `{self, lsb}`: this value in the high bits, `lsb` in the low bits.
    /// Unsigned, as Verilog concatenations are.
    ///
    /// # Panics
    ///
    /// Panics if the combined width exceeds `u32::MAX`.
    pub fn concat(&self, lsb: &Logic) -> Self {
        let width = self
            .width
            .checked_add(lsb.width)
            .expect("concatenation width exceeds u32");
        let words = word_count(width);
        let join = |hi: &[u64], lo: &[u64]| {
            let mut out = lo.to_vec();
            out.resize(words, 0);
            let shifted = shl_words(
                &{
                    let mut h = hi.to_vec();
                    h.resize(words, 0);
                    h
                },
                lsb.width,
                width,
            );
            for (o, s) in out.iter_mut().zip(shifted) {
                *o |= s;
            }
            out
        };
        Logic::raw(
            width,
            false,
            join(&self.value, &lsb.value),
            join(&self.unknown, &lsb.unknown),
        )
    }

    /// `{a, b, c, ...}` for any number of parts, the first being the most
    /// significant. No parts yield a zero-width vector.
    pub fn concat_all<'a>(parts: impl IntoIterator<Item = &'a Logic>) -> Self {
        parts
            .into_iter()
            .fold(Logic::zero(0), |acc, part| acc.concat(part))
    }

    /// `{n{self}}`: this value repeated `n` times; unsigned.
    ///
    /// # Panics
    ///
    /// Panics if the resulting width exceeds `u32::MAX`.
    pub fn replicate(&self, n: u32) -> Self {
        let width = self
            .width
            .checked_mul(n)
            .expect("replication width exceeds u32");
        let mut out = Logic::zero(width);
        for i in 0..n {
            let at = i * self.width;
            let v = shl_words(&self.padded(width, false), at, width);
            let u = shl_words(&self.padded(width, true), at, width);
            for (o, s) in out.value.iter_mut().zip(v) {
                *o |= s;
            }
            for (o, s) in out.unknown.iter_mut().zip(u) {
                *o |= s;
            }
        }
        out
    }

    /// One plane copied and zero-extended to the word count of `width`.
    fn padded(&self, width: u32, unknown: bool) -> Vec<u64> {
        let mut w = if unknown {
            self.unknown.clone()
        } else {
            self.value.clone()
        };
        w.resize(word_count(width), 0);
        w
    }

    /// Changes the width, keeping the signedness: wider values are zero-
    /// extended when unsigned and extended with copies of the MSB when
    /// signed (an `x` or `z` MSB is replicated as is); narrower values keep
    /// their low bits.
    pub fn resize(&self, width: u32) -> Self {
        let words = word_count(width);
        let extend = |plane: &[u64], msb: bool| {
            let mut out = plane.to_vec();
            out.resize(words, 0);
            if width > self.width && msb {
                fill_ones(&mut out, self.width, width);
            }
            mask_top(&mut out, width);
            out
        };
        let (v, u) = if self.signed {
            self.msb().planes()
        } else {
            (false, false)
        };
        Logic::raw(
            width,
            self.signed,
            extend(&self.value, v),
            extend(&self.unknown, u),
        )
    }

    // ---- bitwise operators ----

    /// Asserts that a binary operator's operands have the same width.
    fn same_width(&self, other: &Logic, op: &str) {
        assert_eq!(
            self.width, other.width,
            "operands of `{op}` differ in width; the caller must size them"
        );
    }

    /// Applies a word-wise function to the planes of two operands.
    fn zip_planes(
        &self,
        other: &Logic,
        op: &str,
        f: impl Fn(u64, u64, u64, u64) -> (u64, u64),
    ) -> Self {
        self.same_width(other, op);
        let words = self.value.len();
        let mut value = Vec::with_capacity(words);
        let mut unknown = Vec::with_capacity(words);
        for i in 0..words {
            let (v, u) = f(
                self.value[i],
                self.unknown[i],
                other.value[i],
                other.unknown[i],
            );
            let m = word_mask(i, self.width);
            value.push(v & m);
            unknown.push(u & m);
        }
        Logic::raw(self.width, self.signed && other.signed, value, unknown)
    }

    /// Bitwise AND (`&`), per [`Bit::and`] on every bit.
    pub fn and(&self, other: &Logic) -> Self {
        self.zip_planes(other, "&", |av, au, bv, bu| {
            let a0 = !av & !au;
            let b0 = !bv & !bu;
            let a1 = av & !au;
            let b1 = bv & !bu;
            let r0 = a0 | b0;
            let r1 = a1 & b1;
            (r1, !(r0 | r1))
        })
    }

    /// Bitwise OR (`|`), per [`Bit::or`] on every bit.
    pub fn or(&self, other: &Logic) -> Self {
        self.zip_planes(other, "|", |av, au, bv, bu| {
            let a0 = !av & !au;
            let b0 = !bv & !bu;
            let a1 = av & !au;
            let b1 = bv & !bu;
            let r0 = a0 & b0;
            let r1 = a1 | b1;
            (r1, !(r0 | r1))
        })
    }

    /// Bitwise XOR (`^`), per [`Bit::xor`] on every bit.
    pub fn xor(&self, other: &Logic) -> Self {
        self.zip_planes(other, "^", |av, au, bv, bu| {
            let u = au | bu;
            ((av ^ bv) & !u, u)
        })
    }

    /// Bitwise XNOR (`~^`).
    pub fn xnor(&self, other: &Logic) -> Self {
        self.zip_planes(other, "~^", |av, au, bv, bu| {
            let u = au | bu;
            (!(av ^ bv) & !u, u)
        })
    }

    /// Bitwise NOT (`~`); `z` inverts to `x`.
    pub fn not(&self) -> Self {
        let mut value = Vec::with_capacity(self.value.len());
        for (i, (&v, &u)) in self.value.iter().zip(&self.unknown).enumerate() {
            value.push(!v & !u & word_mask(i, self.width));
        }
        Logic::raw(self.width, self.signed, value, self.unknown.clone())
    }

    // ---- reductions and logical operators ----

    /// True when some bit is a known `0`.
    fn has_known_zero(&self) -> bool {
        self.value
            .iter()
            .zip(&self.unknown)
            .enumerate()
            .any(|(i, (&v, &u))| !v & !u & word_mask(i, self.width) != 0)
    }

    /// True when some bit is a known `1`.
    fn has_known_one(&self) -> bool {
        self.value
            .iter()
            .zip(&self.unknown)
            .any(|(&v, &u)| v & !u != 0)
    }

    /// Reduction AND (`&a`): `0` if any bit is `0`, `1` if all are `1`,
    /// else `x`.
    pub fn reduce_and(&self) -> Self {
        Logic::from_bit(if self.has_known_zero() {
            Bit::Zero
        } else if self.is_fully_known() {
            Bit::One
        } else {
            Bit::X
        })
    }

    /// Reduction OR (`|a`): `1` if any bit is `1`, `0` if all are `0`,
    /// else `x`.
    pub fn reduce_or(&self) -> Self {
        Logic::from_bit(self.truth())
    }

    /// Reduction XOR (`^a`): parity, or `x` if any bit is unknown.
    pub fn reduce_xor(&self) -> Self {
        Logic::from_bit(if self.has_unknown() {
            Bit::X
        } else {
            let ones: u32 = self.value.iter().map(|w| w.count_ones()).sum();
            Bit::from_bool(ones % 2 == 1)
        })
    }

    /// Reduction NAND (`~&a`).
    pub fn reduce_nand(&self) -> Self {
        self.reduce_and().not()
    }

    /// Reduction NOR (`~|a`).
    pub fn reduce_nor(&self) -> Self {
        self.reduce_or().not()
    }

    /// Reduction XNOR (`~^a`).
    pub fn reduce_xnor(&self) -> Self {
        self.reduce_xor().not()
    }

    /// The truth value used by `if`, `&&`, `||` and `!`: `1` when any bit
    /// is a known `1`, `0` when every bit is a known `0`, `x` otherwise.
    pub fn truth(&self) -> Bit {
        if self.has_known_one() {
            Bit::One
        } else if self.is_fully_known() {
            Bit::Zero
        } else {
            Bit::X
        }
    }

    /// Logical AND (`&&`) on the truth values; one unsigned bit.
    pub fn logical_and(&self, other: &Logic) -> Self {
        Logic::from_bit(self.truth().and(other.truth()))
    }

    /// Logical OR (`||`) on the truth values; one unsigned bit.
    pub fn logical_or(&self, other: &Logic) -> Self {
        Logic::from_bit(self.truth().or(other.truth()))
    }

    /// Logical NOT (`!`) of the truth value; one unsigned bit.
    pub fn logical_not(&self) -> Self {
        Logic::from_bit(!self.truth())
    }

    // ---- shifts ----

    /// Logical shift left (`<<`) by a fixed amount; zeros shift in, the
    /// width and signedness are kept.
    pub fn shl(&self, n: u32) -> Self {
        Logic::raw(
            self.width,
            self.signed,
            shl_words(&self.value, n, self.width),
            shl_words(&self.unknown, n, self.width),
        )
    }

    /// Logical shift right (`>>`) by a fixed amount; zeros shift in.
    pub fn shr(&self, n: u32) -> Self {
        Logic::raw(
            self.width,
            self.signed,
            shr_words(&self.value, n, self.width, false),
            shr_words(&self.unknown, n, self.width, false),
        )
    }

    /// Arithmetic shift right (`>>>`): copies of the MSB shift in when the
    /// value is signed (an `x` or `z` MSB is replicated as is); a logical
    /// shift otherwise, as the standard prescribes for unsigned operands.
    pub fn sshr(&self, n: u32) -> Self {
        let (v, u) = if self.signed {
            self.msb().planes()
        } else {
            (false, false)
        };
        Logic::raw(
            self.width,
            self.signed,
            shr_words(&self.value, n, self.width, v),
            shr_words(&self.unknown, n, self.width, u),
        )
    }

    /// The shift amount an operand denotes: `None` when it has `x` or `z`
    /// bits (the shift result is then all `x`), else the amount clamped to
    /// `u32::MAX`, which shifts everything out anyway.
    fn shift_amount(amount: &Logic) -> Option<u32> {
        if amount.has_unknown() {
            return None;
        }
        Some(
            amount
                .to_u64()
                .and_then(|a| u32::try_from(a).ok())
                .unwrap_or(u32::MAX),
        )
    }

    /// `<<` with a vector amount; all `x` if the amount is not fully known.
    pub fn shl_by(&self, amount: &Logic) -> Self {
        match Self::shift_amount(amount) {
            Some(n) => self.shl(n),
            None => Logic::x(self.width).with_signed(self.signed),
        }
    }

    /// `>>` with a vector amount; all `x` if the amount is not fully known.
    pub fn shr_by(&self, amount: &Logic) -> Self {
        match Self::shift_amount(amount) {
            Some(n) => self.shr(n),
            None => Logic::x(self.width).with_signed(self.signed),
        }
    }

    /// `>>>` with a vector amount; all `x` if the amount is not fully known.
    pub fn sshr_by(&self, amount: &Logic) -> Self {
        match Self::shift_amount(amount) {
            Some(n) => self.sshr(n),
            None => Logic::x(self.width).with_signed(self.signed),
        }
    }

    // ---- equality and relations ----

    /// Logical equality (`==`): `0` when some bit position is known on both
    /// sides and differs, `x` when the comparison is otherwise ambiguous
    /// because of `x` or `z` bits, `1` when all bits are known and equal
    /// (§5.1.8).
    pub fn eq(&self, other: &Logic) -> Self {
        self.same_width(other, "==");
        let known_diff = self
            .value
            .iter()
            .zip(&self.unknown)
            .zip(other.value.iter().zip(&other.unknown))
            .any(|((&av, &au), (&bv, &bu))| (av ^ bv) & !au & !bu != 0);
        Logic::from_bit(if known_diff {
            Bit::Zero
        } else if self.has_unknown() || other.has_unknown() {
            Bit::X
        } else {
            Bit::One
        })
    }

    /// Logical inequality (`!=`): the negation of [`Logic::eq`].
    pub fn ne(&self, other: &Logic) -> Self {
        self.eq(other).not()
    }

    /// Case equality (`===`): `1` when every bit matches exactly, `x` and
    /// `z` included; never `x`.
    pub fn case_eq(&self, other: &Logic) -> Self {
        self.same_width(other, "===");
        Logic::from_bool(self.value == other.value && self.unknown == other.unknown)
    }

    /// Case inequality (`!==`).
    pub fn case_ne(&self, other: &Logic) -> Self {
        self.case_eq(other).not()
    }

    /// Wildcard equality (`==?`, SystemVerilog): `x` and `z` bits of
    /// `pattern` match anything; the remaining bits compare as `==`.
    pub fn wildcard_eq(&self, pattern: &Logic) -> Self {
        self.same_width(pattern, "==?");
        let mut known_diff = false;
        let mut ambiguous = false;
        for ((&av, &au), (&pv, &pu)) in self
            .value
            .iter()
            .zip(&self.unknown)
            .zip(pattern.value.iter().zip(&pattern.unknown))
        {
            let care = !pu;
            known_diff |= (av ^ pv) & !au & care != 0;
            ambiguous |= au & care != 0;
        }
        Logic::from_bit(if known_diff {
            Bit::Zero
        } else if ambiguous {
            Bit::X
        } else {
            Bit::One
        })
    }

    /// `casez` item match: `z` bits on either side are don't-cares, every
    /// other bit must match exactly.
    pub fn casez_match(&self, item: &Logic) -> bool {
        self.same_width(item, "casez");
        self.value
            .iter()
            .zip(&self.unknown)
            .zip(item.value.iter().zip(&item.unknown))
            .all(|((&av, &au), (&bv, &bu))| {
                let care = !((av & au) | (bv & bu));
                ((av ^ bv) | (au ^ bu)) & care == 0
            })
    }

    /// `casex` item match: `x` and `z` bits on either side are don't-cares,
    /// every other bit must match exactly.
    pub fn casex_match(&self, item: &Logic) -> bool {
        self.same_width(item, "casex");
        self.value
            .iter()
            .zip(&self.unknown)
            .zip(item.value.iter().zip(&item.unknown))
            .all(|((&av, &au), (&bv, &bu))| {
                let care = !(au | bu);
                (av ^ bv) & care == 0
            })
    }

    /// Orders two fully known operands, signed when both are signed.
    fn compare(&self, other: &Logic, op: &str) -> Option<Ordering> {
        self.same_width(other, op);
        if self.has_unknown() || other.has_unknown() {
            return None;
        }
        if self.signed && other.signed {
            let a_neg = self.msb() == Bit::One;
            let b_neg = other.msb() == Bit::One;
            if a_neg != b_neg {
                return Some(if a_neg {
                    Ordering::Less
                } else {
                    Ordering::Greater
                });
            }
        }
        // Same sign (or unsigned): two's complement order agrees with
        // unsigned order within one sign.
        Some(cmp_words(&self.value, &other.value))
    }

    /// Wraps a relational verdict as one bit, `x` for an ambiguous one.
    fn relation(&self, other: &Logic, op: &str, f: impl Fn(Ordering) -> bool) -> Self {
        Logic::from_bit(match self.compare(other, op) {
            Some(o) => Bit::from_bool(f(o)),
            None => Bit::X,
        })
    }

    /// `<`; `x` if either operand has `x` or `z` bits.
    pub fn lt(&self, other: &Logic) -> Self {
        self.relation(other, "<", |o| o == Ordering::Less)
    }

    /// `<=`; `x` if either operand has `x` or `z` bits.
    pub fn le(&self, other: &Logic) -> Self {
        self.relation(other, "<=", |o| o != Ordering::Greater)
    }

    /// `>`; `x` if either operand has `x` or `z` bits.
    pub fn gt(&self, other: &Logic) -> Self {
        self.relation(other, ">", |o| o == Ordering::Greater)
    }

    /// `>=`; `x` if either operand has `x` or `z` bits.
    pub fn ge(&self, other: &Logic) -> Self {
        self.relation(other, ">=", |o| o != Ordering::Less)
    }

    // ---- arithmetic ----

    /// Runs a two-state arithmetic kernel on the value planes, or yields
    /// all `x` when either operand has unknown bits.
    fn arith(
        &self,
        other: &Logic,
        op: &str,
        f: impl FnOnce(&[u64], &[u64], bool) -> Option<Vec<u64>>,
    ) -> Self {
        self.same_width(other, op);
        let signed = self.signed && other.signed;
        if self.has_unknown() || other.has_unknown() {
            return Logic::x(self.width).with_signed(signed);
        }
        match f(&self.value, &other.value, signed) {
            Some(mut value) => {
                mask_top(&mut value, self.width);
                Logic::raw(self.width, signed, value, vec![0u64; self.value.len()])
            }
            None => Logic::x(self.width).with_signed(signed),
        }
    }

    /// `+`, modulo `2^width`.
    pub fn add(&self, other: &Logic) -> Self {
        self.arith(other, "+", |a, b, _| {
            let mut out = a.to_vec();
            add_into(&mut out, b);
            Some(out)
        })
    }

    /// `-`, modulo `2^width`.
    pub fn sub(&self, other: &Logic) -> Self {
        self.arith(other, "-", |a, b, _| {
            let mut out = a.to_vec();
            sub_into(&mut out, b);
            Some(out)
        })
    }

    /// Unary minus, modulo `2^width`; all `x` if any bit is unknown.
    pub fn neg(&self) -> Self {
        if self.has_unknown() {
            return Logic::x(self.width).with_signed(self.signed);
        }
        Logic::raw(
            self.width,
            self.signed,
            neg_words(&self.value, self.width),
            vec![0u64; self.value.len()],
        )
    }

    /// `*`, keeping the low `width` bits (which are the same for signed and
    /// unsigned operands).
    pub fn mul(&self, other: &Logic) -> Self {
        self.arith(other, "*", |a, b, _| Some(mul_low(a, b)))
    }

    /// Signed-aware division kernel: `(quotient, remainder)` truncating
    /// toward zero, the remainder taking the sign of the dividend.
    fn divide(a: &[u64], b: &[u64], width: u32, signed: bool) -> Option<(Vec<u64>, Vec<u64>)> {
        if is_zero(b) {
            return None;
        }
        let a_neg = signed && get_bit(a, width - 1);
        let b_neg = signed && get_bit(b, width - 1);
        let abs = |w: &[u64], neg: bool| {
            if neg { neg_words(w, width) } else { w.to_vec() }
        };
        let (q, r) = divrem(&abs(a, a_neg), &abs(b, b_neg));
        let q = if a_neg != b_neg {
            neg_words(&q, width)
        } else {
            q
        };
        let r = if a_neg { neg_words(&r, width) } else { r };
        Some((q, r))
    }

    /// `/`, truncating toward zero when signed; all `x` on division by
    /// zero.
    pub fn div(&self, other: &Logic) -> Self {
        let width = self.width;
        self.arith(other, "/", |a, b, signed| {
            Self::divide(a, b, width, signed).map(|(q, _)| q)
        })
    }

    /// `%`, with the sign of the dividend when signed; all `x` on division
    /// by zero.
    pub fn rem(&self, other: &Logic) -> Self {
        let width = self.width;
        self.arith(other, "%", |a, b, signed| {
            Self::divide(a, b, width, signed).map(|(_, r)| r)
        })
    }

    /// `**`, per IEEE 1364-2005 Table 5-6. The exponent is self-determined,
    /// so it may have any width and keeps its own signedness for deciding
    /// whether it is negative; the result has this value's width and is
    /// signed when both operands are.
    ///
    /// With `b` this value and `e` the exponent: `b ** 0` is 1; `0 ** e` is
    /// 0 for positive `e` and `x` for negative `e`; `1 ** e` is 1;
    /// `(-1) ** e` is 1 or -1 by the parity of `e`; any other base raised to
    /// a negative exponent is 0; otherwise the power is taken modulo
    /// `2^width`. Any unknown bit yields all `x`.
    pub fn pow(&self, exponent: &Logic) -> Self {
        let signed = self.signed && exponent.signed;
        let width = self.width;
        let unknown_result = || Logic::x(width).with_signed(signed);
        if self.has_unknown() || exponent.has_unknown() {
            return unknown_result();
        }
        let one = |w: u32| Logic::from_u64(1, w).with_signed(signed);
        let exp_neg = exponent.signed && exponent.msb() == Bit::One;
        let exp_odd = get_bit(&exponent.value, 0);
        if is_zero(&exponent.value) {
            return one(width);
        }
        let base_neg = signed && self.msb() == Bit::One;
        let base_is_one = cmp_words(&self.value, &[1]) == Ordering::Equal;
        let base_is_minus_one = base_neg
            && self
                .value
                .iter()
                .enumerate()
                .all(|(i, &w)| w == word_mask(i, width));
        if is_zero(&self.value) {
            return if exp_neg {
                unknown_result()
            } else {
                Logic::zero(width).with_signed(signed)
            };
        }
        if base_is_one {
            return one(width);
        }
        if base_is_minus_one {
            return if exp_odd { self.clone() } else { one(width) };
        }
        if exp_neg {
            return Logic::zero(width).with_signed(signed);
        }
        // Square-and-multiply modulo 2^width; the low bits of a two's
        // complement product do not depend on the sign, so the raw words
        // serve for signed bases too.
        let top = highest_bit(&exponent.value).expect("exponent is non-zero");
        let mut acc = vec![0u64; self.value.len()];
        acc[0] = 1;
        for i in (0..=top).rev() {
            acc = mul_low(&acc, &acc);
            if get_bit(&exponent.value, i) {
                acc = mul_low(&acc, &self.value);
            }
            mask_top(&mut acc, width);
        }
        Logic::raw(width, signed, acc, vec![0u64; self.value.len()])
    }
}

impl fmt::Display for Logic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_verilog_literal())
    }
}

impl fmt::Debug for Logic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Logic({self})")
    }
}

// ---------------------------------------------------------------------------
// Literal parsing
// ---------------------------------------------------------------------------

/// Why a literal could not be parsed by [`Logic::parse_verilog`] or
/// [`Logic::parse_vhdl_bit_string`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogicParseError {
    /// The text is empty.
    Empty,
    /// The size (Verilog) or length (VHDL) prefix is not a valid number
    /// (zero, malformed, or too large for a `u32`).
    InvalidSize,
    /// The base letter after `'` (Verilog) or before `"` (VHDL) is not one
    /// of the known bases.
    InvalidBase(char),
    /// A `'` with nothing after it.
    MissingBase,
    /// No digits after the base.
    EmptyValue,
    /// A character that is not a digit of the base, nor `_`, nor an
    /// allowed `x` / `z` marker.
    InvalidDigit(char),
    /// A decimal value mixing `x` or `z` with digits, such as `'d1x`.
    MixedUnknownDecimal,
    /// Stray text: a missing or unbalanced quote, or characters after the
    /// literal.
    Malformed,
    /// A VHDL bit-string length is shorter than its value and the dropped
    /// characters are not the padding the base specifier allows.
    Truncated,
    /// A VHDL decimal bit string does not fit its declared length.
    ValueTooLarge,
}

impl fmt::Display for LogicParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LogicParseError::Empty => f.write_str("empty literal"),
            LogicParseError::InvalidSize => f.write_str("invalid size"),
            LogicParseError::InvalidBase(c) => write!(f, "invalid base `{c}`"),
            LogicParseError::MissingBase => f.write_str("missing base after `'`"),
            LogicParseError::EmptyValue => f.write_str("no digits after the base"),
            LogicParseError::InvalidDigit(c) => write!(f, "invalid digit `{c}`"),
            LogicParseError::MixedUnknownDecimal => {
                f.write_str("a decimal value may be all digits or a single x or z")
            }
            LogicParseError::Malformed => f.write_str("malformed literal"),
            LogicParseError::Truncated => f.write_str(
                "bit string is longer than its length and the dropped bits are not padding",
            ),
            LogicParseError::ValueTooLarge => f.write_str("value does not fit the given length"),
        }
    }
}

impl std::error::Error for LogicParseError {}

/// Number base of a literal's digits.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Base {
    Bin,
    Oct,
    Dec,
    Hex,
}

impl Base {
    /// Bits per digit for the binary-friendly bases.
    fn bits(self) -> u32 {
        match self {
            Base::Bin => 1,
            Base::Oct => 3,
            Base::Hex => 4,
            Base::Dec => 0,
        }
    }

    /// Radix for `char::to_digit`.
    fn radix(self) -> u32 {
        match self {
            Base::Bin => 2,
            Base::Oct => 8,
            Base::Dec => 10,
            Base::Hex => 16,
        }
    }
}

/// Parses a decimal number with `_` separators (no leading `_`) into a
/// `u32`, for size prefixes.
fn parse_size(text: &str) -> Result<u32, LogicParseError> {
    if text.is_empty() || text.starts_with('_') {
        return Err(LogicParseError::InvalidSize);
    }
    let mut n: u32 = 0;
    for c in text.chars() {
        if c == '_' {
            continue;
        }
        let d = c.to_digit(10).ok_or(LogicParseError::InvalidSize)?;
        n = n
            .checked_mul(10)
            .and_then(|n| n.checked_add(d))
            .ok_or(LogicParseError::InvalidSize)?;
    }
    Ok(n)
}

/// Parses decimal digits with `_` separators into little-endian words.
fn parse_decimal(text: &str) -> Result<Vec<u64>, LogicParseError> {
    let mut words = vec![0u64];
    let mut seen = false;
    for c in text.chars() {
        if c == '_' {
            if !seen {
                return Err(LogicParseError::InvalidDigit('_'));
            }
            continue;
        }
        let d = c.to_digit(10).ok_or(LogicParseError::InvalidDigit(c))?;
        mul_small_add(&mut words, 10, u64::from(d));
        seen = true;
    }
    if !seen {
        return Err(LogicParseError::EmptyValue);
    }
    Ok(words)
}

/// One digit of a based literal, as `(value, unknown)` nibbles, plus the
/// bit the standard pads with when it is the leftmost digit.
fn based_digit(c: char, base: Base) -> Result<(u64, u64), LogicParseError> {
    let full = (1u64 << base.bits()) - 1;
    match c {
        'x' | 'X' => Ok((0, full)),
        'z' | 'Z' | '?' => Ok((full, full)),
        _ => c
            .to_digit(base.radix())
            .map(|d| (u64::from(d), 0))
            .ok_or(LogicParseError::InvalidDigit(c)),
    }
}

/// Builds a vector from little-endian words, then fits it to `width` bits:
/// extra high bits are dropped, missing ones are `fill`.
fn fit(value: Vec<u64>, unknown: Vec<u64>, have: u32, width: u32, fill: Bit) -> Logic {
    let words = word_count(width);
    let mut value = value;
    let mut unknown = unknown;
    value.resize(words, 0);
    unknown.resize(words, 0);
    mask_top(&mut value, width);
    mask_top(&mut unknown, width);
    if width > have {
        let (v, u) = fill.planes();
        if v {
            fill_ones(&mut value, have, width);
        }
        if u {
            fill_ones(&mut unknown, have, width);
        }
    }
    Logic::raw(width, false, value, unknown)
}

impl Logic {
    /// Parses a Verilog integer literal (IEEE 1364-2005 §3.5.1).
    ///
    /// Accepted forms: a plain decimal such as `12` (signed, 32 bits, or
    /// more if the value needs them); `[size]'[s]<base><digits>` with base
    /// `b`, `o`, `d` or `h` in either case, such as `8'hff`, `4'b10x1`,
    /// `'d12`, `16'shBEEF`; `_` separators after the first digit; `x`, `z`
    /// and `?` (as `z`) digits, which in a based literal fill one digit's
    /// worth of bits and in a decimal literal must stand alone (`8'dx`);
    /// and white space between the size, the base and the digits.
    ///
    /// The size is the width. A value shorter than the size is padded with
    /// zeros, or with `x` / `z` when its leftmost digit is `x` / `z`; a
    /// longer value is truncated from the left, as the standard allows.
    /// Unsized literals are 32 bits wide, or as wide as the value needs
    /// when that is more (the standard requires "at least 32"). A plain
    /// decimal and an `s` base are signed; other forms are unsigned.
    ///
    /// A leading sign is not part of the literal (`32'sd-5` is an error);
    /// the frontend applies unary minus.
    pub fn parse_verilog(text: &str) -> Result<Logic, LogicParseError> {
        let s = text.trim();
        if s.is_empty() {
            return Err(LogicParseError::Empty);
        }
        let Some(quote) = s.find('\'') else {
            let words = parse_decimal(s)?;
            let needed = highest_bit(&words).map_or(1, |b| b + 1);
            let width = needed.max(32);
            let unknown = vec![0u64; words.len()];
            return Ok(fit(words, unknown, width, width, Bit::Zero).as_signed());
        };
        let size_text = s[..quote].trim_end();
        let size = if size_text.is_empty() {
            None
        } else {
            match parse_size(size_text)? {
                0 => return Err(LogicParseError::InvalidSize),
                n => Some(n),
            }
        };
        let mut rest = &s[quote + 1..];
        let signed = rest.starts_with(['s', 'S']);
        if signed {
            rest = &rest[1..];
        }
        let base = match rest.chars().next() {
            None => return Err(LogicParseError::MissingBase),
            Some('b' | 'B') => Base::Bin,
            Some('o' | 'O') => Base::Oct,
            Some('d' | 'D') => Base::Dec,
            Some('h' | 'H') => Base::Hex,
            Some(c) => return Err(LogicParseError::InvalidBase(c)),
        };
        let digits = rest[1..].trim_start();
        if digits.is_empty() {
            return Err(LogicParseError::EmptyValue);
        }
        if base == Base::Dec {
            let first = digits.chars().next().expect("digits are not empty");
            if let Some(bit @ (Bit::X | Bit::Z)) = Bit::from_char(first) {
                if !digits[first.len_utf8()..].chars().all(|c| c == '_') {
                    return Err(LogicParseError::MixedUnknownDecimal);
                }
                return Ok(Logic::filled(size.unwrap_or(32), bit).with_signed(signed));
            }
            let words = parse_decimal(digits).map_err(|e| match e {
                LogicParseError::InvalidDigit('x' | 'X' | 'z' | 'Z' | '?') => {
                    LogicParseError::MixedUnknownDecimal
                }
                e => e,
            })?;
            let needed = highest_bit(&words).map_or(1, |b| b + 1);
            let width = size.unwrap_or(needed.max(32));
            let unknown = vec![0u64; words.len()];
            return Ok(fit(words, unknown, width, width, Bit::Zero).with_signed(signed));
        }
        if digits.starts_with('_') {
            return Err(LogicParseError::InvalidDigit('_'));
        }
        let mut nibbles: Vec<(u64, u64)> = Vec::new();
        for c in digits.chars().filter(|&c| c != '_') {
            nibbles.push(based_digit(c, base)?);
        }
        let per = base.bits();
        let count = u32::try_from(nibbles.len()).map_err(|_| LogicParseError::InvalidSize)?;
        let have = count.checked_mul(per).ok_or(LogicParseError::InvalidSize)?;
        let words = word_count(have);
        let mut value = vec![0u64; words];
        let mut unknown = vec![0u64; words];
        for (i, &(v, u)) in nibbles.iter().rev().enumerate() {
            let at = u32::try_from(i).expect("digit index fits u32") * per;
            for b in 0..per {
                if (v >> b) & 1 != 0 {
                    set_bit(&mut value, at + b, true);
                }
                if (u >> b) & 1 != 0 {
                    set_bit(&mut unknown, at + b, true);
                }
            }
        }
        let fill = match nibbles[0] {
            (_, 0) => Bit::Zero,
            (0, _) => Bit::X,
            _ => Bit::Z,
        };
        let width = size.unwrap_or(have.max(32));
        Ok(fit(value, unknown, have, width, fill).with_signed(signed))
    }

    /// Parses a VHDL bit-string literal (IEEE 1076-2008 §15.8) into its
    /// `std_ulogic` elements, leftmost first.
    ///
    /// Accepted forms: `B"1010"`, `O"17"`, `X"FF"` and their VHDL-2008
    /// extensions `UB` / `UO` / `UX` (same as the plain forms), `SB` /
    /// `SO` / `SX` (sign-extended) and `D"35"` (decimal), each optionally
    /// preceded by a length such as `8X"F"`. Base letters and hex digits
    /// are accepted in either case; `_` may separate digits. In the
    /// binary-friendly bases a non-digit `std_ulogic` character (`U X Z W
    /// L H -`, either case) stands for one digit's worth of identical
    /// elements, so `X"F-"` is `1111----`.
    ///
    /// With a length, a shorter value is extended on the left with `'0'`
    /// (unsigned and decimal) or with its leftmost element (signed); a
    /// longer value is truncated on the left, which is an error unless the
    /// dropped elements are all `'0'` (unsigned) or all equal to the
    /// leftmost element kept (signed), so that the truncation never changes
    /// the value the string denotes: `7SX"3F"` is `0111111` but `6SX"3F"`
    /// is an error because it would flip the sign. A decimal string without
    /// a length is as wide as its value needs (at least one bit); with a
    /// length it must fit.
    pub fn parse_vhdl_bit_string_std9(text: &str) -> Result<Vec<Std9>, LogicParseError> {
        let s = text.trim();
        if s.is_empty() {
            return Err(LogicParseError::Empty);
        }
        let len_end = s
            .find(|c: char| !(c.is_ascii_digit() || c == '_'))
            .ok_or(LogicParseError::Malformed)?;
        let (len_text, rest) = s.split_at(len_end);
        let length = if len_text.is_empty() {
            None
        } else {
            Some(parse_size(len_text)?)
        };
        let open = rest.find('"').ok_or(LogicParseError::Malformed)?;
        let spec = &rest[..open];
        let (signed, base) = match spec.to_ascii_lowercase().as_str() {
            "b" | "ub" => (false, Base::Bin),
            "o" | "uo" => (false, Base::Oct),
            "x" | "ux" => (false, Base::Hex),
            "sb" => (true, Base::Bin),
            "so" => (true, Base::Oct),
            "sx" => (true, Base::Hex),
            "d" => (false, Base::Dec),
            "" => return Err(LogicParseError::MissingBase),
            _ => {
                return Err(LogicParseError::InvalidBase(
                    spec.chars().next().expect("non-empty specifier"),
                ));
            }
        };
        let body = rest[open + 1..]
            .strip_suffix('"')
            .ok_or(LogicParseError::Malformed)?;
        if body.contains('"') {
            return Err(LogicParseError::Malformed);
        }
        // `_` only between two digits.
        if body.starts_with('_') || body.ends_with('_') || body.contains("__") {
            return Err(LogicParseError::InvalidDigit('_'));
        }
        let chars: Vec<char> = body.chars().filter(|&c| c != '_').collect();

        if base == Base::Dec {
            let words = parse_decimal(body)?;
            let needed = highest_bit(&words).map_or(1, |b| b + 1);
            let width = match length {
                Some(l) if l < needed => return Err(LogicParseError::ValueTooLarge),
                Some(l) => l,
                None => needed,
            };
            return Ok((0..width)
                .rev()
                .map(|i| {
                    if get_bit(&words, i) {
                        Std9::One
                    } else {
                        Std9::Zero
                    }
                })
                .collect());
        }

        let per = base.bits();
        let mut out: Vec<Std9> = Vec::with_capacity(chars.len() * per as usize);
        for c in chars {
            match c.to_digit(base.radix()) {
                Some(d) => {
                    for b in (0..per).rev() {
                        out.push(if (d >> b) & 1 != 0 {
                            Std9::One
                        } else {
                            Std9::Zero
                        });
                    }
                }
                None => {
                    let s = Std9::from_char(c).ok_or(LogicParseError::InvalidDigit(c))?;
                    out.extend(std::iter::repeat_n(s, per as usize));
                }
            }
        }
        let Some(length) = length else {
            return Ok(out);
        };
        let have = u32::try_from(out.len()).map_err(|_| LogicParseError::InvalidSize)?;
        if length > have {
            let pad = if signed {
                out.first().copied().unwrap_or(Std9::Zero)
            } else {
                Std9::Zero
            };
            let mut padded = vec![pad; (length - have) as usize];
            padded.extend(out);
            return Ok(padded);
        }
        let drop = (have - length) as usize;
        let keep = out.split_off(drop);
        let reference = if signed {
            keep.first().copied().unwrap_or(Std9::Zero)
        } else {
            Std9::Zero
        };
        if out.iter().any(|&s| s != reference) {
            return Err(LogicParseError::Truncated);
        }
        Ok(keep)
    }

    /// Parses a VHDL bit-string literal into an unsigned four-state vector.
    ///
    /// The forms and extension rules are those of
    /// [`Logic::parse_vhdl_bit_string_std9`]; the nine-state elements fold
    /// through [`Std9::to_bit`], so `L` and `H` become `0` and `1` and `U`,
    /// `W` and `-` become `x`. The leftmost character of the string is the
    /// MSB. The result is unsigned even for the `S` bases, since a VHDL
    /// bit string is an array, not a number; signedness there comes from
    /// the target type.
    pub fn parse_vhdl_bit_string(text: &str) -> Result<Logic, LogicParseError> {
        let mut elems = Logic::parse_vhdl_bit_string_std9(text)?;
        elems.reverse();
        Ok(Logic::from_std9(&elems))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Bit::{One, X, Z, Zero};

    fn v(text: &str) -> Logic {
        Logic::parse_verilog(text).unwrap_or_else(|e| panic!("{text}: {e}"))
    }

    fn bits(text: &str) -> Vec<Bit> {
        text.chars()
            .rev()
            .map(|c| Bit::from_char(c).unwrap())
            .collect()
    }

    /// A deterministic pseudo-random four-state vector.
    fn pseudo(width: u32, seed: u64) -> Logic {
        let mut state = seed;
        let mut next = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            state >> 33
        };
        let bits: Vec<Bit> = (0..width)
            .map(|_| match next() % 4 {
                0 => Zero,
                1 => One,
                2 => X,
                _ => Z,
            })
            .collect();
        Logic::from_bits(&bits)
    }

    // ---- Bit and Std9 ----

    #[test]
    fn bit_tables() {
        let all = [Zero, One, X, Z];
        for a in all {
            for b in all {
                let expect_and = match (a, b) {
                    (Zero, _) | (_, Zero) => Zero,
                    (One, One) => One,
                    _ => X,
                };
                let expect_or = match (a, b) {
                    (One, _) | (_, One) => One,
                    (Zero, Zero) => Zero,
                    _ => X,
                };
                let expect_xor = match (a, b) {
                    (Zero, Zero) | (One, One) => Zero,
                    (Zero, One) | (One, Zero) => One,
                    _ => X,
                };
                assert_eq!(a.and(b), expect_and, "{a} & {b}");
                assert_eq!(a.or(b), expect_or, "{a} | {b}");
                assert_eq!(a.xor(b), expect_xor, "{a} ^ {b}");
            }
        }
        assert_eq!(!Zero, One);
        assert_eq!(!One, Zero);
        assert_eq!(!X, X);
        assert_eq!(!Z, X);
        assert_eq!(format!("{Zero}{One}{X}{Z}"), "01xz");
        assert_eq!(Bit::from_char('?'), Some(Z));
        assert_eq!(Bit::from_char('2'), None);
        assert_eq!(Bit::from(true), One);
        assert_eq!(One.to_bool(), Some(true));
        assert_eq!(Z.to_bool(), None);
    }

    #[test]
    fn std9_conversions() {
        assert_eq!(
            Std9::ALL.map(Std9::to_char),
            ['U', 'X', '0', '1', 'Z', 'W', 'L', 'H', '-']
        );
        for (i, s) in Std9::ALL.iter().enumerate() {
            assert_eq!(s.index(), i);
            assert_eq!(Std9::from_char(s.to_char()), Some(*s));
            assert_eq!(s.to_string(), s.to_char().to_string());
        }
        assert_eq!(Std9::from_char('l'), Some(Std9::L));
        assert_eq!(Std9::from_char('2'), None);
        let folded: Vec<Bit> = Std9::ALL.iter().map(|&s| Bit::from(s)).collect();
        assert_eq!(folded, [X, X, Zero, One, Z, X, Zero, One, X]);
        assert_eq!(Std9::from(Zero), Std9::Zero);
        assert_eq!(Std9::from(One), Std9::One);
        assert_eq!(Std9::from(X), Std9::X);
        assert_eq!(Std9::from(Z), Std9::Z);
    }

    #[test]
    fn std9_resolution_matches_ieee_1164() {
        // IEEE 1164-1993 resolution_table, rows and columns in `U X 0 1 Z W L H -` order.
        const TABLE: [&str; 9] = [
            "UUUUUUUUU", // U
            "UXXXXXXXX", // X
            "UX0X0000X", // 0
            "UXX11111X", // 1
            "UX01ZWLHX", // Z
            "UX01WWWWX", // W
            "UX01LWLWX", // L
            "UX01HWWHX", // H
            "UXXXXXXXX", // -
        ];
        for (i, a) in Std9::ALL.iter().enumerate() {
            for (j, b) in Std9::ALL.iter().enumerate() {
                let expected = Std9::from_char(TABLE[i].chars().nth(j).unwrap()).unwrap();
                assert_eq!(Std9::resolve(*a, *b), expected, "resolve({a}, {b})");
                assert_eq!(Std9::resolve(*a, *b), Std9::resolve(*b, *a));
            }
        }
        assert_eq!(Std9::resolve_all([]), Std9::Z);
        assert_eq!(Std9::resolve_all([Std9::L, Std9::One, Std9::Z]), Std9::One);
        assert_eq!(Std9::resolve_all([Std9::L, Std9::H]), Std9::W);
        assert_eq!(Std9::resolve_all([Std9::One, Std9::Zero, Std9::U]), Std9::U);
    }

    // ---- construction and inspection ----

    #[test]
    fn constructors_and_accessors() {
        for width in [0, 1, 63, 64, 65, 128, 129, 200] {
            let z = Logic::zero(width);
            assert_eq!(z.width(), width);
            assert!(z.is_zero());
            assert!(!z.is_signed());
            assert_eq!(z.to_u64(), Some(0));
            let o = Logic::ones(width);
            assert!(o.is_fully_known());
            assert_eq!(o.bits(), vec![One; width as usize]);
            assert_eq!(o.value_words().len(), word_count(width));
            let x = Logic::x(width);
            assert!(width == 0 || x.has_unknown());
            assert_eq!(x.to_u64(), if width == 0 { Some(0) } else { None });
            let zz = Logic::z(width);
            assert_eq!(zz.bits(), vec![Z; width as usize]);
            if width > 0 {
                assert_eq!(x.msb(), X);
                assert_eq!(zz.msb(), Z);
                assert_eq!(o.msb(), One);
            }
            // Round-trips through the bit list.
            assert_eq!(Logic::from_bits(&zz.bits()), zz);
            assert_eq!(Logic::from_bits(&o.bits()), o);
        }
        assert_eq!(Logic::from_u64(0xff, 8).to_u64(), Some(0xff));
        assert_eq!(Logic::from_u64(0x1ff, 8).to_u64(), Some(0xff));
        assert_eq!(Logic::from_u64(u64::MAX, 65).to_u64(), Some(u64::MAX));
        assert_eq!(Logic::from_bool(true), Logic::from_u64(1, 1));
        assert_eq!(Logic::from_bit(Z), Logic::z(1));
        assert_eq!(
            Logic::from_std9(&[Std9::H, Std9::L, Std9::W]),
            Logic::from_bits(&[One, Zero, X])
        );
        let mut l = Logic::zero(70);
        l.set_bit(69, One);
        l.set_bit(3, Z);
        assert_eq!(l.bit(69), One);
        assert_eq!(l.bit(3), Z);
        assert_eq!(l.bit(64), Zero);
        assert_eq!(l.get(70), None);
        assert_eq!(l.msb(), One);
        assert_eq!(l.to_u64(), None);
        l.set_bit(3, Zero);
        assert_eq!(l.to_u64(), None, "bit 69 does not fit u64");
        assert_eq!(l.slice(69, 64).to_u64(), Some(0b100000));
        assert_eq!(Logic::zero(0).msb(), Zero);
    }

    #[test]
    fn signed_integers() {
        assert_eq!(Logic::from_i64(-1, 8), Logic::ones(8).as_signed());
        assert_eq!(Logic::from_i64(-1, 130), Logic::ones(130).as_signed());
        assert_eq!(Logic::from_i64(-2, 4).to_binary_string(), "1110");
        assert_eq!(Logic::from_i64(5, 4).to_i64(), Some(5));
        assert_eq!(Logic::from_i64(-5, 4).to_i64(), Some(-5));
        assert_eq!(Logic::from_i64(-5, 64).to_i64(), Some(-5));
        assert_eq!(Logic::from_i64(-5, 100).to_i64(), Some(-5));
        assert_eq!(Logic::from_i64(i64::MIN, 64).to_i64(), Some(i64::MIN));
        assert_eq!(Logic::from_i64(i64::MIN, 200).to_i64(), Some(i64::MIN));
        assert_eq!(Logic::from_i64(-5, 100).to_u64(), None);
        assert_eq!(Logic::from_i64(-5, 8).to_u64(), Some(0xfb));
        assert_eq!(Logic::ones(8).to_i64(), Some(255));
        assert_eq!(Logic::ones(64).to_i64(), None);
        assert_eq!(Logic::ones(64).as_signed().to_i64(), Some(-1));
        assert_eq!(Logic::ones(65).as_signed().to_i64(), Some(-1));
        assert_eq!(Logic::from_u64(1, 65).shl(64).as_signed().to_i64(), None);
        assert_eq!(Logic::x(8).as_signed().to_i64(), None);
        assert_eq!(Logic::zero(0).to_i64(), Some(0));
        assert!(Logic::from_i64(-1, 3).is_negative());
        assert!(!Logic::ones(3).is_negative());
        assert!(!Logic::x(3).as_signed().is_negative());
        assert!(
            Logic::from_i64(1, 3)
                .as_unsigned()
                .with_signed(true)
                .is_signed()
        );
    }

    // ---- Verilog literals ----

    #[test]
    fn parses_verilog_literals() {
        let l = v("8'hff");
        assert_eq!(
            (l.width(), l.is_signed(), l.to_u64()),
            (8, false, Some(0xff))
        );
        assert_eq!(v("4'b10x1").bits(), bits("10x1"));
        assert_eq!(v("4'b1?01").bits(), bits("1z01"));
        let d = v("'d12");
        assert_eq!(
            (d.width(), d.is_signed(), d.to_u64()),
            (32, false, Some(12))
        );
        let plain = v("12");
        assert_eq!(
            (plain.width(), plain.is_signed(), plain.to_u64()),
            (32, true, Some(12))
        );
        assert_eq!(v(" 12 "), plain);
        let s = v("16'shBEEF");
        assert_eq!(
            (s.width(), s.is_signed(), s.to_u64()),
            (16, true, Some(0xbeef))
        );
        assert_eq!(s.to_i64(), Some(-16657));
        assert_eq!(v("8'SB1010").to_binary_string(), "00001010");
        assert_eq!(v("8'O17").to_u64(), Some(0o17));
        assert_eq!(v("8'd1_2").to_u64(), Some(12));
        assert_eq!(v("1_6'hff_ff").to_u64(), Some(0xffff));
        assert_eq!(v("8 'h ff").to_u64(), Some(0xff));
        // Padding: zeros, or x / z when the leftmost digit is x / z.
        assert_eq!(v("8'b1").to_binary_string(), "00000001");
        assert_eq!(v("8'hx").to_binary_string(), "xxxxxxxx");
        assert_eq!(v("8'hz").to_binary_string(), "zzzzzzzz");
        assert_eq!(v("8'hzf").to_binary_string(), "zzzz1111");
        assert_eq!(v("8'hxf").to_binary_string(), "xxxx1111");
        assert_eq!(v("8'h1x").to_binary_string(), "0001xxxx");
        assert_eq!(v("8'oz7").to_binary_string(), "zzzzz111");
        assert_eq!(v("6'ox1").to_binary_string(), "xxx001");
        assert_eq!(v("8'b?").to_binary_string(), "zzzzzzzz");
        assert_eq!(v("8'dx").to_binary_string(), "xxxxxxxx");
        assert_eq!(v("8'dz_").to_binary_string(), "zzzzzzzz");
        assert_eq!(v("'dx").width(), 32);
        assert_eq!(v("'hx").to_binary_string(), "x".repeat(32));
        // Truncation from the left.
        assert_eq!(v("4'hff").to_u64(), Some(0xf));
        assert_eq!(v("4'hxf").to_binary_string(), "1111");
        assert_eq!(v("3'b1010").to_binary_string(), "010");
        assert_eq!(v("8'd256").to_u64(), Some(0));
        // Wide values.
        let wide = v("128'hffff_ffff_ffff_ffff_ffff_ffff_ffff_ffff");
        assert_eq!(wide, Logic::ones(128));
        assert_eq!(
            v("128'd340282366920938463463374607431768211455"),
            Logic::ones(128)
        );
        assert_eq!(
            v("128'd18446744073709551616"),
            Logic::from_u64(1, 128).shl(64)
        );
        assert_eq!(
            v("65'h1_0000_0000_0000_0000"),
            Logic::from_u64(1, 65).shl(64)
        );
        assert_eq!(v("64'hffff_ffff_ffff_ffff").to_u64(), Some(u64::MAX));
        // Unsized literals grow past 32 bits when the value needs it.
        let big = v("4294967296");
        assert_eq!(
            (big.width(), big.is_signed(), big.to_u64()),
            (33, true, Some(1 << 32))
        );
        assert_eq!(v("'hf_ffff_ffff").width(), 36);
        assert_eq!(v("'h0").width(), 32);
        assert_eq!(v("0").width(), 32);
    }

    #[test]
    fn rejects_bad_verilog_literals() {
        use LogicParseError::*;
        let e = |t: &str| Logic::parse_verilog(t).unwrap_err();
        assert_eq!(e(""), Empty);
        assert_eq!(e("   "), Empty);
        assert_eq!(e("32'sd-5"), InvalidDigit('-'));
        assert_eq!(e("-5"), InvalidDigit('-'));
        assert_eq!(e("8'"), MissingBase);
        assert_eq!(e("8'h"), EmptyValue);
        assert_eq!(e("8's"), MissingBase);
        assert_eq!(e("8'q1"), InvalidBase('q'));
        assert_eq!(e("8'hg"), InvalidDigit('g'));
        assert_eq!(e("8'b2"), InvalidDigit('2'));
        assert_eq!(e("8'o8"), InvalidDigit('8'));
        assert_eq!(e("8'd1x"), MixedUnknownDecimal);
        assert_eq!(e("8'dx1"), MixedUnknownDecimal);
        assert_eq!(e("8'dxz"), MixedUnknownDecimal);
        assert_eq!(e("0'd1"), InvalidSize);
        assert_eq!(e("_8'd1"), InvalidSize);
        assert_eq!(e("99999999999'd1"), InvalidSize);
        assert_eq!(e("8'h_ff"), InvalidDigit('_'));
        assert_eq!(e("8'd_1"), InvalidDigit('_'));
        assert_eq!(e("_1"), InvalidDigit('_'));
        assert_eq!(e("1.5"), InvalidDigit('.'));
        assert_eq!(e("8'h ff zz"), InvalidDigit(' '));
        assert_eq!(e("'"), MissingBase);
        assert_eq!(e("8'd"), EmptyValue);
        assert!(!e("8'hg").to_string().is_empty());
    }

    #[test]
    fn verilog_literal_display_round_trips() {
        let cases = [
            ("8'hff", "8'hff"),
            ("4'b10x1", "4'b10x1"),
            ("8'hx", "8'hxx"),
            ("8'hz", "8'hzz"),
            ("8'hzf", "8'hzf"),
            ("8'shff", "8'shff"),
            ("5'h1f", "5'h1f"),
            ("5'b1x111", "5'b1x111"),
            ("5'bx1111", "5'hxf"),
            ("6'bzz1111", "6'hzf"),
            ("6'bz01111", "6'bz01111"),
            ("65'h1_0000_0000_0000_0000", "65'h10000000000000000"),
            ("1'b1", "1'h1"),
            ("1'bx", "1'hx"),
            ("12", "32'sh0000000c"),
        ];
        for (input, expected) in cases {
            let l = v(input);
            assert_eq!(l.to_verilog_literal(), expected, "{input}");
            assert_eq!(l.to_string(), expected);
            assert_eq!(v(expected), l, "round trip of {expected}");
            assert_eq!(format!("{l:?}"), format!("Logic({expected})"));
        }
        assert_eq!(Logic::zero(0).to_string(), "0'h0");
        for seed in 1..20 {
            let l = pseudo(70, seed);
            assert_eq!(v(&l.to_verilog_literal()), l);
        }
    }

    // ---- VHDL bit strings ----

    #[test]
    fn parses_vhdl_bit_strings() {
        let p = |t: &str| Logic::parse_vhdl_bit_string(t).unwrap_or_else(|e| panic!("{t}: {e}"));
        let s = |t: &str| p(t).to_binary_string();
        assert_eq!(s(r#"x"FF""#), "11111111");
        assert_eq!(s(r#"X"ff""#), "11111111");
        assert_eq!(s(r#"b"1010""#), "1010");
        assert_eq!(s(r#"o"17""#), "001111");
        assert_eq!(s(r#"B"1111_1111_1111""#), "111111111111");
        assert_eq!(s(r#"x"1_0""#), "00010000");
        assert_eq!(s(r#"b"1x0z""#), "1x0z");
        assert_eq!(s(r#"B"L1H""#), "011");
        assert_eq!(s(r#"X"F-""#), "1111xxxx");
        assert!(!p(r#"x"FF""#).is_signed());
        // VHDL-2008 lengths and sign extension.
        assert_eq!(s(r#"8x"F""#), "00001111");
        assert_eq!(s(r#"8ux"F""#), "00001111");
        assert_eq!(s(r#"8sx"F""#), "11111111");
        assert_eq!(s(r#"8sx"7""#), "00000111");
        assert_eq!(s(r#"12ub"X1""#), "0000000000x1");
        assert_eq!(s(r#"12sb"X1""#), "xxxxxxxxxxx1");
        assert_eq!(s(r#"12ux"F-""#), "00001111xxxx");
        assert_eq!(s(r#"12sx"F-""#), "11111111xxxx");
        assert_eq!(s(r#"12SX"Z1""#), "zzzzzzzz0001");
        assert_eq!(s(r#"7ux"3F""#), "0111111");
        assert_eq!(s(r#"7sx"3F""#), "0111111");
        assert_eq!(s(r#"6sx"FF""#), "111111");
        assert_eq!(s(r#"12sx"WWWWWW""#), "xxxxxxxxxxxx");
        assert_eq!(s(r#"12ux"000WWW""#), "xxxxxxxxxxxx");
        assert_eq!(s(r#"12ux"000111""#), "000100010001");
        assert_eq!(s(r#"2ub"000""#), "00");
        // Decimal.
        assert_eq!(s(r#"d"35""#), "100011");
        assert_eq!(s(r#"D"0""#), "0");
        assert_eq!(s(r#"12d"13""#), "000000001101");
        assert_eq!(
            s(r#"d"18446744073709551616""#),
            format!("1{}", "0".repeat(64))
        );
        assert_eq!(p(r#"x"""#).width(), 0);
        assert_eq!(p(r#"4x"""#), Logic::zero(4));
        // Nine-state elements survive in the std9 form.
        assert_eq!(
            Logic::parse_vhdl_bit_string_std9(r#"o"2W""#).unwrap(),
            [Std9::Zero, Std9::One, Std9::Zero, Std9::W, Std9::W, Std9::W]
        );
        assert_eq!(
            Logic::parse_vhdl_bit_string_std9(r#"4sb"H""#).unwrap(),
            [Std9::H; 4]
        );
    }

    #[test]
    fn rejects_bad_vhdl_bit_strings() {
        use LogicParseError::*;
        let e = |t: &str| Logic::parse_vhdl_bit_string(t).unwrap_err();
        assert_eq!(e(""), Empty);
        assert_eq!(e(r#""1010""#), MissingBase);
        assert_eq!(e(r#"q"1""#), InvalidBase('q'));
        assert_eq!(e(r#"sd"1""#), InvalidBase('s'));
        assert_eq!(e(r#"x"1"#), Malformed);
        assert_eq!(e(r#"x"1"2""#), Malformed);
        assert_eq!(e("x1"), Malformed);
        assert_eq!(e("8"), Malformed);
        assert_eq!(e(r#"x"G""#), InvalidDigit('G'));
        assert_eq!(e(r#"b"2""#), InvalidDigit('2'));
        assert_eq!(e(r#"d"1x""#), InvalidDigit('x'));
        assert_eq!(e(r#"x"_1""#), InvalidDigit('_'));
        assert_eq!(e(r#"x"1_""#), InvalidDigit('_'));
        assert_eq!(e(r#"x"1__1""#), InvalidDigit('_'));
        assert_eq!(e(r#"7ux"FF""#), Truncated);
        assert_eq!(e(r#"6sx"3F""#), Truncated, "sign would change");
        assert_eq!(e(r#"7sx"7F""#), Truncated);
        assert_eq!(e(r#"6sx"2F""#), Truncated);
        assert_eq!(e(r#"3d"13""#), ValueTooLarge);
        assert_eq!(e(r#"99999999999x"1""#), InvalidSize);
        assert_eq!(e(r#"d"""#), EmptyValue);
    }

    // ---- structural operations ----

    #[test]
    fn slice_concat_replicate() {
        let a = v("8'hab");
        assert_eq!(a.slice(7, 4), v("4'ha"));
        assert_eq!(a.slice(3, 0), v("4'hb"));
        assert_eq!(a.slice(7, 0), a);
        assert_eq!(a.slice(5, 5), Logic::from_bool(true));
        assert!(!a.clone().as_signed().slice(7, 0).is_signed());
        let wide = v("130'h3_0000_0000_0000_0000_0000_0000_0000_0005");
        assert_eq!(wide.slice(129, 128), v("2'b11"));
        assert_eq!(
            wide.slice(129, 1),
            v("129'h1_8000_0000_0000_0000_0000_0000_0000_0002")
        );
        assert_eq!(wide.slice(66, 60), Logic::zero(7));
        assert_eq!(wide.slice(2, 0), v("3'd5"));
        // Concatenation puts `self` on top and yields unsigned.
        let c = v("4'ha").as_signed().concat(&v("4'hb").as_signed());
        assert_eq!(c, v("8'hab"));
        assert!(!c.is_signed());
        assert_eq!(v("4'hx").concat(&v("4'hz")), v("8'hxz"));
        let big = v("60'hfff_ffff_ffff_ffff").concat(&v("70'h1"));
        assert_eq!(big.width(), 130);
        assert_eq!(big.slice(129, 70), Logic::ones(60));
        assert_eq!(big.slice(69, 0), v("70'h1"));
        assert_eq!(
            Logic::concat_all([&v("2'b10"), &v("3'b011"), &v("1'bz")]),
            v("6'b10011z")
        );
        assert_eq!(Logic::concat_all([]), Logic::zero(0));
        assert_eq!(Logic::zero(0).concat(&a), a);
        assert_eq!(a.concat(&Logic::zero(0)), a);
        // Replication.
        assert_eq!(v("2'b10").replicate(3), v("6'b101010"));
        assert_eq!(v("33'h1_0000_0001").replicate(3).to_binary_string(), {
            let unit = format!("1{}1", "0".repeat(31));
            unit.repeat(3)
        });
        assert_eq!(v("1'bx").replicate(70), Logic::x(70));
        assert_eq!(a.replicate(0), Logic::zero(0));
        assert_eq!(a.replicate(1), a);
    }

    #[test]
    fn resize_extends_and_truncates() {
        assert_eq!(v("4'hf").resize(8), v("8'h0f"));
        assert_eq!(v("4'shf").resize(8), v("8'shff"));
        assert_eq!(v("4'sh7").resize(8), v("8'sh07"));
        assert_eq!(v("8'hab").resize(4), v("4'hb"));
        assert_eq!(v("8'shab").resize(4), v("4'shb"));
        assert_eq!(v("8'hab").resize(8), v("8'hab"));
        // Unknown vectors: zero-extended when unsigned, MSB-replicated when signed.
        assert_eq!(Logic::x(4).resize(8), v("8'h0x"));
        assert_eq!(Logic::x(4).as_signed().resize(8), Logic::x(8).as_signed());
        assert_eq!(Logic::z(4).as_signed().resize(8), Logic::z(8).as_signed());
        assert_eq!(Logic::z(4).resize(8), v("8'h0z"));
        assert_eq!(v("4'sb0zzz").resize(8), v("8'sb0000_0zzz"));
        assert_eq!(v("4'sbz0x1").resize(8), v("8'sbzzzz_z0x1"));
        // Across word boundaries.
        assert_eq!(
            v("64'shffff_ffff_ffff_ffff").resize(200),
            Logic::ones(200).as_signed()
        );
        assert_eq!(
            v("64'hffff_ffff_ffff_ffff").resize(200).to_u64(),
            Some(u64::MAX)
        );
        assert_eq!(
            Logic::x(64).as_signed().resize(130),
            Logic::x(130).as_signed()
        );
        assert_eq!(Logic::x(130).resize(64), Logic::x(64));
        assert_eq!(Logic::ones(130).as_signed().resize(3), v("3'sb111"));
        assert_eq!(
            Logic::zero(0).as_signed().resize(5),
            Logic::zero(5).as_signed()
        );
        assert_eq!(Logic::ones(5).resize(0), Logic::zero(0));
    }

    // ---- bitwise ----

    #[test]
    fn bitwise_matches_bit_tables() {
        for width in [1, 64, 65, 128, 200] {
            for seed in 1..6 {
                let a = pseudo(width, seed);
                let b = pseudo(width, seed + 100);
                let per_bit = |f: fn(Bit, Bit) -> Bit| {
                    Logic::from_bits(
                        &a.bits()
                            .iter()
                            .zip(b.bits())
                            .map(|(&x, y)| f(x, y))
                            .collect::<Vec<_>>(),
                    )
                };
                assert_eq!(a.and(&b), per_bit(Bit::and), "and {width}/{seed}");
                assert_eq!(a.or(&b), per_bit(Bit::or), "or {width}/{seed}");
                assert_eq!(a.xor(&b), per_bit(Bit::xor), "xor {width}/{seed}");
                assert_eq!(a.xnor(&b), per_bit(|x, y| !x.xor(y)), "xnor {width}/{seed}");
                let not: Vec<Bit> = a.bits().iter().map(|&b| !b).collect();
                assert_eq!(a.not(), Logic::from_bits(&not), "not {width}/{seed}");
                a.and(&b).check();
            }
        }
        assert_eq!(v("4'b01xz").and(&v("4'b1111")), v("4'b01xx"));
        assert_eq!(v("4'b01xz").or(&v("4'b0000")), v("4'b01xx"));
        assert_eq!(v("4'b01xz").and(&v("4'b0000")), v("4'b0000"));
        assert_eq!(v("4'b01xz").or(&v("4'b1111")), v("4'b1111"));
        assert_eq!(v("4'b01xz").xor(&v("4'b1111")), v("4'b10xx"));
        assert_eq!(v("4'b01xz").not(), v("4'b10xx"));
        assert!(v("4'sh1").and(&v("4'sh3")).is_signed());
        assert!(!v("4'sh1").and(&v("4'h3")).is_signed());
        assert_eq!(Logic::ones(64).not(), Logic::zero(64));
        assert_eq!(Logic::zero(65).not(), Logic::ones(65));
    }

    #[test]
    #[should_panic(expected = "differ in width")]
    fn bitwise_width_mismatch_panics() {
        let _ = v("4'h1").and(&v("8'h1"));
    }

    #[test]
    fn reductions() {
        assert_eq!(Logic::ones(100).reduce_and(), Logic::from_bool(true));
        assert_eq!(Logic::ones(100).reduce_or(), Logic::from_bool(true));
        assert_eq!(Logic::ones(100).reduce_xor(), Logic::from_bool(false));
        assert_eq!(Logic::ones(101).reduce_xor(), Logic::from_bool(true));
        assert_eq!(Logic::zero(100).reduce_and(), Logic::from_bool(false));
        assert_eq!(Logic::zero(100).reduce_or(), Logic::from_bool(false));
        assert_eq!(Logic::zero(100).reduce_nor(), Logic::from_bool(true));
        assert_eq!(Logic::zero(100).reduce_nand(), Logic::from_bool(true));
        assert_eq!(Logic::zero(100).reduce_xnor(), Logic::from_bool(true));
        let mut one_x = Logic::ones(100);
        one_x.set_bit(77, X);
        assert_eq!(one_x.reduce_and(), Logic::from_bit(X));
        assert_eq!(one_x.reduce_or(), Logic::from_bool(true));
        assert_eq!(one_x.reduce_xor(), Logic::from_bit(X));
        one_x.set_bit(3, Zero);
        assert_eq!(one_x.reduce_and(), Logic::from_bool(false));
        let mut zero_z = Logic::zero(100);
        zero_z.set_bit(99, Z);
        assert_eq!(zero_z.reduce_or(), Logic::from_bit(X));
        assert_eq!(zero_z.reduce_and(), Logic::from_bool(false));
        assert_eq!(Logic::zero(0).reduce_and(), Logic::from_bool(true));
        assert_eq!(Logic::zero(0).reduce_or(), Logic::from_bool(false));
    }

    #[test]
    fn logical_operators() {
        let t = v("4'b1x00");
        let f = v("4'b0000");
        let u = v("4'b0x00");
        assert_eq!(t.truth(), One);
        assert_eq!(f.truth(), Zero);
        assert_eq!(u.truth(), X);
        assert_eq!(t.logical_and(&t), Logic::from_bool(true));
        assert_eq!(t.logical_and(&f), Logic::from_bool(false));
        assert_eq!(u.logical_and(&f), Logic::from_bool(false));
        assert_eq!(u.logical_and(&t), Logic::from_bit(X));
        assert_eq!(u.logical_or(&t), Logic::from_bool(true));
        assert_eq!(u.logical_or(&f), Logic::from_bit(X));
        assert_eq!(f.logical_or(&f), Logic::from_bool(false));
        assert_eq!(t.logical_not(), Logic::from_bool(false));
        assert_eq!(f.logical_not(), Logic::from_bool(true));
        assert_eq!(u.logical_not(), Logic::from_bit(X));
        // Operands of different widths are fine: each reduces on its own.
        assert_eq!(v("1'b1").logical_and(&v("70'h1")), Logic::from_bool(true));
    }

    // ---- shifts ----

    #[test]
    fn shifts() {
        let a = v("8'b1011_0001");
        assert_eq!(a.shl(1), v("8'b0110_0010"));
        assert_eq!(a.shl(0), a);
        assert_eq!(a.shl(8), Logic::zero(8));
        assert_eq!(a.shl(200), Logic::zero(8));
        assert_eq!(a.shr(4), v("8'b0000_1011"));
        assert_eq!(a.shr(8), Logic::zero(8));
        assert_eq!(a.sshr(4), v("8'b0000_1011"), ">>> on unsigned is logical");
        assert_eq!(a.clone().as_signed().sshr(4), v("8'sb1111_1011"));
        assert_eq!(a.clone().as_signed().sshr(7), v("8'sb1111_1111"));
        assert_eq!(a.clone().as_signed().sshr(8), v("8'sb1111_1111"));
        assert_eq!(a.clone().as_signed().sshr(100), v("8'sb1111_1111"));
        assert_eq!(v("8'sb0111_0000").sshr(4), v("8'sb0000_0111"));
        assert_eq!(v("8'sbx111_0000").sshr(4), v("8'sbxxxx_x111"));
        assert_eq!(v("8'sbz111_0000").sshr(4), v("8'sbzzzz_z111"));
        assert_eq!(v("8'b1x0z_0000").shr(4), v("8'b0000_1x0z"));
        assert_eq!(v("8'b0000_1x0z").shl(4), v("8'b1x0z_0000"));
        // Across word boundaries.
        let one = Logic::from_u64(1, 130);
        assert_eq!(one.shl(64).bit(64), One);
        assert_eq!(one.shl(129).bit(129), One);
        assert_eq!(one.shl(129).shr(129), one);
        assert_eq!(one.shl(130), Logic::zero(130));
        assert_eq!(one.shl(65).shr(1), one.shl(64));
        assert_eq!(
            Logic::ones(130).shl(70).shr(70),
            Logic::ones(60).resize(130)
        );
        assert_eq!(
            Logic::ones(130).as_signed().shl(129).sshr(129),
            Logic::ones(130).as_signed()
        );
        assert_eq!(
            Logic::ones(128).shl(1).to_binary_string(),
            format!("{}0", "1".repeat(127))
        );
        assert_eq!(Logic::ones(64).shr(63).to_u64(), Some(1));
        assert_eq!(Logic::ones(64).shl(63).to_u64(), Some(1 << 63));
        // Vector amounts.
        assert_eq!(a.shl_by(&v("3'd1")), a.shl(1));
        assert_eq!(a.shr_by(&v("3'd4")), a.shr(4));
        assert_eq!(
            a.clone().as_signed().sshr_by(&v("3'd4")),
            a.clone().as_signed().sshr(4)
        );
        assert_eq!(a.shl_by(&v("3'bx01")), Logic::x(8));
        assert_eq!(
            a.clone().as_signed().shr_by(&v("1'bz")),
            Logic::x(8).as_signed()
        );
        assert_eq!(a.shl_by(&v("70'h1_0000_0000_0000_0000")), Logic::zero(8));
        assert_eq!(a.shl_by(&v("64'hffff_ffff_ffff_ffff")), Logic::zero(8));
    }

    // ---- equality and relations ----

    #[test]
    fn equality() {
        let t = Logic::from_bool(true);
        let f = Logic::from_bool(false);
        let x = Logic::from_bit(X);
        assert_eq!(v("8'hab").eq(&v("8'hab")), t);
        assert_eq!(v("8'hab").eq(&v("8'hac")), f);
        assert_eq!(v("8'hab").ne(&v("8'hac")), t);
        assert_eq!(v("8'hxb").eq(&v("8'hab")), x);
        assert_eq!(v("8'hxb").ne(&v("8'hab")), x);
        assert_eq!(v("8'hxb").eq(&v("8'hac")), f, "known bits decide");
        assert_eq!(v("8'hzb").eq(&v("8'hzb")), x);
        assert_eq!(v("8'hxb").case_eq(&v("8'hxb")), t);
        assert_eq!(v("8'hxb").case_eq(&v("8'hzb")), f);
        assert_eq!(v("8'hxb").case_ne(&v("8'hzb")), t);
        assert_eq!(v("8'hab").case_eq(&v("8'hab")), t);
        // Wildcards: x/z in the pattern are don't-cares.
        assert_eq!(v("8'hab").wildcard_eq(&v("8'hax")), t);
        assert_eq!(v("8'hab").wildcard_eq(&v("8'hzb")), t);
        assert_eq!(v("8'hab").wildcard_eq(&v("8'hbx")), f);
        assert_eq!(v("8'hxb").wildcard_eq(&v("8'hxb")), t);
        assert_eq!(v("8'hxb").wildcard_eq(&v("8'hab")), x);
        assert_eq!(v("8'hxb").wildcard_eq(&v("8'hac")), f);
        // casez / casex.
        assert!(v("4'b10zz").casez_match(&v("4'b1001")));
        assert!(v("4'b1001").casez_match(&v("4'b10??")));
        assert!(!v("4'b10xx").casez_match(&v("4'b1001")));
        assert!(v("4'b10xx").casez_match(&v("4'b10xx")));
        assert!(!v("4'b10zz").casez_match(&v("4'b0001")));
        assert!(v("4'b10xz").casex_match(&v("4'b1001")));
        assert!(v("4'b1001").casex_match(&v("4'bx0z1")));
        assert!(!v("4'b1001").casex_match(&v("4'bx1z1")));
        // Wide.
        let a = Logic::ones(130);
        let mut b = Logic::ones(130);
        assert_eq!(a.eq(&b), t);
        b.set_bit(129, Zero);
        assert_eq!(a.eq(&b), f);
        b.set_bit(129, X);
        assert_eq!(a.eq(&b), x);
        assert!(a.casex_match(&b));
        assert!(!a.casez_match(&b));
    }

    #[test]
    fn relations() {
        let t = Logic::from_bool(true);
        let f = Logic::from_bool(false);
        let x = Logic::from_bit(X);
        assert_eq!(v("8'd3").lt(&v("8'd5")), t);
        assert_eq!(v("8'd5").lt(&v("8'd5")), f);
        assert_eq!(v("8'd5").le(&v("8'd5")), t);
        assert_eq!(v("8'd5").gt(&v("8'd3")), t);
        assert_eq!(v("8'd3").ge(&v("8'd5")), f);
        assert_eq!(v("8'd3").ge(&v("8'd3")), t);
        // Signed only when both are signed.
        assert_eq!(v("8'shff").lt(&v("8'sh01")), t);
        assert_eq!(v("8'shff").lt(&v("8'h01")), f);
        assert_eq!(v("8'hff").lt(&v("8'sh01")), f);
        assert_eq!(v("8'sh80").lt(&v("8'sh7f")), t);
        assert_eq!(v("8'shfe").lt(&v("8'shff")), t);
        assert_eq!(v("8'shff").gt(&v("8'shfe")), t);
        assert_eq!(v("8'sh00").ge(&v("8'shff")), t);
        // Unknown bits, anywhere, give x.
        assert_eq!(v("8'd3").lt(&v("8'hx5")), x);
        assert_eq!(v("8'h3z").ge(&v("8'd5")), x);
        // Wide.
        let big = Logic::from_u64(1, 130).shl(129);
        let small = Logic::ones(129).resize(130);
        assert_eq!(small.lt(&big), t);
        let (sb, ss) = (big.clone().as_signed(), small.clone().as_signed());
        assert_eq!(ss.lt(&sb), f);
        assert_eq!(sb.lt(&ss), t);
        assert_eq!(big.le(&big), t);
    }

    // ---- arithmetic ----

    #[test]
    fn add_sub_neg() {
        assert_eq!(v("8'd200").add(&v("8'd100")), v("8'd44"));
        assert_eq!(v("8'd5").sub(&v("8'd7")), v("8'd254"));
        assert_eq!(v("8'sd5").sub(&v("8'sd7")).to_i64(), Some(-2));
        assert!(v("8'sd5").add(&v("8'sd7")).is_signed());
        assert!(!v("8'sd5").add(&v("8'd7")).is_signed());
        assert_eq!(
            Logic::ones(128).add(&Logic::from_u64(1, 128)),
            Logic::zero(128)
        );
        assert_eq!(
            Logic::ones(64).add(&Logic::from_u64(1, 64)),
            Logic::zero(64)
        );
        assert_eq!(
            Logic::ones(65).add(&Logic::from_u64(1, 65)),
            Logic::zero(65)
        );
        assert_eq!(
            Logic::zero(130).sub(&Logic::from_u64(1, 130)),
            Logic::ones(130)
        );
        assert_eq!(
            Logic::ones(64).resize(130).add(&Logic::from_u64(1, 130)),
            Logic::from_u64(1, 130).shl(64)
        );
        assert_eq!(v("8'd5").add(&v("8'hx5")), Logic::x(8));
        assert_eq!(v("8'sd5").sub(&v("8'shz5")), Logic::x(8).as_signed());
        assert!(!v("8'sd5").sub(&v("8'hz5")).is_signed());
        assert_eq!(v("8'sd5").neg(), Logic::from_i64(-5, 8));
        assert_eq!(v("8'sd0").neg(), v("8'sd0"));
        assert_eq!(v("8'sh80").neg(), v("8'sh80"));
        assert_eq!(v("8'hx0").neg(), Logic::x(8));
        assert_eq!(Logic::from_u64(1, 130).neg(), Logic::ones(130));
    }

    #[test]
    fn mul() {
        assert_eq!(v("8'd20").mul(&v("8'd20")), v("8'd144"));
        assert_eq!(
            Logic::from_i64(-3, 8).mul(&Logic::from_i64(5, 8)).to_i64(),
            Some(-15)
        );
        assert_eq!(
            Logic::from_i64(-3, 100)
                .mul(&Logic::from_i64(-5, 100))
                .to_i64(),
            Some(15)
        );
        let big = Logic::from_u64(1, 128)
            .shl(64)
            .add(&Logic::from_u64(1, 128));
        // (2^64 + 1)^2 = 2^128 + 2^65 + 1, truncated to 128 bits.
        assert_eq!(
            big.mul(&big),
            Logic::from_u64(1, 128)
                .shl(65)
                .add(&Logic::from_u64(1, 128))
        );
        assert_eq!(Logic::ones(64).mul(&Logic::ones(64)).to_u64(), Some(1));
        assert_eq!(
            Logic::ones(64)
                .resize(128)
                .mul(&Logic::ones(64).resize(128)),
            v("128'hffff_ffff_ffff_fffe_0000_0000_0000_0001")
        );
        assert_eq!(v("8'd5").mul(&v("8'hx5")), Logic::x(8));
        assert_eq!(Logic::zero(200).mul(&Logic::ones(200)), Logic::zero(200));
    }

    #[test]
    fn div_rem() {
        assert_eq!(v("8'd100").div(&v("8'd7")), v("8'd14"));
        assert_eq!(v("8'd100").rem(&v("8'd7")), v("8'd2"));
        assert_eq!(v("8'd7").div(&v("8'd100")), v("8'd0"));
        assert_eq!(v("8'd7").rem(&v("8'd100")), v("8'd7"));
        assert_eq!(v("8'd0").div(&v("8'd3")), v("8'd0"));
        assert_eq!(v("8'd5").div(&v("8'd0")), Logic::x(8));
        assert_eq!(v("8'd5").rem(&v("8'd0")), Logic::x(8));
        assert_eq!(v("8'd5").div(&v("8'hz0")), Logic::x(8));
        // Signed: truncation toward zero, remainder follows the dividend.
        let s = |n: i64| Logic::from_i64(n, 8);
        assert_eq!(s(-7).div(&s(2)).to_i64(), Some(-3));
        assert_eq!(s(-7).rem(&s(2)).to_i64(), Some(-1));
        assert_eq!(s(7).div(&s(-2)).to_i64(), Some(-3));
        assert_eq!(s(7).rem(&s(-2)).to_i64(), Some(1));
        assert_eq!(s(-7).div(&s(-2)).to_i64(), Some(3));
        assert_eq!(s(-7).rem(&s(-2)).to_i64(), Some(-1));
        assert_eq!(
            s(-128).div(&s(-1)).to_i64(),
            Some(-128),
            "wraps like hardware"
        );
        assert_eq!(s(-128).rem(&s(-1)).to_i64(), Some(0));
        // Mixed signedness is unsigned: 0xf9 / 2.
        assert_eq!(s(-7).div(&Logic::from_u64(2, 8)), v("8'd124"));
        // Wide.
        let n = Logic::ones(128);
        let d = Logic::from_u64(3, 128);
        assert_eq!(n.div(&d), v("128'h5555_5555_5555_5555_5555_5555_5555_5555"));
        assert_eq!(n.rem(&d), Logic::zero(128));
        let n = Logic::from_u64(1, 130).shl(129);
        let d = Logic::from_u64(1, 130)
            .shl(64)
            .add(&Logic::from_u64(1, 130));
        let q = n.div(&d);
        let r = n.rem(&d);
        assert_eq!(q.mul(&d).add(&r), n);
        assert_eq!(r.lt(&d), Logic::from_bool(true));
        assert_eq!(
            Logic::ones(64).div(&Logic::from_u64(u64::MAX, 64)).to_u64(),
            Some(1)
        );
        assert_eq!(
            Logic::ones(64)
                .resize(200)
                .div(&Logic::from_u64(1, 200).shl(63))
                .to_u64(),
            Some(1)
        );
        assert_eq!(
            Logic::ones(64)
                .resize(200)
                .rem(&Logic::from_u64(1, 200).shl(63))
                .to_u64(),
            Some((1 << 63) - 1)
        );
        assert_eq!(
            Logic::from_i64(-1, 130)
                .div(&Logic::from_i64(3, 130))
                .to_i64(),
            Some(0)
        );
        assert_eq!(
            Logic::from_i64(-9, 130)
                .div(&Logic::from_i64(3, 130))
                .to_i64(),
            Some(-3)
        );
    }

    #[test]
    fn pow_follows_table_5_6() {
        let s = |n: i64| Logic::from_i64(n, 8);
        let u = |n: u64| Logic::from_u64(n, 8);
        let x = Logic::x(8);
        // op2 zero: 1.
        assert_eq!(u(0).pow(&u(0)), u(1));
        assert_eq!(s(-5).pow(&s(0)), s(1));
        // op1 zero.
        assert_eq!(u(0).pow(&u(3)), u(0));
        assert_eq!(s(0).pow(&s(-1)), x.clone().as_signed());
        // op1 one.
        assert_eq!(u(1).pow(&u(200)), u(1));
        assert_eq!(s(1).pow(&s(-3)), s(1));
        // op1 minus one.
        assert_eq!(s(-1).pow(&s(3)), s(-1));
        assert_eq!(s(-1).pow(&s(4)), s(1));
        assert_eq!(s(-1).pow(&s(-3)), s(-1));
        assert_eq!(s(-1).pow(&s(-4)), s(1));
        // Unsigned 0xff is 255, not -1.
        assert_eq!(u(255).pow(&u(2)), u(1), "255^2 mod 256");
        // Negative exponent, other bases.
        assert_eq!(s(2).pow(&s(-1)), s(0));
        assert_eq!(s(-2).pow(&s(-1)), s(0));
        // Ordinary powers, truncated.
        assert_eq!(u(2).pow(&u(7)), u(128));
        assert_eq!(u(2).pow(&u(8)), u(0));
        assert_eq!(u(3).pow(&u(4)), u(81));
        assert_eq!(s(-3).pow(&s(3)), s(-27));
        assert_eq!(s(-3).pow(&s(2)), s(9));
        // Exponent is self-determined: any width, own signedness.
        assert_eq!(u(3).pow(&Logic::from_u64(4, 70)), u(81));
        assert_eq!(s(2).pow(&Logic::from_i64(-1, 3)), s(0));
        assert!(!s(2).pow(&u(2)).is_signed());
        assert!(s(2).pow(&s(2)).is_signed());
        // Unknowns.
        assert_eq!(u(2).pow(&v("8'hx")), x);
        assert_eq!(v("8'hz").pow(&u(2)), x);
        // Wide: 3^100 mod 2^128, against repeated multiplication.
        let three = Logic::from_u64(3, 128);
        let mut expected = Logic::from_u64(1, 128);
        for _ in 0..100 {
            expected = expected.mul(&three);
        }
        assert_eq!(three.pow(&Logic::from_u64(100, 8)), expected);
        assert_eq!(
            Logic::from_u64(1, 130).shl(64).pow(&Logic::from_u64(2, 4)),
            Logic::from_u64(1, 130).shl(128)
        );
        assert_eq!(
            Logic::from_u64(2, 130).pow(&Logic::from_u64(130, 8)),
            Logic::zero(130)
        );
        assert_eq!(
            Logic::from_u64(2, 130)
                .pow(&Logic::from_u64(129, 8))
                .bit(129),
            One
        );
    }

    #[test]
    #[should_panic(expected = "differ in width")]
    fn arith_width_mismatch_panics() {
        let _ = v("4'h1").add(&v("8'h1"));
    }

    #[test]
    #[should_panic(expected = "out of range")]
    fn bit_out_of_range_panics() {
        let _ = v("4'h1").bit(4);
    }

    #[test]
    fn from_planes_normalises() {
        let l = Logic::from_planes(4, true, vec![u64::MAX], vec![0]);
        assert_eq!(l, v("4'shf"));
        assert_eq!(Logic::from_planes(0, false, vec![], vec![]), Logic::zero(0));
        assert_eq!(l.value_words(), &[0xf]);
        assert_eq!(l.unknown_words(), &[0]);
    }

    #[test]
    #[should_panic(expected = "word count")]
    fn from_planes_rejects_wrong_word_count() {
        let _ = Logic::from_planes(65, false, vec![0], vec![0]);
    }
}
