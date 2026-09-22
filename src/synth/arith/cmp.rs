//! Comparators: equality, less-than and the signed variants.
//!
//! A magnitude comparison is the same associative scan as a carry chain.
//! Read `g_i` as "bit `i` decides the comparison, and it decides that `a`
//! is below `b`" and `p_i` as "bit `i` does not decide anything, the
//! lower bits do"; then `(g, p) ∘ (g', p')` — the operator of
//! [`super::prefix`] — combines a more significant span with a less
//! significant one, and the generate of the whole product is `a < b`
//! while its propagate is `a == b`. That is why the fast comparator
//! here is literally the prefix adder's tree with different leaves:
//! `a < b` is the borrow out of `a - b`, and a borrow chain is a carry
//! chain.
//!
//! Two architectures:
//!
//! - [`CompareArch::Ripple`] walks the bits from the bottom, one
//!   multiplexer per bit, depth `n`. Fewest cells.
//! - [`CompareArch::Prefix`] reduces the same columns through a balanced
//!   tree, depth `2 log n`. Only the root of the tree is needed, so this
//!   costs `n - 1` operator nodes and no scatter: a comparator is
//!   cheaper to make fast than an adder is.
//!
//! Signedness is one gate's worth of difference: on the most
//! significant bit the roles swap, since there a one means *smaller*.
//! Equality is the same either way.

use super::bits::GateBuilder;
use super::prefix::{self, Prefix};
use crate::ir::ExprId;

/// How a comparison is evaluated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CompareArch {
    /// A chain from the least significant bit up; depth `n`.
    #[default]
    Ripple,
    /// A balanced prefix tree; depth `2 log n`.
    Prefix,
}

impl CompareArch {
    /// Every architecture, in a fixed order.
    pub const ALL: [CompareArch; 2] = [CompareArch::Ripple, CompareArch::Prefix];

    /// The name used in reports and in an `arith` attribute.
    pub fn name(self) -> &'static str {
        match self {
            CompareArch::Ripple => "ripple",
            CompareArch::Prefix => "prefix",
        }
    }

    /// The architecture with the given name, if it is one.
    pub fn from_name(name: &str) -> Option<CompareArch> {
        CompareArch::ALL.into_iter().find(|a| a.name() == name)
    }
}

/// The two verdicts every other comparison is derived from.
#[derive(Clone, Copy, Debug)]
pub struct Comparison {
    /// `a < b`.
    pub lt: ExprId,
    /// `a == b`.
    pub eq: ExprId,
}

impl Comparison {
    /// `a != b`.
    pub fn ne(&self, cx: &mut GateBuilder<'_>) -> ExprId {
        cx.not(self.eq)
    }

    /// `a <= b`.
    pub fn le(&self, cx: &mut GateBuilder<'_>) -> ExprId {
        cx.or(self.lt, self.eq)
    }

    /// `a > b`.
    pub fn gt(&self, cx: &mut GateBuilder<'_>) -> ExprId {
        let le = self.le(cx);
        cx.not(le)
    }

    /// `a >= b`.
    pub fn ge(&self, cx: &mut GateBuilder<'_>) -> ExprId {
        cx.not(self.lt)
    }
}

/// The decision columns of a comparison: `g` is "this bit says `a` is
/// below `b`", `p` is "this bit says nothing".
///
/// On the most significant bit of a signed comparison the two operands'
/// roles swap, because a one there is a sign, not a magnitude.
fn columns(
    cx: &mut GateBuilder<'_>,
    a: &[ExprId],
    b: &[ExprId],
    signed: bool,
) -> (Vec<ExprId>, Vec<ExprId>) {
    let n = a.len();
    let mut below = Vec::with_capacity(n);
    let mut differ = Vec::with_capacity(n);
    for (i, (&x, &y)) in a.iter().zip(b).enumerate() {
        let top = signed && i + 1 == n;
        let (x, y) = if top { (y, x) } else { (x, y) };
        let nx = cx.not(x);
        below.push(cx.and(nx, y));
        differ.push(cx.xor(x, y));
    }
    (below, differ)
}

/// `a < b` and `a == b`, in the given architecture.
///
/// # Panics
///
/// Panics when `a` and `b` have different lengths.
pub fn compare(
    cx: &mut GateBuilder<'_>,
    arch: CompareArch,
    a: &[ExprId],
    b: &[ExprId],
    signed: bool,
) -> Comparison {
    assert_eq!(
        a.len(),
        b.len(),
        "comparator operands must have equal width"
    );
    if a.is_empty() {
        let (lt, eq) = (cx.zero(), cx.one());
        return Comparison { lt, eq };
    }
    let (below, differ) = columns(cx, a, b, signed);
    match arch {
        CompareArch::Ripple => {
            let mut lt = cx.zero();
            for (&d, &bl) in differ.iter().zip(&below) {
                lt = cx.mux(d, bl, lt);
            }
            let any = cx.or_chain(&differ);
            let eq = cx.not(any);
            Comparison { lt, eq }
        }
        CompareArch::Prefix => {
            let items: Vec<Prefix> = below
                .iter()
                .zip(&differ)
                .map(|(&g, &d)| {
                    let p = cx.not(d);
                    Prefix::new(g, p)
                })
                .collect();
            let root = prefix::reduce(cx, &items).expect("a non-empty comparison");
            Comparison {
                lt: root.g,
                eq: root.p,
            }
        }
    }
}

/// `a < b` alone, skipping the equality half of the network.
///
/// # Panics
///
/// Panics when `a` and `b` have different lengths.
pub fn less_than(
    cx: &mut GateBuilder<'_>,
    arch: CompareArch,
    a: &[ExprId],
    b: &[ExprId],
    signed: bool,
) -> ExprId {
    assert_eq!(
        a.len(),
        b.len(),
        "comparator operands must have equal width"
    );
    if a.is_empty() {
        return cx.zero();
    }
    let (below, differ) = columns(cx, a, b, signed);
    match arch {
        CompareArch::Ripple => {
            let mut lt = cx.zero();
            for (&d, &bl) in differ.iter().zip(&below) {
                lt = cx.mux(d, bl, lt);
            }
            lt
        }
        CompareArch::Prefix => {
            let items: Vec<Prefix> = below
                .iter()
                .zip(&differ)
                .map(|(&g, &d)| {
                    let p = cx.not(d);
                    Prefix::new(g, p)
                })
                .collect();
            prefix::reduce_g(cx, &items).expect("a non-empty comparison")
        }
    }
}

/// `a == b` alone: the bits are compared pairwise and the differences
/// collected, linearly for [`CompareArch::Ripple`] and through a
/// balanced tree for [`CompareArch::Prefix`].
///
/// Equality has no signed variant: two bit patterns are equal or they
/// are not.
///
/// # Panics
///
/// Panics when `a` and `b` have different lengths.
pub fn equal(cx: &mut GateBuilder<'_>, arch: CompareArch, a: &[ExprId], b: &[ExprId]) -> ExprId {
    assert_eq!(
        a.len(),
        b.len(),
        "comparator operands must have equal width"
    );
    let differ: Vec<ExprId> = a.iter().zip(b).map(|(&x, &y)| cx.xor(x, y)).collect();
    let any = match arch {
        CompareArch::Ripple => cx.or_chain(&differ),
        CompareArch::Prefix => cx.or_all(&differ),
    };
    cx.not(any)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::arith::testkit::{exhaustive, signed};

    /// Both architectures, both signednesses, every width up to five,
    /// every pair of operands: `<`, `==` and the four derived verdicts.
    #[test]
    fn every_architecture_compares() {
        for arch in CompareArch::ALL {
            for w in 1..=5u32 {
                for is_signed in [false, true] {
                    let label = |what: &str| {
                        format!("{} {what}, {w} bits, signed {is_signed}", arch.name())
                    };
                    let order = move |v: &[u64]| -> (bool, bool) {
                        if is_signed {
                            (signed(v[0], w) < signed(v[1], w), v[0] == v[1])
                        } else {
                            (v[0] < v[1], v[0] == v[1])
                        }
                    };
                    exhaustive(
                        &label("`<`"),
                        &[w, w],
                        |cx, ins| vec![less_than(cx, arch, &ins[0], &ins[1], is_signed)],
                        move |v| u64::from(order(v).0),
                    );
                    exhaustive(
                        &label("`==`"),
                        &[w, w],
                        |cx, ins| vec![equal(cx, arch, &ins[0], &ins[1])],
                        move |v| u64::from(order(v).1),
                    );
                    // `compare` returns both at once; the derived
                    // verdicts are packed above them.
                    exhaustive(
                        &label("the whole comparison"),
                        &[w, w],
                        |cx, ins| {
                            let c = compare(cx, arch, &ins[0], &ins[1], is_signed);
                            vec![c.lt, c.eq, c.ne(cx), c.le(cx), c.gt(cx), c.ge(cx)]
                        },
                        move |v| {
                            let (lt, eq) = order(v);
                            u64::from(lt)
                                | u64::from(eq) << 1
                                | u64::from(!eq) << 2
                                | u64::from(lt || eq) << 3
                                | u64::from(!(lt || eq)) << 4
                                | u64::from(!lt) << 5
                        },
                    );
                }
            }
        }
    }

    /// An empty comparison is the vacuous one: equal, and not below.
    #[test]
    fn an_empty_comparison_is_equal() {
        use crate::ir::Type;
        use crate::ir::builder::ModuleBuilder;
        use crate::synth::arith::testkit::span;

        let mut b = ModuleBuilder::new("m", span());
        let _ = b.input("a", Type::bit());
        let mut cx = GateBuilder::new(&mut b, "u");
        let c = compare(&mut cx, CompareArch::Prefix, &[], &[], false);
        assert_eq!(cx.as_const(c.lt), Some(false));
        assert_eq!(cx.as_const(c.eq), Some(true));
        let lt = less_than(&mut cx, CompareArch::Ripple, &[], &[], true);
        assert_eq!(cx.as_const(lt), Some(false));
        assert_eq!(cx.cell_count(), 0);
    }

    #[test]
    fn architecture_names_round_trip() {
        for arch in CompareArch::ALL {
            assert_eq!(CompareArch::from_name(arch.name()), Some(arch));
        }
        assert_eq!(CompareArch::from_name("kogge_stone"), None);
        assert_eq!(CompareArch::default(), CompareArch::Ripple);
    }
}
