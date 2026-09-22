//! Parallel prefix networks: the shape shared by the fast adders and the
//! fast comparator.
//!
//! A carry chain is a prefix computation. Write `g_i` for "bit `i`
//! generates a carry" and `p_i` for "bit `i` propagates one"; the carry
//! into bit `i` is then the generate of the prefix `(g,p)_{i-1} ∘ … ∘
//! (g,p)_0` under the associative operator
//!
//! ```text
//! (g, p) ∘ (g', p')  =  (g | p & g',  p & p')
//! ```
//!
//! Associativity is what buys the architecture choice: the same product
//! can be evaluated as a chain (ripple carry, depth `n`), as a fully
//! parallel network (Kogge-Stone, depth `log n`, many cells) or as a
//! reduce followed by a scatter (Brent-Kung, depth `2 log n`, few
//! cells). [`kogge_stone`] and [`brent_kung`] return **every** prefix,
//! which is what an adder needs; [`reduce`] returns only the last one,
//! which is all a comparator or a carry-out needs.
//!
//! The comparator uses the identical operator with `g_i` read as "the
//! operands differ here and `a` is the smaller" and `p_i` as "the
//! operands are equal here"; see [`super::cmp`]. That is not a pun: both
//! are the same scan over the same monoid.
//!
//! References:
//!
//! - P. M. Kogge and H. S. Stone, "A Parallel Algorithm for the Efficient
//!   Solution of a General Class of Recurrence Equations", *IEEE
//!   Transactions on Computers* C-22(8), 1973.
//! - R. P. Brent and H. T. Kung, "A Regular Layout for Parallel Adders",
//!   *IEEE Transactions on Computers* C-31(3), 1982.

use super::bits::GateBuilder;
use crate::ir::ExprId;

/// One column of a prefix network: a generate and a propagate bit.
#[derive(Clone, Copy, Debug)]
pub struct Prefix {
    /// True when this span produces a carry regardless of what enters it.
    pub g: ExprId,
    /// True when this span passes an incoming carry through unchanged.
    pub p: ExprId,
}

impl Prefix {
    /// A column with the given generate and propagate.
    pub fn new(g: ExprId, p: ExprId) -> Prefix {
        Prefix { g, p }
    }
}

/// The prefix operator: `hi ∘ lo`, with `hi` the more significant span.
///
/// Costs two cells (one AND, one OR) for the generate and one for the
/// propagate, so a level of a prefix network is two gate delays deep.
pub fn combine(cx: &mut GateBuilder<'_>, hi: Prefix, lo: Prefix) -> Prefix {
    let pg = cx.and(hi.p, lo.g);
    let g = cx.or(hi.g, pg);
    let p = cx.and(hi.p, lo.p);
    Prefix { g, p }
}

/// `hi ∘ lo` when only the generate of the result is ever read: the
/// propagate AND is left out, saving one cell per node.
fn combine_g(cx: &mut GateBuilder<'_>, hi: Prefix, lo_g: ExprId) -> ExprId {
    let pg = cx.and(hi.p, lo_g);
    cx.or(hi.g, pg)
}

/// Every prefix of `items`, computed as a chain: `out[i] = items[i] ∘ …
/// ∘ items[0]`, one operator per step.
///
/// This is the serial evaluation — the ripple carry of the family — and
/// is here so the fast networks have something to be compared against in
/// the same terms.
pub fn serial(cx: &mut GateBuilder<'_>, items: &[Prefix]) -> Vec<Prefix> {
    let mut out: Vec<Prefix> = Vec::with_capacity(items.len());
    for (i, &item) in items.iter().enumerate() {
        let acc = if i == 0 {
            item
        } else {
            combine(cx, item, out[i - 1])
        };
        out.push(acc);
    }
    out
}

/// Every prefix of `items` through a Kogge-Stone network.
///
/// `⌈log2 n⌉` levels, each combining every column with the one `2^k`
/// places below it, so the depth is logarithmic and the cell count is
/// `O(n log n)`. The fan-out of each node is bounded, which is why this
/// is the network of choice when speed is the only concern.
pub fn kogge_stone(cx: &mut GateBuilder<'_>, items: &[Prefix]) -> Vec<Prefix> {
    let n = items.len();
    let mut level = items.to_vec();
    let mut d = 1usize;
    while d < n {
        let mut next = level.clone();
        for i in d..n {
            next[i] = combine(cx, level[i], level[i - d]);
        }
        level = next;
        d *= 2;
    }
    level
}

/// Every prefix of `items` through a Brent-Kung network.
///
/// An up-sweep builds the prefixes at indices `2^k - 1` in place, and a
/// down-sweep fills in the rest. Roughly `2n` operator nodes against
/// Kogge-Stone's `n log n`, at twice the depth — the area-conscious end
/// of the parallel prefix family.
pub fn brent_kung(cx: &mut GateBuilder<'_>, items: &[Prefix]) -> Vec<Prefix> {
    let n = items.len();
    let mut t = items.to_vec();
    let mut strides: Vec<usize> = Vec::new();
    let mut d = 1usize;
    while d < n {
        strides.push(d);
        let mut i = 2 * d - 1;
        while i < n {
            t[i] = combine(cx, t[i], t[i - d]);
            i += 2 * d;
        }
        d *= 2;
    }
    for &d in strides.iter().rev() {
        let mut i = 3 * d - 1;
        while i < n {
            t[i] = combine(cx, t[i], t[i - d]);
            i += 2 * d;
        }
    }
    t
}

/// The product of every item, as a balanced binary tree.
///
/// Only the last prefix is produced, in `⌈log2 n⌉` levels and `n - 1`
/// operator nodes, which is the cheapest way to get a single carry-out
/// or a single comparison verdict.
pub fn reduce(cx: &mut GateBuilder<'_>, items: &[Prefix]) -> Option<Prefix> {
    if items.is_empty() {
        return None;
    }
    let mut layer = items.to_vec();
    while layer.len() > 1 {
        let mut next = Vec::with_capacity(layer.len().div_ceil(2));
        let mut i = 0;
        while i + 1 < layer.len() {
            // `layer[i + 1]` is the more significant of the pair.
            let v = combine(cx, layer[i + 1], layer[i]);
            next.push(v);
            i += 2;
        }
        if i < layer.len() {
            next.push(layer[i]);
        }
        layer = next;
    }
    Some(layer[0])
}

/// The generate of the product of every item, as a balanced binary tree.
///
/// Like [`reduce`] but without the propagate of the root, which nothing
/// reads.
pub fn reduce_g(cx: &mut GateBuilder<'_>, items: &[Prefix]) -> Option<ExprId> {
    let n = items.len();
    if n == 0 {
        return None;
    }
    if n == 1 {
        return Some(items[0].g);
    }
    let (lo, hi) = items.split_at(n / 2);
    let lo_g = reduce_g(cx, lo)?;
    let hi = reduce(cx, hi)?;
    Some(combine_g(cx, hi, lo_g))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::arith::testkit::exhaustive;

    /// The prefix generates, computed by hand.
    fn model(g: u64, p: u64, n: u32) -> u64 {
        let mut out = 0u64;
        let (mut acc_g, mut acc_p) = (false, false);
        for i in 0..n {
            let (gi, pi) = (g >> i & 1 == 1, p >> i & 1 == 1);
            let next_g = gi | (pi & acc_g);
            let next_p = pi & if i == 0 { true } else { acc_p };
            let (cur_g, cur_p) = if i == 0 { (gi, pi) } else { (next_g, next_p) };
            acc_g = cur_g;
            acc_p = cur_p;
            out |= u64::from(acc_g) << i;
        }
        out
    }

    /// All three networks compute the same scan, which is the whole
    /// claim the fast adders rest on.
    #[test]
    fn every_network_computes_the_same_prefixes() {
        type Network = fn(&mut GateBuilder<'_>, &[Prefix]) -> Vec<Prefix>;
        let networks: [(&str, Network); 3] = [
            ("serial", serial),
            ("kogge_stone", kogge_stone),
            ("brent_kung", brent_kung),
        ];
        for (name, network) in networks {
            for n in 1..=5u32 {
                exhaustive(
                    &format!("{name} prefixes over {n} columns"),
                    &[n, n],
                    |cx, ins| {
                        let items: Vec<Prefix> = ins[0]
                            .iter()
                            .zip(&ins[1])
                            .map(|(&g, &p)| Prefix::new(g, p))
                            .collect();
                        let _ = cx;
                        network(cx, &items).iter().map(|x| x.g).collect()
                    },
                    move |v| model(v[0], v[1], n),
                );
            }
        }
    }

    /// The reductions produce the last prefix and nothing else.
    #[test]
    fn the_reductions_are_the_last_prefix() {
        for n in 1..=5u32 {
            exhaustive(
                &format!("reduce over {n} columns"),
                &[n, n],
                |cx, ins| {
                    let items: Vec<Prefix> = ins[0]
                        .iter()
                        .zip(&ins[1])
                        .map(|(&g, &p)| Prefix::new(g, p))
                        .collect();
                    let whole = reduce(cx, &items).expect("non-empty");
                    let only_g = reduce_g(cx, &items).expect("non-empty");
                    vec![whole.g, only_g]
                },
                move |v| {
                    let top = model(v[0], v[1], n) >> (n - 1) & 1;
                    top | top << 1
                },
            );
        }
    }

    /// An empty scan has no last prefix.
    #[test]
    fn an_empty_scan_reduces_to_nothing() {
        use crate::ir::Type;
        use crate::ir::builder::ModuleBuilder;
        use crate::synth::arith::testkit::span;

        let mut b = ModuleBuilder::new("m", span());
        let _ = b.input("a", Type::bit());
        let mut cx = GateBuilder::new(&mut b, "u");
        assert!(reduce(&mut cx, &[]).is_none());
        assert!(reduce_g(&mut cx, &[]).is_none());
        assert!(kogge_stone(&mut cx, &[]).is_empty());
        assert!(brent_kung(&mut cx, &[]).is_empty());
        assert_eq!(cx.cell_count(), 0);
    }
}
