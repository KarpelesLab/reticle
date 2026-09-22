//! Adders and subtractors, five architectures.
//!
//! Every function here takes two equal-length bit vectors (least
//! significant bit first) and a carry in, and returns the sum and the
//! carry out, so a wider adder can be built by chaining two narrower
//! ones and an accumulator can reuse the carry. All five compute exactly
//! the same function; they differ only in how the carry chain is
//! evaluated, which is the whole point of offering a choice.
//!
//! | Architecture | Cells | Depth | When |
//! |--------------|-------|-------|------|
//! | [`ripple_carry`] | `3n` | `n + 1` | the default: smallest, and on an FPGA the carry chain is dedicated silicon |
//! | [`carry_select`] | `~6n + n/k` | `k + n/k` | a middle ground with one knob, the block size `k` |
//! | [`carry_lookahead`] | `O(n·k)` | `~2 log_k n · log k` | the classical group lookahead, group size `k` |
//! | [`kogge_stone`] | `O(n log n)` | `2 log n + 2` | the shallowest; use when the adder is the critical path |
//! | [`brent_kung`] | `O(n)` | `4 log n` | most of Kogge-Stone's depth win at half the cells, from 16 bits up |
//!
//! The cell and depth columns are orders of magnitude; the measured
//! numbers are in `docs/arithmetic.md`, generated from
//! `tests/synth_arith.rs` so they cannot drift.
//!
//! # The carry recurrence
//!
//! With `g_i = a_i & b_i` (this bit makes a carry) and `p_i = a_i ^ b_i`
//! (this bit passes one on), `c_{i+1} = g_i | p_i & c_i` and
//! `s_i = p_i ^ c_i`. The ripple adder evaluates that recurrence one
//! step at a time; [`super::prefix`] evaluates it as an associative scan,
//! which is what the last two rows of the table are. The carry-select
//! adder instead computes each block twice, once for each possible
//! incoming carry, and picks afterwards; the lookahead adder expands the
//! recurrence within a group into a two-level sum of products and
//! applies the same trick to the groups.
//!
//! References: J. L. Hennessy and D. A. Patterson, *Computer
//! Architecture: A Quantitative Approach*, appendix on computer
//! arithmetic (I. Koren, *Computer Arithmetic Algorithms*, chapters 5
//! and 6, for the prefix networks); O. J. Bedrij, "Carry-Select Adder",
//! *IRE Transactions on Electronic Computers* EC-11, 1962.

use super::bits::{GateBuilder, to_usize};
use super::prefix::{self, Prefix};
use crate::ir::ExprId;

/// How the carry chain of an adder or subtractor is evaluated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AdderArch {
    /// One full adder per bit, carries chained ([`ripple_carry`]).
    #[default]
    Ripple,
    /// Blocks computed for both incoming carries and selected
    /// ([`carry_select`]).
    CarrySelect,
    /// Group carry lookahead ([`carry_lookahead`]).
    CarryLookahead,
    /// Fully parallel prefix network ([`kogge_stone`]).
    KoggeStone,
    /// Reduce-then-scatter prefix network ([`brent_kung`]).
    BrentKung,
}

impl AdderArch {
    /// Every architecture, in a fixed order, for tables and tests.
    pub const ALL: [AdderArch; 5] = [
        AdderArch::Ripple,
        AdderArch::CarrySelect,
        AdderArch::CarryLookahead,
        AdderArch::KoggeStone,
        AdderArch::BrentKung,
    ];

    /// The name used in reports and in an `arith` attribute.
    pub fn name(self) -> &'static str {
        match self {
            AdderArch::Ripple => "ripple",
            AdderArch::CarrySelect => "carry_select",
            AdderArch::CarryLookahead => "carry_lookahead",
            AdderArch::KoggeStone => "kogge_stone",
            AdderArch::BrentKung => "brent_kung",
        }
    }

    /// The architecture with the given name, if it is one.
    pub fn from_name(name: &str) -> Option<AdderArch> {
        AdderArch::ALL.into_iter().find(|a| a.name() == name)
    }
}

/// An adder architecture and the knobs that shape it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdderOptions {
    /// Which architecture to build.
    pub arch: AdderArch,
    /// Bits per block of [`AdderArch::CarrySelect`]; clamped to at least
    /// one. Four is a reasonable default: the depth is roughly
    /// `k + n/k`, minimised near `sqrt(n)`.
    pub block_size: u32,
    /// Bits per group of [`AdderArch::CarryLookahead`]; clamped to at
    /// least two. Four is the classical group size, and the one the
    /// 74x181 / 74x182 pair implements in hardware.
    pub group_size: u32,
}

impl Default for AdderOptions {
    fn default() -> Self {
        AdderOptions {
            arch: AdderArch::Ripple,
            block_size: 4,
            group_size: 4,
        }
    }
}

impl AdderOptions {
    /// The options for `arch` with the default block and group sizes.
    pub fn new(arch: AdderArch) -> AdderOptions {
        AdderOptions {
            arch,
            ..AdderOptions::default()
        }
    }
}

/// What an adder produced.
#[derive(Clone, Debug)]
pub struct Sum {
    /// The sum, least significant bit first, as wide as the operands.
    pub bits: Vec<ExprId>,
    /// The carry out of the most significant bit.
    pub carry_out: ExprId,
}

/// What a subtractor produced.
#[derive(Clone, Debug)]
pub struct Difference {
    /// The difference, least significant bit first.
    pub bits: Vec<ExprId>,
    /// True when the subtraction borrowed, that is when `a` was below
    /// `b` read as unsigned values.
    pub borrow_out: ExprId,
}

/// `a + b + cin` in the architecture `options` asks for.
///
/// # Panics
///
/// Panics when `a` and `b` have different lengths.
pub fn add(
    cx: &mut GateBuilder<'_>,
    options: &AdderOptions,
    a: &[ExprId],
    b: &[ExprId],
    cin: ExprId,
) -> Sum {
    assert_eq!(a.len(), b.len(), "adder operands must have equal width");
    match options.arch {
        AdderArch::Ripple => ripple_carry(cx, a, b, cin),
        AdderArch::CarrySelect => carry_select(cx, a, b, cin, to_usize(options.block_size.max(1))),
        AdderArch::CarryLookahead => {
            carry_lookahead(cx, a, b, cin, to_usize(options.group_size.max(2)))
        }
        AdderArch::KoggeStone => kogge_stone(cx, a, b, cin),
        AdderArch::BrentKung => brent_kung(cx, a, b, cin),
    }
}

/// `a - b - borrow_in`, built as `a + !b + !borrow_in` in the
/// architecture `options` asks for.
///
/// The borrow out is the complement of the adder's carry out, so an
/// unsigned comparison falls out of the same network: `a < b` is exactly
/// `borrow_out` with no borrow in.
///
/// # Panics
///
/// Panics when `a` and `b` have different lengths.
pub fn subtract(
    cx: &mut GateBuilder<'_>,
    options: &AdderOptions,
    a: &[ExprId],
    b: &[ExprId],
    borrow_in: ExprId,
) -> Difference {
    assert_eq!(
        a.len(),
        b.len(),
        "subtractor operands must have equal width"
    );
    let nb: Vec<ExprId> = b.iter().map(|&x| cx.not(x)).collect();
    let cin = cx.not(borrow_in);
    let sum = add(cx, options, a, &nb, cin);
    let borrow_out = cx.not(sum.carry_out);
    Difference {
        bits: sum.bits,
        borrow_out,
    }
}

/// The generate and propagate of every bit position.
fn generate_propagate(
    cx: &mut GateBuilder<'_>,
    a: &[ExprId],
    b: &[ExprId],
) -> (Vec<ExprId>, Vec<ExprId>) {
    let g = a.iter().zip(b).map(|(&x, &y)| cx.and(x, y)).collect();
    let p = a.iter().zip(b).map(|(&x, &y)| cx.xor(x, y)).collect();
    (g, p)
}

/// The sum bits from the propagates and the carry into each bit.
fn sum_bits(cx: &mut GateBuilder<'_>, p: &[ExprId], carry_in: &[ExprId]) -> Vec<ExprId> {
    p.iter()
        .zip(carry_in)
        .map(|(&pi, &ci)| cx.xor(pi, ci))
        .collect()
}

/// A chain of full adders: `c_{i+1} = p_i ? c_i : a_i`.
///
/// The carry out of a full adder is the majority of its three inputs,
/// which is a single multiplexer once the propagate is known — the same
/// one bit of logic per stage that an FPGA's dedicated carry chain
/// implements. Three cells and one level of carry per bit, which is why
/// this is the area-oriented default despite the linear depth.
pub fn ripple_carry(cx: &mut GateBuilder<'_>, a: &[ExprId], b: &[ExprId], cin: ExprId) -> Sum {
    let p: Vec<ExprId> = a.iter().zip(b).map(|(&x, &y)| cx.xor(x, y)).collect();
    ripple_from_propagate(cx, a, &p, cin)
}

/// The carry chain of a ripple adder over propagates that are already
/// built, so a carry-select block can evaluate both of its carry
/// assumptions without building them twice.
fn ripple_from_propagate(cx: &mut GateBuilder<'_>, a: &[ExprId], p: &[ExprId], cin: ExprId) -> Sum {
    let mut carry = cin;
    let mut bits = Vec::with_capacity(p.len());
    for (i, &pi) in p.iter().enumerate() {
        let s = cx.xor(pi, carry);
        // p ? carry : a — when the propagate is zero the operands agree
        // and the carry out is that common value.
        carry = cx.mux(pi, carry, a[i]);
        bits.push(s);
    }
    Sum {
        bits,
        carry_out: carry,
    }
}

/// Blocks of `block` bits, each computed for both possible incoming
/// carries and selected when the real one arrives.
///
/// The first block sees the true carry in and is an ordinary ripple.
/// Every later block adds twice — speculatively with a carry in of zero
/// and of one — and a row of multiplexers picks between the two. The
/// carry then crosses the whole adder through one multiplexer per block
/// instead of one per bit, so the depth falls from `n` to about
/// `block + n/block` for roughly twice the cells.
///
/// # Panics
///
/// Panics when `block` is zero.
pub fn carry_select(
    cx: &mut GateBuilder<'_>,
    a: &[ExprId],
    b: &[ExprId],
    cin: ExprId,
    block: usize,
) -> Sum {
    assert!(block > 0, "the carry-select block size must be positive");
    let n = a.len();
    let mut bits: Vec<ExprId> = Vec::with_capacity(n);
    let mut carry = cin;
    let mut start = 0usize;
    let mut first = true;
    while start < n {
        let end = (start + block).min(n);
        let (ba, bb) = (&a[start..end], &b[start..end]);
        let p: Vec<ExprId> = ba.iter().zip(bb).map(|(&x, &y)| cx.xor(x, y)).collect();
        if first {
            let s = ripple_from_propagate(cx, ba, &p, carry);
            bits.extend(s.bits);
            carry = s.carry_out;
            first = false;
        } else {
            let zero = cx.zero();
            let one = cx.one();
            // Both assumptions share the propagates, so a block costs
            // two carry chains rather than two whole adders.
            let s0 = ripple_from_propagate(cx, ba, &p, zero);
            let s1 = ripple_from_propagate(cx, ba, &p, one);
            for (&x0, &x1) in s0.bits.iter().zip(&s1.bits) {
                let v = cx.mux(carry, x1, x0);
                bits.push(v);
            }
            carry = cx.mux(carry, s1.carry_out, s0.carry_out);
        }
        start = end;
    }
    Sum {
        bits,
        carry_out: carry,
    }
}

/// A span's own generate and propagate, written out as a two-level sum
/// of products: does it make a carry by itself, and does it pass an
/// incoming one through?
///
/// `G = g_{k-1} | p_{k-1} g_{k-2} | … | p_{k-1}…p_1 g_0` and
/// `P = p_{k-1} … p_0`. Every product and the sum are balanced trees of
/// two-input gates, so the depth is `O(log k)` whatever `k` is — at the
/// cost of `O(k²)` cells for one span, which is why the lookahead
/// groups are small and why a group size above four is rarely worth it.
fn span_generate_propagate(
    cx: &mut GateBuilder<'_>,
    p: &[ExprId],
    g: &[ExprId],
) -> (ExprId, ExprId) {
    let k = p.len();
    let mut terms: Vec<ExprId> = Vec::with_capacity(k);
    for m in 0..k {
        let mut factors: Vec<ExprId> = Vec::with_capacity(k - m);
        factors.push(g[m]);
        factors.extend_from_slice(&p[m + 1..]);
        let term = cx.and_all(&factors);
        terms.push(term);
    }
    let span_g = cx.or_all(&terms);
    let span_p = cx.and_all(p);
    (span_g, span_p)
}

/// The carries out of bits `0 .. upto` of one group.
///
/// Each is `G_j | P_j & c_in` over the span `0 ..= j`, and the spans do
/// not depend on the carry in, so they are all built in parallel with
/// whatever produces it: the carry in adds two gate levels and no more,
/// however wide the group.
fn flat_carries(
    cx: &mut GateBuilder<'_>,
    p: &[ExprId],
    g: &[ExprId],
    cin: ExprId,
    upto: usize,
) -> Vec<ExprId> {
    let mut out = Vec::with_capacity(upto);
    for j in 0..upto {
        let (span_g, span_p) = span_generate_propagate(cx, &p[..=j], &g[..=j]);
        let through = cx.and(span_p, cin);
        let c = cx.or(span_g, through);
        out.push(c);
    }
    out
}

/// Group carry lookahead with a configurable group size.
///
/// Bits are cut into groups of `group`. Each group's generate and
/// propagate are formed first; a lookahead over *those* — the same
/// construction applied recursively — produces the carry into every
/// group; and each group then expands its internal carries from its own
/// carry in. Two levels of logic per level of the hierarchy, so the
/// depth is `O(log_k n · log k)` and the cell count `O(n·k)`.
///
/// # Panics
///
/// Panics when `group` is below two, which would not terminate.
pub fn carry_lookahead(
    cx: &mut GateBuilder<'_>,
    a: &[ExprId],
    b: &[ExprId],
    cin: ExprId,
    group: usize,
) -> Sum {
    assert!(group >= 2, "the lookahead group size must be at least two");
    let (g, p) = generate_propagate(cx, a, b);
    let carries = lookahead_carries(cx, &p, &g, cin, group);
    let mut carry_in = vec![cin];
    carry_in.extend(carries.iter().take(p.len().saturating_sub(1)).copied());
    let bits = sum_bits(cx, &p, &carry_in);
    let carry_out = carries.last().copied().unwrap_or(cin);
    Sum { bits, carry_out }
}

/// The carry out of every bit, by recursive group lookahead.
fn lookahead_carries(
    cx: &mut GateBuilder<'_>,
    p: &[ExprId],
    g: &[ExprId],
    cin: ExprId,
    group: usize,
) -> Vec<ExprId> {
    let n = p.len();
    if n <= group {
        return flat_carries(cx, p, g, cin, n);
    }
    let bounds: Vec<(usize, usize)> = (0..n.div_ceil(group))
        .map(|i| (i * group, ((i + 1) * group).min(n)))
        .collect();
    let mut gp = Vec::with_capacity(bounds.len());
    let mut gg = Vec::with_capacity(bounds.len());
    for &(lo, hi) in &bounds {
        let (group_g, group_p) = span_generate_propagate(cx, &p[lo..hi], &g[lo..hi]);
        gg.push(group_g);
        gp.push(group_p);
    }
    let group_carries = lookahead_carries(cx, &gp, &gg, cin, group);
    let mut out = vec![cin; n];
    for (i, &(lo, hi)) in bounds.iter().enumerate() {
        let group_cin = if i == 0 { cin } else { group_carries[i - 1] };
        // The last carry of the group is the group carry the level above
        // already produced, so only the internal ones are expanded here.
        let inner = flat_carries(cx, &p[lo..hi], &g[lo..hi], group_cin, hi - lo - 1);
        out[lo..hi - 1].copy_from_slice(&inner);
        out[hi - 1] = group_carries[i];
    }
    out
}

/// The prefix columns of an addition: the carry in as column zero,
/// followed by one column per bit.
///
/// Column zero is `(g, p) = (cin, 0)`, which is the span that generates
/// the carry in and propagates nothing. The generate of the prefix
/// ending at column `i` is therefore exactly the carry into bit `i`.
fn carry_columns(cx: &mut GateBuilder<'_>, g: &[ExprId], p: &[ExprId], cin: ExprId) -> Vec<Prefix> {
    let zero = cx.zero();
    let mut items = Vec::with_capacity(g.len() + 1);
    items.push(Prefix::new(cin, zero));
    items.extend(g.iter().zip(p).map(|(&gi, &pi)| Prefix::new(gi, pi)));
    items
}

/// Builds a sum from a prefix network over the carry columns.
fn prefix_sum(
    cx: &mut GateBuilder<'_>,
    a: &[ExprId],
    b: &[ExprId],
    cin: ExprId,
    network: fn(&mut GateBuilder<'_>, &[Prefix]) -> Vec<Prefix>,
) -> Sum {
    let (g, p) = generate_propagate(cx, a, b);
    let items = carry_columns(cx, &g, &p, cin);
    let scan = network(cx, &items);
    let carry_in: Vec<ExprId> = scan.iter().take(p.len()).map(|c| c.g).collect();
    let bits = sum_bits(cx, &p, &carry_in);
    let carry_out = scan.last().map_or(cin, |c| c.g);
    Sum { bits, carry_out }
}

/// A Kogge-Stone prefix adder: logarithmic depth, `O(n log n)` cells.
///
/// The shallowest of the five and the one to ask for when the adder is
/// on the critical path. See [`super::prefix::kogge_stone`].
pub fn kogge_stone(cx: &mut GateBuilder<'_>, a: &[ExprId], b: &[ExprId], cin: ExprId) -> Sum {
    prefix_sum(cx, a, b, cin, prefix::kogge_stone)
}

/// A Brent-Kung prefix adder: `O(n)` cells at twice Kogge-Stone's depth.
///
/// See [`super::prefix::brent_kung`].
pub fn brent_kung(cx: &mut GateBuilder<'_>, a: &[ExprId], b: &[ExprId], cin: ExprId) -> Sum {
    prefix_sum(cx, a, b, cin, prefix::brent_kung)
}

/// The serial evaluation of the same prefix network, for comparison in
/// the measurement table; functionally a ripple carry built out of
/// prefix cells instead of multiplexers.
pub fn prefix_serial(cx: &mut GateBuilder<'_>, a: &[ExprId], b: &[ExprId], cin: ExprId) -> Sum {
    prefix_sum(cx, a, b, cin, prefix::serial)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::arith::testkit::exhaustive;

    /// Every architecture, at every width up to five and with several
    /// block and group sizes, adds. The check is exhaustive over both
    /// operands and the carry in, and the carry out is part of the
    /// result compared, so the chaining interface is checked too.
    #[test]
    fn every_architecture_adds() {
        for arch in AdderArch::ALL {
            let sizes: &[u32] = match arch {
                AdderArch::CarrySelect | AdderArch::CarryLookahead => &[1, 2, 3, 5],
                _ => &[4],
            };
            for &size in sizes {
                for w in 1..=5u32 {
                    let options = AdderOptions {
                        arch,
                        block_size: size,
                        group_size: size.max(2),
                    };
                    exhaustive(
                        &format!("{} adder, {w} bits, size {size}", arch.name()),
                        &[w, w, 1],
                        |cx, ins| {
                            let s = add(cx, &options, &ins[0], &ins[1], ins[2][0]);
                            let mut bits = s.bits;
                            bits.push(s.carry_out);
                            bits
                        },
                        |v| v[0] + v[1] + v[2],
                    );
                }
            }
        }
    }

    /// The same for subtraction, with the borrow out compared too: it is
    /// set exactly when the difference went below zero.
    #[test]
    fn every_architecture_subtracts() {
        for arch in AdderArch::ALL {
            for w in 1..=5u32 {
                let options = AdderOptions::new(arch);
                exhaustive(
                    &format!("{} subtractor, {w} bits", arch.name()),
                    &[w, w, 1],
                    |cx, ins| {
                        let d = subtract(cx, &options, &ins[0], &ins[1], ins[2][0]);
                        let mut bits = d.bits;
                        bits.push(d.borrow_out);
                        bits
                    },
                    move |v| {
                        let d = i64::try_from(v[0]).expect("a small word")
                            - i64::try_from(v[1] + v[2]).expect("a small word");
                        let low =
                            u64::try_from(d.rem_euclid(1i64 << w)).expect("a non-negative value");
                        low | (u64::from(d < 0) << w)
                    },
                );
            }
        }
    }

    /// The serial prefix evaluation is an adder too, which is what makes
    /// it comparable with the parallel networks.
    #[test]
    fn the_serial_prefix_network_adds() {
        for w in 1..=5u32 {
            exhaustive(
                &format!("serial prefix adder, {w} bits"),
                &[w, w, 1],
                |cx, ins| {
                    let s = prefix_serial(cx, &ins[0], &ins[1], ins[2][0]);
                    let mut bits = s.bits;
                    bits.push(s.carry_out);
                    bits
                },
                |v| v[0] + v[1] + v[2],
            );
        }
    }

    #[test]
    fn architecture_names_round_trip() {
        for arch in AdderArch::ALL {
            assert_eq!(AdderArch::from_name(arch.name()), Some(arch));
        }
        assert_eq!(AdderArch::from_name("wallace"), None);
        assert_eq!(AdderOptions::default().arch, AdderArch::Ripple);
        assert_eq!(AdderOptions::new(AdderArch::BrentKung).block_size, 4);
    }
}
