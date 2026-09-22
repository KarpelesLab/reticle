//! Multipliers: partial products, how they are reduced, and the final
//! adder.
//!
//! Every multiplier here is the same three steps. **Generate** a set of
//! partial products whose sum is the answer; **reduce** them to two rows
//! with adders that do not carry sideways; **add** those two rows with
//! one of [`super::adder`]'s architectures. The architectures differ in
//! the first two steps:
//!
//! | Architecture | Partial products | Reduction |
//! |--------------|------------------|-----------|
//! | [`MultiplierArch::Array`] | one row per multiplier bit | added one row at a time, into an accumulator |
//! | [`MultiplierArch::Booth4`] | one row per *two* multiplier bits, radix-4 recoding | added one row at a time |
//! | [`MultiplierArch::Wallace`] | one row per multiplier bit | carry-save tree, every column reduced as far as it goes at each stage |
//! | [`MultiplierArch::Dadda`] | one row per multiplier bit | carry-save tree, each column reduced only as far as the next stage needs |
//!
//! Array is the smallest and the deepest. Booth halves the number of
//! rows, and with them the depth of the accumulation, at the cost of
//! recoding logic that only pays back in cells at around 32 bits.
//! Wallace and Dadda are logarithmic in depth; Dadda never places an
//! adder the depth did not require, so it is no larger and no deeper
//! than Wallace, which the measured table in `docs/arithmetic.md`
//! confirms at every width.
//!
//! # Signed multiplication
//!
//! Truncated to the width of its operands, a product does not care
//! about signedness: `a * b mod 2^n` is the same function of the bit
//! patterns either way, which is why the generic `mul` cell has no sign.
//! A *full* `n + m` bit product does care, and this module uses the
//! Baugh-Wooley transformation rather than sign-extending the operands:
//! the two rows that would be subtracted are complemented bit by bit and
//! three constant ones are added, which leaves the partial product
//! matrix the same shape as the unsigned one. Writing
//! `A = -a_{n-1}2^{n-1} + Σ a_i 2^i` and expanding `A·B`:
//!
//! ```text
//! A·B = Σ_{i<n-1, j<m-1} a_i b_j 2^{i+j}
//!     + Σ_{i<n-1} ~(a_i b_{m-1}) 2^{i+m-1}
//!     + Σ_{j<m-1} ~(a_{n-1} b_j) 2^{n-1+j}
//!     + a_{n-1} b_{m-1} 2^{n+m-2}
//!     + 2^{m-1} + 2^{n-1} + 2^{n+m-1}        (mod 2^{n+m})
//! ```
//!
//! So the only difference from the unsigned matrix is that the last row
//! and the last column are inverted, and a constant is added. Truncating
//! the product is then just dropping the columns above the requested
//! width.
//!
//! References: C. S. Wallace, "A Suggestion for a Fast Multiplier",
//! *IEEE Transactions on Electronic Computers* EC-13, 1964; L. Dadda,
//! "Some Schemes for Parallel Multipliers", *Alta Frequenza* 34, 1965;
//! A. D. Booth, "A Signed Binary Multiplication Technique", *Quarterly
//! Journal of Mechanics and Applied Mathematics* 4(2), 1951, in the
//! radix-4 form of O. L. MacSorley, "High-Speed Arithmetic in Binary
//! Computers", *Proceedings of the IRE* 49, 1961; C. R. Baugh and B. A.
//! Wooley, "A Two's Complement Parallel Array Multiplication
//! Algorithm", *IEEE Transactions on Computers* C-22, 1973.

use super::adder::{self, AdderOptions};
use super::bits::GateBuilder;
use crate::ir::ExprId;

/// How a multiplier's partial products are made and reduced.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MultiplierArch {
    /// One row per multiplier bit, accumulated one row at a time.
    #[default]
    Array,
    /// Radix-4 Booth recoding, halving the number of rows.
    Booth4,
    /// A Wallace carry-save tree.
    Wallace,
    /// A Dadda carry-save tree.
    Dadda,
}

impl MultiplierArch {
    /// Every architecture, in a fixed order.
    pub const ALL: [MultiplierArch; 4] = [
        MultiplierArch::Array,
        MultiplierArch::Booth4,
        MultiplierArch::Wallace,
        MultiplierArch::Dadda,
    ];

    /// The name used in reports and in an `arith` attribute.
    pub fn name(self) -> &'static str {
        match self {
            MultiplierArch::Array => "array",
            MultiplierArch::Booth4 => "booth4",
            MultiplierArch::Wallace => "wallace",
            MultiplierArch::Dadda => "dadda",
        }
    }

    /// The architecture with the given name, if it is one.
    pub fn from_name(name: &str) -> Option<MultiplierArch> {
        MultiplierArch::ALL.into_iter().find(|a| a.name() == name)
    }
}

/// A multiplier architecture and the adder that finishes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct MultiplierOptions {
    /// Which architecture to build.
    pub arch: MultiplierArch,
    /// The adder used for the final addition (and for every row
    /// addition of [`MultiplierArch::Array`] and
    /// [`MultiplierArch::Booth4`]).
    pub adder: AdderOptions,
}

impl MultiplierOptions {
    /// The options for `arch` with the default adder.
    pub fn new(arch: MultiplierArch) -> MultiplierOptions {
        MultiplierOptions {
            arch,
            adder: AdderOptions::default(),
        }
    }
}

/// The low `width` bits of `a * b`.
///
/// With `width` equal to the operand width the result is the generic
/// `mul` cell's, for which `signed` makes no difference. With `width`
/// equal to `a.len() + b.len()` it is the full product, where it does.
///
/// # Panics
///
/// Panics when either operand is empty.
pub fn multiply(
    cx: &mut GateBuilder<'_>,
    options: &MultiplierOptions,
    a: &[ExprId],
    b: &[ExprId],
    signed: bool,
    width: usize,
) -> Vec<ExprId> {
    assert!(
        !a.is_empty() && !b.is_empty(),
        "a multiplier needs both operands"
    );
    if width == 0 {
        return Vec::new();
    }
    match options.arch {
        MultiplierArch::Array => array(cx, &options.adder, a, b, signed, width),
        MultiplierArch::Booth4 => booth4(cx, &options.adder, a, b, signed, width),
        MultiplierArch::Wallace => tree(cx, options, a, b, signed, width, false),
        MultiplierArch::Dadda => tree(cx, options, a, b, signed, width, true),
    }
}

/// One row of the partial product matrix, and where it starts.
struct Row {
    /// The row's bits, least significant first.
    bits: Vec<ExprId>,
    /// The column the row's least significant bit sits in.
    shift: usize,
}

/// The rows of the (possibly Baugh-Wooley corrected) matrix, plus the
/// constant that correction adds, as bits of weight `2^i`.
///
/// Bit `(i, j)` is `a_i & b_j`, complemented when exactly one of the two
/// indices is the operand's most significant — which is the whole of the
/// signed correction bar the constant. Two of the three constant terms
/// land in the same column when the operands are equally wide, so they
/// are summed with carries rather than merely set.
fn rows(
    cx: &mut GateBuilder<'_>,
    a: &[ExprId],
    b: &[ExprId],
    signed: bool,
    width: usize,
) -> (Vec<Row>, Vec<bool>) {
    let (n, m) = (a.len(), b.len());
    let mut out = Vec::with_capacity(m);
    for (j, &bj) in b.iter().enumerate() {
        if j >= width {
            break;
        }
        let len = (width - j).min(n);
        let mut bits = Vec::with_capacity(len);
        for &ai in a.iter().take(len) {
            bits.push(cx.and(ai, bj));
        }
        if signed {
            for (i, bit) in bits.iter_mut().enumerate() {
                if (i + 1 == n) != (j + 1 == m) {
                    *bit = cx.not(*bit);
                }
            }
        }
        out.push(Row { bits, shift: j });
    }
    let mut constant = vec![false; width];
    if signed {
        for c in [m - 1, n - 1, n + m - 1] {
            add_power_of_two(&mut constant, c);
        }
    }
    (out, constant)
}

/// Adds `2^index` to a little-endian bit vector, discarding a carry out
/// of the top — which is the truncation the product wants anyway.
fn add_power_of_two(bits: &mut [bool], index: usize) {
    let mut i = index;
    while i < bits.len() {
        if !bits[i] {
            bits[i] = true;
            return;
        }
        bits[i] = false;
        i += 1;
    }
}

/// The shift-and-add array: rows accumulated one at a time.
fn array(
    cx: &mut GateBuilder<'_>,
    adder: &AdderOptions,
    a: &[ExprId],
    b: &[ExprId],
    signed: bool,
    width: usize,
) -> Vec<ExprId> {
    let (rows, constant) = rows(cx, a, b, signed, width);
    let mut acc = cx.zeros(width);
    for (c, set) in constant.iter().enumerate() {
        if *set {
            acc[c] = cx.one();
        }
    }
    for row in rows {
        add_row(cx, adder, &mut acc, &row.bits, row.shift, None);
    }
    acc
}

/// `acc[shift..] += row`, with an optional carry in.
fn add_row(
    cx: &mut GateBuilder<'_>,
    adder: &AdderOptions,
    acc: &mut [ExprId],
    row: &[ExprId],
    shift: usize,
    carry_in: Option<ExprId>,
) {
    if shift >= acc.len() {
        return;
    }
    let len = acc.len() - shift;
    let zero = cx.zero();
    let addend = cx.extend(row, len, zero);
    let cin = carry_in.unwrap_or(zero);
    let sum = adder::add(cx, adder, &acc[shift..], &addend, cin);
    acc[shift..].copy_from_slice(&sum.bits);
}

/// Radix-4 Booth recoding: one row per two multiplier bits.
///
/// Digit `t` is `-2·b_{2t+1} + b_{2t} + b_{2t-1}` and lies in
/// `{-2, -1, 0, 1, 2}`, so the row is zero, `±a` or `±2a`, all of which
/// are a selection away. A negative digit is built as the bitwise
/// complement plus one, and the plus one rides in as the row addition's
/// carry in, which costs nothing.
fn booth4(
    cx: &mut GateBuilder<'_>,
    adder: &AdderOptions,
    a: &[ExprId],
    b: &[ExprId],
    signed: bool,
    width: usize,
) -> Vec<ExprId> {
    // The multiplier, extended so its signed reading is its value and
    // its length is even.
    let mut len = if signed { b.len() } else { b.len() + 1 };
    if len % 2 == 1 {
        len += 1;
    }
    let bx = cx.resize(b, len, signed);
    let multiplicand = cx.resize(a, width, signed);
    let zero = cx.zero();
    let mut shifted = vec![zero];
    shifted.extend(multiplicand.iter().take(width - 1).copied());

    let mut acc = cx.zeros(width);
    for t in 0..len / 2 {
        let shift = 2 * t;
        if shift >= width {
            break;
        }
        let low = if t == 0 { zero } else { bx[shift - 1] };
        let mid = bx[shift];
        let high = bx[shift + 1];
        let sel1 = cx.xor(mid, low);
        let sel2 = {
            let nmid = cx.not(mid);
            let nlow = cx.not(low);
            let nhigh = cx.not(high);
            let two_up = {
                let t0 = cx.and(nmid, nlow);
                cx.and(high, t0)
            };
            let two_down = {
                let t0 = cx.and(mid, low);
                cx.and(nhigh, t0)
            };
            cx.or(two_up, two_down)
        };
        let negate = high;
        let row_len = width - shift;
        let row: Vec<ExprId> = (0..row_len)
            .map(|i| {
                let one_x = cx.and(sel1, multiplicand[i]);
                let two_x = cx.and(sel2, shifted[i]);
                let m = cx.or(one_x, two_x);
                cx.xor(m, negate)
            })
            .collect();
        add_row(cx, adder, &mut acc, &row, shift, Some(negate));
    }
    acc
}

/// A carry-save tree over the partial product matrix, Wallace or Dadda,
/// finished with the chosen adder.
fn tree(
    cx: &mut GateBuilder<'_>,
    options: &MultiplierOptions,
    a: &[ExprId],
    b: &[ExprId],
    signed: bool,
    width: usize,
    dadda: bool,
) -> Vec<ExprId> {
    let (rows, constant) = rows(cx, a, b, signed, width);
    let mut columns: Vec<Vec<ExprId>> = vec![Vec::new(); width];
    for row in &rows {
        for (i, &bit) in row.bits.iter().enumerate() {
            let c = row.shift + i;
            if c < width {
                columns[c].push(bit);
            }
        }
    }
    for (c, set) in constant.iter().enumerate() {
        if *set {
            let one = cx.one();
            columns[c].push(one);
        }
    }
    if dadda {
        dadda_reduce(cx, &mut columns);
    } else {
        wallace_reduce(cx, &mut columns);
    }
    let zero = cx.zero();
    let first: Vec<ExprId> = columns
        .iter()
        .map(|c| c.first().copied().unwrap_or(zero))
        .collect();
    let second: Vec<ExprId> = columns
        .iter()
        .map(|c| c.get(1).copied().unwrap_or(zero))
        .collect();
    adder::add(cx, &options.adder, &first, &second, zero).bits
}

/// A full adder: three bits of one column become one bit of it and one
/// of the next.
fn full_adder(cx: &mut GateBuilder<'_>, a: ExprId, b: ExprId, c: ExprId) -> (ExprId, ExprId) {
    let p = cx.xor(a, b);
    let sum = cx.xor(p, c);
    let carry = cx.mux(p, c, a);
    (sum, carry)
}

/// A half adder: two bits of one column become one bit of it and one of
/// the next.
fn half_adder(cx: &mut GateBuilder<'_>, a: ExprId, b: ExprId) -> (ExprId, ExprId) {
    let sum = cx.xor(a, b);
    let carry = cx.and(a, b);
    (sum, carry)
}

/// Wallace reduction: at every stage each column is packed into as many
/// full adders as it holds, with a half adder for a leftover pair.
///
/// The stages are as short as they can be, and the cost is more adders
/// than Dadda needs for the same depth.
fn wallace_reduce(cx: &mut GateBuilder<'_>, columns: &mut Vec<Vec<ExprId>>) {
    let width = columns.len();
    while columns.iter().map(Vec::len).max().unwrap_or(0) > 2 {
        let mut next: Vec<Vec<ExprId>> = vec![Vec::new(); width + 1];
        for (c, column) in columns.iter().enumerate() {
            let mut i = 0;
            while i + 3 <= column.len() {
                let (s, carry) = full_adder(cx, column[i], column[i + 1], column[i + 2]);
                next[c].push(s);
                next[c + 1].push(carry);
                i += 3;
            }
            if i + 2 <= column.len() {
                let (s, carry) = half_adder(cx, column[i], column[i + 1]);
                next[c].push(s);
                next[c + 1].push(carry);
                i += 2;
            }
            if i < column.len() {
                next[c].push(column[i]);
            }
        }
        next.truncate(width);
        *columns = next;
    }
}

/// The Dadda heights: `2, 3, 4, 6, 9, 13, …`, each the largest number
/// of bits a stage may leave behind.
fn dadda_heights(max: usize) -> Vec<usize> {
    let mut out = vec![2usize];
    while *out.last().expect("non-empty") < max {
        let next = out.last().expect("non-empty") * 3 / 2;
        out.push(next);
    }
    out.pop();
    out
}

/// Dadda reduction: each stage reduces every column only as far as the
/// next stage's height allows, so no adder is placed that the depth did
/// not require.
fn dadda_reduce(cx: &mut GateBuilder<'_>, columns: &mut [Vec<ExprId>]) {
    let width = columns.len();
    let max = columns.iter().map(Vec::len).max().unwrap_or(0);
    for &target in dadda_heights(max).iter().rev() {
        for c in 0..width {
            while columns[c].len() > target {
                let excess = columns[c].len() - target;
                let (sum, carry) = if excess >= 2 && columns[c].len() >= 3 {
                    let three: Vec<ExprId> = columns[c].drain(..3).collect();
                    full_adder(cx, three[0], three[1], three[2])
                } else {
                    let two: Vec<ExprId> = columns[c].drain(..2).collect();
                    half_adder(cx, two[0], two[1])
                };
                columns[c].push(sum);
                if c + 1 < width {
                    columns[c + 1].push(carry);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::arith::AdderArch;
    use crate::synth::arith::testkit::{exhaustive, signed, unsigned};

    /// Every architecture multiplies, truncated and in full, signed and
    /// unsigned, over every pair of operands.
    #[test]
    fn every_architecture_multiplies() {
        for arch in MultiplierArch::ALL {
            let options = MultiplierOptions::new(arch);
            for w in 1..=4u32 {
                for full in [false, true] {
                    let out = crate::synth::arith::bits::to_usize(if full { 2 * w } else { w });
                    exhaustive(
                        &format!(
                            "{} multiplier, {w} bits, full {full}, unsigned",
                            arch.name()
                        ),
                        &[w, w],
                        |cx, ins| multiply(cx, &options, &ins[0], &ins[1], false, out),
                        |v| v[0].wrapping_mul(v[1]),
                    );
                    exhaustive(
                        &format!("{} multiplier, {w} bits, full {full}, signed", arch.name()),
                        &[w, w],
                        |cx, ins| multiply(cx, &options, &ins[0], &ins[1], true, out),
                        move |v| unsigned(signed(v[0], w) * signed(v[1], w), 64),
                    );
                }
            }
        }
    }

    /// The final adder is chosen independently of the reduction, so
    /// every pairing is checked.
    #[test]
    fn every_final_adder_finishes_every_tree() {
        for arch in MultiplierArch::ALL {
            for adder in AdderArch::ALL {
                let options = MultiplierOptions {
                    arch,
                    adder: AdderOptions::new(adder),
                };
                exhaustive(
                    &format!("{} multiplier over a {} adder", arch.name(), adder.name()),
                    &[4, 4],
                    |cx, ins| multiply(cx, &options, &ins[0], &ins[1], true, 8),
                    |v| unsigned(signed(v[0], 4) * signed(v[1], 4), 64),
                );
            }
        }
    }

    /// A multiply by a constant folds against the constant instead of
    /// building a row of AND gates that compute nothing.
    #[test]
    fn a_constant_operand_folds() {
        use crate::ir::Type;
        use crate::ir::builder::ModuleBuilder;
        use crate::synth::arith::bits::GateBuilder;
        use crate::synth::arith::testkit::span;

        let mut b = ModuleBuilder::new("m", span());
        let a = b.input("a", Type::bits(8));
        let av = b.net(a);
        let four = b.const_u64(8, 4);
        let cells = {
            let mut cx = GateBuilder::new(&mut b, "u");
            let ab = cx.split(av, 8);
            let bb = cx.split(four, 8);
            let options = MultiplierOptions::new(MultiplierArch::Array);
            let _ = multiply(&mut cx, &options, &ab, &bb, false, 8);
            cx.cell_count()
        };
        assert_eq!(
            cells, 0,
            "a multiply by four is a shift, and a shift by a constant is wiring"
        );
    }

    #[test]
    fn architecture_names_round_trip() {
        for arch in MultiplierArch::ALL {
            assert_eq!(MultiplierArch::from_name(arch.name()), Some(arch));
        }
        assert_eq!(MultiplierArch::from_name("kogge_stone"), None);
        assert_eq!(MultiplierOptions::default().arch, MultiplierArch::Array);
    }

    #[test]
    fn dadda_heights_are_the_sequence() {
        assert_eq!(dadda_heights(2), Vec::<usize>::new());
        assert_eq!(dadda_heights(3), vec![2]);
        assert_eq!(dadda_heights(4), vec![2, 3]);
        assert_eq!(dadda_heights(16), vec![2, 3, 4, 6, 9, 13]);
    }
}
