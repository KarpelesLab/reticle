//! Division: the combinational restoring array, and when not to use it.
//!
//! Division does not have an architecture choice worth the name in
//! combinational logic. A sequential restoring or non-restoring divider
//! computes one quotient bit per clock cycle and is the right circuit
//! for almost every real design, but it is a *sequential* circuit: it
//! has a state machine, a start and a done, and a latency. A pass that
//! replaces one cell with combinational logic cannot produce one
//! without inventing a protocol the surrounding design does not know
//! about.
//!
//! So this module provides the one thing a combinational pass can
//! provide — the restoring array, which is the sequential algorithm
//! unrolled, `w` stages of a `w + 1` bit subtract and a restore — and
//! [`super::ArithOptions::width_thresholds`] caps the width it is built
//! at. Above the cap the `div` or `mod` cell is left alone, with a
//! diagnostic (`S0043`) saying so, because the alternative is silently
//! emitting tens of thousands of cells for one `/` in the source. The
//! answer there is to pipeline the division, which means writing it as
//! a sequential block in the HDL, or to instantiate a divider IP.
//!
//! # Semantics
//!
//! The array follows IEEE 1364-2005 for two-state values and, bit for
//! bit, what [`crate::formal::blast`] and [`crate::synth::aig`] already
//! do, so a lowered divider proves equivalent to the cell it replaced:
//!
//! - Unsigned division is restoring: at each step the shifted remainder
//!   is compared against the divisor and reduced when it fits, and the
//!   comparison's outcome is the quotient bit.
//! - Signed division is truncating, and the remainder takes the sign of
//!   the dividend. It is computed on magnitudes and the signs applied
//!   afterwards.
//! - **Division by zero** yields an all-ones quotient and the dividend
//!   as the remainder. The standard says `x`; every step's trial
//!   subtraction succeeds against a zero divisor, so the array says
//!   all-ones, and the rest of the toolchain agrees.
//!
//! Reference: I. Koren, *Computer Arithmetic Algorithms*, 2nd ed.,
//! chapter 7.

use super::adder::{self, AdderOptions};
use super::bits::GateBuilder;
use crate::ir::ExprId;

/// A quotient and a remainder.
#[derive(Clone, Debug)]
pub struct Division {
    /// `a / b`, as wide as the operands.
    pub quotient: Vec<ExprId>,
    /// `a % b`, as wide as the operands.
    pub remainder: Vec<ExprId>,
}

/// The combinational restoring array for `a / b` and `a % b`.
///
/// `adder` selects the architecture of the subtractor each stage uses;
/// the stages are chained by the data, not by a carry, so a fast adder
/// shortens every one of them.
///
/// # Panics
///
/// Panics when `a` and `b` have different lengths.
pub fn divide(
    cx: &mut GateBuilder<'_>,
    adder: &AdderOptions,
    a: &[ExprId],
    b: &[ExprId],
    signed: bool,
) -> Division {
    assert_eq!(a.len(), b.len(), "divider operands must have equal width");
    if a.is_empty() {
        return Division {
            quotient: Vec::new(),
            remainder: Vec::new(),
        };
    }
    if !signed {
        return unsigned_divide(cx, adder, a, b);
    }
    let sa = *a.last().expect("non-empty");
    let sb = *b.last().expect("non-empty");
    let na = negate(cx, adder, a);
    let nb = negate(cx, adder, b);
    let abs_a = select(cx, sa, &na, a);
    let abs_b = select(cx, sb, &nb, b);
    let d = unsigned_divide(cx, adder, &abs_a, &abs_b);
    let flip = cx.xor(sa, sb);
    let nq = negate(cx, adder, &d.quotient);
    let nr = negate(cx, adder, &d.remainder);
    Division {
        quotient: select(cx, flip, &nq, &d.quotient),
        remainder: select(cx, sa, &nr, &d.remainder),
    }
}

/// The restoring array proper.
///
/// The remainder carries one bit more than the operands so the shifted
/// value cannot overflow before the trial subtraction; the subtraction's
/// carry out is "the divisor fitted", which is both the quotient bit and
/// the select between the reduced and the restored remainder.
fn unsigned_divide(
    cx: &mut GateBuilder<'_>,
    adder: &AdderOptions,
    a: &[ExprId],
    b: &[ExprId],
) -> Division {
    let w = a.len();
    let zero = cx.zero();
    let one = cx.one();
    let mut rem = vec![zero; w + 1];
    let mut quot = vec![zero; w];
    // Every stage subtracts the same divisor, so its complement is
    // built once and each stage is an addition with a carry in.
    let mut complement: Vec<ExprId> = b.iter().map(|&x| cx.not(x)).collect();
    complement.push(one);
    for i in (0..w).rev() {
        let mut shifted = Vec::with_capacity(w + 1);
        shifted.push(a[i]);
        shifted.extend_from_slice(&rem[..w]);
        let d = adder::add(cx, adder, &shifted, &complement, one);
        let fits = d.carry_out;
        quot[i] = fits;
        rem = select(cx, fits, &d.bits, &shifted);
    }
    rem.truncate(w);
    Division {
        quotient: quot,
        remainder: rem,
    }
}

/// Two's complement negation, through the chosen adder.
fn negate(cx: &mut GateBuilder<'_>, adder: &AdderOptions, a: &[ExprId]) -> Vec<ExprId> {
    let zeros = cx.zeros(a.len());
    let zero = cx.zero();
    adder::subtract(cx, adder, &zeros, a, zero).bits
}

/// `s ? t : e`, bit by bit.
fn select(cx: &mut GateBuilder<'_>, s: ExprId, t: &[ExprId], e: &[ExprId]) -> Vec<ExprId> {
    t.iter().zip(e).map(|(&x, &y)| cx.mux(s, x, y)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::arith::AdderArch;
    use crate::synth::arith::testkit::exhaustive;

    /// The model of the array: magnitudes divided, signs applied, and a
    /// zero divisor answering all ones.
    fn divmod(a: u64, b: u64, w: u32, signed: bool) -> (u64, u64) {
        let mask = (1u64 << w) - 1;
        let negate = |x: u64| (1u64 << w).wrapping_sub(x) & mask;
        let (sa, sb) = if signed {
            (a >> (w - 1) & 1 == 1, b >> (w - 1) & 1 == 1)
        } else {
            (false, false)
        };
        let (ma, mb) = (
            if sa { negate(a) } else { a },
            if sb { negate(b) } else { b },
        );
        let (q, r) = match (ma.checked_div(mb), ma.checked_rem(mb)) {
            (Some(q), Some(r)) => (q, r),
            _ => (mask, ma),
        };
        let q = if sa != sb { negate(q) } else { q };
        let r = if sa { negate(r) } else { r };
        (q, r)
    }

    /// Every width up to four, signed and unsigned, over every pair of
    /// operands — which includes every division by zero.
    #[test]
    fn the_array_divides() {
        for arch in [
            AdderArch::Ripple,
            AdderArch::KoggeStone,
            AdderArch::BrentKung,
        ] {
            let options = AdderOptions::new(arch);
            for w in 1..=4u32 {
                for is_signed in [false, true] {
                    exhaustive(
                        &format!("{} divider, {w} bits, signed {is_signed}", arch.name()),
                        &[w, w],
                        |cx, ins| {
                            let d = divide(cx, &options, &ins[0], &ins[1], is_signed);
                            let mut bits = d.quotient;
                            bits.extend(d.remainder);
                            bits
                        },
                        move |v| {
                            let (q, r) = divmod(v[0], v[1], w, is_signed);
                            q | (r << w)
                        },
                    );
                }
            }
        }
    }

    /// Division by zero is the documented all-ones quotient with the
    /// dividend as remainder, which is what the rest of the toolchain
    /// computes and therefore what the proofs compare against.
    #[test]
    fn a_zero_divisor_answers_all_ones() {
        assert_eq!(divmod(5, 0, 4, false), (15, 5));
        assert_eq!(divmod(5, 0, 4, true), (15, 5));
        // -7 / 0 is the same all-ones magnitude with the signs applied:
        // the quotient comes out as -15, that is 1 in four bits.
        assert_eq!(divmod(9, 0, 4, true), (1, 9));
        assert_eq!(divmod(7, 2, 4, false), (3, 1));
        // -7 / 2 truncates towards zero and the remainder keeps the
        // dividend's sign: -3 remainder -1.
        assert_eq!(divmod(9, 2, 4, true), (13, 15));
    }

    /// An empty operand divides into an empty answer rather than
    /// panicking on the slice of a zero-width word.
    #[test]
    fn an_empty_division_is_empty() {
        use crate::ir::Type;
        use crate::ir::builder::ModuleBuilder;
        use crate::synth::arith::testkit::span;

        let mut b = ModuleBuilder::new("m", span());
        let _ = b.input("a", Type::bit());
        let mut cx = GateBuilder::new(&mut b, "u");
        let d = divide(&mut cx, &AdderOptions::default(), &[], &[], true);
        assert!(d.quotient.is_empty() && d.remainder.is_empty());
        assert_eq!(cx.cell_count(), 0);
    }
}
