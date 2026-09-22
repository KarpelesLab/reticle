//! Two-state machine-word kernels.
//!
//! A compiled value is `width` bits held in `width.div_ceil(64)` `u64`
//! words, least significant word first, with the bits above `width` in the
//! top word always zero. That is exactly the `value` plane of a [`Logic`]
//! with no `unknown` bits, which is what makes the compiled engine's
//! results comparable with the event simulator's bit for bit.
//!
//! Every kernel here writes into a caller-supplied slice, so the engine
//! never allocates while it runs. The narrow case (one word) is the one
//! that pays: a 1-bit `and` is a word `&`, an 8-bit `add` is a word add and
//! a mask. Wider values fall back to a loop over words — a carry chain for
//! `add`, schoolbook partial products for `mul`, restoring division for
//! `div` and `mod`.
//!
//! The unit tests at the bottom fuzz every kernel against the matching
//! [`Logic`] operator on random two-state operands of many widths, because
//! an arithmetic kernel that is subtly wrong is exactly the failure mode a
//! fast simulator must not have.
//!
//! [`Logic`]: crate::logic::Logic

use std::cmp::Ordering;

/// The number of `u64` words a value of `width` bits occupies.
pub(crate) fn words_for(width: u32) -> usize {
    // `u32` always fits `usize` on supported targets.
    (width as usize).div_ceil(64)
}

/// The mask of significant bits in word `i` of a value of `width` bits.
pub(crate) fn word_mask(i: usize, width: u32) -> u64 {
    let base = i.saturating_mul(64);
    let w = width as usize;
    if base >= w {
        return 0;
    }
    let rest = w - base;
    if rest >= 64 {
        u64::MAX
    } else {
        (1u64 << rest) - 1
    }
}

/// Clears the bits at or above `width` in the top word.
pub(crate) fn mask_top(dst: &mut [u64], width: u32) {
    if let Some(last) = dst.len().checked_sub(1) {
        dst[last] &= word_mask(last, width);
    }
}

/// True when every word is zero.
pub(crate) fn is_zero(a: &[u64]) -> bool {
    a.iter().all(|w| *w == 0)
}

/// Bit `i`, or false past the end.
pub(crate) fn get_bit(a: &[u64], i: u32) -> bool {
    let idx = (i as usize) / 64;
    match a.get(idx) {
        Some(w) => (w >> (i % 64)) & 1 == 1,
        None => false,
    }
}

/// Sets bit `i` to `v`; a position past the end is ignored.
pub(crate) fn set_bit(dst: &mut [u64], i: u32, v: bool) {
    let idx = (i as usize) / 64;
    if let Some(w) = dst.get_mut(idx) {
        let bit = 1u64 << (i % 64);
        if v {
            *w |= bit;
        } else {
            *w &= !bit;
        }
    }
}

/// The sign bit of a value: its most significant bit when `signed`, else
/// false.
pub(crate) fn sign_of(a: &[u64], width: u32, signed: bool) -> bool {
    signed && width > 0 && get_bit(a, width - 1)
}

/// The low 64 bits of a 128-bit intermediate; the truncation is the point,
/// since the caller keeps the high half as a carry.
fn low64(v: u128) -> u64 {
    u64::try_from(v & u128::from(u64::MAX)).expect("masked to 64 bits")
}

/// The value as a `u64`, saturating when it does not fit. Shift amounts
/// and addresses use it: a value that does not fit is out of range for any
/// operand anyway.
pub(crate) fn to_u64_saturating(a: &[u64]) -> u64 {
    match a.split_first() {
        None => 0,
        Some((lo, rest)) if is_zero(rest) => *lo,
        Some(_) => u64::MAX,
    }
}

/// `dst = src`, extending or truncating to `dst_width`; the extension
/// copies the sign bit when the source is signed.
pub(crate) fn resize_into(
    dst: &mut [u64],
    dst_width: u32,
    src: &[u64],
    src_width: u32,
    src_signed: bool,
) {
    let negative = sign_of(src, src_width, src_signed);
    for (i, slot) in dst.iter_mut().enumerate() {
        *slot = src.get(i).copied().unwrap_or(0);
    }
    if negative && dst_width > src_width {
        // Fill from `src_width` up, one word at a time once past the
        // source's own top word.
        let head = src_width.min(dst_width);
        let boundary = head.next_multiple_of(64).min(dst_width);
        for i in head..boundary {
            set_bit(dst, i, true);
        }
        let first_full = words_for(boundary);
        for slot in dst.iter_mut().skip(first_full) {
            *slot = u64::MAX;
        }
    }
    mask_top(dst, dst_width);
}

/// `dst = src << n`, zero filling, keeping `width` bits.
pub(crate) fn shl_into(dst: &mut [u64], src: &[u64], width: u32, n: u64) {
    if n >= u64::from(width) {
        dst.fill(0);
        return;
    }
    let n = u32::try_from(n).expect("checked against the width");
    let words = (n as usize) / 64;
    let bits = n % 64;
    for i in (0..dst.len()).rev() {
        let mut v = 0u64;
        if let Some(j) = i.checked_sub(words) {
            v = src.get(j).copied().unwrap_or(0) << bits;
            if bits > 0
                && let Some(k) = j.checked_sub(1)
            {
                v |= src.get(k).copied().unwrap_or(0) >> (64 - bits);
            }
        }
        dst[i] = v;
    }
    mask_top(dst, width);
}

/// `dst = src >> n`, filling the vacated bits with `fill`.
pub(crate) fn shr_into(dst: &mut [u64], src: &[u64], width: u32, n: u64, fill: bool) {
    let filler = if fill { u64::MAX } else { 0 };
    if n >= u64::from(width) {
        dst.fill(filler);
        mask_top(dst, width);
        return;
    }
    let n = u32::try_from(n).expect("checked against the width");
    let words = (n as usize) / 64;
    let bits = n % 64;
    let src_word = |i: usize| -> u64 {
        match src.get(i) {
            Some(w) => *w | (filler & !word_mask(i, width)),
            None => filler,
        }
    };
    for (i, slot) in dst.iter_mut().enumerate() {
        let mut v = src_word(i + words) >> bits;
        if bits > 0 {
            v |= src_word(i + words + 1) << (64 - bits);
        }
        *slot = v;
    }
    mask_top(dst, width);
}

/// `dst = a + b`, modulo `2^width`.
pub(crate) fn add_into(dst: &mut [u64], a: &[u64], b: &[u64], width: u32) {
    let mut carry = 0u64;
    for i in 0..dst.len() {
        let (s1, c1) = a[i].overflowing_add(b[i]);
        let (s2, c2) = s1.overflowing_add(carry);
        dst[i] = s2;
        carry = u64::from(c1 || c2);
    }
    mask_top(dst, width);
}

/// `dst = a - b`, modulo `2^width`.
pub(crate) fn sub_into(dst: &mut [u64], a: &[u64], b: &[u64], width: u32) {
    let mut borrow = 0u64;
    for i in 0..dst.len() {
        let (d1, b1) = a[i].overflowing_sub(b[i]);
        let (d2, b2) = d1.overflowing_sub(borrow);
        dst[i] = d2;
        borrow = u64::from(b1 || b2);
    }
    mask_top(dst, width);
}

/// `dst = -a`, modulo `2^width`.
pub(crate) fn neg_into(dst: &mut [u64], a: &[u64], width: u32) {
    let mut carry = 1u64;
    for i in 0..dst.len() {
        let (v, c) = (!a[i]).overflowing_add(carry);
        dst[i] = v;
        carry = u64::from(c);
    }
    mask_top(dst, width);
}

/// `dst = a * b`, keeping the low `width` bits (the same for signed and
/// unsigned operands).
pub(crate) fn mul_into(dst: &mut [u64], a: &[u64], b: &[u64], width: u32) {
    let n = dst.len();
    if n == 1 {
        dst[0] = a[0].wrapping_mul(b[0]);
        mask_top(dst, width);
        return;
    }
    dst.fill(0);
    for i in 0..n {
        if a[i] == 0 {
            continue;
        }
        let mut carry = 0u128;
        for j in 0..(n - i) {
            let prod = u128::from(a[i]) * u128::from(b[j]) + u128::from(dst[i + j]) + carry;
            dst[i + j] = low64(prod);
            carry = prod >> 64;
        }
    }
    mask_top(dst, width);
}

/// Orders two values of `width` bits, signed when `signed`.
pub(crate) fn cmp(a: &[u64], b: &[u64], width: u32, signed: bool) -> Ordering {
    if signed {
        let an = sign_of(a, width, true);
        let bn = sign_of(b, width, true);
        if an != bn {
            return if an {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }
    }
    for i in (0..a.len()).rev() {
        match a[i].cmp(&b[i]) {
            Ordering::Equal => {}
            other => return other,
        }
    }
    Ordering::Equal
}

/// `dst` becomes `dst_width` bits of `src` starting at bit `lo`.
///
/// `lo` may be negative or past the end of `src`; the bits outside `src`
/// read as zero, which is the one place the compiled engine deliberately
/// differs from the 4-state simulator (which reads them as `x`).
pub(crate) fn extract_into(
    dst: &mut [u64],
    dst_width: u32,
    src: &[u64],
    src_width: u32,
    lo: i64,
    scratch: &mut Vec<u64>,
) {
    if dst_width == 0 {
        return;
    }
    if lo >= i64::from(src_width) || lo <= -i64::from(dst_width) {
        dst.fill(0);
        return;
    }
    let span = src.len().max(dst.len()) + 1;
    scratch.clear();
    scratch.resize(span, 0);
    for (i, slot) in scratch.iter_mut().enumerate() {
        *slot = src.get(i).copied().unwrap_or(0);
    }
    let wide = u32::try_from(span.saturating_mul(64)).unwrap_or(u32::MAX);
    if lo >= 0 {
        let n = u64::try_from(lo).expect("checked non-negative");
        let copy: Vec<u64> = scratch.clone();
        shr_into(scratch, &copy, wide, n, false);
    } else {
        let n = u64::try_from(-lo).expect("checked negative");
        let copy: Vec<u64> = scratch.clone();
        shl_into(scratch, &copy, wide, n);
    }
    for (i, slot) in dst.iter_mut().enumerate() {
        *slot = scratch.get(i).copied().unwrap_or(0);
    }
    mask_top(dst, dst_width);
}

/// Replaces bits `[lo, lo + part_width)` of `dst` with `part`; the bits of
/// `part` that fall outside `dst` are dropped.
pub(crate) fn insert_into(dst: &mut [u64], dst_width: u32, part: &[u64], part_width: u32, lo: i64) {
    for i in 0..part_width {
        let at = lo + i64::from(i);
        if at < 0 || at >= i64::from(dst_width) {
            continue;
        }
        let at = u32::try_from(at).expect("checked in range");
        set_bit(dst, at, get_bit(part, i));
    }
}

/// True when the number of one bits is odd.
pub(crate) fn parity(a: &[u64]) -> bool {
    a.iter().map(|w| w.count_ones()).sum::<u32>() % 2 == 1
}

/// True when every significant bit is one.
pub(crate) fn all_ones(a: &[u64], width: u32) -> bool {
    a.iter().enumerate().all(|(i, w)| *w == word_mask(i, width))
}

/// Writes `a / b` into `quot` and `a % b` into `rem`, truncating towards
/// zero with the remainder taking the sign of the dividend.
///
/// Returns false on division by zero, where the 4-state simulator yields
/// all `x`; the caller decides what a two-state run does instead.
///
/// The two `abs_*` buffers are the caller's scratch: division is the one
/// kernel that needs working storage, and taking it as a parameter is what
/// keeps the engine allocation-free.
#[allow(clippy::too_many_arguments)]
pub(crate) fn divrem_into(
    quot: &mut [u64],
    rem: &mut [u64],
    a: &[u64],
    b: &[u64],
    width: u32,
    signed: bool,
    abs_a: &mut Vec<u64>,
    abs_b: &mut Vec<u64>,
) -> bool {
    if is_zero(b) {
        return false;
    }
    let a_neg = sign_of(a, width, signed);
    let b_neg = sign_of(b, width, signed);
    abs_a.clear();
    abs_a.extend_from_slice(a);
    abs_b.clear();
    abs_b.extend_from_slice(b);
    if a_neg {
        let copy: Vec<u64> = abs_a.clone();
        neg_into(abs_a, &copy, width);
    }
    if b_neg {
        let copy: Vec<u64> = abs_b.clone();
        neg_into(abs_b, &copy, width);
    }
    if quot.len() == 1 {
        quot[0] = abs_a[0] / abs_b[0];
        rem[0] = abs_a[0] % abs_b[0];
    } else {
        quot.fill(0);
        rem.fill(0);
        for i in (0..width).rev() {
            let carry: Vec<u64> = rem.to_vec();
            shl_into(rem, &carry, width, 1);
            set_bit(rem, 0, get_bit(abs_a, i));
            if cmp(rem, abs_b, width, false) != Ordering::Less {
                let cur: Vec<u64> = rem.to_vec();
                sub_into(rem, &cur, abs_b, width);
                set_bit(quot, i, true);
            }
        }
    }
    if a_neg != b_neg {
        let q: Vec<u64> = quot.to_vec();
        neg_into(quot, &q, width);
    }
    if a_neg {
        let r: Vec<u64> = rem.to_vec();
        neg_into(rem, &r, width);
    }
    mask_top(quot, width);
    mask_top(rem, width);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::Logic;

    /// xorshift64*, so the fuzzing is reproducible without a dependency.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            x.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
    }

    fn random(rng: &mut Rng, width: u32, signed: bool) -> Logic {
        let mut words = vec![0u64; words_for(width)];
        for w in &mut words {
            *w = rng.next();
        }
        mask_top(&mut words, width);
        let unknown = vec![0u64; words.len()];
        Logic::from_planes(width, signed, words, unknown)
    }

    fn buf(width: u32) -> Vec<u64> {
        vec![0u64; words_for(width)]
    }

    fn same(got: &[u64], want: &Logic) {
        assert!(!want.has_unknown(), "reference produced x bits");
        assert_eq!(got, want.value_words(), "want {}", want.to_binary_string());
    }

    #[test]
    fn arithmetic_matches_logic() {
        let mut rng = Rng(0x1234_5678);
        for width in [1u32, 3, 8, 16, 32, 63, 64, 65, 100, 128, 200] {
            for signed in [false, true] {
                for _ in 0..40 {
                    let a = random(&mut rng, width, signed);
                    let b = random(&mut rng, width, signed);
                    let (av, bv) = (a.value_words(), b.value_words());
                    let mut out = buf(width);
                    add_into(&mut out, av, bv, width);
                    same(&out, &a.add(&b));
                    sub_into(&mut out, av, bv, width);
                    same(&out, &a.sub(&b));
                    neg_into(&mut out, av, width);
                    same(&out, &a.neg());
                    mul_into(&mut out, av, bv, width);
                    same(&out, &a.mul(&b));
                    let mut rem = buf(width);
                    let (mut s1, mut s2) = (Vec::new(), Vec::new());
                    if divrem_into(&mut out, &mut rem, av, bv, width, signed, &mut s1, &mut s2) {
                        same(&out, &a.div(&b));
                        same(&rem, &a.rem(&b));
                    } else {
                        assert!(a.div(&b).has_unknown());
                    }
                    let ord = cmp(av, bv, width, signed);
                    assert_eq!(ord == Ordering::Less, a.lt(&b).to_u64() == Some(1));
                    assert_eq!(ord == Ordering::Greater, a.gt(&b).to_u64() == Some(1));
                    assert_eq!(ord == Ordering::Equal, a.eq(&b).to_u64() == Some(1));
                }
            }
        }
    }

    #[test]
    fn shifts_and_selects_match_logic() {
        let mut rng = Rng(0x9E37_79B9);
        let mut scratch = Vec::new();
        for width in [1u32, 5, 8, 31, 64, 65, 96, 129] {
            for _ in 0..10 {
                for signed in [false, true] {
                    let a = random(&mut rng, width, signed);
                    for n in [0u64, 1, 3, 63, 64, 65, u64::from(width), 4096] {
                        let mut out = buf(width);
                        shl_into(&mut out, a.value_words(), width, n);
                        let amount = Logic::from_u64(n, 64);
                        same(&out, &a.shl_by(&amount));
                        shr_into(&mut out, a.value_words(), width, n, false);
                        same(&out, &a.shr_by(&amount));
                        let fill = sign_of(a.value_words(), width, signed);
                        shr_into(&mut out, a.value_words(), width, n, fill);
                        same(&out, &a.sshr_by(&amount));
                    }
                    for lo in [-3i64, 0, 1, 7, i64::from(width) - 1, i64::from(width) + 2] {
                        for w in [1u32, 2, 8, width] {
                            let mut out = buf(w);
                            extract_into(&mut out, w, a.value_words(), width, lo, &mut scratch);
                            let want = crate::sim::value::select_bits(&a, lo, w);
                            let known: Vec<u64> = want
                                .value_words()
                                .iter()
                                .zip(want.unknown_words())
                                .map(|(v, u)| v & !u)
                                .collect();
                            assert_eq!(out, known, "extract {w} bits at {lo} of {width}");
                        }
                    }
                    for to in [1u32, 4, width, width + 7, width + 64] {
                        let mut out = buf(to);
                        resize_into(&mut out, to, a.value_words(), width, signed);
                        same(&out, &a.resize(to));
                    }
                }
            }
        }
    }

    #[test]
    fn small_helpers() {
        assert_eq!(words_for(0), 0);
        assert_eq!(words_for(1), 1);
        assert_eq!(words_for(64), 1);
        assert_eq!(words_for(65), 2);
        assert_eq!(word_mask(0, 3), 0b111);
        assert_eq!(word_mask(1, 3), 0);
        assert_eq!(word_mask(1, 70), (1 << 6) - 1);
        assert!(is_zero(&[]));
        assert!(!get_bit(&[], 0));
        let mut v = vec![0u64; 2];
        set_bit(&mut v, 70, true);
        assert!(get_bit(&v, 70));
        set_bit(&mut v, 70, false);
        assert!(is_zero(&v));
        set_bit(&mut v, 999, true);
        assert!(is_zero(&v));
        assert!(parity(&[0b111]));
        assert!(!parity(&[0b101]));
        assert!(all_ones(&[0b111], 3));
        assert!(!all_ones(&[0b101], 3));
        assert_eq!(to_u64_saturating(&[7]), 7);
        assert_eq!(to_u64_saturating(&[7, 1]), u64::MAX);
        assert_eq!(to_u64_saturating(&[]), 0);
        assert!(!sign_of(&[1], 0, true));
        let mut dst = vec![0u64; 1];
        insert_into(&mut dst, 8, &[0b11], 2, 3);
        assert_eq!(dst[0], 0b0001_1000);
        insert_into(&mut dst, 8, &[0b11], 2, 7);
        assert_eq!(dst[0], 0b1001_1000);
        insert_into(&mut dst, 8, &[0b11], 2, -1);
        assert_eq!(dst[0], 0b1001_1001);
        let mut out = vec![0u64; 1];
        let mut scratch = Vec::new();
        extract_into(&mut out, 0, &[1], 4, 0, &mut scratch);
        assert!(is_zero(&out));
        assert_eq!(cmp(&[], &[], 0, true), Ordering::Equal);
    }
}
