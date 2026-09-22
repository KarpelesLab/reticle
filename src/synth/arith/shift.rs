//! Shifters: barrel and funnel networks with a variable amount.
//!
//! A variable shift is a mux network. The barrel shifter looks at the
//! shift amount one digit at a time: digit `t` of the amount, in radix
//! `r`, chooses between shifting the running value by `0`, `r^t`,
//! `2·r^t`, … `(r-1)·r^t`, and the digits compose because the shifts
//! add. With `⌈log_r n⌉` stages and `r - 1` multiplexers per output bit
//! per stage, the whole network is `O(n log n)` cells deep `O(log n)`.
//!
//! **The radix is a deliberate knob, and it is not free.** At radix two
//! a stage is one multiplexer per bit and one level deep. At radix four
//! a stage is three multiplexers per bit and *two* levels deep, because
//! a 4:1 multiplexer built from the two-input primitive set is a tree of
//! three 2:1 ones — so in generic cells radix four is strictly worse:
//! more cells at the same depth. It earns its place only after
//! technology mapping, where a 4:1 multiplexer is one LUT6 (or two
//! LUT4s) and the stage really is one level. `docs/arithmetic.md`
//! reports both so the trade is visible rather than asserted.
//!
//! # Amounts out of range
//!
//! IEEE 1364-2005 §5.1.12 says a shift by an amount at or above the
//! width shifts everything out. The amount is read as an unsigned
//! number however wide it is: the stages cover every amount below the
//! next power of the radix, and the amount bits above those are OR-ed
//! into an *overflow* that forces the result to the fill value. That is
//! exactly what [`crate::synth::aig::Aig::shl_bits`] and the bit-blaster
//! do, which is why a lowered shifter proves equivalent to the generic
//! cell it replaces.
//!
//! # Funnel shifting and rotates
//!
//! A funnel shifter slides an `n`-bit window across a `2n`-bit word
//! formed from two operands. [`funnel_right`] takes the window at
//! offset `amount` — that is `{hi, lo} >> amount` truncated — and
//! [`funnel_left`] takes the top window of `{hi, lo} << amount`. Feed
//! the same operand to both halves and the window wraps around: that is
//! a rotation, [`rotate_right`] and [`rotate_left`], with no arithmetic
//! on the amount and no extra logic. Feed a zero or sign-extension half
//! instead and the same network performs the ordinary shifts, which is
//! what [`ShifterArch::Funnel`] does: one network for every shift and
//! rotate a datapath needs, at about twice the cells of a plain barrel.
//!
//! Reference: I. Koren, *Computer Arithmetic Algorithms*, 2nd ed.,
//! §2.6; the funnel formulation follows the MIPS R4000 shifter.

use super::bits::{GateBuilder, to_usize};
use crate::ir::ExprId;

/// How a variable shift is built.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ShifterArch {
    /// A radix-2 barrel shifter: one multiplexer per bit per amount bit.
    #[default]
    Barrel,
    /// A radix-4 barrel shifter: half as many stages, three
    /// multiplexers per bit in each.
    Barrel4,
    /// A funnel shifter over a `2n`-bit word, the network that also
    /// rotates.
    Funnel,
}

impl ShifterArch {
    /// Every architecture, in a fixed order.
    pub const ALL: [ShifterArch; 3] = [
        ShifterArch::Barrel,
        ShifterArch::Barrel4,
        ShifterArch::Funnel,
    ];

    /// The name used in reports and in an `arith` attribute.
    pub fn name(self) -> &'static str {
        match self {
            ShifterArch::Barrel => "barrel",
            ShifterArch::Barrel4 => "barrel4",
            ShifterArch::Funnel => "funnel",
        }
    }

    /// The architecture with the given name, if it is one.
    pub fn from_name(name: &str) -> Option<ShifterArch> {
        ShifterArch::ALL.into_iter().find(|a| a.name() == name)
    }

    /// The radix of the multiplexer network.
    pub fn radix(self) -> u32 {
        match self {
            ShifterArch::Barrel4 => 4,
            _ => 2,
        }
    }
}

/// Which shift a [`shift`] call performs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShiftKind {
    /// Towards the most significant end, filling with zeros.
    Left,
    /// Towards the least significant end, filling with zeros.
    RightLogical,
    /// Towards the least significant end, filling with the sign bit.
    RightArithmetic,
}

/// `a` shifted by `amount` in the given architecture.
///
/// `amount` is read as an unsigned number of any width; see the module
/// docs for amounts at or above the width of `a`.
pub fn shift(
    cx: &mut GateBuilder<'_>,
    arch: ShifterArch,
    a: &[ExprId],
    amount: &[ExprId],
    kind: ShiftKind,
) -> Vec<ExprId> {
    match arch {
        ShifterArch::Barrel | ShifterArch::Barrel4 => barrel(cx, a, amount, kind, arch.radix()),
        ShifterArch::Funnel => funnel_shift(cx, a, amount, kind, arch.radix()),
    }
}

/// A barrel shifter over `a` itself.
///
/// # Panics
///
/// Panics when `radix` is not a power of two above one.
pub fn barrel(
    cx: &mut GateBuilder<'_>,
    a: &[ExprId],
    amount: &[ExprId],
    kind: ShiftKind,
    radix: u32,
) -> Vec<ExprId> {
    let fill = fill_bit(cx, a, kind);
    let left = kind == ShiftKind::Left;
    network(cx, a, amount, left, fill, radix, a.len())
}

/// The same shifts through the funnel network: the operand is paired
/// with a half made of the fill bit, and the window is taken at the
/// shifted offset.
fn funnel_shift(
    cx: &mut GateBuilder<'_>,
    a: &[ExprId],
    amount: &[ExprId],
    kind: ShiftKind,
    radix: u32,
) -> Vec<ExprId> {
    let n = a.len();
    let fill = fill_bit(cx, a, kind);
    let filler = vec![fill; n];
    let (mut wide, left) = match kind {
        // {a, 0} shifted left, top window.
        ShiftKind::Left => (with_high(a, &filler, true), true),
        // {fill, a} shifted right, bottom window.
        _ => (with_high(a, &filler, false), false),
    };
    wide = network(cx, &wide, amount, left, fill, radix, n);
    if left {
        wide.split_off(n)
    } else {
        wide.truncate(n);
        wide
    }
}

/// `{hi, lo}` as one word, LSB first; `a_is_high` says which operand
/// `a` is.
fn with_high(a: &[ExprId], other: &[ExprId], a_is_high: bool) -> Vec<ExprId> {
    let mut out = Vec::with_capacity(a.len() + other.len());
    if a_is_high {
        out.extend_from_slice(other);
        out.extend_from_slice(a);
    } else {
        out.extend_from_slice(a);
        out.extend_from_slice(other);
    }
    out
}

/// The bit shifted in: the sign bit for an arithmetic right shift, zero
/// otherwise.
fn fill_bit(cx: &mut GateBuilder<'_>, a: &[ExprId], kind: ShiftKind) -> ExprId {
    match (kind, a.last()) {
        (ShiftKind::RightArithmetic, Some(&msb)) => msb,
        _ => cx.zero(),
    }
}

/// The `n`-bit window of `{hi, lo}` starting at bit `amount`, that is
/// `({hi, lo} >> amount)` truncated to `n` bits.
///
/// # Panics
///
/// Panics when `hi` and `lo` have different lengths.
pub fn funnel_right(
    cx: &mut GateBuilder<'_>,
    hi: &[ExprId],
    lo: &[ExprId],
    amount: &[ExprId],
    radix: u32,
) -> Vec<ExprId> {
    assert_eq!(hi.len(), lo.len(), "funnel halves must have equal width");
    let n = lo.len();
    let wide = with_high(hi, lo, true);
    let fill = cx.zero();
    let mut out = network(cx, &wide, amount, false, fill, radix, 2 * n);
    out.truncate(n);
    out
}

/// The top `n` bits of `{hi, lo} << amount`.
///
/// # Panics
///
/// Panics when `hi` and `lo` have different lengths.
pub fn funnel_left(
    cx: &mut GateBuilder<'_>,
    hi: &[ExprId],
    lo: &[ExprId],
    amount: &[ExprId],
    radix: u32,
) -> Vec<ExprId> {
    assert_eq!(hi.len(), lo.len(), "funnel halves must have equal width");
    let n = lo.len();
    let wide = with_high(hi, lo, true);
    let fill = cx.zero();
    let out = network(cx, &wide, amount, true, fill, radix, 2 * n);
    out[n..].to_vec()
}

/// `a` rotated right by `amount`.
///
/// The amount must be below the width; a larger one is not a rotation
/// (the window leaves the doubled word and reads zeros). Mask the
/// amount first when the width is a power of two, or compare it against
/// the width when it is not.
pub fn rotate_right(
    cx: &mut GateBuilder<'_>,
    a: &[ExprId],
    amount: &[ExprId],
    radix: u32,
) -> Vec<ExprId> {
    funnel_right(cx, a, a, amount, radix)
}

/// `a` rotated left by `amount`, with the same restriction on the
/// amount as [`rotate_right`].
pub fn rotate_left(
    cx: &mut GateBuilder<'_>,
    a: &[ExprId],
    amount: &[ExprId],
    radix: u32,
) -> Vec<ExprId> {
    funnel_left(cx, a, a, amount, radix)
}

/// The multiplexer network itself.
///
/// `limit` is the width the *overflow* rule is stated in terms of: an
/// amount that cannot be represented by the stages built for `limit`
/// forces the whole word to `fill`. For a plain shifter that is the
/// operand width; for a funnel it is the doubled width, since the
/// window may legitimately sit anywhere in the pair.
fn network(
    cx: &mut GateBuilder<'_>,
    a: &[ExprId],
    amount: &[ExprId],
    left: bool,
    fill: ExprId,
    radix: u32,
    limit: usize,
) -> Vec<ExprId> {
    assert!(
        radix >= 2 && radix.is_power_of_two(),
        "the shifter radix must be a power of two above one"
    );
    let per_stage = to_usize(radix.trailing_zeros());
    let mut cur = a.to_vec();
    let mut used = 0usize;
    let mut step = 1usize;
    while step < limit && used < amount.len() {
        let select = &amount[used..(used + per_stage).min(amount.len())];
        let mut candidates = Vec::with_capacity(to_usize(radix));
        candidates.push(cur.clone());
        for k in 1..to_usize(radix) {
            candidates.push(shift_by(&cur, k * step, left, fill));
        }
        cur = select_word(cx, &candidates, select);
        used += per_stage;
        step = step.saturating_mul(to_usize(radix));
    }
    let rest = &amount[used.min(amount.len())..];
    if rest.is_empty() {
        return cur;
    }
    let over = cx.or_all(rest);
    let all_fill = vec![fill; cur.len()];
    cur.iter()
        .zip(&all_fill)
        .map(|(&x, &f)| cx.mux(over, f, x))
        .collect()
}

/// `a` shifted by the constant `d`, which is pure rewiring.
fn shift_by(a: &[ExprId], d: usize, left: bool, fill: ExprId) -> Vec<ExprId> {
    let n = a.len();
    (0..n)
        .map(|i| {
            if left {
                if i >= d { a[i - d] } else { fill }
            } else if i + d < n {
                a[i + d]
            } else {
                fill
            }
        })
        .collect()
}

/// Selects one of `candidates` with the binary code `select`, as a
/// balanced tree of two-input multiplexers.
///
/// Codes beyond the number of candidates cannot occur: there are
/// exactly `2^select.len()` candidates whenever `select` is a full
/// digit, and a short final digit selects among the first few.
fn select_word(
    cx: &mut GateBuilder<'_>,
    candidates: &[Vec<ExprId>],
    select: &[ExprId],
) -> Vec<ExprId> {
    let mut layer: Vec<Vec<ExprId>> = candidates.to_vec();
    for &s in select {
        if layer.len() == 1 {
            break;
        }
        let mut next = Vec::with_capacity(layer.len().div_ceil(2));
        let mut i = 0;
        while i < layer.len() {
            match layer.get(i + 1) {
                Some(hi) => {
                    let lo = &layer[i];
                    let merged = lo
                        .iter()
                        .zip(hi)
                        .map(|(&e, &t)| cx.mux(s, t, e))
                        .collect::<Vec<_>>();
                    next.push(merged);
                }
                // An odd candidate is reached only by a code that also
                // sets a higher select bit, which the next level mixes
                // in; carrying it forward keeps the tree balanced.
                None => next.push(layer[i].clone()),
            }
            i += 2;
        }
        layer = next;
    }
    layer.remove(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::arith::testkit::{exhaustive, signed, unsigned};

    /// The model of an IEEE 1364 shift: the amount is unsigned, and one
    /// at or above the width shifts everything out.
    fn shifted(value: u64, amount: u64, w: u32, kind: ShiftKind) -> u64 {
        let mask = (1u64 << w) - 1;
        let negative = value >> (w - 1) & 1 == 1;
        if amount >= u64::from(w) {
            return match kind {
                ShiftKind::RightArithmetic if negative => mask,
                _ => 0,
            };
        }
        match kind {
            ShiftKind::Left => (value << amount) & mask,
            ShiftKind::RightLogical => value >> amount,
            ShiftKind::RightArithmetic => unsigned(signed(value, w) >> amount, w),
        }
    }

    /// Every architecture, every shift, every operand and every amount
    /// — including the amounts at and above the width, which is where
    /// the overflow rule lives.
    #[test]
    fn every_architecture_shifts() {
        for arch in ShifterArch::ALL {
            for w in 1..=5u32 {
                for aw in [2u32, 4] {
                    for kind in [
                        ShiftKind::Left,
                        ShiftKind::RightLogical,
                        ShiftKind::RightArithmetic,
                    ] {
                        exhaustive(
                            &format!("{} {kind:?}, {w} bits, amount {aw}", arch.name()),
                            &[w, aw],
                            |cx, ins| shift(cx, arch, &ins[0], &ins[1], kind),
                            move |v| shifted(v[0], v[1], w, kind),
                        );
                    }
                }
            }
        }
    }

    /// The funnel is a window on the doubled word, in both directions.
    #[test]
    fn the_funnel_takes_a_window() {
        for radix in [2u32, 4] {
            for w in 1..=4u32 {
                for aw in [2u32, 3] {
                    exhaustive(
                        &format!("funnel right, {w} bits, amount {aw}, radix {radix}"),
                        &[w, w, aw],
                        |cx, ins| funnel_right(cx, &ins[0], &ins[1], &ins[2], radix),
                        move |v| {
                            let word = v[0] << w | v[1];
                            if v[2] >= 64 { 0 } else { word >> v[2] }
                        },
                    );
                    exhaustive(
                        &format!("funnel left, {w} bits, amount {aw}, radix {radix}"),
                        &[w, w, aw],
                        |cx, ins| funnel_left(cx, &ins[0], &ins[1], &ins[2], radix),
                        move |v| {
                            let word = v[0] << w | v[1];
                            let mask = (1u64 << (2 * w)) - 1;
                            if v[2] >= 64 {
                                0
                            } else {
                                ((word << v[2]) & mask) >> w
                            }
                        },
                    );
                }
            }
        }
    }

    /// A rotate is the funnel fed the same operand twice, for amounts
    /// below the width.
    #[test]
    fn the_funnel_rotates() {
        for radix in [2u32, 4] {
            for (w, aw) in [(2u32, 1u32), (4, 2), (8, 3)] {
                exhaustive(
                    &format!("rotate right, {w} bits, radix {radix}"),
                    &[w, aw],
                    |cx, ins| rotate_right(cx, &ins[0], &ins[1], radix),
                    move |v| (v[0] >> v[1]) | (v[0] << (u64::from(w) - v[1])),
                );
                exhaustive(
                    &format!("rotate left, {w} bits, radix {radix}"),
                    &[w, aw],
                    |cx, ins| rotate_left(cx, &ins[0], &ins[1], radix),
                    move |v| (v[0] << v[1]) | (v[0] >> (u64::from(w) - v[1])),
                );
            }
        }
    }

    /// The funnel formulation of a plain shift folds back into exactly
    /// the barrel network, because one half of its doubled word is
    /// constant. The table in `docs/arithmetic.md` shows the two rows
    /// as equal, and this is why.
    #[test]
    fn a_funnel_shift_folds_back_into_a_barrel() {
        use crate::ir::Type;
        use crate::ir::builder::ModuleBuilder;
        use crate::synth::arith::testkit::span;

        let count = |arch: ShifterArch| {
            let mut b = ModuleBuilder::new("m", span());
            let a = b.input("a", Type::bits(16));
            let amt = b.input("amt", Type::bits(4));
            let (av, amv) = (b.net(a), b.net(amt));
            let mut cx = GateBuilder::new(&mut b, "u");
            let ab = cx.split(av, 16);
            let amb = cx.split(amv, 4);
            let _ = shift(&mut cx, arch, &ab, &amb, ShiftKind::Left);
            cx.cell_count()
        };
        assert_eq!(count(ShifterArch::Funnel), count(ShifterArch::Barrel));
    }

    #[test]
    fn architecture_names_round_trip() {
        for arch in ShifterArch::ALL {
            assert_eq!(ShifterArch::from_name(arch.name()), Some(arch));
        }
        assert_eq!(ShifterArch::from_name("ripple"), None);
        assert_eq!(ShifterArch::Barrel.radix(), 2);
        assert_eq!(ShifterArch::Barrel4.radix(), 4);
        assert_eq!(ShifterArch::default(), ShifterArch::Barrel);
    }
}
