//! Truth tables over a small number of variables, stored as bit vectors.
//!
//! A table over `n` variables has `2^n` bits; bit `i` is the function's
//! value for the input pattern `i`, where variable `v` is bit `v` of the
//! pattern. Tables of up to six variables fit one `u64`; larger ones use
//! `2^(n-6)` words. The variable-`v` projection for `v < 6` is the familiar
//! constant (`0xAAAA…` for `v = 0`, `0xCCCC…` for `v = 1`, …), and for
//! `v >= 6` it is a word-level pattern.
//!
//! These are the tables the rewriting library ([`super::rewrite`]) is indexed
//! by, that [`super::refactor`] derives sum-of-products from, and that the
//! technology mapper turns into LUT `init` strings.

/// The number of `u64` words a table over `vars` variables needs.
pub fn words_for(vars: usize) -> usize {
    if vars <= 6 { 1 } else { 1 << (vars - 6) }
}

/// The mask of meaningful bits in the (single) word of a table over at
/// most six variables.
fn word_mask(vars: usize) -> u64 {
    if vars >= 6 {
        !0
    } else {
        (1u64 << (1 << vars)) - 1
    }
}

/// A truth table over `vars` variables.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TruthTable {
    vars: usize,
    words: Vec<u64>,
}

impl TruthTable {
    /// The constant function `value` over `vars` variables.
    pub fn constant(vars: usize, value: bool) -> TruthTable {
        let fill = if value { word_mask(vars) } else { 0 };
        TruthTable {
            vars,
            words: vec![fill; words_for(vars)],
        }
    }

    /// The projection onto variable `v`.
    pub fn var(vars: usize, v: usize) -> TruthTable {
        assert!(v < vars, "variable {v} out of range for {vars} variables");
        let words = words_for(vars);
        let mut out = vec![0u64; words];
        if v < 6 {
            let pattern = match v {
                0 => 0xAAAA_AAAA_AAAA_AAAA,
                1 => 0xCCCC_CCCC_CCCC_CCCC,
                2 => 0xF0F0_F0F0_F0F0_F0F0,
                3 => 0xFF00_FF00_FF00_FF00,
                4 => 0xFFFF_0000_FFFF_0000,
                _ => 0xFFFF_FFFF_0000_0000,
            };
            for w in &mut out {
                *w = pattern & word_mask(vars);
            }
        } else {
            for (i, w) in out.iter_mut().enumerate() {
                if (i >> (v - 6)) & 1 == 1 {
                    *w = !0;
                }
            }
        }
        TruthTable { vars, words: out }
    }

    /// A table from its words (bit `i` of word `i / 64` is pattern `i`).
    pub fn from_words(vars: usize, words: Vec<u64>) -> TruthTable {
        assert_eq!(words.len(), words_for(vars), "word count");
        let mut t = TruthTable { vars, words };
        if vars < 6 {
            t.words[0] &= word_mask(vars);
        }
        t
    }

    /// A table over at most six variables from one word.
    pub fn from_u64(vars: usize, word: u64) -> TruthTable {
        assert!(vars <= 6);
        TruthTable::from_words(vars, vec![word])
    }

    /// Number of variables.
    pub fn vars(&self) -> usize {
        self.vars
    }

    /// The words, least significant pattern first.
    pub fn words(&self) -> &[u64] {
        &self.words
    }

    /// The single word of a table over at most six variables.
    pub fn as_u64(&self) -> u64 {
        assert!(self.vars <= 6);
        self.words[0]
    }

    /// The value for input pattern `index`.
    pub fn bit(&self, index: usize) -> bool {
        (self.words[index / 64] >> (index % 64)) & 1 == 1
    }

    /// Number of input patterns (`2^vars`).
    pub fn len(&self) -> usize {
        1 << self.vars
    }

    /// Always false: a table has at least one pattern.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// True for the constant-false table.
    pub fn is_zero(&self) -> bool {
        self.words.iter().all(|&w| w == 0)
    }

    /// True for the constant-true table.
    pub fn is_ones(&self) -> bool {
        self.words
            .iter()
            .enumerate()
            .all(|(i, &w)| w == if i == 0 { word_mask(self.vars) } else { !0 })
    }

    /// Number of patterns mapping to true.
    pub fn count_ones(&self) -> u32 {
        self.words.iter().map(|w| w.count_ones()).sum()
    }

    /// The complement.
    pub fn not(&self) -> TruthTable {
        let mask = word_mask(self.vars);
        TruthTable {
            vars: self.vars,
            words: self
                .words
                .iter()
                .enumerate()
                .map(|(i, &w)| if i == 0 { !w & mask } else { !w })
                .collect(),
        }
    }

    fn zip(&self, other: &TruthTable, f: impl Fn(u64, u64) -> u64) -> TruthTable {
        assert_eq!(
            self.vars, other.vars,
            "truth tables over different variables"
        );
        TruthTable {
            vars: self.vars,
            words: self
                .words
                .iter()
                .zip(&other.words)
                .map(|(&a, &b)| f(a, b))
                .collect(),
        }
    }

    /// Conjunction.
    pub fn and(&self, other: &TruthTable) -> TruthTable {
        self.zip(other, |a, b| a & b)
    }

    /// Disjunction.
    pub fn or(&self, other: &TruthTable) -> TruthTable {
        self.zip(other, |a, b| a | b)
    }

    /// Exclusive or.
    pub fn xor(&self, other: &TruthTable) -> TruthTable {
        self.zip(other, |a, b| a ^ b)
    }

    /// True when the function depends on variable `v`.
    pub fn depends_on(&self, v: usize) -> bool {
        self.cofactor(v, false) != self.cofactor(v, true)
    }

    /// The set of variables the function depends on, as a bit mask.
    pub fn support(&self) -> u32 {
        (0..self.vars)
            .filter(|&v| self.depends_on(v))
            .fold(0, |m, v| m | (1 << v))
    }

    /// The cofactor with variable `v` fixed to `value`, still over the same
    /// variables (the result no longer depends on `v`).
    pub fn cofactor(&self, v: usize, value: bool) -> TruthTable {
        let mut out = self.words.clone();
        if v < 6 {
            let shift = 1u32 << v;
            let keep = TruthTable::var(6.min(self.vars).max(v + 1), v).words[0];
            let keep = if value { keep } else { !keep };
            for w in &mut out {
                let kept = *w & keep;
                *w = if value {
                    kept | (kept >> shift)
                } else {
                    kept | (kept << shift)
                };
            }
            if self.vars < 6 {
                out[0] &= word_mask(self.vars);
            }
        } else {
            let stride = 1usize << (v - 6);
            for (i, word) in out.iter_mut().enumerate() {
                let hi = (i >> (v - 6)) & 1 == 1;
                if hi != value {
                    let src = if value { i + stride } else { i - stride };
                    *word = self.words[src];
                }
            }
        }
        TruthTable {
            vars: self.vars,
            words: out,
        }
    }

    /// The same function over `vars` variables (`vars >= self.vars`), the
    /// new variables being don't-cares.
    pub fn extend(&self, vars: usize) -> TruthTable {
        assert!(vars >= self.vars);
        if vars == self.vars {
            return self.clone();
        }
        let mut word = self.words[0];
        let mut cur = self.vars;
        while cur < 6 && cur < vars {
            let width = 1u32 << cur;
            word |= word << width;
            cur += 1;
        }
        let words = words_for(vars);
        if self.vars <= 6 {
            return TruthTable::from_words(vars, vec![word; words]);
        }
        let mut out = Vec::with_capacity(words);
        while out.len() < words {
            out.extend_from_slice(&self.words);
        }
        TruthTable::from_words(vars, out)
    }

    /// The function with its variables permuted: the result's variable
    /// `perm[i]` is this table's variable `i` (`perm` is a permutation of
    /// `0..vars`).
    pub fn permute(&self, perm: &[usize]) -> TruthTable {
        assert_eq!(perm.len(), self.vars);
        let mut out = TruthTable::constant(self.vars, false);
        for index in 0..self.len() {
            if self.bit(index) {
                let mut target = 0usize;
                for (i, &p) in perm.iter().enumerate() {
                    if (index >> i) & 1 == 1 {
                        target |= 1 << p;
                    }
                }
                out.words[target / 64] |= 1u64 << (target % 64);
            }
        }
        out
    }

    /// The function with the variables in `mask` complemented.
    pub fn negate_inputs(&self, mask: u32) -> TruthTable {
        let mut out = TruthTable::constant(self.vars, false);
        let mask = usize::try_from(mask).expect("mask");
        for index in 0..self.len() {
            if self.bit(index) {
                let target = index ^ mask;
                out.words[target / 64] |= 1u64 << (target % 64);
            }
        }
        out
    }

    /// The function with only the variables in `keep` (a bit mask of
    /// `self.vars`) as variables, in ascending order; the others must not
    /// be in the support.
    pub fn restrict(&self, keep: u32) -> TruthTable {
        let kept: Vec<usize> = (0..self.vars).filter(|&v| (keep >> v) & 1 == 1).collect();
        let mut out = TruthTable::constant(kept.len(), false);
        for index in 0..out.len() {
            let mut full = 0usize;
            for (i, &v) in kept.iter().enumerate() {
                if (index >> i) & 1 == 1 {
                    full |= 1 << v;
                }
            }
            if self.bit(full) {
                out.words[index / 64] |= 1u64 << (index % 64);
            }
        }
        out
    }
}

/// The NPN-canonical form of a table over at most six variables: the
/// smallest table (as an integer) reachable by permuting inputs, negating
/// inputs and negating the output, together with a transform that maps the
/// original to it.
///
/// The transform is `(perm, input_negation, output_negation)`: the canonical
/// table equals `self.negate_inputs(neg).permute(perm)`, complemented when
/// `output_negation` is set. This is the brute-force enumeration over all
/// `n! · 2^n · 2` transforms, which is fine for the sizes it is used on
/// (library gates and unit tests); the rewriting table is indexed by the
/// full function instead.
pub fn npn_canonical(tt: &TruthTable) -> (TruthTable, Vec<usize>, u32, bool) {
    let n = tt.vars();
    assert!(n <= 6);
    let mut best: Option<(u64, Vec<usize>, u32, bool)> = None;
    let mut perm: Vec<usize> = (0..n).collect();
    let count = (1..=n).product::<usize>().max(1);
    for _ in 0..count {
        for neg in 0..(1u32 << n) {
            let t = tt.negate_inputs(neg).permute(&perm);
            for out_neg in [false, true] {
                let word = if out_neg {
                    t.not().as_u64()
                } else {
                    t.as_u64()
                };
                if best.as_ref().is_none_or(|b| word < b.0) {
                    best = Some((word, perm.clone(), neg, out_neg));
                }
            }
        }
        if !next_permutation(&mut perm) {
            break;
        }
    }
    let (word, perm, neg, out_neg) = best.expect("at least one transform");
    (TruthTable::from_u64(n, word), perm, neg, out_neg)
}

/// Advances `perm` to the lexicographically next permutation; false when
/// it was the last one.
pub fn next_permutation(perm: &mut [usize]) -> bool {
    let n = perm.len();
    if n < 2 {
        return false;
    }
    let mut i = n - 1;
    while i > 0 && perm[i - 1] >= perm[i] {
        i -= 1;
    }
    if i == 0 {
        return false;
    }
    let mut j = n - 1;
    while perm[j] <= perm[i - 1] {
        j -= 1;
    }
    perm.swap(i - 1, j);
    perm[i..].reverse();
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projections_and_operators() {
        let a = TruthTable::var(3, 0);
        let b = TruthTable::var(3, 1);
        let c = TruthTable::var(3, 2);
        assert_eq!(a.as_u64(), 0xAA);
        assert_eq!(b.as_u64(), 0xCC);
        assert_eq!(c.as_u64(), 0xF0);
        let f = a.and(&b).or(&c);
        assert_eq!(f.as_u64(), 0xF8);
        assert_eq!(f.not().as_u64(), 0x07);
        assert_eq!(a.xor(&b).as_u64(), 0x66);
        assert_eq!(f.count_ones(), 5);
        assert!(f.bit(7));
        assert!(!f.bit(0));
        assert_eq!(f.len(), 8);
        assert!(!f.is_empty());
        assert!(TruthTable::constant(3, false).is_zero());
        assert!(TruthTable::constant(3, true).is_ones());
        assert!(!f.is_ones());
        assert_eq!(f.support(), 0b111);
        assert_eq!(a.support(), 0b001);
        assert_eq!(a.cofactor(0, true).as_u64(), 0xFF);
        assert_eq!(a.cofactor(0, false).as_u64(), 0x00);
        assert_eq!(f.cofactor(2, false).as_u64(), 0x88);
        assert_eq!(f.cofactor(2, true).as_u64(), 0xFF);
        assert!(f.depends_on(2));
        assert!(!a.depends_on(1));
        assert_eq!(a.extend(4).as_u64(), 0xAAAA);
        assert_eq!(a.and(&b).extend(6).as_u64(), 0x8888_8888_8888_8888);
        assert_eq!(a.and(&b).restrict(0b011).as_u64(), 0x8);
        assert_eq!(a.permute(&[1, 0, 2]).as_u64(), 0xCC);
        assert_eq!(a.negate_inputs(1).as_u64(), 0x55);
    }

    #[test]
    fn wide_tables() {
        let v7 = TruthTable::var(8, 7);
        assert_eq!(v7.words().len(), 4);
        assert_eq!(v7.words(), [0, 0, !0, !0]);
        let v6 = TruthTable::var(8, 6);
        assert_eq!(v6.words(), [0, !0, 0, !0]);
        let v0 = TruthTable::var(8, 0);
        let f = v7.and(&v0);
        assert_eq!(
            f.words(),
            [0, 0, 0xAAAA_AAAA_AAAA_AAAA, 0xAAAA_AAAA_AAAA_AAAA]
        );
        assert_eq!(f.cofactor(7, true), v0);
        assert_eq!(f.cofactor(7, false), TruthTable::constant(8, false));
        assert_eq!(f.cofactor(0, true), v7);
        assert!(f.depends_on(7));
        assert!(!f.depends_on(3));
        assert_eq!(f.support(), 0b1000_0001);
        assert_eq!(f.restrict(0b1000_0001).as_u64(), 0x8);
        assert_eq!(v6.extend(9).words().len(), 8);
        assert_eq!(v0.not().count_ones(), 128);
        let from = TruthTable::from_words(8, vec![1, 2, 3, 4]);
        assert!(from.bit(0));
        assert!(from.bit(65));
        assert!(!from.bit(1));
        assert_eq!(words_for(12), 64);
    }

    #[test]
    fn npn_classes_of_two_variables() {
        // Two-variable functions fall into 4 NPN classes: const, x, and, xor.
        let mut classes = std::collections::BTreeSet::new();
        for tt in 0..16u64 {
            let (c, perm, neg, out_neg) = npn_canonical(&TruthTable::from_u64(2, tt));
            let t = TruthTable::from_u64(2, tt)
                .negate_inputs(neg)
                .permute(&perm);
            let t = if out_neg { t.not() } else { t };
            assert_eq!(t, c);
            classes.insert(c.as_u64());
        }
        assert_eq!(classes.len(), 4);
        let mut p = vec![0, 1, 2];
        let mut seen = 1;
        while next_permutation(&mut p) {
            seen += 1;
        }
        assert_eq!(seen, 6);
        assert!(!next_permutation(&mut [0]));
    }
}
